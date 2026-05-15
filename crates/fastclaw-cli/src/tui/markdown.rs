use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, ThemeSet};
use syntect::parsing::SyntaxSet;
use std::sync::LazyLock;

static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

const BG_CODE: Color = Color::Rgb(30, 30, 46);

fn highlight_code_line(line: &str, lang: &str) -> Vec<Span<'static>> {
    let syntax = SYNTAX_SET
        .find_syntax_by_token(lang)
        .unwrap_or_else(|| SYNTAX_SET.find_syntax_plain_text());
    let theme = &THEME_SET.themes["base16-ocean.dark"];
    let mut h = HighlightLines::new(syntax, theme);
    let line_with_nl = format!("{line}\n");
    match h.highlight_line(&line_with_nl, &SYNTAX_SET) {
        Ok(ranges) => ranges
            .iter()
            .map(|(style, text)| {
                let fg = Color::Rgb(style.foreground.r, style.foreground.g, style.foreground.b);
                let mut modifier = Modifier::empty();
                if style.font_style.contains(FontStyle::BOLD) {
                    modifier |= Modifier::BOLD;
                }
                if style.font_style.contains(FontStyle::ITALIC) {
                    modifier |= Modifier::ITALIC;
                }
                Span::styled(
                    text.trim_end_matches('\n').to_string(),
                    Style::default().fg(fg).bg(BG_CODE).add_modifier(modifier),
                )
            })
            .collect(),
        Err(_) => vec![Span::styled(
            line.to_string(),
            Style::default().fg(Color::White).bg(BG_CODE),
        )],
    }
}

pub(crate) fn render_markdown_lines(
    content: &str,
    lines: &mut Vec<Line<'static>>,
    streaming: bool,
) {
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
            let is_diff =
                code_lang == "diff" || code_lang == "patch" || code_lang.starts_with("diff");
            if is_diff {
                let (fg, bg) = if line.starts_with('+') && !line.starts_with("+++") {
                    (Color::Green, Color::Rgb(20, 40, 20))
                } else if line.starts_with('-') && !line.starts_with("---") {
                    (Color::Red, Color::Rgb(50, 20, 20))
                } else if line.starts_with("@@") {
                    (Color::Cyan, Color::Rgb(20, 30, 40))
                } else {
                    (Color::White, Color::Rgb(30, 30, 46))
                };
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(format!("│ {line}"), Style::default().fg(fg).bg(bg)),
                ]));
            } else {
                let highlighted = highlight_code_line(line, &code_lang);
                let mut spans = vec![Span::raw("  ".to_string()), Span::styled("│ ", Style::default().fg(Color::DarkGray).bg(BG_CODE))];
                spans.extend(highlighted);
                lines.push(Line::from(spans));
            }
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
                Style::default()
                    .fg(Color::Cyan)
                    .bg(Color::Rgb(30, 30, 46)),
            ),
        ]));
    }
}

/// Parse inline markdown: **bold**, *italic*, `code`, ~~strikethrough~~
pub(crate) fn parse_inline_markdown(text: &str) -> Vec<Span<'static>> {
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
