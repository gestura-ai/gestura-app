import React, { useCallback, useEffect, useState } from 'react';
import {
  getRingStatus,
  pairRing as pairRingIpc,
  RingStatus,
  scanForRings as scanForRingsIpc,
  sendHapticFeedback,
  startGestureMonitoring,
  stopGestureMonitoring,
} from '../../services/tauri/ring';
import { Button } from '../../shared/components/Button';
import { FormGroup } from '../../shared/components/FormGroup';
import { PanelSection } from '../../shared/components/PanelSection';

const RingPanel: React.FC = () => {
  const [rings, setRings] = useState<string[]>([]);
  const [selectedRing, setSelectedRing] = useState<string>('');
  const [ringStatus, setRingStatus] = useState<RingStatus | null>(null);
  const [scanning, setScanning] = useState(false);
  const [monitoring, setMonitoring] = useState(false);

  const scanForRings = async () => {
    setScanning(true);
    try {
      const foundRings = await scanForRingsIpc();
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
      await pairRingIpc(selectedRing);
      await updateRingStatus();
    } catch (error) {
      console.error('Failed to pair ring:', error);
    }
  };

  const updateRingStatus = useCallback(async () => {
    if (!selectedRing) return;

    try {
      const status = await getRingStatus(selectedRing);
      setRingStatus(status);
    } catch (error) {
      console.error('Failed to get ring status:', error);
    }
  }, [selectedRing]);

  const sendHaptic = async (pattern: string, intensity: number = 0.7, duration: number = 100) => {
    if (!selectedRing) return;

    try {
      await sendHapticFeedback({
        device_id: selectedRing,
        pattern,
        intensity,
        duration_ms: duration,
      });
    } catch (error) {
      console.error('Failed to send haptic:', error);
    }
  };

  const toggleGestureMonitoring = async () => {
    if (!selectedRing) return;

    try {
      if (monitoring) {
        await stopGestureMonitoring(selectedRing);
      } else {
        await startGestureMonitoring(selectedRing);
      }
      setMonitoring(!monitoring);
    } catch (error) {
      console.error('Failed to toggle gesture monitoring:', error);
    }
  };

  useEffect(() => {
    if (selectedRing) {
      void updateRingStatus();
    }
  }, [selectedRing, updateRingStatus]);

  return (
    <div>
      <h2>Haptic Harmony Ring</h2>

      <PanelSection heading="Device Discovery">
        <div style={{ display: 'flex', gap: '0.5rem', marginBottom: '1rem' }}>
          <Button onClick={scanForRings} disabled={scanning}>
            {scanning ? 'Scanning...' : 'Scan for Rings'}
          </Button>
        </div>

        {rings.length > 0 && (
          <FormGroup label="Available Rings">
            <select value={selectedRing} onChange={(e) => setSelectedRing(e.target.value)}>
              {rings.map((ring) => (
                <option key={ring} value={ring}>
                  {ring}
                </option>
              ))}
            </select>
          </FormGroup>
        )}
      </PanelSection>

      {selectedRing && (
        <>
          <PanelSection heading="Ring Status">

            {ringStatus ? (
              <div>
                <p>
                  <strong>Device ID:</strong> {ringStatus.device_id}
                </p>
                <p>
                  <strong>Status:</strong>
                  <span className={`status-indicator ${ringStatus.is_connected ? 'status-connected' : 'status-disconnected'}`}></span>
                  {ringStatus.is_connected ? 'Connected' : 'Disconnected'}
                </p>
                {ringStatus.battery_level && (
                  <p>
                    <strong>Battery:</strong> {ringStatus.battery_level}%
                  </p>
                )}
                {ringStatus.firmware_version && (
                  <p>
                    <strong>Firmware:</strong> {ringStatus.firmware_version}</p>
                )}
                <p>
                  <strong>Last Seen:</strong> {new Date(ringStatus.last_seen).toLocaleString()}
                </p>
              </div>
            ) : (
              <p>Loading status...</p>
            )}

            <div style={{ display: 'flex', gap: '0.5rem', marginTop: '1rem' }}>
              <Button onClick={pairRing}>
                Pair Ring
              </Button>
              <Button tone="secondary" onClick={updateRingStatus}>
                Refresh Status
              </Button>
            </div>
          </PanelSection>

          <PanelSection heading="Haptic Testing">

            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: '0.5rem', marginBottom: '1rem' }}>
              <Button tone="secondary" onClick={() => sendHaptic('click')}>
                Click
              </Button>
              <Button tone="secondary" onClick={() => sendHaptic('pulse')}>
                Pulse
              </Button>
              <Button tone="secondary" onClick={() => sendHaptic('ramp')}>
                Ramp
              </Button>
            </div>

            <div style={{ display: 'flex', gap: '0.5rem' }}>
              <Button tone={monitoring ? 'secondary' : 'default'} onClick={toggleGestureMonitoring}>
                {monitoring ? 'Stop Monitoring' : 'Start Gesture Monitoring'}
              </Button>
            </div>
          </PanelSection>
        </>
      )}
    </div>
  );
};

export default RingPanel;
