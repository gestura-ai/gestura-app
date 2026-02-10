import { invokeTauri } from './invoke';

export interface PermissionStatus {
  id: string;
  name: string;
  description: string;
  instructions?: string;
  status: 'granted' | 'denied' | 'not_determined' | 'pending' | 'unknown';
  required: boolean;
}

export interface SystemPermissionsResult {
  permissions: PermissionStatus[];
  all_required_granted: boolean;
}

export const checkSystemPermissions = async (): Promise<SystemPermissionsResult> => {
  return await invokeTauri<SystemPermissionsResult>('check_system_permissions');
};

export const requestPermission = async (permissionId: string): Promise<void> => {
  await invokeTauri('request_permission', { permission: permissionId });
};

export const openSystemPreferences = async (pane: string): Promise<void> => {
  await invokeTauri('open_system_preferences', { pane });
};
