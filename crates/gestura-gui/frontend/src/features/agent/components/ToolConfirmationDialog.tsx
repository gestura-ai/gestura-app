/**
 * ToolConfirmationDialog — shown when a tool requires user approval in Restricted mode.
 */
import React from 'react';
import type { ToolConfirmation, ToolConfirmationDecision } from '../types';

export interface ToolConfirmationDialogProps {
  confirmation: ToolConfirmation;
  onDecide: (decision: ToolConfirmationDecision) => void;
}

export const ToolConfirmationDialog: React.FC<ToolConfirmationDialogProps> = ({
  confirmation,
  onDecide,
}) => {
  const riskClass = confirmation.risk_level === 'high' ? 'risk-high'
    : confirmation.risk_level === 'medium' ? 'risk-medium' : 'risk-low';

  return (
    <div className="tool-confirmation-overlay" role="dialog" aria-modal="true" aria-labelledby="confirm-title">
      <div className="tool-confirmation-dialog">
        <div className="confirm-header">
          <span className="confirm-icon">🔐</span>
          <h3 id="confirm-title">Tool Permission Required</h3>
          {confirmation.risk_level && (
            <span className={`tool-badge ${riskClass}`}>{confirmation.risk_level} risk</span>
          )}
        </div>

        <div className="confirm-body">
          <p className="confirm-tool-name">
            <strong>{confirmation.tool_name}</strong>
            {confirmation.category && <span className="confirm-category"> · {confirmation.category}</span>}
          </p>
          {confirmation.description && (
            <p className="confirm-description">{confirmation.description}</p>
          )}
          {confirmation.tool_args && (
            <pre className="confirm-args">{confirmation.tool_args}</pre>
          )}
        </div>

        <div className="confirm-actions">
          <div className="confirm-actions-row confirm-actions-allow">
            <button type="button" className="btn-confirm allow-once" onClick={() => onDecide('allow_once')}>
              Allow Once
            </button>
            <button type="button" className="btn-confirm allow-session" onClick={() => onDecide('allow_session')}>
              Allow Session
            </button>
            <button type="button" className="btn-confirm allow-always" onClick={() => onDecide('allow_always')}>
              Always Allow
            </button>
          </div>
          <div className="confirm-actions-row confirm-actions-deny">
            <button type="button" className="btn-confirm deny-once" onClick={() => onDecide('deny_once')}>
              Deny Once
            </button>
            <button type="button" className="btn-confirm deny-session" onClick={() => onDecide('deny_session')}>
              Deny Session
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};

export default ToolConfirmationDialog;

