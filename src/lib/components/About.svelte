<script lang="ts">
  import { open as openExternal } from "@tauri-apps/plugin-shell";
  import { Mail, Users, ExternalLink, Heart } from "lucide-svelte";
  import UpdateIndicator from "./UpdateIndicator.svelte";

  const AUTHOR = "左岚";
  const EMAIL = "zuolan1102@qq.com";
  const QQ_GROUP = "1019721429";
  const BILIBILI = "https://space.bilibili.com/27619688";
  const REPO = "https://github.com/zuoliangyu/ai-cli-installer-dist";
  const SESSION_VIEWER = "https://github.com/zuoliangyu/AI-Session-Viewer";

  async function openUrl(url: string) {
    try {
      await openExternal(url);
    } catch {
      // best-effort
    }
  }
</script>

<section class="flex flex-col gap-6">
  <header>
    <h2 class="text-base font-semibold text-foreground">关于</h2>
    <p class="mt-1 text-xs text-muted-foreground leading-relaxed">
      为 Claude Code 等 AI CLI 工具提供镜像加速下载与一键安装的桌面工具。
    </p>
  </header>

  <!-- Project info + version / update -->
  <div class="flex flex-col gap-3 p-4 rounded-md border border-border bg-card">
    <h3 class="text-sm font-semibold text-foreground">项目</h3>
    <dl class="grid grid-cols-[80px_1fr] gap-y-2 gap-x-3 text-sm">
      <dt class="text-muted-foreground">名称</dt>
      <dd class="text-foreground">AI CLI Installer</dd>

      <dt class="text-muted-foreground">主页</dt>
      <dd>
        <button
          onclick={() => openUrl(REPO)}
          class="inline-flex items-center gap-1 text-primary hover:underline"
        >
          <svg class="w-3.5 h-3.5" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
            <path d="M12 .5C5.65.5.5 5.65.5 12c0 5.08 3.29 9.39 7.86 10.92.58.1.79-.25.79-.56 0-.27-.01-1-.02-1.96-3.2.69-3.87-1.54-3.87-1.54-.52-1.32-1.27-1.67-1.27-1.67-1.04-.71.08-.7.08-.7 1.15.08 1.76 1.18 1.76 1.18 1.02 1.75 2.68 1.25 3.34.96.1-.74.4-1.25.72-1.54-2.55-.29-5.24-1.28-5.24-5.69 0-1.26.45-2.29 1.18-3.1-.12-.29-.51-1.46.11-3.04 0 0 .96-.31 3.15 1.18a10.92 10.92 0 0 1 5.74 0c2.19-1.49 3.15-1.18 3.15-1.18.62 1.58.23 2.75.11 3.04.74.81 1.18 1.84 1.18 3.1 0 4.42-2.69 5.39-5.25 5.68.41.36.78 1.06.78 2.13 0 1.54-.01 2.78-.01 3.16 0 .31.21.67.79.56C20.21 21.39 23.5 17.08 23.5 12 23.5 5.65 18.35.5 12 .5z" />
          </svg>
          GitHub Releases
          <ExternalLink class="w-3 h-3" />
        </button>
      </dd>
    </dl>
    <div class="border-t border-border pt-3">
      <UpdateIndicator />
    </div>
  </div>

  <!-- Author / contact -->
  <div class="flex flex-col gap-3 p-4 rounded-md border border-border bg-card">
    <h3 class="text-sm font-semibold text-foreground">作者</h3>

    <div class="flex items-center gap-2.5 text-sm text-foreground">
      <Heart class="w-4 h-4 text-muted-foreground shrink-0" />
      <span>{AUTHOR}</span>
    </div>

    <button
      onclick={() => openUrl(`mailto:${EMAIL}`)}
      class="flex items-center gap-2.5 text-sm text-foreground hover:text-primary transition-colors text-left"
    >
      <Mail class="w-4 h-4 text-muted-foreground shrink-0" />
      <span>{EMAIL}</span>
    </button>

    <div class="flex items-center gap-2.5 text-sm text-foreground">
      <Users class="w-4 h-4 text-muted-foreground shrink-0" />
      <span>QQ 群：{QQ_GROUP}</span>
    </div>

    <button
      onclick={() => openUrl(BILIBILI)}
      class="flex items-center gap-2.5 text-sm text-foreground hover:text-primary transition-colors text-left"
    >
      <svg class="w-4 h-4 text-muted-foreground shrink-0" viewBox="0 0 24 24" fill="currentColor">
        <path d="M17.813 4.653h.854c1.51.054 2.769.578 3.773 1.574 1.004.995 1.524 2.249 1.56 3.76v7.36c-.036 1.51-.556 2.769-1.56 3.773s-2.262 1.524-3.773 1.56H5.333c-1.51-.036-2.769-.556-3.773-1.56S.036 18.858 0 17.347v-7.36c.036-1.511.556-2.765 1.56-3.76 1.004-.996 2.262-1.52 3.773-1.574h.774l-1.174-1.12a1.234 1.234 0 0 1-.373-.906c0-.356.124-.658.373-.907l.027-.027c.267-.249.573-.373.92-.373.347 0 .653.124.92.373L9.653 4.44c.071.071.134.142.187.213h4.267a.836.836 0 0 1 .16-.213l2.853-2.747c.267-.249.573-.373.92-.373.347 0 .662.151.929.4.267.249.391.551.391.907 0 .355-.124.657-.373.906zM5.333 7.24c-.746.018-1.373.276-1.88.773-.506.498-.769 1.13-.787 1.893v7.44c.018.764.281 1.395.787 1.893.507.498 1.134.756 1.88.773h13.334c.746-.017 1.373-.275 1.88-.773.506-.498.769-1.129.787-1.893v-7.44c-.018-.764-.281-1.395-.787-1.893a2.51 2.51 0 0 0-1.88-.773zM8 11.107c.373 0 .684.124.933.373.25.249.383.569.4.96v1.173c-.017.391-.15.711-.4.96-.249.25-.56.374-.933.374s-.684-.125-.933-.374c-.25-.249-.383-.569-.4-.96V12.44c.017-.391.15-.711.4-.96.249-.249.56-.373.933-.373zm8 0c.373 0 .684.124.933.373.25.249.383.569.4.96v1.173c-.017.391-.15.711-.4.96-.249.25-.56.374-.933.374s-.684-.125-.933-.374c-.25-.249-.383-.569-.4-.96V12.44c.017-.391.15-.711.4-.96.249-.249.56-.373.933-.373z" />
      </svg>
      <span>哔哩哔哩主页</span>
      <ExternalLink class="w-3 h-3 text-muted-foreground" />
    </button>

    <button
      onclick={() => openUrl(REPO)}
      class="flex items-center gap-2.5 text-sm text-foreground hover:text-primary transition-colors text-left"
    >
      <svg class="w-4 h-4 text-muted-foreground shrink-0" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
        <path d="M12 .5C5.65.5.5 5.65.5 12c0 5.08 3.29 9.39 7.86 10.92.58.1.79-.25.79-.56 0-.27-.01-1-.02-1.96-3.2.69-3.87-1.54-3.87-1.54-.52-1.32-1.27-1.67-1.27-1.67-1.04-.71.08-.7.08-.7 1.15.08 1.76 1.18 1.76 1.18 1.02 1.75 2.68 1.25 3.34.96.1-.74.4-1.25.72-1.54-2.55-.29-5.24-1.28-5.24-5.69 0-1.26.45-2.29 1.18-3.1-.12-.29-.51-1.46.11-3.04 0 0 .96-.31 3.15 1.18a10.92 10.92 0 0 1 5.74 0c2.19-1.49 3.15-1.18 3.15-1.18.62 1.58.23 2.75.11 3.04.74.81 1.18 1.84 1.18 3.1 0 4.42-2.69 5.39-5.25 5.68.41.36.78 1.06.78 2.13 0 1.54-.01 2.78-.01 3.16 0 .31.21.67.79.56C20.21 21.39 23.5 17.08 23.5 12 23.5 5.65 18.35.5 12 .5z" />
      </svg>
      <span>GitHub</span>
      <ExternalLink class="w-3 h-3 text-muted-foreground" />
    </button>
  </div>

  <!-- Series -->
  <div class="flex flex-col gap-2 p-4 rounded-md border border-dashed border-border bg-muted/30">
    <h3 class="text-sm font-semibold text-foreground">同系列工具</h3>
    <p class="text-xs text-muted-foreground leading-relaxed">
      AI CLI Installer 与 AI Session Viewer 同属一个工具系列，共享设计语言。
    </p>
    <button
      onclick={() => openUrl(SESSION_VIEWER)}
      class="inline-flex items-center gap-1 text-xs text-primary hover:underline self-start"
    >
      AI Session Viewer
      <ExternalLink class="w-3 h-3" />
    </button>
  </div>
</section>
