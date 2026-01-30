//! Event handling for the TUI
//!
//! This module handles keyboard events and maps them to actions
//! based on the current application mode.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};

use super::app::{Action, ConfirmAction, TuiApp, TuiMode};

/// Handle an event and return the appropriate action
pub fn handle_event(app: &mut TuiApp, event: Event) -> Action {
    match event {
        Event::Key(key) => handle_key_event(app, key),
        Event::Mouse(mouse) => handle_mouse_event(app, mouse),
        Event::Resize(_, _) => Action::Continue, // Terminal will re-render automatically
        _ => Action::Continue,
    }
}

/// Handle keyboard events
fn handle_key_event(app: &mut TuiApp, key: KeyEvent) -> Action {
    // Global keybindings (work in any mode)
    match key.code {
        // Ctrl+C always quits
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
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

    // If we are in the Settings tab (index 3), override some keys for navigation
    if app.mode == TuiMode::Normal && app.active_tab == 3 {
        return handle_settings_tab(app, key);
    }

    // Mode-specific handling
    match app.mode {
        TuiMode::Normal => handle_normal_mode(app, key),
        TuiMode::Insert => handle_insert_mode(app, key),
        TuiMode::Command => handle_command_mode(app, key),
        TuiMode::Help => handle_help_mode(app, key),
        TuiMode::Confirm => handle_confirm_mode(app, key),
        TuiMode::Search => handle_search_mode(app, key),
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
            Action::Continue
        }
        // Navigation
        KeyCode::Char('j') | KeyCode::Down => Action::ScrollDown,
        KeyCode::Char('k') | KeyCode::Up => Action::ScrollUp,
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

/// Handle Insert mode keys (typing messages)
fn handle_insert_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    // Don't process keys while loading
    if app.is_loading {
        return match key.code {
            KeyCode::Esc => Action::Cancel,
            _ => Action::Continue,
        };
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
        // Scroll while in insert mode
        KeyCode::PageUp => Action::ScrollUp,
        KeyCode::PageDown => Action::ScrollDown,
        _ => Action::Continue,
    }
}

/// Handle Command mode keys (slash commands)
fn handle_command_mode(app: &mut TuiApp, key: KeyEvent) -> Action {
    match key.code {
        // Cancel command
        KeyCode::Esc => {
            app.mode = TuiMode::Insert;
            app.clear_input();
            app.command_suggestions.clear();
            Action::Continue
        }
        // Execute command
        KeyCode::Enter => {
            if !app.input.is_empty() {
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
            app.scroll_up();
            Action::Continue
        }
        MouseEventKind::ScrollDown => {
            app.scroll_down();
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

            // Check if click is in messages area
            if let Some(msg_area) = app.layout_areas.messages
                && y >= msg_area.y
                && y < msg_area.y + msg_area.height
            {
                // This is approximate - each message takes ~2 lines
                let relative_y = (y - msg_area.y) as usize;
                let msg_index = relative_y / 2; // Rough estimate
                if msg_index < app.messages.len() {
                    app.message_list_state.select(Some(msg_index));
                    app.user_scrolled = true;
                }
            }

            // Check if click is in input area - switch to insert mode
            if let Some(input_area) = app.layout_areas.input
                && y >= input_area.y
                && y < input_area.y + input_area.height
            {
                app.mode = TuiMode::Insert;
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
/// Handle keys when in the Settings tab
fn handle_settings_tab(app: &mut TuiApp, key: KeyEvent) -> Action {
    use super::app::SettingsField;

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
        // Passthrough for tab switching etc.
        _ => Action::Continue,
    }
}
