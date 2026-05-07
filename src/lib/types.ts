export interface ToolDescriptor {
  id: string;
  name: string;
  description: string;
  installed_version: string | null;
  install_path: string | null;
}

export interface InstallReport {
  tool_id: string;
  version: string;
  install_path: string;
  elapsed_secs: number;
}

export interface DownloadProgress {
  tool_id: string;
  downloaded: number;
  total: number | null;
  mirror: string;
}

export interface MirrorProbe {
  name: string;
  ok: boolean;
  latency_ms: number | null;
  error: string | null;
}

export type Channel = "latest" | "stable" | string;

export interface PathStatus {
  dir: string;
  in_user_path: boolean;
  in_system_path: boolean;
  effective: boolean;
}

export type PathScope = "system" | "user";
