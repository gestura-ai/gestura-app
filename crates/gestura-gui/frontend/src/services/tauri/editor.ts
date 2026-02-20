import { invokeTauri } from './invoke';
import type { EditorReadFileResponse, EditorGitDiffResponse } from '../../features/agent/types';

/**
 * Read a file's content for display in the editor.
 *
 * Returns the content, detected language, and file kind (text / image / binary).
 * IPC contract: `editor_read_file` expects `{ session_id, rel_path }`.
 */
export const editorReadFile = async (
  sessionId: string,
  relPath: string
): Promise<EditorReadFileResponse> => {
  return invokeTauri<EditorReadFileResponse>('editor_read_file', {
    session_id: sessionId,
    rel_path: relPath,
  });
};

/**
 * Write (save) a file's content to disk.
 *
 * IPC contract: `editor_write_file` expects `{ session_id, rel_path, content }`.
 */
export const editorWriteFile = async (
  sessionId: string,
  relPath: string,
  content: string
): Promise<void> => {
  return invokeTauri<void>('editor_write_file', {
    session_id: sessionId,
    rel_path: relPath,
    content,
  });
};

/**
 * Create a new file (or directory) at the given relative path.
 *
 * IPC contract: `editor_create_file` expects `{ session_id, rel_path, is_dir }`.
 */
export const editorCreateFile = async (
  sessionId: string,
  relPath: string,
  isDir = false
): Promise<void> => {
  return invokeTauri<void>('editor_create_file', {
    session_id: sessionId,
    rel_path: relPath,
    is_dir: isDir,
  });
};

/**
 * Delete a file or directory at the given relative path.
 *
 * IPC contract: `editor_delete_file` expects `{ session_id, rel_path }`.
 */
export const editorDeleteFile = async (
  sessionId: string,
  relPath: string
): Promise<void> => {
  return invokeTauri<void>('editor_delete_file', {
    session_id: sessionId,
    rel_path: relPath,
  });
};

/**
 * Rename / move a file or directory.
 *
 * IPC contract: `editor_rename_file` expects `{ session_id, old_rel_path, new_rel_path }`.
 */
export const editorRenameFile = async (
  sessionId: string,
  oldRelPath: string,
  newRelPath: string
): Promise<void> => {
  return invokeTauri<void>('editor_rename_file', {
    session_id: sessionId,
    old_rel_path: oldRelPath,
    new_rel_path: newRelPath,
  });
};

/**
 * Fetch the git diff (original vs. working-tree) for a file.
 * Only available when the workspace is a git repository.
 *
 * IPC contract: `editor_git_diff` expects `{ session_id, rel_path }`.
 */
export const editorGitDiff = async (
  sessionId: string,
  relPath: string
): Promise<EditorGitDiffResponse> => {
  return invokeTauri<EditorGitDiffResponse>('editor_git_diff', {
    session_id: sessionId,
    rel_path: relPath,
  });
};

