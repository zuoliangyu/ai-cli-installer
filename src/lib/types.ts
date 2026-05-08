export type InstallMethod = "native" | "npm";
export type InstallationSource = "native" | "npm_global" | "path";

export interface ToolInstallation {
  source: InstallationSource;
  version: string | null;
  path: string | null;
  current_path: boolean;
  managed: boolean;
}

export interface ToolDescriptor {
  id: string;
  name: string;
  description: string;
  installed_version: string | null;
  latest_version: string | null;
  stable_version: string | null;
  stable_falls_back_to_latest: boolean;
  installations: ToolInstallation[];
  install_path: string | null;
  supports_npm: boolean;
  npm_package: string | null;
  npm_min_node: number | null;
}

export interface InstallReport {
  tool_id: string;
  version: string;
  install_path: string;
  elapsed_secs: number;
  method: InstallMethod;
}

export interface NodeInfo {
  node_version: string;
  node_major: number;
  npm_version: string | null;
}

export type FixTargetFile = "claude_settings" | "claude_json";

export interface FixPatch {
  target: FixTargetFile;
  path: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  value: any;
}

export interface Fix {
  id: string;
  code: string;
  title: string;
  description: string;
  doc_url: string | null;
  patches: FixPatch[];
  configured: boolean;
  configured_patches: number;
  total_patches: number;
}

export interface ApplyFixReport {
  applied_count: number;
  touched_files: string[];
}

export interface RemoveFixReport {
  removed_count: number;
  touched_files: string[];
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

export type UnlistenFn = () => void;

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
