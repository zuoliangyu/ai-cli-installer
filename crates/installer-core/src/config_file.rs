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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

// ponytail: one global lock is enough for tiny config writes; use per-file locks if throughput matters.
static CONFIG_WRITE_LOCK: Mutex<()> = Mutex::new(());
static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
    let target = resolve_symlink(path)?;
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

fn resolve_symlink(path: &Path) -> std::io::Result<PathBuf> {
    let mut target = path.to_path_buf();
    for _ in 0..40 {
        match std::fs::symlink_metadata(&target) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let link = std::fs::read_link(&target)?;
                target = if link.is_absolute() {
                    link
                } else {
                    target.parent().unwrap_or_else(|| Path::new(".")).join(link)
                };
            }
            Ok(_) => return Ok(target),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(target),
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!("too many config symlinks: {}", path.display()),
    ))
}

/// Write `content` to a temp file next to `path`, fsync, then rename over
/// `path`. Permissions of an existing `path` are carried over (e.g. a 0600
/// `~/.claude.json` must not become world-readable).
pub(crate) fn atomic_write(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let permissions = match std::fs::metadata(path) {
        Ok(meta) => Some(meta.permissions()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let (mut file, tmp) = create_temp_file(path)?;
    let result = (|| {
        file.write_all(content)?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)?;
        }
        file.sync_all()
    })();
    drop(file);
    let result = result.and_then(|_| replace_file(&tmp, path));
    if result.is_err() {
        #[cfg(windows)]
        if let Ok(meta) = std::fs::metadata(&tmp) {
            let mut permissions = meta.permissions();
            if permissions.readonly() {
                // This branch is Windows-only: remove the temporary file's readonly attribute.
                #[allow(clippy::permissions_set_readonly_false)]
                permissions.set_readonly(false);
                let _ = std::fs::set_permissions(&tmp, permissions);
            }
        }
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn create_temp_file(path: &Path) -> std::io::Result<(std::fs::File, PathBuf)> {
    let file_name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other(format!("invalid path: {}", path.display())))?;
    for _ in 0..32 {
        let mut tmp_name = std::ffi::OsString::from(".");
        tmp_name.push(file_name);
        tmp_name.push(format!(
            ".tmp-{}-{}",
            std::process::id(),
            TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let tmp = path.with_file_name(tmp_name);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&tmp) {
            Ok(file) => return Ok((file, tmp)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not create a unique config temp file",
    ))
}

fn replace_file(tmp: &Path, path: &Path) -> std::io::Result<()> {
    for attempt in 0..5 {
        match std::fs::rename(tmp, path) {
            Ok(()) => return Ok(()),
            Err(e) if cfg!(windows) && matches!(e.raw_os_error(), Some(32 | 33)) && attempt < 4 => {
                std::thread::sleep(std::time::Duration::from_millis(25 * (attempt + 1)));
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
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
        let dir =
            std::env::temp_dir().join(format!("ai-cli-installer-{}-{}", tag, std::process::id()));
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
        assert_eq!(
            backup_index(p, Path::new("/x/settings.json.bak.12")),
            Some(12)
        );
        assert_eq!(backup_index(p, Path::new("/x/settings.json")), None);
        assert_eq!(backup_index(p, Path::new("/x/settings.json.bakx")), None);
        assert_eq!(backup_index(p, Path::new("/x/other.json.bak")), None);
    }

    #[test]
    fn concurrent_atomic_writes_keep_complete_payloads() {
        let dir = scratch_dir("concurrent-atomic");
        let path = dir.join("settings.json");
        std::fs::write(&path, "before").unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|i| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let content = vec![b'a' + i; 8192];
                    barrier.wait();
                    atomic_write(&path, &content).unwrap();
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        let content = std::fs::read(&path).unwrap();
        assert_eq!(content.len(), 8192);
        assert!(content.iter().all(|b| *b == content[0]));
        assert!((b'a'..=b'h').contains(&content[0]));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn sharing_violation_does_not_truncate_existing_config() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = scratch_dir("sharing-violation");
        let path = dir.join("settings.json");
        std::fs::write(&path, "before").unwrap();
        // Allow reads and writes, but deny deletion/rename like an editor.
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0x1 | 0x2)
            .open(&path)
            .unwrap();
        atomic_write(&path, b"after").expect_err("a locked config must not be replaced in place");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "before");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        drop(held);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn temp_is_private_before_writing_and_existing_mode_is_preserved() {
        use std::os::unix::fs::PermissionsExt;

        let dir = scratch_dir("private-config");
        let path = dir.join("settings.json");
        let (file, tmp) = create_temp_file(&path).unwrap();
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o077, 0);
        drop(file);
        std::fs::remove_file(tmp).unwrap();

        atomic_write(&path, b"new secret").unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o077,
            0
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        atomic_write(&path, b"updated secret").unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn dangling_relative_symlink_chain_keeps_links_and_creates_target() {
        use std::os::unix::fs::symlink;

        let dir = scratch_dir("dangling-config-link");
        let path = dir.join("settings.json");
        let intermediate = dir.join("dotfiles-link");
        let target = dir.join("dotfiles/settings.json");
        symlink("dotfiles-link", &path).unwrap();
        symlink("dotfiles/settings.json", &intermediate).unwrap();

        write_with_backup(&path, "first").unwrap();
        write_with_backup(&path, "second").unwrap();

        assert_eq!(
            std::fs::read_link(&path).unwrap(),
            Path::new("dotfiles-link")
        );
        assert_eq!(
            std::fs::read_link(&intermediate).unwrap(),
            Path::new("dotfiles/settings.json")
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "second");
        assert_eq!(
            std::fs::read_to_string(target.with_extension("json.bak")).unwrap(),
            "first"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_cycle_returns_error_without_replacing_link() {
        use std::os::unix::fs::symlink;

        let dir = scratch_dir("cyclic-config-link");
        let path = dir.join("settings.json");
        symlink("settings.json", &path).unwrap();
        assert!(write_with_backup(&path, "after").is_err());
        assert_eq!(
            std::fs::read_link(&path).unwrap(),
            Path::new("settings.json")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
