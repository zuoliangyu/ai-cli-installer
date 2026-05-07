<script lang="ts">
  interface Props {
    downloaded: number;
    total: number | null;
    mirror?: string;
  }
  let { downloaded, total, mirror }: Props = $props();

  const formatBytes = (n: number): string => {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
    return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
  };

  let pct = $derived(total ? Math.min(100, (downloaded / total) * 100) : 0);
</script>

<div class="wrap">
  <div class="bar">
    {#if total}
      <div class="fill" style="width: {pct}%"></div>
    {:else}
      <div class="fill indeterminate"></div>
    {/if}
  </div>
  <div class="meta">
    <span>{formatBytes(downloaded)}{total ? ` / ${formatBytes(total)}` : ""}</span>
    {#if mirror}<span class="mirror">via {mirror}</span>{/if}
  </div>
</div>

<style>
  .wrap {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  .bar {
    height: 6px;
    background: var(--border);
    border-radius: 3px;
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: var(--accent);
    transition: width 0.15s linear;
  }
  .indeterminate {
    width: 30%;
    animation: slide 1.2s ease-in-out infinite;
  }
  @keyframes slide {
    0% { margin-left: -30%; }
    100% { margin-left: 100%; }
  }
  .meta {
    display: flex;
    justify-content: space-between;
    font-size: 0.8rem;
    color: var(--text-muted);
  }
  .mirror {
    font-family: ui-monospace, "SF Mono", Consolas, monospace;
  }
</style>
