//! Shared code-snippet extraction.
//!
//! Extracts a small window of source lines around a target line, mirroring the
//! `±context_lines` slicing used by `search_in_files`. Operates on whole lines
//! so it is inherently UTF-8 safe (never byte-slices mid-char; see quality rule #1).

/// Upper bound on the characters of a single snippet, to keep tool results within
/// the per-result size budget even when many results each carry a snippet.
pub const MAX_SNIPPET_CHARS: usize = 600;

/// Extract a code snippet around a 1-based `line` with `±context_lines` of context.
///
/// Returns an empty string when `line` is 0 or out of range. Each line is prefixed
/// with its number; the target line is marked with `>`. The output is char-safe
/// truncated to [`MAX_SNIPPET_CHARS`].
pub fn line_snippet(content: &str, line_1based: usize, context_lines: usize) -> String {
    if line_1based == 0 {
        return String::new();
    }
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return String::new();
    }
    let idx = line_1based - 1;
    if idx >= lines.len() {
        return String::new();
    }
    let start = idx.saturating_sub(context_lines);
    let end = (idx + context_lines + 1).min(lines.len());

    let mut out = String::new();
    for (i, l) in lines[start..end].iter().enumerate() {
        let n = start + i + 1;
        let marker = if n == line_1based { '>' } else { ' ' };
        out.push_str(&format!("{marker}{n:>5}| {l}\n"));
    }
    let trimmed = out.trim_end();

    // Char-safe cap (never panic on multi-byte boundaries).
    if trimmed.chars().count() > MAX_SNIPPET_CHARS {
        let capped: String = trimmed.chars().take(MAX_SNIPPET_CHARS).collect();
        format!("{capped}…")
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_window_around_target() {
        let content = "a\nb\nc\nd\ne\n";
        let snip = line_snippet(content, 3, 1);
        assert!(snip.contains("b"));
        assert!(snip.contains(">    3| c"));
        assert!(snip.contains("d"));
        assert!(!snip.contains("a"));
        assert!(!snip.contains("e"));
    }

    #[test]
    fn out_of_range_returns_empty() {
        assert_eq!(line_snippet("a\nb\n", 0, 2), "");
        assert_eq!(line_snippet("a\nb\n", 99, 2), "");
        assert_eq!(line_snippet("", 1, 2), "");
    }

    #[test]
    fn multibyte_is_char_safe() {
        let content = "let 变量 = \"中文内容😀\";\nnext line\n";
        // Must not panic and must include the target line.
        let snip = line_snippet(content, 1, 0);
        assert!(snip.contains("变量"));
    }

    #[test]
    fn long_line_is_truncated_char_safe() {
        let long = "中".repeat(1000);
        let content = format!("{long}\n");
        let snip = line_snippet(&content, 1, 0);
        // Truncated with ellipsis; counted by chars, no panic.
        assert!(snip.chars().count() <= MAX_SNIPPET_CHARS + 10);
        assert!(snip.ends_with('…'));
    }
}
