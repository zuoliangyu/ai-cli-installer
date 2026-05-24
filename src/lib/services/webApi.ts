//! Web-mode API. Hits the Axum HTTP routes exposed by `installer-web` and
//! subscribes to download progress through `/ws/progress`. Function shapes
//! are kept identical to `tauriApi.ts` so `../api.ts` can dispatch to either
//! at runtime.

import { tools, mirrorProbes } from "../stores";
import type {
  ToolDescriptor,
  InstallReport,
  InstallMethod,
  DownloadProgress,
  MirrorProbe,
  Channel,
  PathStatus,
  PathScope,
  ClaudePreset,
  ClaudeSettingsEnv,
  NodeInfo,
  Fix,
  ApplyFixReport,
  RemoveFixReport,
  UnlistenFn,
} from "../types";

const API_ORIGIN = ""; // same-origin

async function get<T>(path: string, query?: Record<string, string>): Promise<T> {
  const url = new URL(`${API_ORIGIN}${path}`, window.location.origin);
  if (query) {
    for (const [k, v] of Object.entries(query)) url.searchParams.set(k, v);
  }
  const resp = await fetch(url, { method: "GET" });
  if (!resp.ok) throw new Error(await resp.text());
  return resp.json() as Promise<T>;
}

async function post<T>(path: string, body?: unknown): Promise<T> {
  const resp = await fetch(`${API_ORIGIN}${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!resp.ok) throw new Error(await resp.text());
  if (resp.status === 204) return undefined as T;
  return resp.json() as Promise<T>;
}

export async function initApp(): Promise<void> {
  const list = await get<ToolDescriptor[]>("/api/tools");
  tools.set(list);

  get<MirrorProbe[]>("/api/mirrors/probe")
    .catch(() => [] as MirrorProbe[])
    .then((probes) => mirrorProbes.set(probes));
}

export async function refreshTools(): Promise<void> {
  const list = await get<ToolDescriptor[]>("/api/tools");
  tools.set(list);
}

export async function probeMirrors(): Promise<MirrorProbe[]> {
  const probes = await post<MirrorProbe[]>("/api/mirrors/probe");
  mirrorProbes.set(probes);
  return probes;
}

export async function installTool(
  toolId: string,
  channel: Channel = "latest",
  method: InstallMethod = "native",
  mirror: string | null = null
): Promise<InstallReport> {
  return post<InstallReport>("/api/tools/install", { toolId, channel, method, mirror });
}

export async function detectNode(): Promise<NodeInfo> {
  return get<NodeInfo>("/api/node");
}

export async function listFixes(): Promise<Fix[]> {
  return get<Fix[]>("/api/fixes");
}

export async function applyFixes(fixIds: string[]): Promise<ApplyFixReport> {
  return post<ApplyFixReport>("/api/fixes/apply", { fixIds });
}

export async function removeFixes(fixIds: string[]): Promise<RemoveFixReport> {
  return post<RemoveFixReport>("/api/fixes/remove", { fixIds });
}

export async function openPath(path: string): Promise<void> {
  await post<void>("/api/open-path", { path });
}

export async function onDownloadProgress(
  cb: (p: DownloadProgress) => void
): Promise<UnlistenFn> {
  // `new URL("/ws/progress", origin)` keeps host + port; switching the
  // protocol from http(s) to ws(s) is the only edit we need.
  const url = new URL("/ws/progress", window.location.origin);
  url.protocol = url.protocol.replace(/^http/, "ws");
  const sock = new WebSocket(url);
  sock.addEventListener("message", (ev) => {
    try {
      const data = JSON.parse(ev.data) as DownloadProgress;
      cb(data);
    } catch {
      // ignore malformed payloads
    }
  });
  return () => {
    sock.close();
  };
}

export async function checkPathStatus(toolId: string): Promise<PathStatus> {
  return get<PathStatus>("/api/path/status", { toolId });
}

export async function addToPath(
  toolId: string,
  scope: PathScope = "system"
): Promise<void> {
  await post<void>("/api/path/add", { toolId, scope });
}

export async function removeFromPath(
  toolId: string,
  scope: PathScope = "system"
): Promise<void> {
  await post<void>("/api/path/remove", { toolId, scope });
}

export async function listClaudePresets(): Promise<ClaudePreset[]> {
  return get<ClaudePreset[]>("/api/presets");
}

export async function getClaudeSettings(): Promise<ClaudeSettingsEnv> {
  return get<ClaudeSettingsEnv>("/api/presets/current");
}

export async function applyClaudePreset(
  baseUrl: string,
  authToken: string
): Promise<void> {
  await post<void>("/api/presets/apply", { baseUrl, authToken });
}

export async function getLogs(): Promise<string[]> {
  return get<string[]>("/api/logs");
}
