import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { tools, mirrorProbes } from "./stores";
import type {
  ToolDescriptor,
  InstallReport,
  DownloadProgress,
  MirrorProbe,
  Channel,
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
  channel: Channel = "latest"
): Promise<InstallReport> {
  return invoke<InstallReport>("install_tool", { toolId, channel });
}

export function onDownloadProgress(
  cb: (p: DownloadProgress) => void
): Promise<UnlistenFn> {
  return listen<DownloadProgress>("download-progress", (e) => cb(e.payload));
}
