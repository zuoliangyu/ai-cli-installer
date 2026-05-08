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
  import { CheckCircle2, AlertTriangle, XCircle, Package } from "lucide-svelte";

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
      if (p.tool_id === tool.id) progress = p;
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
    if (channel === "stable" && tool.stable_falls_back_to_latest) {
      return version
        ? `${action} stable (跟随 latest v${version})`
        : `${action} stable (跟随 latest)`;
    }
    return version ? `${action} ${channel} v${version}` : `${action} ${channel}`;
  }

  function sourceLabel(source: InstallationSource): string {
    if (source === "native") return "Native";
    if (source === "npm_global") return "npm 全局";
    return "PATH";
  }

  function sourceClass(source: InstallationSource): string {
    if (source === "native") return "bg-primary/15 text-primary";
    if (source === "npm_global") return "bg-purple-500/15 text-purple-500 dark:text-purple-300";
    return "bg-success/15 text-success";
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
      installs.filter((item) => item.source !== "path").map((item) => item.source)
    );
    return paths.size > 1 || nonPathSources.size > 1;
  }
</script>

<article class="bg-card border border-border rounded-lg p-4 flex flex-col gap-3">
  <!-- Head -->
  <div class="flex justify-between items-start gap-3">
    <div class="flex-1 min-w-0">
      <div class="flex items-center gap-2 mb-1">
        <Package class="w-4 h-4 text-primary shrink-0" />
        <h2 class="text-base font-semibold text-foreground">{tool.name}</h2>
      </div>
      <p class="text-xs text-muted-foreground mb-1.5">{tool.description}</p>
      {#if tool.installed_version}
        <p class="text-xs text-success flex items-center gap-1">
          <CheckCircle2 class="w-3 h-3" />
          已安装 v{tool.installed_version}
        </p>
      {:else}
        <p class="text-xs text-muted-foreground">未安装</p>
      {/if}
    </div>
    <div class="flex flex-col gap-1.5 shrink-0">
      <button
        onclick={() => handleInstall("latest")}
        disabled={busy}
        class="px-3 py-1.5 text-xs whitespace-nowrap rounded-md bg-primary text-primary-foreground hover:bg-primary/90 transition-colors disabled:opacity-50"
      >
        {channelLabel("latest", tool.latest_version)}
      </button>
      <button
        onclick={() => handleInstall("stable")}
        disabled={busy}
        class="px-3 py-1.5 text-xs whitespace-nowrap rounded-md border border-border bg-muted/50 text-foreground hover:bg-accent transition-colors disabled:opacity-50"
      >
        {channelLabel("stable", tool.stable_version)}
      </button>
    </div>
  </div>

  <!-- Method selector -->
  {#if tool.supports_npm}
    <div class="flex flex-wrap items-center gap-3 px-3 py-2 rounded-md bg-muted/40 text-xs">
      <span class="text-muted-foreground font-medium">安装方式</span>
      <label class="inline-flex items-center gap-1.5 cursor-pointer">
        <input
          type="radio"
          name="method-{tool.id}"
          value="native"
          bind:group={method}
          disabled={busy}
          class="accent-primary"
        />
        镜像加速 <span class="text-muted-foreground">(推荐)</span>
      </label>
      <label class="inline-flex items-center gap-1.5 cursor-pointer">
        <input
          type="radio"
          name="method-{tool.id}"
          value="npm"
          bind:group={method}
          disabled={busy}
          class="accent-primary"
        />
        npm
        {#if tool.npm_package}
          <code class="font-mono text-[10px] bg-muted px-1 py-0.5 rounded text-muted-foreground">
            {tool.npm_package}
          </code>
        {/if}
        {#if tool.npm_min_node}
          <span class="text-[10px] text-muted-foreground">需 Node ≥ {tool.npm_min_node}</span>
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

  <!-- Diagnostics -->
  {#if tool.installations && tool.installations.length > 0}
    <div class="border border-border rounded-md p-2.5 flex flex-col gap-2 text-xs">
      <div class="flex items-center justify-between gap-2 font-medium">
        <span>安装诊断</span>
        {#if hasInstallationConflict()}
          <span class="inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded bg-warning/15 text-warning font-semibold">
            <AlertTriangle class="w-3 h-3" />
            多重安装风险
          </span>
        {:else}
          <span class="inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded bg-success/15 text-success font-semibold">
            <CheckCircle2 class="w-3 h-3" />
            单一来源
          </span>
        {/if}
      </div>
      <div class="flex flex-col gap-1.5">
        {#each tool.installations as item}
          <div class="flex flex-wrap items-center gap-1.5 min-w-0">
            <span class="text-[10px] font-semibold px-1.5 py-0.5 rounded {sourceClass(item.source)}">
              {sourceLabel(item.source)}
            </span>
            <span class="text-foreground font-medium">
              {item.version ? `v${item.version}` : "版本未知"}
            </span>
            {#if item.current_path}
              <span class="text-[10px] px-1.5 py-0.5 rounded bg-success/15 text-success">当前 PATH</span>
            {/if}
            {#if item.managed}
              <span class="text-[10px] px-1.5 py-0.5 rounded bg-primary/15 text-primary">本应用路径</span>
            {/if}
            {#if item.path}
              <code
                class="font-mono text-[10px] px-1.5 py-0.5 rounded bg-muted text-muted-foreground break-all"
                title={item.path}
              >
                {item.path}
              </code>
            {/if}
          </div>
        {/each}
      </div>

      {#if hasInstallationConflict()}
        <div class="flex flex-col gap-1.5 pt-2 border-t border-border text-muted-foreground">
          <strong class="text-[11px] text-warning font-semibold">建议处理</strong>
          <p class="leading-relaxed">先确认实际使用的来源，再保留一种安装方式；卸载前请备份相关配置。</p>
          {#if tool.npm_package && hasSource("npm_global")}
            <code class="font-mono text-[10px] px-1.5 py-0.5 rounded bg-muted text-foreground break-all">
              npm uninstall -g {tool.npm_package}
            </code>
          {/if}
          {#if nativeInstallation()?.path}
            <code class="font-mono text-[10px] px-1.5 py-0.5 rounded bg-muted text-foreground break-all">
              手动确认后删除 {nativeInstallation()?.path}
            </code>
          {/if}
        </div>
      {/if}
    </div>
  {:else}
    <div class="border border-dashed border-border rounded-md px-3 py-2 text-xs text-muted-foreground">
      未检测到本机安装来源
    </div>
  {/if}

  <!-- PATH row -->
  {#if pathStatus}
    <div class="flex items-center justify-between gap-3 px-3 py-2 rounded-md bg-muted/40">
      <div class="flex flex-col gap-1 min-w-0">
        <code class="font-mono text-[11px] text-muted-foreground truncate" title={pathStatus.dir}>
          {pathStatus.dir}
        </code>
        <span>
          {#if pathStatus.in_system_path}
            <span class="inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded bg-success/15 text-success font-semibold">
              <CheckCircle2 class="w-3 h-3" />
              系统 PATH
            </span>
          {:else if pathStatus.in_user_path}
            <span class="inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded bg-warning/15 text-warning font-semibold">
              <AlertTriangle class="w-3 h-3" />
              仅用户 PATH
            </span>
          {:else if pathStatus.effective}
            <span class="inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded bg-warning/15 text-warning font-semibold">
              <AlertTriangle class="w-3 h-3" />
              仅当前会话
            </span>
          {:else}
            <span class="inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded bg-destructive/15 text-destructive font-semibold">
              <XCircle class="w-3 h-3" />
              未在 PATH
            </span>
          {/if}
        </span>
      </div>
      <div class="shrink-0">
        {#if pathStatus.in_system_path}
          <button
            onclick={handleRemovePath}
            disabled={pathBusy}
            class="px-2.5 py-1 text-xs rounded-md border border-border hover:bg-accent transition-colors disabled:opacity-50"
          >
            移除
          </button>
        {:else}
          <button
            onclick={handleAddPath}
            disabled={pathBusy}
            class="px-2.5 py-1 text-xs rounded-md bg-primary text-primary-foreground hover:bg-primary/90 transition-colors disabled:opacity-50"
          >
            一键加入系统 PATH
          </button>
        {/if}
      </div>
    </div>
  {/if}

  {#if message}
    <div class="px-3 py-2 rounded-md text-xs bg-success/10 text-success">{message}</div>
  {/if}
  {#if error}
    <div class="px-3 py-2 rounded-md text-xs font-mono bg-destructive/10 text-destructive whitespace-pre-wrap break-words">
      {error}
    </div>
  {/if}
</article>
