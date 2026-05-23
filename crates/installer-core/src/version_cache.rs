//! Best-effort on-disk fallback for upstream version numbers.
//!
//! Every launch we call `mirrors::fetch_version` for `latest` and `stable`
//! channels of each tool. When the user is on a flaky network or all mirrors
//! happen to be down we used to silently return `None`, and the UI would
//! degrade the install button to a literal "更新到 latest" with no version
//! number — visually indistinguishable from a successful fetch. This module
//! lets us fall back to the *last* successful value so the UI can still show
//! "latest v1.2.3 · 缓存" instead.
//!
//! ## Storage
//!
//! `~/.ai-cli-installer/version-cache.json`. Schema:
//!
//! ```json
//! {
//!   "claude": { "latest": { "version": "1.0.30", "fetched_at": 1748044800 },
//!               "stable": { "version": "1.0.28", "fetched_at": 1748044800 } },
//!   "codex":  { ... }
//! }
//! ```
//!
//! `fetched_at` is unix seconds. We do not currently surface it to callers but
//! it's useful when debugging stale entries by hand.
//!
//! ## Safety
//!
//! All IO is best-effort: any error (missing home dir, IO failure, malformed
//! JSON) is downgraded to "no cache" and logged via `tracing::warn`. This must
//! never block the main version-fetch flow.
//!
//! ## Concurrency
//!
//! `list_tools` fires four `fetch_channel_version` calls inside one
//! `tokio::join!` (2 tools × 2 channels), so up to four writes race for the
//! same file. A module-level `FILE_LOCK` (blocking `std::sync::Mutex`)
//! serializes the read-modify-write critical section. Atomic rename
//! (`.tmp` → final) guards against torn writes in case something else
//! interleaves out-of-band.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// One channel's cached entry — the version string and when we last saw it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedVersion {
    version: String,
    fetched_at: u64,
}

/// Top-level shape: tool_id → channel → entry.
type ToolMap = HashMap<String, HashMap<String, CachedVersion>>;

/// Serializes the entire read-modify-write cycle. `std::sync::Mutex` (not
/// `tokio::sync::Mutex`) because the critical section is short blocking IO
/// and never `.await`s.
static FILE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn cache_file_path() -> Option<PathBuf> {
    Some(
        dirs::home_dir()?
            .join(".ai-cli-installer")
            .join("version-cache.json"),
    )
}

fn load_from_disk() -> ToolMap {
    let Some(path) = cache_file_path() else {
        return HashMap::new();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        // Missing file is the normal first-run path; don't warn.
        return HashMap::new();
    };
    match serde_json::from_slice::<ToolMap>(&bytes) {
        Ok(map) => map,
        Err(e) => {
            tracing::warn!("version cache parse failed ({}): treating as empty", e);
            HashMap::new()
        }
    }
}

fn write_to_disk(map: &ToolMap) {
    let Some(path) = cache_file_path() else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if let Err(e) = std::fs::create_dir_all(parent) {
        tracing::warn!("version cache: mkdir {} failed: {}", parent.display(), e);
        return;
    }
    let bytes = match serde_json::to_vec_pretty(map) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("version cache: serialize failed: {}", e);
            return;
        }
    };
    let tmp = path.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&tmp, &bytes) {
        tracing::warn!("version cache: write tmp {} failed: {}", tmp.display(), e);
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, &path) {
        tracing::warn!("version cache: rename to {} failed: {}", path.display(), e);
        // Try to clean up the orphan tmp file; ignore failures.
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Look up the most recently recorded version for `(tool_id, channel)`.
/// Returns `None` if we've never recorded one or the cache file is unreadable.
pub fn get(tool_id: &str, channel: &str) -> Option<String> {
    let _guard = FILE_LOCK.lock().ok()?;
    let map = load_from_disk();
    map.get(tool_id)
        .and_then(|channels| channels.get(channel))
        .map(|entry| entry.version.clone())
}

/// Persist a fresh `(tool_id, channel) → version` pair. Errors are swallowed
/// and logged — callers should not branch on success.
pub fn record(tool_id: &str, channel: &str, version: &str) {
    if version.is_empty() {
        return;
    }
    let Ok(_guard) = FILE_LOCK.lock() else {
        return;
    };
    let mut map = load_from_disk();
    let fetched_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    map.entry(tool_id.to_string())
        .or_default()
        .insert(
            channel.to_string(),
            CachedVersion {
                version: version.to_string(),
                fetched_at,
            },
        );
    write_to_disk(&map);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_serialization() {
        let mut map: ToolMap = HashMap::new();
        map.entry("claude".to_string())
            .or_default()
            .insert(
                "latest".to_string(),
                CachedVersion {
                    version: "1.0.30".to_string(),
                    fetched_at: 1_748_044_800,
                },
            );
        map.entry("claude".to_string()).or_default().insert(
            "stable".to_string(),
            CachedVersion {
                version: "1.0.28".to_string(),
                fetched_at: 1_748_044_800,
            },
        );
        let bytes = serde_json::to_vec(&map).unwrap();
        let parsed: ToolMap = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.len(), 1);
        let claude = parsed.get("claude").unwrap();
        assert_eq!(claude.get("latest").unwrap().version, "1.0.30");
        assert_eq!(claude.get("stable").unwrap().version, "1.0.28");
        assert_eq!(claude.get("latest").unwrap().fetched_at, 1_748_044_800);
    }

    #[test]
    fn parses_legacy_or_empty_shapes() {
        // Empty object → empty map, not an error.
        let parsed: ToolMap = serde_json::from_str("{}").unwrap();
        assert!(parsed.is_empty());
        // Unknown extra fields inside a channel entry should fail soft —
        // serde will error, and load_from_disk treats that as "empty cache".
        let bytes = br#"{"claude":{"latest":{"version":"1","fetched_at":1,"extra":42}}}"#;
        let parsed: Result<ToolMap, _> = serde_json::from_slice(bytes);
        // We didn't set `deny_unknown_fields`, so extra keys are tolerated.
        assert!(parsed.is_ok());
    }

    #[test]
    fn missing_version_field_rejected() {
        // A malformed entry without `version` must fail deserialization.
        let bytes = br#"{"claude":{"latest":{"fetched_at":1}}}"#;
        let parsed: Result<ToolMap, _> = serde_json::from_slice(bytes);
        assert!(parsed.is_err());
    }
}
