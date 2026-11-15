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
  getInstallState,
  patchInstallState,
  fixesRevision,
  pathRevision,
  pathUpdating,
  type InstallChannel,
} from "./stores";
import { get } from "svelte/store";
import { InstallTaskMonitor } from "./installTasks";
import type {
  ApiTransport,
  ApplyFixReport,
  Fix,
  LogChunk,
  MirrorProbe,
  NodeInfo,
  PathScope,
  PathStatus,
  RemoveFixReport,
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
  taskMonitor.startPolling();
  await Promise.all([refreshTools(), taskMonitor.refresh()]);
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
// Process-owned install jobs — one polling loop for both transports
// ---------------------------------------------------------------------------

const taskMonitor = new InstallTaskMonitor(transport, () => {
  fixesRevision.update((revision) => revision + 1);
  pathRevision.update((revision) => revision + 1);
  void refreshTools().catch((error) => console.warn("安装任务结束后刷新工具失败:", error));
});

export const refreshInstallTasks = (): Promise<void> => taskMonitor.refresh();
export const stopInstallTaskMonitor = (): void => taskMonitor.stopPolling();

// ---------------------------------------------------------------------------
// Install (state kept in installStates, survives ToolCard unmount)
// ---------------------------------------------------------------------------

export async function installTool(toolId: string, channel: InstallChannel): Promise<void> {
  await taskMonitor.install(toolId, channel);
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
    const taskStatus = getInstallState(toolId).taskStatus;
    if (taskStatus !== "queued" && taskStatus !== "running") {
      patchInstallState(toolId, { busy: false });
    }
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
  const report = await (await transport()).applyFixes(fixIds);
  fixesRevision.update((revision) => revision + 1);
  return report;
}

export async function removeFixes(fixIds: string[]): Promise<RemoveFixReport> {
  const report = await (await transport()).removeFixes(fixIds);
  fixesRevision.update((revision) => revision + 1);
  return report;
}

export async function openPath(path: string): Promise<void> {
  return (await transport()).openPath(path);
}

export async function checkPathStatus(toolId: string): Promise<PathStatus> {
  return (await transport()).checkPathStatus(toolId);
}

export async function addToPath(toolId: string, scope: PathScope = "user"): Promise<void> {
  await changePath((t) => t.addToPath(toolId, scope));
}

export async function removeFromPath(toolId: string, scope: PathScope = "user"): Promise<void> {
  await changePath((t) => t.removeFromPath(toolId, scope));
}

async function changePath(change: (t: ApiTransport) => Promise<void>): Promise<void> {
  if (get(pathUpdating)) throw new Error("另一项 PATH 操作正在进行，请稍后重试");
  pathUpdating.set(true);
  try {
    await change(await transport());
    pathRevision.update((revision) => revision + 1);
    try {
      await refreshTools();
    } catch (e) {
      console.warn("PATH 已更新，但刷新安装诊断失败:", e);
    }
  } finally {
    pathUpdating.set(false);
  }
}

export async function getLogs(since: number | null = null): Promise<LogChunk> {
  return (await transport()).getLogs(since);
}
