import { get } from "svelte/store";
import {
  AUTO_MIRROR,
  getInstallState,
  installStates,
  installTaskSync,
  patchInstallState,
  type InstallChannel,
} from "./stores";
import type { ApiTransport, InstallTask, InstallTaskSnapshot, InstallTaskStatus } from "./types";

const active = (status: InstallTaskStatus | null): boolean =>
  status === "queued" || status === "running";

const errorText = (error: unknown): string =>
  error instanceof Error ? error.message : String(error);

function successMessage(task: InstallTask): string {
  const report = task.report;
  if (!report) return "安装任务已完成，请检查安装结果。";
  const via = report.method === "npm" ? "npm" : task.mirror ? `镜像 ${task.mirror}` : "镜像";
  let message = `已通过${via}安装 ${report.version} (${report.elapsed_secs}s)`;
  if (report.auto_applied_fixes.length > 0) {
    message += `\n顺便应用了配置修复：${report.auto_applied_fixes.join("、")}（已写入 Claude 配置，可直接 \`claude login\`；如需撤销请到「配置修复」面板）。`;
  }
  return message;
}

/** Both transports read the same process-owned jobs; no unscoped progress listener is needed. */
export class InstallTaskMonitor {
  private sessionId: string | null = null;
  private known = new Map<string, InstallTaskStatus>();
  private latestTaskIds = new Map<string, string>();
  private submitting = new Set<string>();
  private unconfirmed = new Map<string, string | null>();
  private generation = 0;
  private inFlight: Promise<void> | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private started = false;

  constructor(
    private transport: () => Promise<ApiTransport>,
    private onCompleted: () => void,
  ) {}

  startPolling(): void {
    if (this.started) return;
    this.started = true;
    document.addEventListener("visibilitychange", this.onVisibility);
    void this.poll();
  }

  stopPolling(): void {
    this.started = false;
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
    document.removeEventListener("visibilitychange", this.onVisibility);
  }

  private onVisibility = (): void => {
    if (document.hidden || !this.started) return;
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
    void this.poll();
  };

  private async poll(): Promise<void> {
    await this.refresh();
    if (!this.started || this.timer !== null) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      void this.poll();
    }, document.hidden ? 5000 : 1000);
  }

  refresh(): Promise<void> {
    if (this.inFlight) return this.inFlight;
    const generation = this.generation;
    this.inFlight = (async () => {
      try {
        const snapshot = await (await this.transport()).listInstallTasks();
        // A response started before a submission cannot erase its acknowledgement.
        if (generation !== this.generation) return;
        this.applySnapshot(snapshot);
        installTaskSync.update((state) => ({ ...state, ready: true, error: null }));
      } catch (error) {
        if (generation !== this.generation) return;
        installTaskSync.update((state) => ({
          ...state,
          ready: false,
          error: `暂时无法确认后台安装状态：${errorText(error)}。恢复同步前已暂停新安装。`,
        }));
      } finally {
        this.inFlight = null;
      }
    })();
    return this.inFlight;
  }

  private acceptSession(sessionId: string): void {
    if (this.sessionId && this.sessionId !== sessionId) {
      installStates.update((states) => Object.fromEntries(
        Object.entries(states).map(([id, state]) => [id, {
          ...state,
          busy: false,
          taskId: null,
          taskStatus: null,
          progress: null,
          message: null,
          error: null,
          failedMirror: null,
        }]),
      ));
      this.known.clear();
      this.latestTaskIds.clear();
      this.unconfirmed.clear();
      installTaskSync.update((state) => ({
        ...state,
        notice: "后端已重启，旧安装任务记录已清除。请检查当前安装结果，再决定是否重新安装。",
      }));
      this.onCompleted();
    }
    this.sessionId = sessionId;
  }

  private applyTask(task: InstallTask): boolean {
    this.latestTaskIds.set(task.tool_id, task.task_id);
    const previous = this.known.get(task.task_id);
    this.known.set(task.task_id, task.status);
    if (!active(task.status) && previous === task.status) return false;
    const current = getInstallState(task.tool_id);
    patchInstallState(task.tool_id, {
      taskId: task.task_id,
      taskStatus: task.status,
      busy: active(task.status),
      channel: task.channel === "stable" ? "stable" : "latest",
      method: task.method,
      mirror: task.method === "native" ? task.mirror ?? AUTO_MIRROR : current.mirror,
      progress: active(task.status) ? task.progress : null,
      message: task.status === "succeeded" ? successMessage(task) : null,
      error: task.status === "failed" ? task.error ?? "安装任务失败" : null,
      failedMirror: task.status === "failed" ? task.mirror : null,
    });
    return !active(task.status);
  }

  private applySnapshot(snapshot: InstallTaskSnapshot): void {
    this.acceptSession(snapshot.session_id);
    const latest = new Map<string, InstallTask>();
    const ids = new Set(snapshot.tasks.map((task) => task.task_id));
    for (const task of snapshot.tasks) latest.set(task.tool_id, task);
    let completed = false;
    for (const [toolId, state] of Object.entries(get(installStates))) {
      if (this.submitting.has(toolId)) continue;
      const task = latest.get(toolId);
      if (this.unconfirmed.has(toolId)) {
        const baseline = this.unconfirmed.get(toolId);
        this.unconfirmed.delete(toolId);
        if (!task || (!active(task.status) && task.task_id === baseline)) {
          patchInstallState(toolId, {
            busy: false,
            taskId: null,
            taskStatus: null,
            progress: null,
            message: null,
            error: "未发现本次提交的新任务。请检查安装结果后重试。",
          });
          latest.delete(toolId);
          continue;
        }
      }
      if (state.taskId && active(state.taskStatus) && !ids.has(state.taskId) && !task) {
        patchInstallState(toolId, {
          busy: false,
          taskId: null,
          taskStatus: null,
          progress: null,
          error: "后台已无此安装任务，请检查安装结果后重试。",
        });
      }
    }
    for (const [toolId, task] of latest) {
      if (!this.submitting.has(toolId)) completed = this.applyTask(task) || completed;
    }
    for (const id of this.known.keys()) if (!ids.has(id)) this.known.delete(id);
    if (completed) this.onCompleted();
  }

  async install(toolId: string, channel: InstallChannel): Promise<void> {
    const current = getInstallState(toolId);
    if (current.busy || this.submitting.has(toolId) || !get(installTaskSync).ready) return;
    const mirror = current.method === "native" && current.mirror !== AUTO_MIRROR
      ? current.mirror : null;
    this.submitting.add(toolId);
    this.generation++;
    patchInstallState(toolId, {
      busy: true,
      taskId: null,
      taskStatus: null,
      channel,
      progress: null,
      message: "正在提交安装任务…",
      error: null,
      failedMirror: null,
    });
    try {
      const task = await (await this.transport()).startInstallTask(toolId, channel, current.method, mirror);
      this.generation++;
      this.acceptSession(task.session_id);
      if (this.applyTask(task)) this.onCompleted();
    } catch (error) {
      this.unconfirmed.set(toolId, this.latestTaskIds.get(toolId) ?? current.taskId);
      installTaskSync.update((state) => ({
        ...state,
        ready: false,
        error: `提交结果尚未确认：${errorText(error)}。正在重新读取后台任务，请勿重复安装。`,
      }));
      patchInstallState(toolId, { message: "提交结果尚未确认，正在同步后台任务…" });
    } finally {
      this.submitting.delete(toolId);
      // Complete an older list request before fetching authoritative post-submit state.
      if (this.inFlight) await this.inFlight;
      await this.refresh();
    }
  }
}
