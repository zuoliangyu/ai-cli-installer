//! Same-user coordination between desktop and Web installer processes.

use std::fs::{File, TryLockError};
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::error::{AppError, Result};

pub(crate) const WAIT_TIMEOUT: Duration = Duration::from_secs(30);
const RETRY_INTERVAL: Duration = Duration::from_millis(100);

pub(crate) struct OperationLock {
    path: Option<PathBuf>,
}

impl OperationLock {
    pub(crate) fn for_user() -> Self {
        Self {
            path: dirs::home_dir()
                .map(|home| home.join(".ai-cli-installer").join("operations.lock")),
        }
    }

    #[cfg(test)]
    pub(crate) fn at(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    pub(crate) fn try_acquire(&self) -> Result<Option<OperationGuard>> {
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| AppError::Other("无法确定用户目录，不能取得安装锁".into()))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Never unlink this file: replacing its inode would let two processes hold different locks.
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(OperationGuard {
                _file: Arc::new(file),
            })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(AppError::Other(format!(
                "无法取得安装锁 {}：{error}",
                path.display()
            ))),
        }
    }

    pub(crate) async fn acquire(&self, budget: Duration) -> Result<OperationGuard> {
        let deadline = tokio::time::Instant::now() + budget;
        loop {
            if let Some(guard) = self.try_acquire()? {
                return Ok(guard);
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(busy_error());
            }
            tokio::time::sleep(RETRY_INTERVAL.min(remaining)).await;
        }
    }
}

pub(crate) fn busy_error() -> AppError {
    AppError::Other(
        "已有 AI CLI Installer 窗口或 Web 服务正在安装或诊断，等待超过 30 秒，请稍后重试".into(),
    )
}

#[derive(Clone)]
pub(crate) struct OperationGuard {
    // The OS releases the lock when the last owner closes the handle, including process exit.
    _file: Arc<File>,
}

tokio::task_local! {
    static CURRENT_OPERATION: OperationGuard;
}

impl OperationGuard {
    pub(crate) async fn scope<F: Future>(self, future: F) -> F::Output {
        CURRENT_OPERATION.scope(self, future).await
    }
}

/// Blocking deployments outlive an aborted async waiter and must retain its lock.
pub(crate) fn current_guard() -> Option<OperationGuard> {
    CURRENT_OPERATION.try_with(Clone::clone).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::Path;
    use std::process::{Child, Command, Stdio};

    const FIXTURE_PATH: &str = "AI_CLI_INSTALLER_TEST_LOCK_PATH";
    const FIXTURE_ROLE: &str = "AI_CLI_INSTALLER_TEST_LOCK_ROLE";

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(tag: &str) -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "installer-operation-{tag}-{}-{nonce}",
                std::process::id()
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn lock(&self) -> OperationLock {
            OperationLock::at(self.0.join("operations.lock"))
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct ChildGuard(Child);

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn fixture_command(path: &Path, role: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "operation_lock::tests::lock_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env(FIXTURE_PATH, path)
            .env(FIXTURE_ROLE, role)
            .stdin(Stdio::null());
        crate::proc::silence_windows_std(&mut command);
        command
    }

    #[test]
    #[ignore = "independent-process fixture, launched by operation lock tests"]
    fn lock_fixture() {
        let path = PathBuf::from(std::env::var_os(FIXTURE_PATH).expect("fixture lock path"));
        let lock = OperationLock::at(path.clone());
        if std::env::var(FIXTURE_ROLE).unwrap() == "hold" {
            let _guard = lock
                .try_acquire()
                .unwrap()
                .expect("fixture should acquire lock");
            std::fs::write(path.with_extension("ready"), b"ready").unwrap();
            std::thread::sleep(Duration::from_secs(30));
        } else {
            println!(
                "{}",
                if lock.try_acquire().unwrap().is_some() {
                    "LOCK_AVAILABLE"
                } else {
                    "LOCK_BUSY"
                }
            );
        }
    }

    #[test]
    fn operation_lock_excludes_independent_processes_and_keeps_file() {
        let dir = TestDir::new("contention");
        let lock = dir.lock();
        let guard = lock.try_acquire().unwrap().unwrap();
        let path = dir.0.join("operations.lock");
        let output = fixture_command(&path, "try").output().unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("LOCK_BUSY"));
        drop(guard);
        assert!(path.exists(), "lock file must never be deleted on release");
        let output = fixture_command(&path, "try").output().unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("LOCK_AVAILABLE"));
    }

    #[tokio::test]
    async fn operation_lock_is_released_when_owner_process_exits() {
        let dir = TestDir::new("exit");
        let path = dir.0.join("operations.lock");
        let mut child = ChildGuard(
            fixture_command(&path, "hold")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        tokio::time::timeout(Duration::from_secs(4), async {
            while !path.with_extension("ready").exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("lock holder failed to start");
        let lock = dir.lock();
        assert!(lock.try_acquire().unwrap().is_none());
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        let _guard = lock.acquire(Duration::from_secs(2)).await.unwrap();
        assert!(path.exists());
    }

    #[tokio::test]
    async fn operation_lock_wait_is_bounded_and_cancellable() {
        let dir = TestDir::new("wait");
        let lock = dir.lock();
        let guard = lock.try_acquire().unwrap().unwrap();
        let error = lock
            .acquire(Duration::from_millis(50))
            .await
            .err()
            .expect("busy lock should time out");
        assert!(error.to_string().contains("正在安装或诊断"));
        let waiter = dir.lock();
        let task = tokio::spawn(async move { waiter.acquire(Duration::from_secs(30)).await });
        tokio::task::yield_now().await;
        task.abort();
        assert!(task.await.err().unwrap().is_cancelled());
        drop(guard);
        assert!(lock.try_acquire().unwrap().is_some());
    }

    #[tokio::test]
    async fn operation_lock_outlives_cancelled_blocking_deployment_wait() {
        let dir = TestDir::new("blocking");
        let lock = dir.lock();
        let guard = lock.try_acquire().unwrap().unwrap();
        let dest = dir.0.join("installed.bin");
        let output_path = dest.clone();
        let (ready, started) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let task = tokio::spawn(guard.scope(async move {
            crate::installer::install_executable(dest, move |out| {
                ready.send(()).unwrap();
                wait.recv_timeout(Duration::from_secs(3)).unwrap();
                out.write_all(b"installed")
            })
            .await
        }));
        started.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(
            lock.try_acquire().unwrap().is_none(),
            "blocking deployment released the lock too early"
        );
        release.send(()).unwrap();
        let _guard = lock.acquire(Duration::from_secs(3)).await.unwrap();
        assert_eq!(std::fs::read(output_path).unwrap(), b"installed");
    }
}
