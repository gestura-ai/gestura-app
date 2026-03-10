//! GUI Control Tool
//!
//! Provides a stub tool that the agent can invoke to control the frontend UI.
//! When invoked, the backend validates the action and the frontend intercepts
//! the start of the tool to update its UI state seamlessly.
//!
//! # Tools
//! - `gui_control`: dispatches a synthetic event that the frontend uses for UI
//!   layout.

use crate::error::Result;
use serde::{Deserialize, Serialize};

/// Supported GUI actions
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GuiAction {
    ToggleViewMode,
    OpenExplorer,
    CloseExplorer,
    OpenChat,
    CloseChat,
    NavigateConfig,
}

/// Request payload for GUI control
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiControlRequest {
    pub action: GuiAction,
    pub target: Option<String>,
}

/// Response payload for GUI control
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiControlResponse {
    pub success: bool,
    pub action: GuiAction,
    pub message: String,
}

/// Process a GUI control request.
/// Since the actual UI mutation happens on the frontend when it intercepts
/// the tool call, the backend's job is simply to validate and return a success message.
pub async fn execute_gui_control(req: GuiControlRequest) -> Result<GuiControlResponse> {
    // In the future, we could add validation here.
    // For now, we just acknowledge receipt.
    let message = format!("Dispatched UI action '{:?}' successfully.", req.action);

    Ok(GuiControlResponse {
        success: true,
        action: req.action,
        message,
    })
}
