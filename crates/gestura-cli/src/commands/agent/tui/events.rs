//! Event handling for the TUI
//!
//! This module handles keyboard events and maps them to actions
//! based on the current application mode.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};

use gestura_core::tool_confirmation::{TOOL_CONFIRMATIONS, ToolConfirmationDecision};

use super::app::{Action, ConfirmAction, PendingToolConfirmation, TuiApp, TuiMode};

/// Handle an event and return the appropriate action
pub fn handle_event(app: &mut TuiApp, event: Event) -> Action {
    match event {
        Event::Key(key) => handle_key_event(app, key),
        Event::Mouse(mouse) => handle_mouse_event(app, mouse),
        Event::Paste(text) => handle_paste_event(app, text),
        Event::Resize(_, _) => Action::Continue, // Terminal will re-render automatically
        _ => Action::Continue,
    }
}

fn handle_paste_event(app: &mut TuiApp, text: String) -> Action {
    if text.is_empty() {
        return Action::Continue;
    }

    // Normalize newlines. Some terminals send CRLF.
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");

    match app.mode {
        TuiMode::Insert => {
            app.insert_str(&normalized);

            // If paste starts a command, switch into command mode for consistent UX.
            if app.input.starts_with('/') {
                app.mode = TuiMode::Command;
                app.update_command_suggestions();
            }
        }
        TuiMode::Command => {
            app.insert_str(&normalized);
            app.update_command_suggestions();
        }
        TuiMode::Search => {
            for ch in normalized.chars() {
                if ch == '\n' {
                    continue;
                }
                app.search_insert_char(ch);
            }
        }
        _ => {}
    }

    Action::Continue
}

/// Handle keyboard events
fn handle_key_event(app: &mut TuiApp, key: KeyEvent) -> Action {
    // Global keybindings (work in any mode)
    match key.code {
        // Ctrl+C: copy selection if one exists, otherwise quit
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.selection_anchor.is_some() {
                return Action::CopySelection;
            }
            return Action::Quit;
        }
        // Ctrl+Q always quits
        KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return Action::Quit;
        }
        // Ctrl+R toggles recording
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return Action::ToggleRecording;
        }
        // Ctrl+T cycles theme
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cycle_theme();
            return Action::Continue;
        }
        _ => {}
    }

    // Mode-specific handling
    match app.mode {
        TuiMode::Normal => handle_normal_mode(app, key),
        TuiMode::Insert => handle_insert_mode(app, key),
        TuiMode::Command => handle_command_mode(app, key),
        TuiMode::Help => handle_help_mode(app, key),
        TuiMode::Confirm => handle_confirm_mode(app, key),
        TuiMode::ToolConfirm => handle_tool_confirm_mode(app, key),
        TuiMode::Search => handle_search_mode(app, key),
        TuiMode::ModelPicker => handle_model_picker_mode(app, key),
        TuiMode::Activity => handle_activity_mode(app, key),
        TuiMode::Settings => handle_settings_mode(app, key),
        TuiMode::Workflows => handle_workflows_mode(app, key),
        TuiMode::Tools => handle_tools_mode(app, key),
        TuiMode::Capabilities => handle_capabilities_mode(app, key),
        TuiMode::Mcp => handle_mcp_browser_mode(app, key),
        TuiMode::Knowledge => handle_knowledge_browser_mode(app, key),
        TuiMode::Hooks => handle_hooks_browser_mode(app, key),
        TuiMode::Agent => handle_agent_browser_mode(app, key),
        TuiMode::Memory => handle_memory_browser_mode(app, key),
        TuiMode::Devices => handle_devices_browser_mode(app, key),
        TuiMode::Permissions => handle_permissions_browser_mode(app, key),
        TuiMode::Sessions => handle_sessions_browser_mode(app, key),
        TuiMode::Tasks => handle_tasks_browser_mode(app, key),
        TuiMode::Themes => handle_themes_browser_mode(app, key),
    }
}

/// Handle ToolConfirm mode keys (scoped tool confirmation decisions).
///
/// Key mapping (Claude Code-like):
/// - 1: allow once
/// - 2: allow for this session
/// - 3: allow always
/// - 4: deny once
/// - 5: deny for this session
/// - Esc/q: deny once
fn handle_tool_confirm_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    let decision = match key.code {
        KeyCode::Char('1') => ToolConfirmationDecision::AllowOnce,
        KeyCode::Char('2') => ToolConfirmationDecision::AllowSession,
        KeyCode::Char('3') => ToolConfirmationDecision::AllowAlways,
        KeyCode::Char('4') => ToolConfirmationDecision::DenyOnce,
        KeyCode::Char('5') => ToolConfirmationDecision::DenySession,
        KeyCode::Esc | KeyCode::Char('q') => ToolConfirmationDecision::DenyOnce,
        _ => return Action::Continue,
    };

    let Some(PendingToolConfirmation {
        confirmation_id,
        tool_name,
        ..
    }) = app.take_tool_confirmation()
    else {
        app.set_status("No pending tool confirmation");
        return Action::Continue;
    };

    if let Err(e) = TOOL_CONFIRMATIONS.resolve_decision(
        &confirmation_id,
        Some(app.session.id.as_str()),
        decision,
    ) {
        app.set_error(format!("Failed to resolve tool confirmation: {e}"));
    } else {
        app.set_status(format!("Tool confirmation resolved: {tool_name}"));
    }

    Action::Continue
}

/// Handle ModelPicker overlay keys.
///
/// This overlay is Claude Code-like: type to filter, use arrows to select, Enter to apply.
fn handle_model_picker_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            app.mode = TuiMode::Insert;
            Action::Continue
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.model_picker_state.select_prev();
            Action::Continue
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.model_picker_state.select_next();
            Action::Continue
        }
        KeyCode::Enter => {
            if let Some(item) = app.model_picker_state.selected_item() {
                Action::ExecuteCommand(format!("/model {}", item.label))
            } else {
                let typed = app.model_picker_state.query.trim();
                if typed.is_empty() {
                    Action::Continue
                } else {
                    Action::ExecuteCommand(format!("/model {}", typed))
                }
            }
        }
        KeyCode::Backspace => {
            app.model_picker_state.query.pop();
            app.model_picker_state.refilter();
            Action::Continue
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.model_picker_state.query.clear();
            app.model_picker_state.refilter();
            Action::Continue
        }
        KeyCode::Char(c) => {
            app.model_picker_state.query.push(c);
            app.model_picker_state.refilter();
            Action::Continue
        }
        _ => Action::Continue,
    }
}

/// Handle Activity overlay keys.
///
/// The activity transcript is a scrollable view used to display tool-call progress/results.
fn handle_activity_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.mode = TuiMode::Insert;
            Action::Continue
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.activity_state.scroll_up();
            Action::Continue
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.activity_state.scroll_down();
            Action::Continue
        }
        KeyCode::Char('G') => {
            app.activity_state.scroll_to_bottom();
            Action::Continue
        }
        KeyCode::Char('g') => {
            // Jump to top.
            app.activity_state.list_state.select(Some(0));
            app.activity_state.user_scrolled = true;
            Action::Continue
        }
        _ => Action::Continue,
    }
}

/// Handle Normal mode keys (navigation, commands)
fn handle_normal_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    match key.code {
        // Enter insert mode
        KeyCode::Char('i') => {
            app.mode = TuiMode::Insert;
            Action::Continue
        }
        // Insert at end (vim 'a')
        KeyCode::Char('a') => {
            app.mode = TuiMode::Insert;
            app.cursor_end();
            Action::Continue
        }
        // Insert at start of line (vim 'I')
        KeyCode::Char('I') => {
            app.mode = TuiMode::Insert;
            app.cursor_home();
            Action::Continue
        }
        // Insert at end of line (vim 'A')
        KeyCode::Char('A') => {
            app.mode = TuiMode::Insert;
            app.cursor_end();
            Action::Continue
        }
        // Open new line below (vim 'o') - just enter insert mode
        KeyCode::Char('o') => {
            app.mode = TuiMode::Insert;
            app.cursor_end();
            app.insert_char('\n');
            Action::Continue
        }
        // Start command mode
        KeyCode::Char('/') | KeyCode::Char(':') => {
            app.mode = TuiMode::Command;
            app.clear_input();
            app.insert_char('/');
            app.update_command_suggestions();
            Action::Continue
        }
        // Navigation
        KeyCode::Char('j') | KeyCode::Down => Action::ScrollDown,
        KeyCode::Char('k') | KeyCode::Up => Action::ScrollUp,
        KeyCode::PageUp => Action::PageUp,
        KeyCode::PageDown => Action::PageDown,
        // Copy selected message(s) to clipboard (vim 'y' = yank)
        KeyCode::Char('y') => Action::CopySelection,
        // Copy the focused assistant message's raw markdown to clipboard.
        //
        // This corresponds to the transcript's per-message "copy" overlay control.
        KeyCode::Char('c') => {
            // Only meaningful in the Agent transcript tab.
            if app.active_tab != 0 {
                return Action::Continue;
            }

            let line = app.message_list_state.selected().unwrap_or(0);
            if let Some(&msg_idx) = app.line_to_message_map.get(line)
                && matches!(
                    app.messages.get(msg_idx).map(|m| m.role.as_str()),
                    Some("assistant")
                )
            {
                return Action::CopyMessageRaw(msg_idx);
            }
            Action::Continue
        }
        // Vim motions for cursor in input (when in normal mode)
        KeyCode::Char('h') | KeyCode::Left => {
            app.cursor_left();
            Action::Continue
        }
        KeyCode::Char('l') | KeyCode::Right => {
            app.cursor_right();
            Action::Continue
        }
        KeyCode::Char('w') => {
            app.cursor_word_forward();
            Action::Continue
        }
        KeyCode::Char('b') => {
            app.cursor_word_backward();
            Action::Continue
        }
        KeyCode::Char('0') => {
            app.cursor_home();
            Action::Continue
        }
        KeyCode::Char('$') => {
            app.cursor_end();
            Action::Continue
        }
        KeyCode::Char('g') => {
            // gg goes to top (would need double-tap detection)
            app.message_list_state.select(Some(0));
            app.user_scrolled = true;
            Action::Continue
        }
        KeyCode::Char('G') => {
            app.scroll_to_bottom();
            Action::Continue
        }
        // Delete character under cursor (vim 'x')
        KeyCode::Char('x') => {
            app.delete_char_after();
            Action::Continue
        }
        // Delete to end of line (vim 'D')
        KeyCode::Char('D') => {
            app.delete_to_end();
            Action::Continue
        }
        // Clear line (vim 'dd' - simplified to single 'd')
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.clear_input();
            Action::Continue
        }
        // Tab switching
        KeyCode::Tab => {
            let next = (app.active_tab + 1) % app.tabs.len();
            Action::SwitchTab(next)
        }
        KeyCode::BackTab => {
            let prev = if app.active_tab == 0 {
                app.tabs.len() - 1
            } else {
                app.active_tab - 1
            };
            Action::SwitchTab(prev)
        }
        // Number keys for tabs
        KeyCode::Char('1') => Action::SwitchTab(0),
        KeyCode::Char('2') => Action::SwitchTab(1),
        KeyCode::Char('3') => Action::SwitchTab(2),
        KeyCode::Char('4') => Action::SwitchTab(3),
        // Help
        KeyCode::Char('?') | KeyCode::F(1) => Action::ToggleHelp,
        // Search - Ctrl+F starts search mode
        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.start_search();
            Action::Continue
        }
        // Navigate search matches
        KeyCode::Char('n') => {
            if !app.search_matches.is_empty() {
                app.next_match();
            }
            Action::Continue
        }
        KeyCode::Char('N') => {
            if !app.search_matches.is_empty() {
                app.prev_match();
            }
            Action::Continue
        }
        // Clear search with Escape when search is active
        KeyCode::Esc => {
            if !app.search_query.is_empty() {
                app.clear_search();
            }
            Action::Continue
        }
        // Quit
        KeyCode::Char('q') => Action::Quit,
        _ => Action::Continue,
    }
}

/// Apply the currently highlighted command suggestion while preserving any already-typed arguments.
///
/// The command palette suggestions can include placeholders like `"/session load <id>"`.
/// In command mode, users often type arguments (e.g., `"/session load last"`) while navigating
/// suggestions with the arrow keys. This helper replaces only the *base command* (first token)
/// with the highlighted suggestion's base, keeping the rest of the user's input intact.
fn apply_selected_command_suggestion_preserving_args(app: &mut TuiApp) {
    let Some((cmd, _)) = app.command_suggestions.get(app.command_selection) else {
        return;
    };

    let selected_base = cmd.split_whitespace().next().unwrap_or(cmd);

    let mut parts = app.input.split_whitespace();
    let _typed_base = parts.next();
    let args: Vec<&str> = parts.collect();

    if args.is_empty() {
        app.input = selected_base.to_string();
    } else {
        app.input = format!("{} {}", selected_base, args.join(" "));
    }
    app.cursor_pos = app.input.len();
}

/// Handle Insert mode keys (typing messages)
fn handle_insert_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    // While streaming or recording, keep the UI responsive. We still allow Esc to cancel.
    if (app.is_loading || app.voice_capture_in_progress) && key.code == KeyCode::Esc {
        return Action::Cancel;
    }

    match key.code {
        // Exit insert mode
        KeyCode::Esc => {
            app.mode = TuiMode::Normal;
            Action::Continue
        }
        // Send message
        KeyCode::Enter => {
            if key.modifiers.contains(KeyModifiers::SHIFT)
                || key.modifiers.contains(KeyModifiers::CONTROL)
            {
                // Multi-line: insert newline
                app.insert_char('\n');
                Action::Continue
            } else if app.is_loading {
                // Prevent accidental send (and input loss via take_input) while a stream is active.
                app.set_status("Still streaming… press Esc to cancel".to_string());
                Action::Continue
            } else if app.voice_capture_in_progress {
                app.set_status("Recording… press Esc to cancel".to_string());
                Action::Continue
            } else if !app.input.is_empty() {
                let input = app.take_input();
                // Check if it's a command
                if input.starts_with('/') {
                    Action::ExecuteCommand(input)
                } else {
                    Action::SendMessage(input)
                }
            } else if app.listening_mode {
                Action::ToggleRecording
            } else {
                Action::Continue
            }
        }
        // Cmd+K (Meta+K): Enhance prompt (takes precedence over Ctrl+K kill-to-end)
        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::META) => Action::EnhancePrompt,
        // Cmd+Z / Ctrl+Z: Undo enhancement
        KeyCode::Char('z')
            if (key.modifiers.contains(KeyModifiers::META)
                || key.modifiers.contains(KeyModifiers::CONTROL))
                && app.original_prompt.is_some() =>
        {
            // Restore original prompt
            if let Some(original) = app.original_prompt.take() {
                app.input = original;
                app.cursor_pos = app.input.len();
                app.set_status("Prompt restored");
            }
            Action::Continue
        }
        // Ctrl+A start of line
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cursor_home();
            Action::Continue
        }
        // Ctrl+E end of line
        KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.cursor_end();
            Action::Continue
        }
        // Ctrl+U clear line
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::ClearInput,
        // Ctrl+W delete word before cursor
        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.delete_word_before();
            Action::Continue
        }
        // Ctrl+K delete to end of line (only if not Meta)
        KeyCode::Char('k')
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::META) =>
        {
            app.delete_to_end();
            Action::Continue
        }
        // Ctrl+F start search mode
        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.start_search();
            Action::Continue
        }
        // Alt+B word backward (vim 'b')
        KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::ALT) => {
            app.cursor_word_backward();
            Action::Continue
        }
        // Alt+F word forward (vim 'w')
        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::ALT) => {
            app.cursor_word_forward();
            Action::Continue
        }
        // Character input (must come after Ctrl+key patterns)
        KeyCode::Char(c) => {
            app.insert_char(c);
            // Auto-switch to command mode if starting with /
            if app.input == "/" {
                app.mode = TuiMode::Command;
                app.update_command_suggestions();
            }
            Action::Continue
        }
        // Backspace
        KeyCode::Backspace => {
            app.delete_char_before();
            Action::Continue
        }
        // Delete
        KeyCode::Delete => {
            app.delete_char_after();
            Action::Continue
        }
        // Cursor movement
        KeyCode::Left => {
            app.cursor_left();
            Action::Continue
        }
        KeyCode::Right => {
            app.cursor_right();
            Action::Continue
        }
        KeyCode::Home => {
            app.cursor_home();
            Action::Continue
        }
        KeyCode::End => {
            app.cursor_end();
            Action::Continue
        }
        // Page-level scrolling while in insert mode
        KeyCode::PageUp => Action::PageUp,
        KeyCode::PageDown => Action::PageDown,
        _ => Action::Continue,
    }
}

/// Handle Command mode keys (slash commands)
fn handle_command_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    match key.code {
        // Cancel command
        KeyCode::Esc => {
            // First Esc dismisses the suggestion menu, preserving the user's input.
            // Second Esc exits command mode.
            if !app.command_suggestions.is_empty() {
                app.command_suggestions.clear();
            } else {
                app.mode = TuiMode::Insert;
            }
            Action::Continue
        }
        // Execute command
        KeyCode::Enter => {
            if !app.input.is_empty() {
                if !app.command_suggestions.is_empty() {
                    apply_selected_command_suggestion_preserving_args(app);
                }
                let input = app.take_input();
                app.add_to_command_history(&input);
                app.mode = TuiMode::Insert;
                app.command_suggestions.clear();
                Action::ExecuteCommand(input)
            } else {
                app.mode = TuiMode::Insert;
                app.command_suggestions.clear();
                Action::Continue
            }
        }
        // Character input
        KeyCode::Char(c) => {
            app.insert_char(c);
            app.update_command_suggestions();
            Action::Continue
        }
        // Backspace
        KeyCode::Backspace => {
            app.delete_char_before();
            // If we deleted the /, go back to insert mode
            if app.input.is_empty() || !app.input.starts_with('/') {
                app.mode = TuiMode::Insert;
                app.command_suggestions.clear();
            } else {
                app.update_command_suggestions();
            }
            Action::Continue
        }
        // Tab completion - apply selected suggestion
        KeyCode::Tab => {
            app.apply_command_suggestion();
            app.update_command_suggestions();
            Action::Continue
        }
        // Navigate suggestions with arrow keys, or command history if no suggestions
        KeyCode::Down => {
            if app.command_suggestions.is_empty() {
                app.next_command_history();
            } else {
                app.next_command_suggestion();
            }
            Action::Continue
        }
        KeyCode::Up => {
            if app.command_suggestions.is_empty() {
                app.prev_command_history();
            } else {
                app.prev_command_suggestion();
            }
            Action::Continue
        }
        _ => Action::Continue,
    }
}

/// Handle Help mode keys
fn handle_help_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    match key.code {
        // Any key closes help
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') | KeyCode::F(1) | KeyCode::Enter => {
            app.mode = TuiMode::Insert;
            Action::Continue
        }
        // Scroll help content
        KeyCode::Char('j') | KeyCode::Down => Action::ScrollDown,
        KeyCode::Char('k') | KeyCode::Up => Action::ScrollUp,
        _ => Action::Continue,
    }
}

/// Handle Capabilities mode keys (reference popup, Esc to close, scrollable)
fn handle_capabilities_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.mode = TuiMode::Insert;
            Action::Continue
        }
        // Scroll capabilities content
        KeyCode::Char('j') | KeyCode::Down => {
            let total_lines = app.capabilities_text.lines().count();
            if app.capabilities_scroll + 1 < total_lines {
                app.capabilities_scroll += 1;
            }
            Action::Continue
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.capabilities_scroll = app.capabilities_scroll.saturating_sub(1);
            Action::Continue
        }
        // Page-wise scrolling
        KeyCode::PageDown | KeyCode::Char('d') => {
            let total_lines = app.capabilities_text.lines().count();
            app.capabilities_scroll =
                (app.capabilities_scroll + 10).min(total_lines.saturating_sub(1));
            Action::Continue
        }
        KeyCode::PageUp | KeyCode::Char('u') => {
            app.capabilities_scroll = app.capabilities_scroll.saturating_sub(10);
            Action::Continue
        }
        // Home / End
        KeyCode::Char('g') => {
            app.capabilities_scroll = 0;
            Action::Continue
        }
        KeyCode::Char('G') => {
            let total_lines = app.capabilities_text.lines().count();
            app.capabilities_scroll = total_lines.saturating_sub(1);
            Action::Continue
        }
        _ => Action::Continue,
    }
}

/// Handle Confirm mode keys (confirmation dialogs)
fn handle_confirm_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    match key.code {
        // Confirm with y or Enter
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
            if let Some(action) = app.take_confirm() {
                match action {
                    ConfirmAction::QuitWithoutSave => {
                        app.skip_save_on_exit = true;
                        Action::Quit
                    }
                    ConfirmAction::ClearMessages => {
                        app.messages.clear();
                        app.activity_state.clear();
                        app.message_list_state.select(None);
                        app.set_status("Messages cleared");
                        Action::Continue
                    }
                    ConfirmAction::NewSession => {
                        // Signal to main loop to create new session
                        Action::ExecuteCommand("/new --confirmed".to_string())
                    }
                    ConfirmAction::ExecuteCommand { command, .. } => {
                        Action::ExecuteCommand(command)
                    }
                }
            } else {
                app.mode = TuiMode::Insert;
                Action::Continue
            }
        }
        // Cancel with n, Escape, or any other key
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.cancel_confirm();
            Action::Continue
        }
        _ => Action::Continue,
    }
}

/// Handle mouse events
fn handle_mouse_event(app: &mut TuiApp, mouse: MouseEvent) -> Action {
    let x = mouse.column;
    let y = mouse.row;

    let hit_copy_button = |x: u16, y: u16| {
        app.assistant_copy_buttons.iter().find_map(|hit| {
            let within = x >= hit.rect.x
                && x < hit.rect.x + hit.rect.width
                && y >= hit.rect.y
                && y < hit.rect.y + hit.rect.height;
            within.then_some(hit.message_index)
        })
    };

    match mouse.kind {
        MouseEventKind::Moved => {
            // Hover styling is only relevant for Agent-tab copy controls.
            if app.active_tab != 0 {
                app.hovered_copy_button = None;
                return Action::Continue;
            }

            app.hovered_copy_button = hit_copy_button(x, y);
            Action::Continue
        }
        MouseEventKind::ScrollUp => {
            // Cancel any in-progress "press" if the user scrolls.
            app.pressed_copy_button = None;
            match app.mode {
                TuiMode::Activity => app.activity_state.scroll_up(),
                TuiMode::ModelPicker => app.model_picker_state.select_prev(),
                TuiMode::Mcp => app.mcp_browser_state.select_prev(),
                TuiMode::Knowledge => app.knowledge_browser_state.select_prev(),
                TuiMode::Hooks => app.hooks_browser_state.select_prev(),
                TuiMode::Agent => app.agent_browser_state.select_prev(),
                TuiMode::Memory => app.memory_browser_state.select_prev(),
                TuiMode::Devices => app.devices_browser_state.select_prev(),
                TuiMode::Permissions => app.permissions_browser_state.select_prev(),
                TuiMode::Sessions => app.sessions_browser_state.select_prev(),
                TuiMode::Tasks => app.tasks_browser_state.select_prev(),
                TuiMode::Themes => app.themes_browser_state.select_prev(),
                TuiMode::ToolConfirm => {}
                _ => app.scroll_up(),
            }
            Action::Continue
        }
        MouseEventKind::ScrollDown => {
            // Cancel any in-progress "press" if the user scrolls.
            app.pressed_copy_button = None;
            match app.mode {
                TuiMode::Activity => app.activity_state.scroll_down(),
                TuiMode::ModelPicker => app.model_picker_state.select_next(),
                TuiMode::Mcp => app.mcp_browser_state.select_next(),
                TuiMode::Knowledge => app.knowledge_browser_state.select_next(),
                TuiMode::Hooks => app.hooks_browser_state.select_next(),
                TuiMode::Agent => app.agent_browser_state.select_next(),
                TuiMode::Memory => app.memory_browser_state.select_next(),
                TuiMode::Devices => app.devices_browser_state.select_next(),
                TuiMode::Permissions => app.permissions_browser_state.select_next(),
                TuiMode::Sessions => app.sessions_browser_state.select_next(),
                TuiMode::Tasks => app.tasks_browser_state.select_next(),
                TuiMode::Themes => app.themes_browser_state.select_next(),
                TuiMode::ToolConfirm => {}
                _ => app.scroll_down(),
            }
            Action::Continue
        }
        MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
            // Starting a new click cancels any previous press state.
            app.pressed_copy_button = None;

            // Check if click is in tabs area
            if let Some(tabs_area) = app.layout_areas.tabs
                && y >= tabs_area.y
                && y < tabs_area.y + tabs_area.height
                && !app.tabs.is_empty()
            {
                // Each tab is roughly equal width; clamp to 1 to avoid division-by-zero.
                let tab_width = (tabs_area.width / app.tabs.len() as u16).max(1);
                let tab_index = ((x.saturating_sub(tabs_area.x)) / tab_width) as usize;
                if tab_index < app.tabs.len() {
                    app.active_tab = tab_index;
                    return Action::SwitchTab(tab_index);
                }
            }

            // Check if click is in messages area — start selection anchor
            if let Some(msg_area) = app.layout_areas.messages
                && y >= msg_area.y
                && y < msg_area.y + msg_area.height
            {
                // First: if the user clicked a per-message "copy" overlay control (Agent tab
                // only), trigger the raw-copy action instead of starting a drag selection.
                if app.active_tab == 0
                    && let Some(message_index) = hit_copy_button(x, y)
                {
                    // Ensure the overlay doesn't interfere with selection-copy.
                    app.selection_anchor = None;
                    app.selection_end = None;

                    // Visual pressed feedback: dim while mouse is held.
                    app.pressed_copy_button = Some(message_index);
                    app.hovered_copy_button = Some(message_index);
                    return Action::Continue;
                }

                // Clicking elsewhere in the transcript clears hover state.
                app.hovered_copy_button = None;

                let relative_y = (y - msg_area.y) as usize;
                let offset = app.message_list_state.offset();
                let line_index = offset + relative_y;
                if line_index < app.rendered_line_count {
                    app.message_list_state.select(Some(line_index));
                    app.user_scrolled = true;
                    // Begin selection drag
                    app.selection_anchor = Some(line_index);
                    app.selection_end = Some(line_index);
                }
            }

            // Check if click is in input area - switch to insert mode
            if let Some(input_area) = app.layout_areas.input
                && y >= input_area.y
                && y < input_area.y + input_area.height
            {
                app.mode = TuiMode::Insert;
                // Clear any message selection on input click and re-enable
                // auto-scroll so the next message/stream snaps to the bottom.
                app.selection_anchor = None;
                app.selection_end = None;
                app.user_scrolled = false;
                app.hovered_copy_button = None;
                app.pressed_copy_button = None;
            }

            Action::Continue
        }
        MouseEventKind::Drag(crossterm::event::MouseButton::Left) => {
            // If a copy button is pressed, don't extend selection. Instead, keep hover updated
            // so releasing over the button still counts as an activation.
            if app.pressed_copy_button.is_some() {
                if app.active_tab == 0 {
                    app.hovered_copy_button = hit_copy_button(x, y);
                } else {
                    app.hovered_copy_button = None;
                }
                return Action::Continue;
            }

            // Extend selection as the mouse drags
            if let Some(msg_area) = app.layout_areas.messages
                && y >= msg_area.y
                && y < msg_area.y + msg_area.height
                && app.selection_anchor.is_some()
            {
                let relative_y = (y - msg_area.y) as usize;
                let offset = app.message_list_state.offset();
                let line_index =
                    (offset + relative_y).min(app.rendered_line_count.saturating_sub(1));
                app.selection_end = Some(line_index);
            }
            Action::Continue
        }
        MouseEventKind::Up(crossterm::event::MouseButton::Left) => {
            // If we were pressing a copy button, trigger on release (not on mouse-down).
            if let Some(pressed_message_index) = app.pressed_copy_button.take() {
                // Recompute hover on release in case the terminal doesn't emit `Moved`.
                if app.active_tab == 0 {
                    app.hovered_copy_button = hit_copy_button(x, y);
                } else {
                    app.hovered_copy_button = None;
                }

                if app.active_tab == 0 && app.hovered_copy_button == Some(pressed_message_index) {
                    app.selection_anchor = None;
                    app.selection_end = None;
                    return Action::CopyMessageRaw(pressed_message_index);
                }

                return Action::Continue;
            }

            // Auto-copy to clipboard when the user releases the mouse after a drag
            // selection spanning more than one line.
            if let (Some(anchor), Some(end)) = (app.selection_anchor, app.selection_end)
                && anchor != end
            {
                return Action::CopySelection;
            }
            Action::Continue
        }
        _ => Action::Continue,
    }
}

/// Handle Search mode keys
fn handle_search_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    match key.code {
        // Cancel search
        KeyCode::Esc => {
            app.cancel_search();
            Action::Continue
        }
        // Confirm search and return to normal mode
        KeyCode::Enter => {
            app.confirm_search();
            Action::Continue
        }
        // Backspace removes last character
        KeyCode::Backspace => {
            app.search_backspace();
            Action::Continue
        }
        // Navigate matches while in search mode
        KeyCode::Down | KeyCode::Tab => {
            app.next_match();
            Action::Continue
        }
        KeyCode::Up | KeyCode::BackTab => {
            app.prev_match();
            Action::Continue
        }
        // Toggle filter mode
        KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.toggle_search_filter();
            Action::Continue
        }
        // Type characters into search query
        KeyCode::Char(c) => {
            app.search_insert_char(c);
            Action::Continue
        }
        _ => Action::Continue,
    }
}
/// Handle keys when in Settings mode
fn handle_settings_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    use super::app::SettingsField;

    // Escape returns to agent
    if key.code == KeyCode::Esc && !app.settings_state.is_editing {
        app.active_tab = 0; // Return to agent tab
        app.mode = TuiMode::Insert;
        app.set_status("Returned to agent");
        return Action::Continue;
    }

    // If currently editing a field
    if app.settings_state.is_editing {
        match key.code {
            KeyCode::Esc => {
                app.settings_state.is_editing = false;
                app.set_status("Cancelled edit");
            }
            KeyCode::Enter => {
                // Save value
                let new_value = app.settings_state.edit_buffer.clone();
                if let Some(field) = SettingsField::from_index(app.settings_state.selected_field) {
                    match field {
                        SettingsField::Provider => app.config.llm.primary = new_value,
                        SettingsField::Model => match app.config.llm.primary.as_str() {
                            "openai" => {
                                if let Some(ref mut o) = app.config.llm.openai {
                                    o.model = new_value;
                                }
                            }
                            "anthropic" => {
                                if let Some(ref mut a) = app.config.llm.anthropic {
                                    a.model = new_value;
                                }
                            }
                            _ => {}
                        },
                        SettingsField::SystemPrompt => app.system_prompt = Some(new_value),
                        SettingsField::Temperature => {} // Placeholder
                    }
                    app.set_status("Settings saved");
                }
                app.settings_state.is_editing = false;
            }
            KeyCode::Backspace => {
                app.settings_state.edit_buffer.pop();
            }
            KeyCode::Char(c) => {
                app.settings_state.edit_buffer.push(c);
            }
            _ => {}
        }
        return Action::Continue;
    }

    // Navigation mode
    match key.code {
        KeyCode::Down | KeyCode::Char('j') => {
            app.settings_state.selected_field =
                (app.settings_state.selected_field + 1) % SettingsField::COUNT;
            Action::Continue
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.settings_state.selected_field = if app.settings_state.selected_field == 0 {
                SettingsField::COUNT - 1
            } else {
                app.settings_state.selected_field - 1
            };
            Action::Continue
        }
        KeyCode::Enter => {
            // Start editing
            app.settings_state.is_editing = true;
            // Pre-fill buffer
            app.settings_state.edit_buffer =
                match SettingsField::from_index(app.settings_state.selected_field) {
                    Some(SettingsField::Provider) => app.config.llm.primary.clone(),
                    Some(SettingsField::Model) => app.model_name().to_string(),
                    Some(SettingsField::SystemPrompt) => {
                        app.system_prompt.clone().unwrap_or_default()
                    }
                    Some(SettingsField::Temperature) => "0.7".to_string(),
                    None => String::new(),
                };
            Action::Continue
        }
        _ => Action::Continue,
    }
}

/// Handle keys when in Workflows mode
fn handle_workflows_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            app.active_tab = 0; // Return to agent tab
            app.mode = TuiMode::Insert;
            app.set_status("Returned to agent");
            Action::Continue
        }
        KeyCode::Down | KeyCode::Char('j') => Action::ScrollDown,
        KeyCode::Up | KeyCode::Char('k') => Action::ScrollUp,
        KeyCode::Enter => {
            // TODO: Implement workflow selection and execution
            // For now, just show a message
            app.set_status("Workflow execution not yet implemented in modal mode");
            Action::Continue
        }
        _ => Action::Continue,
    }
}

/// Handle keys when in Tools mode.
///
/// The tools view has two sub-modes:
/// - **List mode** (default): ↑/↓ to navigate, Enter to open detail, Space to toggle enable/disable, Esc to return to agent.
/// - **Detail mode**: shows a detail pane for the selected tool. Esc returns to the list.
fn handle_tools_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    let tool_count = gestura_core::tools::all_tools().len();

    if app.tools_state.detail_mode {
        // Detail sub-mode — Esc returns to list, Space toggles enable/disable.
        match key.code {
            KeyCode::Esc => {
                app.tools_state.detail_mode = false;
                app.set_status("Tools: ↑/↓ navigate, Enter details, Space toggle, Esc close");
                Action::Continue
            }
            KeyCode::Char(' ') => {
                toggle_selected_tool(app);
                Action::Continue
            }
            _ => Action::Continue,
        }
    } else {
        // List sub-mode.
        match key.code {
            KeyCode::Esc => {
                app.active_tab = 0; // Return to agent tab
                app.mode = TuiMode::Insert;
                app.set_status("Returned to agent");
                Action::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.tools_state.select_next(tool_count);
                Action::Continue
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.tools_state.select_prev(tool_count);
                Action::Continue
            }
            KeyCode::Enter => {
                app.tools_state.detail_mode = true;
                let tools = gestura_core::tools::all_tools();
                if let Some(t) = tools.get(app.tools_state.selected_index) {
                    app.set_status(format!(
                        "Tool: {} — Space to toggle, Esc to go back",
                        t.name
                    ));
                }
                Action::Continue
            }
            KeyCode::Char(' ') => {
                toggle_selected_tool(app);
                Action::Continue
            }
            _ => Action::Continue,
        }
    }
}

/// Handle keys when in MCP browser mode.
///
/// The MCP browser has two sub-modes:
/// - **List mode** (default): ↑/↓ navigate, Enter detail, Space toggle enable/disable, Esc close.
/// - **Detail mode**: shows server details. Esc returns to list, Space toggles.
fn handle_mcp_browser_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    if app.mcp_browser_state.detail_mode {
        match key.code {
            KeyCode::Esc => {
                app.mcp_browser_state.detail_mode = false;
                app.set_status(
                    "MCP: ↑/↓ navigate  Enter details  n add  Space toggle  c connect  d disconnect  x remove  Esc close",
                );
                Action::Continue
            }
            KeyCode::Char(' ') => {
                // Route enable/disable through the canonical slash command path.
                let idx = app.mcp_browser_state.selected_index;
                if let Some(entry) = app.mcp_browser_state.servers.get(idx) {
                    let verb = if entry.entry.enabled {
                        "disable"
                    } else {
                        "enable"
                    };
                    return Action::ExecuteCommand(format!("/mcp {verb} {}", entry.entry.name));
                }
                Action::Continue
            }
            _ => Action::Continue,
        }
    } else {
        match key.code {
            KeyCode::Esc => {
                app.mode = TuiMode::Insert;
                app.set_status("Returned to agent");
                Action::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.mcp_browser_state.select_next();
                Action::Continue
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.mcp_browser_state.select_prev();
                Action::Continue
            }
            KeyCode::Enter => {
                // Avoid switching into an empty detail pane when there are no servers.
                if app.mcp_browser_state.servers.is_empty() {
                    app.input = "/mcp add ".to_string();
                    app.cursor_pos = app.input.len();
                    app.mode = TuiMode::Command;
                    app.set_status("Add MCP server: /mcp add <name> <cmd_or_url> (then Enter)");
                    Action::Continue
                } else {
                    if let Some(entry) = app
                        .mcp_browser_state
                        .servers
                        .get(app.mcp_browser_state.selected_index)
                    {
                        app.set_status(format!(
                            "MCP: {} — Space toggle, Esc back",
                            entry.entry.name
                        ));
                    }
                    app.mcp_browser_state.detail_mode = true;
                    Action::Continue
                }
            }
            KeyCode::Char('n') => {
                app.input = "/mcp add ".to_string();
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Add MCP server: /mcp add <name> <cmd_or_url> (then Enter)");
                Action::Continue
            }
            KeyCode::Char(' ') => {
                // Route enable/disable through the canonical slash command path.
                let idx = app.mcp_browser_state.selected_index;
                if let Some(entry) = app.mcp_browser_state.servers.get(idx) {
                    let verb = if entry.entry.enabled {
                        "disable"
                    } else {
                        "enable"
                    };
                    return Action::ExecuteCommand(format!("/mcp {verb} {}", entry.entry.name));
                }
                Action::Continue
            }
            KeyCode::Char('c') => {
                // Connect to selected server
                let idx = app.mcp_browser_state.selected_index;
                if let Some(entry) = app.mcp_browser_state.servers.get(idx) {
                    let name = entry.entry.name.clone();
                    return Action::ExecuteCommand(format!("/mcp connect {}", name));
                }
                Action::Continue
            }
            KeyCode::Char('d') => {
                // Disconnect from selected server
                let idx = app.mcp_browser_state.selected_index;
                if let Some(entry) = app.mcp_browser_state.servers.get(idx) {
                    let name = entry.entry.name.clone();
                    return Action::ExecuteCommand(format!("/mcp disconnect {}", name));
                }
                Action::Continue
            }
            KeyCode::Char('x') => {
                // Remove selected server
                let idx = app.mcp_browser_state.selected_index;
                if let Some(entry) = app.mcp_browser_state.servers.get(idx) {
                    let name = entry.entry.name.clone();
                    return Action::ExecuteCommand(format!("/mcp remove {}", name));
                }
                Action::Continue
            }
            _ => Action::Continue,
        }
    }
}

/// Handle keys when in Knowledge browser mode.
///
/// The knowledge browser has two sub-modes:
/// - **List mode**: ↑/↓ navigate, Enter detail, Space toggle, Esc close.
/// - **Detail mode**: shows item details. Esc returns to list, Space toggles.
fn handle_knowledge_browser_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    if app.knowledge_browser_state.detail_mode {
        match key.code {
            KeyCode::Esc => {
                app.knowledge_browser_state.detail_mode = false;
                app.set_status("Knowledge: ↑/↓ navigate  Enter details  Space toggle  Esc close");
                Action::Continue
            }
            KeyCode::Char(' ') => {
                toggle_selected_knowledge(app);
                Action::Continue
            }
            _ => Action::Continue,
        }
    } else {
        match key.code {
            KeyCode::Esc => {
                app.mode = TuiMode::Insert;
                app.set_status("Returned to agent");
                Action::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.knowledge_browser_state.select_next();
                Action::Continue
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.knowledge_browser_state.select_prev();
                Action::Continue
            }
            KeyCode::Enter => {
                if let Some(item) = app
                    .knowledge_browser_state
                    .items
                    .get(app.knowledge_browser_state.selected_index)
                {
                    app.set_status(format!("Knowledge: {} — Space toggle, Esc back", item.name));
                }
                app.knowledge_browser_state.detail_mode = true;
                Action::Continue
            }
            KeyCode::Char(' ') => {
                toggle_selected_knowledge(app);
                Action::Continue
            }
            _ => Action::Continue,
        }
    }
}

/// Toggle the enabled/disabled state of the currently selected knowledge item.
///
/// Persists the change via [`KnowledgeSettingsManager`] (session-scoped, on disk)
/// instead of the in-memory-only [`KnowledgeStore::set_enabled`].
fn toggle_selected_knowledge(app: &mut TuiApp) {
    let idx = app.knowledge_browser_state.selected_index;
    let Some(item) = app.knowledge_browser_state.items.get(idx) else {
        return;
    };
    let id = item.id.clone();
    let new_enabled = !item.enabled;

    // Persist via KnowledgeSettingsManager (session-scoped, on disk).
    let session_id = &app.session.id;
    let settings_mgr = gestura_core::knowledge::KnowledgeSettingsManager::new(
        dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from(".")),
    );
    let _ = settings_mgr.set_knowledge_enabled(session_id, &id, new_enabled);

    // Update cached state.
    if let Some(item) = app.knowledge_browser_state.items.get_mut(idx) {
        item.enabled = new_enabled;
    }

    let label = if new_enabled { "enabled" } else { "disabled" };
    app.set_status(format!("Knowledge '{}' {}", id, label));
}

/// Toggle the enabled/disabled state of the currently selected tool in session settings.
fn toggle_selected_tool(app: &mut TuiApp) {
    let tools = gestura_core::tools::all_tools();
    let Some(tool) = tools.get(app.tools_state.selected_index) else {
        return;
    };
    let tool_name = tool.name.to_string();

    let settings = app
        .session
        .state
        .tool_settings
        .get_or_insert_with(Default::default);

    let currently_enabled = settings
        .enabled_tools
        .get(&tool_name)
        .copied()
        .unwrap_or(false);
    settings
        .enabled_tools
        .insert(tool_name.clone(), !currently_enabled);

    // Persist the change to disk.
    let _ = super::super::save_cli_session(&app.session);

    let label = if !currently_enabled {
        "enabled"
    } else {
        "disabled"
    };
    app.set_status(format!("Tool '{}' {}", tool_name, label));
}

/// Handle keyboard events in the Hooks browser overlay.
fn handle_hooks_browser_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    if app.hooks_browser_state.detail_mode {
        match key.code {
            KeyCode::Esc => {
                app.hooks_browser_state.detail_mode = false;
                app.set_status(
                    "Hooks: ↑/↓ navigate  Enter details  Space toggle  n new  e edit  x delete  a allow+  r allow-  t timeout  m max  Esc close",
                );
                Action::Continue
            }
            KeyCode::Char(' ') => {
                // Toggle enabled/disabled.
                app.hooks_browser_state.detail_mode = false;
                if app.hooks_browser_data.enabled {
                    Action::ExecuteCommand("/hooks disable".to_string())
                } else {
                    Action::ExecuteCommand("/hooks enable".to_string())
                }
            }
            KeyCode::Char('n') => {
                app.hooks_browser_state.detail_mode = false;
                app.input = "/hooks create ".to_string();
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status(
                    "Create hook: /hooks create <name> <event> <program> [args...] (then Enter)",
                );
                Action::Continue
            }
            KeyCode::Char('e') => {
                let idx = app.hooks_browser_state.selected_index;
                let Some((name, event, program, args)) = app.hooks_browser_data.hooks.get(idx)
                else {
                    return Action::Continue;
                };

                let args = args.trim();
                app.hooks_browser_state.detail_mode = false;
                app.input = if args.is_empty() {
                    format!("/hooks update {name} {event} {program} ")
                } else {
                    format!("/hooks update {name} {event} {program} {args}")
                };
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Edit hook: adjust fields and press Enter");
                Action::Continue
            }
            KeyCode::Char('x') => {
                let idx = app.hooks_browser_state.selected_index;
                let Some((name, event, program, _args)) = app.hooks_browser_data.hooks.get(idx)
                else {
                    return Action::Continue;
                };

                app.hooks_browser_state.detail_mode = false;
                let name_short: String = name.chars().take(80).collect();
                app.show_confirm(ConfirmAction::ExecuteCommand {
                    title: "Delete Hook?".to_string(),
                    message: format!(
                        "This will permanently delete the hook:\n\n  {name_short}\n  ({event}) -> {program}\n\n  [Y] Yes, delete    [N] No, cancel"
                    ),
                    command: format!("/hooks delete {name}"),
                });
                Action::Continue
            }
            KeyCode::Char('a') => {
                let idx = app.hooks_browser_state.selected_index;
                let program = app
                    .hooks_browser_data
                    .hooks
                    .get(idx)
                    .map(|(_, _, program, _)| program.as_str())
                    .unwrap_or("");
                app.hooks_browser_state.detail_mode = false;
                app.input = if program.trim().is_empty() {
                    "/hooks allow add ".to_string()
                } else {
                    format!("/hooks allow add {program}")
                };
                if !app.input.ends_with(' ') {
                    app.input.push(' ');
                }
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Allow program: edit and press Enter");
                Action::Continue
            }
            KeyCode::Char('r') => {
                let idx = app.hooks_browser_state.selected_index;
                let program = app
                    .hooks_browser_data
                    .hooks
                    .get(idx)
                    .map(|(_, _, program, _)| program.as_str())
                    .unwrap_or("");
                app.hooks_browser_state.detail_mode = false;
                app.input = if program.trim().is_empty() {
                    "/hooks allow remove ".to_string()
                } else {
                    format!("/hooks allow remove {program}")
                };
                if !app.input.ends_with(' ') {
                    app.input.push(' ');
                }
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Remove from allowlist: edit and press Enter");
                Action::Continue
            }
            KeyCode::Char('t') => {
                app.hooks_browser_state.detail_mode = false;
                app.input = format!(
                    "/hooks set timeout_ms {} ",
                    app.hooks_browser_data.timeout_ms
                );
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Set hooks timeout (ms) and press Enter");
                Action::Continue
            }
            KeyCode::Char('m') => {
                app.hooks_browser_state.detail_mode = false;
                app.input = format!(
                    "/hooks set max_output_bytes {} ",
                    app.hooks_browser_data.max_output_bytes
                );
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Set hooks max output bytes and press Enter");
                Action::Continue
            }
            _ => Action::Continue,
        }
    } else {
        match key.code {
            KeyCode::Esc => {
                app.mode = TuiMode::Insert;
                app.set_status("Returned to agent");
                Action::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.hooks_browser_state.select_next();
                Action::Continue
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.hooks_browser_state.select_prev();
                Action::Continue
            }
            KeyCode::Enter => {
                // If there are no hooks, pressing Enter should guide creation instead of
                // switching to an empty detail pane.
                if app.hooks_browser_data.hooks.is_empty() {
                    app.input = "/hooks create ".to_string();
                    app.cursor_pos = app.input.len();
                    app.mode = TuiMode::Command;
                    app.set_status(
                        "Create hook: /hooks create <name> <event> <program> [args...] (then Enter)",
                    );
                    Action::Continue
                } else {
                    app.hooks_browser_state.detail_mode = true;
                    Action::Continue
                }
            }
            KeyCode::Char(' ') => {
                if app.hooks_browser_data.enabled {
                    Action::ExecuteCommand("/hooks disable".to_string())
                } else {
                    Action::ExecuteCommand("/hooks enable".to_string())
                }
            }
            KeyCode::Char('n') => {
                app.input = "/hooks create ".to_string();
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status(
                    "Create hook: /hooks create <name> <event> <program> [args...] (then Enter)",
                );
                Action::Continue
            }
            KeyCode::Char('e') => {
                let idx = app.hooks_browser_state.selected_index;
                let Some((name, event, program, args)) = app.hooks_browser_data.hooks.get(idx)
                else {
                    return Action::Continue;
                };

                let args = args.trim();
                app.input = if args.is_empty() {
                    format!("/hooks update {name} {event} {program} ")
                } else {
                    format!("/hooks update {name} {event} {program} {args}")
                };
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Edit hook: adjust fields and press Enter");
                Action::Continue
            }
            KeyCode::Char('x') => {
                let idx = app.hooks_browser_state.selected_index;
                let Some((name, event, program, _args)) = app.hooks_browser_data.hooks.get(idx)
                else {
                    return Action::Continue;
                };

                let name_short: String = name.chars().take(80).collect();
                app.show_confirm(ConfirmAction::ExecuteCommand {
                    title: "Delete Hook?".to_string(),
                    message: format!(
                        "This will permanently delete the hook:\n\n  {name_short}\n  ({event}) -> {program}\n\n  [Y] Yes, delete    [N] No, cancel"
                    ),
                    command: format!("/hooks delete {name}"),
                });
                Action::Continue
            }
            KeyCode::Char('a') => {
                let idx = app.hooks_browser_state.selected_index;
                let program = app
                    .hooks_browser_data
                    .hooks
                    .get(idx)
                    .map(|(_, _, program, _)| program.as_str())
                    .unwrap_or("");
                app.input = if program.trim().is_empty() {
                    "/hooks allow add ".to_string()
                } else {
                    format!("/hooks allow add {program}")
                };
                if !app.input.ends_with(' ') {
                    app.input.push(' ');
                }
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Allow program: edit and press Enter");
                Action::Continue
            }
            KeyCode::Char('r') => {
                let idx = app.hooks_browser_state.selected_index;
                let program = app
                    .hooks_browser_data
                    .hooks
                    .get(idx)
                    .map(|(_, _, program, _)| program.as_str())
                    .unwrap_or("");
                app.input = if program.trim().is_empty() {
                    "/hooks allow remove ".to_string()
                } else {
                    format!("/hooks allow remove {program}")
                };
                if !app.input.ends_with(' ') {
                    app.input.push(' ');
                }
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Remove from allowlist: edit and press Enter");
                Action::Continue
            }
            KeyCode::Char('t') => {
                app.input = format!(
                    "/hooks set timeout_ms {} ",
                    app.hooks_browser_data.timeout_ms
                );
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Set hooks timeout (ms) and press Enter");
                Action::Continue
            }
            KeyCode::Char('m') => {
                app.input = format!(
                    "/hooks set max_output_bytes {} ",
                    app.hooks_browser_data.max_output_bytes
                );
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Set hooks max output bytes and press Enter");
                Action::Continue
            }
            _ => Action::Continue,
        }
    }
}

/// Handle keyboard events in the Agent browser overlay.
fn handle_agent_browser_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    if app.agent_browser_state.detail_mode {
        match key.code {
            KeyCode::Esc => {
                app.agent_browser_state.detail_mode = false;
                app.set_status("Agent: ↑/↓ navigate  Enter details  Esc close");
                Action::Continue
            }
            _ => Action::Continue,
        }
    } else {
        match key.code {
            KeyCode::Esc => {
                app.mode = TuiMode::Insert;
                app.set_status("Returned to agent");
                Action::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.agent_browser_state.select_next();
                Action::Continue
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.agent_browser_state.select_prev();
                Action::Continue
            }
            KeyCode::Enter => {
                app.agent_browser_state.detail_mode = true;
                Action::Continue
            }
            _ => Action::Continue,
        }
    }
}

/// Handle keyboard events in the Memory browser overlay.
fn handle_memory_browser_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    if app.memory_browser_state.detail_mode {
        match key.code {
            KeyCode::Esc => {
                app.memory_browser_state.detail_mode = false;
                app.set_status("Memory: ↑/↓ navigate  Enter details  s save  x delete  Esc close");
                Action::Continue
            }
            KeyCode::Char('s') => {
                app.memory_browser_state.detail_mode = false;
                app.input = "/memory save ".to_string();
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Save memory: add optional flags and press Enter");
                Action::Continue
            }
            KeyCode::Char('x') => {
                let idx = app.memory_browser_state.selected_index;
                let Some(entry) = app.memory_browser_entries.get(idx) else {
                    return Action::Continue;
                };
                let Some(path) = entry.file_path.as_deref() else {
                    app.set_status("Cannot delete: missing memory entry file path");
                    return Action::Continue;
                };

                // Return to list view after confirmation so we don't try to render a deleted entry.
                app.memory_browser_state.detail_mode = false;

                let summary: String = entry.summary.chars().take(80).collect();
                app.show_confirm(ConfirmAction::ExecuteCommand {
                    title: "Delete Memory Entry?".to_string(),
                    message: format!(
                        "This will permanently delete:\n\n  {summary}\n  {path}\n\n  [Y] Yes, delete    [N] No, cancel"
                    ),
                    command: format!("/memory delete --confirmed {path}"),
                });
                Action::Continue
            }
            _ => Action::Continue,
        }
    } else {
        match key.code {
            KeyCode::Esc => {
                app.mode = TuiMode::Insert;
                app.set_status("Returned to agent");
                Action::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.memory_browser_state.select_next();
                Action::Continue
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.memory_browser_state.select_prev();
                Action::Continue
            }
            KeyCode::Enter => {
                // Avoid switching into an empty detail pane when there are no entries.
                if app.memory_browser_entries.is_empty() {
                    app.input = "/memory save ".to_string();
                    app.cursor_pos = app.input.len();
                    app.mode = TuiMode::Command;
                    app.set_status("Save memory: add optional flags and press Enter");
                    Action::Continue
                } else {
                    app.memory_browser_state.detail_mode = true;
                    Action::Continue
                }
            }
            KeyCode::Char('s') => {
                app.input = "/memory save ".to_string();
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Save memory: add optional flags and press Enter");
                Action::Continue
            }
            KeyCode::Char('x') => {
                let idx = app.memory_browser_state.selected_index;
                let Some(entry) = app.memory_browser_entries.get(idx) else {
                    return Action::Continue;
                };
                let Some(path) = entry.file_path.as_deref() else {
                    app.set_status("Cannot delete: missing memory entry file path");
                    return Action::Continue;
                };

                let summary: String = entry.summary.chars().take(80).collect();
                app.show_confirm(ConfirmAction::ExecuteCommand {
                    title: "Delete Memory Entry?".to_string(),
                    message: format!(
                        "This will permanently delete:\n\n  {summary}\n  {path}\n\n  [Y] Yes, delete    [N] No, cancel"
                    ),
                    command: format!("/memory delete --confirmed {path}"),
                });
                Action::Continue
            }
            _ => Action::Continue,
        }
    }
}

fn next_task_status_token(current: &str) -> Option<&'static str> {
    // `TaskBrowserEntry.status` is currently `format!("{:?}", TaskStatus)`.
    let norm = current
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .replace(' ', "");

    match norm.as_str() {
        "notstarted" | "not_started" => Some("in_progress"),
        "inprogress" | "in_progress" => Some("completed"),
        "completed" => Some("cancelled"),
        "cancelled" | "canceled" => Some("not_started"),
        _ => None,
    }
}

/// Handle keyboard events in the Devices browser overlay.
fn handle_devices_browser_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    if app.devices_browser_state.detail_mode {
        match key.code {
            KeyCode::Esc => {
                app.devices_browser_state.detail_mode = false;
                app.set_status("Devices: ↑/↓ navigate  Enter details  Esc close");
                Action::Continue
            }
            _ => Action::Continue,
        }
    } else {
        match key.code {
            KeyCode::Esc => {
                app.mode = TuiMode::Insert;
                app.set_status("Returned to agent");
                Action::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.devices_browser_state.select_next();
                Action::Continue
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.devices_browser_state.select_prev();
                Action::Continue
            }
            KeyCode::Enter => {
                app.devices_browser_state.detail_mode = true;
                Action::Continue
            }
            _ => Action::Continue,
        }
    }
}

/// Handle keyboard events in the Permissions browser overlay.
fn handle_permissions_browser_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    if app.permissions_browser_state.detail_mode {
        match key.code {
            KeyCode::Esc => {
                app.permissions_browser_state.detail_mode = false;
                app.set_status(
                    "Permissions: ↑/↓ navigate  Enter details  g grant  x revoke  r reset  l level  Esc close",
                );
                Action::Continue
            }
            KeyCode::Char('g') => {
                let idx = app.permissions_browser_state.selected_index;
                let prefill = app
                    .permissions_browser_entries
                    .get(idx)
                    .map(|entry| format!("/permissions grant {}.{} ", entry.tool, entry.action));
                app.permissions_browser_state.detail_mode = false;
                app.input = prefill.unwrap_or_else(|| "/permissions grant ".to_string());
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Grant permission: add optional [scope] and press Enter");
                Action::Continue
            }
            KeyCode::Char('x') => {
                let idx = app.permissions_browser_state.selected_index;
                if let Some(entry) = app.permissions_browser_entries.get(idx) {
                    let tool = entry.tool.clone();
                    let action_str = entry.action.clone();
                    app.permissions_browser_state.detail_mode = false;
                    return Action::ExecuteCommand(format!(
                        "/permissions revoke {} {}",
                        tool, action_str
                    ));
                }
                Action::Continue
            }
            KeyCode::Char('r') => {
                app.permissions_browser_state.detail_mode = false;
                app.show_confirm(ConfirmAction::ExecuteCommand {
                    title: "Reset Permissions?".to_string(),
                    message: "This will remove ALL granted tool permissions.\n\n  [Y] Yes, reset    [N] No, cancel"
                        .to_string(),
                    command: "/permissions reset".to_string(),
                });
                Action::Continue
            }
            KeyCode::Char('l') => {
                app.permissions_browser_state.detail_mode = false;
                app.input = "/permissions level set ".to_string();
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Set permission level: sandbox|restricted|full, then Enter");
                Action::Continue
            }
            _ => Action::Continue,
        }
    } else {
        match key.code {
            KeyCode::Esc => {
                app.mode = TuiMode::Insert;
                app.set_status("Returned to agent");
                Action::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.permissions_browser_state.select_next();
                Action::Continue
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.permissions_browser_state.select_prev();
                Action::Continue
            }
            KeyCode::Enter => {
                // Avoid switching into an empty detail pane when there are no permissions.
                if app.permissions_browser_entries.is_empty() {
                    app.input = "/permissions grant ".to_string();
                    app.cursor_pos = app.input.len();
                    app.mode = TuiMode::Command;
                    app.set_status("Grant permission: add optional [scope] and press Enter");
                    Action::Continue
                } else {
                    app.permissions_browser_state.detail_mode = true;
                    Action::Continue
                }
            }
            KeyCode::Char('g') => {
                let idx = app.permissions_browser_state.selected_index;
                let prefill = app
                    .permissions_browser_entries
                    .get(idx)
                    .map(|entry| format!("/permissions grant {}.{} ", entry.tool, entry.action));
                app.input = prefill.unwrap_or_else(|| "/permissions grant ".to_string());
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Grant permission: add optional [scope] and press Enter");
                Action::Continue
            }
            KeyCode::Char('x') => {
                let idx = app.permissions_browser_state.selected_index;
                if let Some(entry) = app.permissions_browser_entries.get(idx) {
                    let tool = entry.tool.clone();
                    let action_str = entry.action.clone();
                    return Action::ExecuteCommand(format!(
                        "/permissions revoke {} {}",
                        tool, action_str
                    ));
                }
                Action::Continue
            }
            KeyCode::Char('r') => {
                app.show_confirm(ConfirmAction::ExecuteCommand {
                    title: "Reset Permissions?".to_string(),
                    message: "This will remove ALL granted tool permissions.\n\n  [Y] Yes, reset    [N] No, cancel"
                        .to_string(),
                    command: "/permissions reset".to_string(),
                });
                Action::Continue
            }
            KeyCode::Char('l') => {
                app.input = "/permissions level set ".to_string();
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Set permission level: sandbox|restricted|full, then Enter");
                Action::Continue
            }
            _ => Action::Continue,
        }
    }
}

/// Handle keyboard events in the Sessions browser overlay.
fn handle_sessions_browser_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    if app.sessions_browser_state.detail_mode {
        match key.code {
            KeyCode::Esc => {
                app.sessions_browser_state.detail_mode = false;
                app.set_status(
                    "Sessions: ↑/↓ navigate  Enter details  l load  x delete  e export  Esc close",
                );
                Action::Continue
            }
            KeyCode::Char('l') => {
                let idx = app.sessions_browser_state.selected_index;
                if let Some(entry) = app.sessions_browser_entries.get(idx) {
                    if !entry.is_current {
                        let id = entry.id.clone();
                        app.sessions_browser_state.detail_mode = false;
                        app.mode = TuiMode::Insert;
                        return Action::ExecuteCommand(format!("/session load {}", id));
                    }
                    app.set_status("Already in this session");
                }
                Action::Continue
            }
            _ => Action::Continue,
        }
    } else {
        match key.code {
            KeyCode::Esc => {
                app.mode = TuiMode::Insert;
                app.set_status("Returned to agent");
                Action::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.sessions_browser_state.select_next();
                Action::Continue
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.sessions_browser_state.select_prev();
                Action::Continue
            }
            KeyCode::Enter => {
                app.sessions_browser_state.detail_mode = true;
                Action::Continue
            }
            KeyCode::Char('l') => {
                let idx = app.sessions_browser_state.selected_index;
                if let Some(entry) = app.sessions_browser_entries.get(idx) {
                    if !entry.is_current {
                        let id = entry.id.clone();
                        app.mode = TuiMode::Insert;
                        return Action::ExecuteCommand(format!("/session load {}", id));
                    }
                    app.set_status("Already in this session");
                }
                Action::Continue
            }
            KeyCode::Char('x') => {
                let idx = app.sessions_browser_state.selected_index;
                if let Some(entry) = app.sessions_browser_entries.get(idx) {
                    if entry.is_current {
                        app.set_status("Cannot delete the current session");
                    } else {
                        let id = entry.id.clone();
                        return Action::ExecuteCommand(format!("/session delete {}", id));
                    }
                }
                Action::Continue
            }
            KeyCode::Char('e') => {
                let idx = app.sessions_browser_state.selected_index;
                if let Some(entry) = app.sessions_browser_entries.get(idx) {
                    let id = entry.id.clone();
                    return Action::ExecuteCommand(format!("/session export {}", id));
                }
                Action::Continue
            }
            _ => Action::Continue,
        }
    }
}

/// Handle keyboard events in the Tasks browser overlay.
fn handle_tasks_browser_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    let sanitize_one_line = |s: &str, max: usize| -> String {
        s.replace(['\n', '\r', '\t'], " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(max)
            .collect()
    };

    if app.tasks_browser_state.detail_mode {
        match key.code {
            KeyCode::Esc => {
                app.tasks_browser_state.detail_mode = false;
                app.set_status(
                    "Tasks: ↑/↓ navigate  Enter details  n new  e name  d desc  s sub  a dep  Space status  c current  u clear  x delete  Esc close",
                );
                Action::Continue
            }
            KeyCode::Char(' ') => {
                let idx = app.tasks_browser_state.selected_index;
                let Some(task) = app.tasks_browser_entries.get(idx) else {
                    return Action::Continue;
                };
                let Some(next) = next_task_status_token(&task.status) else {
                    app.set_status("Unknown task status; cannot cycle");
                    return Action::Continue;
                };

                // Refresh logic will typically return us to list view.
                app.tasks_browser_state.detail_mode = false;
                Action::ExecuteCommand(format!("/task status {} {}", task.id, next))
            }
            KeyCode::Char('c') => {
                let idx = app.tasks_browser_state.selected_index;
                let Some(task) = app.tasks_browser_entries.get(idx) else {
                    return Action::Continue;
                };
                app.tasks_browser_state.detail_mode = false;
                Action::ExecuteCommand(format!("/task current set {}", task.id))
            }
            KeyCode::Char('x') => {
                let idx = app.tasks_browser_state.selected_index;
                let Some(task) = app.tasks_browser_entries.get(idx) else {
                    return Action::Continue;
                };

                // Return to list view after confirmation so we don't try to render a deleted entry.
                app.tasks_browser_state.detail_mode = false;
                let name: String = task.name.chars().take(80).collect();
                app.show_confirm(ConfirmAction::ExecuteCommand {
                    title: "Delete Task?".to_string(),
                    message: format!(
                        "This will permanently delete:\n\n  {name}\n  {}\n\n  [Y] Yes, delete    [N] No, cancel",
                        task.id
                    ),
                    command: format!("/task delete --confirmed {}", task.id),
                });
                Action::Continue
            }
            KeyCode::Char('n') => {
                // Guided flow: prefill a command and switch to command mode.
                app.tasks_browser_state.detail_mode = false;
                app.input = "/task create ".to_string();
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Create task: type <name> [description...] and press Enter");
                Action::Continue
            }
            KeyCode::Char('e') => {
                let idx = app.tasks_browser_state.selected_index;
                let Some(task) = app.tasks_browser_entries.get(idx) else {
                    return Action::Continue;
                };
                let name = sanitize_one_line(&task.name, 120);
                app.tasks_browser_state.detail_mode = false;
                app.input = format!("/task update {} name {}", task.id, name);
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Edit task name and press Enter");
                Action::Continue
            }
            KeyCode::Char('d') => {
                let idx = app.tasks_browser_state.selected_index;
                let Some(task) = app.tasks_browser_entries.get(idx) else {
                    return Action::Continue;
                };
                let desc = sanitize_one_line(&task.description, 200);
                app.tasks_browser_state.detail_mode = false;
                app.input = format!("/task update {} desc {}", task.id, desc);
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Edit task description and press Enter");
                Action::Continue
            }
            KeyCode::Char('s') => {
                let idx = app.tasks_browser_state.selected_index;
                let Some(task) = app.tasks_browser_entries.get(idx) else {
                    return Action::Continue;
                };
                app.tasks_browser_state.detail_mode = false;
                app.input = format!("/task create-sub {} ", task.id);
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Create subtask: type <name> [description...] and press Enter");
                Action::Continue
            }
            KeyCode::Char('a') => {
                let idx = app.tasks_browser_state.selected_index;
                let Some(task) = app.tasks_browser_entries.get(idx) else {
                    return Action::Continue;
                };
                app.tasks_browser_state.detail_mode = false;
                app.input = format!("/task dep add {} ", task.id);
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Add dependency: type <blocked_by_id> and press Enter");
                Action::Continue
            }
            KeyCode::Char('u') => {
                app.tasks_browser_state.detail_mode = false;
                Action::ExecuteCommand("/task current clear".to_string())
            }
            _ => Action::Continue,
        }
    } else {
        match key.code {
            KeyCode::Esc => {
                app.mode = TuiMode::Insert;
                app.set_status("Returned to agent");
                Action::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.tasks_browser_state.select_next();
                Action::Continue
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.tasks_browser_state.select_prev();
                Action::Continue
            }
            KeyCode::Enter => {
                // If there are no tasks, pressing Enter should guide creation instead of
                // switching to an empty detail pane.
                if app.tasks_browser_entries.is_empty() {
                    app.input = "/task create ".to_string();
                    app.cursor_pos = app.input.len();
                    app.mode = TuiMode::Command;
                    app.set_status("Create task: type <name> [description...] and press Enter");
                    Action::Continue
                } else {
                    app.tasks_browser_state.detail_mode = true;
                    Action::Continue
                }
            }
            KeyCode::Char(' ') => {
                let idx = app.tasks_browser_state.selected_index;
                let Some(task) = app.tasks_browser_entries.get(idx) else {
                    return Action::Continue;
                };
                let Some(next) = next_task_status_token(&task.status) else {
                    app.set_status("Unknown task status; cannot cycle");
                    return Action::Continue;
                };
                Action::ExecuteCommand(format!("/task status {} {}", task.id, next))
            }
            KeyCode::Char('c') => {
                let idx = app.tasks_browser_state.selected_index;
                let Some(task) = app.tasks_browser_entries.get(idx) else {
                    return Action::Continue;
                };
                Action::ExecuteCommand(format!("/task current set {}", task.id))
            }
            KeyCode::Char('x') => {
                let idx = app.tasks_browser_state.selected_index;
                let Some(task) = app.tasks_browser_entries.get(idx) else {
                    return Action::Continue;
                };

                let name: String = task.name.chars().take(80).collect();
                app.show_confirm(ConfirmAction::ExecuteCommand {
                    title: "Delete Task?".to_string(),
                    message: format!(
                        "This will permanently delete:\n\n  {name}\n  {}\n\n  [Y] Yes, delete    [N] No, cancel",
                        task.id
                    ),
                    command: format!("/task delete --confirmed {}", task.id),
                });
                Action::Continue
            }
            KeyCode::Char('n') => {
                app.input = "/task create ".to_string();
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Create task: type <name> [description...] and press Enter");
                Action::Continue
            }
            KeyCode::Char('e') => {
                let idx = app.tasks_browser_state.selected_index;
                let Some(task) = app.tasks_browser_entries.get(idx) else {
                    return Action::Continue;
                };
                let name = sanitize_one_line(&task.name, 120);
                app.input = format!("/task update {} name {}", task.id, name);
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Edit task name and press Enter");
                Action::Continue
            }
            KeyCode::Char('d') => {
                let idx = app.tasks_browser_state.selected_index;
                let Some(task) = app.tasks_browser_entries.get(idx) else {
                    return Action::Continue;
                };
                let desc = sanitize_one_line(&task.description, 200);
                app.input = format!("/task update {} desc {}", task.id, desc);
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Edit task description and press Enter");
                Action::Continue
            }
            KeyCode::Char('s') => {
                let idx = app.tasks_browser_state.selected_index;
                let Some(task) = app.tasks_browser_entries.get(idx) else {
                    return Action::Continue;
                };
                app.input = format!("/task create-sub {} ", task.id);
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Create subtask: type <name> [description...] and press Enter");
                Action::Continue
            }
            KeyCode::Char('a') => {
                let idx = app.tasks_browser_state.selected_index;
                let Some(task) = app.tasks_browser_entries.get(idx) else {
                    return Action::Continue;
                };
                app.input = format!("/task dep add {} ", task.id);
                app.cursor_pos = app.input.len();
                app.mode = TuiMode::Command;
                app.set_status("Add dependency: type <blocked_by_id> and press Enter");
                Action::Continue
            }
            KeyCode::Char('u') => Action::ExecuteCommand("/task current clear".to_string()),
            _ => Action::Continue,
        }
    }
}

/// Handle keyboard events in the Themes browser overlay.
fn handle_themes_browser_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => {
            app.mode = TuiMode::Insert;
            app.set_status("Returned to agent");
            Action::Continue
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.themes_browser_state.select_next();
            Action::Continue
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.themes_browser_state.select_prev();
            Action::Continue
        }
        KeyCode::Enter => {
            let idx = app.themes_browser_state.selected_index;
            if let Some(name) = app.themes_browser_names.get(idx) {
                let name = name.clone();
                app.set_theme(&name);
                app.set_status(format!("Theme set to '{}'", name));
            }
            Action::Continue
        }
        _ => Action::Continue,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::new_cli_session;
    use ratatui::layout::Rect;

    use super::super::app::CopyButtonHit;
    use gestura_core::AppConfig;

    /// Helper to create a test app instance for event handling tests.
    fn create_test_app() -> TuiApp {
        let session = new_cli_session(None).unwrap();
        let config = AppConfig::default();
        TuiApp::new(session, config, None)
    }

    #[test]
    fn mouse_move_updates_hovered_copy_button() {
        let mut app = create_test_app();
        app.active_tab = 0;
        app.layout_areas.messages = Some(Rect {
            x: 0,
            y: 10,
            width: 80,
            height: 20,
        });
        app.assistant_copy_buttons = vec![CopyButtonHit {
            message_index: 42,
            rect: Rect {
                x: 70,
                y: 12,
                width: 4,
                height: 1,
            },
        }];

        let action = handle_event(
            &mut app,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: 71,
                row: 12,
                modifiers: KeyModifiers::NONE,
            }),
        );
        assert_eq!(action, Action::Continue);
        assert_eq!(app.hovered_copy_button, Some(42));

        let _ = handle_event(
            &mut app,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: 1,
                row: 1,
                modifiers: KeyModifiers::NONE,
            }),
        );
        assert_eq!(app.hovered_copy_button, None);
    }

    #[test]
    fn mouse_down_and_up_on_copy_button_copies_on_release() {
        let mut app = create_test_app();
        app.active_tab = 0;
        app.layout_areas.messages = Some(Rect {
            x: 0,
            y: 10,
            width: 80,
            height: 20,
        });
        app.assistant_copy_buttons = vec![CopyButtonHit {
            message_index: 7,
            rect: Rect {
                x: 70,
                y: 12,
                width: 4,
                height: 1,
            },
        }];
        app.selection_anchor = Some(0);
        app.selection_end = Some(0);

        let down = handle_event(
            &mut app,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
                column: 71,
                row: 12,
                modifiers: KeyModifiers::NONE,
            }),
        );
        assert_eq!(down, Action::Continue);
        assert_eq!(app.pressed_copy_button, Some(7));
        assert_eq!(app.selection_anchor, None);
        assert_eq!(app.selection_end, None);

        let up = handle_event(
            &mut app,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Up(crossterm::event::MouseButton::Left),
                column: 71,
                row: 12,
                modifiers: KeyModifiers::NONE,
            }),
        );
        assert_eq!(up, Action::CopyMessageRaw(7));
        assert_eq!(app.pressed_copy_button, None);
    }

    #[test]
    fn mouse_release_off_copy_button_does_not_copy() {
        let mut app = create_test_app();
        app.active_tab = 0;
        app.layout_areas.messages = Some(Rect {
            x: 0,
            y: 10,
            width: 80,
            height: 20,
        });
        app.assistant_copy_buttons = vec![CopyButtonHit {
            message_index: 9,
            rect: Rect {
                x: 70,
                y: 12,
                width: 4,
                height: 1,
            },
        }];

        let _ = handle_event(
            &mut app,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
                column: 71,
                row: 12,
                modifiers: KeyModifiers::NONE,
            }),
        );
        assert_eq!(app.pressed_copy_button, Some(9));

        let up = handle_event(
            &mut app,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Up(crossterm::event::MouseButton::Left),
                column: 1,
                row: 1,
                modifiers: KeyModifiers::NONE,
            }),
        );
        assert_eq!(up, Action::Continue);
        assert_eq!(app.pressed_copy_button, None);
    }

    #[test]
    fn tool_confirm_mode_esc_clears_pending_and_returns_to_insert() {
        let mut app = create_test_app();
        app.mode = TuiMode::ToolConfirm;
        app.pending_tool_confirmation = Some(PendingToolConfirmation {
            confirmation_id: "test".to_string(),
            tool_name: "fake-tool".to_string(),
            tool_args: "{}".to_string(),
            description: "testing".to_string(),
            risk_level: 1,
            category: "test".to_string(),
        });

        let action = handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );

        assert_eq!(action, Action::Continue);
        assert_eq!(app.mode, TuiMode::Insert);
        assert!(app.pending_tool_confirmation.is_none());
    }

    #[test]
    fn entering_command_mode_from_normal_populates_suggestions() {
        let mut app = create_test_app();
        app.mode = TuiMode::Normal;

        let action = handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE)),
        );
        assert_eq!(action, Action::Continue);
        assert_eq!(app.mode, TuiMode::Command);
        assert!(!app.command_suggestions.is_empty());
    }

    #[test]
    fn entering_command_mode_from_insert_populates_suggestions() {
        let mut app = create_test_app();
        app.mode = TuiMode::Insert;

        let action = handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE)),
        );
        assert_eq!(action, Action::Continue);
        assert_eq!(app.mode, TuiMode::Command);
        assert!(!app.command_suggestions.is_empty());
    }

    #[test]
    fn paste_in_insert_inserts_text_and_preserves_mode() {
        let mut app = create_test_app();
        app.mode = TuiMode::Insert;

        let action = handle_event(&mut app, Event::Paste("hello world".to_string()));
        assert_eq!(action, Action::Continue);
        assert_eq!(app.mode, TuiMode::Insert);
        assert_eq!(app.input, "hello world");
    }

    #[test]
    fn paste_in_insert_switches_to_command_mode_if_starts_with_slash() {
        let mut app = create_test_app();
        app.mode = TuiMode::Insert;

        let action = handle_event(&mut app, Event::Paste("/theme dark".to_string()));
        assert_eq!(action, Action::Continue);
        assert_eq!(app.mode, TuiMode::Command);
        assert!(app.input.starts_with('/'));
    }

    #[test]
    fn paste_normalizes_crlf_and_cr_newlines() {
        let mut app = create_test_app();
        app.mode = TuiMode::Insert;

        let action = handle_event(&mut app, Event::Paste("a\r\nb\rc".to_string()));
        assert_eq!(action, Action::Continue);
        assert_eq!(app.input, "a\nb\nc");
    }

    #[test]
    fn paste_in_search_ignores_newlines() {
        let mut app = create_test_app();
        app.mode = TuiMode::Search;
        app.search_query.clear();

        let action = handle_event(&mut app, Event::Paste("a\n\nb".to_string()));
        assert_eq!(action, Action::Continue);
        assert_eq!(app.search_query, "ab");
    }

    #[test]
    fn insert_enter_empty_input_with_listening_mode_triggers_recording() {
        let mut app = create_test_app();
        app.mode = TuiMode::Insert;
        app.input.clear();
        app.listening_mode = true;
        app.is_loading = false;
        app.voice_capture_in_progress = false;

        let action = handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );

        assert_eq!(action, Action::ToggleRecording);
    }

    #[test]
    fn insert_enter_while_voice_capture_in_progress_does_not_toggle_recording() {
        let mut app = create_test_app();
        app.mode = TuiMode::Insert;
        app.input.clear();
        app.listening_mode = true;
        app.is_loading = false;
        app.voice_capture_in_progress = true;

        let action = handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );

        assert_eq!(action, Action::Continue);
        assert_eq!(app.status, "Recording… press Esc to cancel");
    }

    #[test]
    fn insert_enter_empty_input_without_listening_mode_is_noop() {
        let mut app = create_test_app();
        app.mode = TuiMode::Insert;
        app.input.clear();
        app.listening_mode = false;
        app.is_loading = false;
        app.voice_capture_in_progress = false;

        let action = handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );

        assert_eq!(action, Action::Continue);
    }

    #[test]
    fn confirm_execute_command_returns_action_and_restores_previous_mode() {
        let mut app = create_test_app();
        app.mode = TuiMode::Memory;
        app.show_confirm(ConfirmAction::ExecuteCommand {
            title: "Confirm".to_string(),
            message: "Delete it?\n\n  [Y] Yes    [N] No".to_string(),
            command: "/memory delete --confirmed .gestura/memory/entry.md".to_string(),
        });

        let action = handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );

        assert_eq!(
            action,
            Action::ExecuteCommand(
                "/memory delete --confirmed .gestura/memory/entry.md".to_string()
            )
        );
        assert_eq!(app.mode, TuiMode::Memory);
        assert!(app.pending_confirm.is_none());
    }

    #[test]
    fn confirm_cancel_restores_previous_mode() {
        let mut app = create_test_app();
        app.mode = TuiMode::Tasks;
        app.show_confirm(ConfirmAction::ClearMessages);

        let action = handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );

        assert_eq!(action, Action::Continue);
        assert_eq!(app.mode, TuiMode::Tasks);
        assert!(app.pending_confirm.is_none());
    }

    #[test]
    fn memory_x_shows_confirm_execute_command() {
        let mut app = create_test_app();
        app.mode = TuiMode::Memory;
        app.memory_browser_entries = vec![super::super::app::MemoryBrowserEntry {
            timestamp: "2026-02-13 12:00".to_string(),
            category: Some("engineering".to_string()),
            summary: "A test memory".to_string(),
            content: "content".to_string(),
            session_id: "session-123".to_string(),
            file_path: Some(".gestura/memory/memory_20260213_120000_session-1.md".to_string()),
        }];
        app.memory_browser_state
            .reset(app.memory_browser_entries.len());

        let action = handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
        );

        assert_eq!(action, Action::Continue);
        assert_eq!(app.mode, TuiMode::Confirm);
        assert_eq!(app.confirm_return_mode, Some(TuiMode::Memory));
        match app.pending_confirm.as_ref().expect("pending confirm") {
            ConfirmAction::ExecuteCommand { command, .. } => {
                assert_eq!(
                    command,
                    "/memory delete --confirmed .gestura/memory/memory_20260213_120000_session-1.md"
                );
            }
            other => panic!("unexpected confirm action: {other:?}"),
        }
    }

    #[test]
    fn tasks_space_cycles_status_to_in_progress() {
        let mut app = create_test_app();
        app.mode = TuiMode::Tasks;
        app.tasks_browser_entries = vec![super::super::app::TaskBrowserEntry {
            id: "task-abc".to_string(),
            name: "Test".to_string(),
            description: "".to_string(),
            status: "NotStarted".to_string(),
            status_icon: "[ ]".to_string(),
            parent_id: None,
            source: "User".to_string(),
            created: "2026-02-13 12:00".to_string(),
        }];
        app.tasks_browser_state
            .reset(app.tasks_browser_entries.len());

        let action = handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)),
        );

        assert_eq!(
            action,
            Action::ExecuteCommand("/task status task-abc in_progress".to_string())
        );
    }

    #[test]
    fn tasks_x_shows_confirm_execute_command() {
        let mut app = create_test_app();
        app.mode = TuiMode::Tasks;
        app.tasks_browser_entries = vec![super::super::app::TaskBrowserEntry {
            id: "task-abc".to_string(),
            name: "Test".to_string(),
            description: "".to_string(),
            status: "NotStarted".to_string(),
            status_icon: "[ ]".to_string(),
            parent_id: None,
            source: "User".to_string(),
            created: "2026-02-13 12:00".to_string(),
        }];
        app.tasks_browser_state
            .reset(app.tasks_browser_entries.len());

        let action = handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
        );

        assert_eq!(action, Action::Continue);
        assert_eq!(app.mode, TuiMode::Confirm);
        assert_eq!(app.confirm_return_mode, Some(TuiMode::Tasks));
        match app.pending_confirm.as_ref().expect("pending confirm") {
            ConfirmAction::ExecuteCommand { command, .. } => {
                assert_eq!(command, "/task delete --confirmed task-abc");
            }
            other => panic!("unexpected confirm action: {other:?}"),
        }
    }

    #[test]
    fn tasks_n_prefills_create_command() {
        let mut app = create_test_app();
        app.mode = TuiMode::Tasks;
        app.tasks_browser_entries = vec![];
        app.tasks_browser_state.reset(0);

        let action = handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE)),
        );

        assert_eq!(action, Action::Continue);
        assert_eq!(app.mode, TuiMode::Command);
        assert_eq!(app.input, "/task create ");
    }

    #[test]
    fn mcp_enter_on_empty_prefills_add_command() {
        let mut app = create_test_app();
        app.mode = TuiMode::Mcp;
        app.mcp_browser_state.servers = vec![];
        app.mcp_browser_state.detail_mode = false;

        let action = handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );

        assert_eq!(action, Action::Continue);
        assert_eq!(app.mode, TuiMode::Command);
        assert_eq!(app.input, "/mcp add ");
    }

    #[test]
    fn memory_enter_on_empty_prefills_save_command() {
        let mut app = create_test_app();
        app.mode = TuiMode::Memory;
        app.memory_browser_entries = vec![];
        app.memory_browser_state.reset(0);

        let action = handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );

        assert_eq!(action, Action::Continue);
        assert_eq!(app.mode, TuiMode::Command);
        assert_eq!(app.input, "/memory save ");
    }

    #[test]
    fn hooks_space_toggles_enabled() {
        let mut app = create_test_app();
        app.mode = TuiMode::Hooks;
        app.hooks_browser_data.enabled = false;
        // Mirror open_hooks_browser behavior: at least 1 item for empty state.
        app.hooks_browser_state.reset(1);

        let action = handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)),
        );

        assert_eq!(action, Action::ExecuteCommand("/hooks enable".to_string()));
    }

    #[test]
    fn hooks_x_shows_confirm_execute_command() {
        let mut app = create_test_app();
        app.mode = TuiMode::Hooks;
        app.hooks_browser_data.hooks = vec![(
            "hook-1".to_string(),
            "PreTool".to_string(),
            "echo".to_string(),
            "hi".to_string(),
        )];
        app.hooks_browser_state.reset(1);

        let action = handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
        );

        assert_eq!(action, Action::Continue);
        assert_eq!(app.mode, TuiMode::Confirm);
        assert_eq!(app.confirm_return_mode, Some(TuiMode::Hooks));
        match app.pending_confirm.as_ref().expect("pending confirm") {
            ConfirmAction::ExecuteCommand { command, .. } => {
                assert_eq!(command, "/hooks delete hook-1");
            }
            other => panic!("unexpected confirm action: {other:?}"),
        }
    }

    #[test]
    fn permissions_g_prefills_grant_command() {
        let mut app = create_test_app();
        app.mode = TuiMode::Permissions;
        app.permissions_browser_entries = vec![super::super::app::PermissionBrowserEntry {
            tool: "file".to_string(),
            action: "read".to_string(),
            scope: "global".to_string(),
            expires: "never".to_string(),
        }];
        app.permissions_browser_state.reset(1);

        let action = handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE)),
        );

        assert_eq!(action, Action::Continue);
        assert_eq!(app.mode, TuiMode::Command);
        assert_eq!(app.input, "/permissions grant file.read ");
    }

    #[test]
    fn permissions_r_shows_confirm_execute_command() {
        let mut app = create_test_app();
        app.mode = TuiMode::Permissions;
        app.permissions_browser_state.reset(0);

        let action = handle_event(
            &mut app,
            Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)),
        );

        assert_eq!(action, Action::Continue);
        assert_eq!(app.mode, TuiMode::Confirm);
        assert_eq!(app.confirm_return_mode, Some(TuiMode::Permissions));
        match app.pending_confirm.as_ref().expect("pending confirm") {
            ConfirmAction::ExecuteCommand { command, .. } => {
                assert_eq!(command, "/permissions reset");
            }
            other => panic!("unexpected confirm action: {other:?}"),
        }
    }
}
