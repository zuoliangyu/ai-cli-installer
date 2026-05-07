<script lang="ts">
  import { onMount } from "svelte";
  import {
    listClaudePresets,
    getClaudeSettings,
    applyClaudePreset,
  } from "../api";
  import { open as openExternal } from "@tauri-apps/plugin-shell";
  import type { ClaudePreset, ClaudeSettingsEnv } from "../types";

  let presets = $state<ClaudePreset[]>([]);
  let currentEnv = $state<ClaudeSettingsEnv | null>(null);
  let selectedPreset = $state<ClaudePreset | null>(null);
  let apiKey = $state("");
  let busy = $state(false);
  let message = $state<string | null>(null);
  let error = $state<string | null>(null);

  onMount(refresh);

  async function refresh() {
    try {
      presets = await listClaudePresets();
      currentEnv = await getClaudeSettings();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  function isActive(p: ClaudePreset): boolean {
    if (!currentEnv?.anthropic_base_url) return false;
    return (
      currentEnv.anthropic_base_url.replace(/\/+$/, "") ===
      p.base_url.replace(/\/+$/, "")
    );
  }

  function pick(p: ClaudePreset) {
    selectedPreset = p;
    apiKey = "";
    error = null;
    message = null;
  }

  function cancel() {
    selectedPreset = null;
    apiKey = "";
  }

  async function apply() {
    if (!selectedPreset || !apiKey.trim()) return;
    busy = true;
    error = null;
    message = null;
    try {
      await applyClaudePreset(selectedPreset.base_url, apiKey.trim());
      message = `已写入 ${selectedPreset.name} 的配置到 ~/.claude/settings.json`;
      selectedPreset = null;
      apiKey = "";
      await refresh();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  async function openPresetUrl(p: ClaudePreset) {
    const url = p.api_key_url ?? p.website_url;
    if (url) await openExternal(url).catch(() => {});
  }
</script>

<section class="presets">
  <header>
    <h2>中转站快捷配置</h2>
    <p class="hint">
      只写入 <code>~/.claude/settings.json</code> 的
      <code>env.ANTHROPIC_BASE_URL</code> 与
      <code>env.ANTHROPIC_AUTH_TOKEN</code>，其它字段保留不动
    </p>
  </header>

  {#if currentEnv?.anthropic_base_url}
    <div class="current">
      当前激活：<span class="url">{currentEnv.anthropic_base_url}</span>
    </div>
  {/if}

  <div class="grid">
    {#each presets as p (p.id)}
      <article class="card" class:active={isActive(p)}>
        <div class="head">
          <span class="name">{p.name}</span>
          {#if isActive(p)}<span class="badge">使用中</span>{/if}
          {#if p.source === "cc_switch"}<span class="src">cc-switch</span>{/if}
        </div>
        <button class="link" onclick={() => openPresetUrl(p)}>
          {p.base_url}
        </button>
        <div class="actions">
          <button class="primary" onclick={() => pick(p)} disabled={busy}>
            {isActive(p) ? "更换 Key" : "使用此预设"}
          </button>
        </div>
      </article>
    {/each}
  </div>

  {#if presets.length === 0}
    <div class="empty">暂无可用预设。</div>
  {/if}

  {#if message}<div class="msg success">{message}</div>{/if}
  {#if error}<div class="msg error">{error}</div>{/if}
</section>

{#if selectedPreset}
  <div
    class="backdrop"
    onclick={cancel}
    role="presentation"
  >
    <div
      class="modal"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => e.stopPropagation()}
      role="dialog"
      aria-modal="true"
      tabindex="-1"
    >
      <h3>{selectedPreset.name}</h3>
      <p class="modal-base">
        Base URL: <code>{selectedPreset.base_url}</code>
      </p>
      <label>
        <span>API Key</span>
        <input
          type="password"
          bind:value={apiKey}
          placeholder="粘贴该中转站的 API key"
          autocomplete="off"
          disabled={busy}
        />
      </label>
      {#if selectedPreset.api_key_url}
        <button class="link" onclick={() => openPresetUrl(selectedPreset!)}>
          没有 key？前往 {selectedPreset.api_key_url} 获取 →
        </button>
      {/if}
      <div class="modal-actions">
        <button onclick={cancel} disabled={busy}>取消</button>
        <button class="primary" onclick={apply} disabled={busy || !apiKey.trim()}>
          {busy ? "应用中…" : "应用"}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .presets {
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
  .hint code {
    font-family: ui-monospace, Consolas, monospace;
    font-size: 0.72rem;
    background: rgba(0, 0, 0, 0.05);
    padding: 0.05rem 0.25rem;
    border-radius: 3px;
  }
  .current {
    font-size: 0.8rem;
    padding: 0.4rem 0.6rem;
    background: rgba(22, 163, 74, 0.08);
    border-radius: 6px;
    color: var(--success);
  }
  .current .url {
    font-family: ui-monospace, Consolas, monospace;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 0.5rem;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    padding: 0.7rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-card);
  }
  .card.active {
    border-color: var(--success);
  }
  .card .head {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }
  .name {
    font-weight: 500;
    flex: 1;
  }
  .badge {
    font-size: 0.65rem;
    padding: 0.1rem 0.3rem;
    background: var(--success);
    color: white;
    border-radius: 3px;
  }
  .src {
    font-size: 0.65rem;
    color: var(--text-muted);
  }
  .link {
    background: none;
    border: none;
    padding: 0;
    color: var(--accent);
    cursor: pointer;
    text-align: left;
    font-size: 0.78rem;
    font-family: ui-monospace, Consolas, monospace;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .link:hover {
    text-decoration: underline;
  }
  .actions {
    margin-top: 0.2rem;
  }
  .actions button {
    width: 100%;
    font-size: 0.8rem;
    padding: 0.3rem 0.4rem;
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
  .msg.error {
    background: rgba(220, 38, 38, 0.1);
    color: var(--error);
    word-break: break-word;
    white-space: pre-wrap;
  }
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
    backdrop-filter: blur(2px);
  }
  .modal {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1.5rem;
    width: 90%;
    max-width: 420px;
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.25);
    display: flex;
    flex-direction: column;
    gap: 0.8rem;
  }
  .modal h3 {
    font-size: 1.05rem;
    font-weight: 600;
  }
  .modal-base {
    font-size: 0.8rem;
    color: var(--text-muted);
  }
  .modal-base code {
    font-family: ui-monospace, Consolas, monospace;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    font-size: 0.85rem;
  }
  input {
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0.4rem 0.55rem;
    background: var(--bg);
    color: var(--text);
    font-family: ui-monospace, Consolas, monospace;
    font-size: 0.85rem;
  }
  input:focus {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
  }
  .modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-top: 0.4rem;
  }
</style>
