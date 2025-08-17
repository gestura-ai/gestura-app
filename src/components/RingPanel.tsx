import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface RingStatus {
  device_id: string;
  battery_level?: number;
  firmware_version?: string;
  is_connected: boolean;
  last_seen: string;
}

const RingPanel: React.FC = () => {
  const [rings, setRings] = useState<string[]>([]);
  const [selectedRing, setSelectedRing] = useState<string>('');
  const [ringStatus, setRingStatus] = useState<RingStatus | null>(null);
  const [scanning, setScanning] = useState(false);
  const [monitoring, setMonitoring] = useState(false);

  const scanForRings = async () => {
    setScanning(true);
    try {
      const foundRings = await invoke<string[]>('scan_for_rings');
      setRings(foundRings);
      if (foundRings.length > 0 && !selectedRing) {
        setSelectedRing(foundRings[0]);
      }
    } catch (error) {
      console.error('Failed to scan for rings:', error);
    } finally {
      setScanning(false);
    }
  };

  const pairRing = async () => {
    if (!selectedRing) return;
    
    try {
      await invoke('pair_ring', { deviceId: selectedRing });
      await updateRingStatus();
    } catch (error) {
      console.error('Failed to pair ring:', error);
    }
  };

  const updateRingStatus = async () => {
    if (!selectedRing) return;
    
    try {
      const status = await invoke<RingStatus | null>('get_ring_status', { deviceId: selectedRing });
      setRingStatus(status);
    } catch (error) {
      console.error('Failed to get ring status:', error);
    }
  };

  const sendHaptic = async (pattern: string, intensity: number = 0.7, duration: number = 100) => {
    if (!selectedRing) return;
    
    try {
      await invoke('send_haptic_feedback', {
        deviceId: selectedRing,
        pattern,
        intensity,
        durationMs: duration
      });
    } catch (error) {
      console.error('Failed to send haptic:', error);
    }
  };

  const toggleGestureMonitoring = async () => {
    if (!selectedRing) return;
    
    try {
      if (monitoring) {
        await invoke('stop_gesture_monitoring', { deviceId: selectedRing });
      } else {
        await invoke('start_gesture_monitoring', { deviceId: selectedRing });
      }
      setMonitoring(!monitoring);
    } catch (error) {
      console.error('Failed to toggle gesture monitoring:', error);
    }
  };

  useEffect(() => {
    if (selectedRing) {
      updateRingStatus();
    }
  }, [selectedRing]);

  return (
    <div>
      <h2>Haptic Harmony Ring</h2>
      
      <div className="panel">
        <h3>Device Discovery</h3>
        
        <div style={{ display: 'flex', gap: '0.5rem', marginBottom: '1rem' }}>
          <button 
            className="btn" 
            onClick={scanForRings}
            disabled={scanning}
          >
            {scanning ? 'Scanning...' : 'Scan for Rings'}
          </button>
        </div>

        {rings.length > 0 && (
          <div className="form-group">
            <label>Available Rings</label>
            <select 
              value={selectedRing} 
              onChange={(e) => setSelectedRing(e.target.value)}
            >
              {rings.map(ring => (
                <option key={ring} value={ring}>{ring}</option>
              ))}
            </select>
          </div>
        )}
      </div>

      {selectedRing && (
        <>
          <div className="panel">
            <h3>Ring Status</h3>
            
            {ringStatus ? (
              <div>
                <p><strong>Device ID:</strong> {ringStatus.device_id}</p>
                <p>
                  <strong>Status:</strong> 
                  <span className={`status-indicator ${ringStatus.is_connected ? 'status-connected' : 'status-disconnected'}`}></span>
                  {ringStatus.is_connected ? 'Connected' : 'Disconnected'}
                </p>
                {ringStatus.battery_level && (
                  <p><strong>Battery:</strong> {ringStatus.battery_level}%</p>
                )}
                {ringStatus.firmware_version && (
                  <p><strong>Firmware:</strong> {ringStatus.firmware_version}</p>
                )}
                <p><strong>Last Seen:</strong> {new Date(ringStatus.last_seen).toLocaleString()}</p>
              </div>
            ) : (
              <p>Loading status...</p>
            )}
            
            <div style={{ marginTop: '1rem' }}>
              <button className="btn" onClick={pairRing}>
                Pair Ring
              </button>
              <button className="btn btn-secondary" onClick={updateRingStatus} style={{ marginLeft: '0.5rem' }}>
                Refresh Status
              </button>
            </div>
          </div>

          <div className="panel">
            <h3>Haptic Testing</h3>
            
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: '0.5rem', marginBottom: '1rem' }}>
              <button className="btn btn-secondary" onClick={() => sendHaptic('click')}>
                Click
              </button>
              <button className="btn btn-secondary" onClick={() => sendHaptic('pulse')}>
                Pulse
              </button>
              <button className="btn btn-secondary" onClick={() => sendHaptic('ramp')}>
                Ramp
              </button>
            </div>
            
            <div style={{ display: 'flex', gap: '0.5rem' }}>
              <button 
                className={`btn ${monitoring ? 'btn-secondary' : ''}`}
                onClick={toggleGestureMonitoring}
              >
                {monitoring ? 'Stop Monitoring' : 'Start Gesture Monitoring'}
              </button>
            </div>
          </div>
        </>
      )}
    </div>
  );
};

export default RingPanel;
