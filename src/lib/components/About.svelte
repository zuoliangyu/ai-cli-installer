<script lang="ts">
  import { open as openExternal } from "@tauri-apps/plugin-shell";

  interface Props {
    ondismiss: () => void;
  }
  let { ondismiss }: Props = $props();

  const AUTHOR = "左岚";
  const BILIBILI = "https://space.bilibili.com/27619688";
  const REPO = "https://github.com/zuoliangyu/ai-cli-installer-dist";

  async function openUrl(url: string) {
    try {
      await openExternal(url);
    } catch {
      // shell.open is best-effort; silently swallow
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") ondismiss();
  }
</script>

<svelte:window onkeydown={onKey} />

<div
  class="backdrop"
  onclick={ondismiss}
  role="presentation"
>
  <div
    class="modal"
    onclick={(e) => e.stopPropagation()}
    onkeydown={(e) => e.stopPropagation()}
    role="dialog"
    aria-modal="true"
    aria-labelledby="about-title"
    tabindex="-1"
  >
    <h2 id="about-title">关于 AI CLI Installer</h2>
    <p class="desc">
      为 Claude Code 等 AI CLI 工具提供镜像加速下载与一键安装的桌面工具。
    </p>

    <dl>
      <dt>作者</dt>
      <dd>
        <span>{AUTHOR}</span>
        <button class="link" onclick={() => openUrl(BILIBILI)}>哔哩哔哩主页 →</button>
      </dd>

      <dt>项目</dt>
      <dd>
        <button class="link" onclick={() => openUrl(REPO)}>
          GitHub Releases →
        </button>
      </dd>

      <dt>版本</dt>
      <dd>v0.0.10</dd>
    </dl>

    <div class="actions">
      <button class="primary" onclick={ondismiss}>关闭</button>
    </div>
  </div>
</div>

<style>
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
    gap: 1rem;
  }
  h2 {
    font-size: 1.1rem;
    font-weight: 600;
  }
  .desc {
    font-size: 0.85rem;
    color: var(--text-muted);
    line-height: 1.5;
  }
  dl {
    display: grid;
    grid-template-columns: 60px 1fr;
    row-gap: 0.5rem;
    column-gap: 1rem;
    font-size: 0.85rem;
  }
  dt {
    color: var(--text-muted);
    font-weight: 500;
  }
  dd {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
  }
  .link {
    background: none;
    border: none;
    padding: 0;
    color: var(--accent);
    cursor: pointer;
    text-align: left;
    font-size: inherit;
    font-family: inherit;
  }
  .link:hover {
    color: var(--accent-hover);
    text-decoration: underline;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    margin-top: 0.5rem;
  }
</style>
