use std::path::{Path, PathBuf};
use std::sync::Mutex;

// ponytail: one global lock is enough for tiny config writes; use per-file locks if throughput matters.
static CONFIG_WRITE_LOCK: Mutex<()> = Mutex::new(());

pub fn write_with_backup(path: &Path, content: impl AsRef<[u8]>) -> std::io::Result<()> {
    let _guard = CONFIG_WRITE_LOCK
        .lock()
        .map_err(|_| std::io::Error::other("config write lock poisoned"))?;
    if path.exists() {
        std::fs::copy(path, next_backup_path(path))?;
    }
    std::fs::write(path, content)
}

fn next_backup_path(path: &Path) -> PathBuf {
    let mut backup = path.as_os_str().to_os_string();
    backup.push(".bak");
    let first = PathBuf::from(&backup);
    if !first.exists() {
        return first;
    }
    // ponytail: linear scan is fine for human-scale backups; add rotation if these reach hundreds.
    for index in 1.. {
        let mut candidate = backup.clone();
        candidate.push(format!(".{index}"));
        let candidate = PathBuf::from(candidate);
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_the_previous_file() {
        let dir = std::env::temp_dir().join(format!("ai-cli-installer-{}", std::process::id()));
        let path = dir.join("settings.json");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
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
}
