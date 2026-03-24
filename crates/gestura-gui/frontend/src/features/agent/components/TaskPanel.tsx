import { useCallback, useState } from "react";
import {
  createTask,
  deleteTask,
  updateTaskStatus,
} from "../../../services/tauri/agent";
import type { Task, TaskHierarchy, TaskRuntimeSnapshot, TaskStatus } from "../types";
import { parseMarkdown } from "../utils/markdown";
import { TaskBreakdownModal } from "./TaskBreakdownModal";

interface TaskPanelProps {
  isOpen: boolean;
  onClose: () => void;
  sessionId: string;
  tasks: TaskHierarchy;
  runtimeTaskSnapshot?: TaskRuntimeSnapshot | null;
  onRefreshTasks: () => Promise<void>;
  onSendMessage: (text: string, taskId?: string | null) => Promise<void>;
  onShowToast: (msg: string, kind?: "success" | "error" | "warning" | "info") => void;
}

const STATUS_ORDER: TaskStatus[] = ["NotStarted", "InProgress", "Completed", "Cancelled"];

function flattenTasks(tasks: TaskHierarchy): Task[] {
  return tasks.flatMap((task) => [task, ...flattenTasks(task.subtasks ?? [])]);
}

function isTerminalTaskStatus(status: TaskStatus): boolean {
  switch (status) {
    case "Completed":
    case "Cancelled":
      return true;
    case "NotStarted":
    case "Blocked":
    case "InProgress":
      return false;
    default: {
      const exhaustiveCheck: never = status;
      return exhaustiveCheck;
    }
  }
}

function statusClass(s: TaskStatus): string {
  switch (s) {
    case "InProgress": return "in-progress";
    case "Completed": return "completed";
    case "Cancelled": return "cancelled";
    default: return "not-started";
  }
}

function statusIcon(s: TaskStatus): string {
  switch (s) {
    case "InProgress": return "icon-refresh";
    case "Completed": return "icon-check";
    case "Cancelled": return "icon-close";
    default: return "icon-circle";
  }
}

function nextStatus(s: TaskStatus): TaskStatus {
  const idx = STATUS_ORDER.indexOf(s);
  return STATUS_ORDER[(idx + 1) % STATUS_ORDER.length];
}

function summarizeRuntimeTasks(tasks: TaskRuntimeSnapshot['ready_tasks'], limit = 3): string | null {
  if (tasks.length === 0) return null;
  const names = tasks.slice(0, limit).map((task) => task.name);
  const extra = tasks.length - names.length;
  return extra > 0 ? `${names.join(', ')} +${extra} more` : names.join(', ');
}

export function TaskPanel({
  isOpen, onClose, sessionId, tasks, runtimeTaskSnapshot, onRefreshTasks, onSendMessage, onShowToast,
}: TaskPanelProps) {
  const [showCreateForm, setShowCreateForm] = useState(false);
  const [newName, setNewName] = useState("");
  const [newDesc, setNewDesc] = useState("");
  const [saving, setSaving] = useState(false);
  const [showBreakdown, setShowBreakdown] = useState(false);

  const allTasks = flattenTasks(tasks);
  const firstOpenTask = allTasks.find((task) => !isTerminalTaskStatus(task.status)) ?? allTasks[0];

  const handleCreate = useCallback(async () => {
    if (!newName.trim()) return;
    setSaving(true);
    try {
      await createTask(sessionId, newName.trim(), newDesc.trim() || null);
      await onRefreshTasks();
      setNewName(""); setNewDesc(""); setShowCreateForm(false);
      onShowToast("Task created", "success");
    } catch (e) {
      onShowToast(`Failed to create task: ${e}`, "error");
    } finally { setSaving(false); }
  }, [sessionId, newName, newDesc, onRefreshTasks, onShowToast]);

  const handleCycleStatus = useCallback(async (task: Task) => {
    try {
      await updateTaskStatus(sessionId, task.id, nextStatus(task.status));
      await onRefreshTasks();
    } catch (e) { onShowToast(`Status update failed: ${e}`, "error"); }
  }, [sessionId, onRefreshTasks, onShowToast]);

  const handlePlay = useCallback(async (task: Task) => {
    await onSendMessage(
      `Please work on this task: ${task.name}${task.description ? "\n" + task.description : ""}`,
      task.id,
    );
    onShowToast(`Started task: ${task.name}`, "info");
    onClose();
  }, [onSendMessage, onShowToast, onClose]);

  const handleDelete = useCallback(async (task: Task) => {
    if (!confirm(`Delete task "${task.name}"?`)) return;
    try {
      await deleteTask(sessionId, task.id);
      await onRefreshTasks();
      onShowToast("Task deleted", "success");
    } catch (e) { onShowToast(`Delete failed: ${e}`, "error"); }
  }, [sessionId, onRefreshTasks, onShowToast]);

  const handleCleanup = useCallback(async () => {
    const done = allTasks.filter((task) => isTerminalTaskStatus(task.status));
    await Promise.all(done.map(t => deleteTask(sessionId, t.id).catch(() => { })));
    await onRefreshTasks();
    onShowToast(`Cleaned up ${done.length} finished task(s)`, "success");
  }, [allTasks, sessionId, onRefreshTasks, onShowToast]);

  return (
    <>
      <div className={`session-panel-overlay${isOpen ? " visible" : ""}`} onClick={onClose} />
      <div className={`session-panel${isOpen ? " open" : ""}`}>
        <div className="session-panel-header">
          <h3>Tasks</h3>
          <div className="task-header-actions">
            <button className="task-header-btn" onClick={() => setShowCreateForm(v => !v)} title="Add Task">
              <span className="icon-plus" />
            </button>
            <button className="task-header-btn" onClick={() => setShowBreakdown(true)} title="Break Down Requirements">
              <span className="icon-file-plus-02" />
            </button>
            <button className="task-header-btn" onClick={() => firstOpenTask && handlePlay(firstOpenTask)} title="Play First Task">
              <span className="icon-play-circle" />
            </button>
            <button className="task-header-btn" onClick={handleCleanup} title="Cleanup Finished">
              <span className="icon-file-x-02" />
            </button>
          </div>
          <button className="session-panel-close" onClick={onClose} title="Close">
            <span className="icon-close" />
          </button>
        </div>

        <div className="session-panel-content">
          {showCreateForm && (
            <div className="task-create-form visible">
              <input value={newName} onChange={e => setNewName(e.target.value)}
                placeholder="Task title..." autoFocus onKeyDown={e => e.key === "Enter" && handleCreate()} />
              <textarea value={newDesc} onChange={e => setNewDesc(e.target.value)}
                placeholder="Description (optional)..." />
              <div className="task-create-actions">
                <button className="task-create-cancel" onClick={() => { setShowCreateForm(false); setNewName(""); setNewDesc(""); }}>Cancel</button>
                <button className="task-create-save" onClick={handleCreate} disabled={saving || !newName.trim()}>
                  {saving ? "Saving..." : "Add Task"}
                </button>
              </div>
            </div>
          )}

          {runtimeTaskSnapshot && (
            <div className="task-runtime-summary" aria-label="Runtime task status">
              <div className="task-runtime-summary-header">
                <strong>Runtime focus</strong>
                {runtimeTaskSnapshot.current_task && (
                  <span className="task-runtime-current">
                    {runtimeTaskSnapshot.current_task.name} [{runtimeTaskSnapshot.current_task.status}]
                  </span>
                )}
              </div>
              <p>{runtimeTaskSnapshot.status_message}</p>
              {runtimeTaskSnapshot.missing_requirements.length > 0 && (
                <p>
                  Remaining checks: {runtimeTaskSnapshot.missing_requirements.join(', ')}
                </p>
              )}
              {summarizeRuntimeTasks(runtimeTaskSnapshot.ready_tasks) && (
                <p>Ready now: {summarizeRuntimeTasks(runtimeTaskSnapshot.ready_tasks)}</p>
              )}
              {!runtimeTaskSnapshot.ready_tasks.length
                && summarizeRuntimeTasks(runtimeTaskSnapshot.parallel_ready_tasks) && (
                  <p>
                    Parallel-ready: {summarizeRuntimeTasks(runtimeTaskSnapshot.parallel_ready_tasks)}
                  </p>
                )}
              {!runtimeTaskSnapshot.ready_tasks.length
                && !runtimeTaskSnapshot.parallel_ready_tasks.length
                && summarizeRuntimeTasks(runtimeTaskSnapshot.blocked_tasks) && (
                  <p>Blocked on: {summarizeRuntimeTasks(runtimeTaskSnapshot.blocked_tasks)}</p>
                )}
            </div>
          )}

          <div className="task-list">
            {allTasks.length === 0 ? (
              <div className="task-empty">No tasks yet. Click + to get started.</div>
            ) : (
              tasks.map((task) => (
                <TaskItem
                  key={task.id}
                  task={task}
                  runtimeCurrentTaskId={runtimeTaskSnapshot?.current_task?.id ?? null}
                  onCycleStatus={handleCycleStatus}
                  onPlay={handlePlay}
                  onDelete={handleDelete}
                />
              ))
            )}
          </div>
        </div>
      </div>

      <TaskBreakdownModal
        isOpen={showBreakdown}
        sessionId={sessionId}
        onClose={() => setShowBreakdown(false)}
        onRefreshTasks={onRefreshTasks}
        onShowToast={onShowToast}
      />
    </>
  );
}

interface TaskItemProps {
  task: Task;
  depth?: number;
  runtimeCurrentTaskId?: string | null;
  onCycleStatus: (t: Task) => Promise<void>;
  onPlay: (t: Task) => Promise<void>;
  onDelete: (t: Task) => Promise<void>;
}

function TaskItem({
  task,
  depth = 0,
  runtimeCurrentTaskId,
  onCycleStatus,
  onPlay,
  onDelete,
}: TaskItemProps) {
  const isRuntimeCurrent = runtimeCurrentTaskId === task.id;

  return (
    <>
      <div
        className={`task-item${depth > 0 ? " subtask" : ""}${isRuntimeCurrent ? " runtime-current" : ""}`}
        style={depth > 0 ? { marginLeft: `${depth * 16}px` } : undefined}
      >
        <div className="task-header">
          <div className={`task-status-icon ${statusClass(task.status)}`}
            onClick={() => onCycleStatus(task)} title={`Status: ${task.status} — click to cycle`}>
            <span className={statusIcon(task.status)} />
          </div>
          <span className="task-name">{task.name}</span>
          <div className="task-item-actions">
            <button className="task-icon-btn play" onClick={() => onPlay(task)} title="Run task">
              <span className="icon-play" />
            </button>
            <button className="task-icon-btn delete" onClick={() => onDelete(task)} title="Delete">
              <span className="icon-trash" />
            </button>
          </div>
        </div>
        {task.description && (
          <div
            className="task-description text-content markdown-body"
            dangerouslySetInnerHTML={{ __html: parseMarkdown(task.description) }}
          />
        )}
      </div>
      {(task.subtasks ?? []).map((child) => (
        <TaskItem
          key={child.id}
          task={child}
          depth={depth + 1}
          runtimeCurrentTaskId={runtimeCurrentTaskId}
          onCycleStatus={onCycleStatus}
          onPlay={onPlay}
          onDelete={onDelete}
        />
      ))}
    </>
  );
}

