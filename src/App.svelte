<script lang="ts">
  import { onMount } from "svelte";
  import ToolCard from "./lib/components/ToolCard.svelte";
  import PresetSection from "./lib/components/PresetSection.svelte";
  import FixesSection from "./lib/components/FixesSection.svelte";
  import About from "./lib/components/About.svelte";
  import Sidebar from "./lib/components/Sidebar.svelte";
  import UpdateToast from "./lib/components/UpdateToast.svelte";
  import { tools } from "./lib/stores";
  import { initApp } from "./lib/api";
  import { page } from "./lib/page";
  import { runStartupCheck } from "./lib/updateStore";
  import "./lib/theme";

  let appReady = $state(false);
  let initError = $state<string | null>(null);

  onMount(async () => {
    try {
      await initApp();
      appReady = true;
    } catch (err) {
      initError = err instanceof Error ? err.message : String(err);
    }
    // 不阻塞 UI；失败也不影响主流程
    runStartupCheck().catch(() => {});
  });

  const titles = {
    tools: "CLI 工具",
    presets: "中转预设",
    fixes: "配置修复",
    about: "关于",
  } as const;
</script>

<div class="flex h-screen overflow-hidden">
  <Sidebar />

  <main class="flex-1 min-w-0 {$page === 'fixes' ? 'overflow-hidden' : 'overflow-y-auto'}">
    {#if initError}
      <div class="m-6 px-4 py-3 rounded-md text-sm bg-destructive/10 text-destructive">
        初始化失败：{initError}
      </div>
    {:else if !appReady}
      <div class="flex h-full items-center justify-center text-sm text-muted-foreground">
        正在加载…
      </div>
    {:else}
      <div class="max-w-3xl mx-auto p-6 flex flex-col gap-6 {$page === 'fixes' ? 'h-full min-h-0' : ''}">
        <header class="pb-3 border-b border-border shrink-0">
          <h1 class="text-lg font-semibold text-foreground">{titles[$page]}</h1>
        </header>

        {#if $page === "tools"}
          <section class="flex flex-col gap-3">
            {#each $tools as tool (tool.id)}
              <ToolCard {tool} />
            {/each}
            {#if $tools.length === 0}
              <div class="px-3 py-6 text-center text-xs text-muted-foreground border border-dashed border-border rounded-md">
                没有可用的 CLI 工具。
              </div>
            {/if}
          </section>
        {:else if $page === "presets"}
          <PresetSection />
        {:else if $page === "fixes"}
          <FixesSection />
        {:else if $page === "about"}
          <About />
        {/if}
      </div>
    {/if}
  </main>
  <UpdateToast />
</div>
