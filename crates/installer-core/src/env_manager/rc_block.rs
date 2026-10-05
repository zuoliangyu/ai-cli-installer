//! Pure string / path logic behind the Unix rc-file PATH block. Kept out of
//! `unix.rs` so it compiles (and is unit-tested) on every platform.

use std::path::{Path, PathBuf};

pub const MARKER_BEGIN: &str = "# >>> ai-cli-installer (PATH) >>>";
pub const MARKER_END: &str = "# <<< ai-cli-installer (PATH) <<<";

/// rc files that never get created, only edited when they already exist.
const KNOWN_RC_FILES: &[&str] = &[".zshrc", ".bashrc", ".bash_profile", ".profile"];

/// The rc file the user's login shell actually reads, by `$SHELL`:
/// zsh → `.zshrc`; bash → `.bash_profile` on macOS (Terminal.app starts
/// login shells, which skip `.bashrc`) / `.bashrc` on Linux; anything else
/// → `.profile`.
pub fn preferred_rc(shell: Option<&str>, macos: bool) -> &'static str {
    let name = shell
        .map(Path::new)
        .and_then(Path::file_name)
        .and_then(|n| n.to_str())
        .unwrap_or("");
    match name {
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
    exists: impl Fn(&Path) -> bool,
) -> Vec<PathBuf> {
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

/// The marker block that puts `dir` on PATH.
pub fn path_block(dir: &str) -> String {
    format!(
        "\n{}\nexport PATH=\"{}:$PATH\"\n{}\n",
        MARKER_BEGIN, dir, MARKER_END
    )
}

/// `existing` with `block` appended (adding a newline first if needed), or
/// `None` when a block is already present.
pub fn append_block(existing: &str, block: &str) -> Option<String> {
    if existing.contains(MARKER_BEGIN) {
        return None;
    }
    let mut out = existing.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(block);
    Some(out)
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
        let added = append_block(original, &path_block("/home/u/.local/bin")).unwrap();
        assert!(added.contains("export PATH=\"/home/u/.local/bin:$PATH\""));
        assert_eq!(append_block(&added, &path_block("/x")), None);
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
        assert_eq!(preferred_rc(Some("/usr/bin/fish"), false), ".profile");
        assert_eq!(preferred_rc(None, true), ".profile");
    }

    #[test]
    fn rc_targets_include_existing_and_preferred() {
        let home = Path::new("/h");
        // Fresh macOS zsh account: only .zshrc, created.
        assert_eq!(
            rc_targets(home, Some("/bin/zsh"), true, |_| false),
            vec![home.join(".zshrc")]
        );
        // Existing .profile on a zsh system: keep editing it, but also .zshrc.
        let targets = rc_targets(home, Some("/bin/zsh"), true, |p| p.ends_with(".profile"));
        assert_eq!(targets, vec![home.join(".profile"), home.join(".zshrc")]);
        // No duplicates when the preferred file already exists.
        let targets = rc_targets(home, Some("/bin/bash"), false, |p| p.ends_with(".bashrc"));
        assert_eq!(targets, vec![home.join(".bashrc")]);
    }
}
