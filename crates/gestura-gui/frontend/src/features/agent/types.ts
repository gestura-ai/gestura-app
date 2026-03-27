// ─── View modes ──────────────────────────────────────────────────────────────

/** The agent window supports two layout modes. */
export type ViewMode = 'message-only' | 'editor';

// ─── Editor tabs ─────────────────────────────────────────────────────────────

/** A single open editor tab. */
export type EditorTabViewMode = 'edit' | 'preview';

export interface EditorOpenOptions {
  viewMode?: EditorTabViewMode;
}

export interface EditorTab {
  /** Stable unique identifier (nanoid or UUID). */
  id: string;
  /** Relative path from workspace root (e.g. "src/main.rs"). */
  relPath: string;
  /** Display filename (basename). */
  label: string;
  /** Content currently in the editor (may differ from disk). */
  content: string;
  /** True when the in-memory content differs from the last saved disk state. */
  isDirty: boolean;
  /** Byte offset of the editor cursor / scroll anchor; persisted for restore. */
  scrollOffset: number;
  /** Whether the tab shows the editable source or the rendered markdown preview. */
  viewMode: EditorTabViewMode;
  /** Whether this tab is in diff view mode. */
  isDiffView: boolean;
  /** Language identifier for syntax highlighting (e.g. "rust", "javascript"). */
  language: string;
  /**
   * File kind — governs how the tab is rendered.
   * text   → CodeMirror editor or rendered markdown preview (depends on viewMode)
   * image  → <img> preview (read-only)
   * binary → "Cannot open binary file" message
   */
  kind: 'text' | 'image' | 'binary';
}

// ─── Explorer types ───────────────────────────────────────────────────────────

/** Mirrors the Rust ExplorerGitChangeKind enum. */
export type ExplorerGitChangeKind =
  | 'added'
  | 'modified'
  | 'deleted'
  | 'renamed'
  | 'copied'
  | 'untracked'
  | 'unknown';

/** Combined staged/unstaged/untracked status for a single path (matches Rust). */
export interface ExplorerGitPathStatus {
  staged?: ExplorerGitChangeKind | null;
  unstaged?: ExplorerGitChangeKind | null;
  untracked: boolean;
}

/** A single file-system entry from explorer_list_dir. */
export interface ExplorerEntry {
  name: string;
  rel_path: string;
  /** Mirrors Rust ExplorerEntryKind: "file" | "dir" */
  kind: 'file' | 'dir';
  is_symlink: boolean;
  size?: number;
  git_status?: ExplorerGitPathStatus | null;
}

/** Response from explorer_get_root. */
export interface ExplorerRootResponse {
  root: string;
  is_git_repo: boolean;
}

/** Response from explorer_list_dir. */
export interface ExplorerListDirResponse {
  root: string;
  dir_rel: string;
  entries: ExplorerEntry[];
  truncated: boolean;
}

/** Response from explorer_git_status. */
export interface ExplorerGitStatusResponse {
  root: string;
  is_git_repo: boolean;
  paths: Record<string, ExplorerGitPathStatus>;
  error?: string | null;
}

// ─── Editor Tauri command responses ──────────────────────────────────────────

/** Response from editor_read_file. */
export interface EditorReadFileResponse {
  rel_path: string;
  content: string;
  language: string;
  kind: 'text' | 'image' | 'binary';
  /** Base64-encoded data URL for image files. */
  data_url?: string;
}

/** Response from editor_git_diff. */
export interface EditorGitDiffResponse {
  rel_path: string;
  original: string;
  modified: string;
  has_diff: boolean;
}

// ─── Chat / Agent streaming block types ──────────────────────────────────────

export type ShellState = 'Started' | 'Running' | 'Paused' | 'Resumed' | 'Completed' | 'Failed' | 'Stopped';

export type ShellSessionState = 'Starting' | 'Idle' | 'Busy' | 'Interrupting' | 'Stopping' | 'Stopped' | 'Failed';

export interface ShellLine {
  stream: 'Stdout' | 'Stderr';
  data: string;
}

export interface ThinkingBlock {
  kind: 'thinking';
  id: string;
  content: string;
  done: boolean;
  collapsed: boolean;
}

export interface TextBlock {
  kind: 'text';
  id: string;
  content: string;
}

export interface ToolBlock {
  kind: 'tool';
  id: string;
  name: string;
  args: string;
  status: 'running' | 'executing' | 'success' | 'error' | 'blocked';
  result?: string | null;
  durationMs?: number | null;
  collapsed: boolean;
}

export interface IterationMarkerBlock {
  kind: 'iteration-marker';
  id: string;
  /** Primary status line shown centered between two horizontal rules. */
  label: string;
  /** Optional contextual detail shown below the marker line describing what the agent is reviewing. */
  detail?: string;
}

export interface NarrationBlock {
  kind: 'narration';
  id: string;
  title?: string | null;
  message: string;
  summary?: string | null;
  reason?: string | null;
  nextStep?: string | null;
  evidence: string[];
  stage: 'context' | 'planning' | 'execution' | 'verification' | 'blocked' | 'progress';
  source?: 'llm' | 'review-fallback';
}

export interface ShellBlock {
  kind: 'shell';
  id: string;
  processId: string;
  shellSessionId?: string | null;
  command: string;
  cwd: string | null;
  state: ShellState;
  exitCode?: number | null;
  durationMs?: number | null;
  startedAt?: number | null;
  lastActivityAt?: number | null;
  lines: ShellLine[];
  collapsed: boolean;
}

export interface ShellSessionRecord {
  kind: 'shell-session';
  id: string;
  shellSessionId: string;
  cwd: string | null;
  state: ShellSessionState;
  interactive: boolean;
  userManaged: boolean;
  activeProcessId?: string | null;
  activeCommand?: string | null;
  lastExitCode?: number | null;
  durationMs?: number | null;
  startedAt?: number | null;
  lastActivityAt?: number | null;
  lines: ShellLine[];
  collapsed: boolean;
  availableForReuse: boolean;
}

export type MsgBlock =
  | ThinkingBlock
  | TextBlock
  | ToolBlock
  | IterationMarkerBlock
  | NarrationBlock
  | ShellBlock
  | ShellSessionRecord;

export interface AgentMessage {
  id: string;
  role: 'user' | 'assistant';
  /** Accumulated raw markdown text (for copy-to-clipboard). */
  rawMarkdown: string;
  blocks: MsgBlock[];
  isStreaming: boolean;
  timestamp: number;
}

// ─── Task types ───────────────────────────────────────────────────────────────

export type TaskStatus = 'NotStarted' | 'InProgress' | 'Completed' | 'Cancelled' | 'Blocked';

export interface Task {
  id: string;
  name: string;
  description?: string | null;
  status: TaskStatus;
  subtasks?: Task[];
}

export type TaskHierarchy = Task[];

export interface TaskRuntimeTaskView {
  id: string;
  name: string;
  status: string;
}

export interface TaskRuntimeSnapshot {
  root_task_id: string;
  current_task?: TaskRuntimeTaskView | null;
  ready_tasks: TaskRuntimeTaskView[];
  parallel_ready_tasks: TaskRuntimeTaskView[];
  blocked_tasks: TaskRuntimeTaskView[];
  open_tasks: TaskRuntimeTaskView[];
  completed_tasks: TaskRuntimeTaskView[];
  missing_requirements: string[];
  status_message: string;
}

// ─── Knowledge types ──────────────────────────────────────────────────────────

export interface KnowledgeItem {
  id: string;
  name: string;
  description?: string | null;
  category?: string | null;
  enabled?: boolean;
}

// ─── Tool confirmation types ──────────────────────────────────────────────────

export interface ToolConfirmation {
  confirmation_id: string;
  tool_name: string;
  tool_args?: string;
  description?: string;
  risk_level?: string;
  category?: string;
  session_id?: string | null;
}

export type ToolConfirmationDecision =
  | 'allow_once'
  | 'allow_session'
  | 'allow_always'
  | 'deny_once'
  | 'deny_session';

// ─── Stream health types ──────────────────────────────────────────────────────

export type StreamHealthStatus = 'healthy' | 'degraded' | 'reconnecting' | 'failed';

export interface StreamHealthPayload {
  status: StreamHealthStatus;
  message?: string | null;
}

// ─── Status bar ───────────────────────────────────────────────────────────────

export type StatusKind = 'ready' | 'busy' | 'listening' | 'error' | 'reflection';

export interface StatusState {
  text: string;
  kind: StatusKind;
}

// ─── Provider / model types ───────────────────────────────────────────────────

export type MessageRole = 'user' | 'assistant' | 'tool' | 'system';

/** Legacy simple chat message (kept for session history loading). */
export interface ChatMessage {
  id: string;
  role: MessageRole;
  content: string;
  timestamp: number;
  isStreaming?: boolean;
}

export interface ProviderOption {
  id: string;
  label: string;
}

export interface ModelOption {
  id: string;
  label: string;
  providerId: string;
}

