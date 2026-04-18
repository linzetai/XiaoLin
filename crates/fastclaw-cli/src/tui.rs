use std::io::IsTerminal;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use futures::{SinkExt, StreamExt};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

// ── Slash commands ──────────────────────────────────────────────────

const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/help", "Show available commands"),
    ("/agent", "Switch agent: /agent <id>"),
    ("/agents", "List available agents"),
    ("/new", "Start a new session"),
    ("/sessions", "List recent sessions"),
    ("/resume", "Resume session: /resume <id>"),
    ("/clear", "Clear message view"),
    ("/model", "Show current model info"),
    ("/quit", "Exit TUI"),
];

// ── Data types ──────────────────────────────────────────────────────

#[derive(Clone)]
struct ChatMsg {
    role: String,
    content: String,
    timestamp: String,
}

struct AgentInfo {
    id: String,
    name: String,
    model: String,
}

pub struct TuiApp {
    input: String,
    cursor_pos: usize,
    messages: Vec<ChatMsg>,
    scroll_offset: u16,
    status: String,
    connected: bool,
    streaming: bool,
    session_id: Option<String>,
    agent_id: String,
    agents: Vec<AgentInfo>,
    ws_url: String,
    api_key: Option<String>,
    should_quit: bool,
    req_counter: u64,
    input_history: Vec<String>,
    history_index: Option<usize>,
    history_stash: String,
    tab_completions: Vec<String>,
    tab_index: usize,
    tab_prefix: String,
    show_popup: Option<PopupKind>,
}

#[derive(Clone)]
enum PopupKind {
    Help,
    Agents,
    Sessions(Vec<Value>),
}

impl TuiApp {
    pub fn new(ws_url: String, api_key: Option<String>, session_id: Option<String>) -> Self {
        Self {
            input: String::new(),
            cursor_pos: 0,
            messages: Vec::new(),
            scroll_offset: 0,
            status: "Connecting...".into(),
            connected: false,
            streaming: false,
            session_id,
            agent_id: "default".into(),
            agents: Vec::new(),
            ws_url,
            api_key,
            should_quit: false,
            req_counter: 0,
            input_history: Vec::new(),
            history_index: None,
            history_stash: String::new(),
            tab_completions: Vec::new(),
            tab_index: 0,
            tab_prefix: String::new(),
            show_popup: None,
        }
    }

    fn next_id(&mut self) -> String {
        self.req_counter += 1;
        format!("tui-{}", self.req_counter)
    }

    fn push_system(&mut self, content: String) {
        self.messages.push(ChatMsg {
            role: "system".into(),
            content,
            timestamp: now_hms(),
        });
    }

    fn push_history(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.input_history.last().map(String::as_str) != Some(text) {
            self.input_history.push(text.to_string());
        }
        self.history_index = None;
    }

    fn reset_tab(&mut self) {
        self.tab_completions.clear();
        self.tab_index = 0;
        self.tab_prefix.clear();
    }
}

fn now_hms() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

// ── Entry point ─────────────────────────────────────────────────────

pub async fn run_tui(url: &str, token: Option<&str>, session: Option<&str>) -> anyhow::Result<()> {
    if !std::io::stdout().is_terminal() {
        anyhow::bail!("TUI requires an interactive terminal (TTY). Use --help for options.");
    }

    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;

    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)?;

    let mut app = TuiApp::new(
        url.to_string(),
        token.map(String::from),
        session.map(String::from),
    );

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
        .map_err(|e| anyhow::anyhow!("Failed to connect to gateway at {}: {e}", app.ws_url))?;
    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    app.connected = true;
    app.status = "Connected".into();

    let agents_req = json!({"id": "init-agents", "method": "agents"});
    let _ = ws_tx
        .send(Message::Text(agents_req.to_string().into()))
        .await;

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
                        }
                    }
                    break;
                }
            }
        }
    }

    app.push_system("Welcome to FastClaw TUI! Type /help for commands.".into());

    let result = run_event_loop(&mut terminal, &mut app, &mut ws_tx, &mut ws_rx).await;

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

// ── WebSocket type aliases ──────────────────────────────────────────

type WsTx = futures::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;
type WsRx = futures::stream::SplitStream<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
>;

// ── Main event loop ─────────────────────────────────────────────────

async fn run_event_loop(
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    app: &mut TuiApp,
    ws_tx: &mut WsTx,
    ws_rx: &mut WsRx,
) -> anyhow::Result<()> {
    loop {
        terminal.draw(|f| draw_ui(f, app))?;

        if app.should_quit {
            break;
        }

        tokio::select! {
            biased;

            ws_msg = ws_rx.next() => {
                match ws_msg {
                    Some(Ok(Message::Text(text))) => {
                        handle_ws_message(app, &text);
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        app.connected = false;
                        app.status = "Disconnected".into();
                        break;
                    }
                    _ => {}
                }
            }

            has_event = poll_crossterm_event() => {
                if has_event {
                    if let Ok(Event::Key(key)) = event::read() {
                        handle_key_event(app, ws_tx, key).await;
                    }
                }
            }
        }
    }
    Ok(())
}

async fn poll_crossterm_event() -> bool {
    tokio::task::spawn_blocking(|| event::poll(Duration::from_millis(50)).unwrap_or(false))
        .await
        .unwrap_or(false)
}

// ── Key handling ────────────────────────────────────────────────────

async fn handle_key_event(app: &mut TuiApp, ws_tx: &mut WsTx, key: KeyEvent) {
    // Popup dismissal
    if app.show_popup.is_some() {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter) {
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

        // Tab completion
        (_, KeyCode::Tab) if !app.streaming => {
            handle_tab_completion(app);
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

        // Text editing
        (_, KeyCode::Char(c)) if !app.streaming => {
            app.reset_tab();
            app.input.insert(app.cursor_pos, c);
            app.cursor_pos += 1;
        }
        (_, KeyCode::Backspace) if app.cursor_pos > 0 && !app.streaming => {
            app.reset_tab();
            app.cursor_pos -= 1;
            app.input.remove(app.cursor_pos);
        }
        (_, KeyCode::Delete) if app.cursor_pos < app.input.len() && !app.streaming => {
            app.reset_tab();
            app.input.remove(app.cursor_pos);
        }
        (_, KeyCode::Left) if app.cursor_pos > 0 => {
            app.cursor_pos -= 1;
        }
        (_, KeyCode::Right) if app.cursor_pos < app.input.len() => {
            app.cursor_pos += 1;
        }
        (KeyModifiers::CONTROL, KeyCode::Char('a')) => {
            app.cursor_pos = 0;
        }
        (KeyModifiers::CONTROL, KeyCode::Char('e')) => {
            app.cursor_pos = app.input.len();
        }
        (_, KeyCode::Home) => {
            app.cursor_pos = 0;
        }
        (_, KeyCode::End) => {
            app.cursor_pos = app.input.len();
        }
        (KeyModifiers::CONTROL, KeyCode::Char('l')) => {
            app.messages.clear();
            app.session_id = None;
            app.scroll_offset = 0;
        }
        (_, KeyCode::Esc) if app.streaming => {
            app.streaming = false;
            app.status = "Cancelled".into();
        }
        _ => {}
    }
}

// ── Slash commands ──────────────────────────────────────────────────

async fn handle_slash_command(app: &mut TuiApp, ws_tx: &mut WsTx, text: &str) {
    app.input.clear();
    app.cursor_pos = 0;

    let parts: Vec<&str> = text.splitn(2, ' ').collect();
    let cmd = parts[0];
    let arg = parts.get(1).copied().unwrap_or("");

    match cmd {
        "/help" => {
            app.show_popup = Some(PopupKind::Help);
        }
        "/quit" | "/exit" => {
            app.should_quit = true;
        }
        "/clear" => {
            app.messages.clear();
            app.scroll_offset = 0;
        }
        "/new" => {
            app.messages.clear();
            app.session_id = None;
            app.scroll_offset = 0;
            app.push_system("New session started.".into());
        }
        "/agent" if !arg.is_empty() => {
            if app.agents.iter().any(|a| a.id == arg) {
                app.agent_id = arg.to_string();
                app.push_system(format!("Switched to agent: {arg}"));
            } else {
                let available: Vec<_> = app.agents.iter().map(|a| a.id.as_str()).collect();
                app.push_system(format!(
                    "Agent '{arg}' not found. Available: {}",
                    available.join(", ")
                ));
            }
        }
        "/agent" => {
            app.push_system(format!("Current agent: {}", app.agent_id));
        }
        "/agents" => {
            app.show_popup = Some(PopupKind::Agents);
        }
        "/model" => {
            if let Some(a) = app.agents.iter().find(|a| a.id == app.agent_id) {
                app.push_system(format!("Agent: {} ({})\nModel: {}", a.id, a.name, a.model));
            }
        }
        "/sessions" => {
            let id = app.next_id();
            let req = json!({"id": id, "method": "sessions.list", "params": {"limit": 10}});
            let _ = ws_tx.send(Message::Text(req.to_string().into())).await;
            app.status = "Loading sessions...".into();
        }
        "/resume" if !arg.is_empty() => {
            app.session_id = Some(arg.to_string());
            app.messages.clear();
            app.scroll_offset = 0;

            let id = app.next_id();
            let req =
                json!({"id": id, "method": "sessions.messages", "params": {"sessionId": arg}});
            let _ = ws_tx.send(Message::Text(req.to_string().into())).await;
            app.push_system(format!("Resuming session: {}", &arg[..arg.len().min(12)]));
        }
        "/resume" => {
            app.push_system("Usage: /resume <session-id>".into());
        }
        _ => {
            app.push_system(format!(
                "Unknown command: {cmd}. Type /help for available commands."
            ));
        }
    }
}

// ── Tab completion ──────────────────────────────────────────────────

fn handle_tab_completion(app: &mut TuiApp) {
    if app.tab_completions.is_empty() {
        let prefix = app.input.clone();
        let mut completions: Vec<String> = Vec::new();

        if prefix.starts_with('/') {
            // Slash command completion
            for (cmd, _) in SLASH_COMMANDS {
                if cmd.starts_with(&prefix) {
                    completions.push(cmd.to_string());
                }
            }

            // /agent <name> completion
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
        app.cursor_pos = app.input.len();
    }
}

// ── History navigation ──────────────────────────────────────────────

fn navigate_history(app: &mut TuiApp, up: bool) {
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
                app.cursor_pos = app.input.len();
                app.history_index = None;
                return;
            }
            None => return,
        }
    }

    if let Some(i) = app.history_index {
        if let Some(entry) = app.input_history.get(i) {
            app.input = entry.clone();
            app.cursor_pos = app.input.len();
        }
    }
}

// ── Send chat ───────────────────────────────────────────────────────

async fn send_chat(app: &mut TuiApp, ws_tx: &mut WsTx) {
    let text = app.input.drain(..).collect::<String>();
    app.cursor_pos = 0;

    app.messages.push(ChatMsg {
        role: "user".into(),
        content: text.clone(),
        timestamp: now_hms(),
    });

    let id = app.next_id();
    let mut params = json!({
        "messages": [{"role": "user", "content": text}],
        "agentId": app.agent_id,
    });
    if let Some(sid) = &app.session_id {
        params["sessionId"] = json!(sid);
    }

    let req = json!({"id": id, "method": "chat", "params": params});
    let _ = ws_tx.send(Message::Text(req.to_string().into())).await;

    app.streaming = true;
    app.status = "Thinking...".into();
    app.scroll_offset = 0;
    app.messages.push(ChatMsg {
        role: "assistant".into(),
        content: String::new(),
        timestamp: now_hms(),
    });
}

// ── WebSocket message handler ───────────────────────────────────────

fn handle_ws_message(app: &mut TuiApp, text: &str) {
    let msg: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return,
    };

    let msg_type = msg["type"].as_str().unwrap_or("");
    match msg_type {
        "connected" => {
            app.status = "Connected".into();
        }
        "chat.start" => {
            if let Some(sid) = msg["data"]["sessionId"].as_str() {
                app.session_id = Some(sid.to_string());
            }
            app.status = "Streaming...".into();
        }
        "chat.delta" => {
            if let Some(content) = msg["data"]["content"].as_str() {
                if let Some(last) = app.messages.last_mut() {
                    if last.role == "assistant" {
                        last.content.push_str(content);
                    }
                }
            }
            app.scroll_offset = 0;
        }
        "chat.tool.start" => {
            let tool = msg["data"]["tool"].as_str().unwrap_or("unknown");
            app.status = format!("Running tool: {tool}...");
        }
        "chat.tool.done" => {
            app.status = "Tool complete, continuing...".into();
        }
        "chat.complete" => {
            app.streaming = false;
            let sid = app.session_id.as_deref().unwrap_or("none");
            app.status = format!("Ready | session: {}", &sid[..sid.len().min(8)]);
        }
        "chat.error" => {
            app.streaming = false;
            let err = msg["error"]["message"].as_str().unwrap_or("unknown error");
            app.status = format!("Error: {err}");
            if let Some(last) = app.messages.last_mut() {
                if last.role == "assistant" && last.content.is_empty() {
                    last.content = format!("[Error: {err}]");
                }
            }
        }
        "sessions.list" => {
            if let Some(sessions) = msg["data"]["sessions"].as_array() {
                app.show_popup = Some(PopupKind::Sessions(sessions.clone()));
                app.status = "Ready".into();
            }
        }
        "sessions.messages" => {
            if let Some(messages) = msg["data"]["messages"].as_array() {
                for m in messages {
                    let role = m["role"].as_str().unwrap_or("unknown").to_string();
                    let content = m["content"].as_str().unwrap_or("").to_string();
                    let ts = m["created_at"]
                        .as_str()
                        .map(|s| s.split(' ').last().unwrap_or(s).to_string())
                        .unwrap_or_default();
                    app.messages.push(ChatMsg {
                        role,
                        content,
                        timestamp: ts,
                    });
                }
                app.scroll_offset = 0;
            }
        }
        "heartbeat" | "pong" => {}
        _ => {}
    }
}

// ── Drawing ─────────────────────────────────────────────────────────

fn draw_ui(f: &mut Frame, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(f.area());

    draw_title_bar(f, app, chunks[0]);
    draw_messages(f, app, chunks[1]);
    draw_input(f, app, chunks[2]);
    draw_status_bar(f, app, chunks[3]);

    if let Some(popup) = &app.show_popup {
        draw_popup(f, popup, &app.agents);
    }
}

fn draw_title_bar(f: &mut Frame, app: &TuiApp, area: Rect) {
    let conn = if app.connected { "●" } else { "○" };
    let conn_color = if app.connected {
        Color::Green
    } else {
        Color::Red
    };
    let session = app.session_id.as_deref().unwrap_or("new");
    let sid_short = &session[..session.len().min(8)];

    let agent_display = app
        .agents
        .iter()
        .find(|a| a.id == app.agent_id)
        .map(|a| format!("{}({})", a.id, a.name))
        .unwrap_or_else(|| app.agent_id.clone());

    let title = Line::from(vec![
        Span::styled(
            " FastClaw TUI ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(conn, Style::default().fg(conn_color)),
        Span::raw("  "),
        Span::styled(
            format!("agent:{agent_display}"),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw("  "),
        Span::styled(
            format!("session:{sid_short}"),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    f.render_widget(
        Paragraph::new(title).style(Style::default().bg(Color::DarkGray)),
        area,
    );
}

fn draw_messages(f: &mut Frame, app: &TuiApp, area: Rect) {
    let inner = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner_area = inner.inner(area);
    f.render_widget(inner, area);

    let mut lines: Vec<Line> = Vec::new();
    for msg in &app.messages {
        let (prefix, color) = match msg.role.as_str() {
            "user" => ("You", Color::Green),
            "assistant" => ("AI", Color::Cyan),
            "system" => ("Sys", Color::Yellow),
            _ => ("???", Color::White),
        };

        let ts = if msg.timestamp.is_empty() {
            String::new()
        } else {
            format!(" {}", msg.timestamp)
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!("[{prefix}]"),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(ts, Style::default().fg(Color::DarkGray)),
        ]));

        if msg.role == "assistant" {
            render_markdown_lines(&msg.content, &mut lines, app.streaming);
        } else {
            for text_line in msg.content.lines() {
                lines.push(Line::from(Span::raw(format!("  {text_line}"))));
            }
        }

        if msg.content.is_empty() && msg.role == "assistant" && app.streaming {
            lines.push(Line::from(Span::styled(
                "  ▌",
                Style::default().fg(Color::Cyan),
            )));
        }
        lines.push(Line::default());
    }

    let total_lines = lines.len() as u16;
    let visible = inner_area.height;
    let scroll = if total_lines > visible {
        (total_lines - visible).saturating_sub(app.scroll_offset)
    } else {
        0
    };

    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    f.render_widget(paragraph, inner_area);
}

// ── Markdown rendering ──────────────────────────────────────────────

fn render_markdown_lines(content: &str, lines: &mut Vec<Line<'static>>, streaming: bool) {
    let mut in_code_block = false;
    let mut code_lang = String::new();

    for line in content.lines() {
        if line.starts_with("```") {
            if in_code_block {
                in_code_block = false;
                code_lang.clear();
            } else {
                in_code_block = true;
                code_lang = line.trim_start_matches('`').trim().to_string();
                let label = if code_lang.is_empty() {
                    " code ".to_string()
                } else {
                    format!(" {code_lang} ")
                };
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        format!("─── {label} ───"),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
            continue;
        }

        if in_code_block {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("│ {line}"),
                    Style::default().fg(Color::White).bg(Color::Rgb(30, 30, 46)),
                ),
            ]));
            continue;
        }

        // Headings
        if let Some(rest) = line.strip_prefix("### ") {
            lines.push(Line::from(Span::styled(
                format!("  ### {rest}"),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            )));
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            lines.push(Line::from(Span::styled(
                format!("  ## {rest}"),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            )));
            continue;
        }
        if let Some(rest) = line.strip_prefix("# ") {
            lines.push(Line::from(Span::styled(
                format!("  # {rest}"),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )));
            continue;
        }

        // Bullet lists
        if line.starts_with("- ") || line.starts_with("* ") || line.starts_with("• ") {
            let rest = &line[2..];
            let spans = parse_inline_markdown(rest);
            let mut result = vec![Span::styled("  • ", Style::default().fg(Color::Yellow))];
            result.extend(spans);
            lines.push(Line::from(result));
            continue;
        }

        // Numbered lists
        if let Some(pos) = line.find(". ") {
            if pos <= 3 && line[..pos].chars().all(|c| c.is_ascii_digit()) {
                let num = &line[..pos];
                let rest = &line[pos + 2..];
                let spans = parse_inline_markdown(rest);
                let mut result = vec![Span::styled(
                    format!("  {num}. "),
                    Style::default().fg(Color::Yellow),
                )];
                result.extend(spans);
                lines.push(Line::from(result));
                continue;
            }
        }

        // Regular text with inline markdown
        let spans = parse_inline_markdown(line);
        let mut result = vec![Span::raw("  ".to_string())];
        result.extend(spans);
        lines.push(Line::from(result));
    }

    // Unclosed code block while streaming
    if in_code_block && streaming {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "│ ▌",
                Style::default().fg(Color::Cyan).bg(Color::Rgb(30, 30, 46)),
            ),
        ]));
    }
}

/// Parse inline markdown: **bold**, *italic*, `code`, ~~strikethrough~~
fn parse_inline_markdown(text: &str) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        // Inline code
        if let Some(start) = remaining.find('`') {
            if start > 0 {
                spans.push(Span::raw(remaining[..start].to_string()));
            }
            let after = &remaining[start + 1..];
            if let Some(end) = after.find('`') {
                spans.push(Span::styled(
                    after[..end].to_string(),
                    Style::default()
                        .fg(Color::LightYellow)
                        .bg(Color::Rgb(40, 40, 50)),
                ));
                remaining = &after[end + 1..];
                continue;
            }
            spans.push(Span::raw(remaining[start..].to_string()));
            break;
        }

        // Bold
        if let Some(start) = remaining.find("**") {
            if start > 0 {
                spans.push(Span::raw(remaining[..start].to_string()));
            }
            let after = &remaining[start + 2..];
            if let Some(end) = after.find("**") {
                spans.push(Span::styled(
                    after[..end].to_string(),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
                remaining = &after[end + 2..];
                continue;
            }
            spans.push(Span::raw(remaining[start..].to_string()));
            break;
        }

        // Italic (single *)
        if let Some(start) = remaining.find('*') {
            if start > 0 {
                spans.push(Span::raw(remaining[..start].to_string()));
            }
            let after = &remaining[start + 1..];
            if let Some(end) = after.find('*') {
                spans.push(Span::styled(
                    after[..end].to_string(),
                    Style::default().add_modifier(Modifier::ITALIC),
                ));
                remaining = &after[end + 1..];
                continue;
            }
            spans.push(Span::raw(remaining[start..].to_string()));
            break;
        }

        spans.push(Span::raw(remaining.to_string()));
        break;
    }

    spans
}

// ── Input rendering ─────────────────────────────────────────────────

fn draw_input(f: &mut Frame, app: &TuiApp, area: Rect) {
    let hint = if app.streaming {
        " (streaming... Esc to cancel)"
    } else if !app.tab_completions.is_empty() {
        " (Tab to cycle)"
    } else {
        " (Enter: send | /: commands)"
    };

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if app.streaming {
            Color::DarkGray
        } else {
            Color::Cyan
        }))
        .title(format!(" Message{hint} "));

    let input_text = Paragraph::new(app.input.as_str())
        .style(Style::default().fg(if app.streaming {
            Color::DarkGray
        } else {
            Color::White
        }))
        .block(input_block);
    f.render_widget(input_text, area);

    if !app.streaming {
        f.set_cursor_position((area.x + 1 + app.cursor_pos as u16, area.y + 1));
    }
}

fn draw_status_bar(f: &mut Frame, app: &TuiApp, area: Rect) {
    let status = Line::from(vec![
        Span::styled(" ", Style::default()),
        Span::styled(&app.status, Style::default().fg(Color::White)),
        Span::raw("  "),
        Span::styled(
            "Ctrl+C:quit  Shift+↑↓:scroll  /help:commands  Tab:complete",
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    f.render_widget(
        Paragraph::new(status).style(Style::default().bg(Color::Black)),
        area,
    );
}

// ── Popup rendering ─────────────────────────────────────────────────

fn draw_popup(f: &mut Frame, popup: &PopupKind, agents: &[AgentInfo]) {
    let area = f.area();
    let popup_area = centered_rect(60, 60, area);

    f.render_widget(Clear, popup_area);

    match popup {
        PopupKind::Help => {
            let mut lines = vec![
                Line::from(Span::styled(
                    "Slash Commands",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::default(),
            ];
            for (cmd, desc) in SLASH_COMMANDS {
                lines.push(Line::from(vec![
                    Span::styled(format!("  {cmd:<15}"), Style::default().fg(Color::Yellow)),
                    Span::raw(desc.to_string()),
                ]));
            }
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(
                "Keyboard Shortcuts",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::default());
            for (key, desc) in [
                ("Ctrl+C/Q", "Quit"),
                ("Enter", "Send message"),
                ("Tab", "Auto-complete command"),
                ("↑/↓", "Input history"),
                ("Shift+↑/↓", "Scroll messages"),
                ("Ctrl+A/E", "Home/End cursor"),
                ("Ctrl+L", "Clear & new session"),
                ("Esc", "Cancel streaming"),
            ] {
                lines.push(Line::from(vec![
                    Span::styled(format!("  {key:<15}"), Style::default().fg(Color::Yellow)),
                    Span::raw(desc),
                ]));
            }
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(
                " Press Esc/Enter to close ",
                Style::default().fg(Color::DarkGray),
            )));

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Help ");
            f.render_widget(
                Paragraph::new(lines)
                    .block(block)
                    .wrap(Wrap { trim: false }),
                popup_area,
            );
        }
        PopupKind::Agents => {
            let mut lines = vec![
                Line::from(Span::styled(
                    "Available Agents",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::default(),
            ];
            for a in agents {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("  {:<15}", a.id),
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::raw(format!("{} ({})", a.name, a.model)),
                ]));
            }
            lines.push(Line::default());
            lines.push(Line::from(Span::raw("  Use /agent <id> to switch")));
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(
                " Press Esc/Enter to close ",
                Style::default().fg(Color::DarkGray),
            )));

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Agents ");
            f.render_widget(
                Paragraph::new(lines)
                    .block(block)
                    .wrap(Wrap { trim: false }),
                popup_area,
            );
        }
        PopupKind::Sessions(sessions) => {
            let mut lines = vec![
                Line::from(Span::styled(
                    "Recent Sessions",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::default(),
            ];
            for s in sessions {
                let id = s["id"].as_str().unwrap_or("?");
                let agent = s["agentId"].as_str().unwrap_or("?");
                let msgs = s["messageCount"].as_i64().unwrap_or(0);
                let updated = s["updatedAt"].as_str().unwrap_or("?");
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("  {}", &id[..id.len().min(12)]),
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::raw(format!("  agent={agent} msgs={msgs} {updated}")),
                ]));
            }
            lines.push(Line::default());
            lines.push(Line::from(Span::raw(
                "  Use /resume <id> to restore a session",
            )));
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(
                " Press Esc/Enter to close ",
                Style::default().fg(Color::DarkGray),
            )));

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Sessions ");
            f.render_widget(
                Paragraph::new(lines)
                    .block(block)
                    .wrap(Wrap { trim: false }),
                popup_area,
            );
        }
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
