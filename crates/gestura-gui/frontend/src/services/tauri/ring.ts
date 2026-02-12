import { invokeTauri } from './invoke';

export interface RingStatus {
  device_id: string;
  battery_level?: number;
  firmware_version?: string;
  is_connected: boolean;
  last_seen: string;
}

export const scanForRings = async (): Promise<string[]> => {
  return await invokeTauri<string[]>('scan_for_rings');
};

export const pairRing = async (deviceId: string): Promise<void> => {
  await invokeTauri('pair_ring', { device_id: deviceId });
};

export const getRingStatus = async (deviceId: string): Promise<RingStatus | null> => {
  return await invokeTauri<RingStatus | null>('get_ring_status', { device_id: deviceId });
};

export const sendHapticFeedback = async (params: {
  device_id: string;
  pattern: string;
  intensity: number;
  duration_ms: number;
}): Promise<void> => {
  await invokeTauri('send_haptic_feedback', params);
};

export const startGestureMonitoring = async (deviceId: string): Promise<void> => {
  await invokeTauri('start_gesture_monitoring', { device_id: deviceId });
};

export const stopGestureMonitoring = async (deviceId: string): Promise<void> => {
  await invokeTauri('stop_gesture_monitoring', { device_id: deviceId });
};
