use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use super::markdown::render_markdown_lines;
use super::state::*;
use super::widgets::draw_popup;

pub(crate) fn draw_ui(f: &mut Frame, app: &TuiApp) {
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
        draw_popup(f, popup, &app.agents, app.select_state.as_ref());
    }
}

pub(crate) fn draw_title_bar(f: &mut Frame, app: &TuiApp, area: Rect) {
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

pub(crate) fn draw_messages(f: &mut Frame, app: &TuiApp, area: Rect) {
    let inner = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner_area = inner.inner(area);
    f.render_widget(inner, area);

    let is_last_msg = |idx: usize| idx == app.messages.len().saturating_sub(1);

    let mut lines: Vec<Line> = Vec::new();
    for (msg_idx, msg) in app.messages.iter().enumerate() {
        let (prefix, color) = match msg.role.as_str() {
            "user" => ("❯", Color::Green),
            "assistant" => ("⎿", Color::Cyan),
            "system" => ("·", Color::Rgb(140, 140, 160)),
            _ => ("?", Color::White),
        };

        let ts = if msg.timestamp.is_empty() {
            String::new()
        } else {
            format!(" {}", msg.timestamp)
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!(" {prefix}"),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(ts, Style::default().fg(Color::DarkGray)),
        ]));

        if msg.role == "assistant" {
            render_markdown_lines(
                &msg.content,
                &mut lines,
                app.streaming && is_last_msg(msg_idx),
            );
        } else if msg.role == "system" {
            for text_line in msg.content.lines() {
                if text_line.starts_with("[Error]") || text_line.contains("Error:") {
                    lines.push(Line::from(vec![
                        Span::styled("  ▎", Style::default().fg(Color::Red)),
                        Span::styled(
                            text_line.to_string(),
                            Style::default().fg(Color::Red),
                        ),
                    ]));
                } else {
                    lines.push(Line::from(Span::styled(
                        format!("  {text_line}"),
                        Style::default().fg(Color::Rgb(140, 140, 160)),
                    )));
                }
            }
        } else {
            for text_line in msg.content.lines() {
                lines.push(Line::from(Span::raw(format!("  {text_line}"))));
            }
        }

        // Spinner at end of streaming assistant message
        if msg.content.is_empty()
            && msg.role == "assistant"
            && app.streaming
            && is_last_msg(msg_idx)
        {
            lines.push(Line::from(Span::styled(
                format!("  {}", app.spinner.display()),
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

pub(crate) fn draw_input(f: &mut Frame, app: &TuiApp, area: Rect) {
    // Fulltext search indicator
    if app.search_active {
        let match_info = if app.search_matches.is_empty() {
            if app.search_query.is_empty() {
                String::new()
            } else {
                " (no matches)".to_string()
            }
        } else {
            format!(" ({}/{})", app.search_current + 1, app.search_matches.len())
        };
        let search_prefix = format!("Search: {}{}", app.search_query, match_info);
        let spans = vec![Span::styled(
            &search_prefix,
            Style::default().fg(Color::Yellow),
        )];

        let input_block = Block::default()
            .borders(Borders::TOP | Borders::BOTTOM)
            .border_style(Style::default().fg(Color::Yellow));

        let input_paragraph = Paragraph::new(Line::from(spans)).block(input_block);
        f.render_widget(input_paragraph, area);

        let cursor_x = area.x + search_prefix.len() as u16 - match_info.len() as u16;
        f.set_cursor_position((cursor_x, area.y + 1));
        return;
    }

    // History search indicator
    if app.history_search_active {
        let search_prefix = format!("(reverse-i-search)`{}': ", app.history_search_query);
        let spans = vec![
            Span::styled(
                &search_prefix,
                Style::default().fg(Color::Rgb(180, 140, 255)),
            ),
            Span::styled(app.input.clone(), Style::default().fg(Color::White)),
        ];

        let input_block = Block::default()
            .borders(Borders::TOP | Borders::BOTTOM)
            .border_style(Style::default().fg(Color::Rgb(100, 80, 160)));

        let input_paragraph = Paragraph::new(Line::from(spans)).block(input_block);
        f.render_widget(input_paragraph, area);

        let prefix_width = UnicodeWidthStr::width(search_prefix.as_str());
        let input_display_width = display_width_chars(&app.input, app.cursor_pos);
        let cursor_x = area.x + prefix_width as u16 + input_display_width as u16;
        f.set_cursor_position((cursor_x, area.y + 1));
        return;
    }

    let (mode_prefix, mode_color) = if app.execution_mode == "plan" {
        ("[Plan]", Color::Rgb(180, 140, 255))
    } else {
        ("[Agent]", Color::Rgb(100, 200, 130))
    };

    let prompt_char = if app.streaming { "…" } else { "❯" };
    let prompt_color = if app.streaming {
        Color::DarkGray
    } else {
        Color::Cyan
    };

    let prefix_spans = vec![
        Span::styled(mode_prefix, Style::default().fg(mode_color)),
        Span::styled(
            format!("{prompt_char} "),
            Style::default().fg(prompt_color),
        ),
    ];
    let prefix_width =
        UnicodeWidthStr::width(mode_prefix) + UnicodeWidthStr::width(prompt_char) + 1;

    let input_color = if app.streaming {
        Color::DarkGray
    } else {
        Color::White
    };

    let mut spans = prefix_spans;

    // For multi-line: only show current line in input box, show line count indicator
    let line_count = app.input.chars().filter(|c| *c == '\n').count() + 1;
    if line_count > 1 {
        let display_text = app
            .input
            .lines()
            .last()
            .unwrap_or(&app.input)
            .to_string();
        spans.push(Span::styled(display_text, Style::default().fg(input_color)));
        spans.push(Span::styled(
            format!(" [{line_count} lines]"),
            Style::default().fg(Color::DarkGray),
        ));
    } else {
        spans.push(Span::styled(
            app.input.clone(),
            Style::default().fg(input_color),
        ));
    }

    // Show stash indicator
    if app.stashed_input.is_some() {
        spans.push(Span::styled(
            " [stash]",
            Style::default().fg(Color::Rgb(200, 150, 50)),
        ));
    }

    let input_block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(Color::Rgb(50, 50, 70)));

    let input_paragraph = Paragraph::new(Line::from(spans)).block(input_block);
    f.render_widget(input_paragraph, area);

    // Suggestion overlay for slash commands
    if !app.streaming && app.input.starts_with('/') && app.input.len() > 1 {
        let partial = &app.input[1..];
        let matches: Vec<&(&str, &str)> = SLASH_COMMANDS
            .iter()
            .filter(|(cmd, _)| cmd[1..].starts_with(partial))
            .take(8)
            .collect();
        if !matches.is_empty() && matches.len() < SLASH_COMMANDS.len() {
            let overlay_height = matches.len() as u16 + 2;
            let overlay_area = Rect {
                x: area.x,
                y: area.y.saturating_sub(overlay_height),
                width: area.width.min(60),
                height: overlay_height,
            };
            let mut overlay_lines: Vec<Line> = Vec::new();
            for (i, (cmd, desc)) in matches.iter().enumerate() {
                let is_selected = app.tab_index == i && !app.tab_completions.is_empty();
                let cmd_style = if is_selected {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                        .bg(Color::Rgb(40, 40, 60))
                } else {
                    Style::default().fg(Color::Cyan)
                };
                let desc_style = if is_selected {
                    Style::default()
                        .fg(Color::White)
                        .bg(Color::Rgb(40, 40, 60))
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                overlay_lines.push(Line::from(vec![
                    Span::styled(format!(" {cmd:<14}"), cmd_style),
                    Span::styled(*desc, desc_style),
                ]));
            }
            let overlay_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(60, 60, 90)));
            f.render_widget(Clear, overlay_area);
            f.render_widget(
                Paragraph::new(overlay_lines).block(overlay_block),
                overlay_area,
            );
        }
    }

    if !app.streaming {
        let input_display_width = display_width_chars(&app.input, app.cursor_pos);
        let cursor_x = area.x + prefix_width as u16 + input_display_width as u16;
        f.set_cursor_position((cursor_x, area.y + 1));
    }
}

pub(crate) fn draw_status_bar(f: &mut Frame, app: &TuiApp, area: Rect) {
    let mut spans: Vec<Span> = Vec::new();

    // Left: mode indicator
    let (mode_label, mode_color) = if app.execution_mode == "plan" {
        (" PLAN ", Color::Rgb(180, 140, 255))
    } else {
        (" AGENT ", Color::Rgb(100, 200, 130))
    };
    spans.push(Span::styled(
        mode_label,
        Style::default()
            .fg(Color::Black)
            .bg(mode_color)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw(" "));

    // Model name
    if !app.current_model.is_empty() {
        spans.push(Span::styled(
            &app.current_model,
            Style::default().fg(Color::Rgb(180, 180, 200)),
        ));
        spans.push(Span::raw(" "));
    }

    // Spinner or status
    if app.streaming {
        spans.push(Span::styled(
            app.spinner.display(),
            Style::default().fg(Color::Cyan),
        ));
    } else {
        spans.push(Span::styled(
            &app.status,
            Style::default().fg(Color::Rgb(140, 140, 160)),
        ));
    }

    // Context bar (middle section)
    if app.ctx_limit_tokens > 0 {
        let pct = (app.ctx_used_tokens as f64 / app.ctx_limit_tokens as f64 * 100.0) as u8;
        let bar_width = 10u8;
        let filled = (pct as u16 * bar_width as u16 / 100).min(bar_width as u16) as u8;
        let empty = bar_width - filled;
        let bar_color = if pct >= 85 {
            Color::Red
        } else if pct >= 70 {
            Color::Yellow
        } else {
            Color::Rgb(100, 180, 230)
        };
        let used_k = app.ctx_used_tokens / 1000;
        let limit_k = app.ctx_limit_tokens / 1000;
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            "ctx:",
            Style::default().fg(Color::DarkGray),
        ));
        spans.push(Span::styled(
            format!("{pct}%"),
            Style::default().fg(bar_color),
        ));
        spans.push(Span::styled("[", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(
            "█".repeat(filled as usize),
            Style::default().fg(bar_color),
        ));
        spans.push(Span::styled(
            "░".repeat(empty as usize),
            Style::default().fg(Color::Rgb(60, 60, 80)),
        ));
        spans.push(Span::styled("]", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(
            format!(" {used_k}k/{limit_k}k"),
            Style::default().fg(Color::DarkGray),
        ));
    }

    // Right: cost/tokens summary
    if app.total_messages > 0 {
        let total_tok = app.total_input_tokens + app.total_output_tokens;
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("↑{}↓{}", app.total_input_tokens, app.total_output_tokens),
            Style::default().fg(Color::Rgb(100, 100, 140)),
        ));
        if total_tok >= 1000 {
            spans.push(Span::styled(
                format!(" ({:.1}k)", total_tok as f64 / 1000.0),
                Style::default().fg(Color::Rgb(80, 80, 100)),
            ));
        }
    }

    // Shortcut hints (right-aligned conceptually)
    if !app.streaming {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            "? help · Shift+Tab mode · Ctrl+R search",
            Style::default().fg(Color::Rgb(80, 80, 100)),
        ));
    }

    let status = Line::from(spans);
    f.render_widget(
        Paragraph::new(status).style(Style::default().bg(Color::Rgb(20, 20, 30))),
        area,
    );
}
