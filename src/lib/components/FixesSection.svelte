<script lang="ts">
  import { onMount } from "svelte";
  import { listFixes, applyFixes, removeFixes, openPath } from "../api";
  import { open as openExternal } from "@tauri-apps/plugin-shell";
  import type { Fix } from "../types";
  import { ExternalLink, FileText } from "lucide-svelte";

  let fixes = $state<Fix[]>([]);
  let selected = $state<Set<string>>(new Set());
  let busy = $state(false);
  let message = $state<string | null>(null);
  let touchedFiles = $state<string[]>([]);
  let error = $state<string | null>(null);
  let loadError = $state<string | null>(null);
  let filter = $state<"all" | "configured" | "pending">("all");

  onMount(async () => {
    await loadFixes();
  });

  async function loadFixes() {
    try {
      fixes = await listFixes();
      selected = new Set(
        [...selected].filter((id) =>
          fixes.some((fix) => fix.id === id && !fix.configured)
        )
      );
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
    }
  }

  function toggle(fix: Fix) {
    if (fix.configured) return;
    const id = fix.id;
    if (selected.has(id)) selected.delete(id);
    else selected.add(id);
    selected = new Set(selected);
  }

  function filteredFixes(): Fix[] {
    if (filter === "configured") return fixes.filter((fix) => fix.configured);
    if (filter === "pending") return fixes.filter((fix) => !fix.configured);
    return fixes;
  }

  function configuredCount(): number {
    return fixes.filter((fix) => fix.configured).length;
  }

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

  async function apply() {
    if (selected.size === 0) return;
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
    await openExternal(url).catch(() => {});
  }

  async function openFile(path: string) {
    error = null;
    await openPath(path).catch((e) => {
      error = e instanceof Error ? e.message : String(e);
    });
  }
</script>

<section class="flex flex-col gap-3">
  <header>
    <h2 class="text-base font-semibold text-foreground">故障排查 / 配置补丁</h2>
    <p class="mt-1 text-xs text-muted-foreground leading-relaxed">
      勾选后点击「应用」会把对应字段写入对应配置文件，<strong class="text-foreground">保留</strong>已有内容。
      内容来源：<button
        class="text-primary hover:underline"
        onclick={() => openDoc('https://docs.openclaudecode.cn')}
      >OCC 配置文档</button>。
    </p>
  </header>

  {#if loadError}
    <div class="px-3 py-2 rounded-md text-xs bg-destructive/10 text-destructive">
      加载修复列表失败：{loadError}
    </div>
  {:else if fixes.length === 0}
    <div class="px-3 py-6 text-center text-xs text-muted-foreground border border-dashed border-border rounded-md">
      加载中…
    </div>
  {:else}
    <!-- Filters -->
    <div class="flex flex-wrap gap-1.5">
      {#each [{ k: "all", l: `全部 ${fixes.length}` }, { k: "configured", l: `已配置 ${configuredCount()}` }, { k: "pending", l: `未配置 ${fixes.length - configuredCount()}` }] as f}
        <button
          onclick={() => (filter = f.k as typeof filter)}
          class="px-2.5 py-1 text-xs rounded-md border transition-colors {filter === f.k
            ? 'border-primary bg-primary/10 text-primary'
            : 'border-border text-muted-foreground hover:bg-accent/50 hover:text-foreground'}"
        >
          {f.l}
        </button>
      {/each}
    </div>

    <!-- List -->
    <ul class="flex flex-col gap-2">
      {#each filteredFixes() as fix (fix.id)}
        <li
          class="border rounded-md transition-colors {fix.configured
            ? 'border-success/40 bg-success/5'
            : selected.has(fix.id)
              ? 'border-primary bg-primary/5'
              : 'border-border bg-card'}"
        >
          <label class="flex items-start gap-3 p-3 cursor-pointer">
            <input
              type="checkbox"
              checked={fix.configured || selected.has(fix.id)}
              onchange={() => toggle(fix)}
              disabled={busy || fix.configured}
              class="mt-0.5 accent-primary cursor-pointer disabled:cursor-not-allowed"
            />
            <div class="flex-1 min-w-0 flex flex-col gap-1.5">
              <div class="flex items-center justify-between gap-3">
                <div class="flex items-center gap-2 flex-wrap min-w-0">
                  <span class="font-mono text-[10px] px-1.5 py-0.5 rounded bg-muted text-muted-foreground">
                    {fix.code}
                  </span>
                  <span class="text-sm font-medium text-foreground">{fix.title}</span>
                  {#if fix.configured}
                    <span class="text-[10px] px-1.5 py-0.5 rounded bg-success/15 text-success font-semibold">
                      已配置
                    </span>
                  {:else if fix.total_patches > 0 && fix.configured_patches > 0}
                    <span class="text-[10px] px-1.5 py-0.5 rounded bg-warning/15 text-warning font-semibold">
                      已配置 {fix.configured_patches}/{fix.total_patches}
                    </span>
                  {/if}
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
          </label>
        </li>
      {/each}
    </ul>

    <!-- Action bar -->
    <div class="flex justify-between items-center pt-1">
      <span class="text-xs text-muted-foreground">已选 {selected.size} / {fixes.length}</span>
      <button
        disabled={busy || selected.size === 0}
        onclick={apply}
        class="px-3 py-1.5 text-xs rounded-md bg-primary text-primary-foreground hover:bg-primary/90 transition-colors disabled:opacity-50"
      >
        {busy ? "应用中…" : `应用选中的 ${selected.size} 项`}
      </button>
    </div>
  {/if}

  {#if message}
    <div class="px-3 py-2 rounded-md text-xs bg-success/10 text-success flex flex-col gap-1">
      <div>{message}</div>
      {#if touchedFiles.length > 0}
        <div class="flex flex-col gap-0.5 mt-0.5">
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
    <div class="px-3 py-2 rounded-md text-xs font-mono bg-destructive/10 text-destructive whitespace-pre-wrap break-words">
      {error}
    </div>
  {/if}
</section>
