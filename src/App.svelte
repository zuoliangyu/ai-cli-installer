<script lang="ts">
  import ToolCard from "./lib/components/ToolCard.svelte";
  import MirrorStatus from "./lib/components/MirrorStatus.svelte";
  import { tools } from "./lib/stores";
  import { onMount } from "svelte";
  import { initApp } from "./lib/api";

  let appReady = $state(false);
  let initError = $state<string | null>(null);

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
  {/if}

  <footer>
    <span>v0.0.5</span>
  </footer>
</main>

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
    color: var(--text-muted);
    text-align: center;
    font-size: 0.85rem;
    padding-top: 1rem;
  }
</style>
