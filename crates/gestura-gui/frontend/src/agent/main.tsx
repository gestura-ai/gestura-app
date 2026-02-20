import React from 'react';
import ReactDOM from 'react-dom/client';
import AgentApp from '../features/agent/AgentApp';

// Parse session_id from the URL query string supplied by window_manager.rs.
// e.g. agent_v2.html?session_id=abc123
const params = new URLSearchParams(window.location.search);
const sessionId = params.get('session_id') ?? '';

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <AgentApp sessionId={sessionId} />
  </React.StrictMode>
);

