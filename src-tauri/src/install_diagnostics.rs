use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tokio::process::Command;

use crate::npm_installer;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstallationSource {
    Native,
    NpmGlobal,
    Path,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInstallation {
    pub source: InstallationSource,
    pub version: Option<String>,
    pub path: Option<String>,
    pub current_path: bool,
    pub managed: bool,
}

pub async fn diagnose(
    command_name: &str,
    native_path: Option<PathBuf>,
    managed_dir: Option<PathBuf>,
    npm_package: Option<&str>,
) -> Vec<ToolInstallation> {
    let mut installs = Vec::new();

    if let Some(path) = native_path {
        if path.exists() {
            installs.push(ToolInstallation {
                source: InstallationSource::Native,
                version: run_version(&path, command_name).await,
                path: Some(path.to_string_lossy().to_string()),
                current_path: false,
                managed: true,
            });
        }
    }

    if let Some(package) = npm_package {
        if let Some(mut npm_install) = detect_npm_global(command_name, package).await {
            npm_install.managed = managed_dir
                .as_ref()
                .and_then(|dir| {
                    npm_install
                        .path
                        .as_ref()
                        .map(|path| path_starts_with(path, dir))
                })
                .unwrap_or(false);
            installs.push(npm_install);
        }
    }

    let path_version = run_version(Path::new(command_name), command_name).await;
    let resolved = if path_version.is_some() {
        resolve_command_path(command_name).await
    } else {
        None
    };
    if let Some(path) = resolved.as_deref() {
        if let Some(existing) = installs
            .iter_mut()
            .find(|install| install.path.as_deref().is_some_and(|p| same_path(p, path)))
        {
            existing.current_path = true;
            if existing.version.is_none() {
                existing.version = path_version;
            }
            return installs;
        }
    }

    if let Some(version) = path_version {
        installs.push(ToolInstallation {
            source: InstallationSource::Path,
            version: Some(version),
            managed: managed_dir
                .as_ref()
                .and_then(|dir| resolved.as_ref().map(|path| path_starts_with(path, dir)))
                .unwrap_or(false),
            path: resolved,
            current_path: true,
        });
    }

    installs
}

async fn detect_npm_global(command_name: &str, package: &str) -> Option<ToolInstallation> {
    let output = Command::new("npm")
        .args(["list", "-g", package, "--json", "--depth=0"])
        .output()
        .await
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Value = serde_json::from_str(&stdout).ok()?;
    let version = parsed
        .pointer(&format!(
            "/dependencies/{}/version",
            escape_json_pointer(package)
        ))
        .and_then(Value::as_str)
        .map(String::from)?;

    let bin_path = npm_global_bin_path(command_name).await;
    Some(ToolInstallation {
        source: InstallationSource::NpmGlobal,
        version: Some(version),
        path: bin_path.map(|p| p.to_string_lossy().to_string()),
        current_path: false,
        managed: false,
    })
}

async fn npm_global_bin_path(command_name: &str) -> Option<PathBuf> {
    let bin = npm_installer::npm_global_bin().await.ok()?;
    let dir = PathBuf::from(bin);
    for candidate in executable_candidates(command_name) {
        let path = dir.join(candidate);
        if path.exists() {
            return Some(path);
        }
    }
    Some(dir.join(command_name))
}

fn executable_candidates(command_name: &str) -> Vec<String> {
    if cfg!(target_os = "windows") {
        vec![
            format!("{}.cmd", command_name),
            format!("{}.exe", command_name),
            format!("{}.ps1", command_name),
            command_name.to_string(),
        ]
    } else {
        vec![command_name.to_string()]
    }
}

async fn resolve_command_path(command_name: &str) -> Option<String> {
    let output = if cfg!(target_os = "windows") {
        Command::new("where")
            .arg(command_name)
            .output()
            .await
            .ok()?
    } else {
        Command::new("sh")
            .args(["-c", &format!("command -v {}", command_name)])
            .output()
            .await
            .ok()?
    };
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(String::from)
}

async fn run_version(path: &Path, command_name: &str) -> Option<String> {
    let output = Command::new(path).arg("--version").output().await.ok()?;
    if !output.status.success() {
        return None;
    }
    parse_version(command_name, &String::from_utf8_lossy(&output.stdout))
}

fn parse_version(command_name: &str, output: &str) -> Option<String> {
    if command_name == "codex" {
        return output
            .split_whitespace()
            .find(|word| word.chars().next().is_some_and(|c| c.is_ascii_digit()))
            .map(String::from);
    }
    output.split_whitespace().next().map(String::from)
}

fn escape_json_pointer(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn path_starts_with(path: &str, dir: &Path) -> bool {
    Path::new(path).starts_with(dir)
}

fn same_path(a: &str, b: &str) -> bool {
    if cfg!(target_os = "windows") {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::parse_version;

    #[test]
    fn parses_claude_version() {
        assert_eq!(
            parse_version("claude", "2.1.132 (Claude Code)\n"),
            Some("2.1.132".to_string())
        );
    }

    #[test]
    fn parses_codex_named_version() {
        assert_eq!(
            parse_version("codex", "codex-cli 0.128.0\n"),
            Some("0.128.0".to_string())
        );
    }

    #[test]
    fn parses_codex_plain_version() {
        assert_eq!(
            parse_version("codex", "0.128.0\n"),
            Some("0.128.0".to_string())
        );
    }
}
