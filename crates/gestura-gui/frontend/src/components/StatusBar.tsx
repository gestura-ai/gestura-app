import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface StatusBarProps { }

const StatusBar: React.FC<StatusBarProps> = () => {
  const [ringConnected, setRingConnected] = useState(false);
  const [natsConnected, setNatsConnected] = useState(false);
  const [agentCount, setAgentCount] = useState(0);

  useEffect(() => {
    let cancelled = false;

    const refresh = async () => {
      try {
        const [agentsRes, natsRes, ringsRes] = await Promise.allSettled([
          invoke<any>('list_agents'),
          invoke<boolean>('get_nats_status'),
          invoke<string[]>('scan_for_rings'),
        ]);

        if (cancelled) return;

        // Agent count
        if (agentsRes.status === 'fulfilled') {
          const count = typeof agentsRes.value?.count === 'number' ? agentsRes.value.count : 0;
          setAgentCount(count);
        } else {
          setAgentCount(0);
        }

        // NATS status
        if (natsRes.status === 'fulfilled') {
          setNatsConnected(Boolean(natsRes.value));
        } else {
          setNatsConnected(false);
        }

        // Ring status (connected if any discovered ring reports connected)
        if (ringsRes.status === 'fulfilled' && Array.isArray(ringsRes.value) && ringsRes.value.length > 0) {
          const statuses = await Promise.all(
            ringsRes.value.map((deviceId) =>
              invoke<any>('get_ring_status', { device_id: deviceId }).catch(() => null)
            )
          );
          if (cancelled) return;
          setRingConnected(statuses.some((s) => s && s.is_connected === true));
        } else {
          setRingConnected(false);
        }
      } catch {
        if (cancelled) return;
        setRingConnected(false);
        setNatsConnected(false);
        setAgentCount(0);
      }
    };

    // Initial load + periodic refresh.
    refresh();
    const interval = setInterval(refresh, 5000);

    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, []);

  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: '1rem', fontSize: '0.875rem' }}>
      <div style={{ display: 'flex', alignItems: 'center' }}>
        <span className={`status-indicator ${ringConnected ? 'status-connected' : 'status-disconnected'}`}></span>
        Ring
      </div>

      <div style={{ display: 'flex', alignItems: 'center' }}>
        <span className={`status-indicator ${natsConnected ? 'status-connected' : 'status-disconnected'}`}></span>
        NATS
      </div>

      <div style={{ color: 'var(--muted)' }}>
        {agentCount} agent{agentCount !== 1 ? 's' : ''}
      </div>
    </div>
  );
};

export default StatusBar;
