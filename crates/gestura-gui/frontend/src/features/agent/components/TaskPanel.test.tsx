import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { TaskPanel } from './TaskPanel';
import type { TaskHierarchy, TaskRuntimeSnapshot } from '../types';

vi.mock('../../../services/tauri/agent', () => ({
  createTask: vi.fn(),
  deleteTask: vi.fn(),
  updateTaskStatus: vi.fn(),
}));

vi.mock('./TaskBreakdownModal', () => ({
  TaskBreakdownModal: () => null,
}));

afterEach(() => {
  cleanup();
});

function renderTaskPanel(
  tasks: TaskHierarchy,
  runtimeTaskSnapshot: TaskRuntimeSnapshot | null = null,
  highlightedTaskId: string | null = null,
) {
  const onSendMessage = vi.fn().mockResolvedValue(undefined);
  const onRefreshTasks = vi.fn().mockResolvedValue(undefined);
  const onShowToast = vi.fn();
  const onClose = vi.fn();

  render(
    <TaskPanel
      isOpen
      onClose={onClose}
      sessionId="session-123"
      tasks={tasks}
      runtimeTaskSnapshot={runtimeTaskSnapshot}
      highlightedTaskId={highlightedTaskId}
      onRefreshTasks={onRefreshTasks}
      onSendMessage={onSendMessage}
      onShowToast={onShowToast}
    />,
  );

  return { onClose, onRefreshTasks, onSendMessage, onShowToast };
}

describe('TaskPanel', () => {
  it('starts the first non-terminal task from the panel', async () => {
    const tasks: TaskHierarchy = [{
      id: 'task-main-app',
      name: 'Create desktop app shell',
      description: 'Build the primary desktop application shell.',
      status: 'Completed',
      subtasks: [{
        id: 'task-build-app',
        name: 'Build the Tauri app',
        description: 'Compile and verify the application.',
        status: 'InProgress',
        subtasks: [{
          id: 'task-run-tests',
          name: 'Run tests',
          description: 'Confirm the build with tests.',
          status: 'NotStarted',
          subtasks: [],
        }],
      }],
    }];
    const expectedTask = tasks[0].subtasks![0];

    const { onClose, onSendMessage, onShowToast } = renderTaskPanel(tasks);

    expect(screen.getByText('Run tests')).toBeInTheDocument();

    fireEvent.click(screen.getByTitle('Play First Task'));

    await waitFor(() => {
      expect(onSendMessage).toHaveBeenCalledWith(
        `Please work on this task: ${expectedTask.name}\n${expectedTask.description}`,
        expectedTask.id,
      );
    });

    expect(onShowToast).toHaveBeenCalledWith(`Started task: ${expectedTask.name}`, 'info');
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('treats blocked tasks as actionable when choosing the first task to play', async () => {
    const tasks: TaskHierarchy = [
      {
        id: 'task-completed',
        name: 'Already done',
        description: 'This should be skipped.',
        status: 'Completed',
        subtasks: [],
      },
      {
        id: 'task-blocked',
        name: 'Waiting on approval',
        description: 'This should still be the first actionable task.',
        status: 'Blocked',
        subtasks: [],
      },
      {
        id: 'task-not-started',
        name: 'Later task',
        description: 'This should not be chosen first.',
        status: 'NotStarted',
        subtasks: [],
      },
    ];
    const expectedTask = tasks[1];

    const { onSendMessage } = renderTaskPanel(tasks);

    fireEvent.click(screen.getByTitle('Play First Task'));

    await waitFor(() => {
      expect(onSendMessage).toHaveBeenCalledWith(
        `Please work on this task: ${expectedTask.name}\n${expectedTask.description}`,
        expectedTask.id,
      );
    });
  });

  it('renders task descriptions as markdown in the task maintenance panel', () => {
    const tasks: TaskHierarchy = [{
      id: 'task-markdown',
      name: 'Review validation output',
      description: 'Check **failing crates** before rerunning `cargo test`.\n\n- inspect logs\n  - capture crate name',
      status: 'InProgress',
      subtasks: [],
    }];

    renderTaskPanel(tasks);

    const description = screen.getByText('failing crates').closest('.task-description');

    expect(description).not.toBeNull();
    expect(screen.getByText('failing crates').tagName).toBe('STRONG');
    expect(screen.getByText('cargo test').tagName).toBe('CODE');
    expect(screen.getByText('inspect logs').tagName).toBe('LI');
    expect(screen.getByText('capture crate name').tagName).toBe('LI');
  });

  it('keeps the runtime task highlighted without rendering the runtime summary', () => {
    const tasks: TaskHierarchy = [{
      id: 'verify-task',
      name: 'Verify facts',
      description: 'Cross-check the final details.',
      status: 'NotStarted',
      subtasks: [],
    }];
    const runtimeTaskSnapshot: TaskRuntimeSnapshot = {
      root_task_id: 'root-task',
      current_task: { id: 'verify-task', name: 'Verify facts', status: 'not_started' },
      ready_tasks: [{ id: 'verify-task', name: 'Verify facts', status: 'not_started' }],
      parallel_ready_tasks: [],
      blocked_tasks: [],
      open_tasks: [{ id: 'verify-task', name: 'Verify facts', status: 'not_started' }],
      completed_tasks: [],
      missing_requirements: ['verification still required'],
      status_message: 'Verification remains open',
    };

    renderTaskPanel(tasks, runtimeTaskSnapshot);

    expect(screen.queryByLabelText('Runtime task status')).not.toBeInTheDocument();
    expect(screen.queryByText('Runtime focus')).not.toBeInTheDocument();
    expect(screen.getByText('Verify facts').closest('.task-item')).toHaveClass('runtime-current');
  });

  it('highlights a linked task when opened from chat', () => {
    const originalScrollIntoView = window.HTMLElement.prototype.scrollIntoView;
    window.HTMLElement.prototype.scrollIntoView = vi.fn();
    const tasks: TaskHierarchy = [{
      id: 'task-root',
      name: 'Root task',
      description: 'Top level work.',
      status: 'InProgress',
      subtasks: [{
        id: 'task-focus',
        name: 'Focus task',
        description: 'Jump directly here.',
        status: 'NotStarted',
        subtasks: [],
      }],
    }];

    try {
      renderTaskPanel(tasks, null, 'task-focus');

      expect(screen.getByText('Focus task').closest('.task-item')).toHaveClass('linked-highlight');
    } finally {
      window.HTMLElement.prototype.scrollIntoView = originalScrollIntoView;
    }
  });

  it('waits for the panel transition before scrolling the linked task into view', () => {
    vi.useFakeTimers();
    const originalScrollIntoView = window.HTMLElement.prototype.scrollIntoView;
    const scrollIntoView = vi.fn();
    window.HTMLElement.prototype.scrollIntoView = scrollIntoView;

    const tasks: TaskHierarchy = [{
      id: 'task-root',
      name: 'Root task',
      description: 'Top level work.',
      status: 'InProgress',
      subtasks: [{
        id: 'task-focus',
        name: 'Focus task',
        description: 'Jump directly here.',
        status: 'NotStarted',
        subtasks: [],
      }],
    }];

    try {
      renderTaskPanel(tasks, null, 'task-focus');

      expect(scrollIntoView).not.toHaveBeenCalled();

      vi.advanceTimersByTime(219);
      expect(scrollIntoView).not.toHaveBeenCalled();

      vi.advanceTimersByTime(1);
      expect(scrollIntoView).toHaveBeenCalledWith({ block: 'center', behavior: 'smooth' });
    } finally {
      window.HTMLElement.prototype.scrollIntoView = originalScrollIntoView;
      vi.useRealTimers();
    }
  });
});

