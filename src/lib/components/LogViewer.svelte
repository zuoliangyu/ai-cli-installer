<script lang="ts">
  import { onMount } from "svelte";
  import { RefreshCw, ArrowDownToLine } from "lucide-svelte";
  import { getLogs } from "../api";

  /** 本地最多保留的行数。 */
  const MAX_LINES = 5000;
  const POLL_MS = 3000;

  interface LogLine {
    /** 后端行序号，作为 {#each} 的稳定 key */
    seq: number;
    /** 已转义并着色的 HTML，每行只计算一次 */
    html: string;
  }

  let lines = $state.raw<LogLine[]>([]);
  let loading = $state(false);
  let fetchError = $state<string | null>(null);
  let autoScroll = $state(true);
  let container: HTMLPreElement | undefined = $state();
  /** 下次增量拉取传给后端的序号；null 表示拉取全部 */
  let next: number | null = null;

  async function refresh() {
    if (loading) return;
    loading = true;
    let changed = false;
    try {
      const chunk = await getLogs(next);
      const first = chunk.next - chunk.lines.length;
      const base = chunk.reset ? [] : lines;
      const lastSeq = base.length > 0 ? base[base.length - 1].seq : -Infinity;
      const added: LogLine[] = [];
      chunk.lines.forEach((text, i) => {
        const seq = first + i;
        // 防御：丢弃序号不递增的行，保证 key 唯一
        if (seq > lastSeq) added.push({ seq, html: colorize(text) });
      });
      if (chunk.reset || added.length > 0) {
        const merged = added.length > 0 ? base.concat(added) : base;
        lines = merged.length > MAX_LINES ? merged.slice(-MAX_LINES) : merged;
        changed = true;
      }
      next = chunk.next;
      fetchError = null;
    } catch (error) {
      fetchError = error instanceof Error ? error.message : String(error);
    } finally {
      loading = false;
    }
    if (changed && autoScroll) scrollToBottom();
  }

  function scrollToBottom() {
    requestAnimationFrame(() => {
      if (container) container.scrollTop = container.scrollHeight;
    });
  }

  onMount(() => {
    refresh();
    // 页面不可见（最小化、切到后台标签页）时暂停轮询，恢复可见时立即补拉一次。
    const timer = setInterval(() => {
      if (!document.hidden) refresh();
    }, POLL_MS);
    const onVisibility = () => {
      if (!document.hidden) refresh();
    };
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisibility);
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

  {#if fetchError}
    <div role="alert" class="shrink-0 px-3 py-2 rounded-md text-xs bg-destructive/10 text-destructive wrap-break-word">
      获取日志失败：{fetchError}
    </div>
  {/if}

  <pre
    bind:this={container}
    class="flex-1 min-h-0 overflow-auto rounded-md border border-border bg-muted/30 p-3 text-[11px] leading-relaxed font-mono text-foreground whitespace-pre-wrap break-all"
  >{#if lines.length === 0}<span class="text-muted-foreground">暂无日志</span>{:else}{#each lines as line (line.seq)}{@html line.html}
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
