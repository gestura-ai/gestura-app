import React from 'react';
import ReactDOM from 'react-dom/client';
import AgentApp from '../features/agent/AgentApp';

class AgentRootErrorBoundary extends React.Component<
  { sessionId: string; children: React.ReactNode },
  { hasError: boolean }
> {
  public constructor(props: { sessionId: string; children: React.ReactNode }) {
    super(props);
    this.state = { hasError: false };
  }

  public static getDerivedStateFromError(): { hasError: boolean } {
    return { hasError: true };
  }

  public componentDidCatch(error: Error, errorInfo: React.ErrorInfo): void {
    console.error('[AgentRoot] failed to render:', error, errorInfo);
  }

  public render(): React.ReactNode {
    if (this.state.hasError) {
      return (
        <div style={{ padding: 24, color: '#f5f7fa', background: '#0b0f14', minHeight: '100vh' }}>
          <h2 style={{ marginTop: 0 }}>Agent session failed to load</h2>
          <p>Please close this window and try again.</p>
          <p style={{ opacity: 0.7, fontSize: 12 }}>Session: {this.props.sessionId || 'unknown'}</p>
        </div>
      );
    }

    return this.props.children;
  }
}

// Parse session_id from the URL query string supplied by window_manager.rs.
// e.g. agent_v2.html?session_id=abc123
const params = new URLSearchParams(window.location.search);
const sessionId = params.get('session_id') ?? '';

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <AgentRootErrorBoundary sessionId={sessionId}>
      <AgentApp sessionId={sessionId} />
    </AgentRootErrorBoundary>
  </React.StrictMode>
);

