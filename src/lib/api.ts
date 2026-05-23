//! Unified API entry point. Picks the Tauri or Web implementation based on
//! the `__IS_TAURI__` Vite define. Each named export forwards to the matching
//! function in the loaded module — keeps existing import paths
//! (`from "../api"`) working unchanged across the Svelte components.

import type * as TauriApiNs from "./services/tauriApi";

type ApiModule = typeof TauriApiNs;

const apiModulePromise: Promise<ApiModule> = __IS_TAURI__
  ? import("./services/tauriApi")
  : (import("./services/webApi") as unknown as Promise<ApiModule>);

function bind<K extends keyof ApiModule>(name: K): ApiModule[K] {
  return ((...args: unknown[]) =>
    apiModulePromise.then((m) =>
      (m[name] as (...a: unknown[]) => unknown)(...args)
    )) as unknown as ApiModule[K];
}

export const initApp = bind("initApp");
export const refreshTools = bind("refreshTools");
export const probeMirrors = bind("probeMirrors");
export const installTool = bind("installTool");
export const detectNode = bind("detectNode");
export const listFixes = bind("listFixes");
export const applyFixes = bind("applyFixes");
export const removeFixes = bind("removeFixes");
export const openPath = bind("openPath");
export const onDownloadProgress = bind("onDownloadProgress");
export const checkPathStatus = bind("checkPathStatus");
export const addToPath = bind("addToPath");
export const removeFromPath = bind("removeFromPath");
export const listClaudePresets = bind("listClaudePresets");
export const getClaudeSettings = bind("getClaudeSettings");
export const applyClaudePreset = bind("applyClaudePreset");
export const getLogs = bind("getLogs");
