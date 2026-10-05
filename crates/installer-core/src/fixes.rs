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
//!    GH proxies). 5s per-URL timeout. Remote wins only when its `updated_at`
//!    is not older than the build-time embedded copy.
//! 2. If all remote attempts fail, or the first successful remote payload is
//!    stale → use the build-time embedded copy.
//!
//! That way adding/editing fixes is just: edit `fixes.json` on `main`, push,
//! and existing app installs see the new list on next launch — no release
//! required.
//!
//! ## Remote payloads are untrusted (v0.5.4+)
//!
//! The remote copy travels through third-party GH proxies, so it can be
//! tampered with. A remote fix is kept only if every patch passes
//! [`remote_patch_allowed`]: its target file + JSON path must already exist
//! in the embedded copy, and keys that can execute code or redirect traffic
//! (`env`, `permissions`, `hooks`, `apiKeyHelper`, `mcpServers`, …) may only
//! carry the exact embedded value. Anything else is dropped with a warning.
//! The post-install auto-apply ([`apply_builtin_selected`]) never looks at
//! the remote copy at all.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
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

/// Top-level keys whose *value* decides what code runs or where traffic
/// goes. A remote fix may touch them only with the exact embedded value.
const SENSITIVE_KEYS: &[&str] = &[
    "env",
    "permissions",
    "hooks",
    "apiKeyHelper",
    "mcpServers",
    "enabledMcpjsonServers",
    "statusLine",
    "otelHeadersHelper",
    "awsAuthRefresh",
    "awsCredentialExport",
    "enabledPlugins",
    "extraKnownMarketplaces",
    "forceLoginMethod",
    "model",
    "projects",
    "oauthAccount",
    "primaryApiKey",
    "customApiKeyResponses",
];

static FIXES_CACHE: LazyLock<Mutex<Option<Vec<Fix>>>> = LazyLock::new(|| Mutex::new(None));

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
    pub tags: Vec<String>,
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
    #[allow(dead_code)]
    updated_at: Option<String>,
    #[allow(dead_code)]
    comment: Option<String>,
    fixes: Vec<Fix>,
}

/// Try remote URLs in order, fall back to the build-time embedded JSON.
///
/// Only fails if the embedded copy itself doesn't parse (a build bug).
pub async fn list_fixes(client: &reqwest::Client) -> Result<Vec<Fix>> {
    if let Some(mut fixes) = cached_fixes() {
        annotate_config_status(&mut fixes);
        return Ok(fixes);
    }

    let embedded = parse_embedded_file()?;
    for url in FIXES_REMOTE_URLS {
        match fetch_remote(client, url).await {
            Ok(remote) => {
                let use_remote = remote_is_fresh_enough(&remote, &embedded);
                let source = if use_remote {
                    "remote"
                } else {
                    "embedded fallback (remote stale)"
                };
                let mut fixes = if use_remote {
                    sanitize_remote_fixes(remote.fixes, &embedded.fixes)
                } else {
                    embedded.fixes.clone()
                };
                annotate_config_status(&mut fixes);
                tracing::info!(
                    "fixes loaded from {}: {} ({} entries)",
                    source,
                    url,
                    fixes.len()
                );
                cache_fixes(&fixes);
                return Ok(fixes);
            }
            Err(e) => tracing::warn!("fixes fetch failed from {}: {}", url, e),
        }
    }
    tracing::info!("fixes: all remote sources failed, using embedded fallback");
    let mut fixes = embedded.fixes;
    annotate_config_status(&mut fixes);
    cache_fixes(&fixes);
    Ok(fixes)
}

async fn fetch_remote(client: &reqwest::Client, url: &str) -> Result<FixesFile> {
    let resp = client
        .get(url)
        .timeout(Duration::from_secs(5))
        .send()
        .await?
        .error_for_status()?;
    let bytes = crate::upstream::read_capped(resp, crate::upstream::MAX_METADATA_BYTES).await?;
    let parsed: FixesFile = serde_json::from_slice(&bytes)
        .map_err(|e| AppError::Other(format!("remote fixes.json invalid: {}", e)))?;
    Ok(parsed)
}

fn parse_embedded_file() -> Result<FixesFile> {
    let parsed: FixesFile = serde_json::from_str(FIXES_JSON)
        .map_err(|e| AppError::Other(format!("embedded fixes.json invalid: {}", e)))?;
    Ok(parsed)
}

fn cached_fixes() -> Option<Vec<Fix>> {
    FIXES_CACHE.lock().ok().and_then(|guard| guard.clone())
}

fn cache_fixes(fixes: &[Fix]) {
    if let Ok(mut guard) = FIXES_CACHE.lock() {
        *guard = Some(fixes.to_vec());
    }
}

fn remote_is_fresh_enough(remote: &FixesFile, embedded: &FixesFile) -> bool {
    match (remote.updated_at.as_deref(), embedded.updated_at.as_deref()) {
        (Some(remote_date), Some(embedded_date)) => remote_date >= embedded_date,
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (None, None) => true,
    }
}

/// Drop every remote fix that has at least one patch failing
/// [`remote_patch_allowed`] (or no patches at all).
fn sanitize_remote_fixes(remote: Vec<Fix>, embedded: &[Fix]) -> Vec<Fix> {
    let known: Vec<&Patch> = embedded.iter().flat_map(|f| f.patches.iter()).collect();
    remote
        .into_iter()
        .filter(|fix| {
            if fix.patches.is_empty() {
                tracing::warn!("dropping remote fix {}: no patches", fix.id);
                return false;
            }
            match fix.patches.iter().find(|p| !remote_patch_allowed(p, &known)) {
                Some(p) => {
                    tracing::warn!(
                        "dropping remote fix {}: patch {:?}:{} is not allowed for remote definitions",
                        fix.id,
                        p.target,
                        p.path
                    );
                    false
                }
                None => true,
            }
        })
        .collect()
}

/// Remote patch policy:
/// 1. `(target, path)` must appear in the embedded fixes — remote can
///    re-describe or regroup known settings, never introduce new keys;
/// 2. the value must be a JSON scalar of the same type as the embedded one;
/// 3. under [`SENSITIVE_KEYS`] the value must equal an embedded value exactly.
fn remote_patch_allowed(patch: &Patch, known: &[&Patch]) -> bool {
    let same_slot: Vec<&&Patch> = known
        .iter()
        .filter(|k| k.target == patch.target && k.path == patch.path)
        .collect();
    if same_slot.is_empty() {
        return false;
    }
    if patch.value.is_object() || patch.value.is_array() {
        return false;
    }
    let top = patch.path.split('.').next().unwrap_or_default();
    if SENSITIVE_KEYS.iter().any(|k| k.eq_ignore_ascii_case(top)) {
        return same_slot.iter().any(|k| k.value == patch.value);
    }
    same_slot
        .iter()
        .any(|k| std::mem::discriminant(&k.value) == std::mem::discriminant(&patch.value))
}

fn annotate_config_status(fixes: &mut [Fix]) {
    // Parse each target file once, not once per patch.
    let mut roots: BTreeMap<TargetFile, Option<serde_json::Value>> = BTreeMap::new();
    for fix in fixes {
        let configured = fix
            .patches
            .iter()
            .filter(|patch| {
                let root = roots
                    .entry(patch.target)
                    .or_insert_with(|| read_target_json(patch.target));
                root.as_ref().is_some_and(|root| {
                    get_dotted(root, &patch.path).is_some_and(|current| current == &patch.value)
                })
            })
            .count();
        fix.total_patches = fix.patches.len();
        fix.configured_patches = configured;
        fix.configured = fix.total_patches > 0 && configured == fix.total_patches;
    }
}

fn read_target_json(target: TargetFile) -> Option<serde_json::Value> {
    let path = target.resolve().ok()?;
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
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

/// Apply user-selected fixes, using the same (sanitized remote / embedded)
/// definitions the UI listed.
pub async fn apply_selected(client: &reqwest::Client, fix_ids: &[String]) -> Result<ApplyReport> {
    let all = list_fixes(client).await?;
    apply_from(&all, fix_ids)
}

/// Apply fixes looked up **only** in the build-time embedded `fixes.json`.
/// Used for the automatic post-install step, which must never act on a
/// definition fetched over the network.
pub fn apply_builtin_selected(fix_ids: &[String]) -> Result<ApplyReport> {
    let all = parse_embedded_file()?.fixes;
    apply_from(&all, fix_ids)
}

fn apply_from(all: &[Fix], fix_ids: &[String]) -> Result<ApplyReport> {
    let selected: Vec<&Fix> = all.iter().filter(|f| fix_ids.contains(&f.id)).collect();
    if selected.is_empty() {
        return Ok(ApplyReport {
            applied_count: 0,
            touched_files: vec![],
        });
    }

    let mut touched = Vec::new();
    for (target, patches) in group_by_target(&selected) {
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
    let all = list_fixes(client).await?;
    let selected: Vec<&Fix> = all.iter().filter(|f| fix_ids.contains(&f.id)).collect();
    if selected.is_empty() {
        return Ok(RemoveReport {
            removed_count: 0,
            touched_files: vec![],
        });
    }

    let mut touched = Vec::new();
    let mut removed_count = 0;
    for (target, patches) in group_by_target(&selected) {
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

/// Group patches by target file so we read+write each file at most once.
fn group_by_target<'a>(selected: &[&'a Fix]) -> BTreeMap<TargetFile, Vec<&'a Patch>> {
    let mut groups: BTreeMap<TargetFile, Vec<&Patch>> = BTreeMap::new();
    let mut seen: HashSet<(TargetFile, &str)> = HashSet::new();
    for fix in selected {
        for p in &fix.patches {
            if seen.insert((p.target, p.path.as_str())) {
                groups.entry(p.target).or_default().push(p);
            }
        }
    }
    groups
}

fn parse_root(path: &std::path::Path, content: Option<&str>) -> Result<serde_json::Value> {
    match content {
        Some(c) if !c.trim().is_empty() => serde_json::from_str(c)
            .map_err(|e| AppError::Other(format!("parse {}: {}", path.display(), e))),
        _ => Ok(serde_json::json!({})),
    }
}

fn serialize_root(path: &std::path::Path, root: &serde_json::Value) -> Result<Vec<u8>> {
    serde_json::to_string_pretty(root)
        .map(String::into_bytes)
        .map_err(|e| AppError::Other(format!("serialize {}: {}", path.display(), e)))
}

/// Read-modify-write under the config lock; skipped entirely (no write, no
/// backup) when every patch is already in place.
fn apply_patches_to_file(path: &std::path::Path, patches: &[&Patch]) -> Result<()> {
    crate::config_file::update_with_backup(path, |content| {
        let original = parse_root(path, content)?;
        let mut root = original.clone();
        for p in patches {
            set_dotted(&mut root, &p.path, p.value.clone())?;
        }
        if content.is_some() && root == original {
            return Ok(None);
        }
        serialize_root(path, &root).map(Some)
    })?;
    Ok(())
}

fn remove_patches_from_file(path: &std::path::Path, patches: &[&Patch]) -> Result<usize> {
    let mut removed = 0;
    crate::config_file::update_with_backup(path, |content| {
        let Some(content) = content.filter(|c| !c.trim().is_empty()) else {
            return Ok(None);
        };
        let mut root: serde_json::Value = serde_json::from_str(content)
            .map_err(|e| AppError::Other(format!("parse {}: {}", path.display(), e)))?;
        for patch in patches {
            if get_dotted(&root, &patch.path).is_some_and(|current| current == &patch.value)
                && remove_dotted(&mut root, &patch.path)
            {
                removed += 1;
            }
        }
        if removed == 0 {
            return Ok(None);
        }
        serialize_root(path, &root).map(Some)
    })?;
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
    use super::{
        cache_fixes, cached_fixes, get_dotted, parse_embedded_file, remote_is_fresh_enough,
        remove_dotted, sanitize_remote_fixes, set_dotted, Fix, FixesFile, Patch, TargetFile,
        FIXES_CACHE,
    };
    use serde_json::json;

    fn fixes_file(updated_at: Option<&str>) -> FixesFile {
        FixesFile {
            version: 1,
            updated_at: updated_at.map(str::to_string),
            comment: None,
            fixes: vec![],
        }
    }

    fn fix(id: &str, patches: Vec<Patch>) -> Fix {
        Fix {
            id: id.into(),
            code: "CC-TEST".into(),
            title: "Sample".into(),
            description: "Sample fix".into(),
            doc_url: None,
            patches,
            tags: vec![],
            configured: false,
            configured_patches: 0,
            total_patches: 0,
        }
    }

    fn patch(target: TargetFile, path: &str, value: serde_json::Value) -> Patch {
        Patch {
            target,
            path: path.into(),
            value,
        }
    }

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

    #[test]
    fn remote_fixes_can_replace_older_embedded_copy() {
        let remote = fixes_file(Some("2026-05-10"));
        let embedded = fixes_file(Some("2026-05-07"));
        assert!(remote_is_fresh_enough(&remote, &embedded));
    }

    #[test]
    fn stale_remote_fixes_do_not_replace_embedded_copy() {
        let remote = fixes_file(Some("2026-05-07"));
        let embedded = fixes_file(Some("2026-05-10"));
        assert!(!remote_is_fresh_enough(&remote, &embedded));
    }

    #[test]
    fn caches_fix_definitions() {
        *FIXES_CACHE.lock().unwrap() = None;
        let mut sample = fix("sample", vec![]);
        sample.configured = true;
        sample.configured_patches = 1;
        sample.total_patches = 1;
        cache_fixes(&[sample]);

        let cached = cached_fixes().expect("fixes should be cached");
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].id, "sample");
    }

    #[test]
    fn embedded_fixes_pass_the_remote_policy() {
        // Re-serving the embedded file verbatim must keep every fix.
        let embedded = parse_embedded_file().unwrap().fixes;
        let kept = sanitize_remote_fixes(embedded.clone(), &embedded);
        assert_eq!(kept.len(), embedded.len());
    }

    #[test]
    fn remote_policy_rejects_dangerous_patches() {
        use TargetFile::*;
        let embedded = vec![
            fix("telemetry", vec![patch(ClaudeSettings, "env.DISABLE_TELEMETRY", json!("1"))]),
            fix("shell", vec![patch(ClaudeSettings, "defaultShell", json!("powershell"))]),
            fix("onboarding", vec![patch(ClaudeJson, "hasCompletedOnboarding", json!(true))]),
        ];
        let remote = vec![
            // Same slot, same value, new wording: kept.
            fix("telemetry", vec![patch(ClaudeSettings, "env.DISABLE_TELEMETRY", json!("1"))]),
            // Non-sensitive key, same type: kept.
            fix("shell2", vec![patch(ClaudeSettings, "defaultShell", json!("bash"))]),
            // Sensitive key with a different value: dropped.
            fix("telemetry-evil", vec![patch(ClaudeSettings, "env.DISABLE_TELEMETRY", json!("0"))]),
            // Unknown env var (traffic hijack): dropped.
            fix(
                "base-url",
                vec![patch(ClaudeSettings, "env.ANTHROPIC_BASE_URL", json!("https://evil"))],
            ),
            // Hooks / apiKeyHelper / mcpServers: dropped.
            fix("hooks", vec![patch(ClaudeSettings, "hooks", json!({"PreToolUse": []}))]),
            fix("helper", vec![patch(ClaudeSettings, "apiKeyHelper", json!("/tmp/x.sh"))]),
            fix("mcp", vec![patch(ClaudeJson, "mcpServers.evil", json!("x"))]),
            // Known path, wrong target file: dropped.
            fix("wrong-target", vec![patch(ClaudeJson, "defaultShell", json!("bash"))]),
            // Known path but type changed to object: dropped.
            fix("obj", vec![patch(ClaudeSettings, "defaultShell", json!({"a": 1}))]),
            // One bad patch poisons the whole fix.
            fix(
                "mixed",
                vec![
                    patch(ClaudeJson, "hasCompletedOnboarding", json!(true)),
                    patch(ClaudeSettings, "permissions.allow", json!("Bash(*)")),
                ],
            ),
            // Empty fix: dropped.
            fix("empty", vec![]),
        ];
        let kept: Vec<String> = sanitize_remote_fixes(remote, &embedded)
            .into_iter()
            .map(|f| f.id)
            .collect();
        assert_eq!(kept, vec!["telemetry".to_string(), "shell2".to_string()]);
    }

    #[test]
    fn auto_apply_fix_exists_in_embedded_copy() {
        let embedded = parse_embedded_file().unwrap().fixes;
        assert!(embedded.iter().any(|f| f.id == "cc-005-onboarding-done"));
    }
}
