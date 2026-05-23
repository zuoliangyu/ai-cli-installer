<script lang="ts">
  import { onMount } from "svelte";
  import { RefreshCw, ArrowDownToLine, Trash2 } from "lucide-svelte";
  import { getLogs } from "../api";

  let lines = $state<string[]>([]);
  let loading = $state(false);
  let autoScroll = $state(true);
  let container: HTMLPreElement | undefined = $state();
  let timer: ReturnType<typeof setInterval> | undefined;

  async function refresh() {
    loading = true;
    try {
      lines = await getLogs();
    } catch {
      lines = ["[获取日志失败]"];
    } finally {
      loading = false;
    }
    if (autoScroll) scrollToBottom();
  }

  function scrollToBottom() {
    requestAnimationFrame(() => {
      if (container) container.scrollTop = container.scrollHeight;
    });
  }

  function clear() {
    lines = [];
  }

  onMount(() => {
    refresh();
    timer = setInterval(refresh, 3000);
    return () => {
      if (timer) clearInterval(timer);
    };
  });
</script>

<section class="flex flex-col h-full min-h-0 gap-3">
  <div class="flex items-center gap-2 shrink-0">
    <button
      onclick={refresh}
      disabled={loading}
      class="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border border-border bg-card hover:bg-accent/50 text-muted-foreground hover:text-foreground transition-colors disabled:opacity-50"
    >
      <RefreshCw class="w-3 h-3 {loading ? 'animate-spin' : ''}" />
      刷新
    </button>

    <button
      onclick={scrollToBottom}
      class="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border border-border bg-card hover:bg-accent/50 text-muted-foreground hover:text-foreground transition-colors"
    >
      <ArrowDownToLine class="w-3 h-3" />
      滚动到底部
    </button>

    <button
      onclick={clear}
      class="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border border-border bg-card hover:bg-accent/50 text-muted-foreground hover:text-foreground transition-colors"
    >
      <Trash2 class="w-3 h-3" />
      清除显示
    </button>

    <label class="ml-auto flex items-center gap-1.5 text-xs text-muted-foreground cursor-pointer select-none">
      <input
        type="checkbox"
        bind:checked={autoScroll}
        class="rounded border-border"
      />
      自动滚动
    </label>

    <span class="text-[11px] text-muted-foreground font-mono">{lines.length} 行</span>
  </div>

  <pre
    bind:this={container}
    class="flex-1 min-h-0 overflow-auto rounded-md border border-border bg-muted/30 p-3 text-[11px] leading-relaxed font-mono text-foreground whitespace-pre-wrap break-all"
  >{#if lines.length === 0}<span class="text-muted-foreground">暂无日志</span>{:else}{#each lines as line}{@html colorize(line)}
{/each}{/if}</pre>
</section>

<script lang="ts" module>
  function colorize(line: string): string {
    const esc = (s: string) =>
      s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
    const escaped = esc(line);

    if (escaped.includes(" ERROR "))
      return `<span class="text-destructive">${escaped}</span>`;
    if (escaped.includes("  WARN "))
      return `<span class="text-warning">${escaped}</span>`;
    if (escaped.includes(" DEBUG ") || escaped.includes(" TRACE "))
      return `<span class="text-muted-foreground">${escaped}</span>`;
    return escaped;
  }
</script>
