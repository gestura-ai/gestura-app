import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { Task, TaskHierarchy } from '../../agent/types';
import {
  clearMemoryConsoleEntries,
  deleteMemoryEntry,
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
  return (detail?.lifecycle.events ?? []) as TaskMemoryLifecycleEvent[];
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
      const next = await getMemoryConsoleOverview(selectedSessionId, selectedWorkspaceDir);
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
      setTaskDetail(await getMemoryTaskLifecycle(selectedSessionId, nextTaskId));
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
      setWorking(await getMemoryWorkingSnapshot(selectedSessionId));
    }

    if (activeTab === 'promotions' && selectedSessionId) {
      setPromotions(await getMemoryPromotionCandidates(selectedSessionId, 12));
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
      getMemoryWorkingSnapshot(selectedSessionId).then(setWorking).catch((err) => setError(String(err)));
    }
    if (activeTab === 'promotions' && selectedSessionId) {
      getMemoryPromotionCandidates(selectedSessionId, 12)
        .then(setPromotions)
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
        await searchMemoryConsoleEntries(
          { include_archived: true, limit: 24, ...query },
          selectedSessionId,
          selectedWorkspaceDir,
        ),
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
    setEntryDetail(await getMemoryEntryDetail(entryId, selectedSessionId, selectedWorkspaceDir));
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
      setEntryDetail(updated);
      await refreshOverview();
    } finally {
      setBusy(false);
    }
  }

  const tabs: Array<[TabKey, string]> = [
    ['overview', 'Overview'],
    ['search', 'Search'],
    ['working', 'Working (session)'],
    ['durable', 'Memory Bank (durable)'],
    ['promotions', 'Promotions'],
    ['task', 'Task'],
    ['maintenance', 'Maintenance'],
  ];

  return (
    <div className="session-panel">
      <div className="task-panel-header">
        <div>
          <h3>{title}</h3>
          <p className="task-panel-subtitle">Short-term session working memory, long-term memory bank entries, promotions, and task lifecycle in one console.</p>
        </div>
        <div className="task-header-actions">
          {allowSessionSelection && (
            <select
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
          )}
          <button className="task-header-btn" onClick={() => void refreshOverview()} disabled={busy}>Refresh</button>
        </div>
      </div>

      <div className="task-panel-filters" style={{ gap: 8, alignItems: 'center' }}>
        {tabs.map(([key, label]) => (
          <button key={key} className="task-header-btn" onClick={() => setActiveTab(key)}>{label}</button>
        ))}
        <input value={searchText} onChange={(e) => setSearchText(e.target.value)} placeholder="Search memory" />
        <button className="task-create-btn" onClick={() => void runSearch()} disabled={busy}>Search</button>
        {sharedCognitionCount > 0 && (
          <button
            className="task-header-btn"
            onClick={() => void runSearchWithQuery({ category: 'shared_cognition', text: null })}
            disabled={busy}
          >
            Shared cognition
          </button>
        )}
      </div>

      {error && <div className="task-empty-state">{error}</div>}

      {activeTab === 'overview' && overview && (
        <div className="task-section-card">
          <p><strong>Workspace:</strong> {overview.workspace_dir}</p>
          <p><strong>Memory bank entries:</strong> {overview.durable_total}</p>
          <p><strong>Session working-memory resources / decisions:</strong> {overview.working_resource_count} / {overview.working_decision_count}</p>
          <p><strong>Open blockers / promotions:</strong> {overview.open_blocker_count} / {overview.promotion_candidate_count}</p>
          <p><strong>Governance review / issues:</strong> {overview.governance_review_count} / {overview.governance_issue_count}</p>
          <p><strong>Shared cognition entries:</strong> {sharedCognitionCount}</p>
          {overview.counts_by_category.length > 0 && (
            <p><strong>Categories:</strong> {overview.counts_by_category.map((item) => `${item.key} (${item.count})`).join(', ')}</p>
          )}
          <p><strong>Summary:</strong> {overview.working_summary || 'No working-memory summary yet.'}</p>
        </div>
      )}

      {activeTab === 'search' && searchResults && (
        <div className="task-section-card">
          <h4>Working-memory matches</h4>
          {searchResults.working_memory.map((item) => <p key={item.id}>{item.section}: {item.summary}</p>)}
          <h4>Durable-memory matches</h4>
          {searchResults.durable_memory.map((item) => (
            <button key={item.entry_id} className="task-item" onClick={() => void openEntry(item.entry_id)}>
              {item.summary}
              <span className="task-path-label">
                {item.scope} · {item.memory_type}
                {item.category ? ` · ${item.category}` : ''} · {Math.round(item.confidence * 100)}% · {governanceLabel(item.governance_state)}
                {item.task_id ? ` · task ${item.task_id}` : ''}
                {item.directive_id ? ` · directive ${item.directive_id}` : ''}
                {item.governance_issue_count > 0 ? ` · ${item.governance_issue_count} issues` : ''}
              </span>
            </button>
          ))}
        </div>
      )}

      {activeTab === 'working' && working && (
        <div className="task-section-card">
          <p><strong>Scope:</strong> Short-term working memory for the active session only. This is not the durable memory bank.</p>
          <p><strong>Summary:</strong> {working.summary || 'No summary'}</p>
          <p><strong>Resources:</strong> {working.resources.length}</p>
          <p><strong>Decisions:</strong> {working.decisions.length}</p>
          <p><strong>Blockers:</strong> {working.blockers.length}</p>
          <p><strong>Next actions:</strong> {working.next_actions.join(', ') || 'None'}</p>
        </div>
      )}

      {activeTab === 'durable' && (
        <div className="task-section-card">
          <p><strong>Scope:</strong> Durable memory-bank entries persisted beyond the active session and available for later retrieval.</p>
          {durableEntries.map((item) => (
            <button key={item.entry_id} className="task-item" onClick={() => void openEntry(item.entry_id)}>
              {item.summary}{' '}
              <span className="task-path-label">
                {item.scope} · {item.memory_type}
                {item.category ? ` · ${item.category}` : ''} · {Math.round(item.confidence * 100)}% · {governanceLabel(item.governance_state)}
                {item.task_id ? ` · task ${item.task_id}` : ''}
                {item.directive_id ? ` · directive ${item.directive_id}` : ''}
                {item.governance_issue_count > 0 ? ` · ${item.governance_issue_count} issues` : ''}
              </span>
            </button>
          ))}
        </div>
      )}

      {activeTab === 'promotions' && (
        <div className="task-section-card">
          {promotions.map((item, index) => (
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
          ))}
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
                    className="task-item"
                    onClick={() => void loadTaskDetail(task.id)}
                  >
                    {task.name}
                    <span className="task-path-label">{task.id} · {formatTaskStatus(task.status)}</span>
                  </button>
                ))}
              </div>
            </>
          )}
          <div className="task-panel-filters" style={{ gap: 8 }}>
            <input value={taskId} onChange={(e) => setTaskId(e.target.value)} placeholder="Task id" />
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
          <button
            className="task-action-btn"
            onClick={() => {
              void refreshMemoryConsoleGovernance(selectedSessionId, selectedWorkspaceDir)
                .then(async () => {
                  await refreshOverview();
                  if (entryDetail) {
                    setEntryDetail(await getMemoryEntryDetail(entryDetail.summary.entry_id, selectedSessionId, selectedWorkspaceDir));
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
          />
          <textarea
            value={entryDetail.governance_note ?? ''}
            onChange={(e) => setEntryDetail({ ...entryDetail, governance_note: e.target.value || null })}
            rows={3}
            placeholder="Governance note"
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
            <button className="task-action-btn" onClick={() => void setMemoryEntryArchived(entryDetail.summary.entry_id, !entryDetail.summary.archived, selectedSessionId, selectedWorkspaceDir).then(async (next) => { setEntryDetail(next); await refreshOverview(); })}>
              {entryDetail.summary.archived ? 'Restore' : 'Archive'}
            </button>
            <button className="task-action-btn danger" onClick={() => window.confirm('Delete this memory entry?') && void deleteMemoryEntry(entryDetail.summary.entry_id, selectedSessionId, selectedWorkspaceDir).then(() => { setEntryDetail(null); return refreshOverview(); })}>Delete</button>
          </div>
        </div>
      )}
    </div>
  );
}

export default MemoryConsolePanel;