<script lang="ts">
  import { mirrorProbes } from "../stores";
  import { probeMirrors } from "../api";

  let probing = $state(false);

  async function rerun() {
    probing = true;
    try {
      await probeMirrors();
    } finally {
      probing = false;
    }
  }

  let summary = $derived.by(() => {
    const list = $mirrorProbes;
    if (list.length === 0) return "镜像测试中…";
    const ok = list.filter((p) => p.ok).length;
    return `${ok}/${list.length} 镜像可用`;
  });
</script>

<div class="status">
  <button onclick={rerun} disabled={probing} title="重新测试镜像延迟">
    {probing ? "测试中…" : summary}
  </button>
  {#if $mirrorProbes.length > 0}
    <div class="popup">
      <ul>
        {#each $mirrorProbes as p (p.name)}
          <li class:ok={p.ok}>
            <span class="name">{p.name}</span>
            <span class="latency">
              {#if p.ok && p.latency_ms !== null}
                {p.latency_ms} ms
              {:else}
                {p.error ?? "失败"}
              {/if}
            </span>
          </li>
        {/each}
      </ul>
    </div>
  {/if}
</div>

<style>
  .status {
    position: relative;
  }
  .status:hover .popup {
    display: block;
  }
  .popup {
    display: none;
    position: absolute;
    right: 0;
    top: calc(100% + 4px);
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.5rem;
    min-width: 220px;
    z-index: 10;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.1);
  }
  ul {
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  li {
    display: flex;
    justify-content: space-between;
    font-size: 0.8rem;
    color: var(--text-muted);
    font-family: ui-monospace, Consolas, monospace;
  }
  li.ok {
    color: var(--text);
  }
  li.ok .latency {
    color: var(--success);
  }
  .name {
    margin-right: 0.5rem;
  }
</style>
