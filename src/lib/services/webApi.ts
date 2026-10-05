//! Web-mode transport. Hits the Axum HTTP routes exposed by `installer-web`
//! and subscribes to download progress through `/ws/progress`. 与
//! `tauriApi.ts` 一样实现 `ApiTransport`，由 `../api.ts` 在运行时选择；
//! store 写入统一在 `../api.ts`，这里只负责传输。

import type {
  ApiTransport,
  ToolDescriptor,
  InstallReport,
  DownloadProgress,
  MirrorProbe,
  PathStatus,
  NodeInfo,
  Fix,
  ApplyFixReport,
  RemoveFixReport,
  LogChunk,
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

/** WebSocket 断线重连的退避参数：1s 起步，每次翻倍，最多 30s。 */
const WS_RETRY_BASE_MS = 1000;
const WS_RETRY_MAX_MS = 30_000;

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
  const text = (await resp.text().catch(() => "")).trim();
  return new Error(text || `${resp.status} ${resp.statusText}`.trim());
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

function progressSocketUrl(): URL {
  // `new URL("/ws/progress", origin)` keeps host + port; switching the
  // protocol from http(s) to ws(s) is the only edit we need.
  const url = new URL("/ws/progress", window.location.origin);
  url.protocol = url.protocol.replace(/^http/, "ws");
  if (ACCESS_TOKEN) url.searchParams.set("token", ACCESS_TOKEN);
  return url;
}

/** 打开 `/ws/progress`，断开（服务重启、系统休眠等）后按指数退避自动重连；
 * 调用返回的函数主动取消订阅后不再重连。 */
function subscribeProgress(cb: (p: DownloadProgress) => void): Promise<UnlistenFn> {
  let sock: WebSocket | null = null;
  let retryTimer: ReturnType<typeof setTimeout> | null = null;
  let attempt = 0;
  let stopped = false;

  const scheduleReconnect = () => {
    if (stopped || retryTimer !== null) return;
    const delay = Math.min(WS_RETRY_BASE_MS * 2 ** attempt, WS_RETRY_MAX_MS);
    attempt += 1;
    retryTimer = setTimeout(() => {
      retryTimer = null;
      connect();
    }, delay);
  };

  const connect = () => {
    if (stopped) return;
    let s: WebSocket;
    try {
      s = new WebSocket(progressSocketUrl());
    } catch {
      scheduleReconnect();
      return;
    }
    sock = s;
    s.addEventListener("open", () => {
      attempt = 0;
    });
    s.addEventListener("message", (ev) => {
      try {
        cb(JSON.parse(ev.data) as DownloadProgress);
      } catch {
        // ignore malformed payloads
      }
    });
    // `error` 之后浏览器总会再派发 `close`，重连统一在 close 里处理。
    s.addEventListener("close", () => {
      if (sock !== s) return;
      sock = null;
      scheduleReconnect();
    });
  };

  connect();

  return Promise.resolve(() => {
    stopped = true;
    if (retryTimer !== null) {
      clearTimeout(retryTimer);
      retryTimer = null;
    }
    const s = sock;
    sock = null;
    s?.close();
  });
}

export const webApi: ApiTransport = {
  listTools: () => get<ToolDescriptor[]>("/api/tools"),

  probeMirrors: () => post<MirrorProbe[]>("/api/mirrors/probe"),

  installTool: (toolId, channel, method, mirror) =>
    post<InstallReport>("/api/tools/install", { toolId, channel, method, mirror }),

  detectNode: () => get<NodeInfo>("/api/node"),

  listFixes: () => get<Fix[]>("/api/fixes"),

  applyFixes: (fixIds) => post<ApplyFixReport>("/api/fixes/apply", { fixIds }),

  removeFixes: (fixIds) => post<RemoveFixReport>("/api/fixes/remove", { fixIds }),

  openPath: async (path) => {
    await post<void>("/api/open-path", { path });
  },

  subscribeProgress,

  checkPathStatus: (toolId) => get<PathStatus>("/api/path/status", { toolId }),

  addToPath: async (toolId, scope) => {
    await post<void>("/api/path/add", { toolId, scope });
  },

  removeFromPath: async (toolId, scope) => {
    await post<void>("/api/path/remove", { toolId, scope });
  },

  getLogs: (since) =>
    get<LogChunk>("/api/logs", since === null ? undefined : { since: String(since) }),
};
