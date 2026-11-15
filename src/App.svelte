<script lang="ts">
  import { onMount } from "svelte";
  import ToolCard from "./lib/components/ToolCard.svelte";
  import FixesSection from "./lib/components/FixesSection.svelte";
  import About from "./lib/components/About.svelte";
  import LogViewer from "./lib/components/LogViewer.svelte";
  import Sidebar from "./lib/components/Sidebar.svelte";
  import UpdateToast from "./lib/components/UpdateToast.svelte";
  import { tools, installTaskSync } from "./lib/stores";
  import { initApp, refreshInstallTasks, stopInstallTaskMonitor } from "./lib/api";
  import { page, type Page } from "./lib/page";
  import { runStartupCheck } from "./lib/updateStore";
  import "./lib/theme";

  let appReady = $state(false);
  let initError = $state<string | null>(null);
  let fixesMounted = $state(false);

  $effect(() => {
    if ($page === "fixes") fixesMounted = true;
  });

  async function initialize() {
    initError = null;
    appReady = false;
    try {
      await initApp();
      appReady = true;
    } catch (err) {
      initError = err instanceof Error ? err.message : String(err);
    }
  }

  onMount(() => {
    initialize();
    // 不阻塞 UI；失败也不影响主流程
    runStartupCheck().catch(() => {});
    return stopInstallTaskMonitor;
  });

  const titles: Record<Page, string> = {
    tools: "CLI 工具",
    fixes: "配置修复",
    about: "关于",
    logs: "查看日志",
  };
</script>

<div class="flex h-screen overflow-hidden">
  <Sidebar />

  <main class="flex-1 min-w-0 {$page === 'fixes' || $page === 'logs' ? 'overflow-hidden' : 'overflow-y-auto'}">
    {#if initError}
      <div role="alert" class="m-6 px-4 py-3 rounded-md text-sm bg-destructive/10 text-destructive">
        <div>初始化失败：{initError}</div>
        <button onclick={initialize} class="mt-2 text-xs text-primary hover:underline">重新加载</button>
      </div>
    {:else if !appReady}
      <div role="status" class="flex h-full items-center justify-center text-sm text-muted-foreground">
        正在读取本机安装、版本和后台任务状态…
      </div>
    {:else}
      <div class="max-w-3xl mx-auto p-6 flex flex-col gap-6 {$page === 'fixes' || $page === 'logs' ? 'h-full min-h-0' : ''}">
        <header class="pb-3 border-b border-border shrink-0">
          <h1 class="text-lg font-semibold text-foreground">{titles[$page]}</h1>
        </header>

        {#if !$installTaskSync.ready}
          <div role="alert" class="shrink-0 px-3 py-2 rounded-md text-xs bg-warning/10 text-warning">
            <div>{$installTaskSync.error ?? "正在同步后台安装任务，暂时不能提交新安装…"}</div>
            <button onclick={refreshInstallTasks} class="mt-1 text-primary hover:underline">重新同步任务</button>
          </div>
        {/if}
        {#if $installTaskSync.notice}
          <div role="status" class="shrink-0 px-3 py-2 rounded-md text-xs bg-warning/10 text-warning">
            {$installTaskSync.notice}
          </div>
        {/if}

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
        {:else if $page === "about"}
          <About />
        {:else if $page === "logs"}
          <LogViewer />
        {/if}
        {#if fixesMounted}
          <div class="{$page === 'fixes' ? 'contents' : 'hidden'}">
            <FixesSection active={$page === "fixes"} />
          </div>
        {/if}
      </div>
    {/if}
  </main>
  <UpdateToast />
</div>
