import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import * as svelteStore from "svelte/store";

// Run the real TypeScript modules with an in-memory transport, without a browser or installer.
function load(name, requireModule, globals = {}) {
  const source = readFileSync(new URL(name, import.meta.url), "utf8");
  const code = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const exports = {};
  vm.runInNewContext(code, { exports, require: requireModule, console, ...globals });
  return exports;
}

function setup() {
  const stores = load("./stores.ts", () => svelteStore);
  const calls = [];
  let completed = 0;
  let snapshot = { session_id: "process-a", tasks: [] };
  const transport = {
    listInstallTasks: async () => snapshot,
    startInstallTask: async (...args) => {
      calls.push(args);
      const task = job({ tool_id: args[0], channel: args[1], method: args[2], mirror: args[3] });
      snapshot = { ...snapshot, tasks: [task] };
      return task;
    },
  };
  const timers = new Map();
  let timerId = 0;
  const document = { hidden: false, addEventListener() {}, removeEventListener() {} };
  const { InstallTaskMonitor } = load("./installTasks.ts", (name) => {
    if (name === "svelte/store") return svelteStore;
    if (name === "./stores") return stores;
    throw new Error(`Unexpected dependency: ${name}`);
  }, {
    document,
    setTimeout: (fn, ms) => { timers.set(++timerId, { fn, ms }); return timerId; },
    clearTimeout: (id) => timers.delete(id),
  });
  const monitor = new InstallTaskMonitor(async () => transport, () => completed++);
  return {
    stores, transport, monitor, calls, timers, document,
    setSnapshot(value) { snapshot = value; },
    state: (id = "claude-code") => stores.getInstallState(id),
    sync: () => svelteStore.get(stores.installTaskSync),
    completed: () => completed,
  };
}

function job(overrides = {}) {
  return {
    session_id: "process-a", task_id: "process-a-1", tool_id: "claude-code",
    channel: "latest", method: "native", mirror: null,
    status: "running", progress: null, report: null, error: null,
    ...overrides,
  };
}

function finished(overrides = {}) {
  return job({
    status: "succeeded",
    report: { version: "1.0.0", method: "native", elapsed_secs: 1, auto_applied_fixes: [] },
    ...overrides,
  });
}

function deferred() {
  let resolve;
  const promise = new Promise((done) => { resolve = done; });
  return { promise, resolve };
}

test("startup blocks new installs and restores process-owned task progress", async () => {
  const s = setup();
  await s.monitor.install("claude-code", "latest");
  assert.equal(s.calls.length, 0);
  s.setSnapshot({ session_id: "process-a", tasks: [job({
    status: "queued", channel: "stable", method: "npm",
    progress: { tool_id: "claude-code", downloaded: 42, total: 100, mirror: "test" },
  })] });
  await s.monitor.refresh();
  assert.equal(s.state().busy, true);
  assert.equal(s.state().taskId, "process-a-1");
  assert.equal(s.state().taskStatus, "queued");
  assert.equal(s.state().channel, "stable");
  assert.equal(s.state().method, "npm");
  assert.equal(s.state().progress.downloaded, 42);
  assert.equal(s.sync().ready, true);
});

test("transient disconnection preserves active jobs and refreshes completion only once", async () => {
  const s = setup();
  s.setSnapshot({ session_id: "process-a", tasks: [job()] });
  await s.monitor.refresh();
  s.transport.listInstallTasks = async () => { throw new Error("offline"); };
  await s.monitor.refresh();
  assert.equal(s.state().busy, true);
  assert.equal(s.sync().ready, false);
  await s.monitor.install("codex-cli", "latest");
  assert.equal(s.calls.length, 0);
  s.transport.listInstallTasks = async () => ({ session_id: "process-a", tasks: [finished()] });
  await s.monitor.refresh();
  await s.monitor.refresh();
  assert.equal(s.state().busy, false);
  assert.equal(s.state().taskStatus, "succeeded");
  assert.equal(s.sync().error, null);
  assert.equal(s.completed(), 1);
});

test("lost submission acknowledgement recovers the accepted task without submitting again", async () => {
  const s = setup();
  await s.monitor.refresh();
  let attempts = 0;
  s.transport.startInstallTask = async () => {
    attempts++;
    s.setSnapshot({ session_id: "process-a", tasks: [job()] });
    throw new Error("response lost");
  };
  await s.monitor.install("claude-code", "latest");
  await s.monitor.install("claude-code", "latest");
  assert.equal(attempts, 1);
  assert.equal(s.state().taskId, "process-a-1");
  assert.equal(s.state().busy, true);
  assert.equal(s.sync().ready, true);
});

test("unaccepted submissions do not reuse an old success result on repeated retries", async () => {
  const s = setup();
  s.setSnapshot({ session_id: "process-a", tasks: [finished()] });
  await s.monitor.refresh();
  s.transport.startInstallTask = async () => { throw new Error("unavailable"); };
  for (let attempt = 0; attempt < 2; attempt++) {
    await s.monitor.install("claude-code", "latest");
    assert.equal(s.state().busy, false);
    assert.match(s.state().error, /未发现本次提交的新任务/);
    assert.equal(s.state().message, null);
  }
});

test("a pre-submission snapshot cannot erase the returned task", async () => {
  const s = setup();
  await s.monitor.refresh();
  const stale = deferred();
  const acknowledged = deferred();
  let reads = 0;
  const task = job({ status: "queued" });
  s.transport.listInstallTasks = () => ++reads === 1
    ? stale.promise
    : Promise.resolve({ session_id: "process-a", tasks: [task] });
  s.transport.startInstallTask = async () => { acknowledged.resolve(); return task; };
  const refresh = s.monitor.refresh();
  const install = s.monitor.install("claude-code", "latest");
  await acknowledged.promise;
  stale.resolve({ session_id: "process-a", tasks: [] });
  await Promise.all([refresh, install]);
  assert.equal(s.state().taskId, task.task_id);
  assert.equal(s.state().busy, true);
});

test("a backend restart clears stale busy state and explicitly reports loss of history", async () => {
  const s = setup();
  s.setSnapshot({ session_id: "process-a", tasks: [job()] });
  await s.monitor.refresh();
  s.setSnapshot({ session_id: "process-b", tasks: [] });
  await s.monitor.refresh();
  assert.equal(s.state().busy, false);
  assert.equal(s.state().taskId, null);
  assert.equal(s.state().progress, null);
  assert.match(s.sync().notice, /后端已重启/);
  assert.equal(s.completed(), 1);
});

test("npm submission omits a saved Native mirror choice", async () => {
  const s = setup();
  await s.monitor.refresh();
  s.stores.patchInstallState("claude-code", { method: "npm", mirror: "saved-native-mirror" });
  await s.monitor.install("claude-code", "latest");
  assert.equal(s.calls[0][3], null);
  assert.equal(s.state().mirror, "saved-native-mirror");
});

test("progress and completion belong to the newest task ID for each tool", async () => {
  const s = setup();
  const old = finished();
  const current = job({
    task_id: "process-a-2",
    progress: { tool_id: "claude-code", downloaded: 7, total: 100, mirror: "current" },
  });
  s.setSnapshot({ session_id: "process-a", tasks: [old, current] });
  await s.monitor.refresh();
  assert.equal(s.state().taskId, "process-a-2");
  assert.equal(s.state().progress.downloaded, 7);
  assert.equal(s.state().message, null);
  assert.equal(s.completed(), 0);
  s.setSnapshot({ session_id: "process-a", tasks: [old, {
    ...current, status: "failed", progress: null, error: "current failed",
  }] });
  await s.monitor.refresh();
  assert.equal(s.state().error, "current failed");
  assert.equal(s.state().progress, null);
  assert.equal(s.completed(), 1);
});

test("only one poll loop runs and hidden pages use a slower interval", async () => {
  const s = setup();
  s.monitor.startPolling();
  s.monitor.startPolling();
  await s.monitor.refresh();
  await Promise.resolve();
  assert.equal(s.timers.size, 1);
  const [id, timer] = [...s.timers][0];
  assert.equal(timer.ms, 1000);
  s.timers.delete(id);
  s.document.hidden = true;
  timer.fn();
  await s.monitor.refresh();
  await Promise.resolve();
  assert.equal([...s.timers.values()][0].ms, 5000);
  s.monitor.stopPolling();
  assert.equal(s.timers.size, 0);
});
