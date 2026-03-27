export const WORKSPACE_CHANGED_EVENT = 'gestura:workspace:changed';
export const WORKSPACE_ENTRY_RENAMED_EVENT = 'gestura:workspace:entry-renamed';

export interface WorkspaceEntryRenamedDetail {
  oldRelPath: string;
  newRelPath: string;
}

export function dispatchWorkspaceChanged(): void {
  window.dispatchEvent(new CustomEvent(WORKSPACE_CHANGED_EVENT));
}

export function dispatchWorkspaceEntryRenamed(detail: WorkspaceEntryRenamedDetail): void {
  window.dispatchEvent(new CustomEvent<WorkspaceEntryRenamedDetail>(WORKSPACE_ENTRY_RENAMED_EVENT, { detail }));
  dispatchWorkspaceChanged();
}