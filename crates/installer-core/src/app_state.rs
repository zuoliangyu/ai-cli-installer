//! Shared runtime state — held by both the Tauri shell and the Web shell.
//!
//! Holds the `reqwest::Client` (so connection pools are reused across
//! commands) and the in-memory `MirrorList`. Higher-level service helpers
//! consume `&AppState` and a `ProgressCallback`, then call into the rest
//! of the core crate.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};

use crate::env_manager::{self, PathScope, PathStatus};
use crate::error::{AppError, Result};
use crate::fixes::{self, ApplyReport, Fix, RemoveReport};
use crate::install_diagnostics::{self, InstallationSource};
use crate::mirrors::{self, MirrorList, MirrorProbe};
use crate::npm_installer::{self, NodeInfo};
use crate::progress::ProgressCallback;
use crate::tools::{
    claude_code::ClaudeCode, codex::CodexCli, InstallMethod, InstallReport, Tool, ToolDescriptor,
};
use crate::version_cache;

/// How long a successful channel → version lookup is reused in memory
/// (list refresh → the install that follows) before racing mirrors again.
const VERSION_MEMO_TTL: Duration = Duration::from_secs(5 * 60);

pub struct AppState {
    pub client: reqwest::Client,
    pub mirrors: RwLock<MirrorList>,
    /// Serializes installs and diagnostics. Introduced in v0.5.3: running a
    /// tool's `--version` while an install replaces it caused Windows
    /// file-in-use errors.
    tool_operations: Mutex<()>,
    /// Last successful `list_tools` result, served while an operation holds
    /// `tool_operations` so the UI doesn't freeze during an install.
    tools_cache: std::sync::Mutex<Option<Vec<ToolDescriptor>>>,
    /// `(tool_id, channel) → (version, fetched)`; see [`VERSION_MEMO_TTL`].
    version_memo: std::sync::Mutex<HashMap<(String, String), (String, Instant)>>,
}

impl AppState {
    pub fn new() -> Self {
        // GUI launches (Finder / desktop launchers) inherit a minimal PATH;
        // fix it before anything spawns `node` / `npm` / `claude`.
        crate::platform::ensure_login_shell_path();
        let client = reqwest::Client::builder()
            .user_agent(concat!("ai-cli-installer/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(60))
            .build()
            .expect("build reqwest client");
        Self::with_client(client)
    }

    fn with_client(client: reqwest::Client) -> Self {
        Self {
            client,
            mirrors: RwLock::new(MirrorList::builtin()),
            tool_operations: Mutex::new(()),
            tools_cache: std::sync::Mutex::new(None),
            version_memo: std::sync::Mutex::new(HashMap::new()),
        }
    }

    fn memo_get(&self, tool_id: &str, channel: &str) -> Option<String> {
        let memo = self.version_memo.lock().ok()?;
        memo.get(&(tool_id.to_string(), channel.to_string()))
            .filter(|(_, at)| at.elapsed() < VERSION_MEMO_TTL)
            .map(|(v, _)| v.clone())
    }

    fn memo_put(&self, tool_id: &str, channel: &str, version: &str) {
        if let Ok(mut memo) = self.version_memo.lock() {
            memo.insert(
                (tool_id.to_string(), channel.to_string()),
                (version.to_string(), Instant::now()),
            );
        }
    }

    fn cached_tools(&self) -> Option<Vec<ToolDescriptor>> {
        self.tools_cache.lock().ok()?.clone()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------- High-level service API ----------
//
// Each function below replaces one of the previous Tauri commands. Both
// `src-tauri/commands.rs` and `installer-web/routes.rs` call into these so
// the two shells share a single source of truth for behavior.

pub async fn list_tools(state: &AppState) -> Result<Vec<ToolDescriptor>> {
    // ponytail: global lock; split download/deploy phases if parallel installs matter.
    // While an install / diagnosis holds the lock, running `--version` on the
    // binaries being replaced is exactly what v0.5.3 serialized away — so
    // answer from the last result instead of blocking the UI for minutes.
    let _operation = match state.tool_operations.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            if let Some(cached) = state.cached_tools() {
                tracing::debug!("list_tools: operation in progress, serving cached result");
                return Ok(cached);
            }
            state.tool_operations.lock().await
        }
    };
    let tools = list_tools_uncached(state).await;
    if let Ok(mut cache) = state.tools_cache.lock() {
        *cache = Some(tools.clone());
    }
    Ok(tools)
}

async fn list_tools_uncached(state: &AppState) -> Vec<ToolDescriptor> {
    let cc = ClaudeCode;
    let mut cd = cc.descriptor();

    let cx = CodexCli;
    let mut xd = cx.descriptor();

    // Network (version races) and local probing run side by side; the
    // environment probe (`npm prefix -g`, `npm list -g`, `pnpm bin -g`,
    // `yarn global bin`) happens once and is shared by both tools.
    let ((cc_latest, cc_stable, cx_latest, cx_stable), (cc_installations, cx_installations)) = tokio::join!(
        async {
            tokio::join!(
                fetch_channel_version(state, &cc, "latest"),
                fetch_channel_version(state, &cc, "stable"),
                fetch_channel_version(state, &cx, "latest"),
                fetch_channel_version(state, &cx, "stable"),
            )
        },
        async {
            let env = install_diagnostics::probe_env().await;
            tokio::join!(
                install_diagnostics::diagnose_with(
                    &env,
                    "claude",
                    native_launcher_path(&cc, "claude"),
                    cc.launcher_dir(),
                    cc.npm_package(),
                ),
                install_diagnostics::diagnose_with(
                    &env,
                    "codex",
                    native_launcher_path(&cx, "codex"),
                    cx.launcher_dir(),
                    cx.npm_package(),
                ),
            )
        }
    );

    let (cc_latest, cc_latest_stale) = cc_latest;
    let (cc_stable_raw, cc_stable_stale_raw) = cc_stable;
    let (cx_latest, cx_latest_stale) = cx_latest;
    let (cx_stable_raw, cx_stable_stale_raw) = cx_stable;

    let (cc_stable, cc_falls_back, cc_stable_stale) =
        resolve_stable(cc_stable_raw, cc_stable_stale_raw, &cc_latest, cc_latest_stale);
    let (cx_stable, cx_falls_back, cx_stable_stale) =
        resolve_stable(cx_stable_raw, cx_stable_stale_raw, &cx_latest, cx_latest_stale);

    // `installed_version` comes from the diagnosis (which already ran
    // `--version` on every candidate) instead of a second detect pass.
    cd.installed_version = installed_from(&cc_installations);
    cd.latest_version = cc_latest;
    cd.latest_version_stale = cc_latest_stale;
    cd.stable_version = cc_stable;
    cd.stable_version_stale = cc_stable_stale;
    cd.stable_falls_back_to_latest = cc_falls_back;
    cd.installations = cc_installations;

    xd.installed_version = installed_from(&cx_installations);
    xd.latest_version = cx_latest;
    xd.latest_version_stale = cx_latest_stale;
    xd.stable_version = cx_stable;
    xd.stable_version_stale = cx_stable_stale;
    xd.stable_falls_back_to_latest = cx_falls_back;
    xd.installations = cx_installations;

    vec![cd, xd]
}

/// 从诊断结果推导顶部"已安装"版本，优先级与原 `Tool::detect_installed` 一致：
/// 应用自管的 native launcher → `where`/`command -v` 当前命中项（current_path）
/// → 任一带版本号的安装。桌面进程 PATH 不全或 .cmd shim 解析失败时，后两级兜底。
fn installed_from(installs: &[install_diagnostics::ToolInstallation]) -> Option<String> {
    installs
        .iter()
        .find(|i| i.source == InstallationSource::Native && i.managed && i.version.is_some())
        .or_else(|| installs.iter().find(|i| i.current_path && i.version.is_some()))
        .or_else(|| installs.iter().find(|i| i.version.is_some()))
        .and_then(|i| i.version.clone())
}

pub async fn list_mirrors(state: &AppState) -> Result<MirrorList> {
    Ok(state.mirrors.read().await.clone())
}

pub async fn probe_mirrors(state: &AppState) -> Result<Vec<MirrorProbe>> {
    let list = state.mirrors.read().await.clone();
    Ok(mirrors::probe_all(&state.client, &list).await)
}

pub async fn install_tool(
    state: &AppState,
    progress: ProgressCallback,
    tool_id: &str,
    channel: Option<String>,
    method: Option<InstallMethod>,
    mirror: Option<String>,
) -> Result<InstallReport> {
    let _operation = state.tool_operations.lock().await;
    let requested_channel = channel.unwrap_or_else(|| "latest".to_string());
    let method = method.unwrap_or_default();
    let client = state.client.clone();
    match tool_id {
        ClaudeCode::ID => {
            let mirrors = filter_mirrors(ClaudeCode.mirror_list(), mirror.as_deref())?;
            let (channel, version) =
                resolve_install_version(state, &ClaudeCode, requested_channel, &mirrors).await?;
            ClaudeCode
                .install_version(method, progress, client, mirrors, channel, version)
                .await
        }
        CodexCli::ID => {
            let mirrors = filter_mirrors(CodexCli.mirror_list(), mirror.as_deref())?;
            let (channel, version) =
                resolve_install_version(state, &CodexCli, requested_channel, &mirrors).await?;
            CodexCli
                .install_version(method, progress, client, mirrors, channel, version)
                .await
        }
        other => Err(AppError::Other(format!("unknown tool: {}", other))),
    }
}

/// If the caller pinned a specific mirror by name, narrow the list to that
/// one entry. Empty after filtering = the name didn't match anything →
/// surface an actionable error instead of silently falling back to "auto"
/// (which would let the install proceed with the full list, masking the
/// user's explicit choice). `None` means "auto mode" — full list.
fn filter_mirrors(mut list: MirrorList, name: Option<&str>) -> Result<MirrorList> {
    let Some(name) = name else {
        return Ok(list);
    };
    list.mirrors.retain(|m| m.name() == name);
    if list.mirrors.is_empty() {
        return Err(AppError::Other(format!(
            "未知镜像: `{}`。请改回「自动」或换一个有效镜像。",
            name
        )));
    }
    Ok(list)
}

pub async fn detect_node() -> Result<NodeInfo> {
    npm_installer::detect_node().await
}

pub async fn list_fixes(state: &AppState) -> Result<Vec<Fix>> {
    fixes::list_fixes(&state.client).await
}

pub async fn apply_fixes(state: &AppState, fix_ids: &[String]) -> Result<ApplyReport> {
    fixes::apply_selected(&state.client, fix_ids).await
}

pub async fn remove_fixes(state: &AppState, fix_ids: &[String]) -> Result<RemoveReport> {
    fixes::remove_selected(&state.client, fix_ids).await
}

pub async fn check_path_status(tool_id: &str) -> Result<PathStatus> {
    let dir = launcher_dir_for(tool_id)?;
    env_manager::status(&dir).await
}

pub async fn add_to_path(tool_id: &str, scope: PathScope) -> Result<()> {
    let dir = launcher_dir_for(tool_id)?;
    env_manager::add(&dir, scope).await
}

pub async fn remove_from_path(tool_id: &str, scope: PathScope) -> Result<()> {
    let dir = launcher_dir_for(tool_id)?;
    env_manager::remove(&dir, scope).await
}

/// Open a JSON config file with the system's default associated app.
/// Whitelisted to `.json` files only — the UI only ever clicks paths produced
/// by `apply_fixes` / `remove_fixes`, which always write JSON.
pub fn open_path(path: &str) -> Result<()> {
    let raw = PathBuf::from(path);
    let canonical = raw
        .canonicalize()
        .map_err(|e| AppError::Other(format!("path not found: {} ({})", raw.display(), e)))?;

    let metadata = std::fs::metadata(&canonical)
        .map_err(|e| AppError::Other(format!("stat {}: {}", canonical.display(), e)))?;
    if !metadata.is_file() {
        return Err(AppError::Other(format!(
            "refusing to open non-file path: {}",
            canonical.display()
        )));
    }

    let ext_ok = canonical
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"));
    if !ext_ok {
        return Err(AppError::Other(format!(
            "refusing to open non-json path: {}",
            canonical.display()
        )));
    }

    open_path_with_system(&canonical)
}

/// The path is canonical and absolute, so `open` / `xdg-open` can't mistake
/// it for an option. On Windows we avoid `cmd /c start`, whose parser
/// interprets `&`, `^`, `%` in the path; `explorer.exe` takes the path as a
/// plain argument but doesn't understand the verbatim prefix that
/// `canonicalize` adds, so that's stripped first.
fn open_path_with_system(path: &std::path::Path) -> Result<()> {
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut cmd = std::process::Command::new("explorer.exe");
        cmd.arg(strip_verbatim_prefix(path));
        crate::proc::silence_windows_std(&mut cmd);
        cmd
    };

    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut cmd = std::process::Command::new("open");
        cmd.arg(path);
        cmd
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut cmd = std::process::Command::new("xdg-open");
        cmd.arg(path);
        cmd
    };

    let mut child = cmd.spawn()?;
    // Reap the launcher so it doesn't linger as a zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Turn a verbatim `\\?\X:\...` path into `X:\...` and `\\?\UNC\srv\share`
/// into `\\srv\share` (what `dunce::simplified` does, minus the dependency).
/// Other verbatim forms have no plain spelling and are returned unchanged.
#[cfg(any(windows, test))]
fn strip_verbatim_prefix(path: &std::path::Path) -> PathBuf {
    const VERBATIM: &str = "\\\\?\\";
    const VERBATIM_UNC: &str = "\\\\?\\UNC\\";
    let Some(s) = path.to_str() else {
        return path.to_path_buf();
    };
    if let Some(rest) = s.strip_prefix(VERBATIM_UNC) {
        return PathBuf::from(format!("\\\\{}", rest));
    }
    match s.strip_prefix(VERBATIM) {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
}

// ---------- Helpers ----------

fn launcher_dir_for(tool_id: &str) -> Result<PathBuf> {
    match tool_id {
        ClaudeCode::ID => ClaudeCode
            .launcher_dir()
            .ok_or_else(|| AppError::Other("home dir not available".into())),
        CodexCli::ID => CodexCli
            .launcher_dir()
            .ok_or_else(|| AppError::Other("home dir not available".into())),
        other => Err(AppError::Other(format!("unknown tool: {}", other))),
    }
}

/// Resolve a stable-channel version, falling back to latest when the mirror
/// has no separate stable pointer. Returns `(version, falls_back, stale)`:
/// - `falls_back` true → button labels with "跟随 latest" (existing behavior)
/// - `stale` true → version came from cache (either stable cache directly, or
///   latest cache when we fell back). UI suffixes "缓存".
fn resolve_stable(
    stable: Option<String>,
    stable_stale: bool,
    latest: &Option<String>,
    latest_stale: bool,
) -> (Option<String>, bool, bool) {
    match stable {
        Some(v) => (Some(v), false, stable_stale),
        None => (latest.clone(), latest.is_some(), latest_stale),
    }
}

/// Returns `(version, stale)`. `stale` is true when the version came from the
/// on-disk fallback cache rather than a fresh mirror response; the UI uses
/// this to label the button with a "缓存" suffix. When even the cache is
/// empty, returns `(None, false)` and the UI shifts the button into its
/// destructive retry state. Fresh results are memoized for
/// [`VERSION_MEMO_TTL`].
async fn fetch_channel_version<T: Tool>(
    state: &AppState,
    tool: &T,
    channel: &str,
) -> (Option<String>, bool) {
    let tool_id = tool.id();
    if let Some(v) = state.memo_get(tool_id, channel) {
        return (Some(v), false);
    }
    let mirrors = tool.mirror_list();
    // fetch_version itself has a PER_MIRROR_TIMEOUT (8s) ceiling on each
    // racer, so the whole call bottoms out at ~8s worst-case. No need for
    // an outer wrapper timeout (pre-v0.5 we had Duration::from_secs(10)
    // here as a guard against the old sequential-loop fetch_version).
    let fresh = mirrors::fetch_version(&state.client, &mirrors, channel)
        .await
        .ok()
        .map(|(_, version)| version);

    match fresh {
        Some(v) => {
            state.memo_put(tool_id, channel, &v);
            version_cache::record(tool_id, channel, &v);
            (Some(v), false)
        }
        None => match version_cache::get(tool_id, channel) {
            Some(cached) => {
                tracing::warn!(
                    "version fetch failed for {}/{} — falling back to cached {}",
                    tool_id,
                    channel,
                    cached
                );
                (Some(cached), true)
            }
            None => (None, false),
        },
    }
}

fn native_launcher_path<T: Tool>(tool: &T, command_name: &str) -> Option<PathBuf> {
    let file_name = if cfg!(target_os = "windows") {
        format!("{}.exe", command_name)
    } else {
        command_name.to_string()
    };
    Some(tool.launcher_dir()?.join(file_name))
}

/// Decide the channel and the exact version to install, looking the version
/// up only once (a fresh memo from the preceding list refresh is reused).
///
/// Treat a cached stable version (stale=true) as "stable exists" — the
/// actual binary fetch downstream will hit the mirror chain itself and
/// surface AllMirrorsFailed if the network is still down. Better to honor
/// the user's stable pick than to silently jump to latest. A stale cache
/// entry is never used as the install version itself; that still needs a
/// fresh answer from the (possibly user-pinned) mirrors.
async fn resolve_install_version<T: Tool>(
    state: &AppState,
    tool: &T,
    channel: String,
    mirrors: &MirrorList,
) -> Result<(String, String)> {
    let channel = if channel == "stable"
        && fetch_channel_version(state, tool, "stable")
            .await
            .0
            .is_none()
    {
        "latest".to_string()
    } else {
        channel
    };
    if let Some(v) = state.memo_get(tool.id(), &channel) {
        tracing::info!("{} {} -> {} (memoized)", tool.id(), channel, v);
        return Ok((channel, v));
    }
    let (_, version) = mirrors::fetch_version(&state.client, mirrors, &channel).await?;
    tracing::info!("{} resolved {} -> {}", tool.id(), channel, version);
    state.memo_put(tool.id(), &channel, &version);
    version_cache::record(tool.id(), &channel, &version);
    Ok((channel, version))
}

/// Helper for Tauri / Axum shells to wrap their Arc-based state without each
/// having to know how the inner type is constructed.
pub fn shared() -> Arc<AppState> {
    Arc::new(AppState::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install_diagnostics::ToolInstallation;

    fn inst(source: InstallationSource, version: &str, current: bool, managed: bool) -> ToolInstallation {
        ToolInstallation {
            source,
            version: Some(version.to_string()),
            path: None,
            current_path: current,
            on_path: current,
            managed,
        }
    }

    #[test]
    fn installed_version_prefers_managed_native() {
        let installs = vec![
            inst(InstallationSource::NpmGlobal, "1.0.0", true, false),
            inst(InstallationSource::Native, "2.0.0", false, true),
        ];
        assert_eq!(installed_from(&installs).as_deref(), Some("2.0.0"));
        let installs = vec![
            inst(InstallationSource::Nvm, "0.9.0", false, false),
            inst(InstallationSource::NpmGlobal, "1.0.0", true, false),
        ];
        assert_eq!(installed_from(&installs).as_deref(), Some("1.0.0"));
        assert_eq!(installed_from(&[]), None);
    }

    #[test]
    fn strips_verbatim_prefixes() {
        let bs = "\\";
        let drive = format!("C:{bs}Users{bs}a b{bs}x.json");
        let verbatim = format!("{bs}{bs}?{bs}{drive}");
        assert_eq!(strip_verbatim_prefix(std::path::Path::new(&verbatim)), PathBuf::from(&drive));

        let unc = format!("{bs}{bs}?{bs}UNC{bs}srv{bs}share{bs}x.json");
        assert_eq!(
            strip_verbatim_prefix(std::path::Path::new(&unc)),
            PathBuf::from(format!("{bs}{bs}srv{bs}share{bs}x.json"))
        );

        assert_eq!(strip_verbatim_prefix(std::path::Path::new(&drive)), PathBuf::from(&drive));
        let volume = format!("{bs}{bs}?{bs}Volume{{abc}}{bs}x.json");
        assert_eq!(strip_verbatim_prefix(std::path::Path::new(&volume)), PathBuf::from(&volume));
    }

    #[test]
    fn version_memo_expires() {
        let state = AppState::with_client(reqwest::Client::new());
        state.memo_put("t", "latest", "1.2.3");
        assert_eq!(state.memo_get("t", "latest").as_deref(), Some("1.2.3"));
        assert_eq!(state.memo_get("t", "stable"), None);
        state.version_memo.lock().unwrap().insert(
            ("t".into(), "latest".into()),
            ("1.2.3".into(), Instant::now() - VERSION_MEMO_TTL - Duration::from_secs(1)),
        );
        assert_eq!(state.memo_get("t", "latest"), None);
    }

    fn assert_send<T: Send>(_: &T) {}

    /// axum handlers and tauri commands need `Send` futures; a borrowed
    /// iterator held across `.await` once broke installer-web's build.
    #[test]
    fn service_futures_are_send() {
        let state = AppState::with_client(reqwest::Client::new());
        let install = install_tool(
            &state,
            crate::progress::noop_progress(),
            ClaudeCode::ID,
            None,
            None,
            None,
        );
        assert_send(&install);
        assert_send(&list_tools(&state));
        assert_send(&apply_fixes(&state, &[]));
        assert_send(&add_to_path(ClaudeCode::ID, PathScope::User));
    }

    #[tokio::test]
    async fn list_tools_serves_cache_while_an_operation_runs() {
        let state = AppState::with_client(reqwest::Client::new());
        let mut cached = ClaudeCode.descriptor();
        cached.installed_version = Some("9.9.9".into());
        *state.tools_cache.lock().unwrap() = Some(vec![cached]);

        let _busy = state.tool_operations.lock().await;
        let tools = tokio::time::timeout(Duration::from_secs(2), list_tools(&state))
            .await
            .expect("must not wait for the operation lock")
            .unwrap();
        assert_eq!(tools[0].installed_version.as_deref(), Some("9.9.9"));
    }
}
