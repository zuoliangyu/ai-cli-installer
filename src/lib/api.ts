//! Unified API entry point. Picks the Tauri or Web transport based on the
//! `__IS_TAURI__` Vite define; both implement `ApiTransport` and are checked
//! against it at compile time. All store writes (tools, mirror probes,
//! per-tool install state, download progress) live here exactly once — the
//! transports only move data.

import {
  tools,
  mirrorProbes,
  mirrorProbing,
  mirrorProbeError,
  AUTO_MIRROR,
  getInstallState,
  patchInstallState,
  installStates,
  type InstallChannel,
} from "./stores";
import type {
  ApiTransport,
  ApplyFixReport,
  DownloadProgress,
  Fix,
  LogChunk,
  MirrorProbe,
  NodeInfo,
  PathScope,
  PathStatus,
  RemoveFixReport,
  UnlistenFn,
} from "./types";

const transportPromise: Promise<ApiTransport> = __IS_TAURI__
  ? import("./services/tauriApi").then((m) => m.tauriApi)
  : import("./services/webApi").then((m) => m.webApi);

function transport(): Promise<ApiTransport> {
  return transportPromise;
}

function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

// ---------------------------------------------------------------------------
// App bootstrap / tools / mirrors
// ---------------------------------------------------------------------------

export async function initApp(): Promise<void> {
  ensureProgressSubscription();
  await refreshTools();
  // Probe mirrors lazily, don't block first paint. 失败写入 mirrorProbeError，
  // 由 Sidebar 展示。
  probeMirrors().catch(() => {});
}

/** 递增序号：并发刷新时只采纳最后一次发起的请求结果，避免慢的旧请求
 * （list_tools 可能要数秒）覆盖新结果。 */
let toolsRequestSeq = 0;

export async function refreshTools(): Promise<void> {
  const seq = ++toolsRequestSeq;
  const list = await (await transport()).listTools();
  if (seq === toolsRequestSeq) tools.set(list);
}

let probeInFlight: Promise<MirrorProbe[]> | null = null;

/** 镜像测速。并发调用共享同一个进行中的请求；结果写入 mirrorProbes，
 * 错误写入 mirrorProbeError（同时抛出，调用方可自行处理）。 */
export function probeMirrors(): Promise<MirrorProbe[]> {
  if (probeInFlight) return probeInFlight;
  mirrorProbing.set(true);
  mirrorProbeError.set(null);
  const run = (async () => {
    try {
      const probes = await (await transport()).probeMirrors();
      mirrorProbes.set(probes);
      return probes;
    } catch (e) {
      mirrorProbeError.set(errorMessage(e) || "镜像测速失败");
      throw e;
    } finally {
      probeInFlight = null;
      mirrorProbing.set(false);
    }
  })();
  probeInFlight = run;
  return run;
}

// ---------------------------------------------------------------------------
// Download progress — one shared subscription for the whole app
// ---------------------------------------------------------------------------

let progressSubscription: Promise<UnlistenFn> | null = null;

function handleProgress(p: DownloadProgress): void {
  installStates.update((all) => {
    const cur = all[p.tool_id];
    // 只在安装进行中接收进度，避免结束后迟到的事件把进度条又显示出来。
    if (!cur?.busy) return all;
    return { ...all, [p.tool_id]: { ...cur, progress: p } };
  });
}

/** 建立全局唯一的进度订阅（Tauri 一个 listener / Web 一条 WebSocket），
 * 常驻到页面关闭；进度按 tool_id 写入 installStates，卡片只读 store。 */
export function ensureProgressSubscription(): void {
  if (progressSubscription) return;
  progressSubscription = transport()
    .then((t) => t.subscribeProgress(handleProgress))
    .catch((e) => {
      console.warn("订阅下载进度失败:", e);
      progressSubscription = null;
      return () => {};
    });
}

// ---------------------------------------------------------------------------
// Install (state kept in installStates, survives ToolCard unmount)
// ---------------------------------------------------------------------------

export async function installTool(toolId: string, channel: InstallChannel): Promise<void> {
  const cur = getInstallState(toolId);
  if (cur.busy) return; // 防止重复安装
  const pin = cur.mirror === AUTO_MIRROR ? null : cur.mirror;
  patchInstallState(toolId, {
    busy: true,
    channel,
    error: null,
    message: null,
    progress: null,
    failedMirror: null,
  });
  ensureProgressSubscription();
  try {
    const report = await (await transport()).installTool(toolId, channel, cur.method, pin);
    const via = report.method === "npm" ? "npm" : pin ? `镜像 ${pin}` : "镜像";
    let msg = `已通过${via}安装 ${report.version} (${report.elapsed_secs}s)`;
    if (report.auto_applied_fixes.length > 0) {
      msg += `\n顺便应用了配置修复：${report.auto_applied_fixes.join("、")}（已写入 Claude 配置，可直接 \`claude login\`；如需撤销请到「配置修复」面板）。`;
    }
    patchInstallState(toolId, { message: msg });
    try {
      await refreshTools();
    } catch (e) {
      console.warn("安装后刷新工具列表失败:", e);
    }
  } catch (e) {
    patchInstallState(toolId, { error: errorMessage(e), failedMirror: pin });
  } finally {
    patchInstallState(toolId, { busy: false, progress: null });
  }
}

/** 「获取版本失败 · 点此重试」：复用该工具的 busy 锁，重试期间禁用其它按钮。 */
export async function retryToolVersions(toolId: string): Promise<void> {
  if (getInstallState(toolId).busy) return;
  patchInstallState(toolId, { busy: true, error: null, message: null, failedMirror: null });
  try {
    await refreshTools();
  } catch (e) {
    patchInstallState(toolId, { error: errorMessage(e) });
  } finally {
    patchInstallState(toolId, { busy: false });
  }
}

// ---------------------------------------------------------------------------
// Pass-through calls
// ---------------------------------------------------------------------------

export async function detectNode(): Promise<NodeInfo> {
  return (await transport()).detectNode();
}

export async function listFixes(): Promise<Fix[]> {
  return (await transport()).listFixes();
}

export async function applyFixes(fixIds: string[]): Promise<ApplyFixReport> {
  return (await transport()).applyFixes(fixIds);
}

export async function removeFixes(fixIds: string[]): Promise<RemoveFixReport> {
  return (await transport()).removeFixes(fixIds);
}

export async function openPath(path: string): Promise<void> {
  return (await transport()).openPath(path);
}

export async function checkPathStatus(toolId: string): Promise<PathStatus> {
  return (await transport()).checkPathStatus(toolId);
}

export async function addToPath(toolId: string, scope: PathScope = "user"): Promise<void> {
  return (await transport()).addToPath(toolId, scope);
}

export async function removeFromPath(toolId: string, scope: PathScope = "user"): Promise<void> {
  return (await transport()).removeFromPath(toolId, scope);
}

export async function getLogs(since: number | null = null): Promise<LogChunk> {
  return (await transport()).getLogs(since);
}
