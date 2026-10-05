export type InstallMethod = "native" | "npm";
export type InstallationSource =
  | "native"
  | "npm_global"
  | "pnpm"
  | "yarn"
  | "bun"
  | "nvm"
  | "path";

export interface ToolInstallation {
  source: InstallationSource;
  version: string | null;
  path: string | null;
  /** `where <cmd>` 当前解析到这一项 */
  current_path: boolean;
  /** 该项目录在 PATH 中（即便不是当前 winner） */
  on_path: boolean;
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
  /** version came from the on-disk cache, not a fresh mirror response */
  latest_version_stale: boolean;
  stable_version_stale: boolean;
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
  /** Fix IDs auto-applied as part of this install (e.g.
   * `"cc-005-onboarding-done"` for Claude Code). UI surfaces these as a
   * follow-up note so users know we touched their settings file. */
  auto_applied_fixes: string[];
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
  tags: string[];
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

/** 增量日志拉取结果。`next` 是下一行的序号，下次请求原样传回；
 * `reset` 为 true 时应替换本地全部行（后端日志被截断/重启），否则追加。 */
export interface LogChunk {
  lines: string[];
  next: number;
  reset: boolean;
}

/** Tauri 与 Web 两种传输层都必须满足的接口：只负责传输，不读写任何 store。
 * `services/tauriApi.ts` / `services/webApi.ts` 以显式类型标注导出实现，
 * 签名不一致会在编译期报错。 */
export interface ApiTransport {
  listTools(): Promise<ToolDescriptor[]>;
  probeMirrors(): Promise<MirrorProbe[]>;
  installTool(
    toolId: string,
    channel: Channel,
    method: InstallMethod,
    mirror: string | null
  ): Promise<InstallReport>;
  detectNode(): Promise<NodeInfo>;
  listFixes(): Promise<Fix[]>;
  applyFixes(fixIds: string[]): Promise<ApplyFixReport>;
  removeFixes(fixIds: string[]): Promise<RemoveFixReport>;
  openPath(path: string): Promise<void>;
  /** 订阅下载进度；返回的函数用于主动取消订阅。 */
  subscribeProgress(cb: (p: DownloadProgress) => void): Promise<UnlistenFn>;
  checkPathStatus(toolId: string): Promise<PathStatus>;
  addToPath(toolId: string, scope: PathScope): Promise<void>;
  removeFromPath(toolId: string, scope: PathScope): Promise<void>;
  /** `since` 为 null 时拉取全部日志。 */
  getLogs(since: number | null): Promise<LogChunk>;
}
