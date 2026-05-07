import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { tools, mirrorProbes } from "./stores";
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
} from "./types";

export async function initApp(): Promise<void> {
  const list = await invoke<ToolDescriptor[]>("list_tools");
  tools.set(list);

  // Probe mirrors lazily, don't block first paint
  invoke<MirrorProbe[]>("probe_mirrors")
    .then((probes) => mirrorProbes.set(probes))
    .catch(() => mirrorProbes.set([]));
}

export async function refreshTools(): Promise<void> {
  const list = await invoke<ToolDescriptor[]>("list_tools");
  tools.set(list);
}

export async function probeMirrors(): Promise<MirrorProbe[]> {
  const probes = await invoke<MirrorProbe[]>("probe_mirrors");
  mirrorProbes.set(probes);
  return probes;
}

export async function installTool(
  toolId: string,
  channel: Channel = "latest",
  method: InstallMethod = "native"
): Promise<InstallReport> {
  return invoke<InstallReport>("install_tool", { toolId, channel, method });
}

export async function detectNode(): Promise<NodeInfo> {
  return invoke<NodeInfo>("detect_node");
}

export function onDownloadProgress(
  cb: (p: DownloadProgress) => void
): Promise<UnlistenFn> {
  return listen<DownloadProgress>("download-progress", (e) => cb(e.payload));
}

export async function checkPathStatus(toolId: string): Promise<PathStatus> {
  return invoke<PathStatus>("check_path_status", { toolId });
}

export async function addToPath(
  toolId: string,
  scope: PathScope = "system"
): Promise<void> {
  await invoke<void>("add_to_path", { toolId, scope });
}

export async function removeFromPath(
  toolId: string,
  scope: PathScope = "system"
): Promise<void> {
  await invoke<void>("remove_from_path", { toolId, scope });
}

export async function listClaudePresets(): Promise<ClaudePreset[]> {
  return invoke<ClaudePreset[]>("list_claude_presets");
}

export async function getClaudeSettings(): Promise<ClaudeSettingsEnv> {
  return invoke<ClaudeSettingsEnv>("get_claude_settings");
}

export async function applyClaudePreset(
  baseUrl: string,
  authToken: string
): Promise<void> {
  await invoke<void>("apply_claude_preset", {
    baseUrl,
    authToken,
  });
}
