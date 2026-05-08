<script lang="ts">
  import { Bell, ArrowDownToLine, X } from "lucide-svelte";
  import { updateState, downloadAndInstall, dismiss } from "../updateStore";
</script>

{#if __IS_TAURI__ && $updateState.status === "available" && !$updateState.dismissed}
  <div
    class="fixed bottom-4 right-4 z-50 w-72 bg-card border border-primary/40 rounded-lg shadow-lg p-3.5 space-y-2.5"
  >
    <div class="flex items-start justify-between gap-2">
      <div class="flex items-center gap-2">
        <Bell class="w-4 h-4 text-primary shrink-0" />
        <span class="text-sm font-medium text-foreground">发现新版本</span>
      </div>
      <button
        onclick={dismiss}
        class="p-0.5 text-muted-foreground hover:text-foreground transition-colors shrink-0"
        title="忽略此版本"
      >
        <X class="w-3.5 h-3.5" />
      </button>
    </div>
    <div class="text-xs text-muted-foreground">
      v{$updateState.currentVersion} →
      <span class="text-primary font-medium">v{$updateState.newVersion}</span>
    </div>
    <button
      onclick={() => {
        dismiss();
        downloadAndInstall();
      }}
      class="w-full flex items-center justify-center gap-1.5 px-2.5 py-1.5 rounded-md text-xs font-medium bg-primary text-primary-foreground hover:bg-primary/90 transition-colors"
    >
      <ArrowDownToLine class="w-3.5 h-3.5" />
      更新并重启
    </button>
  </div>
{/if}
