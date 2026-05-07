<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { installTool, onDownloadProgress, refreshTools } from "../api";
  import type { ToolDescriptor, DownloadProgress } from "../types";
  import ProgressBar from "./ProgressBar.svelte";

  interface Props {
    tool: ToolDescriptor;
  }
  let { tool }: Props = $props();

  let busy = $state(false);
  let progress = $state<DownloadProgress | null>(null);
  let message = $state<string | null>(null);
  let error = $state<string | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => {
    unlisten = await onDownloadProgress((p) => {
      if (p.tool_id === tool.id) {
        progress = p;
      }
    });
  });

  onDestroy(() => {
    unlisten?.();
  });

  async function handleInstall(channel: "latest" | "stable" = "latest") {
    busy = true;
    error = null;
    message = null;
    progress = null;
    try {
      const report = await installTool(tool.id, channel);
      message = `已安装 ${report.version} (${report.elapsed_secs}s)`;
      await refreshTools();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
      progress = null;
    }
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
        {tool.installed_version ? "更新到 latest" : "安装 latest"}
      </button>
      <button onclick={() => handleInstall("stable")} disabled={busy}>
        stable 通道
      </button>
    </div>
  </div>

  {#if progress}
    <ProgressBar
      downloaded={progress.downloaded}
      total={progress.total}
      mirror={progress.mirror}
    />
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
