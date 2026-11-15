//! Pure string / path logic behind the Unix rc-file PATH block. Kept out of
//! `unix.rs` so it compiles (and is unit-tested) on every platform.

use std::path::{Path, PathBuf};

pub const MARKER_BEGIN: &str = "# >>> ai-cli-installer (PATH) >>>";
pub const MARKER_END: &str = "# <<< ai-cli-installer (PATH) <<<";

/// rc files that never get created, only edited when they already exist.
const KNOWN_RC_FILES: &[&str] = &[".zshrc", ".bashrc", ".bash_profile", ".profile"];

/// The rc file the user's login shell actually reads, by `$SHELL`:
/// zsh → `.zshrc`; bash → `.bash_profile` on macOS (Terminal.app starts
/// login shells, which skip `.bashrc`) / `.bashrc` on Linux. Fish uses its
/// own startup snippet; other shells fall back to `.profile`.
pub fn preferred_rc(shell: Option<&str>, macos: bool) -> &'static str {
    let name = shell
        .map(Path::new)
        .and_then(Path::file_name)
        .and_then(|n| n.to_str())
        .unwrap_or("");
    match name {
        "fish" => ".config/fish/conf.d/ai-cli-installer.fish",
        "zsh" => ".zshrc",
        "bash" if macos => ".bash_profile",
        "bash" => ".bashrc",
        _ => ".profile",
    }
}

/// Every known rc file that exists, plus the current shell's preferred one
/// (created if missing). Pre-v0.5.4 a fresh macOS account (zsh, no rc files)
/// only got `.profile`, which zsh never reads.
pub fn rc_targets(
    home: &Path,
    shell: Option<&str>,
    macos: bool,
    xdg_config_home: Option<&Path>,
    exists: impl Fn(&Path) -> bool,
) -> Vec<PathBuf> {
    if is_fish(shell) {
        let config_home = xdg_config_home
            .filter(|path| path.is_absolute())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| home.join(".config"));
        return vec![config_home.join("fish/conf.d/ai-cli-installer.fish")];
    }
    let mut out: Vec<PathBuf> = KNOWN_RC_FILES
        .iter()
        .map(|n| home.join(n))
        .filter(|p| exists(p))
        .collect();
    let preferred = home.join(preferred_rc(shell, macos));
    if !out.contains(&preferred) {
        out.push(preferred);
    }
    out
}

fn is_fish(shell: Option<&str>) -> bool {
    shell
        .map(Path::new)
        .and_then(Path::file_name)
        .is_some_and(|name| name == "fish")
}

/// The marker block that puts the literal directory on PATH.
pub fn path_block(dir: &str, shell: Option<&str>) -> String {
    let command = if is_fish(shell) {
        let quoted = format!("'{}'", dir.replace('\\', "\\\\").replace('\'', "\\'"));
        format!("if not contains -- {quoted} $PATH\n    set -gx PATH {quoted} $PATH\nend")
    } else {
        let quoted = format!("'{}'", dir.replace('\'', "'\\''"));
        format!("export PATH={quoted}:\"$PATH\"")
    };
    format!("\n{}\n{}\n{}\n", MARKER_BEGIN, command, MARKER_END)
}

/// Insert or refresh complete marker blocks in place, preserving every byte
/// outside them. A broken block is an error rather than a guessed edit.
pub fn append_block(existing: &str, block: &str) -> std::io::Result<Option<String>> {
    let lines: Vec<&str> = existing.split_inclusive('\n').collect();
    let mut out = String::with_capacity(existing.len() + block.len());
    let mut found = false;
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim() == MARKER_BEGIN {
            let end = lines[i + 1..]
                .iter()
                .position(|line| line.trim() == MARKER_END)
                .map(|offset| i + 1 + offset)
                .ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "PATH 标记块缺少结束标记，请先手动修复",
                    )
                })?;
            out.push_str(block.trim_start_matches('\n'));
            found = true;
            i = end + 1;
        } else {
            out.push_str(lines[i]);
            i += 1;
        }
    }
    if !found {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(block);
    }
    Ok((out != existing).then_some(out))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Strip {
    /// Every marker block was removed; here's the new content.
    Removed(String),
    /// No begin marker in the file.
    NotFound,
    /// A begin marker without a matching end marker (the user edited the
    /// block). We refuse to guess where it ends — deleting "everything after
    /// the begin marker" used to wipe the rest of the user's rc file.
    Incomplete,
}

/// Remove every complete `MARKER_BEGIN ... MARKER_END` block, plus the blank
/// line [`path_block`] puts in front of it. Line endings are preserved.
pub fn strip_marker_block(s: &str) -> Strip {
    let lines: Vec<&str> = s.split_inclusive('\n').collect();
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    let mut removed = false;
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim() == MARKER_BEGIN {
            let Some(end) = lines[i + 1..]
                .iter()
                .position(|l| l.trim() == MARKER_END)
                .map(|p| i + 1 + p)
            else {
                return Strip::Incomplete;
            };
            if out.last().is_some_and(|l| l.trim().is_empty()) {
                out.pop();
            }
            removed = true;
            i = end + 1;
            continue;
        }
        out.push(lines[i]);
        i += 1;
    }
    if removed {
        Strip::Removed(out.concat())
    } else {
        Strip::NotFound
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_then_remove_round_trips() {
        let original = "export FOO=1\nalias ll='ls -l'\n";
        let block = path_block("/home/u/.local/bin", Some("/bin/bash"));
        let added = append_block(original, &block).unwrap().unwrap();
        assert!(added.contains("export PATH='/home/u/.local/bin':\"$PATH\""));
        assert_eq!(append_block(&added, &block).unwrap(), None);
        assert_eq!(
            strip_marker_block(&added),
            Strip::Removed(original.to_string())
        );
    }

    #[test]
    fn missing_end_marker_leaves_file_alone() {
        let s = format!(
            "a\n{}\nexport PATH=x\nuser stuff\nmore stuff\n",
            MARKER_BEGIN
        );
        assert_eq!(strip_marker_block(&s), Strip::Incomplete);
        assert!(append_block(&s, &path_block("/safe", None)).is_err());
    }

    #[test]
    fn no_block_is_not_found_and_crlf_is_preserved() {
        assert_eq!(strip_marker_block("a\nb\n"), Strip::NotFound);
        let s = format!(
            "a\r\n{}\r\nexport PATH=x\r\n{}\r\nb\r\n",
            MARKER_BEGIN, MARKER_END
        );
        assert_eq!(
            strip_marker_block(&s),
            Strip::Removed("a\r\nb\r\n".to_string())
        );
    }

    #[test]
    fn preferred_rc_follows_shell() {
        assert_eq!(preferred_rc(Some("/bin/zsh"), true), ".zshrc");
        assert_eq!(preferred_rc(Some("/bin/bash"), true), ".bash_profile");
        assert_eq!(preferred_rc(Some("/usr/bin/bash"), false), ".bashrc");
        assert_eq!(
            preferred_rc(Some("/usr/bin/fish"), false),
            ".config/fish/conf.d/ai-cli-installer.fish"
        );
        assert_eq!(preferred_rc(None, true), ".profile");
    }

    #[test]
    fn rc_targets_include_existing_and_preferred() {
        let home = Path::new("/h");
        // Fresh macOS zsh account: only .zshrc, created.
        assert_eq!(
            rc_targets(home, Some("/bin/zsh"), true, None, |_| false),
            vec![home.join(".zshrc")]
        );
        // Existing .profile on a zsh system: keep editing it, but also .zshrc.
        let targets = rc_targets(home, Some("/bin/zsh"), true, None, |p| {
            p.ends_with(".profile")
        });
        assert_eq!(targets, vec![home.join(".profile"), home.join(".zshrc")]);
        // No duplicates when the preferred file already exists.
        let targets = rc_targets(home, Some("/bin/bash"), false, None, |p| {
            p.ends_with(".bashrc")
        });
        assert_eq!(targets, vec![home.join(".bashrc")]);
    }

    #[test]
    fn fish_targets_use_xdg_and_never_treat_profile_as_active() {
        let home = std::env::temp_dir().join("fish-home");
        let custom_config = std::env::temp_dir().join("fish-custom-config");
        for shell in ["/usr/bin/fish", "/opt/homebrew/bin/fish"] {
            let targets = rc_targets(&home, Some(shell), false, Some(&custom_config), |_| true);
            assert_eq!(
                targets,
                vec![custom_config.join("fish/conf.d/ai-cli-installer.fish")]
            );
            for xdg in [
                None,
                Some(Path::new("relative-config")),
                Some(Path::new("")),
            ] {
                assert_eq!(
                    rc_targets(&home, Some(shell), false, xdg, |_| true),
                    vec![home.join(".config/fish/conf.d/ai-cli-installer.fish")]
                );
            }
        }
    }

    #[test]
    fn refreshing_legacy_block_preserves_surrounding_content() {
        let before = "# personal settings\r\nalias ll='ls -l'\r\n\r\n";
        let after = "# keep this after the managed block\r\nexport FOO=bar\r\n";
        let old =
            format!("{before}{MARKER_BEGIN}\nexport PATH=\"/old:$PATH\"\n{MARKER_END}\n{after}");
        let new_block = path_block("/home/it's $literal/bin", Some("/bin/zsh"));
        let updated = append_block(&old, &new_block).unwrap().unwrap();
        assert_eq!(
            updated,
            format!("{before}{}{after}", new_block.trim_start_matches('\n'))
        );
        assert_eq!(append_block(&updated, &new_block).unwrap(), None);
    }

    #[test]
    fn fish_block_quotes_literal_directory_and_round_trips() {
        let dir = "/home/a\\b ' $name `echo no` \"quoted\"/bin";
        let block = path_block(dir, Some("/usr/bin/fish"));
        let quoted = "'/home/a\\\\b \\' $name `echo no` \"quoted\"/bin'";
        assert!(block.contains(&format!("contains -- {quoted} $PATH")));
        assert!(block.contains(&format!("set -gx PATH {quoted} $PATH")));
        assert!(!block.contains("export PATH="));
        let original = "set -gx PERSONAL unchanged\n";
        let added = append_block(original, &block).unwrap().unwrap();
        assert_eq!(append_block(&added, &block).unwrap(), None);
        assert_eq!(strip_marker_block(&added), Strip::Removed(original.into()));
    }

    #[test]
    fn posix_directory_is_quoted_as_one_literal() {
        let block = path_block(
            "/home/a ' $name `echo no` \"quoted\"/bin",
            Some("/bin/bash"),
        );
        assert!(
            block.contains("export PATH='/home/a '\\'' $name `echo no` \"quoted\"/bin':\"$PATH\"")
        );
    }

    #[cfg(unix)]
    #[test]
    fn posix_shell_preserves_special_directory_without_expansion() {
        let dir = "/tmp/a ' \" $SHOULD_NOT_EXPAND $(printf injected) `printf injected` \\bin";
        let script = format!(
            "PATH=/usr/bin:/bin\n{}printf '%s' \"$PATH\"",
            path_block(dir, Some("/bin/sh"))
        );
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", &script])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("{dir}:/usr/bin:/bin")
        );
    }
}
