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
    // While streaming, keep the UI responsive. We still allow Esc to cancel the active stream.
    if app.is_loading && key.code == KeyCode::Esc {
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
            } else if !app.input.is_empty() {
                let input = app.take_input();
                // Check if it's a command
                if input.starts_with('/') {
                    Action::ExecuteCommand(input)
                } else {
                    Action::SendMessage(input)
                }
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

    match mouse.kind {
        MouseEventKind::ScrollUp => {
            match app.mode {
                TuiMode::Activity => app.activity_state.scroll_up(),
                TuiMode::ModelPicker => app.model_picker_state.select_prev(),
                TuiMode::ToolConfirm => {}
                _ => app.scroll_up(),
            }
            Action::Continue
        }
        MouseEventKind::ScrollDown => {
            match app.mode {
                TuiMode::Activity => app.activity_state.scroll_down(),
                TuiMode::ModelPicker => app.model_picker_state.select_next(),
                TuiMode::ToolConfirm => {}
                _ => app.scroll_down(),
            }
            Action::Continue
        }
        MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
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
            }

            Action::Continue
        }
        MouseEventKind::Drag(crossterm::event::MouseButton::Left) => {
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

    // Escape returns to chat
    if key.code == KeyCode::Esc && !app.settings_state.is_editing {
        app.active_tab = 0; // Return to chat tab
        app.mode = TuiMode::Insert;
        app.set_status("Returned to chat");
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
            app.active_tab = 0; // Return to chat tab
            app.mode = TuiMode::Insert;
            app.set_status("Returned to chat");
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
/// - **List mode** (default): ↑/↓ to navigate, Enter to open detail, Space to toggle enable/disable, Esc to return to chat.
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
                app.active_tab = 0; // Return to chat tab
                app.mode = TuiMode::Insert;
                app.set_status("Returned to chat");
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::chat::new_cli_session;
    use gestura_core::AppConfig;

    /// Helper to create a test app instance for event handling tests.
    fn create_test_app() -> TuiApp {
        let session = new_cli_session(None).unwrap();
        let config = AppConfig::default();
        TuiApp::new(session, config, None)
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
}
