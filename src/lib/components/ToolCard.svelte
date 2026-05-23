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

  /** 用户点了红色「获取版本失败 · 点此重试」按钮时调用。复用 busy 锁
   * 让其它按钮跟着禁用，避免重试期间用户又去点别的。 */
  async function handleRetry() {
    busy = true;
    error = null;
    message = null;
    try {
      await refreshTools();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
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
    const stale =
      channel === "latest" ? tool.latest_version_stale : tool.stable_version_stale;
    if (channel === "stable" && tool.stable_falls_back_to_latest) {
      const base = version
        ? `${action} stable (跟随 latest v${version})`
        : `${action} stable (跟随 latest)`;
      return stale ? `${base} · 缓存` : base;
    }
    if (!version) return `${action} ${channel}`;
    return stale ? `${action} ${channel} v${version} · 缓存` : `${action} ${channel} v${version}`;
  }

  function channelFailed(channel: "latest" | "stable"): boolean {
    return channel === "latest"
      ? tool.latest_version === null
      : tool.stable_version === null;
  }

  const RETRY_LABEL = "获取版本失败 · 点此重试";
  const STALE_HINT = "未能访问镜像，使用缓存版本号（可能已过期）";

  function sourceLabel(source: InstallationSource): string {
    if (source === "native") return "Native";
    if (source === "npm_global") return "npm 全局";
    if (source === "pnpm") return "pnpm 全局";
    if (source === "yarn") return "yarn 全局";
    if (source === "bun") return "bun 全局";
    if (source === "nvm") return "nvm";
    return "PATH";
  }

  function sourceClass(source: InstallationSource): string {
    if (source === "native") return "bg-primary/15 text-primary";
    if (source === "npm_global") return "bg-purple-500/15 text-purple-500 dark:text-purple-300";
    if (source === "pnpm") return "bg-amber-500/15 text-amber-600 dark:text-amber-300";
    if (source === "yarn") return "bg-blue-500/15 text-blue-600 dark:text-blue-300";
    if (source === "bun") return "bg-pink-500/15 text-pink-600 dark:text-pink-300";
    if (source === "nvm") return "bg-emerald-500/15 text-emerald-600 dark:text-emerald-300";
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

  function shadowedInstalls(): ToolInstallation[] {
    return (tool.installations ?? []).filter(
      (i) => !i.current_path && !i.on_path && Boolean(i.path)
    );
  }

  function hasNoCurrentPath(): boolean {
    const installs = tool.installations ?? [];
    return installs.length > 0 && installs.every((i) => !i.current_path);
  }

  /** launcher_dir 是「本应用的 native 安装目录」(~/.local/bin)。只有用户在用
   * native 安装、或还没装任何东西时才有意义；如果用户走的是 npm/pnpm/yarn/bun
   * 等其它 shim，把那行藏掉避免"加入系统 PATH"的按钮误导。 */
  function shouldShowLauncherPath(): boolean {
    const installs = tool.installations ?? [];
    if (installs.length === 0) return true;
    return Boolean(nativeInstallation());
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
      {#if channelFailed("latest")}
        <button
          onclick={handleRetry}
          disabled={busy}
          title="所有镜像 10 秒内都没拿到版本号，本地也没有缓存。点此重试拉取。"
          class="px-3 py-1.5 text-xs whitespace-nowrap rounded-md bg-destructive text-destructive-foreground hover:bg-destructive/90 transition-colors disabled:opacity-50"
        >
          {RETRY_LABEL}
        </button>
      {:else}
        <button
          onclick={() => handleInstall("latest")}
          disabled={busy}
          title={tool.latest_version_stale ? STALE_HINT : undefined}
          class="px-3 py-1.5 text-xs whitespace-nowrap rounded-md bg-primary text-primary-foreground hover:bg-primary/90 transition-colors disabled:opacity-50"
        >
          {channelLabel("latest", tool.latest_version)}
        </button>
      {/if}
      {#if channelFailed("stable")}
        <button
          onclick={handleRetry}
          disabled={busy}
          title="所有镜像 10 秒内都没拿到版本号，本地也没有缓存。点此重试拉取。"
          class="px-3 py-1.5 text-xs whitespace-nowrap rounded-md bg-destructive text-destructive-foreground hover:bg-destructive/90 transition-colors disabled:opacity-50"
        >
          {RETRY_LABEL}
        </button>
      {:else}
        <button
          onclick={() => handleInstall("stable")}
          disabled={busy}
          title={tool.stable_version_stale ? STALE_HINT : undefined}
          class="px-3 py-1.5 text-xs whitespace-nowrap rounded-md border border-border bg-muted/50 text-foreground hover:bg-accent transition-colors disabled:opacity-50"
        >
          {channelLabel("stable", tool.stable_version)}
        </button>
      {/if}
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
            {:else if item.on_path}
              <span class="text-[10px] px-1.5 py-0.5 rounded bg-warning/15 text-warning" title="目录在 PATH 中但被同名命令拦截">
                被遮蔽
              </span>
            {:else}
              <span class="text-[10px] px-1.5 py-0.5 rounded bg-destructive/15 text-destructive" title="该目录未加入 PATH，运行命令时找不到">
                未在 PATH
              </span>
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

      {#if hasNoCurrentPath()}
        <div class="flex flex-col gap-1.5 pt-2 border-t border-border">
          <strong class="text-[11px] text-destructive font-semibold">PATH 提示</strong>
          <p class="leading-relaxed text-muted-foreground">
            检测到 {tool.installations.length} 处安装，但都不在当前 PATH。终端运行
            <code class="font-mono bg-muted px-1 py-0.5 rounded text-foreground">{tool.id === "claude" ? "claude" : tool.id}</code>
            时会找不到命令；可使用上面的「一键加入系统 PATH」按钮，或选其中一处的目录手动加进 PATH。
          </p>
        </div>
      {:else if shadowedInstalls().length > 0}
        <div class="flex flex-col gap-1.5 pt-2 border-t border-border">
          <strong class="text-[11px] text-warning font-semibold">PATH 提示</strong>
          <p class="leading-relaxed text-muted-foreground">
            以下 {shadowedInstalls().length} 处安装的目录不在 PATH 中，调用时会被「当前 PATH」那一项接管：
          </p>
          <ul class="list-disc list-inside space-y-0.5">
            {#each shadowedInstalls() as item}
              <li class="text-muted-foreground">
                <span class="font-semibold">{sourceLabel(item.source)}</span>
                {#if item.version} v{item.version}{/if}
                — <code class="font-mono bg-muted px-1 py-0.5 rounded text-foreground break-all">{item.path}</code>
              </li>
            {/each}
          </ul>
        </div>
      {/if}

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

  <!-- PATH row（仅当与 native 安装相关） -->
  {#if pathStatus && shouldShowLauncherPath()}
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
