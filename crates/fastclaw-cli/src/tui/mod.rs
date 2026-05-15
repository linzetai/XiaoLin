mod commands;
mod input;
mod markdown;
mod render;
pub(crate) mod state;
mod widgets;
mod ws;

use std::io::IsTerminal;
use std::time::Duration;

use crossterm::event::{self, Event};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

pub use state::TuiApp;
use state::*;

// ── WebSocket type aliases ──────────────────────────────────────────

pub(crate) type WsTx = futures::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;
pub(crate) type WsRx = futures::stream::SplitStream<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
>;

// ── Entry point ─────────────────────────────────────────────────────

pub async fn run_tui(
    url: &str,
    token: Option<&str>,
    session: Option<&str>,
    work_dir: Option<String>,
    config_mode: &fastclaw_core::config::ConfigMode,
) -> anyhow::Result<()> {
    if !std::io::stdout().is_terminal() {
        anyhow::bail!("TUI requires an interactive terminal (TTY). Use --help for options.");
    }

    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen)?;

    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)?;

    let mut app = TuiApp::new(
        url.to_string(),
        token.map(String::from),
        session.map(String::from),
    );
    app.work_dir = work_dir;
    app.config_mode = config_mode.clone();

    let ws_url = {
        let mut u = app.ws_url.clone();
        if let Some(key) = &app.api_key {
            u.push_str(&format!(
                "{}token={}",
                if u.contains('?') { "&" } else { "?" },
                key,
            ));
        }
        u
    };

    let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "Failed to connect to gateway at {url}: {e}\n\n\
                 Troubleshooting:\n  \
                 • Run `fastclaw tui` (auto-starts gateway)\n  \
                 • Or manually: `fastclaw serve` in another terminal\n  \
                 • Check if port is in use: `ss -tlnp | grep {port}`",
                url = app.ws_url,
                port = app
                    .ws_url
                    .split(':')
                    .nth(2)
                    .unwrap_or("18789")
                    .split('/')
                    .next()
                    .unwrap_or("18789"),
            )
        })?;
    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    app.connected = true;
    app.status = "Connected".into();

    let agents_req = json!({"id": "init-agents", "method": "agents"});
    let _ = ws_tx.send(Message::Text(agents_req.to_string())).await;

    for _ in 0..5 {
        if let Some(Ok(Message::Text(text))) = ws_rx.next().await {
            if let Ok(msg) = serde_json::from_str::<Value>(&text) {
                if msg.get("type").and_then(|v| v.as_str()) == Some("agents") {
                    if let Some(arr) = msg["data"]["agents"].as_array() {
                        for a in arr {
                            let id = a["agentId"].as_str().unwrap_or("").to_string();
                            let name = a["name"].as_str().unwrap_or("").to_string();
                            let model = a["model"].as_str().unwrap_or("").to_string();
                            app.agents.push(AgentInfo { id, name, model });
                        }
                        if let Some(first) = app.agents.first() {
                            app.agent_id = first.id.clone();
                            app.current_model = first.model.clone();
                        }
                    }
                    break;
                }
            }
        }
    }

    app.push_system("Welcome to FastClaw TUI! Type /help for commands.".into());
    commands::run_preflight_checks(&mut app);

    let result = run_event_loop(&mut terminal, &mut app, &mut ws_tx, &mut ws_rx).await;

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;

    result
}

// ── Main event loop ─────────────────────────────────────────────────

async fn run_event_loop(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    app: &mut TuiApp,
    ws_tx: &mut WsTx,
    ws_rx: &mut WsRx,
) -> anyhow::Result<()> {
    let mut tick_interval = tokio::time::interval(Duration::from_millis(80));
    tick_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        terminal.draw(|f| render::draw_ui(f, app))?;

        if app.should_quit {
            break;
        }

        tokio::select! {
            biased;

            ws_msg = ws_rx.next() => {
                match ws_msg {
                    Some(Ok(Message::Text(text))) => {
                        ws::handle_ws_message(app, &text);
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        app.connected = false;
                        app.status = "Disconnected — gateway closed connection".into();
                        app.messages.push(ChatMsg {
                            role: "system".into(),
                            content: "Connection lost. Press Ctrl+C to exit or restart with `fastclaw tui`.".into(),
                            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                        });
                        break;
                    }
                    _ => {}
                }
            }

            has_event = poll_crossterm_event() => {
                if has_event {
                    if let Ok(Event::Key(key)) = event::read() {
                        input::handle_key_event(app, ws_tx, key).await;
                    }
                }
            }

            _ = tick_interval.tick() => {
                if app.streaming {
                    app.spinner.tick();
                    if let Some(start) = app.chat_start_time {
                        let elapsed = start.elapsed();
                        if elapsed.as_secs() >= 120 && !app.timeout_warned {
                            app.timeout_warned = true;
                            app.push_system(
                                "Request has been running for over 2 minutes. Press Ctrl+C to cancel.".into(),
                            );
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

async fn poll_crossterm_event() -> bool {
    tokio::task::spawn_blocking(|| event::poll(Duration::from_millis(30)).unwrap_or(false))
        .await
        .unwrap_or(false)
}

// ══════════════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};
    use std::time::{Duration, Instant};
    use unicode_width::UnicodeWidthStr;

    use input::{handle_tab_completion, navigate_history};
    use markdown::{parse_inline_markdown, render_markdown_lines};
    use render::{draw_input, draw_messages, draw_status_bar, draw_title_bar, draw_ui};
    use widgets::draw_popup;
    use ws::handle_ws_message;

    fn new_app() -> TuiApp {
        TuiApp::new("ws://127.0.0.1:9999/ws".into(), None, None)
    }

    // ── format_elapsed ──────────────────────────────────────────────

    #[test]
    fn format_elapsed_milliseconds() {
        assert_eq!(format_elapsed(0), "0ms");
        assert_eq!(format_elapsed(1), "1ms");
        assert_eq!(format_elapsed(999), "999ms");
    }

    #[test]
    fn format_elapsed_seconds() {
        assert_eq!(format_elapsed(1000), "1.0s");
        assert_eq!(format_elapsed(1500), "1.5s");
        assert_eq!(format_elapsed(59999), "60.0s");
    }

    #[test]
    fn format_elapsed_minutes() {
        assert_eq!(format_elapsed(60000), "1m0s");
        assert_eq!(format_elapsed(65000), "1m5s");
        assert_eq!(format_elapsed(125000), "2m5s");
    }

    // ── Tab completion ──────────────────────────────────────────────

    #[test]
    fn tab_completion_matches_prefix() {
        let mut app = new_app();
        app.input = "/he".into();
        app.cursor_pos = 3;
        handle_tab_completion(&mut app);
        assert_eq!(app.input, "/help");
        assert_eq!(app.cursor_pos, 5);
    }

    #[test]
    fn tab_completion_no_match() {
        let mut app = new_app();
        app.input = "/zzz".into();
        app.cursor_pos = 4;
        handle_tab_completion(&mut app);
        assert_eq!(app.input, "/zzz");
    }

    #[test]
    fn tab_completion_cycles() {
        let mut app = new_app();
        app.input = "/".into();
        app.cursor_pos = 1;
        handle_tab_completion(&mut app);
        let first = app.input.clone();
        handle_tab_completion(&mut app);
        let second = app.input.clone();
        assert_ne!(first, second);
    }

    #[test]
    fn tab_completion_agent_subcommand() {
        let mut app = new_app();
        app.agents.push(AgentInfo {
            id: "coder".into(),
            name: "Coder".into(),
            model: "test".into(),
        });
        app.input = "/agent c".into();
        app.cursor_pos = 8;
        handle_tab_completion(&mut app);
        assert_eq!(app.input, "/agent coder");
    }

    // ── History navigation ──────────────────────────────────────────

    #[test]
    fn history_up_shows_last_entry() {
        let mut app = new_app();
        app.input_history = vec!["first".into(), "second".into()];
        app.input = "current".into();
        navigate_history(&mut app, true);
        assert_eq!(app.input, "second");
        assert_eq!(app.history_index, Some(1));
    }

    #[test]
    fn history_up_up_shows_older() {
        let mut app = new_app();
        app.input_history = vec!["first".into(), "second".into()];
        app.input = "current".into();
        navigate_history(&mut app, true);
        navigate_history(&mut app, true);
        assert_eq!(app.input, "first");
        assert_eq!(app.history_index, Some(0));
    }

    #[test]
    fn history_up_at_top_stays() {
        let mut app = new_app();
        app.input_history = vec!["only".into()];
        app.input = "current".into();
        navigate_history(&mut app, true);
        navigate_history(&mut app, true);
        assert_eq!(app.input, "only");
    }

    #[test]
    fn history_down_restores_stash() {
        let mut app = new_app();
        app.input_history = vec!["first".into(), "second".into()];
        app.input = "typing".into();
        navigate_history(&mut app, true);
        assert_eq!(app.input, "second");
        navigate_history(&mut app, false);
        assert_eq!(app.input, "typing");
        assert_eq!(app.history_index, None);
    }

    #[test]
    fn history_empty_does_nothing() {
        let mut app = new_app();
        app.input = "hello".into();
        navigate_history(&mut app, true);
        assert_eq!(app.input, "hello");
    }

    // ── push_history ────────────────────────────────────────────────

    #[test]
    fn push_history_deduplicates_consecutive() {
        let mut app = new_app();
        app.push_history("hello");
        app.push_history("hello");
        assert_eq!(app.input_history.len(), 1);
    }

    #[test]
    fn push_history_ignores_empty() {
        let mut app = new_app();
        app.push_history("");
        assert!(app.input_history.is_empty());
    }

    // ── handle_ws_message ───────────────────────────────────────────

    #[test]
    fn ws_message_connected() {
        let mut app = new_app();
        handle_ws_message(&mut app, r#"{"type":"connected"}"#);
        assert_eq!(app.status, "Connected");
    }

    #[test]
    fn ws_message_chat_start_sets_session() {
        let mut app = new_app();
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.start","data":{"sessionId":"sess-123"}}"#,
        );
        assert_eq!(app.session_id, Some("sess-123".into()));
        assert_eq!(app.spinner.verb, "thinking");
    }

    #[test]
    fn ws_message_chat_delta_appends_content() {
        let mut app = new_app();
        app.messages.push(ChatMsg {
            role: "assistant".into(),
            content: "Hello".into(),
            timestamp: "00:00:00".into(),
        });
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.delta","data":{"content":" world"}}"#,
        );
        assert_eq!(app.messages.last().unwrap().content, "Hello world");
    }

    #[test]
    fn ws_message_chat_delta_ignores_non_assistant() {
        let mut app = new_app();
        app.messages.push(ChatMsg {
            role: "user".into(),
            content: "question".into(),
            timestamp: "00:00:00".into(),
        });
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.delta","data":{"content":"extra"}}"#,
        );
        assert_eq!(app.messages.last().unwrap().content, "question");
    }

    #[test]
    fn ws_message_chat_tool_start() {
        let mut app = new_app();
        app.messages.push(ChatMsg {
            role: "assistant".into(),
            content: String::new(),
            timestamp: "00:00:00".into(),
        });
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.tool.start","data":{"tool":"file_read","params":{"path":"/tmp/x"}}}"#,
        );
        assert!(app.messages.last().unwrap().content.contains("Read file"));
        assert!(app.messages.last().unwrap().content.contains("/tmp/x"));
        assert_eq!(app.spinner.tool_name, Some("file_read".into()));
    }

    #[test]
    fn ws_message_chat_tool_done() {
        let mut app = new_app();
        app.messages.push(ChatMsg {
            role: "assistant".into(),
            content: String::new(),
            timestamp: "00:00:00".into(),
        });
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.tool.done","data":{"success":true,"elapsedMs":150}}"#,
        );
        let content = &app.messages.last().unwrap().content;
        assert!(content.contains("✓"));
        assert!(content.contains("150ms"));
    }

    #[test]
    fn ws_message_chat_tool_done_failure() {
        let mut app = new_app();
        app.messages.push(ChatMsg {
            role: "assistant".into(),
            content: String::new(),
            timestamp: "00:00:00".into(),
        });
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.tool.done","data":{"success":false,"elapsedMs":50}}"#,
        );
        assert!(app.messages.last().unwrap().content.contains("✗"));
    }

    #[test]
    fn ws_message_context_usage() {
        let mut app = new_app();
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.context_usage","data":{"usedTokens":5000,"limitTokens":128000}}"#,
        );
        assert_eq!(app.ctx_used_tokens, 5000);
        assert_eq!(app.ctx_limit_tokens, 128000);
    }

    #[test]
    fn ws_message_chat_complete_updates_stats() {
        let mut app = new_app();
        app.messages.push(ChatMsg {
            role: "assistant".into(),
            content: "response".into(),
            timestamp: "00:00:00".into(),
        });
        app.streaming = true;
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.complete","data":{"elapsedMs":2000,"inputTokensEstimate":100,"outputTokensEstimate":50}}"#,
        );
        assert!(!app.streaming);
        assert_eq!(app.last_elapsed_ms, Some(2000));
        assert_eq!(app.total_input_tokens, 100);
        assert_eq!(app.total_output_tokens, 50);
        assert_eq!(app.total_messages, 1);
    }

    #[test]
    fn ws_message_chat_error() {
        let mut app = new_app();
        app.messages.push(ChatMsg {
            role: "assistant".into(),
            content: String::new(),
            timestamp: "00:00:00".into(),
        });
        app.streaming = true;
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.error","error":{"message":"rate limited"}}"#,
        );
        assert!(!app.streaming);
        assert!(app.status.contains("rate limited"));
        assert!(app.messages.last().unwrap().content.contains("rate limited"));
    }

    #[test]
    fn ws_message_invalid_json_ignored() {
        let mut app = new_app();
        handle_ws_message(&mut app, "not json at all");
        assert_eq!(app.status, "Connecting...");
    }

    // ── parse_inline_markdown ───────────────────────────────────────

    #[test]
    fn markdown_plain_text() {
        let spans = parse_inline_markdown("hello world");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "hello world");
    }

    #[test]
    fn markdown_inline_code() {
        let spans = parse_inline_markdown("use `foo` here");
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content, "use ");
        assert_eq!(spans[1].content, "foo");
        assert_eq!(spans[2].content, " here");
    }

    #[test]
    fn markdown_bold() {
        let spans = parse_inline_markdown("this is **bold** text");
        assert!(spans.iter().any(|s| s.content == "bold"));
    }

    #[test]
    fn markdown_italic() {
        let spans = parse_inline_markdown("this is *italic* text");
        assert!(spans.iter().any(|s| s.content == "italic"));
    }

    #[test]
    fn markdown_unclosed_backtick() {
        let spans = parse_inline_markdown("unclosed `code");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content, "unclosed ");
        assert_eq!(spans[1].content, "`code");
    }

    // ── render_markdown_lines ───────────────────────────────────────

    #[test]
    fn markdown_code_block() {
        let content = "```rust\nfn main() {}\n```";
        let mut lines = Vec::new();
        render_markdown_lines(content, &mut lines, false);
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn markdown_heading() {
        let content = "# Title\n## Subtitle\n### Section";
        let mut lines = Vec::new();
        render_markdown_lines(content, &mut lines, false);
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn markdown_bullet_list() {
        let content = "- item one\n- item two";
        let mut lines = Vec::new();
        render_markdown_lines(content, &mut lines, false);
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn markdown_numbered_list() {
        let content = "1. first\n2. second";
        let mut lines = Vec::new();
        render_markdown_lines(content, &mut lines, false);
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn markdown_streaming_cursor() {
        let content = "```\nincomplete";
        let mut lines = Vec::new();
        render_markdown_lines(content, &mut lines, true);
        let last_text: String = lines
            .last()
            .unwrap()
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(last_text.contains("▌"));
    }

    // ── SpinnerState ────────────────────────────────────────────────

    #[test]
    fn spinner_set_thinking() {
        let mut s = SpinnerState::new();
        s.set_tool("file_read");
        assert_eq!(s.tool_name, Some("file_read".into()));
        s.set_thinking();
        assert_eq!(s.tool_name, None);
        assert_eq!(s.verb, "thinking");
    }

    #[test]
    fn spinner_set_tool_maps_verb() {
        let mut s = SpinnerState::new();
        s.set_tool("shell_exec");
        assert_eq!(s.verb, "running command");
        s.set_tool("web_search");
        assert_eq!(s.verb, "searching web");
        s.set_tool("unknown_tool");
        assert_eq!(s.verb, "running tool");
    }

    #[test]
    fn spinner_tick_cycles() {
        let mut s = SpinnerState::new();
        for _ in 0..20 {
            s.tick();
        }
        assert_eq!(s.frame, 20 % SPINNER_FRAMES.len());
    }

    // ── Snapshot tests ──────────────────────────────────────────────

    #[test]
    fn snapshot_status_bar_agent_mode() {
        let app = new_app();
        let mut terminal = Terminal::new(TestBackend::new(100, 1)).unwrap();
        terminal
            .draw(|f| {
                draw_status_bar(f, &app, f.area());
            })
            .unwrap();
        insta::assert_snapshot!("status_bar_agent", terminal.backend().to_string());
    }

    #[test]
    fn snapshot_status_bar_plan_mode() {
        let mut app = new_app();
        app.execution_mode = "plan".into();
        app.connected = true;
        app.ctx_used_tokens = 50000;
        app.ctx_limit_tokens = 128000;
        let mut terminal = Terminal::new(TestBackend::new(100, 1)).unwrap();
        terminal
            .draw(|f| {
                draw_status_bar(f, &app, f.area());
            })
            .unwrap();
        insta::assert_snapshot!("status_bar_plan", terminal.backend().to_string());
    }

    #[test]
    fn snapshot_input_empty() {
        let app = new_app();
        let mut terminal = Terminal::new(TestBackend::new(80, 3)).unwrap();
        terminal
            .draw(|f| {
                draw_input(f, &app, f.area());
            })
            .unwrap();
        insta::assert_snapshot!("input_empty", terminal.backend().to_string());
    }

    #[test]
    fn snapshot_input_with_text() {
        let mut app = new_app();
        app.input = "hello world".into();
        app.cursor_pos = 11;
        app.connected = true;
        let mut terminal = Terminal::new(TestBackend::new(80, 3)).unwrap();
        terminal
            .draw(|f| {
                draw_input(f, &app, f.area());
            })
            .unwrap();
        insta::assert_snapshot!("input_with_text", terminal.backend().to_string());
    }

    #[test]
    fn snapshot_input_slash_suggestion() {
        let mut app = new_app();
        app.input = "/he".into();
        app.cursor_pos = 3;
        app.connected = true;
        let mut terminal = Terminal::new(TestBackend::new(80, 5)).unwrap();
        terminal
            .draw(|f| {
                draw_input(f, &app, f.area());
            })
            .unwrap();
        insta::assert_snapshot!("input_slash_suggestion", terminal.backend().to_string());
    }

    #[test]
    fn snapshot_messages_mixed() {
        let mut app = new_app();
        app.messages = vec![
            ChatMsg {
                role: "user".into(),
                content: "Hello".into(),
                timestamp: "10:00:00".into(),
            },
            ChatMsg {
                role: "assistant".into(),
                content: "Hi there! How can I help?".into(),
                timestamp: "10:00:01".into(),
            },
            ChatMsg {
                role: "system".into(),
                content: "Session started".into(),
                timestamp: "10:00:02".into(),
            },
        ];
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal
            .draw(|f| {
                draw_messages(f, &app, f.area());
            })
            .unwrap();
        insta::assert_snapshot!("messages_mixed", terminal.backend().to_string());
    }

    #[test]
    fn snapshot_title_bar() {
        let mut app = new_app();
        app.connected = true;
        app.session_id = Some("sess-abc123".into());
        let mut terminal = Terminal::new(TestBackend::new(80, 1)).unwrap();
        terminal
            .draw(|f| {
                draw_title_bar(f, &app, f.area());
            })
            .unwrap();
        insta::assert_snapshot!("title_bar", terminal.backend().to_string());
    }

    #[test]
    fn snapshot_popup_help() {
        let app = new_app();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| {
                draw_popup(f, &PopupKind::Help, &app.agents, None);
            })
            .unwrap();
        insta::assert_snapshot!("popup_help", terminal.backend().to_string());
    }

    // ── Unicode / CJK input tests ───────────────────────────────────

    fn simulate_char_input(app: &mut TuiApp, c: char) {
        let byte_pos = char_to_byte(&app.input, app.cursor_pos);
        app.input.insert(byte_pos, c);
        app.cursor_pos += 1;
    }

    fn simulate_backspace(app: &mut TuiApp) {
        if app.cursor_pos > 0 {
            app.cursor_pos -= 1;
            let byte_pos = char_to_byte(&app.input, app.cursor_pos);
            let next_byte = char_to_byte(&app.input, app.cursor_pos + 1);
            app.input.drain(byte_pos..next_byte);
        }
    }

    #[test]
    fn chinese_input_insert_no_panic() {
        let mut app = new_app();
        simulate_char_input(&mut app, '你');
        simulate_char_input(&mut app, '好');
        simulate_char_input(&mut app, '世');
        simulate_char_input(&mut app, '界');
        assert_eq!(app.input, "你好世界");
        assert_eq!(app.cursor_pos, 4);
    }

    #[test]
    fn chinese_input_backspace_removes_whole_char() {
        let mut app = new_app();
        simulate_char_input(&mut app, '你');
        simulate_char_input(&mut app, '好');
        assert_eq!(app.input, "你好");
        simulate_backspace(&mut app);
        assert_eq!(app.input, "你");
        assert_eq!(app.cursor_pos, 1);
        simulate_backspace(&mut app);
        assert_eq!(app.input, "");
        assert_eq!(app.cursor_pos, 0);
    }

    #[test]
    fn chinese_input_left_right_cursor() {
        let mut app = new_app();
        simulate_char_input(&mut app, '你');
        simulate_char_input(&mut app, '好');
        assert_eq!(app.cursor_pos, 2);
        app.cursor_pos -= 1;
        assert_eq!(app.cursor_pos, 1);
        simulate_char_input(&mut app, '的');
        assert_eq!(app.input, "你的好");
        assert_eq!(app.cursor_pos, 2);
    }

    #[test]
    fn mixed_ascii_cjk_input() {
        let mut app = new_app();
        for c in "hi你好".chars() {
            simulate_char_input(&mut app, c);
        }
        assert_eq!(app.input, "hi你好");
        assert_eq!(app.cursor_pos, 4);
        simulate_backspace(&mut app);
        assert_eq!(app.input, "hi你");
        assert_eq!(app.cursor_pos, 3);
    }

    #[test]
    fn emoji_input() {
        let mut app = new_app();
        simulate_char_input(&mut app, '🎉');
        simulate_char_input(&mut app, '🚀');
        assert_eq!(app.input, "🎉🚀");
        assert_eq!(app.cursor_pos, 2);
        simulate_backspace(&mut app);
        assert_eq!(app.input, "🎉");
        assert_eq!(app.cursor_pos, 1);
    }

    #[test]
    fn cursor_pos_is_always_valid_char_boundary() {
        let mut app = new_app();
        for c in "abc你好def世界".chars() {
            simulate_char_input(&mut app, c);
        }
        assert_eq!(app.cursor_pos, app.input.chars().count());
        for _ in 0..app.input.chars().count() {
            app.cursor_pos -= 1;
            let byte_pos = char_to_byte(&app.input, app.cursor_pos);
            assert!(
                app.input.is_char_boundary(byte_pos),
                "cursor_pos {} maps to byte {} which is not a char boundary",
                app.cursor_pos,
                byte_pos
            );
        }
    }

    #[test]
    fn ctrl_w_with_chinese() {
        let mut app = new_app();
        app.input = "你好 世界".into();
        app.cursor_pos = app.input.chars().count();
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
        assert_eq!(app.input, "你好 ");
        assert_eq!(app.cursor_pos, 3);
    }

    #[test]
    fn display_width_cjk_chars() {
        assert_eq!(display_width_chars("abc", 3), 3);
        assert_eq!(display_width_chars("你好", 2), 4);
        assert_eq!(display_width_chars("hi你好", 4), 6);
        assert_eq!(display_width_chars("❯", 1), 1);
    }

    #[test]
    fn char_to_byte_conversion() {
        let s = "你好世界";
        assert_eq!(char_to_byte(s, 0), 0);
        assert_eq!(char_to_byte(s, 1), 3);
        assert_eq!(char_to_byte(s, 2), 6);
        assert_eq!(char_to_byte(s, 4), 12);
        let s2 = "hi你好";
        assert_eq!(char_to_byte(s2, 0), 0);
        assert_eq!(char_to_byte(s2, 1), 1);
        assert_eq!(char_to_byte(s2, 2), 2);
        assert_eq!(char_to_byte(s2, 3), 5);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Integration-level state machine tests
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn integration_full_chat_flow() {
        let mut app = new_app();
        app.streaming = true;
        app.chat_start_time = Some(Instant::now());
        app.messages.push(ChatMsg {
            role: "assistant".into(),
            content: String::new(),
            timestamp: "12:00:00".into(),
        });
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.start","data":{"sessionId":"s1","model":"gpt-4o"}}"#,
        );
        assert_eq!(app.session_id, Some("s1".into()));
        assert!(app.streaming);
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.delta","data":{"content":"Hello "}}"#,
        );
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.delta","data":{"content":"world!"}}"#,
        );
        assert_eq!(app.messages.last().unwrap().content, "Hello world!");
        handle_ws_message(&mut app, r#"{"type":"chat.complete","data":{"elapsedMs":1200,"inputTokensEstimate":50,"outputTokensEstimate":25}}"#);
        assert!(!app.streaming);
        assert_eq!(app.total_messages, 1);
        assert_eq!(app.total_input_tokens, 50);
        assert_eq!(app.total_output_tokens, 25);
    }

    #[test]
    fn integration_tool_call_flow() {
        let mut app = new_app();
        app.streaming = true;
        app.messages.push(ChatMsg {
            role: "assistant".into(),
            content: String::new(),
            timestamp: "12:00:00".into(),
        });
        handle_ws_message(&mut app, r#"{"type":"chat.tool.start","data":{"tool":"file_read","callId":"c1","params":{"path":"/tmp/test.rs"}}}"#);
        assert_eq!(app.spinner.verb, "reading");
        let content = &app.messages.last().unwrap().content;
        assert!(content.contains("Read file"));
        assert!(content.contains("/tmp/test.rs"));
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.tool.progress","data":{"content":"reading file..."}}"#,
        );
        let content = &app.messages.last().unwrap().content;
        assert!(content.contains("reading file..."));
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.tool.done","data":{"success":true,"elapsedMs":150}}"#,
        );
        assert_eq!(app.spinner.verb, "thinking");
        let content = &app.messages.last().unwrap().content;
        assert!(content.contains("✓"));
    }

    #[test]
    fn integration_subagent_flow() {
        let mut app = new_app();
        app.streaming = true;
        app.messages.push(ChatMsg {
            role: "assistant".into(),
            content: String::new(),
            timestamp: "12:00:00".into(),
        });
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.subagent.start","data":{"runId":"r1","label":"code-review"}}"#,
        );
        assert!(app.spinner.verb.contains("code-review"));
        assert!(app.messages.last().unwrap().content.contains("Sub-agent"));
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.subagent.delta","data":{"runId":"r1","content":"Reviewing..."}}"#,
        );
        assert!(app.messages.last().unwrap().content.contains("Reviewing..."));
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.subagent.tool.start","data":{"runId":"r1","tool":"grep"}}"#,
        );
        assert!(app.spinner.verb.contains("grep"));
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.subagent.tool.done","data":{"runId":"r1"}}"#,
        );
        assert_eq!(app.spinner.verb, "thinking");
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.subagent.complete","data":{"runId":"r1","elapsedMs":3000}}"#,
        );
        assert_eq!(app.spinner.verb, "thinking");
    }

    #[test]
    fn integration_context_warnings() {
        let mut app = new_app();
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.context.warning","data":{"usedPercent":85}}"#,
        );
        assert!(app.messages.iter().any(|m| m.content.contains("85%")));
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.compact.warning","data":{}}"#,
        );
        assert!(app.messages.iter().any(|m| m.content.contains("compacted")));
    }

    #[test]
    fn integration_context_usage() {
        let mut app = new_app();
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.context_usage","data":{"usedTokens":5000,"limitTokens":128000}}"#,
        );
        assert_eq!(app.ctx_used_tokens, 5000);
        assert_eq!(app.ctx_limit_tokens, 128000);
    }

    #[test]
    fn integration_suggestions() {
        let mut app = new_app();
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.suggestions","data":{"items":["fix the bug","add tests","refactor"]}}"#,
        );
        assert!(app
            .messages
            .iter()
            .any(|m| m.content.contains("fix the bug")));
    }

    #[test]
    fn integration_chat_error_during_streaming() {
        let mut app = new_app();
        app.streaming = true;
        app.messages.push(ChatMsg {
            role: "assistant".into(),
            content: String::new(),
            timestamp: "12:00:00".into(),
        });
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.error","error":{"message":"rate limit exceeded"}}"#,
        );
        assert!(!app.streaming);
        assert!(app.status.contains("rate limit"));
        assert!(app
            .messages
            .last()
            .unwrap()
            .content
            .contains("rate limit"));
    }

    #[test]
    fn integration_generic_error() {
        let mut app = new_app();
        handle_ws_message(
            &mut app,
            r#"{"type":"error","error":{"code":401,"message":"invalid token"}}"#,
        );
        assert!(app.status.contains("Auth error"));
    }

    #[test]
    fn integration_error_403() {
        let mut app = new_app();
        handle_ws_message(
            &mut app,
            r#"{"type":"error","error":{"code":403,"message":"forbidden"}}"#,
        );
        assert!(app.status.contains("Access denied"));
    }

    #[test]
    fn integration_mode_switch() {
        let mut app = new_app();
        assert_eq!(app.execution_mode, "agent");
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.set_mode","data":{"ok":true,"from":"agent","to":"plan"}}"#,
        );
        assert_eq!(app.execution_mode, "plan");
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.mode_change","data":{"to":"agent"}}"#,
        );
        assert_eq!(app.execution_mode, "agent");
    }

    #[test]
    fn integration_plan_file() {
        let mut app = new_app();
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.plan_file","data":{"path":"/tmp/plan.md","exists":true}}"#,
        );
        assert_eq!(app.plan_file_path, Some("/tmp/plan.md".into()));
        assert!(app.plan_file_exists);
    }

    #[test]
    fn integration_ask_question() {
        let mut app = new_app();
        handle_ws_message(&mut app, r#"{"type":"chat.ask_question","data":{"requestId":"q1","question":"Proceed?","options":[{"id":"yes","label":"Yes"},{"id":"no","label":"No"}]}}"#);
        assert!(app.show_popup.is_some());
        match &app.show_popup {
            Some(PopupKind::AskQuestion {
                request_id,
                question,
                options,
            }) => {
                assert_eq!(request_id, "q1");
                assert_eq!(question, "Proceed?");
                assert_eq!(options.len(), 2);
            }
            _ => panic!("expected AskQuestion popup"),
        }
    }

    #[test]
    fn integration_cancel_confirmed() {
        let mut app = new_app();
        app.streaming = true;
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.cancel","data":{"cancelled":true}}"#,
        );
        assert!(!app.streaming);
        assert!(app.status.contains("Cancelled"));
    }

    #[test]
    fn integration_invalid_json_ignored() {
        let mut app = new_app();
        handle_ws_message(&mut app, "this is not json");
        handle_ws_message(&mut app, "");
        handle_ws_message(&mut app, "{broken json");
        assert_eq!(app.status, "Connecting...");
    }

    #[test]
    fn integration_unknown_type_ignored() {
        let mut app = new_app();
        handle_ws_message(
            &mut app,
            r#"{"type":"some.future.type","data":{"foo":"bar"}}"#,
        );
        assert_eq!(app.status, "Connecting...");
    }

    #[test]
    fn integration_heartbeat_and_pong_ignored() {
        let mut app = new_app();
        handle_ws_message(&mut app, r#"{"type":"heartbeat"}"#);
        handle_ws_message(&mut app, r#"{"type":"pong"}"#);
        assert_eq!(app.status, "Connecting...");
    }

    #[test]
    fn integration_timeout_warning_flag() {
        let mut app = new_app();
        app.streaming = true;
        app.chat_start_time = Some(Instant::now() - Duration::from_secs(130));
        app.timeout_warned = false;
        if let Some(start) = app.chat_start_time {
            if start.elapsed().as_secs() >= 120 && !app.timeout_warned {
                app.timeout_warned = true;
                app.push_system(
                    "Request has been running for over 2 minutes. Press Ctrl+C to cancel."
                        .into(),
                );
            }
        }
        assert!(app.timeout_warned);
        assert!(app
            .messages
            .iter()
            .any(|m| m.content.contains("2 minutes")));
    }

    #[test]
    fn integration_timeout_not_warned_before_threshold() {
        let mut app = new_app();
        app.streaming = true;
        app.chat_start_time = Some(Instant::now() - Duration::from_secs(60));
        app.timeout_warned = false;
        if let Some(start) = app.chat_start_time {
            if start.elapsed().as_secs() >= 120 && !app.timeout_warned {
                app.timeout_warned = true;
            }
        }
        assert!(!app.timeout_warned);
    }

    #[test]
    fn integration_timeout_warned_only_once() {
        let mut app = new_app();
        app.streaming = true;
        app.chat_start_time = Some(Instant::now() - Duration::from_secs(200));
        app.timeout_warned = true;
        let msg_count_before = app.messages.len();
        if let Some(start) = app.chat_start_time {
            if start.elapsed().as_secs() >= 120 && !app.timeout_warned {
                app.push_system("should not appear".into());
            }
        }
        assert_eq!(app.messages.len(), msg_count_before);
    }

    #[test]
    fn integration_timeout_reset_on_streaming_start() {
        let mut app = new_app();
        app.timeout_warned = true;
        app.streaming = true;
        app.timeout_warned = false;
        app.chat_start_time = Some(Instant::now());
        assert!(!app.timeout_warned);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Cursor position rendering verification tests
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn cursor_position_empty_input() {
        let mut app = new_app();
        app.connected = true;
        app.execution_mode = "agent".into();
        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw_ui(f, &app)).unwrap();
        let (cx, _cy) = terminal.get_cursor_position().unwrap().into();
        let expected_prefix_width =
            UnicodeWidthStr::width("[Agent]") + UnicodeWidthStr::width("❯") + 1;
        assert_eq!(
            cx, expected_prefix_width as u16,
            "cursor at start of empty input"
        );
    }

    #[test]
    fn cursor_position_ascii_input() {
        let mut app = new_app();
        app.connected = true;
        app.execution_mode = "agent".into();
        app.input = "hello".into();
        app.cursor_pos = 5;
        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw_ui(f, &app)).unwrap();
        let (cx, _cy) = terminal.get_cursor_position().unwrap().into();
        let prefix_w =
            UnicodeWidthStr::width("[Agent]") + UnicodeWidthStr::width("❯") + 1;
        assert_eq!(cx, (prefix_w + 5) as u16, "cursor after 'hello'");
    }

    #[test]
    fn cursor_position_cjk_input() {
        let mut app = new_app();
        app.connected = true;
        app.execution_mode = "agent".into();
        app.input = "你好".into();
        app.cursor_pos = 2;
        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw_ui(f, &app)).unwrap();
        let (cx, _cy) = terminal.get_cursor_position().unwrap().into();
        let prefix_w =
            UnicodeWidthStr::width("[Agent]") + UnicodeWidthStr::width("❯") + 1;
        assert_eq!(
            cx,
            (prefix_w + 4) as u16,
            "cursor after CJK '你好' (4 columns)"
        );
    }

    #[test]
    fn cursor_position_mixed_input_middle() {
        let mut app = new_app();
        app.connected = true;
        app.execution_mode = "agent".into();
        app.input = "hi你好".into();
        app.cursor_pos = 2;
        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw_ui(f, &app)).unwrap();
        let (cx, _cy) = terminal.get_cursor_position().unwrap().into();
        let prefix_w =
            UnicodeWidthStr::width("[Agent]") + UnicodeWidthStr::width("❯") + 1;
        assert_eq!(
            cx,
            (prefix_w + 2) as u16,
            "cursor between ASCII and CJK"
        );
    }

    #[test]
    fn cursor_position_plan_mode() {
        let mut app = new_app();
        app.connected = true;
        app.execution_mode = "plan".into();
        app.input = "test".into();
        app.cursor_pos = 4;
        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw_ui(f, &app)).unwrap();
        let (cx, _cy) = terminal.get_cursor_position().unwrap().into();
        let prefix_w =
            UnicodeWidthStr::width("[Plan]") + UnicodeWidthStr::width("❯") + 1;
        assert_eq!(cx, (prefix_w + 4) as u16, "cursor in plan mode");
    }

    #[test]
    fn integration_multi_turn_session_persistence() {
        let mut app = new_app();
        app.streaming = true;
        app.chat_start_time = Some(Instant::now());
        app.messages.push(ChatMsg {
            role: "assistant".into(),
            content: String::new(),
            timestamp: "12:00:00".into(),
        });
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.start","data":{"sessionId":"s1"}}"#,
        );
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.delta","data":{"content":"Turn 1 reply"}}"#,
        );
        handle_ws_message(&mut app, r#"{"type":"chat.complete","data":{"elapsedMs":500,"inputTokensEstimate":10,"outputTokensEstimate":5}}"#);
        assert!(!app.streaming);
        assert_eq!(app.session_id, Some("s1".into()));
        assert_eq!(app.total_messages, 1);
        app.streaming = true;
        app.chat_start_time = Some(Instant::now());
        app.messages.push(ChatMsg {
            role: "assistant".into(),
            content: String::new(),
            timestamp: "12:01:00".into(),
        });
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.start","data":{"sessionId":"s1"}}"#,
        );
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.delta","data":{"content":"Turn 2 reply"}}"#,
        );
        handle_ws_message(&mut app, r#"{"type":"chat.complete","data":{"elapsedMs":800,"inputTokensEstimate":20,"outputTokensEstimate":15}}"#);
        assert!(!app.streaming);
        assert_eq!(app.total_messages, 2);
        assert_eq!(app.total_input_tokens, 30);
        assert_eq!(app.total_output_tokens, 20);
        assert_eq!(app.total_elapsed_ms, 1300);
    }

    #[test]
    fn integration_error_mid_stream_recovers() {
        let mut app = new_app();
        app.streaming = true;
        app.messages.push(ChatMsg {
            role: "assistant".into(),
            content: String::new(),
            timestamp: "12:00:00".into(),
        });
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.start","data":{"sessionId":"s1"}}"#,
        );
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.delta","data":{"content":"partial response..."}}"#,
        );
        handle_ws_message(
            &mut app,
            r#"{"type":"chat.error","error":{"message":"upstream timeout"}}"#,
        );
        assert!(!app.streaming);
        assert!(app.status.contains("upstream timeout"));
        assert_eq!(
            app.messages.last().unwrap().content,
            "partial response..."
        );
    }

    #[test]
    fn integration_sessions_list_popup() {
        let mut app = new_app();
        handle_ws_message(
            &mut app,
            r#"{"type":"sessions.list","data":{"sessions":[{"id":"s1","title":"Test"}]}}"#,
        );
        match &app.show_popup {
            Some(PopupKind::Sessions(sessions)) => {
                assert_eq!(sessions.len(), 1);
            }
            _ => panic!("expected Sessions popup"),
        }
    }

    #[test]
    fn integration_models_list() {
        let mut app = new_app();
        handle_ws_message(&mut app, r#"{"type":"models.list","data":{"models":[{"provider":"openai","model":"gpt-4o"},{"provider":"anthropic","model":"claude-3.5-sonnet"}]}}"#);
        assert!(app
            .messages
            .iter()
            .any(|m| m.content.contains("1. openai/gpt-4o")));
        assert!(app
            .messages
            .iter()
            .any(|m| m.content.contains("2. anthropic/claude-3.5-sonnet")));
        assert_eq!(app.models_cache.len(), 2);
        assert_eq!(app.models_cache[0], ("openai".into(), "gpt-4o".into()));
    }

    #[test]
    fn integration_mcp_status_empty() {
        let mut app = new_app();
        handle_ws_message(
            &mut app,
            r#"{"type":"mcp.status","data":{"servers":[]}}"#,
        );
        assert!(app.messages.iter().any(|m| m.content.contains("No MCP")));
    }

    #[test]
    fn integration_mcp_status_with_servers() {
        let mut app = new_app();
        handle_ws_message(
            &mut app,
            r#"{"type":"mcp.status","data":{"servers":[{"id":"github","status":"running"}]}}"#,
        );
        assert!(app.messages.iter().any(|m| m.content.contains("github")));
    }

    // ── History search tests ────────────────────────────────────────

    #[test]
    fn history_search_basic() {
        let mut app = new_app();
        app.input_history = vec![
            "hello world".into(),
            "fix bug".into(),
            "hello rust".into(),
        ];
        app.history_search_active = true;
        app.history_search_query = "hello".into();

        // Should find last match
        super::input::search_history_from_start(&mut app);
        assert_eq!(app.history_search_index, Some(2));
        assert_eq!(app.input, "hello rust");
    }
}
