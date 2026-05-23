use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::error::Result;
use crate::install_diagnostics::ToolInstallation;
use crate::mirrors::MirrorList;
use crate::progress::ProgressCallback;

pub type ToolId = &'static str;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstallMethod {
    /// Download native binary from our mirror, verify SHA256, place on disk.
    Native,
    /// Run `npm install -g <package>`. Requires Node.js on the user's machine.
    Npm,
}

impl Default for InstallMethod {
    fn default() -> Self {
        Self::Native
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolDescriptor {
    pub id: String,
    pub name: String,
    pub description: String,
    pub installed_version: Option<String>,
    pub latest_version: Option<String>,
    pub stable_version: Option<String>,
    /// True when `stable_version` is actually the resolved `latest` value
    /// because the mirror has no separate stable channel pointer. UI should
    /// label the stable button accordingly so the version number isn't
    /// misattributed.
    pub stable_falls_back_to_latest: bool,
    /// True when `latest_version` came from the on-disk fallback cache rather
    /// than a fresh mirror response. UI marks the button with a "缓存"
    /// suffix so the user knows the number may be out of date.
    pub latest_version_stale: bool,
    /// Same idea as `latest_version_stale`, for the stable channel. When
    /// `stable_falls_back_to_latest` is true, this mirrors the latest flag.
    pub stable_version_stale: bool,
    pub installations: Vec<ToolInstallation>,
    pub install_path: Option<String>,
    /// True if this tool can be installed via npm. UI surfaces the toggle.
    pub supports_npm: bool,
    /// npm package name (for display / docs). None when `supports_npm` is false.
    pub npm_package: Option<String>,
    /// Minimum Node.js major version required for npm route.
    pub npm_min_node: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallReport {
    pub tool_id: String,
    pub version: String,
    pub install_path: String,
    pub elapsed_secs: u64,
    pub method: InstallMethod,
}

/// Trait every supported CLI tool implements.
///
/// Native async-fn-in-trait (Rust 1.75+) — no async_trait crate needed.
#[allow(async_fn_in_trait, dead_code)]
pub trait Tool: Send + Sync {
    fn id(&self) -> ToolId;

    fn descriptor(&self) -> ToolDescriptor;

    /// Directory the tool's launcher lives in — needs to be on PATH (native route).
    fn launcher_dir(&self) -> Option<PathBuf>;

    /// Mirror sources for this tool (native route only).
    fn mirror_list(&self) -> MirrorList {
        MirrorList::builtin()
    }

    /// npm package name. `None` means the tool doesn't support npm route.
    fn npm_package(&self) -> Option<&'static str> {
        None
    }

    /// Minimum Node major version for npm route. Default 18 (matches both
    /// Claude Code and OpenAI Codex's stated requirements).
    fn npm_min_node(&self) -> u32 {
        18
    }

    async fn detect_installed(&self) -> Option<String>;

    async fn install(
        &self,
        method: InstallMethod,
        progress: ProgressCallback,
        client: reqwest::Client,
        mirrors: MirrorList,
        channel: String,
    ) -> Result<InstallReport>;
}
