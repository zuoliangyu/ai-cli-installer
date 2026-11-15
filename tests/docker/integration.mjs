// Run inside the validation container only. Every case gets an isolated HOME.
import assert from 'node:assert/strict';
import { spawn, execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdir, mkdtemp, readFile, writeFile, readdir, stat } from 'node:fs/promises';
import { join, relative, isAbsolute } from 'node:path';
import { randomBytes } from 'node:crypto';

assert.equal(process.platform, 'linux', 'Run this script in the Docker validation image');
assert.notEqual(process.getuid(), 0, 'Tests must run as a normal user');
const exec = promisify(execFile);
const binary = process.env.INSTALLER_BINARY || '/app/target/debug/installer-web';
const artifacts = process.env.ARTIFACT_DIR || '/tmp/validation-artifacts';
const root = await mkdtemp('/tmp/ai-cli-validation-');
const timeout = Number(process.env.INSTALL_TIMEOUT_MS || 600_000);
const group = process.env.TEST_GROUP || 'all';
const selected = process.env.INSTALL_CASE;
assert.ok(Number.isFinite(timeout) && timeout > 0, 'Invalid INSTALL_TIMEOUT_MS');
assert.ok(['all', 'paths', 'install'].includes(group), 'Invalid TEST_GROUP');
const results = [];
await mkdir(artifacts, { recursive: true });
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function caseRun(name, run) {
  const started = Date.now();
  console.log(`START ${name}`);
  try {
    const details = await run();
    results.push({ name, status: 'passed', seconds: (Date.now() - started) / 1000, ...details });
    console.log(`PASS ${name} ${JSON.stringify(details)}`);
  } catch (error) {
    results.push({ name, status: 'failed', seconds: (Date.now() - started) / 1000, error: String(error.stack || error) });
    console.error(`FAIL ${name}: ${error.message}`);
    process.exitCode = 1;
  }
  await writeFile(join(artifacts, `${process.env.RESULT_NAME || 'integration'}.json`), JSON.stringify({
    sourceSha256: process.env.TEST_SOURCE_SHA256 || null, at: new Date().toISOString(), platform: process.platform,
    arch: process.arch, node: process.version, uid: process.getuid(), results,
  }, null, 2));
}

async function withServer(name, shell, run) {
  const home = join(root, name);
  await mkdir(home, { recursive: true });
  const env = {
    ...process.env, HOME: home, SHELL: `/bin/${shell}`, ZDOTDIR: home,
    XDG_CONFIG_HOME: join(home, '.config'), XDG_CACHE_HOME: join(home, '.cache'),
    NPM_CONFIG_PREFIX: join(home, '.npm-global'), NPM_CONFIG_CACHE: join(home, '.npm-cache'),
    PATH: '/usr/local/bin:/usr/bin:/bin', INSTALLER_HOST: '127.0.0.1', INSTALLER_PORT: '3210',
    INSTALLER_TOKEN: randomBytes(24).toString('hex'), RUST_LOG: 'info',
  };
  // Never inherit model-provider credentials into test tools.
  for (const key of Object.keys(env)) {
    if (/^(ANTHROPIC|OPENAI|CLAUDE|CODEX)_/.test(key)) delete env[key];
  }
  const log = [];
  const server = spawn(binary, [], { env, stdio: ['ignore', 'pipe', 'pipe'] });
  let spawnError;
  const exited = new Promise((resolve) => {
    server.once('error', (error) => { spawnError = error; resolve(); });
    server.once('exit', resolve);
  });
  server.stdout.on('data', (data) => log.push(data));
  server.stderr.on('data', (data) => log.push(data));
  const base = 'http://127.0.0.1:3210';
  async function request(path, body, expected = 200, extraHeaders = {}) {
    const response = await fetch(`${base}${path}`, {
      method: body === undefined ? 'GET' : 'POST',
      headers: { Authorization: `Bearer ${env.INSTALLER_TOKEN}`, 'Content-Type': 'application/json', ...extraHeaders },
      body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(35_000),
    });
    const text = await response.text();
    assert.equal(response.status, expected, `${path}: ${text}`);
    return text && response.headers.get('content-type')?.includes('application/json') ? JSON.parse(text) : text;
  }
  try {
    const deadline = Date.now() + 30_000;
    while (true) {
      if (spawnError) throw spawnError;
      assert.equal(server.exitCode, null, 'Web server exited during startup');
      assert.equal(server.signalCode, null, 'Web server was killed during startup');
      try { await request('/api/install-tasks'); break; } catch (error) {
        if (Date.now() >= deadline) throw error;
        await sleep(150);
      }
    }
    return await run({ home, env, request, base });
  } finally {
    if (server.exitCode === null && server.signalCode === null && !spawnError) {
      server.kill('SIGTERM');
      const forced = setTimeout(() => server.kill('SIGKILL'), 5000);
      try { await exited; } finally { clearTimeout(forced); }
    }
    const text = Buffer.concat(log).toString().replaceAll(env.INSTALLER_TOKEN, '[test-token]');
    await writeFile(join(artifacts, `${name.replace(/[^a-z0-9-]/gi, '_')}.log`), text);
  }
}

async function finished(request, task) {
  const deadline = Date.now() + timeout;
  let previous = '';
  while (Date.now() < deadline) {
    const snapshot = await request('/api/install-tasks');
    assert.equal(snapshot.session_id, task.session_id);
    const current = snapshot.tasks.find((item) => item.task_id === task.task_id);
    assert.ok(current, 'Submitted task disappeared');
    const progress = `${current.status}/${current.progress?.mirror || ''}`;
    if (progress !== previous) { console.log(`  ${task.tool_id} ${progress}`); previous = progress; }
    if (['succeeded', 'failed'].includes(current.status)) return current;
    await sleep(1000);
  }
  throw new Error(`Install task ${task.task_id} exceeded ${timeout}ms`);
}

async function emptyStaging(home) {
  const stage = join(home, '.cache/ai-cli-installer/staging');
  try { assert.deepEqual(await readdir(stage), [], 'Installation left staging files'); }
  catch (error) { if (error.code !== 'ENOENT') throw error; }
}

if (group === 'all' || group === 'paths') {
  for (const shell of ['bash', 'zsh', 'fish']) {
    await caseRun(`path-${shell}`, () => withServer(`path-${shell}-it's $literal`, shell, async ({ home, env, request }) => {
      const rc = shell === 'fish' ? join(env.XDG_CONFIG_HOME, 'fish/conf.d/ai-cli-installer.fish')
        : join(home, shell === 'bash' ? '.bashrc' : '.zshrc');
      await mkdir(join(home, '.local/bin'), { recursive: true });
      await mkdir(join(env.XDG_CONFIG_HOME, 'fish/conf.d'), { recursive: true });
      const original = '# Keep personal configuration\n';
      await writeFile(rc, original, { mode: 0o640 });
      await writeFile(join(home, '.local/bin/path-probe'), '#!/bin/sh\nprintf path-ok\n', { mode: 0o755 });
      const body = { toolId: 'codex-cli', scope: 'user' };
      assert.equal((await request('/api/path/status?toolId=codex-cli')).in_user_path, false);
      await request('/api/path/add', body, 204);
      assert.equal((await request('/api/path/status?toolId=codex-cli')).in_user_path, true);
      const first = await readFile(rc, 'utf8');
      const firstStat = await stat(rc);
      await request('/api/path/add', body, 204);
      assert.equal(await readFile(rc, 'utf8'), first, 'Repeated add changed config');
      assert.equal((await stat(rc)).mtimeMs, firstStat.mtimeMs, 'No-op add rewrote config');
      assert.equal(firstStat.mode & 0o777, 0o640, 'Existing file permissions changed');
      assert.equal(await readFile(`${rc}.bak`, 'utf8'), original);
      const args = shell === 'bash' ? ['--noprofile', '--norc', '-c', '. "$1"; path-probe', '_', rc]
        : ['-c', 'path-probe'];
      // zsh reads .zshrc only in interactive mode; fish reads conf.d at startup.
      if (shell === 'zsh') args[0] = '-ic';
      const output = await exec(`/bin/${shell}`, args, { env, timeout: 10_000 });
      assert.equal(output.stdout.trim(), 'path-ok', 'New shell could not use configured PATH');
      await request('/api/path/remove', body, 204);
      assert.equal(await readFile(rc, 'utf8'), original);
      assert.equal((await request('/api/path/status?toolId=codex-cli')).in_user_path, false);
      const broken = '# >>> ai-cli-installer (PATH) >>>\n# Keep this incomplete block\n';
      await writeFile(rc, broken);
      await request('/api/path/add', body, 500);
      await request('/api/path/remove', body, 500);
      assert.equal(await readFile(rc, 'utf8'), broken, 'Malformed config was overwritten');
      return { shell, checks: ['add', 'idempotency', 'backup', 'permissions', 'fresh-shell', 'remove', 'malformed-preserved'] };
    }));
  }
}

if (group === 'all' || group === 'install') {
  for (const toolId of ['claude-code', 'codex-cli']) {
    for (const method of ['native', 'npm']) {
      const name = `${toolId}-${method}`;
      if (selected && selected !== name) continue;
      await caseRun(name, () => withServer(name, 'bash', async ({ home, env, request, base }) => {
        assert.equal((await fetch(`${base}/api/install-tasks`)).status, 401);
        await request('/api/install-tasks', { toolId: 'unknown' }, 400);
        await request('/api/install-tasks', { toolId, method: 'invalid' }, 400);
        await request('/api/install-tasks', { toolId }, 403, { Origin: 'https://untrusted.invalid' });
        await writeFile(join(home, '.claude.json'), JSON.stringify({ personalSetting: 'preserve' }), { mode: 0o600 });
        const invalid = await request('/api/install-tasks', { toolId, method, mirror: 'nonexistent-validation-mirror' });
        const failed = await finished(request, invalid);
        assert.equal(failed.status, 'failed');
        assert.match(failed.error, /镜像/);
        await emptyStaging(home);
        const body = { toolId, method, channel: 'latest' };
        const task = await request('/api/install-tasks', body);
        const duplicate = await request('/api/install-tasks', body);
        assert.equal(task.task_id, duplicate.task_id, 'Active install was duplicated');
        const done = await finished(request, task);
        assert.equal(done.status, 'succeeded', done.error);
        assert.equal(done.report.method, method);
        const installed = done.report.install_path;
        const rel = relative(home, installed);
        assert.ok(rel && !isAbsolute(rel) && rel !== '..' && !rel.startsWith('../'), 'Install escaped isolated HOME');
        const info = await stat(installed);
        assert.ok(info.isFile() && (info.mode & 0o111), 'Installed program is not executable');
        const { stdout } = await exec(installed, ['--version'], { env, timeout: 30_000 });
        const version = stdout.trim();
        assert.equal(version.match(/\d+\.\d+\.\d+(?:-[\w.-]+)?/)?.[0], done.report.version.replace(/^v/, ''), `Version mismatch: ${version}`);
        await emptyStaging(home);
        const detected = (await request('/api/tools')).find((tool) => tool.id === toolId);
        assert.equal(detected.installed_version, done.report.version, 'Tool detection missed installation');
        if (toolId === 'claude-code') {
          const config = JSON.parse(await readFile(join(home, '.claude.json'), 'utf8'));
          assert.equal(config.personalSetting, 'preserve');
          assert.equal(config.hasCompletedOnboarding, true);
          assert.equal((await stat(join(home, '.claude.json'))).mode & 0o777, 0o600);
        }
        let reinstalled = false;
        if (toolId === 'codex-cli' && method === 'native') {
          // Public API accepts channels, not a version in the channel field.
          // Reusing latest within the server's memo window should install the same version.
          const repeat = await request('/api/install-tasks', body);
          assert.notEqual(repeat.task_id, task.task_id);
          const repeated = await finished(request, repeat);
          assert.equal(repeated.status, 'succeeded', repeated.error);
          assert.equal(repeated.report.version, done.report.version);
          assert.equal((await exec(installed, ['--version'], { env, timeout: 30_000 })).stdout.trim(), version);
          await emptyStaging(home);
          reinstalled = true;
        }
        return { method, version, installed, reinstalled, checks: ['auth', 'bad-input', 'origin', 'failed-task', 'deduplication', 'real-install', 'version', 'detection', 'staging-cleanup'] };
      }));
    }
  }
}
assert.ok(results.length, 'No test cases selected');
console.log(`RESULT ${results.filter((item) => item.status === 'passed').length}/${results.length} passed; artifacts=${artifacts}`);
