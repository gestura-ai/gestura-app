import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface DelegatedTask {
  id: string;
  agent_id: string;
  description: string;
  priority: number;
  context: Record<string, unknown>;
}

interface Agent {
  id: string;
  name: string;
  status: string;
}

const WorkflowsPanel: React.FC = () => {
  const [activeTasks, setActiveTasks] = useState<DelegatedTask[]>([]);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [loading, setLoading] = useState(true);
  const [showNewTask, setShowNewTask] = useState(false);
  const [newTask, setNewTask] = useState({
    description: '',
    agentId: '',
    priority: 5,
  });

  useEffect(() => {
    loadData();
    const interval = setInterval(loadData, 5000); // Refresh every 5s
    return () => clearInterval(interval);
  }, []);

  const loadData = async () => {
    try {
      const [tasks, agentList] = await Promise.all([
        invoke<DelegatedTask[]>('list_active_tasks'),
        invoke<Agent[]>('list_agents'),
      ]);
      setActiveTasks(tasks);
      setAgents(agentList);
    } catch (error) {
      console.error('Failed to load workflow data:', error);
    } finally {
      setLoading(false);
    }
  };

  const handleCreateTask = async () => {
    if (!newTask.description.trim()) return;

    try {
      const task: DelegatedTask = {
        id: `task-${Date.now()}`,
        agent_id: newTask.agentId || 'default',
        description: newTask.description,
        priority: newTask.priority,
        context: {},
      };
      await invoke('delegate_task', { task });
      setNewTask({ description: '', agentId: '', priority: 5 });
      setShowNewTask(false);
      loadData();
    } catch (error) {
      console.error('Failed to create task:', error);
    }
  };

  const handleCancelTask = async (taskId: string) => {
    try {
      await invoke('cancel_task', { taskId });
      loadData();
    } catch (error) {
      console.error('Failed to cancel task:', error);
    }
  };

  const handleSpawnAgent = async () => {
    const id = `agent-${Date.now()}`;
    const name = `Agent ${agents.length + 1}`;
    try {
      await invoke('spawn_subagent', { agentId: id, name });
      loadData();
    } catch (error) {
      console.error('Failed to spawn agent:', error);
    }
  };

  if (loading) {
    return (
      <div className="workflows-panel">
        <h2>Workflows</h2>
        <p>Loading...</p>
      </div>
    );
  }

  return (
    <div className="workflows-panel">
      <div className="workflows-header">
        <h2>Workflows & Tasks</h2>
        <div className="workflows-actions">
          <button className="btn btn-secondary" onClick={handleSpawnAgent}>
            + Agent
          </button>
          <button className="btn" onClick={() => setShowNewTask(true)}>
            + Task
          </button>
        </div>
      </div>

      <div className="workflows-content">
        {/* Agents Section */}
        <div className="workflows-section">
          <h3>Agents ({agents.length})</h3>
          {agents.length === 0 ? (
            <p className="empty-state">No agents running. Click "+ Agent" to spawn one.</p>
          ) : (
            <div className="agent-list">
              {agents.map((agent) => (
                <div key={agent.id} className="agent-card">
                  <div className="agent-name">🤖 {agent.name}</div>
                  <div className="agent-status">{agent.status}</div>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Active Tasks Section */}
        <div className="workflows-section">
          <h3>Active Tasks ({activeTasks.length})</h3>
          {activeTasks.length === 0 ? (
            <p className="empty-state">No active tasks. Create one to get started.</p>
          ) : (
            <div className="task-list">
              {activeTasks.map((task) => (
                <div key={task.id} className="task-card">
                  <div className="task-header">
                    <span className="task-id">{task.id}</span>
                    <span className={`task-priority priority-${task.priority > 7 ? 'high' : task.priority > 3 ? 'medium' : 'low'}`}>
                      P{task.priority}
                    </span>
                  </div>
                  <div className="task-description">{task.description}</div>
                  <div className="task-footer">
                    <span className="task-agent">Agent: {task.agent_id}</span>
                    <button className="btn btn-small btn-danger" onClick={() => handleCancelTask(task.id)}>
                      Cancel
                    </button>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      {/* New Task Modal */}
      {showNewTask && (
        <div className="modal-overlay" onClick={() => setShowNewTask(false)}>
          <div className="modal-content" onClick={(e) => e.stopPropagation()}>
            <h3>Create New Task</h3>
            <div className="form-group">
              <label>Description</label>
              <textarea
                value={newTask.description}
                onChange={(e) => setNewTask({ ...newTask, description: e.target.value })}
                placeholder="Describe the task..."
                rows={3}
              />
            </div>
            <div className="form-group">
              <label>Agent</label>
              <select
                value={newTask.agentId}
                onChange={(e) => setNewTask({ ...newTask, agentId: e.target.value })}
              >
                <option value="">Auto-assign</option>
                {agents.map((agent) => (
                  <option key={agent.id} value={agent.id}>
                    {agent.name}
                  </option>
                ))}
              </select>
            </div>
            <div className="form-group">
              <label>Priority (1-10)</label>
              <input
                type="range"
                min="1"
                max="10"
                value={newTask.priority}
                onChange={(e) => setNewTask({ ...newTask, priority: parseInt(e.target.value) })}
              />
              <span>{newTask.priority}</span>
            </div>
            <div className="modal-actions">
              <button className="btn btn-secondary" onClick={() => setShowNewTask(false)}>
                Cancel
              </button>
              <button className="btn" onClick={handleCreateTask}>
                Create Task
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};

export default WorkflowsPanel;
