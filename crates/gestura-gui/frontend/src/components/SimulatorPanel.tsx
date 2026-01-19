import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface SimulatorInfo {
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

interface TestResults {
  connectivity: boolean;
  latency_ms: number;
  haptic_tests: Array<{
    pattern: string;
    success: boolean;
    error?: string;
  }>;
}

interface SimulatorPanelProps {
  onClose: () => void;
}

const SimulatorPanel: React.FC<SimulatorPanelProps> = ({ onClose }) => {
  const [simulators, setSimulators] = useState<Record<string, SimulatorInfo>>({});
  const [selectedSimulator, setSelectedSimulator] = useState<string>('');
  const [isScanning, setIsScanning] = useState(false);
  const [testResults, setTestResults] = useState<TestResults | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [developerMode, setDeveloperMode] = useState(false);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    loadSimulators();
    checkDeveloperMode();
  }, []);

  const loadSimulators = async () => {
    try {
      const sims = await invoke<Record<string, SimulatorInfo>>('get_simulators');
      setSimulators(sims);
      if (Object.keys(sims).length > 0 && !selectedSimulator) {
        setSelectedSimulator(Object.keys(sims)[0]);
      }
    } catch (error) {
      console.error('Failed to load simulators:', error);
    } finally {
      setLoading(false);
    }
  };

  const checkDeveloperMode = async () => {
    try {
      const enabled = await invoke<boolean>('is_developer_mode_enabled');
      setDeveloperMode(enabled);
    } catch (error) {
      console.error('Failed to check developer mode:', error);
    }
  };

  const scanForSimulators = async () => {
    setIsScanning(true);
    try {
      const found = await invoke<string[]>('scan_for_simulators');
      console.log('Found simulators:', found);
      await loadSimulators();
    } catch (error) {
      console.error('Failed to scan for simulators:', error);
    } finally {
      setIsScanning(false);
    }
  };

  const resetSimulator = async (deviceId: string) => {
    try {
      await invoke('reset_simulator', { device_id: deviceId });
      await loadSimulators();
    } catch (error) {
      console.error('Failed to reset simulator:', error);
    }
  };

  const sendTestHaptic = async (deviceId: string, patternType: string) => {
    try {
      await invoke('send_test_haptic', { device_id: deviceId, pattern_type: patternType });
    } catch (error) {
      console.error('Failed to send test haptic:', error);
    }
  };

  const runComprehensiveTest = async (deviceId: string) => {
    try {
      const results = await invoke<TestResults>('run_simulator_test', { device_id: deviceId });
      setTestResults(results);
    } catch (error) {
      console.error('Failed to run comprehensive test:', error);
    }
  };

  const loadLogs = async (deviceId: string) => {
    try {
      const logEntries = await invoke<string[]>('get_simulator_logs', { device_id: deviceId });
      setLogs(logEntries);
    } catch (error) {
      console.error('Failed to load logs:', error);
    }
  };

  const toggleDeveloperMode = async () => {
    try {
      await invoke('toggle_developer_mode', { enabled: !developerMode });
      setDeveloperMode(!developerMode);
    } catch (error) {
      console.error('Failed to toggle developer mode:', error);
    }
  };

  const getStatusColor = (status: SimulatorInfo['status']) => {
    if (status === 'Healthy') return 'text-green-600';
    if (status === 'Degraded') return 'text-yellow-600';
    if (status === 'Offline') return 'text-red-600';
    return 'text-red-600';
  };

  const getStatusIcon = (status: SimulatorInfo['status']) => {
    if (status === 'Healthy') return '🟢';
    if (status === 'Degraded') return '🟡';
    if (status === 'Offline') return '🔴';
    return '❌';
  };

  if (loading) {
    return (
      <div className="p-6">
        <div className="flex items-center justify-center">
          <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600"></div>
          <span className="ml-2">Loading simulators...</span>
        </div>
      </div>
    );
  }

  return (
    <div className="p-6 max-w-6xl mx-auto">
      <div className="flex justify-between items-center mb-6">
        <h2 className="text-2xl font-bold">Haptic Harmony Ring Simulators</h2>
        <div className="flex gap-2">
          <button
            onClick={toggleDeveloperMode}
            className={`px-4 py-2 rounded-lg ${developerMode
                ? 'bg-green-600 text-white'
                : 'bg-gray-200 text-gray-700'
              }`}
          >
            Developer Mode: {developerMode ? 'ON' : 'OFF'}
          </button>
          <button
            onClick={onClose}
            className="px-4 py-2 bg-gray-200 text-gray-700 rounded-lg hover:bg-gray-300"
          >
            Close
          </button>
        </div>
      </div>

      {!developerMode && (
        <div className="bg-yellow-50 border border-yellow-200 rounded-lg p-4 mb-6">
          <p className="text-yellow-800">
            Enable Developer Mode to access simulator features and debugging tools.
          </p>
        </div>
      )}

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* Simulator List */}
        <div className="bg-white rounded-lg shadow-md p-6">
          <div className="flex justify-between items-center mb-4">
            <h3 className="text-lg font-semibold">Connected Simulators</h3>
            <button
              onClick={scanForSimulators}
              disabled={isScanning}
              className="px-3 py-1 bg-blue-600 text-white rounded hover:bg-blue-700 disabled:opacity-50"
            >
              {isScanning ? 'Scanning...' : 'Scan'}
            </button>
          </div>

          {Object.keys(simulators).length === 0 ? (
            <p className="text-gray-500">No simulators found. Click "Scan" to discover simulators.</p>
          ) : (
            <div className="space-y-3">
              {Object.entries(simulators).map(([id, sim]) => (
                <div
                  key={id}
                  className={`p-3 border rounded-lg cursor-pointer ${selectedSimulator === id ? 'border-blue-500 bg-blue-50' : 'border-gray-200'
                    }`}
                  onClick={() => setSelectedSimulator(id)}
                >
                  <div className="flex items-center justify-between">
                    <div>
                      <div className="flex items-center gap-2">
                        <span>{getStatusIcon(sim.status)}</span>
                        <span className="font-medium">{sim.device_name}</span>
                      </div>
                      <p className="text-sm text-gray-500">{sim.device_id}</p>
                    </div>
                    <div className="text-right">
                      <p className={`text-sm font-medium ${getStatusColor(sim.status)}`}>
                        {typeof sim.status === 'string' ? sim.status : 'Error'}
                      </p>
                      <p className="text-xs text-gray-500">
                        Uptime: {Math.floor(sim.metrics.uptime_seconds / 60)}m
                      </p>
                    </div>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Simulator Controls */}
        {selectedSimulator && simulators[selectedSimulator] && (
          <div className="bg-white rounded-lg shadow-md p-6">
            <h3 className="text-lg font-semibold mb-4">
              Controls - {simulators[selectedSimulator].device_name}
            </h3>

            <div className="space-y-4">
              {/* Test Haptic Patterns */}
              <div>
                <h4 className="font-medium mb-2">Test Haptic Patterns</h4>
                <div className="grid grid-cols-2 gap-2">
                  {['connectivity', 'latency', 'intensity', 'duration', 'complex'].map((pattern) => (
                    <button
                      key={pattern}
                      onClick={() => sendTestHaptic(selectedSimulator, pattern)}
                      className="px-3 py-2 bg-green-600 text-white rounded hover:bg-green-700 text-sm"
                      disabled={!developerMode}
                    >
                      {pattern.charAt(0).toUpperCase() + pattern.slice(1)}
                    </button>
                  ))}
                </div>
              </div>

              {/* Simulator Actions */}
              <div>
                <h4 className="font-medium mb-2">Simulator Actions</h4>
                <div className="flex gap-2">
                  <button
                    onClick={() => resetSimulator(selectedSimulator)}
                    className="px-4 py-2 bg-orange-600 text-white rounded hover:bg-orange-700"
                    disabled={!developerMode}
                  >
                    Reset
                  </button>
                  <button
                    onClick={() => runComprehensiveTest(selectedSimulator)}
                    className="px-4 py-2 bg-purple-600 text-white rounded hover:bg-purple-700"
                    disabled={!developerMode}
                  >
                    Run Tests
                  </button>
                  <button
                    onClick={() => loadLogs(selectedSimulator)}
                    className="px-4 py-2 bg-gray-600 text-white rounded hover:bg-gray-700"
                    disabled={!developerMode}
                  >
                    View Logs
                  </button>
                </div>
              </div>
            </div>
          </div>
        )}
      </div>

      {/* Metrics and Test Results */}
      {selectedSimulator && simulators[selectedSimulator] && (
        <div className="mt-6 grid grid-cols-1 lg:grid-cols-2 gap-6">
          {/* Metrics */}
          <div className="bg-white rounded-lg shadow-md p-6">
            <h3 className="text-lg font-semibold mb-4">Performance Metrics</h3>
            <div className="space-y-2">
              <div className="flex justify-between">
                <span>Latency:</span>
                <span>{simulators[selectedSimulator].metrics.latency_ms?.toFixed(1) || 'N/A'} ms</span>
              </div>
              <div className="flex justify-between">
                <span>Packet Loss:</span>
                <span>{(simulators[selectedSimulator].metrics.packet_loss_rate * 100).toFixed(2)}%</span>
              </div>
              <div className="flex justify-between">
                <span>Haptic Commands:</span>
                <span>{simulators[selectedSimulator].metrics.haptic_commands_sent}</span>
              </div>
              <div className="flex justify-between">
                <span>Gestures Received:</span>
                <span>{simulators[selectedSimulator].metrics.gestures_received}</span>
              </div>
            </div>
          </div>

          {/* Test Results or Logs */}
          <div className="bg-white rounded-lg shadow-md p-6">
            <h3 className="text-lg font-semibold mb-4">
              {testResults ? 'Test Results' : 'Connection Logs'}
            </h3>

            {testResults ? (
              <div className="space-y-2">
                <div className="flex justify-between">
                  <span>Connectivity:</span>
                  <span className={testResults.connectivity ? 'text-green-600' : 'text-red-600'}>
                    {testResults.connectivity ? '✓ Pass' : '✗ Fail'}
                  </span>
                </div>
                <div className="flex justify-between">
                  <span>Latency:</span>
                  <span>{testResults.latency_ms.toFixed(1)} ms</span>
                </div>
                <div className="mt-4">
                  <h4 className="font-medium mb-2">Haptic Tests:</h4>
                  {testResults.haptic_tests.map((test, index) => (
                    <div key={index} className="flex justify-between text-sm">
                      <span>{test.pattern}:</span>
                      <span className={test.success ? 'text-green-600' : 'text-red-600'}>
                        {test.success ? '✓' : '✗'}
                      </span>
                    </div>
                  ))}
                </div>
              </div>
            ) : (
              <div className="space-y-1 max-h-40 overflow-y-auto">
                {logs.length > 0 ? (
                  logs.map((log, index) => (
                    <div key={index} className="text-sm text-gray-600 font-mono">
                      {log}
                    </div>
                  ))
                ) : (
                  <p className="text-gray-500">No logs available. Click "View Logs" to load.</p>
                )}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
};

export default SimulatorPanel;
