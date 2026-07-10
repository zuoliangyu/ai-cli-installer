<script lang="ts">
  import { onMount } from "svelte";
  import { listFixes, applyFixes, removeFixes, openPath } from "../api";
  import { openExternalUrl } from "../openExternal";
  import type { Fix } from "../types";
  import { ExternalLink, FileText } from "lucide-svelte";

  let fixes = $state<Fix[]>([]);
  let selected = $state<Set<string>>(new Set());
  let busy = $state(false);
  let loading = $state(true);
  let message = $state<string | null>(null);
  let touchedFiles = $state<string[]>([]);
  let error = $state<string | null>(null);
  let loadError = $state<string | null>(null);
  let filter = $state<"all" | "configured" | "pending">("all");
  let currentPage = $state(1);
  let listViewport = $state<HTMLDivElement | null>(null);
  let tagPanelOpen = $state(false);
  let selectedTags = $state<Set<string>>(new Set());
  let query = $state("");

  const pageSize = 10;

  type FixTag = {
    label: string;
    tone: "primary" | "info" | "warning" | "danger" | "success" | "muted";
  };

  const TAG_TONE: Record<string, FixTag["tone"]> = {
    "推荐": "primary", "国内网络推荐": "primary",
    "Windows": "info", "MCP": "info", "PowerShell": "info",
    "终端": "warning", "网络": "warning",
    "安全": "danger",
    "隐私": "success",
    "搜索": "muted",
  };

  const TAG_CLASS: Record<FixTag["tone"], string> = {
    primary: "bg-primary/15 text-primary",
    info: "bg-blue-500/10 text-blue-600",
    warning: "bg-warning/15 text-warning",
    danger: "bg-destructive/10 text-destructive",
    success: "bg-success/15 text-success",
    muted: "bg-muted text-muted-foreground",
  };

  onMount(async () => {
    await loadFixes();
  });

  async function loadFixes() {
    loading = true;
    loadError = null;
    try {
      fixes = await listFixes();
      selected = new Set(
        [...selected].filter((id) =>
          fixes.some((fix) => fix.id === id && !fix.configured)
        )
      );
      clampPage();
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  function toggle(fix: Fix) {
    if (fix.configured) return;
    const id = fix.id;
    if (selected.has(id)) selected.delete(id);
    else selected.add(id);
    selected = new Set(selected);
  }

  const filteredFixes = $derived.by(() => {
    let list = fixes;
    if (filter === "configured") list = list.filter((f) => f.configured);
    if (filter === "pending") list = list.filter((f) => !f.configured);
    if (selectedTags.size > 0) {
      list = list.filter((f) => fixTags(f, false).some((t) => selectedTags.has(t.label)));
    }
    const needle = query.trim().toLocaleLowerCase();
    if (!needle) return list;
    return list.filter((fix) =>
      [
        fix.code,
        fix.title,
        fix.description,
        ...fix.tags,
        ...fix.patches.map((patch) => patch.path),
      ].some((text) => text.toLocaleLowerCase().includes(needle))
    );
  });
  const pageCount = $derived(Math.max(1, Math.ceil(filteredFixes.length / pageSize)));
  const pagedFixes = $derived(
    filteredFixes.slice((currentPage - 1) * pageSize, currentPage * pageSize)
  );

  function setFilter(next: typeof filter) {
    filter = next;
    currentPage = 1;
    scrollListTop();
  }

  function toggleTag(label: string) {
    if (selectedTags.has(label)) selectedTags.delete(label);
    else selectedTags.add(label);
    selectedTags = new Set(selectedTags);
    currentPage = 1;
    scrollListTop();
  }

  function clearTags() {
    selectedTags = new Set();
    currentPage = 1;
    scrollListTop();
  }

  function updateQuery(event: Event) {
    query = (event.currentTarget as HTMLInputElement).value;
    currentPage = 1;
  }

  function clearSelection() {
    selected = new Set();
  }

  function setPage(next: number) {
    currentPage = Math.min(Math.max(next, 1), pageCount);
    scrollListTop();
  }

  function clampPage() {
    currentPage = Math.min(currentPage, pageCount);
  }

  function scrollListTop() {
    listViewport?.scrollTo({ top: 0, behavior: "smooth" });
  }

  const configuredCount = $derived(fixes.filter((f) => f.configured).length);

  function targetLabel(target: string): string {
    if (target === "claude_settings") return "~/.claude/settings.json";
    if (target === "claude_json") return "~/.claude.json";
    return target;
  }

  function previewValue(v: unknown): string {
    if (typeof v === "string") return JSON.stringify(v);
    if (typeof v === "boolean") return v ? "true" : "false";
    if (v === null) return "null";
    return JSON.stringify(v);
  }

  function fixTags(fix: Fix, limit = true): FixTag[] {
    const tags = fix.tags.map((label) => ({
      label,
      tone: (TAG_TONE[label] ?? "muted") as FixTag["tone"],
    }));
    return limit ? tags.slice(0, 3) : tags;
  }

  const tagOptions = $derived.by(() => {
    const options = new Map<string, FixTag & { count: number }>();
    for (const fix of fixes) {
      for (const tag of fixTags(fix, false)) {
        const current = options.get(tag.label);
        if (current) current.count += 1;
        else options.set(tag.label, { ...tag, count: 1 });
      }
    }
    return [...options.values()];
  });

  function tagClass(tone: FixTag["tone"]): string {
    return `text-[10px] px-1.5 py-0.5 rounded font-semibold ${TAG_CLASS[tone]}`;
  }

  async function apply() {
    if (selected.size === 0) return;
    if (!window.confirm(`将应用 ${selected.size} 项配置。同名字段会被覆盖，写入前会生成 .bak 递增备份。是否继续？`)) return;
    busy = true;
    error = null;
    message = null;
    touchedFiles = [];
    try {
      const ids = [...selected];
      const report = await applyFixes(ids);
      message = `已应用 ${report.applied_count} 个修复，写入 ${report.touched_files.length} 个文件`;
      touchedFiles = report.touched_files;
      selected = new Set();
      await loadFixes();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  async function removeFix(fix: Fix) {
    if (!fix.configured) return;
    if (!window.confirm(`将移除“${fix.title}”对应的配置项，写入前会生成 .bak 递增备份。是否继续？`)) return;
    busy = true;
    error = null;
    message = null;
    touchedFiles = [];
    try {
      const report = await removeFixes([fix.id]);
      message = `已取消 ${report.removed_count} 项配置，写入 ${report.touched_files.length} 个文件`;
      touchedFiles = report.touched_files;
      await loadFixes();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  async function openDoc(url: string | null) {
    if (!url) return;
    await openExternalUrl(url).catch(() => {});
  }

  async function openFile(path: string) {
    error = null;
    await openPath(path).catch((e) => {
      error = e instanceof Error ? e.message : String(e);
    });
  }
</script>

<section class="flex-1 min-h-0 flex flex-col gap-3">
  <header class="shrink-0">
    <h2 class="text-base font-semibold text-foreground">故障排查 / 配置补丁</h2>
    <p class="mt-1 text-xs text-muted-foreground leading-relaxed">
      勾选后点击「应用」会写入对应配置文件；其他字段保留，同名字段会覆盖，原文件保留为 <code class="font-mono">.bak*</code> 递增备份。
    </p>
  </header>

  {#if loadError}
    <div role="alert" class="px-3 py-2 rounded-md text-xs bg-destructive/10 text-destructive">
      <div>加载修复列表失败：{loadError}</div>
      <button onclick={loadFixes} class="mt-1 text-primary hover:underline">重试</button>
    </div>
  {:else if loading}
    <div class="px-3 py-6 text-center text-xs text-muted-foreground border border-dashed border-border rounded-md">
      加载中…
    </div>
  {:else if fixes.length === 0}
    <div class="px-3 py-6 text-center text-xs text-muted-foreground border border-dashed border-border rounded-md">
      暂无配置修复。
    </div>
  {:else}
    <!-- Filters -->
    <div class="shrink-0 flex flex-col gap-2">
      <input
        type="search"
        value={query}
        oninput={updateQuery}
        aria-label="搜索配置修复"
        placeholder="搜索标题、编号、说明、标签或配置项"
        class="w-full rounded-md border border-border bg-background px-3 py-1.5 text-xs text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-primary"
      />
      <div class="flex flex-wrap gap-1.5">
        {#each [{ k: "all", l: `全部 ${fixes.length}` }, { k: "configured", l: `已配置 ${configuredCount}` }, { k: "pending", l: `未配置 ${fixes.length - configuredCount}` }] as f}
          <button
            onclick={() => setFilter(f.k as typeof filter)}
            aria-pressed={filter === f.k}
            class="px-2.5 py-1 text-xs rounded-md border transition-colors {filter === f.k
              ? 'border-primary bg-primary/10 text-primary'
              : 'border-border text-muted-foreground hover:bg-accent/50 hover:text-foreground'}"
          >
            {f.l}
          </button>
        {/each}
        <button
          onclick={() => (tagPanelOpen = !tagPanelOpen)}
          aria-expanded={tagPanelOpen}
          class="px-2.5 py-1 text-xs rounded-md border transition-colors {tagPanelOpen || selectedTags.size > 0
            ? 'border-primary bg-primary/10 text-primary'
            : 'border-border text-muted-foreground hover:bg-accent/50 hover:text-foreground'}"
        >
          标签筛选{selectedTags.size > 0 ? ` ${selectedTags.size}` : ""}
        </button>
        {#if selectedTags.size > 0}
          <button
            onclick={clearTags}
            class="px-2.5 py-1 text-xs rounded-md border border-border text-muted-foreground hover:bg-accent/50 hover:text-foreground"
          >
            清除标签
          </button>
        {/if}
      </div>

      {#if tagPanelOpen}
        <div class="rounded-md border border-border bg-muted/30 p-2">
          <div class="flex flex-wrap gap-1.5">
            {#each tagOptions as tag}
              <label
                class="inline-flex items-center gap-1.5 px-2 py-1 rounded-md border text-xs cursor-pointer transition-colors {selectedTags.has(tag.label)
                  ? 'border-primary bg-primary/10 text-primary'
                  : 'border-border bg-background text-muted-foreground hover:text-foreground hover:bg-accent/50'}"
              >
                <input
                  type="checkbox"
                  checked={selectedTags.has(tag.label)}
                  onchange={() => toggleTag(tag.label)}
                  class="accent-primary"
                />
                <span class={tagClass(tag.tone)}>{tag.label}</span>
                <span class="text-[10px] text-muted-foreground">{tag.count}</span>
              </label>
            {/each}
          </div>
        </div>
      {/if}
    </div>

    <!-- List -->
    <div bind:this={listViewport} class="min-h-0 flex-1 overflow-y-auto pr-1">
      <ul class="flex flex-col gap-2">
        {#each pagedFixes as fix (fix.id)}
          {@const tags = fixTags(fix)}
          {@const hasStatus = fix.configured || (fix.total_patches > 0 && fix.configured_patches > 0)}
          <li
            class="border rounded-md transition-colors {fix.configured
              ? 'border-success/40 bg-success/5'
              : selected.has(fix.id)
                ? 'border-primary bg-primary/5'
                : 'border-border bg-card'}"
          >
            <div class="flex items-start gap-3 p-3">
              <input
                type="checkbox"
                checked={fix.configured || selected.has(fix.id)}
                onchange={() => toggle(fix)}
                disabled={busy || fix.configured}
                aria-label={fix.configured ? `${fix.title} 已配置` : `选择 ${fix.title}`}
                class="mt-0.5 accent-primary cursor-pointer disabled:cursor-not-allowed"
              />
              <div class="flex-1 min-w-0 flex flex-col gap-1.5">
                <div class="flex items-center justify-between gap-3">
                  <div class="flex items-center gap-2 flex-wrap min-w-0">
                    <span class="font-mono text-[10px] px-1.5 py-0.5 rounded bg-muted text-muted-foreground">
                      {fix.code}
                    </span>
                    <span class="text-[15px] font-semibold text-foreground">{fix.title}</span>
                    {#if fix.configured}
                      <span class="text-[10px] px-1.5 py-0.5 rounded bg-success/15 text-success font-semibold">
                        已配置
                      </span>
                    {:else if fix.total_patches > 0 && fix.configured_patches > 0}
                      <span class="text-[10px] px-1.5 py-0.5 rounded bg-warning/15 text-warning font-semibold">
                        已配置 {fix.configured_patches}/{fix.total_patches}
                      </span>
                    {/if}
                    {#if hasStatus && tags.length > 0}
                      <span class="text-[10px] text-muted-foreground/60">|</span>
                    {/if}
                    {#each tags as tag}
                      <span class={tagClass(tag.tone)}>{tag.label}</span>
                    {/each}
                  </div>
                  {#if fix.configured}
                    <button
                      disabled={busy}
                      onclick={(e) => {
                        e.preventDefault();
                        removeFix(fix);
                      }}
                      class="shrink-0 px-2 py-0.5 text-[11px] rounded border border-warning/30 bg-warning/10 text-warning hover:bg-warning hover:text-warning-foreground transition-colors disabled:opacity-50"
                    >
                      取消配置
                    </button>
                  {/if}
                </div>
                <p class="text-xs text-muted-foreground leading-relaxed">{fix.description}</p>
                <div class="flex flex-col gap-0.5">
                  {#each fix.patches as p}
                    <div class="font-mono text-[11px] flex flex-wrap gap-1.5 items-baseline">
                      <span class="text-muted-foreground">{targetLabel(p.target)}</span>
                      <span class="text-foreground font-medium">{p.path}</span>
                      <span class="text-muted-foreground">=</span>
                      <span class="text-primary">{previewValue(p.value)}</span>
                    </div>
                  {/each}
                </div>
                {#if fix.doc_url}
                  <button
                    class="inline-flex items-center gap-1 text-[11px] text-primary hover:underline self-start"
                    onclick={(e) => {
                      e.preventDefault();
                      openDoc(fix.doc_url);
                    }}
                  >
                    详细文档
                    <ExternalLink class="w-3 h-3" />
                  </button>
                {/if}
              </div>
            </div>
          </li>
        {/each}
      </ul>
      {#if pagedFixes.length === 0}
        <div class="px-3 py-6 text-center text-xs text-muted-foreground">
          没有匹配的配置修复。
        </div>
      {/if}
    </div>
  {/if}

  {#if fixes.length > 0 && !loadError}
    <footer class="shrink-0 border-t border-border pt-3 flex flex-col gap-2 bg-background">
      <div class="flex flex-wrap items-center justify-between gap-2">
        <div class="flex items-center gap-2 text-xs text-muted-foreground">
          <span>已选 {selected.size} 项 · 当前结果 {filteredFixes.length} 项</span>
          {#if selected.size > 0}
            <button onclick={clearSelection} class="text-primary hover:underline">清空已选</button>
          {/if}
          {#if filteredFixes.length > pageSize}
            <span>第 {currentPage} / {pageCount} 页</span>
          {/if}
        </div>
        <div class="flex items-center gap-2">
          {#if filteredFixes.length > pageSize}
            <button
              disabled={currentPage === 1}
              onclick={() => setPage(currentPage - 1)}
              class="px-2 py-1 text-xs rounded-md border border-border text-muted-foreground hover:bg-accent/50 hover:text-foreground disabled:opacity-50"
            >
              上一页
            </button>
            <button
              disabled={currentPage === pageCount}
              onclick={() => setPage(currentPage + 1)}
              class="px-2 py-1 text-xs rounded-md border border-border text-muted-foreground hover:bg-accent/50 hover:text-foreground disabled:opacity-50"
            >
              下一页
            </button>
          {/if}
          <button
            disabled={busy || selected.size === 0}
            onclick={apply}
            class="px-3 py-1.5 text-xs rounded-md bg-primary text-primary-foreground hover:bg-primary/90 transition-colors disabled:opacity-50"
          >
            {busy ? "应用中…" : `应用选中的 ${selected.size} 项`}
          </button>
        </div>
      </div>
      {#if message}
        <div role="status" class="px-3 py-2 rounded-md text-xs bg-success/10 text-success flex flex-col gap-1">
          <div>{message}</div>
          {#if touchedFiles.length > 0}
            <div class="flex flex-col gap-0.5">
              {#each touchedFiles as path}
                <button
                  onclick={() => openFile(path)}
                  title={path}
                  class="inline-flex items-center gap-1 self-start font-mono text-[11px] text-success hover:underline break-all text-left"
                >
                  <FileText class="w-3 h-3 shrink-0" />
                  {path}
                </button>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
      {#if error}
        <div role="alert" class="px-3 py-2 rounded-md text-xs font-mono bg-destructive/10 text-destructive whitespace-pre-wrap break-words">
          {error}
        </div>
      {/if}
    </footer>
  {/if}
</section>

