//! Windows PATH manipulation via registry + elevated PowerShell.
//!
//! Reading: open the env keys read-only, no elevation.
//! Writing System PATH: spawn `powershell -Verb RunAs` with an inline script that
//! mutates HKLM and broadcasts WM_SETTINGCHANGE so new processes see the change.
//! User PATH: same approach via HKCU, but no elevation needed.
//!
//! The scripts go through the registry API directly rather than
//! `[Environment]::Get/SetEnvironmentVariable`: those return the *expanded*
//! value and write it back as a plain string, permanently turning entries
//! like `%USERPROFILE%\AppData\Local\Microsoft\WindowsApps` into hard-coded
//! paths (and `REG_EXPAND_SZ` into `REG_SZ`). We read with
//! `DoNotExpandEnvironmentNames`, write `ExpandString`, and broadcast
//! `WM_SETTINGCHANGE` ourselves.

use std::path::Path;
use std::time::Duration;
use tokio::process::Command;

use crate::proc::{output_with_timeout, silence_windows, INSTALL_TIMEOUT};
use winreg::enums::*;
use winreg::RegKey;

use super::{PathScope, PathStatus};
use crate::error::{AppError, Result};

const HKCU_ENV: &str = r"Environment";
const HKLM_ENV: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";

/// Non-elevated PowerShell (registry edit + `Add-Type` for the broadcast)
/// takes a few seconds on a cold machine; anything beyond this is a hang.
const LOCAL_PS_TIMEOUT: Duration = Duration::from_secs(60);

/// Raw (unexpanded) value — `winreg` doesn't expand `REG_EXPAND_SZ`.
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

/// Expand `%NAME%` references using `lookup`; unknown names are left as-is
/// (same as Windows itself).
fn expand_env_vars(s: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match lookup(name) {
                    Some(value) => out.push_str(&value),
                    None => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn path_contains(path: &str, dir: &str) -> bool {
    let d = dir.trim().trim_end_matches('\\');
    path.split(';').any(|p| {
        let expanded = expand_env_vars(p.trim(), |name| std::env::var(name).ok());
        // Windows paths are case-insensitive
        expanded.trim_end_matches('\\').eq_ignore_ascii_case(d)
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
            .map(|p| path_contains(&p, &dir_str))
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
    // The inner script travels as -EncodedCommand: Start-Process joins
    // -ArgumentList with spaces and doesn't escape quotes, so a plain
    // -Command would mangle anything containing `"`.
    let outer = format!(
        r#"$p = Start-Process powershell -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList '-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-EncodedCommand','{encoded}'; exit $p.ExitCode"#,
        encoded = encode_ps_command(script)
    );

    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        &outer,
    ]);
    silence_windows(&mut cmd);
    // Includes the time the user spends looking at the UAC prompt.
    let output = output_with_timeout(&mut cmd, INSTALL_TIMEOUT)
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

/// `-EncodedCommand` payload: base64 of the UTF-16LE script.
fn encode_ps_command(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64_encode(&bytes)
}

fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Wrap a string as a single-quoted PowerShell literal (escape internal `'` as `''`).
fn ps_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Opens the scope's Environment key writable as `$key`.
fn open_key_snippet(scope: PathScope) -> &'static str {
    match scope {
        PathScope::System => {
            r"$key = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey('SYSTEM\CurrentControlSet\Control\Session Manager\Environment', $true)"
        }
        PathScope::User => r"$key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment')",
    }
}

/// Tell Explorer & friends to reload the environment. Failure is non-fatal:
/// the registry write already happened, new logons pick it up regardless.
const BROADCAST_SNIPPET: &str = r#"
try {
    $sig = '[DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)] public static extern IntPtr SendMessageTimeout(IntPtr hWnd, uint Msg, UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out UIntPtr lpdwResult);'
    $native = Add-Type -MemberDefinition $sig -Name 'EnvBroadcast' -Namespace 'AiCliInstaller' -PassThru
    $res = [UIntPtr]::Zero
    [void]$native::SendMessageTimeout([IntPtr]0xffff, 0x1A, [UIntPtr]::Zero, 'Environment', 2, 5000, [ref]$res)
} catch { }
"#;

/// Shared prologue: open the key and read the raw, unexpanded Path.
const READ_SNIPPET: &str = r#"
$ErrorActionPreference = 'Stop'
__OPEN_KEY__
$cur = [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
$dir = __DIR__
$dirNorm = $dir.TrimEnd('\')
"#;

fn add_script(scope: PathScope, dir: &str) -> String {
    // Idempotent: only append if not already present (case-insensitive,
    // comparing expanded forms so `%USERPROFILE%\.local\bin` counts).
    let body = r#"
$present = $false
foreach ($p in ($cur -split ';')) {
    if ($p -and ([Environment]::ExpandEnvironmentVariables($p).TrimEnd('\') -ieq $dirNorm)) { $present = $true; break }
}
if (-not $present) {
    if ($cur -and -not $cur.EndsWith(';')) { $cur += ';' }
    $key.SetValue('Path', $cur + $dir, [Microsoft.Win32.RegistryValueKind]::ExpandString)
    __BROADCAST__
}
$key.Close()
exit 0
"#;
    build_script(scope, dir, body)
}

fn remove_script(scope: PathScope, dir: &str) -> String {
    // Other entries are kept byte-for-byte (still unexpanded).
    let body = r#"
if (-not $cur) { $key.Close(); exit 0 }
$kept = @($cur -split ';' | Where-Object { $_ -and ([Environment]::ExpandEnvironmentVariables($_).TrimEnd('\') -ine $dirNorm) })
$new = ($kept -join ';')
if ($new -ne $cur) {
    $key.SetValue('Path', $new, [Microsoft.Win32.RegistryValueKind]::ExpandString)
    __BROADCAST__
}
$key.Close()
exit 0
"#;
    build_script(scope, dir, body)
}

fn build_script(scope: PathScope, dir: &str, body: &str) -> String {
    let mut script = String::from(READ_SNIPPET);
    script.push_str(body);
    script
        .replace("__OPEN_KEY__", open_key_snippet(scope))
        .replace("__DIR__", &ps_single_quote(dir))
        .replace("__BROADCAST__", BROADCAST_SNIPPET)
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
    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-EncodedCommand",
        &encode_ps_command(script),
    ]);
    silence_windows(&mut cmd);
    let output = output_with_timeout(&mut cmd, LOCAL_PS_TIMEOUT)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_known_vars_and_keeps_unknown() {
        let lookup = |name: &str| match name {
            "USERPROFILE" => Some(r"C:\Users\alice".to_string()),
            _ => None,
        };
        assert_eq!(
            expand_env_vars(r"%USERPROFILE%\.local\bin", lookup),
            r"C:\Users\alice\.local\bin"
        );
        assert_eq!(expand_env_vars(r"%NOPE%\x", lookup), r"%NOPE%\x");
        assert_eq!(expand_env_vars("100% sure", lookup), "100% sure");
        assert_eq!(expand_env_vars("%%", lookup), "%%");
    }

    #[test]
    fn path_contains_matches_unexpanded_entries() {
        let profile = std::env::var("USERPROFILE").unwrap();
        let dir = format!(r"{}\.local\bin", profile);
        assert!(path_contains(r"C:\Windows;%USERPROFILE%\.local\bin\", &dir));
        assert!(!path_contains(r"C:\Windows", &dir));
    }

    #[test]
    fn base64_matches_reference() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        // "a" as UTF-16LE = 61 00
        assert_eq!(encode_ps_command("a"), "YQA=");
    }

    #[test]
    fn scripts_preserve_unexpanded_values() {
        let s = add_script(PathScope::User, r"C:\it's\bin");
        assert!(s.contains("DoNotExpandEnvironmentNames"));
        assert!(s.contains("ExpandString"));
        assert!(s.contains(r"'C:\it''s\bin'"));
        assert!(!s.contains("SetEnvironmentVariable"));
        assert!(!s.contains("__"));
        let r = remove_script(PathScope::System, r"C:\x");
        assert!(r.contains("LocalMachine"));
        assert!(r.contains("SendMessageTimeout"));
    }
}
