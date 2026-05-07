use serde::Serialize;
use std::path::PathBuf;
use tauri::AppHandle;

use crate::error::Result;
use crate::mirrors::MirrorList;

pub type ToolId = &'static str;

#[derive(Debug, Clone, Serialize)]
pub struct ToolDescriptor {
    pub id: String,
    pub name: String,
    pub description: String,
    pub installed_version: Option<String>,
    pub install_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallReport {
    pub tool_id: String,
    pub version: String,
    pub install_path: String,
    pub elapsed_secs: u64,
}

/// Trait every supported CLI tool implements.
///
/// Native async-fn-in-trait (Rust 1.75+) — no async_trait crate needed.
#[allow(async_fn_in_trait, dead_code)]
pub trait Tool: Send + Sync {
    fn id(&self) -> ToolId;

    fn descriptor(&self) -> ToolDescriptor;

    /// Directory the tool's launcher lives in — needs to be on PATH.
    /// Returned path may not exist yet (e.g. before install).
    fn launcher_dir(&self) -> Option<PathBuf>;

    async fn detect_installed(&self) -> Option<String>;

    async fn install(
        &self,
        app: AppHandle,
        client: reqwest::Client,
        mirrors: MirrorList,
        channel: String,
    ) -> Result<InstallReport>;
}
