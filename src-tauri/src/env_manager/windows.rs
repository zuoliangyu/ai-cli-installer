//! Windows PATH manipulation via registry + elevated PowerShell.
//!
//! Reading: open the env keys read-only, no elevation.
//! Writing System PATH: spawn `powershell -Verb RunAs` with an inline script that
//! mutates HKLM and broadcasts WM_SETTINGCHANGE so new processes see the change.
//! User PATH: same approach via HKCU, but no elevation needed.

use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;
use winreg::enums::*;
use winreg::RegKey;

use super::{PathScope, PathStatus};
use crate::error::{AppError, Result};

const HKCU_ENV: &str = r"Environment";
const HKLM_ENV: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";

fn read_user_path() -> Result<String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let env = hkcu
        .open_subkey(HKCU_ENV)
        .map_err(|e| AppError::Other(format!("open HKCU\\Environment: {}", e)))?;
    Ok(env.get_value::<String, _>("Path").unwrap_or_default())
}

fn read_system_path() -> Result<String> {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let env = hklm
        .open_subkey(HKLM_ENV)
        .map_err(|e| AppError::Other(format!("open HKLM env: {}", e)))?;
    Ok(env.get_value::<String, _>("Path").unwrap_or_default())
}

fn path_contains(path: &str, dir: &str) -> bool {
    path.split(';').any(|p| {
        let p = p.trim().trim_end_matches('\\');
        let d = dir.trim().trim_end_matches('\\');
        // Windows paths are case-insensitive
        p.eq_ignore_ascii_case(d)
    })
}

pub async fn status(dir: &Path) -> Result<PathStatus> {
    let dir_str = dir.to_string_lossy().to_string();
    let user_path = read_user_path().unwrap_or_default();
    let system_path = read_system_path().unwrap_or_default();
    let in_user_path = path_contains(&user_path, &dir_str);
    let in_system_path = path_contains(&system_path, &dir_str);
    let effective = in_user_path
        || in_system_path
        || std::env::var("PATH")
            .map(|p| {
                p.split(';')
                    .any(|s| s.trim().eq_ignore_ascii_case(dir_str.trim()))
            })
            .unwrap_or(false);
    Ok(PathStatus {
        dir: dir_str,
        in_user_path,
        in_system_path,
        effective,
    })
}

/// Run a PowerShell snippet with elevation (UAC). Returns Ok(()) only if the
/// elevated process exits with code 0.
async fn run_elevated_powershell(script: &str) -> Result<()> {
    // Outer PowerShell starts the inner one with -Verb RunAs (UAC).
    // -Wait makes Start-Process block; -PassThru lets us read the exit code.
    let outer = format!(
        r#"$p = Start-Process powershell -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList '-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-Command',{script_lit}; exit $p.ExitCode"#,
        script_lit = ps_single_quote(script)
    );

    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &outer,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| AppError::Other(format!("spawn powershell: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let code = output.status.code().unwrap_or(-1);
        // Common UAC denial: exit code 1223 from Start-Process is "operation cancelled"
        if code == 1223 || stderr.contains("cancel") {
            return Err(AppError::Other(
                "用户取消了管理员授权（UAC）。系统 PATH 未修改。".into(),
            ));
        }
        return Err(AppError::Other(format!(
            "elevated powershell exit={} stdout={} stderr={}",
            code, stdout, stderr
        )));
    }
    Ok(())
}

/// Wrap a string as a single-quoted PowerShell literal (escape internal `'` as `''`).
fn ps_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn add_script(scope: PathScope, dir: &str) -> String {
    let target = match scope {
        PathScope::System => "Machine",
        PathScope::User => "User",
    };
    // Idempotent: only append if not already present (case-insensitive).
    format!(
        r#"
$target = '{target}'
$dir = '{dir}'
$cur = [Environment]::GetEnvironmentVariable('Path', $target)
if (-not $cur) {{ $cur = '' }}
$parts = $cur -split ';' | ForEach-Object {{ $_.TrimEnd('\') }}
$dirNorm = $dir.TrimEnd('\')
$present = $false
foreach ($p in $parts) {{ if ($p -ieq $dirNorm) {{ $present = $true; break }} }}
if (-not $present) {{
    if ($cur -and -not $cur.EndsWith(';')) {{ $cur += ';' }}
    $new = $cur + $dir
    [Environment]::SetEnvironmentVariable('Path', $new, $target)
}}
exit 0
"#,
        target = target,
        dir = dir.replace('\'', "''")
    )
}

fn remove_script(scope: PathScope, dir: &str) -> String {
    let target = match scope {
        PathScope::System => "Machine",
        PathScope::User => "User",
    };
    format!(
        r#"
$target = '{target}'
$dir = '{dir}'
$cur = [Environment]::GetEnvironmentVariable('Path', $target)
if (-not $cur) {{ exit 0 }}
$dirNorm = $dir.TrimEnd('\')
$kept = $cur -split ';' | Where-Object {{ $_ -and ($_.TrimEnd('\') -ine $dirNorm) }}
$new = ($kept -join ';')
[Environment]::SetEnvironmentVariable('Path', $new, $target)
exit 0
"#,
        target = target,
        dir = dir.replace('\'', "''")
    )
}

pub async fn add(dir: &Path, scope: PathScope) -> Result<()> {
    let dir_str = dir.to_string_lossy().to_string();
    let script = add_script(scope, &dir_str);
    match scope {
        PathScope::System => run_elevated_powershell(&script).await,
        PathScope::User => run_local_powershell(&script).await,
    }
}

pub async fn remove(dir: &Path, scope: PathScope) -> Result<()> {
    let dir_str = dir.to_string_lossy().to_string();
    let script = remove_script(scope, &dir_str);
    match scope {
        PathScope::System => run_elevated_powershell(&script).await,
        PathScope::User => run_local_powershell(&script).await,
    }
}

async fn run_local_powershell(script: &str) -> Result<()> {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| AppError::Other(format!("spawn powershell: {}", e)))?;
    if !output.status.success() {
        return Err(AppError::Other(format!(
            "powershell failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}
