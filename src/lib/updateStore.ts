import { writable, get } from "svelte/store";
import type { Update } from "@tauri-apps/plugin-updater";
import { openExternalUrl } from "./openExternal";

declare const __IS_TAURI__: boolean;

export type UpdateStatus =
  | "idle"
  | "checking"
  | "available"
  | "downloading"
  | "installing"
  | "error";

/** 出错的环节：检查更新 / 下载安装更新，用于展示不同的文案。 */
export type UpdateErrorKind = "check" | "install";

export interface UpdateState {
  status: UpdateStatus;
  currentVersion: string;
  newVersion: string | null;
  releaseNotes: string | null;
  downloadProgress: number;
  dismissed: boolean;
  errorMessage: string | null;
  errorKind: UpdateErrorKind | null;
  /** 启动后是否已经触发过一次自动检查（避免每次切页都跑） */
  startupChecked: boolean;
}

const DISMISSED_VERSION_KEY = "aci_update_dismissed_version";

const initial: UpdateState = {
  status: "idle",
  currentVersion: "",
  newVersion: null,
  releaseNotes: null,
  downloadProgress: 0,
  dismissed: false,
  errorMessage: null,
  errorKind: null,
  startupChecked: false,
};

export const updateState = writable<UpdateState>(initial);

function patch(p: Partial<UpdateState>) {
  updateState.update((s) => ({ ...s, ...p }));
}

/** 最近一次 check() 得到的 Update 对象，安装时直接复用，不再重复 check。 */
let pendingUpdate: Update | null = null;

function setPendingUpdate(update: Update | null) {
  if (pendingUpdate && pendingUpdate !== update) {
    // 释放旧的 Rust 侧资源
    pendingUpdate.close().catch(() => {});
  }
  pendingUpdate = update;
}

function readDismissedVersion(): string | null {
  try {
    return localStorage.getItem(DISMISSED_VERSION_KEY);
  } catch {
    return null;
  }
}

export async function loadCurrentVersion(): Promise<void> {
  if (!__IS_TAURI__) return;
  try {
    const { getVersion } = await import("@tauri-apps/api/app");
    const v = await getVersion();
    patch({ currentVersion: v });
  } catch {
    // ignore
  }
}

export async function checkForUpdate(): Promise<void> {
  if (!__IS_TAURI__) return;
  const { status } = get(updateState);
  if (status === "checking" || status === "downloading" || status === "installing") return;
  patch({ status: "checking", errorMessage: null, errorKind: null });
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    const update = await check();
    setPendingUpdate(update);
    if (update) {
      const dismissedVersion = readDismissedVersion();
      const isDismissed = dismissedVersion === update.version;
      patch({
        status: "available",
        newVersion: update.version,
        releaseNotes: update.body ?? null,
        dismissed: isDismissed,
      });
    } else {
      patch({ status: "idle", newVersion: null, releaseNotes: null });
    }
  } catch (e) {
    console.warn("Update check failed:", e);
    setPendingUpdate(null);
    patch({ status: "error", errorMessage: String(e), errorKind: "check" });
  }
}

export async function downloadAndInstall(): Promise<void> {
  if (!__IS_TAURI__) return;
  // 防重入：只有「有可用更新」时才开始；下载/安装中再次点击直接忽略。
  const update = pendingUpdate;
  if (get(updateState).status !== "available" || !update) return;
  patch({ status: "downloading", downloadProgress: 0, errorMessage: null, errorKind: null });
  try {
    const { relaunch } = await import("@tauri-apps/plugin-process");

    let totalLength = 0;
    let downloaded = 0;

    await update.downloadAndInstall((event) => {
      switch (event.event) {
        case "Started":
          totalLength = event.data.contentLength ?? 0;
          break;
        case "Progress":
          downloaded += event.data.chunkLength;
          if (totalLength > 0) {
            patch({
              downloadProgress: Math.round((downloaded / totalLength) * 100),
            });
          }
          break;
        case "Finished":
          patch({ status: "installing", downloadProgress: 100 });
          break;
      }
    });

    await relaunch();
  } catch (e) {
    console.error("Update install failed:", e);
    patch({ status: "error", errorMessage: String(e), errorKind: "install" });
  }
}

export async function openDownloadPage(): Promise<void> {
  const { newVersion } = get(updateState);
  const tag = newVersion ? `v${newVersion}` : "latest";
  const url = `https://github.com/zuoliangyu/ai-cli-installer/releases/tag/${tag}`;
  await openExternalUrl(url);
}

export function dismiss(): void {
  const { newVersion } = get(updateState);
  if (newVersion) {
    try {
      localStorage.setItem(DISMISSED_VERSION_KEY, newVersion);
    } catch {
      // 存储不可用时仅在本次会话内忽略
    }
  }
  patch({ dismissed: true });
}

/** 启动时跑一次：加载版本号并静默检查，结果由非阻塞提示展示。 */
export async function runStartupCheck(): Promise<void> {
  if (!__IS_TAURI__) return;
  const cur = get(updateState);
  if (cur.startupChecked) return;
  patch({ startupChecked: true });

  await loadCurrentVersion();

  // 给 UI 1.5 秒安顿一下再发请求，避免和 list_tools 的网络抢资源
  await new Promise((r) => setTimeout(r, 1500));
  await checkForUpdate();
}
