//! Safe read-modify-write for user config files (`~/.claude/settings.json`,
//! `~/.claude.json`, shell rc files).
//!
//! - The whole read → modify → write cycle runs under one lock, so two
//!   concurrent edits (UI apply + post-install auto-apply) can't lose each
//!   other's changes.
//! - Writes go to a sibling temp file which is fsynced and renamed over the
//!   target: a crash mid-write never leaves a half-written JSON behind.
//! - Unchanged content is not rewritten and produces no backup.
//! - `.bak` / `.bak.N` backups are rotated, keeping the newest
//!   [`MAX_BACKUPS`].

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

// ponytail: one global lock is enough for tiny config writes; use per-file locks if throughput matters.
static CONFIG_WRITE_LOCK: Mutex<()> = Mutex::new(());

/// How many `.bak*` files to keep per config file.
pub const MAX_BACKUPS: usize = 5;

/// Replace `path` with `content` (backup + atomic write). No-op when the
/// file already holds exactly `content`.
#[cfg(test)]
pub fn write_with_backup(path: &Path, content: impl AsRef<[u8]>) -> std::io::Result<()> {
    let content = content.as_ref();
    update_with_backup(path, |_| Ok::<_, std::io::Error>(Some(content.to_vec())))?;
    Ok(())
}

/// Read `path` (as `None` when missing), let `edit` compute the new content,
/// and write it back — all under [`CONFIG_WRITE_LOCK`].
///
/// `edit` returns `Ok(None)` for "nothing to change". Returns `Ok(true)`
/// when the file was actually rewritten.
pub fn update_with_backup<E, F>(path: &Path, edit: F) -> Result<bool, E>
where
    E: From<std::io::Error>,
    F: FnOnce(Option<&str>) -> Result<Option<Vec<u8>>, E>,
{
    let _guard = CONFIG_WRITE_LOCK
        .lock()
        .map_err(|_| std::io::Error::other("config write lock poisoned"))?;

    // Follow a symlinked config (dotfile managers) so the rename below
    // replaces the real file instead of the link.
    let target = resolve_symlink(path);
    let current = match std::fs::read(&target) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let current_text = current
        .as_deref()
        .map(|b| String::from_utf8_lossy(b).into_owned());

    let Some(new_content) = edit(current_text.as_deref())? else {
        return Ok(false);
    };
    if current.as_deref() == Some(new_content.as_slice()) {
        return Ok(false);
    }

    if current.is_some() {
        let backup = next_backup_path(&target);
        std::fs::copy(&target, &backup)?;
        prune_backups(&target, MAX_BACKUPS);
    } else if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    atomic_write(&target, &new_content)?;
    Ok(true)
}

fn resolve_symlink(path: &Path) -> PathBuf {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
        }
        _ => path.to_path_buf(),
    }
}

/// Write `content` to a temp file next to `path`, fsync, then rename over
/// `path`. Permissions of an existing `path` are carried over (e.g. a 0600
/// `~/.claude.json` must not become world-readable).
pub(crate) fn atomic_write(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let file_name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other(format!("invalid path: {}", path.display())))?;
    let mut tmp_name = std::ffi::OsString::from(".");
    tmp_name.push(file_name);
    tmp_name.push(format!(".tmp-{}", std::process::id()));
    let tmp = path.with_file_name(tmp_name);

    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(content)?;
        f.sync_all()?;
        drop(f);
        if let Ok(meta) = std::fs::metadata(path) {
            std::fs::set_permissions(&tmp, meta.permissions())?;
        }
        std::fs::rename(&tmp, path)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        // Windows refuses to rename over a file another process holds open
        // without FILE_SHARE_DELETE. Writing in place is still better than
        // failing the user's edit outright.
        if cfg!(windows) && path.exists() {
            tracing::warn!(
                "atomic rename onto {} failed ({}); writing in place",
                path.display(),
                e
            );
            return std::fs::write(path, content);
        }
        return Err(e);
    }
    Ok(())
}

/// `path.bak` is index 0, `path.bak.N` is index N.
fn backup_index(path: &Path, candidate: &Path) -> Option<u64> {
    let base = path.file_name()?.to_str()?;
    let name = candidate.file_name()?.to_str()?;
    let rest = name.strip_prefix(base)?.strip_prefix(".bak")?;
    if rest.is_empty() {
        return Some(0);
    }
    rest.strip_prefix('.')?.parse().ok()
}

fn existing_backups(path: &Path) -> Vec<(u64, PathBuf)> {
    let Some(dir) = path.parent() else {
        return Vec::new();
    };
    let dir = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    let mut out: Vec<(u64, PathBuf)> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter_map(|p| backup_index(path, &p).map(|i| (i, p)))
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|(i, _)| *i);
    out
}

/// Next backup slot: `.bak` first, then one past the highest existing index
/// (not the first gap), so index order always equals age order.
fn next_backup_path(path: &Path) -> PathBuf {
    let mut backup = path.as_os_str().to_os_string();
    backup.push(".bak");
    match existing_backups(path).last() {
        None => PathBuf::from(backup),
        Some((max, _)) => {
            backup.push(format!(".{}", max + 1));
            PathBuf::from(backup)
        }
    }
}

/// Delete the oldest backups so at most `keep` remain.
fn prune_backups(path: &Path, keep: usize) {
    let backups = existing_backups(path);
    if backups.len() <= keep {
        return;
    }
    for (_, old) in &backups[..backups.len() - keep] {
        if let Err(e) = std::fs::remove_file(old) {
            tracing::warn!("remove old backup {}: {}", old.display(), e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ai-cli-installer-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn preserves_the_previous_file() {
        let dir = scratch_dir("backup");
        let path = dir.join("settings.json");
        std::fs::write(&path, "before").unwrap();

        write_with_backup(&path, "after").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "after");
        assert_eq!(
            std::fs::read_to_string(path.with_extension("json.bak")).unwrap(),
            "before"
        );
        write_with_backup(&path, "latest").unwrap();
        assert_eq!(
            std::fs::read_to_string(path.with_extension("json.bak.1")).unwrap(),
            "after"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unchanged_content_skips_write_and_backup() {
        let dir = scratch_dir("unchanged");
        let path = dir.join("settings.json");
        std::fs::write(&path, "same").unwrap();
        write_with_backup(&path, "same").unwrap();
        assert!(!path.with_extension("json.bak").exists());
        let changed = update_with_backup(&path, |_| Ok::<_, std::io::Error>(None)).unwrap();
        assert!(!changed);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn backups_are_rotated() {
        let dir = scratch_dir("rotate");
        let path = dir.join("cfg.json");
        std::fs::write(&path, "v0").unwrap();
        for i in 1..=8 {
            write_with_backup(&path, format!("v{i}")).unwrap();
        }
        let backups = existing_backups(&path);
        assert_eq!(backups.len(), MAX_BACKUPS);
        // Newest backup holds the previous content, oldest kept is v3.
        let (_, newest) = backups.last().unwrap();
        assert_eq!(std::fs::read_to_string(newest).unwrap(), "v7");
        let (_, oldest) = backups.first().unwrap();
        assert_eq!(std::fs::read_to_string(oldest).unwrap(), "v3");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "v8");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn update_sees_current_content_and_creates_missing_file() {
        let dir = scratch_dir("update");
        let path = dir.join("nested").join("new.json");
        let wrote = update_with_backup(&path, |cur| {
            assert!(cur.is_none());
            Ok::<_, std::io::Error>(Some(b"{}".to_vec()))
        })
        .unwrap();
        assert!(wrote);
        update_with_backup(&path, |cur| {
            assert_eq!(cur, Some("{}"));
            Ok::<_, std::io::Error>(Some(b"{\"a\":1}".to_vec()))
        })
        .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":1}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn backup_index_parsing() {
        let p = Path::new("/x/settings.json");
        assert_eq!(backup_index(p, Path::new("/x/settings.json.bak")), Some(0));
        assert_eq!(backup_index(p, Path::new("/x/settings.json.bak.12")), Some(12));
        assert_eq!(backup_index(p, Path::new("/x/settings.json")), None);
        assert_eq!(backup_index(p, Path::new("/x/settings.json.bakx")), None);
        assert_eq!(backup_index(p, Path::new("/x/other.json.bak")), None);
    }
}
