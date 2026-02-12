import React, { useState } from 'react';
import { Agent, listAgents } from '../../services/tauri/agents';
import { cancelTask, delegateTask, DelegatedTask, listActiveTasks, spawnSubagent } from '../../services/tauri/workflows';
import { Button } from '../../shared/components/Button';
import { FormGroup } from '../../shared/components/FormGroup';
import { useAsyncState } from '../../shared/hooks/useAsyncState';
import { useInterval } from '../../shared/hooks/useInterval';

const WorkflowsPanel: React.FC = () => {
  const [showNewTask, setShowNewTask] = useState(false);
  const [newTask, setNewTask] = useState({
    description: '',
    agentId: '',
    priority: 5,
  });

  const workflowsState = useAsyncState(
    async () => {
      const [tasks, agentRes] = await Promise.all([listActiveTasks(), listAgents()]);
      return {
        activeTasks: tasks,
        agents: (Array.isArray(agentRes?.agents) ? agentRes.agents : []) as Agent[],
      };
    },
    { errorMessage: 'Failed to load workflow data:' }
  );

  const activeTasks: DelegatedTask[] = workflowsState.data?.activeTasks ?? [];
  const agents: Agent[] = workflowsState.data?.agents ?? [];

  useInterval(() => {
    void workflowsState.reload({ showLoading: false });
  }, 5000);

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
      await delegateTask(task);
      setNewTask({ description: '', agentId: '', priority: 5 });
      setShowNewTask(false);
      void workflowsState.reload({ showLoading: false });
    } catch (error) {
      console.error('Failed to create task:', error);
    }
  };

  const handleCancelTask = async (taskId: string) => {
    try {
      await cancelTask(taskId);
      void workflowsState.reload({ showLoading: false });
    } catch (error) {
      console.error('Failed to cancel task:', error);
    }
  };

  const handleSpawnAgent = async () => {
    const id = `agent-${Date.now()}`;
    const name = `Agent ${agents.length + 1}`;
    try {
      await spawnSubagent(id, name);
      void workflowsState.reload({ showLoading: false });
    } catch (error) {
      console.error('Failed to spawn agent:', error);
    }
  };

  if (workflowsState.loading) {
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
          <Button tone="secondary" onClick={handleSpawnAgent}>
            + Agent
          </Button>
          <Button onClick={() => setShowNewTask(true)}>
            + Task
          </Button>
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
                    <span
                      className={`task-priority priority-${task.priority > 7 ? 'high' : task.priority > 3 ? 'medium' : 'low'}`}
                    >
                      P{task.priority}
                    </span>
                  </div>
                  <div className="task-description">{task.description}</div>
                  <div className="task-footer">
                    <span className="task-agent">Agent: {task.agent_id}</span>
                    <Button tone="danger" size="small" onClick={() => handleCancelTask(task.id)}>
                      Cancel
                    </Button>
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
            <FormGroup label="Description">
              <textarea
                value={newTask.description}
                onChange={(e) => setNewTask({ ...newTask, description: e.target.value })}
                placeholder="Describe the task..."
                rows={3}
              />
            </FormGroup>

            <FormGroup label="Agent">
              <select value={newTask.agentId} onChange={(e) => setNewTask({ ...newTask, agentId: e.target.value })}>
                <option value="">Auto-assign</option>
                {agents.map((agent) => (
                  <option key={agent.id} value={agent.id}>
                    {agent.name}
                  </option>
                ))}
              </select>
            </FormGroup>

            <FormGroup label="Priority (1-10)">
              <input
                type="range"
                min="1"
                max="10"
                value={newTask.priority}
                onChange={(e) => setNewTask({ ...newTask, priority: parseInt(e.target.value) })}
              />
              <span>{newTask.priority}</span>
            </FormGroup>
            <div className="modal-actions">
              <Button tone="secondary" onClick={() => setShowNewTask(false)}>
                Cancel
              </Button>
              <Button onClick={handleCreateTask}>
                Create Task
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};

export default WorkflowsPanel;
