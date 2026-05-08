//! Built-in "fix recipes" sourced from the OCC docs knowledge base.
//!
//! Each fix is a list of *patches* (path + value to insert into a JSON config
//! file). UI shows them as a checklist; user picks which to apply, app merges
//! them into `~/.claude/settings.json` or `~/.claude.json`, preserving every
//! other field already in those files.
//!
//! ## Loading order (v0.0.10+)
//!
//! 1. Try to fetch latest `fixes.json` from a list of remote URLs (raw GH +
//!    GH proxies). 5s per-URL timeout. First success wins.
//! 2. If all remote attempts fail → fall back to the build-time embedded copy.
//!
//! That way adding/editing fixes is just: edit `fixes.json` on `main`, push,
//! and existing app installs see the new list on next launch — no release
//! required.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

use crate::error::{AppError, Result};

const FIXES_JSON: &str = include_str!("../fixes.json");

/// Remote candidates for fetching the latest `fixes.json`. Tried in order.
/// Direct first; falls through to GH proxies on failure.
const FIXES_REMOTE_URLS: &[&str] = &[
    "https://raw.githubusercontent.com/zuoliangyu/ai-cli-installer/main/crates/installer-core/fixes.json",
    "https://gh-proxy.com/https://raw.githubusercontent.com/zuoliangyu/ai-cli-installer/main/crates/installer-core/fixes.json",
    "https://fastgit.cc/https://raw.githubusercontent.com/zuoliangyu/ai-cli-installer/main/crates/installer-core/fixes.json",
    "https://github.chenc.dev/https://raw.githubusercontent.com/zuoliangyu/ai-cli-installer/main/crates/installer-core/fixes.json",
];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TargetFile {
    /// `~/.claude/settings.json` — main Claude Code settings.
    ClaudeSettings,
    /// `~/.claude.json` — Claude Code's per-user state file.
    ClaudeJson,
}

impl TargetFile {
    fn resolve(&self) -> Result<PathBuf> {
        let home = dirs::home_dir().ok_or_else(|| AppError::Other("no home dir".into()))?;
        Ok(match self {
            TargetFile::ClaudeSettings => home.join(".claude").join("settings.json"),
            TargetFile::ClaudeJson => home.join(".claude.json"),
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Patch {
    pub target: TargetFile,
    /// Dot-separated path inside the target JSON. e.g. `env.FOO` or `skipWebFetchPreflight`.
    pub path: String,
    /// Any JSON value (string, bool, number, object).
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Fix {
    pub id: String,
    pub code: String,
    pub title: String,
    pub description: String,
    pub doc_url: Option<String>,
    pub patches: Vec<Patch>,
    #[serde(default)]
    pub configured: bool,
    #[serde(default)]
    pub configured_patches: usize,
    #[serde(default)]
    pub total_patches: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct FixesFile {
    #[allow(dead_code)]
    version: u32,
    fixes: Vec<Fix>,
}

/// Try remote URLs in order, fall back to the build-time embedded JSON.
pub async fn list_fixes(client: &reqwest::Client) -> Result<Vec<Fix>> {
    for url in FIXES_REMOTE_URLS {
        match fetch_remote(client, url).await {
            Ok(mut fixes) => {
                annotate_config_status(&mut fixes);
                tracing::info!(
                    "fixes loaded from remote: {} ({} entries)",
                    url,
                    fixes.len()
                );
                return Ok(fixes);
            }
            Err(e) => tracing::warn!("fixes fetch failed from {}: {}", url, e),
        }
    }
    tracing::info!("fixes: all remote sources failed, using embedded fallback");
    let mut fixes = parse_embedded()?;
    annotate_config_status(&mut fixes);
    Ok(fixes)
}

async fn fetch_remote(client: &reqwest::Client, url: &str) -> Result<Vec<Fix>> {
    let resp = client
        .get(url)
        .timeout(Duration::from_secs(5))
        .send()
        .await?
        .error_for_status()?;
    let bytes = resp.bytes().await?;
    let parsed: FixesFile = serde_json::from_slice(&bytes)
        .map_err(|e| AppError::Other(format!("remote fixes.json invalid: {}", e)))?;
    Ok(parsed.fixes)
}

fn parse_embedded() -> Result<Vec<Fix>> {
    let parsed: FixesFile = serde_json::from_str(FIXES_JSON)
        .map_err(|e| AppError::Other(format!("embedded fixes.json invalid: {}", e)))?;
    Ok(parsed.fixes)
}

/// Sync helper that only reads the embedded copy. Used by `apply_selected`
/// (which needs to look up fix definitions by id without making a network
/// call mid-apply). Future enhancement: cache last-good remote payload on
/// disk so apply_selected sees the fresh definitions too.
fn list_fixes_embedded() -> Result<Vec<Fix>> {
    parse_embedded()
}

fn annotate_config_status(fixes: &mut [Fix]) {
    for fix in fixes {
        let configured = fix
            .patches
            .iter()
            .filter(|patch| patch_is_configured(patch))
            .count();
        fix.total_patches = fix.patches.len();
        fix.configured_patches = configured;
        fix.configured = fix.total_patches > 0 && configured == fix.total_patches;
    }
}

fn patch_is_configured(patch: &Patch) -> bool {
    let Ok(path) = patch.target.resolve() else {
        return false;
    };
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(root) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };
    get_dotted(&root, &patch.path).is_some_and(|current| current == &patch.value)
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplyReport {
    pub applied_count: usize,
    pub touched_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RemoveReport {
    pub removed_count: usize,
    pub touched_files: Vec<String>,
}

pub async fn apply_selected(client: &reqwest::Client, fix_ids: &[String]) -> Result<ApplyReport> {
    // Use the same loader as list_fixes so apply respects remote-edited
    // definitions. Embedded fallback covers offline.
    let all = match list_fixes(client).await {
        Ok(v) => v,
        Err(_) => list_fixes_embedded()?,
    };
    let selected: Vec<&Fix> = all.iter().filter(|f| fix_ids.contains(&f.id)).collect();
    if selected.is_empty() {
        return Ok(ApplyReport {
            applied_count: 0,
            touched_files: vec![],
        });
    }

    // Group patches by target file so we read+write each file at most once.
    let mut groups: std::collections::BTreeMap<TargetFile, Vec<&Patch>> =
        std::collections::BTreeMap::new();
    for fix in &selected {
        for p in &fix.patches {
            groups.entry(p.target).or_default().push(p);
        }
    }

    let mut touched = Vec::new();
    for (target, patches) in groups {
        let path = target.resolve()?;
        apply_patches_to_file(&path, &patches)?;
        touched.push(path.to_string_lossy().to_string());
    }

    Ok(ApplyReport {
        applied_count: selected.len(),
        touched_files: touched,
    })
}

pub async fn remove_selected(client: &reqwest::Client, fix_ids: &[String]) -> Result<RemoveReport> {
    let all = match list_fixes(client).await {
        Ok(v) => v,
        Err(_) => list_fixes_embedded()?,
    };
    let selected: Vec<&Fix> = all.iter().filter(|f| fix_ids.contains(&f.id)).collect();
    if selected.is_empty() {
        return Ok(RemoveReport {
            removed_count: 0,
            touched_files: vec![],
        });
    }

    let mut groups: std::collections::BTreeMap<TargetFile, Vec<&Patch>> =
        std::collections::BTreeMap::new();
    for fix in &selected {
        for p in &fix.patches {
            groups.entry(p.target).or_default().push(p);
        }
    }

    let mut touched = Vec::new();
    let mut removed_count = 0;
    for (target, patches) in groups {
        let path = target.resolve()?;
        let removed = remove_patches_from_file(&path, &patches)?;
        if removed > 0 {
            removed_count += removed;
            touched.push(path.to_string_lossy().to_string());
        }
    }

    Ok(RemoveReport {
        removed_count,
        touched_files: touched,
    })
}

fn apply_patches_to_file(path: &std::path::Path, patches: &[&Patch]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut root: serde_json::Value = if path.exists() {
        let content = std::fs::read_to_string(path)?;
        if content.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(&content)
                .map_err(|e| AppError::Other(format!("parse {}: {}", path.display(), e)))?
        }
    } else {
        serde_json::json!({})
    };

    for p in patches {
        set_dotted(&mut root, &p.path, p.value.clone())?;
    }

    let pretty = serde_json::to_string_pretty(&root)
        .map_err(|e| AppError::Other(format!("serialize {}: {}", path.display(), e)))?;
    std::fs::write(path, pretty)?;
    Ok(())
}

fn remove_patches_from_file(path: &std::path::Path, patches: &[&Patch]) -> Result<usize> {
    if !path.exists() {
        return Ok(0);
    }

    let content = std::fs::read_to_string(path)?;
    if content.trim().is_empty() {
        return Ok(0);
    }

    let mut root: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| AppError::Other(format!("parse {}: {}", path.display(), e)))?;

    let mut removed = 0;
    for patch in patches {
        if get_dotted(&root, &patch.path).is_some_and(|current| current == &patch.value)
            && remove_dotted(&mut root, &patch.path)
        {
            removed += 1;
        }
    }

    if removed > 0 {
        let pretty = serde_json::to_string_pretty(&root)
            .map_err(|e| AppError::Other(format!("serialize {}: {}", path.display(), e)))?;
        std::fs::write(path, pretty)?;
    }

    Ok(removed)
}

/// Set a dotted-path value inside a JSON object, creating intermediate objects
/// as needed. Replaces existing leaf values; doesn't merge nested objects.
fn set_dotted(root: &mut serde_json::Value, path: &str, value: serde_json::Value) -> Result<()> {
    if path.is_empty() {
        return Err(AppError::Other("empty patch path".into()));
    }
    let segments: Vec<&str> = path.split('.').collect();
    if !root.is_object() {
        return Err(AppError::Other("target file root is not an object".into()));
    }

    let mut current = root;
    for (i, seg) in segments.iter().enumerate() {
        let is_last = i == segments.len() - 1;
        // Replace non-objects encountered mid-path with objects.
        if !current.is_object() {
            *current = serde_json::Value::Object(serde_json::Map::new());
        }
        let map = current.as_object_mut().unwrap();

        if is_last {
            map.insert((*seg).to_string(), value.clone());
            return Ok(());
        }

        if !map.contains_key(*seg) || !map[*seg].is_object() {
            map.insert(
                (*seg).to_string(),
                serde_json::Value::Object(serde_json::Map::new()),
            );
        }
        current = map.get_mut(*seg).unwrap();
    }
    Ok(())
}

fn get_dotted<'a>(root: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    if path.is_empty() {
        return None;
    }
    let mut current = root;
    for seg in path.split('.') {
        current = current.as_object()?.get(seg)?;
    }
    Some(current)
}

fn remove_dotted(root: &mut serde_json::Value, path: &str) -> bool {
    let Some((parent_path, leaf)) = path.rsplit_once('.') else {
        return root
            .as_object_mut()
            .and_then(|map| map.remove(path))
            .is_some();
    };
    let Some(parent) = get_dotted_mut(root, parent_path) else {
        return false;
    };
    parent
        .as_object_mut()
        .and_then(|map| map.remove(leaf))
        .is_some()
}

fn get_dotted_mut<'a>(
    root: &'a mut serde_json::Value,
    path: &str,
) -> Option<&'a mut serde_json::Value> {
    if path.is_empty() {
        return None;
    }
    let mut current = root;
    for seg in path.split('.') {
        current = current.as_object_mut()?.get_mut(seg)?;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::{get_dotted, remove_dotted, set_dotted};
    use serde_json::json;

    #[test]
    fn reads_dotted_json_value() {
        let root = json!({ "env": { "DISABLE_TELEMETRY": "1" } });
        assert_eq!(
            get_dotted(&root, "env.DISABLE_TELEMETRY"),
            Some(&json!("1"))
        );
    }

    #[test]
    fn missing_dotted_json_value_returns_none() {
        let root = json!({ "env": {} });
        assert_eq!(get_dotted(&root, "env.DISABLE_TELEMETRY"), None);
    }

    #[test]
    fn set_then_read_dotted_json_value() {
        let mut root = json!({});
        set_dotted(&mut root, "env.DISABLE_ERROR_REPORTING", json!("1")).unwrap();
        assert_eq!(
            get_dotted(&root, "env.DISABLE_ERROR_REPORTING"),
            Some(&json!("1"))
        );
    }

    #[test]
    fn remove_dotted_json_value() {
        let mut root = json!({ "env": { "DISABLE_TELEMETRY": "1" } });
        assert!(remove_dotted(&mut root, "env.DISABLE_TELEMETRY"));
        assert_eq!(get_dotted(&root, "env.DISABLE_TELEMETRY"), None);
    }
}
