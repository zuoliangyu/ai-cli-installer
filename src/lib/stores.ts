import { writable, get } from "svelte/store";
import type {
  ToolDescriptor,
  MirrorProbe,
  DownloadProgress,
  InstallMethod,
} from "./types";

export const tools = writable<ToolDescriptor[]>([]);
export const mirrorProbes = writable<MirrorProbe[]>([]);
/** 镜像测速是否进行中（启动时的自动测速和手动重试共用）。 */
export const mirrorProbing = writable<boolean>(false);
/** 最近一次镜像测速的错误；成功后清空。 */
export const mirrorProbeError = writable<string | null>(null);

/** "auto" = race all mirrors (default). Any other value = pin to that
 * mirror name; back-end refuses to fall back so the user's choice is
 * honored even when it fails. */
export const AUTO_MIRROR = "auto";

export type InstallChannel = "latest" | "stable";

/** 单个工具的安装状态。放在模块级 store 里，离开「CLI 工具」页、
 * ToolCard 被销毁后状态依然保留，切回来能看到进行中的安装和结果。 */
export interface ToolInstallState {
  /** 安装 / 重新拉取版本进行中，期间禁用安装按钮 */
  busy: boolean;
  /** 正在或最近一次安装使用的通道 */
  channel: InstallChannel;
  /** 用户选择的安装方式 */
  method: InstallMethod;
  /** 用户选择的下载来源（AUTO_MIRROR 或镜像名） */
  mirror: string;
  progress: DownloadProgress | null;
  message: string | null;
  error: string | null;
  /** 最近一次失败的安装所指定的镜像；null 表示自动模式或错误与安装无关 */
  failedMirror: string | null;
}

export const DEFAULT_INSTALL_STATE: ToolInstallState = Object.freeze({
  busy: false,
  channel: "latest",
  method: "native",
  mirror: AUTO_MIRROR,
  progress: null,
  message: null,
  error: null,
  failedMirror: null,
});

/** 按 tool_id 存储的安装状态。 */
export const installStates = writable<Record<string, ToolInstallState>>({});

export function getInstallState(toolId: string): ToolInstallState {
  return get(installStates)[toolId] ?? DEFAULT_INSTALL_STATE;
}

export function patchInstallState(
  toolId: string,
  patch: Partial<ToolInstallState>
): void {
  installStates.update((all) => ({
    ...all,
    [toolId]: { ...(all[toolId] ?? DEFAULT_INSTALL_STATE), ...patch },
  }));
}
