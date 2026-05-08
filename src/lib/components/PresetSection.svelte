<script lang="ts">
  import { onMount } from "svelte";
  import {
    listClaudePresets,
    getClaudeSettings,
    applyClaudePreset,
  } from "../api";
  import { open as openExternal } from "@tauri-apps/plugin-shell";
  import type { ClaudePreset, ClaudeSettingsEnv } from "../types";
  import { CheckCircle2, ExternalLink, X } from "lucide-svelte";

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

<section class="flex flex-col gap-3">
  <header>
    <h2 class="text-base font-semibold text-foreground">中转站快捷配置</h2>
    <p class="mt-1 text-xs text-muted-foreground leading-relaxed">
      只写入 <code class="font-mono text-[11px] bg-muted px-1 py-0.5 rounded">~/.claude/settings.json</code>
      的 <code class="font-mono text-[11px] bg-muted px-1 py-0.5 rounded">env.ANTHROPIC_BASE_URL</code> 与
      <code class="font-mono text-[11px] bg-muted px-1 py-0.5 rounded">env.ANTHROPIC_AUTH_TOKEN</code>，
      其它字段保留不动
    </p>
  </header>

  {#if currentEnv?.anthropic_base_url}
    <div class="flex items-center gap-2 px-3 py-2 rounded-md bg-success/10 text-xs text-success">
      <CheckCircle2 class="w-3.5 h-3.5 shrink-0" />
      <span>当前激活：</span>
      <span class="font-mono truncate">{currentEnv.anthropic_base_url}</span>
    </div>
  {/if}

  <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-2.5">
    {#each presets as p (p.id)}
      <article
        class="flex flex-col gap-2 p-3 rounded-md border bg-card transition-colors {isActive(p)
          ? 'border-success/60 bg-success/5'
          : 'border-border'}"
      >
        <div class="flex items-center gap-2">
          <span class="font-medium text-sm text-foreground flex-1 truncate">{p.name}</span>
          {#if isActive(p)}
            <span class="text-[10px] px-1.5 py-0.5 rounded bg-success text-success-foreground font-semibold">
              使用中
            </span>
          {/if}
          {#if p.source === "cc_switch"}
            <span class="text-[10px] text-muted-foreground">cc-switch</span>
          {/if}
        </div>
        <button
          onclick={() => openPresetUrl(p)}
          class="text-left text-[11px] font-mono text-primary hover:underline truncate"
          title={p.base_url}
        >
          {p.base_url}
        </button>
        <button
          onclick={() => pick(p)}
          disabled={busy}
          class="mt-1 w-full px-2.5 py-1.5 text-xs rounded-md bg-primary text-primary-foreground hover:bg-primary/90 transition-colors disabled:opacity-50"
        >
          {isActive(p) ? "更换 Key" : "使用此预设"}
        </button>
      </article>
    {/each}
  </div>

  {#if presets.length === 0}
    <div class="px-3 py-6 text-center text-xs text-muted-foreground border border-dashed border-border rounded-md">
      暂无可用预设。
    </div>
  {/if}

  {#if message}
    <div class="px-3 py-2 rounded-md text-xs bg-success/10 text-success">{message}</div>
  {/if}
  {#if error}
    <div class="px-3 py-2 rounded-md text-xs bg-destructive/10 text-destructive whitespace-pre-wrap break-words">
      {error}
    </div>
  {/if}
</section>

{#if selectedPreset}
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
    onclick={cancel}
    role="presentation"
  >
    <div
      class="bg-card border border-border rounded-lg shadow-lg w-[26rem] max-w-[90vw] flex flex-col"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => e.stopPropagation()}
      role="dialog"
      aria-modal="true"
      tabindex="-1"
    >
      <div class="flex items-center justify-between p-4 border-b border-border">
        <h3 class="text-sm font-semibold text-foreground">{selectedPreset.name}</h3>
        <button
          onclick={cancel}
          class="p-1 rounded text-muted-foreground hover:text-foreground hover:bg-accent/50 transition-colors"
        >
          <X class="w-4 h-4" />
        </button>
      </div>
      <div class="p-4 flex flex-col gap-3 text-sm">
        <p class="text-xs text-muted-foreground">
          Base URL:
          <code class="font-mono text-foreground">{selectedPreset.base_url}</code>
        </p>
        <label class="flex flex-col gap-1.5">
          <span class="text-xs text-muted-foreground">API Key</span>
          <input
            type="password"
            bind:value={apiKey}
            placeholder="粘贴该中转站的 API key"
            autocomplete="off"
            disabled={busy}
            class="w-full bg-muted border border-border rounded-md px-3 py-2 text-xs font-mono text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-primary"
          />
        </label>
        {#if selectedPreset.api_key_url}
          <button
            onclick={() => openPresetUrl(selectedPreset!)}
            class="inline-flex items-center gap-1 text-xs text-primary hover:underline self-start"
          >
            没有 key？前往 {selectedPreset.api_key_url} 获取
            <ExternalLink class="w-3 h-3" />
          </button>
        {/if}
      </div>
      <div class="flex justify-end gap-2 px-4 py-3 border-t border-border">
        <button
          onclick={cancel}
          disabled={busy}
          class="px-3 py-1.5 text-xs rounded-md border border-border hover:bg-accent transition-colors disabled:opacity-50"
        >
          取消
        </button>
        <button
          onclick={apply}
          disabled={busy || !apiKey.trim()}
          class="px-3 py-1.5 text-xs rounded-md bg-primary text-primary-foreground hover:bg-primary/90 transition-colors disabled:opacity-50"
        >
          {busy ? "应用中…" : "应用"}
        </button>
      </div>
    </div>
  </div>
{/if}
