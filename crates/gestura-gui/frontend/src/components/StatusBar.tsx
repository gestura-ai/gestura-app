import React, { useState, useEffect } from 'react';

interface StatusBarProps {}

const StatusBar: React.FC<StatusBarProps> = () => {
  const [ringConnected, setRingConnected] = useState(false);
  const [natsConnected, setNatsConnected] = useState(false);
  const [agentCount, setAgentCount] = useState(0);

  useEffect(() => {
    // TODO: Implement real status monitoring
    // For now, show mock status
    setRingConnected(false);
    setNatsConnected(true);
    setAgentCount(1);
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
