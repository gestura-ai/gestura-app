import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { TaskPanel } from './TaskPanel';
import type { TaskHierarchy } from '../types';

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

function renderTaskPanel(tasks: TaskHierarchy) {
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
      id: 'task-hello-world',
      name: 'Create hello world Tauri app',
      description: 'Build a small Tauri GUI that renders hello world.',
      status: 'Completed',
      subtasks: [{
        id: 'task-build-app',
        name: 'Build the Tauri app',
        description: 'Compile and verify the hello world app.',
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
});

