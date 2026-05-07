<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import {
    installTool,
    onDownloadProgress,
    refreshTools,
    checkPathStatus,
    addToPath,
    removeFromPath,
  } from "../api";
  import type {
    ToolDescriptor,
    DownloadProgress,
    PathStatus,
    InstallMethod,
    ToolInstallation,
    InstallationSource,
  } from "../types";
  import ProgressBar from "./ProgressBar.svelte";

  interface Props {
    tool: ToolDescriptor;
  }
  let { tool }: Props = $props();

  let busy = $state(false);
  let pathBusy = $state(false);
  let progress = $state<DownloadProgress | null>(null);
  let message = $state<string | null>(null);
  let error = $state<string | null>(null);
  let pathStatus = $state<PathStatus | null>(null);
  let method = $state<InstallMethod>("native");
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onDownloadProgress((p) => {
      if (p.tool_id === tool.id) {
        progress = p;
      }
    });
    refreshPathStatus();
  });

  onDestroy(() => {
    unlisten?.();
  });

  async function refreshPathStatus() {
    try {
      pathStatus = await checkPathStatus(tool.id);
    } catch {
      pathStatus = null;
    }
  }

  async function handleInstall(channel: "latest" | "stable" = "latest") {
    busy = true;
    error = null;
    message = null;
    progress = null;
    try {
      const report = await installTool(tool.id, channel, method);
      const via = report.method === "npm" ? "npm" : "镜像";
      message = `已通过${via}安装 ${report.version} (${report.elapsed_secs}s)`;
      await refreshTools();
      await refreshPathStatus();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
      progress = null;
    }
  }

  async function handleAddPath() {
    pathBusy = true;
    error = null;
    message = null;
    try {
      await addToPath(tool.id, "system");
      message = "已加入系统 PATH。请重启终端或新开窗口生效。";
      await refreshPathStatus();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      pathBusy = false;
    }
  }

  async function handleRemovePath() {
    pathBusy = true;
    error = null;
    message = null;
    try {
      await removeFromPath(tool.id, "system");
      message = "已从系统 PATH 移除。";
      await refreshPathStatus();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      pathBusy = false;
    }
  }

  function channelLabel(channel: "latest" | "stable", version: string | null): string {
    const action = tool.installed_version ? "更新到" : "安装";
    return version ? `${action} ${channel} v${version}` : `${action} ${channel}`;
  }

  function sourceLabel(source: InstallationSource): string {
    if (source === "native") return "Native";
    if (source === "npm_global") return "npm 全局";
    return "PATH";
  }

  function sourceClass(source: InstallationSource): string {
    if (source === "native") return "native";
    if (source === "npm_global") return "npm";
    return "path";
  }

  function hasSource(source: InstallationSource): boolean {
    return (tool.installations ?? []).some((item) => item.source === source);
  }

  function nativeInstallation(): ToolInstallation | undefined {
    return (tool.installations ?? []).find((item) => item.source === "native");
  }

  function hasInstallationConflict(): boolean {
    const installs = tool.installations ?? [];
    const paths = new Set(
      installs
        .map((item) => item.path?.toLowerCase())
        .filter((path): path is string => Boolean(path))
    );
    const nonPathSources = new Set(
      installs
        .filter((item) => item.source !== "path")
        .map((item) => item.source)
    );
    return paths.size > 1 || nonPathSources.size > 1;
  }
</script>

<article class="card">
  <div class="head">
    <div class="meta">
      <h2>{tool.name}</h2>
      <p class="desc">{tool.description}</p>
      {#if tool.installed_version}
        <p class="installed">已安装 v{tool.installed_version}</p>
      {:else}
        <p class="not-installed">未安装</p>
      {/if}
    </div>
    <div class="actions">
      <button class="primary" onclick={() => handleInstall("latest")} disabled={busy}>
        {channelLabel("latest", tool.latest_version)}
      </button>
      <button class="primary" onclick={() => handleInstall("stable")} disabled={busy}>
        {channelLabel("stable", tool.stable_version)}
      </button>
    </div>
  </div>

  {#if tool.supports_npm}
    <div class="method-row">
      <span class="method-label">安装方式</span>
      <label>
        <input
          type="radio"
          name="method-{tool.id}"
          value="native"
          bind:group={method}
          disabled={busy}
        />
        镜像加速 (推荐)
      </label>
      <label>
        <input
          type="radio"
          name="method-{tool.id}"
          value="npm"
          bind:group={method}
          disabled={busy}
        />
        npm
        {#if tool.npm_package}
          <code class="pkg">{tool.npm_package}</code>
        {/if}
        {#if tool.npm_min_node}
          <span class="hint">需 Node ≥ {tool.npm_min_node}</span>
        {/if}
      </label>
    </div>
  {/if}

  {#if progress}
    <ProgressBar
      downloaded={progress.downloaded}
      total={progress.total}
      mirror={progress.mirror}
    />
  {/if}

  {#if tool.installations && tool.installations.length > 0}
    <div class="diagnostics">
      <div class="diagnostics-head">
        <span>安装诊断</span>
        {#if hasInstallationConflict()}
          <span class="badge warn">多重安装风险</span>
        {:else}
          <span class="badge ok">单一来源</span>
        {/if}
      </div>
      <div class="install-list">
        {#each tool.installations as item}
          <div class="install-row">
            <span class={`source ${sourceClass(item.source)}`}>{sourceLabel(item.source)}</span>
            <span class="install-version">{item.version ? `v${item.version}` : "版本未知"}</span>
            {#if item.current_path}
              <span class="badge ok">当前 PATH</span>
            {/if}
            {#if item.managed}
              <span class="badge ok">本应用路径</span>
            {/if}
            {#if item.path}
              <code title={item.path}>{item.path}</code>
            {/if}
          </div>
        {/each}
      </div>

      {#if hasInstallationConflict()}
        <div class="resolution">
          <strong>建议处理</strong>
          <p>先确认实际使用的来源，再保留一种安装方式；卸载前请备份相关配置。</p>
          {#if tool.npm_package && hasSource("npm_global")}
            <code>npm uninstall -g {tool.npm_package}</code>
          {/if}
          {#if nativeInstallation()?.path}
            <code>手动确认后删除 {nativeInstallation()?.path}</code>
          {/if}
        </div>
      {/if}
    </div>
  {:else}
    <div class="diagnostics empty-diagnostics">未检测到本机安装来源</div>
  {/if}

  {#if pathStatus}
    <div class="path-row">
      <div class="path-label">
        <span class="path-dir">{pathStatus.dir}</span>
        {#if pathStatus.in_system_path}
          <span class="badge ok">系统 PATH ✓</span>
        {:else if pathStatus.in_user_path}
          <span class="badge warn">仅用户 PATH</span>
        {:else if pathStatus.effective}
          <span class="badge warn">仅当前会话</span>
        {:else}
          <span class="badge err">未在 PATH</span>
        {/if}
      </div>
      <div class="path-actions">
        {#if pathStatus.in_system_path}
          <button onclick={handleRemovePath} disabled={pathBusy}>移除</button>
        {:else}
          <button class="primary" onclick={handleAddPath} disabled={pathBusy}>
            一键加入系统 PATH
          </button>
        {/if}
      </div>
    </div>
  {/if}

  {#if message}
    <div class="msg success">{message}</div>
  {/if}
  {#if error}
    <div class="msg error">{error}</div>
  {/if}
</article>

<style>
  .card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 1rem;
  }
  .meta {
    flex: 1;
    min-width: 0;
  }
  h2 {
    font-size: 1.05rem;
    font-weight: 600;
    margin-bottom: 0.25rem;
  }
  .desc {
    font-size: 0.85rem;
    color: var(--text-muted);
    margin-bottom: 0.4rem;
  }
  .installed {
    font-size: 0.8rem;
    color: var(--success);
  }
  .not-installed {
    font-size: 0.8rem;
    color: var(--text-muted);
  }
  .actions {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    flex-shrink: 0;
  }
  .actions button {
    white-space: nowrap;
  }
  .method-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.75rem;
    padding: 0.4rem 0.6rem;
    background: rgba(0, 0, 0, 0.03);
    border-radius: 6px;
    font-size: 0.78rem;
  }
  .method-label {
    color: var(--text-muted);
    font-weight: 500;
    margin-right: 0.2rem;
  }
  .method-row label {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    cursor: pointer;
  }
  .method-row input[type="radio"] {
    margin: 0;
    accent-color: var(--accent);
    cursor: pointer;
  }
  .method-row .pkg {
    font-family: ui-monospace, Consolas, monospace;
    font-size: 0.72rem;
    color: var(--text-muted);
    background: rgba(0, 0, 0, 0.05);
    padding: 0.05rem 0.3rem;
    border-radius: 3px;
  }
  .method-row .hint {
    color: var(--text-muted);
    font-size: 0.7rem;
  }
  .path-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.75rem;
    padding: 0.5rem 0.75rem;
    background: rgba(0, 0, 0, 0.03);
    border-radius: 6px;
    font-size: 0.85rem;
  }
  .path-label {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    min-width: 0;
  }
  .path-dir {
    font-family: ui-monospace, "SF Mono", Consolas, monospace;
    font-size: 0.75rem;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .badge {
    display: inline-block;
    padding: 0.1rem 0.4rem;
    border-radius: 3px;
    font-size: 0.7rem;
    font-weight: 500;
  }
  .badge.ok {
    background: rgba(22, 163, 74, 0.15);
    color: var(--success);
  }
  .badge.warn {
    background: rgba(217, 119, 6, 0.15);
    color: var(--warning);
  }
  .badge.err {
    background: rgba(220, 38, 38, 0.15);
    color: var(--error);
  }
  .path-actions {
    flex-shrink: 0;
  }
  .path-actions button {
    font-size: 0.78rem;
    padding: 0.3rem 0.6rem;
  }
  .diagnostics {
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.6rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    font-size: 0.8rem;
  }
  .empty-diagnostics {
    color: var(--text-muted);
  }
  .diagnostics-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    font-weight: 500;
  }
  .install-list {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }
  .install-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.35rem;
    min-width: 0;
  }
  .install-row code,
  .resolution code {
    font-family: ui-monospace, Consolas, monospace;
    font-size: 0.72rem;
    background: rgba(0, 0, 0, 0.05);
    border-radius: 3px;
    padding: 0.08rem 0.3rem;
    color: var(--text-muted);
    word-break: break-all;
  }
  .source {
    border-radius: 3px;
    padding: 0.08rem 0.35rem;
    font-size: 0.7rem;
    font-weight: 600;
  }
  .source.native {
    background: rgba(37, 99, 235, 0.12);
    color: #2563eb;
  }
  .source.npm {
    background: rgba(147, 51, 234, 0.12);
    color: #7e22ce;
  }
  .source.path {
    background: rgba(22, 163, 74, 0.12);
    color: var(--success);
  }
  .install-version {
    color: var(--text);
    font-weight: 500;
  }
  .resolution {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    padding-top: 0.45rem;
    border-top: 1px solid var(--border);
    color: var(--text-muted);
  }
  .resolution strong {
    color: var(--warning);
    font-size: 0.78rem;
  }
  .resolution p {
    line-height: 1.45;
  }
  .msg {
    padding: 0.5rem 0.75rem;
    border-radius: 4px;
    font-size: 0.85rem;
  }
  .msg.success {
    background: rgba(22, 163, 74, 0.1);
    color: var(--success);
  }
  .msg.error {
    background: rgba(220, 38, 38, 0.1);
    color: var(--error);
    word-break: break-word;
    white-space: pre-wrap;
    font-family: ui-monospace, Consolas, monospace;
    font-size: 0.78rem;
  }
</style>
