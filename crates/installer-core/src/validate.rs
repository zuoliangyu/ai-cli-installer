//! Sanity checks for strings that come off the network.
//!
//! Version pointers, manifest file names and npm tarball names are all served
//! by the mirror chain — which includes free third-party GH proxies. Anything
//! that ends up in a filesystem path or a version cache must pass through
//! here first, so a rate-limited proxy returning a 200 HTML page (or a
//! malicious one returning `../../.bashrc`) is treated as a mirror failure
//! instead of being written to disk.

use crate::error::{AppError, Result};

/// Upper bound for a version string. Real ones are ~10 chars; anything
/// longer is almost certainly an HTML error page that slipped through.
const MAX_VERSION_LEN: usize = 64;

/// Upper bound for a single file name (common filesystem limit).
const MAX_FILE_NAME_LEN: usize = 255;

/// `^\d+\.\d+\.\d+[0-9A-Za-z.+-]*$`, hand-rolled to avoid pulling in `regex`.
///
/// Accepts `2.1.132`, `0.128.0-alpha.1`, `1.0.0+build.5`; rejects
/// `<!DOCTYPE html>`, `v1.2.3`, `1.2`, `1.2.3/../x`, empty strings.
pub fn is_valid_version(s: &str) -> bool {
    if s.is_empty() || s.len() > MAX_VERSION_LEN {
        return false;
    }
    let bytes = s.as_bytes();
    let mut i = 0;
    // Three dot-separated numeric components; the third one isn't followed
    // by a mandatory dot.
    for component in 0..3 {
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return false;
        }
        if component < 2 {
            if i >= bytes.len() || bytes[i] != b'.' {
                return false;
            }
            i += 1;
        }
    }
    bytes[i..]
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-'))
}

/// True when `name` is a single, ordinary file name: non-empty, no path
/// separators, not `.`/`..`, no drive/ADS colon, no control characters.
/// Anything that would let `dir.join(name)` escape `dir` is rejected.
pub fn is_plain_file_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_FILE_NAME_LEN || name == "." || name == ".." {
        return false;
    }
    if name
        .chars()
        .any(|c| matches!(c, '/' | '\\' | ':' | '\0') || c.is_control())
    {
        return false;
    }
    // Belt and braces: the platform's own path parser must agree that this
    // is exactly one normal component.
    let mut components = std::path::Path::new(name).components();
    matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(_)), None)
    )
}

/// `Ok(name)` when [`is_plain_file_name`] accepts it, otherwise a
/// user-facing error naming the offending field.
pub fn ensure_file_name<'a>(name: &'a str, what: &str) -> Result<&'a str> {
    if is_plain_file_name(name) {
        Ok(name)
    } else {
        Err(AppError::Other(format!(
            "{} 不是合法的文件名：`{}`（可能是镜像返回了被篡改的数据）",
            what,
            name.escape_debug()
        )))
    }
}

/// `Ok(version)` when [`is_valid_version`] accepts it.
pub fn ensure_version<'a>(version: &'a str, what: &str) -> Result<&'a str> {
    if is_valid_version(version) {
        Ok(version)
    } else {
        Err(AppError::Other(format!(
            "{} 不是合法的版本号：`{}`",
            what,
            truncate_for_display(version)
        )))
    }
}

/// Short, single-line preview of an untrusted string for logs / errors.
pub(crate) fn truncate_for_display(s: &str) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(40)
        .collect();
    if s.chars().count() > 40 {
        format!("{}…", flat)
    } else {
        flat
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_real_versions() {
        for v in [
            "2.1.132",
            "0.128.0",
            "0.128.0-alpha.1",
            "1.0.0+build.5",
            "10.20.30rc1",
        ] {
            assert!(is_valid_version(v), "{v} should be valid");
        }
    }

    #[test]
    fn rejects_garbage_versions() {
        for v in [
            "",
            "<!DOCTYPE html>",
            "v1.2.3",
            "1.2",
            "1..2.3",
            "1.2.3/../../x",
            "1.2.3 extra",
            "1.2.3\n",
            "1.2.3_x",
            &"1.2.3".repeat(20),
        ] {
            assert!(!is_valid_version(v), "{v:?} should be rejected");
        }
    }

    #[test]
    fn accepts_plain_file_names() {
        for n in [
            "codex",
            "codex.exe",
            "claude-code-2.1.289.tgz",
            "codex-0.1.0-linux-x64.tgz",
        ] {
            assert!(is_plain_file_name(n), "{n} should be valid");
        }
    }

    #[test]
    fn rejects_path_like_file_names() {
        for n in [
            "",
            ".",
            "..",
            "../evil",
            "a/b",
            "a\\b",
            "/etc/passwd",
            "C:\\Windows\\x.exe",
            "C:evil",
            "file.txt:stream",
            "nul\0byte",
            "line\nbreak",
        ] {
            assert!(!is_plain_file_name(n), "{n:?} should be rejected");
        }
    }

    #[test]
    fn ensure_helpers_report_field() {
        let err = ensure_file_name("../x", "tgz").unwrap_err().to_string();
        assert!(err.contains("tgz"));
        assert!(ensure_version("1.2.3", "version").is_ok());
        assert!(ensure_version("<html>", "version").is_err());
    }
}
