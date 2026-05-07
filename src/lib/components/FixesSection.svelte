<script lang="ts">
  import { onMount } from "svelte";
  import { listFixes, applyFixes, removeFixes, openPath } from "../api";
  import { open as openExternal } from "@tauri-apps/plugin-shell";
  import type { Fix } from "../types";

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
    if (selected.has(id)) {
      selected.delete(id);
    } else {
      selected.add(id);
    }
    selected = new Set(selected); // trigger reactivity
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

<section class="fixes">
  <header>
    <h2>故障排查 / 配置补丁</h2>
    <p class="hint">
      勾选后点击「应用」会把对应字段写入对应配置文件，<strong>保留</strong>已有内容。
      内容来源：<button class="link" onclick={() => openDoc('https://docs.openclaudecode.cn')}>OCC 配置文档</button>。
    </p>
  </header>

  {#if loadError}
    <div class="msg error">加载修复列表失败：{loadError}</div>
  {:else if fixes.length === 0}
    <div class="empty">加载中…</div>
  {:else}
    <div class="filters">
      <button class:active={filter === "all"} onclick={() => (filter = "all")}>
        全部 {fixes.length}
      </button>
      <button class:active={filter === "configured"} onclick={() => (filter = "configured")}>
        已配置 {configuredCount()}
      </button>
      <button class:active={filter === "pending"} onclick={() => (filter = "pending")}>
        未配置 {fixes.length - configuredCount()}
      </button>
    </div>

    <ul class="list">
      {#each filteredFixes() as fix (fix.id)}
        <li class="item" class:checked={selected.has(fix.id)} class:configured={fix.configured}>
          <label class="row">
            <input
              type="checkbox"
              checked={fix.configured || selected.has(fix.id)}
              onchange={() => toggle(fix)}
              disabled={busy || fix.configured}
            />
            <div class="content">
              <div class="title">
                <div class="title-main">
                  <span class="code">{fix.code}</span>
                  <span>{fix.title}</span>
                  {#if fix.configured}
                    <span class="configured-badge">已配置</span>
                  {:else if fix.total_patches > 0 && fix.configured_patches > 0}
                    <span class="partial-badge">
                      已配置 {fix.configured_patches}/{fix.total_patches}
                    </span>
                  {/if}
                </div>
                {#if fix.configured}
                  <button
                    class="remove-config"
                    disabled={busy}
                    onclick={(e) => {
                      e.preventDefault();
                      removeFix(fix);
                    }}
                  >
                    取消配置
                  </button>
                {/if}
              </div>
              <p class="desc">{fix.description}</p>
              <div class="patches">
                {#each fix.patches as p}
                  <div class="patch">
                    <span class="file">{targetLabel(p.target)}</span>
                    <span class="path">{p.path}</span>
                    <span class="eq">=</span>
                    <span class="val">{previewValue(p.value)}</span>
                  </div>
                {/each}
              </div>
              {#if fix.doc_url}
                <button class="link doc" onclick={(e) => { e.preventDefault(); openDoc(fix.doc_url); }}>
                  详细文档 →
                </button>
              {/if}
            </div>
          </label>
        </li>
      {/each}
    </ul>

    <div class="bar">
      <span class="count">已选 {selected.size} / {fixes.length}</span>
      <button class="primary" disabled={busy || selected.size === 0} onclick={apply}>
        {busy ? "应用中…" : `应用选中的 ${selected.size} 项`}
      </button>
    </div>
  {/if}

  {#if message}
    <div class="msg success">
      <div>{message}</div>
      {#if touchedFiles.length > 0}
        <div class="touched-files">
          {#each touchedFiles as path}
            <button class="file-link" onclick={() => openFile(path)} title={path}>
              {path}
            </button>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
  {#if error}<div class="msg error">{error}</div>{/if}
</section>

<style>
  .fixes {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  header h2 {
    font-size: 1rem;
    font-weight: 600;
  }
  .hint {
    font-size: 0.78rem;
    color: var(--text-muted);
    margin-top: 0.2rem;
  }
  .list {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .filters {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
  }
  .filters button {
    font-size: 0.78rem;
    padding: 0.3rem 0.65rem;
  }
  .filters button.active {
    background: var(--accent);
    border-color: var(--accent);
    color: white;
  }
  .item {
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-card);
    transition: border-color 0.15s, background 0.15s;
  }
  .item.checked {
    border-color: var(--accent);
    background: rgba(217, 119, 6, 0.05);
  }
  .item.configured {
    border-color: rgba(22, 163, 74, 0.28);
    background: rgba(22, 163, 74, 0.06);
  }
  .row {
    display: flex;
    gap: 0.6rem;
    padding: 0.7rem;
    cursor: pointer;
    align-items: flex-start;
  }
  .row input[type="checkbox"] {
    margin-top: 0.2rem;
    accent-color: var(--accent);
    cursor: pointer;
  }
  .content {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }
  .title {
    font-size: 0.9rem;
    font-weight: 500;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
  }
  .title-main {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
    min-width: 0;
  }
  .code {
    font-family: ui-monospace, Consolas, monospace;
    font-size: 0.7rem;
    background: rgba(0, 0, 0, 0.07);
    color: var(--text-muted);
    padding: 0.05rem 0.4rem;
    border-radius: 3px;
  }
  .configured-badge,
  .partial-badge {
    border-radius: 3px;
    padding: 0.05rem 0.35rem;
    font-size: 0.68rem;
    font-weight: 600;
  }
  .configured-badge {
    background: rgba(22, 163, 74, 0.14);
    color: var(--success);
  }
  .partial-badge {
    background: rgba(217, 119, 6, 0.14);
    color: var(--warning);
  }
  .remove-config {
    flex-shrink: 0;
    padding: 0.18rem 0.45rem;
    font-size: 0.72rem;
    color: var(--warning);
    border-color: rgba(217, 119, 6, 0.32);
    background: rgba(217, 119, 6, 0.08);
  }
  .remove-config:hover:not(:disabled) {
    color: white;
    border-color: var(--accent);
    background: var(--accent);
  }
  .desc {
    font-size: 0.78rem;
    color: var(--text-muted);
    line-height: 1.45;
  }
  .patches {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    margin-top: 0.2rem;
  }
  .patch {
    font-family: ui-monospace, Consolas, monospace;
    font-size: 0.72rem;
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
    align-items: baseline;
  }
  .patch .file {
    color: var(--text-muted);
  }
  .patch .path {
    color: var(--text);
    font-weight: 500;
  }
  .patch .eq {
    color: var(--text-muted);
  }
  .patch .val {
    color: var(--accent);
  }
  .link {
    background: none;
    border: none;
    padding: 0;
    color: var(--accent);
    cursor: pointer;
    font-size: inherit;
    font-family: inherit;
  }
  .link:hover {
    text-decoration: underline;
  }
  .doc {
    font-size: 0.75rem;
    align-self: flex-start;
  }
  .bar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding-top: 0.4rem;
  }
  .count {
    font-size: 0.8rem;
    color: var(--text-muted);
  }
  .empty {
    padding: 1rem;
    text-align: center;
    color: var(--text-muted);
    font-size: 0.85rem;
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
  .touched-files {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    margin-top: 0.12rem;
  }
  .file-link {
    align-self: flex-start;
    max-width: 100%;
    padding: 0;
    border: none;
    background: none;
    color: var(--success);
    font-family: ui-monospace, Consolas, monospace;
    font-size: 0.76rem;
    text-align: left;
    word-break: break-all;
  }
  .file-link:hover:not(:disabled) {
    text-decoration: underline;
    border-color: transparent;
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
