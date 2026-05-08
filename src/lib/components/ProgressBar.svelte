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

<div class="flex flex-col gap-1">
  <div class="h-1.5 bg-muted rounded-full overflow-hidden">
    {#if total}
      <div class="h-full bg-primary transition-[width] duration-150" style="width: {pct}%"></div>
    {:else}
      <div class="h-full w-1/3 bg-primary indeterminate"></div>
    {/if}
  </div>
  <div class="flex justify-between text-[11px] text-muted-foreground">
    <span class="font-mono">
      {formatBytes(downloaded)}{total ? ` / ${formatBytes(total)}` : ""}
    </span>
    {#if mirror}<span class="font-mono">via {mirror}</span>{/if}
  </div>
</div>

<style>
  .indeterminate {
    animation: slide 1.2s ease-in-out infinite;
  }
  @keyframes slide {
    0% {
      margin-left: -33%;
    }
    100% {
      margin-left: 100%;
    }
  }
</style>
