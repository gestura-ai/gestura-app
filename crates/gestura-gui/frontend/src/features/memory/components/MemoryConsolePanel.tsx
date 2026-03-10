import { useCallback, useEffect, useMemo, useState } from 'react';
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
  searchMemoryConsoleEntries,
  setMemoryEntryArchived,
  updateMemoryEntryDetail,
  type MemoryConsoleEntryDetail,
  type MemoryConsoleOverview,
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
  allowSessionSelection?: boolean;
  title?: string;
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

export function MemoryConsolePanel({
  sessionId,
  workspaceDir,
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

  const durableEntries = useMemo(
    () => searchResults?.durable_memory ?? overview?.recent_entries ?? [],
    [overview, searchResults],
  );

  async function runSearch() {
    setBusy(true);
    setError(null);
    try {
      setSearchResults(
        await searchMemoryConsoleEntries(
          { text: searchText || null, limit: 24, include_archived: true },
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
    ['working', 'Working'],
    ['durable', 'Durable'],
    ['promotions', 'Promotions'],
    ['task', 'Task'],
    ['maintenance', 'Maintenance'],
  ];

  return (
    <div className="session-panel">
      <div className="task-panel-header">
        <div>
          <h3>{title}</h3>
          <p className="task-panel-subtitle">One memory console across working memory, durable memory, promotions, and task lifecycle.</p>
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
      </div>

      {error && <div className="task-empty-state">{error}</div>}

      {activeTab === 'overview' && overview && (
        <div className="task-section-card">
          <p><strong>Workspace:</strong> {overview.workspace_dir}</p>
          <p><strong>Durable entries:</strong> {overview.durable_total}</p>
          <p><strong>Working resources / decisions:</strong> {overview.working_resource_count} / {overview.working_decision_count}</p>
          <p><strong>Open blockers / promotions:</strong> {overview.open_blocker_count} / {overview.promotion_candidate_count}</p>
          <p><strong>Summary:</strong> {overview.working_summary || 'No working-memory summary yet.'}</p>
        </div>
      )}

      {activeTab === 'search' && searchResults && (
        <div className="task-section-card">
          <h4>Working-memory matches</h4>
          {searchResults.working_memory.map((item) => <p key={item.id}>{item.section}: {item.summary}</p>)}
          <h4>Durable-memory matches</h4>
          {searchResults.durable_memory.map((item) => (
            <button key={item.entry_id} className="task-item" onClick={() => void openEntry(item.entry_id)}>{item.summary}</button>
          ))}
        </div>
      )}

      {activeTab === 'working' && working && (
        <div className="task-section-card">
          <p><strong>Summary:</strong> {working.summary || 'No summary'}</p>
          <p><strong>Resources:</strong> {working.resources.length}</p>
          <p><strong>Decisions:</strong> {working.decisions.length}</p>
          <p><strong>Blockers:</strong> {working.blockers.length}</p>
          <p><strong>Next actions:</strong> {working.next_actions.join(', ') || 'None'}</p>
        </div>
      )}

      {activeTab === 'durable' && (
        <div className="task-section-card">
          {durableEntries.map((item) => (
            <button key={item.entry_id} className="task-item" onClick={() => void openEntry(item.entry_id)}>
              {item.summary} <span className="task-path-label">{item.scope} · {item.memory_type}</span>
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
          <div className="task-panel-filters" style={{ gap: 8 }}>
            <input value={taskId} onChange={(e) => setTaskId(e.target.value)} placeholder="Task id" />
            <button
              className="task-create-btn"
              onClick={() => selectedSessionId && getMemoryTaskLifecycle(selectedSessionId, taskId).then(setTaskDetail).catch((err) => setError(String(err)))}
              disabled={!selectedSessionId || !taskId}
            >
              Load
            </button>
          </div>
          {taskDetail && (
            <p>
              {taskDetail.lifecycle.last_memory_file_path || `${taskDetail.lifecycle.events.length} memory events`}
            </p>
          )}
        </div>
      )}

      {activeTab === 'maintenance' && (
        <div className="task-section-card">
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
          <input
            value={entryDetail.summary.summary}
            onChange={(e) => setEntryDetail({ ...entryDetail, summary: { ...entryDetail.summary, summary: e.target.value } })}
          />
          <textarea
            value={entryDetail.content}
            onChange={(e) => setEntryDetail({ ...entryDetail, content: e.target.value })}
            rows={8}
          />
          <div className="task-header-actions">
            <button className="task-action-btn" onClick={() => void saveEntry()} disabled={busy}>Save</button>
            <button className="task-action-btn" onClick={() => void setMemoryEntryArchived(entryDetail.summary.entry_id, !entryDetail.summary.archived, selectedSessionId, selectedWorkspaceDir).then(setEntryDetail)}>
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