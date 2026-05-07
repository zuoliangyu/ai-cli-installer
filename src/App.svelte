<script lang="ts">
  import ToolCard from "./lib/components/ToolCard.svelte";
  import MirrorStatus from "./lib/components/MirrorStatus.svelte";
  import About from "./lib/components/About.svelte";
  import PresetSection from "./lib/components/PresetSection.svelte";
  import FixesSection from "./lib/components/FixesSection.svelte";
  import { tools } from "./lib/stores";
  import { onMount } from "svelte";
  import { initApp } from "./lib/api";

  let appReady = $state(false);
  let initError = $state<string | null>(null);
  let aboutOpen = $state(false);

  onMount(async () => {
    try {
      await initApp();
      appReady = true;
    } catch (err) {
      initError = err instanceof Error ? err.message : String(err);
    }
  });
</script>

<main>
  <header>
    <h1>AI CLI Installer</h1>
    <MirrorStatus />
  </header>

  {#if initError}
    <div class="error-banner">初始化失败：{initError}</div>
  {:else if !appReady}
    <div class="loading">正在加载…</div>
  {:else}
    <section class="tools">
      {#each $tools as tool (tool.id)}
        <ToolCard {tool} />
      {/each}
    </section>

    <PresetSection />

    <FixesSection />
  {/if}

  <footer>
    <button class="footer-btn" onclick={() => (aboutOpen = true)}>
      v0.0.9 · 关于
    </button>
  </footer>
</main>

{#if aboutOpen}
  <About ondismiss={() => (aboutOpen = false)} />
{/if}

<style>
  main {
    flex: 1;
    display: flex;
    flex-direction: column;
    max-width: 720px;
    width: 100%;
    margin: 0 auto;
    padding: 1.5rem;
    gap: 1rem;
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding-bottom: 1rem;
    border-bottom: 1px solid var(--border);
  }

  header h1 {
    font-size: 1.4rem;
    font-weight: 600;
  }

  .tools {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .loading,
  .error-banner {
    padding: 2rem;
    text-align: center;
    color: var(--text-muted);
  }

  .error-banner {
    color: var(--error);
  }

  footer {
    margin-top: auto;
    text-align: center;
    padding-top: 1rem;
  }
  .footer-btn {
    background: none;
    border: none;
    padding: 0.25rem 0.5rem;
    color: var(--text-muted);
    font-size: 0.85rem;
    cursor: pointer;
    border-radius: 4px;
    transition: background 0.15s;
  }
  .footer-btn:hover {
    background: rgba(0, 0, 0, 0.05);
    color: var(--text);
  }
</style>
