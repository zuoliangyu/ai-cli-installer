use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::npm_installer;
use crate::proc::{resolve_command_path, run_executable, shell_command};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstallationSource {
    Native,
    NpmGlobal,
    Pnpm,
    Yarn,
    Bun,
    Nvm,
    Path,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInstallation {
    pub source: InstallationSource,
    pub version: Option<String>,
    pub path: Option<String>,
    /// `where <cmd>` 当前解析到的就是这一项
    pub current_path: bool,
    /// 该项目录在 PATH 中（可能被前面的同名项 shadow 掉）
    pub on_path: bool,
    pub managed: bool,
}

pub async fn diagnose(
    command_name: &str,
    native_path: Option<PathBuf>,
    managed_dir: Option<PathBuf>,
    npm_package: Option<&str>,
) -> Vec<ToolInstallation> {
    let mut installs = Vec::new();

    // 1) Native（应用自管路径）
    if let Some(path) = native_path {
        if path.exists() {
            installs.push(ToolInstallation {
                source: InstallationSource::Native,
                version: run_version(&path, command_name).await,
                path: Some(path.to_string_lossy().to_string()),
                current_path: false,
                on_path: false,
                managed: true,
            });
        }
    }

    // 2) npm 全局（含 nvm 当前激活版本）
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
            push_unique(&mut installs, npm_install);
        }
    }

    // 3) pnpm / yarn / bun 全局
    for (source, bin_dir) in pnpm_yarn_bun_bins().await {
        if let Some(install) = detect_via_bin_dir(&bin_dir, command_name, source).await {
            push_unique(&mut installs, install);
        }
    }

    // 4) nvm（含未激活的版本）
    for install in detect_nvm(command_name).await {
        push_unique(&mut installs, install);
    }

    // 5) PATH 解析
    let resolved = resolve_command_path(command_name).await;
    if let Some(path) = resolved.as_deref() {
        let path_str = path.to_string_lossy().to_string();
        let path_version = run_executable(path, &["--version"])
            .await
            .and_then(|s| parse_version(command_name, &s));
        if let Some(existing) = installs.iter_mut().find(|install| {
            install
                .path
                .as_deref()
                .is_some_and(|p| same_path(p, &path_str))
        }) {
            existing.current_path = true;
            if existing.version.is_none() {
                existing.version = path_version.clone();
            }
        } else if let Some(version) = path_version.clone() {
            installs.push(ToolInstallation {
                source: InstallationSource::Path,
                version: Some(version),
                managed: managed_dir
                    .as_ref()
                    .map(|dir| path_starts_with(&path_str, dir))
                    .unwrap_or(false),
                path: Some(path_str),
                current_path: true,
                on_path: true,
            });
        }
    }

    // 6) 标记 on_path：每个 install 的目录是否在 PATH 中
    let path_dirs = current_path_dirs();
    for install in installs.iter_mut() {
        if install.current_path {
            install.on_path = true;
            continue;
        }
        let Some(p) = install.path.as_deref() else {
            continue;
        };
        let parent = Path::new(p).parent();
        install.on_path = parent
            .map(|d| path_dirs.iter().any(|pd| same_path_buf(pd, d)))
            .unwrap_or(false);
    }

    // 7) where/command -v 没解析到时的兜底：如果只有一个 install 在 PATH 上，
    //    那它就是实际会被命中的；标 current_path=true，避免错误地标"被遮蔽"。
    if !installs.iter().any(|i| i.current_path) {
        let on_path_count = installs.iter().filter(|i| i.on_path).count();
        if on_path_count == 1 {
            if let Some(only) = installs.iter_mut().find(|i| i.on_path) {
                only.current_path = true;
            }
        }
    }

    installs
}

async fn detect_npm_global(command_name: &str, package: &str) -> Option<ToolInstallation> {
    // 先用 `npm list -g`（拿权威版本号）
    let mut version = None;
    if let Ok(output) = shell_command("npm")
        .args(["list", "-g", package, "--json", "--depth=0"])
        .output()
        .await
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Ok(parsed) = serde_json::from_str::<Value>(&stdout) {
            version = parsed
                .pointer(&format!(
                    "/dependencies/{}/version",
                    escape_json_pointer(package)
                ))
                .and_then(Value::as_str)
                .map(String::from);
        }
    }

    // 找 bin 路径：优先 `npm prefix -g`，失败则扫常见 npm 全局目录
    let bin_path = npm_global_bin_path(command_name).await;

    // 都找不到才放弃
    if version.is_none() && bin_path.is_none() {
        return None;
    }

    // 没有版本号但找到二进制 → 直接跑 --version
    if version.is_none() {
        if let Some(p) = &bin_path {
            version = run_version(p, command_name).await;
        }
    }

    // 仍然啥也没有就别报这一项
    if version.is_none() && bin_path.is_none() {
        return None;
    }

    Some(ToolInstallation {
        source: InstallationSource::NpmGlobal,
        version,
        path: bin_path.map(|p| p.to_string_lossy().to_string()),
        current_path: false,
        on_path: false,
        managed: false,
    })
}

async fn npm_global_bin_path(command_name: &str) -> Option<PathBuf> {
    if let Ok(bin) = npm_installer::npm_global_bin().await {
        let dir = PathBuf::from(bin);
        for cand in executable_candidates(command_name) {
            let p = dir.join(&cand);
            if p.exists() {
                return Some(p);
            }
        }
    }
    // Fallback：扫已知 npm 全局 bin 目录
    for dir in well_known_npm_dirs() {
        for cand in executable_candidates(command_name) {
            let p = dir.join(&cand);
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

fn well_known_npm_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if cfg!(target_os = "windows") {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            out.push(PathBuf::from(appdata).join("npm"));
        }
        // nvm-windows 当前激活版本通过 %APPDATA%\nvm\nodejs symlink 暴露
        if let Some(appdata) = std::env::var_os("APPDATA") {
            out.push(PathBuf::from(appdata).join("nvm").join("nodejs"));
        }
    } else {
        if let Some(home) = dirs::home_dir() {
            out.push(home.join(".npm-global").join("bin"));
            out.push(home.join(".local").join("bin"));
        }
        out.push(PathBuf::from("/usr/local/bin"));
        out.push(PathBuf::from("/opt/homebrew/bin"));
    }
    out
}

async fn detect_via_bin_dir(
    bin_dir: &Path,
    command_name: &str,
    source: InstallationSource,
) -> Option<ToolInstallation> {
    if !bin_dir.exists() {
        return None;
    }
    for cand in executable_candidates(command_name) {
        let path = bin_dir.join(&cand);
        if path.exists() {
            let version = run_version(&path, command_name).await;
            return Some(ToolInstallation {
                source,
                version,
                path: Some(path.to_string_lossy().to_string()),
                current_path: false,
                on_path: false,
                managed: false,
            });
        }
    }
    None
}

async fn pnpm_yarn_bun_bins() -> Vec<(InstallationSource, PathBuf)> {
    let mut out = Vec::new();
    if let Some(p) = run_path_command("pnpm", &["bin", "-g"]).await {
        out.push((InstallationSource::Pnpm, p));
    }
    if let Some(p) = run_path_command("yarn", &["global", "bin"]).await {
        out.push((InstallationSource::Yarn, p));
    }
    if let Some(p) = bun_global_bin().await {
        out.push((InstallationSource::Bun, p));
    }
    out
}

async fn bun_global_bin() -> Option<PathBuf> {
    if let Some(custom) = std::env::var_os("BUN_INSTALL") {
        let bin = PathBuf::from(custom).join("bin");
        if bin.exists() {
            return Some(bin);
        }
    }
    let bin = dirs::home_dir()?.join(".bun").join("bin");
    if bin.exists() {
        Some(bin)
    } else {
        None
    }
}

async fn run_path_command(program: &str, args: &[&str]) -> Option<PathBuf> {
    let output = shell_command(program).args(args).output().await.ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        return None;
    }
    let p = PathBuf::from(raw);
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

async fn detect_nvm(command_name: &str) -> Vec<ToolInstallation> {
    let mut out = Vec::new();

    // Unix nvm: ~/.nvm/versions/node/<v>/bin/<cmd>
    if let Some(home) = dirs::home_dir() {
        let nvm_dir = home.join(".nvm").join("versions").join("node");
        if let Ok(entries) = std::fs::read_dir(&nvm_dir) {
            for e in entries.flatten() {
                let bin_path = e.path().join("bin").join(command_name);
                if bin_path.exists() {
                    let version = run_version(&bin_path, command_name).await;
                    out.push(ToolInstallation {
                        source: InstallationSource::Nvm,
                        version,
                        path: Some(bin_path.to_string_lossy().to_string()),
                        current_path: false,
                        on_path: false,
                        managed: false,
                    });
                }
            }
        }
    }

    // Windows nvm-windows: %APPDATA%\nvm\v<x.y.z>\<cmd>(.cmd|.exe|.ps1)
    if cfg!(target_os = "windows") {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let nvm_dir = PathBuf::from(appdata).join("nvm");
            if let Ok(entries) = std::fs::read_dir(&nvm_dir) {
                for e in entries.flatten() {
                    let dir = e.path();
                    if !dir.is_dir() {
                        continue;
                    }
                    let name = dir.file_name().and_then(|s| s.to_str()).unwrap_or("");
                    if !name.starts_with('v') {
                        continue;
                    }
                    for cand in executable_candidates(command_name) {
                        let bin_path = dir.join(&cand);
                        if bin_path.exists() {
                            let version = run_version(&bin_path, command_name).await;
                            out.push(ToolInstallation {
                                source: InstallationSource::Nvm,
                                version,
                                path: Some(bin_path.to_string_lossy().to_string()),
                                current_path: false,
                                on_path: false,
                                managed: false,
                            });
                            break;
                        }
                    }
                }
            }
        }
    }

    out
}

fn push_unique(installs: &mut Vec<ToolInstallation>, new_install: ToolInstallation) {
    let already = new_install
        .path
        .as_deref()
        .map(|p| {
            installs
                .iter()
                .any(|i| i.path.as_deref().is_some_and(|q| same_path(q, p)))
        })
        .unwrap_or(false);
    if !already {
        installs.push(new_install);
    }
}

fn current_path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|raw| std::env::split_paths(&raw).collect())
        .unwrap_or_default()
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

pub async fn run_version(path: &Path, command_name: &str) -> Option<String> {
    let raw = run_executable(path, &["--version"]).await?;
    parse_version(command_name, &raw)
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
    let path = Path::new(path);
    if cfg!(target_os = "windows") {
        let mut path_components = path.components();
        for dir_component in dir.components() {
            let Some(path_component) = path_components.next() else {
                return false;
            };
            let dir_str = dir_component.as_os_str().to_string_lossy();
            let path_str = path_component.as_os_str().to_string_lossy();
            if !dir_str.eq_ignore_ascii_case(&path_str) {
                return false;
            }
        }
        true
    } else {
        path.starts_with(dir)
    }
}

fn same_path(a: &str, b: &str) -> bool {
    if cfg!(target_os = "windows") {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

fn same_path_buf(a: &Path, b: &Path) -> bool {
    let a = a.to_string_lossy();
    let b = b.to_string_lossy();
    if cfg!(target_os = "windows") {
        a.eq_ignore_ascii_case(&b)
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_version, path_starts_with};
    use std::path::Path;

    #[cfg(target_os = "windows")]
    #[test]
    fn path_starts_with_is_case_insensitive_on_windows() {
        assert!(path_starts_with(
            r"C:\Users\Alice\AppData\Local\app\bin\claude.exe",
            Path::new(r"c:\users\alice\appdata\local\app"),
        ));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn path_starts_with_uses_byte_comparison_on_unix() {
        assert!(path_starts_with(
            "/home/alice/.local/share/app/bin/claude",
            Path::new("/home/alice/.local/share/app"),
        ));
        assert!(!path_starts_with(
            "/home/Alice/.local/share/app/bin/claude",
            Path::new("/home/alice/.local/share/app"),
        ));
    }

    #[test]
    fn path_starts_with_rejects_unrelated_path() {
        assert!(!path_starts_with(
            if cfg!(target_os = "windows") {
                r"C:\other\path\claude.exe"
            } else {
                "/other/path/claude"
            },
            Path::new(if cfg!(target_os = "windows") {
                r"C:\app"
            } else {
                "/app"
            }),
        ));
    }

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
