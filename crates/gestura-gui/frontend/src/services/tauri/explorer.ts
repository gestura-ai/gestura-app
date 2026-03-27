import { invokeTauri } from './invoke';
import type {
  ExplorerRootResponse,
  ExplorerListDirResponse,
  ExplorerGitStatusResponse,
} from '../../features/agent/types';

/**
 * Returns the workspace root path and whether it is a git repository.
 *
 * IPC contract: `explorer_get_root` expects `{ session_id }`.
 */
export const explorerGetRoot = async (
  sessionId: string
): Promise<ExplorerRootResponse> => {
  return invokeTauri<ExplorerRootResponse>('explorer_get_root', {
    session_id: sessionId,
  });
};

/**
 * Lists directory entries for a path relative to the workspace root.
 *
 * IPC contract: `explorer_list_dir` expects `{ session_id, dir_rel }`.
 */
export const explorerListDir = async (
  sessionId: string,
  dirRel: string
): Promise<ExplorerListDirResponse> => {
  return invokeTauri<ExplorerListDirResponse>('explorer_list_dir', {
    session_id: sessionId,
    dir_rel: dirRel,
  });
};

/**
 * Opens the current session workspace root in the system file manager.
 *
 * IPC contract: `explorer_open_root_in_file_manager` expects `{ session_id }`.
 */
export const explorerOpenRootInFileManager = async (
  sessionId: string
): Promise<void> => {
  return invokeTauri<void>('explorer_open_root_in_file_manager', {
    session_id: sessionId,
  });
};

/**
 * Opens a workspace entry in the system file manager.
 *
 * Directories open directly. Files open their containing directory.
 *
 * IPC contract: `explorer_open_entry_in_file_manager` expects
 * `{ session_id, rel_path }`.
 */
export const explorerOpenEntryInFileManager = async (
  sessionId: string,
  relPath: string
): Promise<void> => {
  return invokeTauri<void>('explorer_open_entry_in_file_manager', {
    session_id: sessionId,
    rel_path: relPath,
  });
};

/**
 * Returns git status for all changed paths in the workspace.
 *
 * IPC contract: `explorer_git_status` expects `{ session_id }`.
 */
export const explorerGitStatus = async (
  sessionId: string
): Promise<ExplorerGitStatusResponse> => {
  return invokeTauri<ExplorerGitStatusResponse>('explorer_git_status', {
    session_id: sessionId,
  });
};

