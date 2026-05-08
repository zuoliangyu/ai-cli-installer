<script lang="ts">
  import { Wrench, Plug, Bandage, Info, Sun, Moon, Monitor, RefreshCw } from "lucide-svelte";
  import { page, navigate, type Page } from "../page";
  import { theme, setTheme } from "../theme";
  import { mirrorProbes } from "../stores";
  import { probeMirrors } from "../api";
  import { updateState } from "../updateStore";

  let probing = $state(false);
  let hasUpdate = $derived(
    $updateState.status === "available" && !$updateState.dismissed
  );

  async function rerunProbe() {
    probing = true;
    try {
      await probeMirrors();
    } finally {
      probing = false;
    }
  }

  let mirrorOk = $derived($mirrorProbes.filter((p) => p.ok).length);
  let mirrorTotal = $derived($mirrorProbes.length);

  type NavItem = { id: Page; label: string; icon: typeof Wrench };
  const items: NavItem[] = [
    { id: "tools", label: "CLI 工具", icon: Wrench },
    { id: "presets", label: "中转预设", icon: Plug },
    { id: "fixes", label: "配置修复", icon: Bandage },
    { id: "about", label: "关于", icon: Info },
  ];
</script>

<aside class="w-60 h-full border-r border-border bg-card flex flex-col shrink-0">
  <!-- Header -->
  <div class="p-4 border-b border-border">
    <h1 class="text-sm font-semibold text-foreground text-center">
      AI CLI Installer
    </h1>
    <p class="mt-1 text-[11px] text-muted-foreground text-center">
      镜像加速 · 一键安装
    </p>
  </div>

  <!-- Mirror status -->
  <div class="px-3 pt-3">
    <button
      onclick={rerunProbe}
      disabled={probing}
      class="w-full flex items-center justify-between gap-2 px-3 py-2 rounded-md text-xs border border-border bg-muted/40 hover:bg-accent/50 hover:text-foreground text-muted-foreground transition-colors disabled:opacity-60"
      title="重新测试镜像延迟"
    >
      <span class="flex items-center gap-1.5">
        <RefreshCw class="w-3 h-3 {probing ? 'animate-spin' : ''}" />
        镜像状态
      </span>
      {#if mirrorTotal > 0}
        <span class="font-mono">
          <span class={mirrorOk > 0 ? "text-success" : "text-destructive"}>
            {mirrorOk}
          </span>
          <span class="text-muted-foreground">/{mirrorTotal}</span>
        </span>
      {:else}
        <span class="text-muted-foreground">…</span>
      {/if}
    </button>
    {#if mirrorTotal > 0}
      <ul class="mt-2 space-y-0.5">
        {#each $mirrorProbes as p (p.name)}
          <li class="flex items-center justify-between text-[11px] font-mono">
            <span class="truncate text-muted-foreground" title={p.name}>{p.name}</span>
            {#if p.ok && p.latency_ms !== null}
              <span class="text-success shrink-0 ml-2">{p.latency_ms}ms</span>
            {:else}
              <span class="text-destructive shrink-0 ml-2 truncate max-w-[6rem]" title={p.error ?? "失败"}>
                {p.error ?? "失败"}
              </span>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  <!-- Nav -->
  <nav class="flex-1 overflow-y-auto p-2 mt-2">
    <h2 class="px-3 py-1 text-[10px] font-medium text-muted-foreground uppercase tracking-wider">
      导航
    </h2>
    <div class="mt-1 space-y-0.5">
      {#each items as item}
        {@const Icon = item.icon}
        <button
          onclick={() => navigate(item.id)}
          class="w-full flex items-center gap-2 px-3 py-2 rounded-md text-sm transition-colors {$page === item.id
            ? 'bg-accent text-accent-foreground'
            : 'text-muted-foreground hover:bg-accent/50 hover:text-foreground'}"
        >
          <Icon class="w-4 h-4" />
          {item.label}
        </button>
      {/each}
    </div>
  </nav>

  <!-- Footer -->
  <div class="p-3 border-t border-border">
    <div class="flex items-center justify-between gap-2">
      <button
        onclick={() => navigate("about")}
        class="inline-flex items-center gap-1.5 text-xs text-muted-foreground font-mono hover:text-foreground transition-colors"
        title={hasUpdate ? `有新版本 v${$updateState.newVersion ?? ""}` : "查看版本/关于"}
      >
        <span>v{__APP_VERSION__}</span>
        {#if hasUpdate}
          <span class="relative flex h-2 w-2" aria-hidden="true">
            <span class="animate-ping absolute inline-flex h-full w-full rounded-full bg-primary opacity-75"></span>
            <span class="relative inline-flex rounded-full h-2 w-2 bg-primary"></span>
          </span>
        {/if}
      </button>
      <div class="flex rounded-md bg-muted p-0.5">
        <button
          onclick={() => setTheme("light")}
          class="p-1 rounded transition-colors {$theme === 'light'
            ? 'bg-background text-foreground shadow-sm'
            : 'text-muted-foreground hover:text-foreground'}"
          title="亮色模式"
        >
          <Sun class="w-3.5 h-3.5" />
        </button>
        <button
          onclick={() => setTheme("system")}
          class="p-1 rounded transition-colors {$theme === 'system'
            ? 'bg-background text-foreground shadow-sm'
            : 'text-muted-foreground hover:text-foreground'}"
          title="跟随系统"
        >
          <Monitor class="w-3.5 h-3.5" />
        </button>
        <button
          onclick={() => setTheme("dark")}
          class="p-1 rounded transition-colors {$theme === 'dark'
            ? 'bg-background text-foreground shadow-sm'
            : 'text-muted-foreground hover:text-foreground'}"
          title="暗色模式"
        >
          <Moon class="w-3.5 h-3.5" />
        </button>
      </div>
    </div>
  </div>
</aside>
