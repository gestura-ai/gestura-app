import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { Task, TaskHierarchy } from '../../agent/types';
import {
  clearMemoryConsoleEntries,
  deleteMemoryEntry,
  type MemoryConsoleEntrySummary,
  getMemoryConsoleOverview,
  getMemoryConsoleSessions,
  getMemoryEntryDetail,
  getMemoryPromotionCandidates,
  getMemoryTaskLifecycle,
  getMemoryWorkingSnapshot,
  promoteMemoryCandidateEntry,
  refreshMemoryConsoleGovernance,
  searchMemoryConsoleEntries,
  setMemoryEntryArchived,
  updateMemoryEntryDetail,
  type MemoryConsoleEntryDetail,
  type MemoryConsoleOverview,
  type MemoryConsoleQuery,
  type MemoryConsoleSearchResponse,
  type MemoryConsoleSessionSummary,
  type SessionMemoryPromotionCandidate,
  type SessionWorkingMemory,
  type TaskMemoryConsoleDetail,
} from '../../../services/tauri/agent';

type TabKey = 'overview' | 'search' | 'working' | 'durable' | 'promotions' | 'task' | 'maintenance';

interface MemoryConsolePanelProps {
  sessionId?: string | null;
  workspaceDir?: string | null;
  tasks?: TaskHierarchy;
  refreshSignal?: number;
  allowSessionSelection?: boolean;
  isOpen?: boolean;
  onClose?: () => void;
  title?: string;
}

type TaskMemoryLifecycleEvent = {
  phase?: string;
  summary?: string;
  scope?: string | null;
  memory_type?: string | null;
  memory_file_path?: string | null;
  recorded_at?: string;
};

const emptyWorkingMemory: SessionWorkingMemory = {
  resources: [],
  decisions: [],
  blockers: [],
  timeline: [],
  next_actions: [],
  open_questions: [],
  summary: null,
};

function arrayOrEmpty<T>(value: T[] | null | undefined): T[] {
  return Array.isArray(value) ? value : [];
}

function normalizeEntrySummary(entry: MemoryConsoleEntrySummary): MemoryConsoleEntrySummary {
  return {
    ...entry,
    tags: arrayOrEmpty(entry.tags),
    matched_fields: arrayOrEmpty(entry.matched_fields),
  };
}

function normalizeOverview(next: MemoryConsoleOverview): MemoryConsoleOverview {
  return {
    ...next,
    recent_entries: arrayOrEmpty(next.recent_entries).map(normalizeEntrySummary),
    counts_by_kind: arrayOrEmpty(next.counts_by_kind),
    counts_by_type: arrayOrEmpty(next.counts_by_type),
    counts_by_scope: arrayOrEmpty(next.counts_by_scope),
    counts_by_category: arrayOrEmpty(next.counts_by_category),
    counts_by_governance: arrayOrEmpty(next.counts_by_governance),
  };
}

function normalizeSearchResponse(next: MemoryConsoleSearchResponse): MemoryConsoleSearchResponse {
  return {
    ...next,
    working_memory: arrayOrEmpty(next.working_memory),
    durable_memory: arrayOrEmpty(next.durable_memory).map(normalizeEntrySummary),
  };
}

function normalizeWorkingMemory(next: SessionWorkingMemory | null | undefined): SessionWorkingMemory | null {
  if (!next) {
    return null;
  }

  return {
    ...emptyWorkingMemory,
    ...next,
    resources: arrayOrEmpty(next.resources),
    decisions: arrayOrEmpty(next.decisions),
    blockers: arrayOrEmpty(next.blockers),
    timeline: arrayOrEmpty(next.timeline),
    next_actions: arrayOrEmpty(next.next_actions),
    open_questions: arrayOrEmpty(next.open_questions),
  };
}

function normalizePromotions(next: SessionMemoryPromotionCandidate[]): SessionMemoryPromotionCandidate[] {
  return arrayOrEmpty(next).map((item) => ({
    ...item,
    tags: arrayOrEmpty(item.tags),
  }));
}

function normalizeTaskDetail(next: TaskMemoryConsoleDetail): TaskMemoryConsoleDetail {
  return {
    ...next,
    lifecycle: {
      ...next.lifecycle,
      events: arrayOrEmpty(next.lifecycle?.events as Array<Record<string, unknown>> | undefined),
      last_memory_file_path: next.lifecycle?.last_memory_file_path ?? null,
    },
  };
}

function normalizeEntryDetail(next: MemoryConsoleEntryDetail): MemoryConsoleEntryDetail {
  return {
    ...next,
    summary: normalizeEntrySummary(next.summary),
    governance_suggestions: arrayOrEmpty(next.governance_suggestions),
    outcome_labels: arrayOrEmpty(next.outcome_labels),
  };
}

function promotionSourceLabel(source: SessionMemoryPromotionCandidate['source']) {
  switch (source) {
    case 'resource': return 'resource';
    case 'decision': return 'decision';
    case 'blocker': return 'blocker';
    case 'timeline': return 'timeline';
    case 'next_action': return 'next action';
  }
}

function promotionMemoryType(source: SessionMemoryPromotionCandidate['source']) {
  switch (source) {
    case 'resource': return 'resource';
    case 'decision': return 'decision';
    case 'blocker': return 'blocker';
    case 'timeline': return 'episodic';
    case 'next_action': return 'procedural';
  }
}

function promotionConfidence(score: number) {
  return Math.min(0.95, Math.max(0.55, score / 5));
}

function governanceLabel(state: MemoryConsoleEntryDetail['summary']['governance_state']) {
  return state.replace(/_/g, ' ');
}

function countForKey(counts: Array<{ key: string; count: number }>, key: string): number {
  return counts.find((item) => item.key === key)?.count ?? 0;
}

function flattenTasks(tasks: TaskHierarchy | undefined): Task[] {
  if (!tasks?.length) return [];

  const flattened: Task[] = [];
  const visit = (task: Task) => {
    flattened.push(task);
    task.subtasks?.forEach(visit);
  };

  tasks.forEach(visit);
  return flattened;
}

function formatTaskStatus(status: Task['status']): string {
  switch (status) {
    case 'NotStarted': return 'not started';
    case 'InProgress': return 'in progress';
    case 'Completed': return 'completed';
    case 'Cancelled': return 'cancelled';
    case 'Blocked': return 'blocked';
  }
}

function getTaskMemoryEvents(detail: TaskMemoryConsoleDetail | null): TaskMemoryLifecycleEvent[] {
  return arrayOrEmpty(detail?.lifecycle.events as TaskMemoryLifecycleEvent[] | undefined);
}

function formatEventPhase(phase?: string): string {
  return phase ? phase.replace(/_/g, ' ').toLowerCase() : 'event';
}

export function MemoryConsolePanel({
  sessionId,
  workspaceDir,
  tasks,
  refreshSignal = 0,
  allowSessionSelection = false,
  isOpen = true,
  onClose,
  title = 'Memory',
}: MemoryConsolePanelProps) {
  const [activeTab, setActiveTab] = useState<TabKey>('overview');
  const [sessions, setSessions] = useState<MemoryConsoleSessionSummary[]>([]);
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(sessionId ?? null);
  const [selectedWorkspaceDir, setSelectedWorkspaceDir] = useState<string | null>(workspaceDir ?? null);
  const [overview, setOverview] = useState<MemoryConsoleOverview | null>(null);
  const [searchText, setSearchText] = useState('');
  const [searchResults, setSearchResults] = useState<MemoryConsoleSearchResponse | null>(null);
  const [working, setWorking] = useState<SessionWorkingMemory | null>(null);
  const [promotions, setPromotions] = useState<SessionMemoryPromotionCandidate[]>([]);
  const [taskId, setTaskId] = useState('');
  const [taskDetail, setTaskDetail] = useState<TaskMemoryConsoleDetail | null>(null);
  const [entryDetail, setEntryDetail] = useState<MemoryConsoleEntryDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const lastRefreshSignalRef = useRef(refreshSignal);

  const sessionTasks = useMemo(() => flattenTasks(tasks), [tasks]);
  const preferredTaskId = useMemo(
    () => sessionTasks.find((task) => task.status === 'InProgress')?.id
      ?? sessionTasks.find((task) => task.status === 'Completed')?.id
      ?? sessionTasks[0]?.id
      ?? '',
    [sessionTasks],
  );

  useEffect(() => {
    setSelectedSessionId(sessionId ?? null);
  }, [sessionId]);

  useEffect(() => {
    setSelectedWorkspaceDir(workspaceDir ?? null);
  }, [workspaceDir]);

  useEffect(() => {
    if (!allowSessionSelection) {
      return;
    }
    getMemoryConsoleSessions(12)
      .then((items) => {
        setSessions(items);
        if (!selectedSessionId && items[0]) {
          setSelectedSessionId(items[0].session_id);
          setSelectedWorkspaceDir(items[0].workspace_dir ?? null);
        }
      })
      .catch((err) => setError(String(err)));
  }, [allowSessionSelection, selectedSessionId]);

  const refreshOverview = useCallback(async () => {
    if (!selectedSessionId && !selectedWorkspaceDir) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const next = normalizeOverview(await getMemoryConsoleOverview(selectedSessionId, selectedWorkspaceDir));
      setOverview(next);
      if (!selectedWorkspaceDir && next.workspace_dir) {
        setSelectedWorkspaceDir(next.workspace_dir);
      }
    } finally {
      setBusy(false);
    }
  }, [selectedSessionId, selectedWorkspaceDir]);

  useEffect(() => {
    refreshOverview().catch((err) => setError(String(err)));
  }, [refreshOverview]);

  const loadTaskDetail = useCallback(async (nextTaskId: string, suppressErrors = false) => {
    if (!selectedSessionId || !nextTaskId) return false;

    try {
      setTaskDetail(normalizeTaskDetail(await getMemoryTaskLifecycle(selectedSessionId, nextTaskId)));
      setTaskId(nextTaskId);
      if (!suppressErrors) {
        setError(null);
      }
      return true;
    } catch (err) {
      if (!suppressErrors) {
        setError(String(err));
      }
      return false;
    }
  }, [selectedSessionId]);

  const refreshActiveMemoryTab = useCallback(async () => {
    if (!selectedSessionId && !selectedWorkspaceDir) return;

    await refreshOverview();

    if (activeTab === 'working' && selectedSessionId) {
      setWorking(normalizeWorkingMemory(await getMemoryWorkingSnapshot(selectedSessionId)));
    }

    if (activeTab === 'promotions' && selectedSessionId) {
      setPromotions(normalizePromotions(await getMemoryPromotionCandidates(selectedSessionId, 12)));
    }

    if (activeTab === 'task' && selectedSessionId) {
      const candidateIds = [taskId, preferredTaskId, ...sessionTasks.map((task) => task.id)]
        .filter((value, index, all) => Boolean(value) && all.indexOf(value) === index);

      let loaded = false;
      for (const candidateId of candidateIds) {
        loaded = await loadTaskDetail(candidateId, true);
        if (loaded) break;
      }

      if (!loaded) {
        setTaskDetail(null);
      }
    }
  }, [
    activeTab,
    loadTaskDetail,
    preferredTaskId,
    refreshOverview,
    selectedSessionId,
    selectedWorkspaceDir,
    sessionTasks,
    taskId,
  ]);

  useEffect(() => {
    if (activeTab === 'working' && selectedSessionId) {
      getMemoryWorkingSnapshot(selectedSessionId)
        .then((next) => setWorking(normalizeWorkingMemory(next)))
        .catch((err) => setError(String(err)));
    }
    if (activeTab === 'promotions' && selectedSessionId) {
      getMemoryPromotionCandidates(selectedSessionId, 12)
        .then((next) => setPromotions(normalizePromotions(next)))
        .catch((err) => setError(String(err)));
    }
  }, [activeTab, selectedSessionId]);

  useEffect(() => {
    if (activeTab !== 'task' || !selectedSessionId) {
      return;
    }

    const candidateIds = [taskId, preferredTaskId, ...sessionTasks.map((task) => task.id)]
      .filter((value, index, all) => Boolean(value) && all.indexOf(value) === index);

    if (candidateIds.length === 0) {
      return;
    }

    let cancelled = false;

    const loadFirstAvailableTask = async () => {
      for (const candidateId of candidateIds) {
        const loaded = await loadTaskDetail(candidateId, true);
        if (loaded || cancelled) {
          return;
        }
      }
    };

    loadFirstAvailableTask().catch((err) => {
      if (!cancelled) {
        setError(String(err));
      }
    });

    return () => {
      cancelled = true;
    };
  }, [activeTab, loadTaskDetail, preferredTaskId, selectedSessionId, sessionTasks, taskId]);

  useEffect(() => {
    if (lastRefreshSignalRef.current === refreshSignal) {
      return;
    }

    lastRefreshSignalRef.current = refreshSignal;
    refreshActiveMemoryTab().catch((err) => setError(String(err)));
  }, [refreshActiveMemoryTab, refreshSignal]);

  const durableEntries = useMemo(
    () => searchResults?.durable_memory ?? overview?.recent_entries ?? [],
    [overview, searchResults],
  );

  const sharedCognitionCount = countForKey(overview?.counts_by_category ?? [], 'shared_cognition');

  async function runSearchWithQuery(query: MemoryConsoleQuery) {
    setBusy(true);
    setError(null);
    try {
      setSearchResults(
        normalizeSearchResponse(await searchMemoryConsoleEntries(
          { include_archived: true, limit: 24, ...query },
          selectedSessionId,
          selectedWorkspaceDir,
        )),
      );
      setActiveTab('search');
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  async function runSearch() {
    await runSearchWithQuery({ text: searchText || null });
  }

  async function openEntry(entryId: string) {
    setEntryDetail(normalizeEntryDetail(await getMemoryEntryDetail(entryId, selectedSessionId, selectedWorkspaceDir)));
  }

  async function saveEntry() {
    if (!entryDetail) return;
    setBusy(true);
    try {
      const updated = await updateMemoryEntryDetail(
        entryDetail.summary.entry_id,
        {
          summary: entryDetail.summary.summary,
          content: entryDetail.content,
          tags: entryDetail.summary.tags,
          confidence: entryDetail.summary.confidence,
          governance_state: entryDetail.summary.governance_state,
          governance_note: entryDetail.governance_note ?? null,
        },
        selectedSessionId,
        selectedWorkspaceDir,
      );
      setEntryDetail(normalizeEntryDetail(updated));
      await refreshOverview();
    } finally {
      setBusy(false);
    }
  }

  const tabs: Array<[TabKey, string]> = [
    ['overview', 'Overview'],
    ['search', 'Search'],
    ['working', 'Session working memory'],
    ['durable', 'Durable memory bank'],
    ['promotions', 'Promotions'],
    ['task', 'Task memory'],
    ['maintenance', 'Maintenance'],
  ];

  const activeTabLabel = tabs.find(([key]) => key === activeTab)?.[1] ?? 'Overview';
  const workingStats = working ? [
    ['Resources', working.resources.length],
    ['Decisions', working.decisions.length],
    ['Blockers', working.blockers.length],
    ['Timeline', working.timeline.length],
  ] : [];
  const overviewStats = overview ? [
    ['Memory bank entries', overview.durable_total],
    ['Working resources / decisions', `${overview.working_resource_count} / ${overview.working_decision_count}`],
    ['Open blockers / promotions', `${overview.open_blocker_count} / ${overview.promotion_candidate_count}`],
    ['Governance review / issues', `${overview.governance_review_count} / ${overview.governance_issue_count}`],
    ['Shared cognition entries', sharedCognitionCount],
  ] : [];

  return (
    <>
      {onClose && <div className={`session-panel-overlay${isOpen ? " visible" : ""}`} onClick={onClose} />}
      <div className={`session-panel${isOpen ? " open" : ""}`}>
        <div className="session-panel-header">
          <div>
            <h3>{title}</h3>
            <p className="task-panel-subtitle">Short-term session working memory, long-term memory bank entries, promotions, and task lifecycle in one console.</p>
          </div>
          {onClose && (
            <button className="session-panel-close" onClick={onClose} title="Close">
              <span className="icon-close" />
            </button>
          )}
        </div>

        <div className="session-panel-content">
          <div className="memory-console-toolbar">
            <div className="memory-console-toolbar-row">
              <label className="memory-console-field">
                <span>View</span>
                <select
                  aria-label="Memory view"
                  className="task-filter-select"
                  value={activeTab}
                  onChange={(event) => setActiveTab(event.target.value as TabKey)}
                >
                  {tabs.map(([key, label]) => (
                    <option key={key} value={key}>{label}</option>
                  ))}
                </select>
              </label>
              {allowSessionSelection && (
                <label className="memory-console-field">
                  <span>Session</span>
                  <select
                    aria-label="Memory session"
                    className="task-filter-select"
                    value={selectedSessionId ?? ''}
                    onChange={(event) => {
                      const nextId = event.target.value || null;
                      setSelectedSessionId(nextId);
                      const selected = sessions.find((item) => item.session_id === nextId);
                      setSelectedWorkspaceDir(selected?.workspace_dir ?? selectedWorkspaceDir);
                    }}
                  >
                    <option value="">Select session</option>
                    {sessions.map((item) => (
                      <option key={item.session_id} value={item.session_id}>{item.title}</option>
                    ))}
                  </select>
                </label>
              )}
              <div className="memory-console-field memory-console-field--actions">
                <span>Actions</span>
                <button className="task-action-btn" onClick={() => void refreshOverview()} disabled={busy}>Refresh</button>
              </div>
            </div>

            <div className="memory-console-toolbar-row memory-console-toolbar-row--search">
              <label className="memory-console-field memory-console-field--wide">
                <span>Search</span>
                <input
                  value={searchText}
                  onChange={(e) => setSearchText(e.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter') {
                      event.preventDefault();
                      void runSearch();
                    }
                  }}
                  placeholder="Search memory"
                />
              </label>
              <div className="memory-console-field memory-console-field--actions">
                <span>Quick actions</span>
                <div className="memory-console-inline-actions">
                  <button className="task-create-btn" onClick={() => void runSearch()} disabled={busy}>Search</button>
                  {sharedCognitionCount > 0 && (
                    <button
                      className="task-header-btn memory-console-chip-btn"
                      onClick={() => void runSearchWithQuery({ category: 'shared_cognition', text: null })}
                      disabled={busy}
                    >
                      Shared cognition
                    </button>
                  )}
                </div>
              </div>
            </div>
          </div>

          <div className="memory-console-active-view">Viewing: {activeTabLabel}</div>

          {error && <div className="task-empty-state">{error}</div>}

          {activeTab === 'overview' && overview && (
            <div className="task-section-card">
              <div className="memory-console-summary">
                <strong>Workspace</strong>
                <div className="task-path-label">{overview.workspace_dir}</div>
              </div>
              <div className="memory-console-stat-grid">
                {overviewStats.map(([label, value]) => (
                  <div key={label} className="memory-console-stat-card">
                    <span className="memory-console-stat-label">{label}</span>
                    <strong className="memory-console-stat-value">{value}</strong>
                  </div>
                ))}
              </div>
              {overview.counts_by_category.length > 0 && (
                <div>
                  <strong>Categories</strong>
                  <div className="memory-console-badges">
                    {overview.counts_by_category.map((item) => (
                      <span key={item.key} className="memory-console-badge">{item.key} ({item.count})</span>
                    ))}
                  </div>
                </div>
              )}
              <div className="memory-console-summary">
                <strong>Summary</strong>
                <p>{overview.working_summary || 'No working-memory summary yet.'}</p>
              </div>
            </div>
          )}

          {activeTab === 'search' && searchResults && (
            <div className="task-section-card">
              <h4>Working-memory matches</h4>
              {searchResults.working_memory.length > 0 ? (
                <div className="memory-console-list">
                  {searchResults.working_memory.map((item) => (
                    <div key={item.id} className="task-item-row">
                      <div>
                        <strong>{item.summary}</strong>
                        <div className="task-item-meta">{item.section}{item.status ? ` · ${item.status}` : ''}</div>
                      </div>
                    </div>
                  ))}
                </div>
              ) : (
                <div className="task-empty">No working-memory matches.</div>
              )}
              <h4>Durable-memory matches</h4>
              {searchResults.durable_memory.length > 0 ? searchResults.durable_memory.map((item) => (
                <button key={item.entry_id} type="button" className="task-item-row memory-console-row-button" onClick={() => void openEntry(item.entry_id)}>
                  <div style={{ wordBreak: 'break-word', whiteSpace: 'pre-wrap' }}>
                    <strong>{item.summary}</strong>
                    <div className="task-item-meta">
                      {item.scope} · {item.memory_type}
                      {item.category ? ` · ${item.category}` : ''} · {Math.round(item.confidence * 100)}% · {governanceLabel(item.governance_state)}
                      {item.task_id ? ` · task ${item.task_id}` : ''}
                      {item.directive_id ? ` · directive ${item.directive_id}` : ''}
                      {item.governance_issue_count > 0 ? ` · ${item.governance_issue_count} issues` : ''}
                    </div>
                  </div>
                </button>
              )) : <div className="task-empty">No durable-memory matches.</div>}
            </div>
          )}

          {activeTab === 'search' && !searchResults && (
            <div className="task-section-card">
              <div className="task-empty">Run a search to inspect working and durable memory matches.</div>
            </div>
          )}

          {activeTab === 'working' && working && (
            <div className="task-section-card">
              <p><strong>Scope:</strong> Short-term working memory for the active session only. This is not the durable memory bank.</p>
              <div className="memory-console-summary">
                <strong>Summary</strong>
                <p>{working.summary || 'No summary yet for this session.'}</p>
              </div>
              <div className="memory-console-stat-grid">
                {workingStats.map(([label, value]) => (
                  <div key={label} className="memory-console-stat-card">
                    <span className="memory-console-stat-label">{label}</span>
                    <strong className="memory-console-stat-value">{value}</strong>
                  </div>
                ))}
              </div>
              <div className="memory-console-split-grid">
                <div className="memory-console-summary">
                  <strong>Next actions</strong>
                  <p>{working.next_actions.join(', ') || 'None'}</p>
                </div>
                <div className="memory-console-summary">
                  <strong>Open questions</strong>
                  <p>{working.open_questions.join(', ') || 'None'}</p>
                </div>
              </div>
            </div>
          )}

          {activeTab === 'durable' && (
            <div className="task-section-card">
              <p><strong>Scope:</strong> Durable memory-bank entries persisted beyond the active session and available for later retrieval.</p>
              {durableEntries.length > 0 ? durableEntries.map((item) => (
                <button key={item.entry_id} type="button" className="task-item-row memory-console-row-button" onClick={() => void openEntry(item.entry_id)}>
                  <div style={{ wordBreak: 'break-word', whiteSpace: 'pre-wrap' }}>
                    <strong>{item.summary}</strong>
                    <div className="task-item-meta">
                      {item.scope} · {item.memory_type}
                      {item.category ? ` · ${item.category}` : ''} · {Math.round(item.confidence * 100)}% · {governanceLabel(item.governance_state)}
                      {item.task_id ? ` · task ${item.task_id}` : ''}
                      {item.directive_id ? ` · directive ${item.directive_id}` : ''}
                      {item.governance_issue_count > 0 ? ` · ${item.governance_issue_count} issues` : ''}
                    </div>
                  </div>
                </button>
              )) : <div className="task-empty">No durable memory entries yet.</div>}
            </div>
          )}

          {activeTab === 'promotions' && (
            <div className="task-section-card">
              {promotions.length > 0 ? promotions.map((item, index) => (
                <div key={`${item.summary}-${index}`} className="task-item-row">
                  <div>
                    <strong>{item.summary}</strong>
                    <div className="task-item-meta">{promotionSourceLabel(item.source)} · {promotionMemoryType(item.source)}</div>
                  </div>
                  <button
                    className="task-action-btn"
                    onClick={() => selectedSessionId && promoteMemoryCandidateEntry(selectedSessionId, {
                      summary: item.summary,
                      detail: item.detail,
                      memory_kind: 'long_term',
                      memory_type: promotionMemoryType(item.source),
                      scope: 'workspace',
                      tags: item.tags,
                      confidence: promotionConfidence(item.score),
                      promotion_reason: 'Promoted from GUI memory console',
                    }).then(() => refreshOverview())}
                  >
                    Promote
                  </button>
                </div>
              )) : <div className="task-empty">No promotion candidates yet.</div>}
            </div>
          )}

          {activeTab === 'task' && (
            <div className="task-section-card">
              {sessionTasks.length > 0 && (
                <>
                  <h4>Recent session tasks</h4>
                  <div className="task-list">
                    {sessionTasks.map((task) => (
                      <button
                        key={task.id}
                        type="button"
                        className="task-item-row memory-console-row-button"
                        onClick={() => void loadTaskDetail(task.id)}
                      >
                        <div>
                          <strong>{task.name}</strong>
                          <div className="task-item-meta">{task.id} · {formatTaskStatus(task.status)}</div>
                        </div>
                      </button>
                    ))}
                  </div>
                </>
              )}
              <div className="task-panel-filters memory-console-compact-actions">
                <input
                  value={taskId}
                  onChange={(e) => setTaskId(e.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter') {
                      event.preventDefault();
                      void loadTaskDetail(taskId);
                    }
                  }}
                  placeholder="Task id"
                />
                <button
                  className="task-create-btn"
                  onClick={() => void loadTaskDetail(taskId)}
                  disabled={!selectedSessionId || !taskId}
                >
                  Load
                </button>
              </div>
              {taskDetail && (
                <>
                  <p>
                    <strong>Latest durable memory:</strong> {taskDetail.lifecycle.last_memory_file_path || 'None yet'}
                  </p>
                  <p>
                    <strong>Memory events:</strong> {taskDetail.lifecycle.events.length}
                  </p>
                  {getTaskMemoryEvents(taskDetail).length > 0 ? (
                    <div className="task-list">
                      {getTaskMemoryEvents(taskDetail).map((event, index) => (
                        <div key={`${event.recorded_at ?? index}-${event.summary ?? index}`} className="task-item-row">
                          <div>
                            <strong>{event.summary || 'Memory lifecycle event'}</strong>
                            <div className="task-item-meta">
                              {formatEventPhase(event.phase)}
                              {event.scope ? ` · ${event.scope}` : ''}
                              {event.memory_type ? ` · ${event.memory_type}` : ''}
                              {event.memory_file_path ? ` · ${event.memory_file_path}` : ''}
                            </div>
                          </div>
                          {event.recorded_at && (
                            <span className="task-path-label">{new Date(event.recorded_at).toLocaleString()}</span>
                          )}
                        </div>
                      ))}
                    </div>
                  ) : (
                    <div className="task-empty">No memory lifecycle events recorded for this task yet.</div>
                  )}
                </>
              )}
            </div>
          )}

          {activeTab === 'maintenance' && (
            <div className="task-section-card">
              <div className="memory-console-maintenance-actions">
                <button
                  className="task-action-btn"
                  onClick={() => {
                    void refreshMemoryConsoleGovernance(selectedSessionId, selectedWorkspaceDir)
                      .then(async () => {
                        await refreshOverview();
                        if (entryDetail) {
                          setEntryDetail(normalizeEntryDetail(await getMemoryEntryDetail(entryDetail.summary.entry_id, selectedSessionId, selectedWorkspaceDir)));
                        }
                      })
                      .catch((err) => setError(String(err)));
                  }}
                >
                  Refresh governance suggestions
                </button>
                <button
                  className="task-action-btn danger"
                  onClick={() => {
                    if (window.confirm('Clear all durable memory entries for this workspace?')) {
                      void clearMemoryConsoleEntries(selectedSessionId, selectedWorkspaceDir).then(() => refreshOverview());
                    }
                  }}
                >
                  Clear durable memory
                </button>
              </div>
            </div>
          )}

          {entryDetail && (
            <div className="task-section-card">
              <h4>Entry Detail</h4>
              <p>
                <strong>Governance:</strong> {governanceLabel(entryDetail.summary.governance_state)}
                {entryDetail.summary.governance_issue_count > 0 ? ` · ${entryDetail.summary.governance_issue_count} suggestions` : ''}
              </p>
              <p>
                <strong>Category / scope / type:</strong> {entryDetail.summary.category ?? 'uncategorized'} / {entryDetail.summary.scope} / {entryDetail.summary.memory_type}
              </p>
              <p>
                <strong>Task / directive / agent:</strong> {entryDetail.summary.task_id ?? 'n/a'} / {entryDetail.summary.directive_id ?? 'n/a'} / {entryDetail.summary.agent_id ?? 'n/a'}
              </p>
              <p>
                <strong>Confidence / tags:</strong> {Math.round(entryDetail.summary.confidence * 100)}% / {entryDetail.summary.tags.join(', ') || 'none'}
              </p>
              <input
                value={entryDetail.summary.summary}
                onChange={(e) => setEntryDetail({ ...entryDetail, summary: { ...entryDetail.summary, summary: e.target.value } })}
                style={{ width: '100%', padding: '8px', marginBottom: '8px', borderRadius: '4px', border: '1px solid var(--glass-border)', background: 'var(--bg-base)', color: 'var(--text-primary)' }}
              />
              <select
                value={entryDetail.summary.governance_state}
                onChange={(e) => setEntryDetail({
                  ...entryDetail,
                  summary: {
                    ...entryDetail.summary,
                    governance_state: e.target.value as MemoryConsoleEntryDetail['summary']['governance_state'],
                  },
                })}
                style={{ width: '100%', padding: '8px', marginBottom: '8px', borderRadius: '4px', border: '1px solid var(--glass-border)', background: 'var(--bg-base)', color: 'var(--text-primary)' }}
              >
                <option value="active">active</option>
                <option value="pinned">pinned</option>
                <option value="needs_review">needs review</option>
                <option value="superseded">superseded</option>
                <option value="archived">archived</option>
              </select>
              <textarea
                value={entryDetail.content}
                onChange={(e) => setEntryDetail({ ...entryDetail, content: e.target.value })}
                rows={8}
                style={{ width: '100%', padding: '8px', marginBottom: '8px', borderRadius: '4px', border: '1px solid var(--glass-border)', background: 'var(--bg-base)', color: 'var(--text-primary)', resize: 'vertical' }}
              />
              <textarea
                value={entryDetail.governance_note ?? ''}
                onChange={(e) => setEntryDetail({ ...entryDetail, governance_note: e.target.value || null })}
                rows={3}
                placeholder="Governance note"
                style={{ width: '100%', padding: '8px', marginBottom: '8px', borderRadius: '4px', border: '1px solid var(--glass-border)', background: 'var(--bg-base)', color: 'var(--text-primary)', resize: 'vertical' }}
              />
              {entryDetail.governance_suggestions.length > 0 && (
                <div>
                  <h5>Governance suggestions</h5>
                  {entryDetail.governance_suggestions.map((suggestion) => (
                    <p key={`${suggestion.relationship}-${suggestion.entry_id}`}>
                      <strong>{suggestion.relationship.replace(/_/g, ' ')}</strong>: {suggestion.rationale} ({Math.round(suggestion.confidence * 100)}%)
                    </p>
                  ))}
                </div>
              )}
              {(entryDetail.strategy_key || entryDetail.outcome_labels.length > 0) && (
                <div>
                  <p><strong>Strategy:</strong> {entryDetail.strategy_key || 'n/a'}</p>
                  <p><strong>Outcomes:</strong> {entryDetail.outcome_labels.join(', ') || 'none'}</p>
                </div>
              )}
              <div className="task-header-actions">
                <button className="task-action-btn" onClick={() => void saveEntry()} disabled={busy}>Save</button>
                <button className="task-action-btn" onClick={() => void setMemoryEntryArchived(entryDetail.summary.entry_id, !entryDetail.summary.archived, selectedSessionId, selectedWorkspaceDir).then(async (next) => { setEntryDetail(normalizeEntryDetail(next)); await refreshOverview(); })}>
                  {entryDetail.summary.archived ? 'Restore' : 'Archive'}
                </button>
                <button className="task-action-btn danger" onClick={() => window.confirm('Delete this memory entry?') && void deleteMemoryEntry(entryDetail.summary.entry_id, selectedSessionId, selectedWorkspaceDir).then(() => { setEntryDetail(null); return refreshOverview(); })}>Delete</button>
              </div>
            </div>
          )}
        </div>
      </div>
    </>
  );
}

export default MemoryConsolePanel;