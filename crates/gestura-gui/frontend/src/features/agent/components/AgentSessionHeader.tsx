import React from 'react';

import type { ToastKind } from '../hooks/useToast';
import { LlmProviderSelector } from './LlmProviderSelector';

interface AgentSessionHeaderProps {
  sessionId: string;
  onShowToast: (msg: string, kind?: ToastKind) => void;
  onOptionsOverlayOpenChange?: (open: boolean) => void;
}

export const AgentSessionHeader: React.FC<AgentSessionHeaderProps> = ({
  sessionId,
  onShowToast,
  onOptionsOverlayOpenChange,
}) => {
  return (
    <header className="agent-session-header" data-testid="agent-session-header">
      <div className="agent-session-header__controls">
        <LlmProviderSelector
          sessionId={sessionId}
          onShowToast={onShowToast}
          onHeaderDropdownOpenChange={onOptionsOverlayOpenChange}
          variant="header"
        />
      </div>
    </header>
  );
};
