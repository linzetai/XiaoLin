use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use super::state::*;

pub(crate) fn draw_popup(f: &mut Frame, popup: &PopupKind, agents: &[AgentInfo], select_state: Option<&super::state::SelectState>) {
    let area = f.area();
    let popup_area = centered_rect(60, 60, area);

    f.render_widget(Clear, popup_area);

    match popup {
        PopupKind::Help => {
            let mut lines = vec![
                Line::from(Span::styled(
                    " FastClaw Help",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::default(),
                Line::from(vec![
                    Span::styled(
                        " Input Modes         ",
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        "Keyboard Shortcuts   ",
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        "Navigation",
                        Style::default()
                            .fg(Color::Magenta)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::default(),
            ];

            let col1: &[(&str, &str)] = &[
                ("/ + cmd", "Slash commands"),
                ("/help", "This help menu"),
                ("/plan", "Toggle mode"),
                ("/agents", "List agents"),
                ("/new", "New session"),
                ("/stats", "Usage stats"),
                ("/compact", "Compress ctx"),
                ("/todo", "Todo list"),
                ("/mcp", "MCP status"),
            ];
            let col2: &[(&str, &str)] = &[
                ("Ctrl+C", "Quit"),
                ("Ctrl+L", "Clear+new session"),
                ("Ctrl+U", "Clear line"),
                ("Ctrl+W", "Delete word"),
                ("Ctrl+A/E", "Home/End"),
                ("Ctrl+S", "Stash input"),
                ("Ctrl+R", "History search"),
                ("Shift+Tab", "Toggle mode"),
                ("Shift+Enter", "Multi-line"),
            ];
            let col3: &[(&str, &str)] = &[
                ("Enter", "Send message"),
                ("Tab", "Auto-complete"),
                ("↑/↓", "History"),
                ("Shift+↑↓", "Scroll msgs"),
                ("PageUp/Dn", "Scroll msgs"),
                ("Esc", "Cancel stream"),
                ("Esc×2", "Clear input"),
                ("", ""),
                ("", ""),
            ];

            let max_rows = col1.len().max(col2.len()).max(col3.len());
            for i in 0..max_rows {
                let c1 = col1.get(i).copied().unwrap_or(("", ""));
                let c2 = col2.get(i).copied().unwrap_or(("", ""));
                let c3 = col3.get(i).copied().unwrap_or(("", ""));
                lines.push(Line::from(vec![
                    Span::styled(
                        format!(" {:<12}", c1.0),
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::styled(format!("{:<14}", c1.1), Style::default().fg(Color::White)),
                    Span::styled(
                        format!("{:<12}", c2.0),
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::styled(format!("{:<14}", c2.1), Style::default().fg(Color::White)),
                    Span::styled(
                        format!("{:<10}", c3.0),
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::styled(c3.1.to_string(), Style::default().fg(Color::White)),
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
            let help_area = centered_rect(80, 60, area);
            f.render_widget(Clear, help_area);
            f.render_widget(
                Paragraph::new(lines)
                    .block(block)
                    .wrap(Wrap { trim: false }),
                help_area,
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
        PopupKind::AskQuestion {
            question, options, ..
        } => {
            let mut lines = vec![
                Line::from(Span::styled(
                    "Agent Question",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::default(),
                Line::from(Span::raw(format!("  {question}"))),
                Line::default(),
            ];
            for (i, (_, label)) in options.iter().enumerate() {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("  {}. ", i + 1),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(label.clone()),
                ]));
            }
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(
                " Press number to answer, Esc to dismiss ",
                Style::default().fg(Color::DarkGray),
            )));

            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
                .title(" Question ");
            f.render_widget(
                Paragraph::new(lines)
                    .block(block)
                    .wrap(Wrap { trim: false }),
                popup_area,
            );
        }
        PopupKind::ModelPicker => {
            if let Some(sel) = select_state {
                let mut lines = vec![
                    Line::from(Span::styled(
                        " Select Model",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::default(),
                ];
                if !sel.filter.is_empty() {
                    lines.push(Line::from(vec![
                        Span::styled("  Filter: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(sel.filter.clone(), Style::default().fg(Color::Yellow)),
                    ]));
                    lines.push(Line::default());
                }
                for (vi, &idx) in sel.filtered_indices.iter().enumerate() {
                    let item = &sel.items[idx];
                    let is_selected = vi == sel.selected;
                    let marker = if item.is_current { "● " } else { "  " };
                    let style = if is_selected {
                        Style::default()
                            .fg(Color::White)
                            .bg(Color::Rgb(50, 50, 80))
                            .add_modifier(Modifier::BOLD)
                    } else if item.is_current {
                        Style::default().fg(Color::Green)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {marker}{}", item.label), style),
                        Span::styled(
                            format!("  {}", item.description),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ]));
                }
                if sel.filtered_indices.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "  No matching models",
                        Style::default().fg(Color::DarkGray),
                    )));
                }
                lines.push(Line::default());
                lines.push(Line::from(Span::styled(
                    " ↑↓ navigate · Enter select · Type to filter · Esc close ",
                    Style::default().fg(Color::DarkGray),
                )));

                let block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(" Model ");
                let picker_area = centered_rect(60, 70, f.area());
                f.render_widget(Clear, picker_area);
                f.render_widget(
                    Paragraph::new(lines)
                        .block(block)
                        .wrap(Wrap { trim: false }),
                    picker_area,
                );
            }
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

pub(crate) fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
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
