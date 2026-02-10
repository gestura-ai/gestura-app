import { invokeTauri } from './invoke';

export interface SimulatorInfo {
  device_id: string;
  device_name: string;
  status: 'Healthy' | 'Degraded' | 'Offline' | { Error: string };
  last_health_check: string;
  connection_time: string;
  metrics: {
    latency_ms?: number;
    packet_loss_rate: number;
    uptime_seconds: number;
    haptic_commands_sent: number;
    gestures_received: number;
  };
}

export interface TestResults {
  connectivity: boolean;
  latency_ms: number;
  haptic_tests: Array<{
    pattern: string;
    success: boolean;
    error?: string;
  }>;
}

export const getSimulators = async (): Promise<Record<string, SimulatorInfo>> => {
  return await invokeTauri<Record<string, SimulatorInfo>>('get_simulators');
};

export const isDeveloperModeEnabled = async (): Promise<boolean> => {
  return await invokeTauri<boolean>('is_developer_mode_enabled');
};

export const scanForSimulators = async (): Promise<string[]> => {
  return await invokeTauri<string[]>('scan_for_simulators');
};

export const resetSimulator = async (deviceId: string): Promise<void> => {
  await invokeTauri('reset_simulator', { device_id: deviceId });
};

export const sendTestHaptic = async (deviceId: string, patternType: string): Promise<void> => {
  await invokeTauri('send_test_haptic', { device_id: deviceId, pattern_type: patternType });
};

export const runSimulatorTest = async (deviceId: string): Promise<TestResults> => {
  return await invokeTauri<TestResults>('run_simulator_test', { device_id: deviceId });
};

export const getSimulatorLogs = async (deviceId: string): Promise<string[]> => {
  return await invokeTauri<string[]>('get_simulator_logs', { device_id: deviceId });
};

export const toggleDeveloperMode = async (enabled: boolean): Promise<void> => {
  await invokeTauri('toggle_developer_mode', { enabled });
};
