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
  NodeInfo,
  Fix,
  ApplyFixReport,
  RemoveFixReport,
  UnlistenFn,
} from "../types";

const API_ORIGIN = ""; // same-origin
const ACCESS_TOKEN = (() => {
  const url = new URL(window.location.href);
  const fromUrl = url.searchParams.get("token");
  if (fromUrl) {
    try {
      sessionStorage.setItem("installer_access_token", fromUrl);
    } catch {
      // The current page still keeps the in-memory token when storage is disabled.
    }
    url.searchParams.delete("token");
    history.replaceState(null, "", `${url.pathname}${url.search}${url.hash}`);
  }
  if (fromUrl) return fromUrl;
  try {
    return sessionStorage.getItem("installer_access_token");
  } catch {
    return null;
  }
})();

function requestHeaders(json = false): Record<string, string> {
  return {
    ...(json ? { "Content-Type": "application/json" } : {}),
    ...(ACCESS_TOKEN ? { Authorization: `Bearer ${ACCESS_TOKEN}` } : {}),
  };
}

async function responseError(resp: Response): Promise<Error> {
  if (resp.status === 401) {
    return new Error("访问未授权，请使用带 ?token=访问令牌 的地址重新打开页面");
  }
  return new Error(await resp.text());
}

async function get<T>(path: string, query?: Record<string, string>): Promise<T> {
  const url = new URL(`${API_ORIGIN}${path}`, window.location.origin);
  if (query) {
    for (const [k, v] of Object.entries(query)) url.searchParams.set(k, v);
  }
  const resp = await fetch(url, { method: "GET", headers: requestHeaders() });
  if (!resp.ok) throw await responseError(resp);
  return resp.json() as Promise<T>;
}

async function post<T>(path: string, body?: unknown): Promise<T> {
  const resp = await fetch(`${API_ORIGIN}${path}`, {
    method: "POST",
    headers: requestHeaders(true),
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!resp.ok) throw await responseError(resp);
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
  if (ACCESS_TOKEN) url.searchParams.set("token", ACCESS_TOKEN);
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
  scope: PathScope = "user"
): Promise<void> {
  await post<void>("/api/path/add", { toolId, scope });
}

export async function removeFromPath(
  toolId: string,
  scope: PathScope = "user"
): Promise<void> {
  await post<void>("/api/path/remove", { toolId, scope });
}

export async function getLogs(): Promise<string[]> {
  return get<string[]>("/api/logs");
}
