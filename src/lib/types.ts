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

export type PresetSource = "builtin" | "cc_switch";

export interface ClaudePreset {
  id: string;
  name: string;
  base_url: string;
  website_url: string | null;
  api_key_url: string | null;
  source: PresetSource;
}

export interface ClaudeSettingsEnv {
  anthropic_base_url: string | null;
  anthropic_auth_token: string | null;
}
