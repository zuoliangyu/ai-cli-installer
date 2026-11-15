//! Process-spawning helpers that work consistently on Windows.
//!
//! Rust's `Command::new("npm")` is supposed to fall back to `.cmd`/`.bat` when
//! resolving an executable through `PATH`, but in practice (Tauri-bundled apps
//! launched from Explorer, nvm-shimmed `npm`) it often fails. We always go
//! through `cmd.exe /c` on Windows to make node-ecosystem shims (`npm.cmd`,
//! `pnpm.cmd`, `codex.cmd`, ...) reliably runnable.
//!
//! All Windows spawns set `CREATE_NO_WINDOW (0x08000000)` so console children
//! (`cmd.exe`, `where.exe`, `npm.cmd` shim, ...) don't flash a black box from a
//! GUI-subsystem Tauri host.
//!
//! Every child we wait on goes through [`output_with_timeout`]: a hung
//! `npm install` / `claude install` / rc-file-heavy `--version` used to block
//! the global tool-operation lock forever. A job/process-group guard also
//! terminates descendants when a wait times out or is cancelled.

use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::Duration;
use tokio::process::Command;

/// Budget for quick probes: `--version`, `where`, `npm prefix -g`, ...
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// Budget for real work: `npm install -g`, `claude install`, UAC prompts.
pub const INSTALL_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// CreationFlags bit that suppresses the per-child console window.
/// <https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags>
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Apply Windows-only flags (suppress console window). No-op on Unix.
#[cfg(windows)]
fn silence(cmd: &mut Command) {
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn silence(_cmd: &mut Command) {}

/// Public re-export: hide the spawned console window on Windows. Use for
/// `tokio::process::Command` instances built outside this module.
pub fn silence_windows(cmd: &mut Command) {
    silence(cmd);
}

/// Same as [`silence_windows`] but for `std::process::Command`.
pub fn silence_windows_std(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

/// Allocate a `Command` that won't flash a console window on Windows.
fn silent_command(program: &str) -> Command {
    let mut c = Command::new(program);
    silence(&mut c);
    c
}

fn silent_command_path(program: &Path) -> Command {
    let mut c = Command::new(program);
    silence(&mut c);
    c
}

/// Build a `Command` that spawns `program`. On Windows, wraps in `cmd /c` so
/// that `.cmd`/`.bat` shims resolve correctly even when launched from a
/// non-shell parent process. The console window is suppressed.
pub fn shell_command(program: &str) -> Command {
    if cfg!(windows) {
        let mut c = silent_command("cmd");
        c.arg("/c").arg(program);
        c
    } else {
        Command::new(program)
    }
}

/// Run `cmd` to completion, capturing stdout/stderr, bounded by `timeout`.
///
/// - stdin is closed so a child waiting for input (npm prompts, a shell rc
///   doing `read`) fails fast instead of hanging;
/// - a Windows Job Object / Unix process group owns the command and its
///   descendants, including when the caller drops this future;
/// - on timeout the tree is terminated before waiting for the child to exit.
///
/// A timeout is reported as `io::ErrorKind::TimedOut` with a Chinese
/// message, so callers' existing `map_err(|e| format!(..., e))` stays useful.
pub async fn output_with_timeout(cmd: &mut Command, timeout: Duration) -> std::io::Result<Output> {
    cmd.kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let (child, tree) = process_tree::spawn(cmd)?;
    let output = child.wait_with_output();
    tokio::pin!(output);
    match tokio::time::timeout(timeout, &mut output).await {
        Ok(res) => res,
        Err(_) => {
            tree.terminate();
            let _ = tokio::time::timeout(Duration::from_secs(5), &mut output).await;
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("命令执行超时（超过 {} 秒），已强制结束", timeout.as_secs()),
            ))
        }
    }
}

#[cfg(windows)]
mod process_tree {
    use std::io;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use tokio::process::{Child, Command};
    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{
        OpenThread, ResumeThread, CREATE_NO_WINDOW, CREATE_SUSPENDED, THREAD_SUSPEND_RESUME,
    };

    pub(super) struct Guard(OwnedHandle);

    impl Guard {
        fn new() -> io::Result<Self> {
            // Null security attributes keep the kill-on-close handle out of children.
            let job = owned(unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) })?;
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = unsafe {
                SetInformationJobObject(
                    job.as_raw_handle(),
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of_val(&limits) as u32,
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self(job))
        }

        pub(super) fn terminate(&self) {
            if unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) } == 0 {
                tracing::warn!("terminate process job: {}", io::Error::last_os_error());
            }
        }
    }

    pub(super) fn spawn(cmd: &mut Command) -> io::Result<(Child, Guard)> {
        let tree = Guard::new()?;
        // No child code may run before job assignment, otherwise descendants can escape.
        cmd.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
        let child = cmd.spawn()?;
        let process = child
            .raw_handle()
            .ok_or_else(|| io::Error::other("missing child handle"))?;
        if unsafe { AssignProcessToJobObject(tree.0.as_raw_handle(), process) } == 0 {
            return Err(io::Error::last_os_error());
        }
        resume_primary_thread(
            child
                .id()
                .ok_or_else(|| io::Error::other("missing child pid"))?,
        )?;
        Ok((child, tree))
    }

    fn owned(handle: HANDLE) -> io::Result<OwnedHandle> {
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            // Every handle passed here was just created and has exactly this owner.
            Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
        }
    }

    fn resume_primary_thread(pid: u32) -> io::Result<()> {
        // std/Tokio retain the process handle, but close CreateProcess's thread handle.
        // A CREATE_SUSPENDED process has not run code that could create other threads.
        let snapshot = owned(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) })?;
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        let mut available = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) };
        while available != 0 {
            if entry.th32OwnerProcessID == pid {
                let thread =
                    owned(unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) })?;
                if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
                    return Err(io::Error::last_os_error());
                }
                return Ok(());
            }
            entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
            available = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) };
        }
        Err(io::Error::other("suspended child has no primary thread"))
    }
}

#[cfg(unix)]
mod process_tree {
    use std::io;
    use tokio::process::{Child, Command};

    pub(super) struct Guard(libc::pid_t);

    impl Guard {
        pub(super) fn terminate(&self) {
            // spawn established a fresh group whose ID is this child's PID.
            if unsafe { libc::kill(-self.0, libc::SIGKILL) } != 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) {
                    tracing::warn!("terminate process group: {error}");
                }
            }
        }
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            self.terminate();
        }
    }

    pub(super) fn spawn(cmd: &mut Command) -> io::Result<(Child, Guard)> {
        cmd.process_group(0);
        let child = cmd.spawn()?;
        let pid = child
            .id()
            .ok_or_else(|| io::Error::other("missing child pid"))?;
        Ok((child, Guard(pid as libc::pid_t)))
    }
}

/// Run an executable on disk and return stdout. On Windows, `.cmd`/`.bat`
/// shims are routed through `cmd /c` and `.ps1` shims through
/// `powershell -File` (going through `cmd /c` would hit the file association
/// and open the script in Notepad). No console window appears. Bounded by
/// [`PROBE_TIMEOUT`].
pub async fn run_executable(path: &Path, args: &[&str]) -> Option<String> {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    let mut cmd = if cfg!(windows) && (lower.ends_with(".cmd") || lower.ends_with(".bat")) {
        let mut c = silent_command("cmd");
        c.arg("/c").arg(path);
        c
    } else if cfg!(windows) && lower.ends_with(".ps1") {
        let mut c = silent_command("powershell");
        c.args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(path);
        c
    } else {
        silent_command_path(path)
    };
    cmd.args(args);

    let output = output_with_timeout(&mut cmd, PROBE_TIMEOUT).await.ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Resolve a bare command name to an absolute path via `where` (Windows) or
/// `command -v` (Unix). Returns the first match.
pub async fn resolve_command_path(command_name: &str) -> Option<PathBuf> {
    let mut cmd = if cfg!(windows) {
        let mut c = silent_command("where");
        c.arg(command_name);
        c
    } else {
        // Pass the name as a positional parameter instead of splicing it
        // into the script, so it's never interpreted by the shell.
        let mut c = Command::new("sh");
        c.args(["-c", "command -v \"$1\"", "sh", command_name]);
        c
    };
    let output = output_with_timeout(&mut cmd, PROBE_TIMEOUT).await.ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(PathBuf::from)
}

/// Resolve `command_name` then run `<resolved> --version` (or any args). On
/// Windows this dodges Rust's flaky `.cmd` PATH-extension lookup.
pub async fn run_version_by_name(command_name: &str, args: &[&str]) -> Option<String> {
    let path = resolve_command_path(command_name).await?;
    run_executable(&path, args).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{SocketAddr, TcpListener};

    const FIXTURE_ROLE: &str = "AI_CLI_INSTALLER_TEST_PROCESS_ROLE";
    const FIXTURE_DIR: &str = "AI_CLI_INSTALLER_TEST_PROCESS_DIR";

    #[test]
    #[ignore = "subprocess fixture, launched only by the process-tree tests"]
    fn process_tree_fixture() {
        let role = std::env::var(FIXTURE_ROLE).expect("fixture role");
        let dir = PathBuf::from(std::env::var_os(FIXTURE_DIR).expect("fixture dir"));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        std::fs::write(dir.join(&role), listener.local_addr().unwrap().to_string()).unwrap();
        if role == "parent" {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "proc::tests::process_tree_fixture",
                    "--ignored",
                    "--nocapture",
                ])
                .env(FIXTURE_ROLE, "grandchild")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            silence_windows_std(&mut command);
            command.spawn().unwrap().wait().unwrap();
        } else {
            // Safety bound in case a broken implementation fails to terminate this fixture.
            std::thread::sleep(Duration::from_secs(30));
        }
        drop(listener);
    }

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(tag: &str) -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "installer-process-{tag}-{}-{nonce}",
                std::process::id()
            ));
            std::fs::create_dir(&dir).unwrap();
            Self(dir)
        }

        fn command(&self) -> Command {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "proc::tests::process_tree_fixture",
                    "--ignored",
                    "--nocapture",
                ])
                .env(FIXTURE_ROLE, "parent")
                .env(FIXTURE_DIR, &self.0);
            command
        }

        async fn ready(&self) -> [SocketAddr; 2] {
            tokio::time::timeout(Duration::from_secs(4), async {
                loop {
                    let parent = std::fs::read_to_string(self.0.join("parent"))
                        .ok()
                        .and_then(|s| s.parse::<SocketAddr>().ok());
                    let grandchild = std::fs::read_to_string(self.0.join("grandchild"))
                        .ok()
                        .and_then(|s| s.parse::<SocketAddr>().ok());
                    if let (Some(parent), Some(grandchild)) = (parent, grandchild) {
                        assert!(TcpListener::bind(parent).is_err(), "parent already exited");
                        assert!(
                            TcpListener::bind(grandchild).is_err(),
                            "grandchild already exited"
                        );
                        return [parent, grandchild];
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("fixture process tree did not start")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(self.0.join("parent"));
            let _ = std::fs::remove_file(self.0.join("grandchild"));
            let _ = std::fs::remove_dir(&self.0);
        }
    }

    async fn assert_tree_stopped(addresses: [SocketAddr; 2]) {
        tokio::time::timeout(Duration::from_secs(3), async {
            for address in addresses {
                while TcpListener::bind(address).is_err() {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
        })
        .await
        .expect("a fixture process still owns its socket after termination");
    }

    #[tokio::test]
    async fn timeout_terminates_entire_process_tree() {
        let fixture = Fixture::new("timeout");
        let mut command = fixture.command();
        let task =
            tokio::spawn(
                async move { output_with_timeout(&mut command, Duration::from_secs(5)).await },
            );
        let addresses = fixture.ready().await;
        let error = task.await.unwrap().unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert_tree_stopped(addresses).await;
    }

    #[tokio::test]
    async fn cancellation_terminates_entire_process_tree() {
        let fixture = Fixture::new("cancel");
        let mut command = fixture.command();
        let task = tokio::spawn(async move {
            output_with_timeout(&mut command, Duration::from_secs(30)).await
        });
        let addresses = fixture.ready().await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_tree_stopped(addresses).await;
    }

    #[tokio::test]
    async fn successful_command_preserves_output() {
        let mut command = if cfg!(windows) {
            let mut command = shell_command("echo");
            command.arg("process-tree-ok");
            command
        } else {
            let mut command = Command::new("printf");
            command.arg("process-tree-ok");
            command
        };
        let output = output_with_timeout(&mut command, Duration::from_secs(5))
            .await
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "process-tree-ok"
        );
    }
}
