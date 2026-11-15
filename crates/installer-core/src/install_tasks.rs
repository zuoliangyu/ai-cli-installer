//! Process-local install jobs, independent of the requesting window or connection.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::progress::{DownloadProgress, ProgressCallback};
use crate::tools::{InstallMethod, InstallReport};
use crate::Result;

const HISTORY_LIMIT: usize = 20;
static SESSION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Serialize)]
pub struct InstallRequest {
    pub tool_id: String,
    pub channel: String,
    pub method: InstallMethod,
    pub mirror: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
}

impl TaskStatus {
    fn active(self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct InstallTask {
    pub session_id: String,
    pub task_id: String,
    #[serde(flatten)]
    pub request: InstallRequest,
    pub status: TaskStatus,
    pub progress: Option<DownloadProgress>,
    pub report: Option<InstallReport>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TaskSnapshot {
    pub session_id: String,
    pub tasks: Vec<InstallTask>,
}

type JobFuture = Pin<Box<dyn Future<Output = Result<InstallReport>> + Send>>;
type JobRunner = Box<dyn FnOnce(ProgressCallback) -> JobFuture + Send>;

#[derive(Default)]
struct Registry {
    sequence: u64,
    tasks: VecDeque<InstallTask>,
    pending: VecDeque<(String, JobRunner)>,
    worker_running: bool,
}

pub struct InstallTasks {
    session_id: String,
    registry: Mutex<Registry>,
}

impl Default for InstallTasks {
    fn default() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Self {
            session_id: format!(
                "{:x}-{nanos:x}-{:x}",
                std::process::id(),
                SESSION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ),
            registry: Mutex::new(Registry::default()),
        }
    }
}

impl InstallTasks {
    fn lock(&self) -> MutexGuard<'_, Registry> {
        self.registry.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn snapshot(&self) -> TaskSnapshot {
        TaskSnapshot {
            session_id: self.session_id.clone(),
            tasks: self.lock().tasks.iter().cloned().collect(),
        }
    }

    pub(crate) fn start<F, Fut>(self: &Arc<Self>, request: InstallRequest, run: F) -> InstallTask
    where
        F: FnOnce(ProgressCallback) -> Fut + Send + 'static,
        Fut: Future<Output = Result<InstallReport>> + Send + 'static,
    {
        let mut registry = self.lock();
        if let Some(existing) = registry
            .tasks
            .iter()
            .find(|t| t.request.tool_id == request.tool_id && t.status.active())
        {
            return existing.clone();
        }
        registry.sequence += 1;
        let task = InstallTask {
            session_id: self.session_id.clone(),
            task_id: format!("{}-{}", self.session_id, registry.sequence),
            request,
            status: TaskStatus::Queued,
            progress: None,
            report: None,
            error: None,
        };
        registry.tasks.push_back(task.clone());
        registry.pending.push_back((
            task.task_id.clone(),
            Box::new(move |progress| Box::pin(run(progress))),
        ));
        if !registry.worker_running {
            registry.worker_running = true;
            let manager = self.clone();
            // Dropping the response never aborts this worker or its queued jobs.
            tokio::spawn(async move { manager.run_queue().await });
        }
        task
    }

    async fn run_queue(self: Arc<Self>) {
        loop {
            let (task_id, run) = {
                let mut registry = self.lock();
                let Some((id, run)) = registry.pending.pop_front() else {
                    registry.worker_running = false;
                    return;
                };
                if let Some(task) = registry.tasks.iter_mut().find(|t| t.task_id == id) {
                    task.status = TaskStatus::Running;
                }
                (id, run)
            };
            let manager = self.clone();
            let progress_id = task_id.clone();
            let progress: ProgressCallback = Arc::new(move |progress| {
                let mut registry = manager.lock();
                if let Some(task) = registry.tasks.iter_mut().find(|t| t.task_id == progress_id) {
                    if task.status == TaskStatus::Running
                        && task.request.tool_id == progress.tool_id
                    {
                        task.progress = Some(progress);
                    }
                }
            });
            // A panicking installer must fail its own job and leave the queue usable.
            let outcome = tokio::spawn(async move { run(progress).await }).await;
            let mut registry = self.lock();
            if let Some(task) = registry.tasks.iter_mut().find(|t| t.task_id == task_id) {
                task.progress = None;
                match outcome {
                    Ok(Ok(report)) => {
                        task.status = TaskStatus::Succeeded;
                        task.report = Some(report);
                    }
                    result => {
                        task.status = TaskStatus::Failed;
                        task.error = Some(match result {
                            Ok(Err(error)) => error.to_string(),
                            Err(error) => format!("后台安装任务异常终止：{error}"),
                            Ok(Ok(_)) => unreachable!(),
                        });
                    }
                }
            }
            while registry.tasks.iter().filter(|t| !t.status.active()).count() > HISTORY_LIMIT {
                if let Some(index) = registry.tasks.iter().position(|t| !t.status.active()) {
                    registry.tasks.remove(index);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppError;
    use tokio::sync::oneshot;
    use tokio::time::{timeout, Duration};

    fn request(tool: &str) -> InstallRequest {
        InstallRequest {
            tool_id: tool.into(),
            channel: "latest".into(),
            method: InstallMethod::Native,
            mirror: None,
        }
    }

    fn report(tool: &str) -> InstallReport {
        InstallReport {
            tool_id: tool.into(),
            version: "1.0.0".into(),
            install_path: "/fake/bin".into(),
            elapsed_secs: 0,
            method: InstallMethod::Native,
            auto_applied_fixes: vec![],
        }
    }

    async fn wait_finished(manager: &InstallTasks, id: &str) -> InstallTask {
        timeout(Duration::from_secs(2), async {
            loop {
                if let Some(task) = manager
                    .snapshot()
                    .tasks
                    .into_iter()
                    .find(|t| t.task_id == id)
                {
                    if !task.status.active() {
                        return task;
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("job should complete")
    }

    #[tokio::test]
    async fn queues_deduplicates_and_keeps_progress_on_its_task() {
        let manager = Arc::new(InstallTasks::default());
        let (release, blocked) = oneshot::channel();
        let (started, running) = oneshot::channel();
        let first = manager.start(request("claude"), move |progress| async move {
            progress(DownloadProgress {
                tool_id: "claude".into(),
                downloaded: 42,
                total: Some(100),
                mirror: "test".into(),
            });
            started.send(()).unwrap();
            blocked.await.unwrap();
            Ok(report("claude"))
        });
        running.await.unwrap();
        let duplicate = manager.start(request("claude"), |_| async {
            panic!("duplicate must not execute")
        });
        assert_eq!(first.task_id, duplicate.task_id);
        let second = manager.start(request("codex"), |_| async { Ok(report("codex")) });
        let snapshot = manager.snapshot();
        assert_eq!(snapshot.tasks[0].status, TaskStatus::Running);
        assert_eq!(snapshot.tasks[0].progress.as_ref().unwrap().downloaded, 42);
        assert_eq!(snapshot.tasks[1].status, TaskStatus::Queued);
        assert!(snapshot.tasks[1].progress.is_none());
        release.send(()).unwrap();
        assert_eq!(
            wait_finished(&manager, &second.task_id).await.status,
            TaskStatus::Succeeded
        );
    }

    #[tokio::test]
    async fn dropped_caller_and_failed_job_do_not_stop_the_queue() {
        let manager = Arc::new(InstallTasks::default());
        let (submitted, accepted) = oneshot::channel();
        let caller_manager = manager.clone();
        let caller = tokio::spawn(async move {
            let task = caller_manager.start(request("claude"), |_| async {
                Err(AppError::Install("fake failure".into()))
            });
            submitted.send(task.task_id).unwrap();
            std::future::pending::<()>().await;
        });
        let id = accepted.await.unwrap();
        caller.abort();
        let failed = wait_finished(&manager, &id).await;
        assert_eq!(failed.status, TaskStatus::Failed);
        assert!(failed.error.unwrap().contains("fake failure"));
        let next = manager.start(request("claude"), |_| async { Ok(report("claude")) });
        assert_ne!(next.task_id, id);
        assert_eq!(
            wait_finished(&manager, &next.task_id).await.status,
            TaskStatus::Succeeded
        );
    }

    #[tokio::test]
    async fn panic_is_reported_and_completed_history_is_bounded() {
        let manager = Arc::new(InstallTasks::default());
        let failed = manager.start(request("claude"), |_| async { panic!("fake panic") });
        assert_eq!(
            wait_finished(&manager, &failed.task_id).await.status,
            TaskStatus::Failed
        );
        for _ in 0..HISTORY_LIMIT + 3 {
            let task = manager.start(request("claude"), |_| async { Ok(report("claude")) });
            wait_finished(&manager, &task.task_id).await;
        }
        let snapshot = manager.snapshot();
        assert_eq!(snapshot.tasks.len(), HISTORY_LIMIT);
        assert!(!snapshot.tasks.iter().any(|t| t.task_id == failed.task_id));
        assert_ne!(
            snapshot.session_id,
            InstallTasks::default().snapshot().session_id
        );
    }
}
