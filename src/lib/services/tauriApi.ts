//! Tauri-mode transport. Calls the Rust commands registered in
//! `src-tauri/src/lib.rs` through `invoke()`, and listens to the
//! `download-progress` event for the streaming install progress.
//! Store 写入统一在 `../api.ts`，这里只负责传输。

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
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
} from "../types";

export const tauriApi: ApiTransport = {
  listTools: () => invoke<ToolDescriptor[]>("list_tools"),

  probeMirrors: () => invoke<MirrorProbe[]>("probe_mirrors"),

  installTool: (toolId, channel, method, mirror) =>
    invoke<InstallReport>("install_tool", { toolId, channel, method, mirror }),

  detectNode: () => invoke<NodeInfo>("detect_node"),

  listFixes: () => invoke<Fix[]>("list_fixes"),

  applyFixes: (fixIds) => invoke<ApplyFixReport>("apply_fixes", { fixIds }),

  removeFixes: (fixIds) => invoke<RemoveFixReport>("remove_fixes", { fixIds }),

  openPath: async (path) => {
    await invoke<void>("open_path", { path });
  },

  subscribeProgress: (cb) =>
    listen<DownloadProgress>("download-progress", (e) => cb(e.payload)),

  checkPathStatus: (toolId) => invoke<PathStatus>("check_path_status", { toolId }),

  addToPath: async (toolId, scope) => {
    await invoke<void>("add_to_path", { toolId, scope });
  },

  removeFromPath: async (toolId, scope) => {
    await invoke<void>("remove_from_path", { toolId, scope });
  },

  getLogs: (since) => invoke<LogChunk>("get_logs", { since }),
};
