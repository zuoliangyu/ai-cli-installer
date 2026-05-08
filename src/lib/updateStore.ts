import { writable, get } from "svelte/store";

declare const __IS_TAURI__: boolean;

export type UpdateStatus =
  | "idle"
  | "checking"
  | "available"
  | "downloading"
  | "installing"
  | "error";

export interface UpdateState {
  status: UpdateStatus;
  currentVersion: string;
  newVersion: string | null;
  releaseNotes: string | null;
  downloadProgress: number;
  dismissed: boolean;
  errorMessage: string | null;
  /** 启动后是否已经触发过一次自动检查（避免每次切页都跑） */
  startupChecked: boolean;
  /** 启动后是否已经弹过确认框（避免同一会话弹两次） */
  startupPrompted: boolean;
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
  startupChecked: false,
  startupPrompted: false,
};

export const updateState = writable<UpdateState>(initial);

function patch(p: Partial<UpdateState>) {
  updateState.update((s) => ({ ...s, ...p }));
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
  patch({ status: "checking", errorMessage: null });
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    const update = await check();
    if (update) {
      const dismissedVersion = localStorage.getItem(DISMISSED_VERSION_KEY);
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
    patch({ status: "error", errorMessage: String(e) });
  }
}

export async function downloadAndInstall(): Promise<void> {
  if (!__IS_TAURI__) return;
  patch({ status: "downloading", downloadProgress: 0, errorMessage: null });
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    const { relaunch } = await import("@tauri-apps/plugin-process");
    const update = await check();
    if (!update) {
      patch({ status: "idle" });
      return;
    }

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
    patch({ status: "error", errorMessage: String(e) });
  }
}

export async function openDownloadPage(): Promise<void> {
  const { newVersion } = get(updateState);
  const tag = newVersion ? `v${newVersion}` : "latest";
  const url = `https://github.com/zuoliangyu/ai-cli-installer/releases/tag/${tag}`;
  if (!__IS_TAURI__) {
    window.open(url, "_blank");
    return;
  }
  try {
    const { open } = await import("@tauri-apps/plugin-shell");
    await open(url);
  } catch {
    window.open(url, "_blank");
  }
}

export function dismiss(): void {
  const { newVersion } = get(updateState);
  if (newVersion) {
    localStorage.setItem(DISMISSED_VERSION_KEY, newVersion);
  }
  patch({ dismissed: true });
}

/** 启动时跑一次：加载版本号 + 静默检查 + 发现新版时弹一次系统对话框 */
export async function runStartupCheck(): Promise<void> {
  if (!__IS_TAURI__) return;
  const cur = get(updateState);
  if (cur.startupChecked) return;
  patch({ startupChecked: true });

  await loadCurrentVersion();

  // 给 UI 1.5 秒安顿一下再发请求，避免和 list_tools 的网络抢资源
  await new Promise((r) => setTimeout(r, 1500));
  await checkForUpdate();

  const after = get(updateState);
  if (
    after.status !== "available" ||
    !after.newVersion ||
    after.dismissed ||
    after.startupPrompted
  ) {
    return;
  }
  patch({ startupPrompted: true });

  try {
    const { confirm } = await import("@tauri-apps/plugin-dialog");
    const ok = await confirm(
      `当前版本 v${after.currentVersion || "?"}，检测到新版本 v${after.newVersion}。是否立即更新并在完成后重启？`,
      {
        title: "发现新版本",
        kind: "info",
        okLabel: "立即更新",
        cancelLabel: "暂不更新",
      }
    );
    if (ok) {
      await downloadAndInstall();
    }
  } catch (e) {
    console.warn("Startup update prompt failed:", e);
  }
}
