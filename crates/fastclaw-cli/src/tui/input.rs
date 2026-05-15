use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures::SinkExt;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;

use super::commands::handle_slash_command;
use super::state::*;
use super::ws::send_chat;
use super::WsTx;

// ── Key handling ────────────────────────────────────────────────────

pub(crate) async fn handle_key_event(app: &mut TuiApp, ws_tx: &mut WsTx, key: KeyEvent) {
    // Fulltext search mode: intercept keys
    if app.search_active {
        handle_search_key(app, key);
        return;
    }

    // History search mode: intercept keys
    if app.history_search_active {
        handle_history_search_key(app, key);
        return;
    }

    // Popup handling
    if app.show_popup.is_some() {
        let ask_data = if let Some(PopupKind::AskQuestion {
            ref request_id,
            ref options,
            ..
        }) = app.show_popup
        {
            Some((request_id.clone(), options.clone()))
        } else {
            None
        };

        if let Some((request_id, options)) = ask_data {
            if let KeyCode::Char(c) = key.code {
                if let Some(digit) = c.to_digit(10) {
                    let idx = (digit as usize).wrapping_sub(1);
                    if idx < options.len() {
                        let answer_id = options[idx].0.clone();
                        let answer_label = options[idx].1.clone();
                        let id = app.next_id();
                        let req = json!({
                            "id": id,
                            "method": "chat.answer",
                            "params": {"requestId": request_id, "answer": answer_id}
                        });
                        let _ = ws_tx.send(Message::Text(req.to_string())).await;
                        app.push_system(format!("Answered: {answer_label}"));
                        app.show_popup = None;
                        app.status = "Streaming...".into();
                        return;
                    }
                }
            }
            if matches!(key.code, KeyCode::Esc) {
                app.show_popup = None;
                app.status = "Question dismissed".into();
            }
            return;
        }

        // ModelPicker popup
        if matches!(app.show_popup, Some(PopupKind::ModelPicker)) {
            handle_model_picker_key(app, key);
            return;
        }

        // Non-AskQuestion popups
        if matches!(
            key.code,
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter
        ) {
            app.show_popup = None;
        }
        return;
    }

    match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('c' | 'q')) => {
            app.should_quit = true;
        }

        // Enter: send message or execute slash command
        (_, KeyCode::Enter) if !app.input.is_empty() && !app.streaming => {
            app.reset_tab();
            let text = app.input.clone();
            app.push_history(&text);

            if text.starts_with('/') {
                handle_slash_command(app, ws_tx, &text).await;
            } else {
                send_chat(app, ws_tx).await;
            }
        }

        // Shift+Enter: multi-line input (insert newline)
        (KeyModifiers::SHIFT, KeyCode::Enter) if !app.streaming => {
            let byte_pos = char_to_byte(&app.input, app.cursor_pos);
            app.input.insert(byte_pos, '\n');
            app.cursor_pos += 1;
        }

        // Alt+Enter: also multi-line input
        (KeyModifiers::ALT, KeyCode::Enter) if !app.streaming => {
            let byte_pos = char_to_byte(&app.input, app.cursor_pos);
            app.input.insert(byte_pos, '\n');
            app.cursor_pos += 1;
        }

        // Tab completion
        (_, KeyCode::Tab) if !app.streaming => {
            handle_tab_completion(app);
        }

        // Shift+Tab: toggle Plan/Agent mode
        (KeyModifiers::SHIFT, KeyCode::BackTab) if !app.streaming => {
            let new_mode = if app.execution_mode == "plan" {
                "agent"
            } else {
                "plan"
            };
            let id = app.next_id();
            let req =
                json!({"id": id, "method": "chat.set_mode", "params": {"mode": new_mode}});
            let _ = ws_tx.send(Message::Text(req.to_string())).await;
        }

        // History navigation
        (_, KeyCode::Up)
            if !app.streaming && app.input.is_empty() || app.history_index.is_some() =>
        {
            navigate_history(app, true);
        }
        (_, KeyCode::Down) if app.history_index.is_some() => {
            navigate_history(app, false);
        }

        // Scroll messages
        (KeyModifiers::SHIFT, KeyCode::Up) | (_, KeyCode::PageUp) => {
            app.scroll_offset = app.scroll_offset.saturating_add(3);
        }
        (KeyModifiers::SHIFT, KeyCode::Down) | (_, KeyCode::PageDown) => {
            app.scroll_offset = app.scroll_offset.saturating_sub(3);
        }

        // Ctrl+R: history search
        (KeyModifiers::CONTROL, KeyCode::Char('r')) if !app.streaming => {
            app.history_search_active = true;
            app.history_search_query.clear();
            app.history_search_index = None;
        }

        // Ctrl+U: clear line (readline standard)
        (KeyModifiers::CONTROL, KeyCode::Char('u')) if !app.streaming => {
            let byte_pos = char_to_byte(&app.input, app.cursor_pos);
            app.input.drain(..byte_pos);
            app.cursor_pos = 0;
            app.reset_tab();
        }

        // Ctrl+W: delete previous word
        (KeyModifiers::CONTROL, KeyCode::Char('w')) if !app.streaming => {
            if app.cursor_pos > 0 {
                let byte_pos = char_to_byte(&app.input, app.cursor_pos);
                let before = &app.input[..byte_pos];
                let trimmed = before.trim_end();
                let new_byte_pos = trimmed
                    .rfind(|c: char| c.is_whitespace())
                    .map(|i| i + 1)
                    .unwrap_or(0);
                let new_char_pos = app.input[..new_byte_pos].chars().count();
                app.input.drain(new_byte_pos..byte_pos);
                app.cursor_pos = new_char_pos;
                app.reset_tab();
            }
        }

        // Ctrl+S: stash/unstash input
        (KeyModifiers::CONTROL, KeyCode::Char('s')) if !app.streaming => {
            if let Some((stashed, pos)) = app.stashed_input.take() {
                app.input = stashed;
                app.cursor_pos = pos;
            } else if !app.input.is_empty() {
                app.stashed_input = Some((app.input.clone(), app.cursor_pos));
                app.input.clear();
                app.cursor_pos = 0;
            }
        }

        // Ctrl+K: toggle thinking collapse
        (KeyModifiers::CONTROL, KeyCode::Char('k')) => {
            app.thinking_collapsed = !app.thinking_collapsed;
        }

        // Ctrl+O: fulltext search messages
        (KeyModifiers::CONTROL, KeyCode::Char('o')) => {
            app.search_active = true;
            app.search_query.clear();
            app.search_matches.clear();
            app.search_current = 0;
        }

        // Ctrl+T: show todo list
        (KeyModifiers::CONTROL, KeyCode::Char('t')) if !app.streaming => {
            let id = app.next_id();
            let req = json!({"id": id, "method": "todo.list"});
            let _ = ws_tx.send(Message::Text(req.to_string())).await;
        }

        // Text editing
        (_, KeyCode::Char(c)) if !app.streaming => {
            app.reset_tab();
            let byte_pos = char_to_byte(&app.input, app.cursor_pos);
            app.input.insert(byte_pos, c);
            app.cursor_pos += 1;
        }
        (_, KeyCode::Backspace) if app.cursor_pos > 0 && !app.streaming => {
            app.reset_tab();
            app.cursor_pos -= 1;
            let byte_pos = char_to_byte(&app.input, app.cursor_pos);
            let next_byte = char_to_byte(&app.input, app.cursor_pos + 1);
            app.input.drain(byte_pos..next_byte);
        }
        (_, KeyCode::Delete)
            if app.cursor_pos < app.input.chars().count() && !app.streaming =>
        {
            app.reset_tab();
            let byte_pos = char_to_byte(&app.input, app.cursor_pos);
            let next_byte = char_to_byte(&app.input, app.cursor_pos + 1);
            app.input.drain(byte_pos..next_byte);
        }
        (_, KeyCode::Left) if app.cursor_pos > 0 => {
            app.cursor_pos -= 1;
        }
        (_, KeyCode::Right) if app.cursor_pos < app.input.chars().count() => {
            app.cursor_pos += 1;
        }
        (KeyModifiers::CONTROL, KeyCode::Char('a')) => {
            app.cursor_pos = 0;
        }
        (KeyModifiers::CONTROL, KeyCode::Char('e')) => {
            app.cursor_pos = app.input.chars().count();
        }
        (_, KeyCode::Home) => {
            app.cursor_pos = 0;
        }
        (_, KeyCode::End) => {
            app.cursor_pos = app.input.chars().count();
        }
        // Ctrl+L: clear screen & new session
        (KeyModifiers::CONTROL, KeyCode::Char('l')) => {
            app.messages.clear();
            app.session_id = None;
            app.scroll_offset = 0;
            app.status = "Ready".into();
        }
        // Esc handling: cancel streaming OR double-Esc to clear input
        (_, KeyCode::Esc) => {
            if app.streaming {
                if let Some(rid) = app.last_request_id.take() {
                    let cancel_id = app.next_id();
                    let cancel_req = json!({"id": cancel_id, "method": "chat.cancel", "params": {"requestId": rid}});
                    let _ = ws_tx.send(Message::Text(cancel_req.to_string())).await;
                }
                app.streaming = false;
                app.status = "Cancelled".into();
            } else {
                let now = Instant::now();
                let is_double = app
                    .last_esc_at
                    .map(|t| now.duration_since(t) < Duration::from_millis(400))
                    .unwrap_or(false);
                if is_double {
                    app.input.clear();
                    app.cursor_pos = 0;
                    app.last_esc_at = None;
                } else {
                    app.last_esc_at = Some(now);
                }
            }
        }
        _ => {}
    }
}

// ── Tab completion ──────────────────────────────────────────────────

pub(crate) fn handle_tab_completion(app: &mut TuiApp) {
    if app.tab_completions.is_empty() {
        let prefix = app.input.clone();
        let mut completions: Vec<String> = Vec::new();

        if prefix.starts_with('/') {
            for (cmd, _) in SLASH_COMMANDS {
                if cmd.starts_with(&prefix) {
                    completions.push(cmd.to_string());
                }
            }

            if prefix.starts_with("/agent ") {
                let agent_prefix = prefix.strip_prefix("/agent ").unwrap_or("");
                for a in &app.agents {
                    if a.id.starts_with(agent_prefix) {
                        completions.push(format!("/agent {}", a.id));
                    }
                }
            }
        }

        if completions.is_empty() {
            return;
        }

        app.tab_prefix = prefix;
        app.tab_completions = completions;
        app.tab_index = 0;
    } else {
        app.tab_index = (app.tab_index + 1) % app.tab_completions.len();
    }

    if let Some(completion) = app.tab_completions.get(app.tab_index) {
        app.input = completion.clone();
        app.cursor_pos = app.input.chars().count();
    }
}

// ── History navigation ──────────────────────────────────────────────

pub(crate) fn navigate_history(app: &mut TuiApp, up: bool) {
    if app.input_history.is_empty() {
        return;
    }

    if up {
        match app.history_index {
            None => {
                app.history_stash = app.input.clone();
                app.history_index = Some(app.input_history.len() - 1);
            }
            Some(0) => return,
            Some(i) => {
                app.history_index = Some(i - 1);
            }
        }
    } else {
        match app.history_index {
            Some(i) if i + 1 < app.input_history.len() => {
                app.history_index = Some(i + 1);
            }
            Some(_) => {
                app.input = app.history_stash.clone();
                app.cursor_pos = app.input.chars().count();
                app.history_index = None;
                return;
            }
            None => return,
        }
    }

    if let Some(i) = app.history_index {
        if let Some(entry) = app.input_history.get(i) {
            app.input = entry.clone();
            app.cursor_pos = app.input.chars().count();
        }
    }
}

// ── History search (Ctrl+R) ─────────────────────────────────────────

fn handle_history_search_key(app: &mut TuiApp, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.history_search_active = false;
            app.history_search_query.clear();
            app.history_search_index = None;
        }
        KeyCode::Enter => {
            // Accept current search result
            app.history_search_active = false;
            app.history_search_query.clear();
            app.history_search_index = None;
            app.cursor_pos = app.input.chars().count();
        }
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            // Search backwards again
            search_history_backwards(app);
        }
        KeyCode::Backspace => {
            app.history_search_query.pop();
            search_history_from_start(app);
        }
        KeyCode::Char(c) => {
            app.history_search_query.push(c);
            search_history_from_start(app);
        }
        _ => {}
    }
}

pub(super) fn search_history_from_start(app: &mut TuiApp) {
    if app.history_search_query.is_empty() {
        app.history_search_index = None;
        return;
    }
    let query = app.history_search_query.to_lowercase();
    for (i, entry) in app.input_history.iter().enumerate().rev() {
        if entry.to_lowercase().contains(&query) {
            app.history_search_index = Some(i);
            app.input = entry.clone();
            app.cursor_pos = app.input.chars().count();
            return;
        }
    }
    app.history_search_index = None;
}

fn search_history_backwards(app: &mut TuiApp) {
    if app.history_search_query.is_empty() {
        return;
    }
    let query = app.history_search_query.to_lowercase();
    let start = app
        .history_search_index
        .map(|i| i.saturating_sub(1))
        .unwrap_or(app.input_history.len().saturating_sub(1));
    for i in (0..=start).rev() {
        if app.input_history[i].to_lowercase().contains(&query) {
            app.history_search_index = Some(i);
            app.input = app.input_history[i].clone();
            app.cursor_pos = app.input.chars().count();
            return;
        }
    }
}

fn handle_search_key(app: &mut TuiApp, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.search_active = false;
            app.search_query.clear();
            app.search_matches.clear();
        }
        KeyCode::Enter => {
            app.search_active = false;
        }
        KeyCode::Backspace => {
            app.search_query.pop();
            update_search_matches(app);
        }
        KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if !app.search_matches.is_empty() {
                app.search_current = (app.search_current + 1) % app.search_matches.len();
            }
        }
        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if !app.search_matches.is_empty() {
                app.search_current = if app.search_current == 0 {
                    app.search_matches.len() - 1
                } else {
                    app.search_current - 1
                };
            }
        }
        KeyCode::Down => {
            if !app.search_matches.is_empty() {
                app.search_current = (app.search_current + 1) % app.search_matches.len();
            }
        }
        KeyCode::Up => {
            if !app.search_matches.is_empty() {
                app.search_current = if app.search_current == 0 {
                    app.search_matches.len() - 1
                } else {
                    app.search_current - 1
                };
            }
        }
        KeyCode::Char(c) => {
            app.search_query.push(c);
            update_search_matches(app);
        }
        _ => {}
    }
}

fn update_search_matches(app: &mut TuiApp) {
    app.search_matches.clear();
    app.search_current = 0;
    if app.search_query.is_empty() {
        return;
    }
    let query = app.search_query.to_lowercase();
    for (i, msg) in app.messages.iter().enumerate() {
        if msg.content.to_lowercase().contains(&query) {
            app.search_matches.push(i);
        }
    }
}

fn handle_model_picker_key(app: &mut TuiApp, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.show_popup = None;
            app.select_state = None;
        }
        KeyCode::Up => {
            if let Some(ref mut sel) = app.select_state {
                sel.move_up();
            }
        }
        KeyCode::Down => {
            if let Some(ref mut sel) = app.select_state {
                sel.move_down();
            }
        }
        KeyCode::Enter => {
            let chosen = app
                .select_state
                .as_ref()
                .and_then(|s| s.selected_item())
                .map(|item| item.id.clone());
            if let Some(model_id) = chosen {
                app.model_override = model_id.clone();
                app.current_model = model_id.clone();
                app.push_system(format!("Model switched to: {model_id}"));
            }
            app.show_popup = None;
            app.select_state = None;
        }
        KeyCode::Backspace => {
            if let Some(ref mut sel) = app.select_state {
                sel.filter.pop();
                sel.apply_filter();
            }
        }
        KeyCode::Char(c) => {
            if let Some(ref mut sel) = app.select_state {
                sel.filter.push(c);
                sel.apply_filter();
            }
        }
        _ => {}
    }
}
