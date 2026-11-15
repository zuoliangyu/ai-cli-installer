// Run with playwright@1.58.2 in the matching official Playwright Docker image.
// The real-install case needs a disposable installer-web server and user home.
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const require = createRequire(import.meta.url);
const { chromium } = require("playwright");
const base = new URL(process.env.BASE_URL || "http://127.0.0.1:3210");
const token = process.env.INSTALLER_TOKEN;
assert(token, "INSTALLER_TOKEN is required (use a disposable test token)");
assert(["http:", "https:"].includes(base.protocol), "BASE_URL must use HTTP(S)");
const artifactDir = path.resolve(process.env.ARTIFACT_DIR || "target/docker-validation/browser");
const installTimeout = Number(process.env.INSTALL_TIMEOUT_MS || 600_000);
assert(Number.isFinite(installTimeout) && installTimeout > 0, "Invalid INSTALL_TIMEOUT_MS");
const redact = (value) => String(value).replaceAll(token, "[redacted]").replaceAll(encodeURIComponent(token), "[redacted]");
const result = {
  started_at: new Date().toISOString(),
  base_url: base.origin,
  playwright_version: require("playwright/package.json").version,
  tests: [],
  page_errors: [],
  console_errors: [],
};
await mkdir(artifactDir, { recursive: true });

async function until(check, description, timeout = 30_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await check()) return;
    await delay(100);
  }
  throw new Error(`Timed out: ${description}`);
}

function card(page, name = "Codex") {
  return page.getByRole("article").filter({ has: page.getByRole("heading", { name, exact: true }) });
}

function latestButton(target) {
  return target.getByRole("button", { name: /^(安装|更新到) latest/ });
}

async function ready(page) {
  await page.getByRole("heading", { name: "CLI 工具", exact: true }).waitFor({ timeout: 90_000 });
  await card(page).waitFor();
  await card(page, "Claude Code").waitFor();
}

async function open(page) {
  const url = new URL(base);
  url.searchParams.set("token", token);
  await page.goto(url.href, { waitUntil: "domcontentloaded" });
  await ready(page);
  assert(!new URL(page.url()).searchParams.has("token"), "The access token must be removed from the URL");
}

let browser;
async function scenario(name, mode, action) {
  const entry = { name, mode, status: "running", started_at: new Date().toISOString() };
  result.tests.push(entry);
  console.log(`START ${name} (${mode})`);
  const context = await browser.newContext({ viewport: { width: 1440, height: 1100 }, locale: "zh-CN" });
  const page = await context.newPage();
  page.setDefaultTimeout(30_000);
  const pageErrors = [];
  const consoleErrors = [];
  page.on("pageerror", (error) => {
    const message = redact(error.stack || error.message);
    pageErrors.push(message);
    result.page_errors.push({ scenario: name, message });
  });
  page.on("console", (message) => {
    if (message.type() !== "error") return;
    const text = redact(message.text());
    const expected = mode === "simulated-task-api" && /Failed to load resource.*503/.test(text);
    consoleErrors.push({ message: text, expected });
    result.console_errors.push({ scenario: name, message: text, expected });
  });
  try {
    entry.details = await action(page, context);
    assert.equal(pageErrors.length, 0, "The page produced an uncaught JavaScript error");
    assert.equal(consoleErrors.filter((error) => !error.expected).length, 0, "The browser reported an unexpected console error");
    entry.status = "passed";
  } catch (error) {
    entry.status = "failed";
    entry.error = redact(error.stack || error.message);
  } finally {
    entry.finished_at = new Date().toISOString();
    const filename = `${result.tests.length}-${name}.png`;
    try {
      await page.screenshot({ path: path.join(artifactDir, filename), fullPage: true });
      entry.screenshot = filename;
    } catch (error) {
      entry.screenshot_error = redact(error.message);
    }
    await context.close();
    console.log(`${entry.status.toUpperCase()} ${name}${entry.error ? `: ${entry.error}` : ""}`);
  }
}

const fixtureTools = [
  { id: "claude-code", name: "Claude Code", npm_package: "@anthropic-ai/claude-code" },
  { id: "codex-cli", name: "Codex", npm_package: "@openai/codex" },
].map((tool) => ({
  ...tool,
  description: "Docker browser fixture (not an installed tool)",
  installed_version: null,
  latest_version: "0.0.0-browser-test",
  stable_version: "0.0.0-browser-test",
  stable_falls_back_to_latest: false,
  latest_version_stale: false,
  stable_version_stale: false,
  installations: [],
  install_path: null,
  supports_npm: true,
  npm_min_node: 20,
}));

// Only simulated scenarios use this router. Never pass a mutation through to
// the actual installer, even if a future UI feature starts calling a new API.
async function mockTasks(page, taskHandler) {
  await page.route("**/api/**", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    if (url.pathname === "/api/install-tasks") return taskHandler(route);
    const responses = {
      "/api/tools": fixtureTools,
      "/api/mirrors/probe": [{ name: "docker-test-mirror", ok: true, latency_ms: 1, error: null }],
      "/api/path/status": { dir: "/tmp/docker-browser/.local/bin", in_user_path: false, in_system_path: false, effective: false },
      "/api/node": { node_version: "v24.0.0", node_major: 24, npm_version: "11.0.0" },
    };
    if (Object.hasOwn(responses, url.pathname)) return route.fulfill({ json: responses[url.pathname] });
    return route.fulfill({ status: 400, body: `Unexpected simulated API request: ${request.method()} ${url.pathname}` });
  });
}

function fakeTask(body, sequence, overrides = {}) {
  return {
    session_id: "docker-browser-fixture",
    task_id: `docker-browser-fixture-${sequence}`,
    tool_id: body.toolId,
    channel: body.channel,
    method: body.method,
    mirror: body.mirror,
    status: "running",
    progress: { tool_id: body.toolId, downloaded: 42, total: 100, mirror: "docker-test-mirror" },
    report: null,
    error: null,
    ...overrides,
  };
}

function succeed(task) {
  return {
    ...task,
    status: "succeeded",
    progress: null,
    report: {
      tool_id: task.tool_id,
      version: "0.0.0-browser-test",
      install_path: "/tmp/docker-browser/.local/bin/codex",
      elapsed_secs: 1,
      method: task.method,
      auto_applied_fixes: [],
    },
  };
}

try {
  browser = await chromium.launch({ headless: true });
  if (process.env.BROWSER_REAL_INSTALL !== "0") {
    await scenario("real-install-and-refresh", "real-backend-real-install", async (page, context) => {
      let anonymous;
      await until(async () => {
        try {
          anonymous = await context.request.get(new URL("/api/install-tasks", base).href, { timeout: 2_000 });
          return anonymous.status() === 401;
        } catch {
          return false;
        }
      }, "the server is ready and requires authentication", 45_000);
      assert.equal(anonymous.status(), 401, "The task API must reject an unauthenticated request");
      const submissions = [];
      page.on("request", (request) => {
        if (request.method() === "POST" && new URL(request.url()).pathname === "/api/install-tasks") {
          submissions.push(request.postDataJSON());
        }
      });
      await open(page);
      await card(page).getByRole("radio", { name: /镜像加速/ }).check();
      const button = latestButton(card(page));
      await until(() => button.isEnabled(), "the real latest-install button is enabled", 90_000);
      const submitted = page.waitForResponse((response) => response.request().method() === "POST"
        && new URL(response.url()).pathname === "/api/install-tasks");
      // A real double-click must create just one installation task.
      await button.click({ clickCount: 2 });
      const submission = await submitted;
      assert.equal(submission.status(), 200);
      const task = await submission.json();
      assert.equal(task.tool_id, "codex-cli");
      assert.equal(task.method, "native");
      assert.equal(submissions.length, 1, "Double-click submitted more than one installation");

      await page.reload({ waitUntil: "domcontentloaded" });
      // Register after navigation: an old page's in-flight poll can otherwise
      // satisfy the predicate and lose its response body when Chromium reloads.
      const restored = await page.waitForResponse((response) => response.request().method() === "GET"
        && new URL(response.url()).pathname === "/api/install-tasks");
      const snapshot = await restored.json();
      const restoredTask = snapshot.tasks.find((entry) => entry.task_id === task.task_id);
      assert(restoredTask, "Reload lost the accepted backend task");
      await ready(page);
      assert(!new URL(page.url()).searchParams.has("token"));
      assert.equal(submissions.length, 1, "Reload submitted an extra installation");

      let completed;
      await until(async () => {
        const response = await context.request.get(new URL("/api/install-tasks", base).href, {
          headers: { Authorization: `Bearer ${token}` },
        });
        assert.equal(response.status(), 200);
        const current = (await response.json()).tasks.find((entry) => entry.task_id === task.task_id);
        assert(current, "The active task disappeared from the backend");
        if (["succeeded", "failed"].includes(current.status)) completed = current;
        if (!completed) await delay(900);
        return Boolean(completed);
      }, "real Codex installation finishes", installTimeout);
      assert.equal(completed.status, "succeeded", redact(completed.error || "Installation failed"));
      await card(page).getByRole("status").filter({ hasText: `安装 ${completed.report.version}` }).waitFor();
      // Completion and installation diagnostics use separate requests. A green
      // task message alone does not prove the tool card refreshed successfully.
      const diagnosisStarted = Date.now();
      const waitForInstalledCard = async () => {
        await card(page).getByText(`已安装 v${completed.report.version}`, { exact: true }).waitFor({ timeout: 60_000 });
        await card(page).getByText("Native", { exact: true }).waitFor({ timeout: 60_000 });
        assert.equal(await card(page).getByText("未检测到本机安装来源", { exact: true }).count(), 0);
      };
      await waitForInstalledCard();
      const diagnosisRefreshMs = Date.now() - diagnosisStarted;
      const diagnosisResponse = await context.request.get(new URL("/api/tools", base).href, {
        headers: { Authorization: `Bearer ${token}` }, timeout: 60_000,
      });
      assert.equal(diagnosisResponse.status(), 200);
      const diagnosis = (await diagnosisResponse.json()).find((entry) => entry.id === "codex-cli");
      assert.equal(diagnosis?.installed_version, completed.report.version);
      assert(diagnosis.installations.some((entry) => entry.source === "native"
        && entry.version === completed.report.version && entry.path === completed.report.install_path),
      "The diagnostics API did not identify the completed native installation");
      assert.equal(submissions.length, 1);
      await page.getByRole("button", { name: "关于", exact: true }).click();
      await page.getByRole("heading", { name: "关于", exact: true, level: 1 }).waitFor();
      await page.getByRole("button", { name: "CLI 工具", exact: true }).click();
      await card(page).getByRole("status").filter({ hasText: `安装 ${completed.report.version}` }).waitFor();
      await waitForInstalledCard();
      return {
        authenticated_reload: true,
        submit_count: submissions.length,
        task_id: task.task_id,
        reload_observed_status: restoredTask.status,
        report: completed.report,
        diagnosis_refresh_ms: diagnosisRefreshMs,
        diagnosis: { installed_version: diagnosis.installed_version, installations: diagnosis.installations },
      };
    });
  } else {
    result.tests.push({ name: "real-install-and-refresh", mode: "real-backend-real-install", status: "skipped", reason: "BROWSER_REAL_INSTALL=0" });
  }

  await scenario("failure-and-auto-retry", "simulated-task-api", async (page) => {
    const submissions = [];
    let task;
    await mockTasks(page, async (route) => {
      if (route.request().method() === "POST") {
        const body = route.request().postDataJSON();
        submissions.push(body);
        task = fakeTask(body, submissions.length, submissions.length === 1 ? {
          status: "failed", progress: null, error: "Docker 测试：模拟下载失败",
        } : {});
        return route.fulfill({ json: task });
      }
      return route.fulfill({ json: { session_id: "docker-browser-fixture", tasks: task ? [task] : [] } });
    });
    await open(page);
    await card(page).getByLabel("下载来源").selectOption("docker-test-mirror");
    await latestButton(card(page)).click();
    await card(page).getByRole("alert").filter({ hasText: "Docker 测试：模拟下载失败" }).waitFor();
    await card(page).getByRole("button", { name: "改用自动模式重试", exact: true }).click();
    await until(() => Promise.resolve(submissions.length === 2), "the retry is submitted");
    assert.equal(submissions[0].mirror, "docker-test-mirror");
    assert.equal(submissions[1].mirror, null, "Automatic retry retained the failed pinned mirror");
    await card(page).getByRole("button", { name: "安装中…", exact: true }).waitFor();
    assert(await card(page).getByRole("button", { name: "安装中…", exact: true }).isDisabled());
    task = succeed(task);
    await card(page).getByRole("status").filter({ hasText: "安装 0.0.0-browser-test" }).waitFor();
    assert.equal(await card(page).getByRole("alert").count(), 0);
    assert(await latestButton(card(page)).isEnabled());
    return { submit_count: submissions.length, retry_cleared_pinned_mirror: true, failure_ui_recovered: true };
  });

  await scenario("polling-disconnect-and-recovery", "simulated-task-api", async (page) => {
    let task;
    let sessionId = "docker-browser-fixture";
    let disconnected = false;
    let submissions = 0;
    await mockTasks(page, async (route) => {
      if (route.request().method() === "POST") {
        submissions++;
        task = fakeTask(route.request().postDataJSON(), submissions);
        return route.fulfill({ json: task });
      }
      if (disconnected) return route.fulfill({ status: 503, body: "Docker 测试：模拟后台暂时失联" });
      return route.fulfill({ json: { session_id: sessionId, tasks: task ? [task] : [] } });
    });
    await open(page);
    await latestButton(card(page)).click();
    await card(page).getByRole("status").filter({ hasText: "正在后台安装" }).waitFor();
    disconnected = true;
    await page.getByRole("alert").filter({ hasText: "恢复同步前已暂停新安装" }).waitFor();
    assert(await card(page).getByRole("button", { name: "安装中…", exact: true }).isDisabled());
    assert(await latestButton(card(page, "Claude Code")).isDisabled());
    disconnected = false;
    await page.getByRole("button", { name: "重新同步任务", exact: true }).click();
    await until(async () => (await page.getByRole("alert").count()) === 0, "task polling reconnects");
    await page.reload({ waitUntil: "domcontentloaded" });
    await ready(page);
    await card(page).getByRole("status").filter({ hasText: "正在后台安装" }).waitFor();
    assert.equal(submissions, 1, "Reload resubmitted the simulated active task");
    assert(await card(page).getByRole("button", { name: "安装中…", exact: true }).isDisabled());
    task = succeed(task);
    await card(page).getByRole("status").filter({ hasText: "安装 0.0.0-browser-test" }).waitFor();
    sessionId = "docker-browser-restarted";
    task = undefined;
    await page.getByRole("status").filter({ hasText: "后端已重启，旧安装任务记录已清除" }).waitFor();
    assert(await latestButton(card(page)).isEnabled());
    return { submit_count: submissions, active_task_restored: true, disconnected_install_blocked: true, restart_notice: true };
  });
} catch (error) {
  result.fatal_error = redact(error.stack || error.message);
} finally {
  await browser?.close();
  result.finished_at = new Date().toISOString();
  result.status = result.fatal_error || result.tests.some((entry) => entry.status === "failed") ? "failed" : "passed";
  await writeFile(path.join(artifactDir, "results.json"), `${JSON.stringify(result, null, 2)}\n`);
  console.log(JSON.stringify(result, null, 2));
  if (result.status === "failed") process.exitCode = 1;
}
