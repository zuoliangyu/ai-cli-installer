//! Claude provider presets ("中转站快捷配置").
//!
//! Two sources merged:
//! 1. Built-in (compiled in): Micu + E-FlowCode
//! 2. cc-switch's SQLite db at `~/.cc-switch/cc-switch.db` (if present)
//!
//! Applying a preset writes only the `env.ANTHROPIC_BASE_URL` and
//! `env.ANTHROPIC_AUTH_TOKEN` keys into `~/.claude/settings.json`. Other
//! settings (effortLevel, plugins, etc.) are intentionally NOT touched —
//! v0.0.6 keeps the surface small.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::error::{AppError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresetSource {
    Builtin,
    CcSwitch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudePreset {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub website_url: Option<String>,
    pub api_key_url: Option<String>,
    pub source: PresetSource,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ClaudeSettingsEnv {
    pub anthropic_base_url: Option<String>,
    pub anthropic_auth_token: Option<String>,
}

// ---------- Built-in presets ----------

pub fn builtin_presets() -> Vec<ClaudePreset> {
    vec![
        ClaudePreset {
            id: "builtin:micu".into(),
            name: "Micu (米醋)".into(),
            base_url: "https://www.micuapi.ai".into(),
            website_url: Some("https://www.micuapi.ai".into()),
            api_key_url: Some("https://www.micuapi.ai".into()),
            source: PresetSource::Builtin,
        },
        ClaudePreset {
            id: "builtin:eflowcode".into(),
            name: "E-FlowCode".into(),
            base_url: "https://e-flowcode.cc".into(),
            website_url: Some("https://e-flowcode.cc".into()),
            api_key_url: Some("https://e-flowcode.cc".into()),
            source: PresetSource::Builtin,
        },
    ]
}

// ---------- cc-switch DB sync ----------

fn cc_switch_db_path() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".cc-switch").join("cc-switch.db"))
}

/// Read Claude providers from cc-switch's SQLite DB. Returns empty Vec if the
/// db doesn't exist or any read step fails — never propagates DB errors to the
/// caller (sync is best-effort).
pub fn read_cc_switch_presets() -> Vec<ClaudePreset> {
    let path = match cc_switch_db_path() {
        Some(p) if p.exists() => p,
        _ => return vec![],
    };

    let conn = match rusqlite::Connection::open_with_flags(
        &path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("cc-switch db open failed: {}", e);
            return vec![];
        }
    };

    let mut stmt = match conn.prepare(
        "SELECT id, name, settings_config, website_url, api_key_url
         FROM providers WHERE app_type = 'claude'",
    ) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("cc-switch db prepare failed: {}", e);
            return vec![];
        }
    };

    let rows = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let name: String = row.get(1)?;
        let settings_json: String = row.get(2)?;
        let website_url: Option<String> = row.get(3).ok();
        let api_key_url: Option<String> = row.get(4).ok();
        Ok((id, name, settings_json, website_url, api_key_url))
    });

    let mut out = Vec::new();
    if let Ok(iter) = rows {
        for row in iter.flatten() {
            let (id, name, settings_json, website_url, api_key_url) = row;
            let base_url = match extract_base_url(&settings_json) {
                Some(u) => u,
                None => continue,
            };
            out.push(ClaudePreset {
                id: format!("cc-switch:{}", id),
                name,
                base_url,
                website_url,
                api_key_url,
                source: PresetSource::CcSwitch,
            });
        }
    }
    out
}

fn extract_base_url(settings_json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(settings_json).ok()?;
    v.get("env")?
        .get("ANTHROPIC_BASE_URL")?
        .as_str()
        .map(String::from)
}

/// Built-in + cc-switch merged. Built-in always first.
/// Dedupe by base_url (case-insensitive) — built-in wins.
pub fn list_all_presets() -> Vec<ClaudePreset> {
    let mut seen_urls: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::new();

    for p in builtin_presets() {
        seen_urls.insert(p.base_url.to_ascii_lowercase());
        out.push(p);
    }
    for p in read_cc_switch_presets() {
        let key = p.base_url.to_ascii_lowercase();
        if seen_urls.insert(key) {
            out.push(p);
        }
    }
    out
}

// ---------- Claude settings.json read/write ----------

fn claude_settings_path() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| AppError::Other("no home dir".into()))?;
    Ok(home.join(".claude").join("settings.json"))
}

pub fn read_current_env() -> Result<ClaudeSettingsEnv> {
    let path = claude_settings_path()?;
    if !path.exists() {
        return Ok(ClaudeSettingsEnv::default());
    }
    let content = std::fs::read_to_string(&path)?;
    if content.trim().is_empty() {
        return Ok(ClaudeSettingsEnv::default());
    }
    let v: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| AppError::Other(format!("parse settings.json: {}", e)))?;

    let env = v.get("env");
    Ok(ClaudeSettingsEnv {
        anthropic_base_url: env
            .and_then(|e| e.get("ANTHROPIC_BASE_URL"))
            .and_then(|v| v.as_str())
            .map(String::from),
        anthropic_auth_token: env
            .and_then(|e| e.get("ANTHROPIC_AUTH_TOKEN"))
            .and_then(|v| v.as_str())
            .map(String::from),
    })
}

/// Merge BASE_URL + AUTH_TOKEN into existing settings.json. Other keys
/// (effortLevel, plugins, env.*) are preserved.
pub fn apply_env(base_url: &str, auth_token: &str) -> Result<()> {
    let path = claude_settings_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut value: serde_json::Value = if path.exists() {
        let content = std::fs::read_to_string(&path)?;
        if content.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(&content)
                .map_err(|e| AppError::Other(format!("parse settings.json: {}", e)))?
        }
    } else {
        serde_json::json!({})
    };

    let root = value
        .as_object_mut()
        .ok_or_else(|| AppError::Other("settings.json root must be object".into()))?;

    let env_obj = root
        .entry("env".to_string())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let env_map = env_obj
        .as_object_mut()
        .ok_or_else(|| AppError::Other("settings.json `env` must be object".into()))?;

    env_map.insert(
        "ANTHROPIC_BASE_URL".to_string(),
        serde_json::Value::String(base_url.to_string()),
    );
    env_map.insert(
        "ANTHROPIC_AUTH_TOKEN".to_string(),
        serde_json::Value::String(auth_token.to_string()),
    );

    let pretty = serde_json::to_string_pretty(&value)
        .map_err(|e| AppError::Other(format!("serialize settings.json: {}", e)))?;
    std::fs::write(&path, pretty)?;
    Ok(())
}
