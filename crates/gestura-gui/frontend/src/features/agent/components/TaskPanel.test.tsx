import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

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

describe('TaskPanel', () => {
  it('passes the selected task id when starting a task from the panel', async () => {
    const onSendMessage = vi.fn().mockResolvedValue(undefined);
    const onRefreshTasks = vi.fn().mockResolvedValue(undefined);
    const onShowToast = vi.fn();
    const onClose = vi.fn();

    const tasks: TaskHierarchy = [[{
      id: 'task-hello-world',
      name: 'Create hello world Tauri app',
      description: 'Build a small Tauri GUI that renders hello world.',
      status: 'NotStarted',
    }, []]];

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

    fireEvent.click(screen.getByTitle('Play First Task'));

    await waitFor(() => {
      expect(onSendMessage).toHaveBeenCalledWith(
        'Please work on this task: Create hello world Tauri app\nBuild a small Tauri GUI that renders hello world.',
        'task-hello-world',
      );
    });

    expect(onShowToast).toHaveBeenCalledWith('Started task: Create hello world Tauri app', 'info');
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});

