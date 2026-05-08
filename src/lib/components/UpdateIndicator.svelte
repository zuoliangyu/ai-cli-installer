<script lang="ts">
  import {
    ArrowDownToLine,
    ExternalLink,
    X,
    RefreshCw,
    CheckCircle2,
  } from "lucide-svelte";
  import {
    updateState,
    checkForUpdate,
    downloadAndInstall,
    openDownloadPage,
    dismiss,
  } from "../updateStore";

  let isChecking = $derived($updateState.status === "checking");
  let isWorking = $derived(
    $updateState.status === "downloading" || $updateState.status === "installing"
  );
  let hasUpdate = $derived(
    $updateState.status === "available" && !$updateState.dismissed
  );
</script>

<div class="flex flex-col gap-2.5">
  <!-- Header: current version + check button -->
  <div class="flex items-center justify-between gap-2">
    <div class="flex items-center gap-2 text-sm">
      <span class="text-muted-foreground">当前版本</span>
      <span class="font-mono text-foreground">
        v{$updateState.currentVersion || "..."}
      </span>
      {#if hasUpdate}
        <span class="relative flex h-2 w-2" aria-label="有新版本">
          <span
            class="animate-ping absolute inline-flex h-full w-full rounded-full bg-primary opacity-75"
          ></span>
          <span class="relative inline-flex rounded-full h-2 w-2 bg-primary"></span>
        </span>
      {/if}
    </div>
    <button
      onclick={() => checkForUpdate()}
      disabled={!__IS_TAURI__ || isChecking || isWorking}
      class="inline-flex items-center gap-1.5 px-2.5 py-1 text-xs rounded-md border border-border bg-muted/40 hover:bg-accent hover:text-foreground text-muted-foreground transition-colors disabled:opacity-50"
      title={__IS_TAURI__ ? "检查更新" : "Web 模式不支持自动更新"}
    >
      <RefreshCw class="w-3 h-3 {isChecking ? 'animate-spin' : ''}" />
      {isChecking ? "检查中…" : "检查更新"}
    </button>
  </div>

  {#if !__IS_TAURI__}
    <p class="text-xs text-muted-foreground">
      Web 模式下无法自动更新，请到
      <button
        onclick={() => openDownloadPage()}
        class="text-primary hover:underline">GitHub Releases</button
      >下载新版本桌面端。
    </p>
  {:else}
    <!-- States -->
    {#if $updateState.status === "idle" && !$updateState.newVersion}
      <div class="flex items-center gap-2 text-xs text-muted-foreground">
        <CheckCircle2 class="w-3.5 h-3.5 text-success shrink-0" />
        <span>已是最新版本</span>
      </div>
    {/if}

    {#if hasUpdate || ($updateState.status === "idle" && $updateState.newVersion)}
      <div class="rounded-md border border-primary/30 bg-primary/5 p-3 space-y-2">
        <div class="flex items-center justify-between gap-2">
          <span class="text-sm font-medium text-foreground">新版本可用</span>
          <button
            onclick={dismiss}
            class="text-muted-foreground hover:text-foreground transition-colors"
            title="忽略此版本"
          >
            <X class="w-3.5 h-3.5" />
          </button>
        </div>
        <div class="text-xs text-muted-foreground">
          v{$updateState.currentVersion} →
          <span class="text-primary font-medium">v{$updateState.newVersion}</span>
        </div>
        {#if $updateState.releaseNotes}
          <pre
            class="text-[11px] text-muted-foreground whitespace-pre-wrap leading-relaxed max-h-48 overflow-y-auto border-t border-border pt-2 font-mono">{$updateState.releaseNotes}</pre>
        {/if}
        <div class="flex gap-2">
          <button
            onclick={() => downloadAndInstall()}
            class="flex-1 flex items-center justify-center gap-1.5 px-2.5 py-1.5 rounded-md text-xs font-medium bg-primary text-primary-foreground hover:bg-primary/90 transition-colors"
          >
            <ArrowDownToLine class="w-3.5 h-3.5" />
            更新并重启
          </button>
          <button
            onclick={() => openDownloadPage()}
            class="inline-flex items-center justify-center gap-1.5 px-2.5 py-1.5 rounded-md text-xs border border-border bg-muted/40 text-foreground hover:bg-accent transition-colors"
            title="在浏览器查看 release"
          >
            <ExternalLink class="w-3.5 h-3.5" />
          </button>
        </div>
      </div>
    {/if}

    {#if $updateState.status === "downloading"}
      <div class="space-y-1.5">
        <div class="text-xs font-medium text-foreground">正在下载更新…</div>
        <div class="w-full bg-muted rounded-full h-1.5 overflow-hidden">
          <div
            class="bg-primary h-full transition-[width] duration-300"
            style="width: {$updateState.downloadProgress}%"
          ></div>
        </div>
        <div class="text-xs text-muted-foreground text-right font-mono">
          {$updateState.downloadProgress}%
        </div>
      </div>
    {/if}

    {#if $updateState.status === "installing"}
      <div class="flex items-center gap-2 text-xs text-muted-foreground">
        <RefreshCw class="w-3.5 h-3.5 animate-spin shrink-0" />
        <span>正在安装，即将重启…</span>
      </div>
    {/if}

    {#if $updateState.status === "error"}
      <div class="rounded-md border border-destructive/30 bg-destructive/5 p-2.5 space-y-1.5">
        <div class="text-xs font-medium text-destructive">更新检查失败</div>
        {#if $updateState.errorMessage}
          <div
            class="text-[11px] text-muted-foreground break-all line-clamp-2 font-mono"
          >
            {$updateState.errorMessage}
          </div>
        {/if}
        <div class="flex gap-2">
          <button
            onclick={() => checkForUpdate()}
            class="text-xs text-primary hover:underline"
          >
            重试
          </button>
          <button
            onclick={() => openDownloadPage()}
            class="text-xs text-muted-foreground hover:text-foreground"
          >
            手动到 GitHub 下载
          </button>
        </div>
      </div>
    {/if}
  {/if}
</div>
