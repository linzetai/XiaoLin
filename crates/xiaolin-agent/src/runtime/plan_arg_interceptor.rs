use std::path::{Path, PathBuf};

/// Intercepts streaming tool call arguments to extract plan file content deltas.
///
/// When the LLM streams a `write_file` (or `edit_file`) tool call targeting the
/// plan file, this interceptor incrementally parses the JSON arguments, unescapes
/// the `content` string value, and emits text deltas via a callback — enabling
/// real-time plan rendering in the frontend without waiting for tool execution.
pub(crate) struct PlanArgInterceptor {
    plan_file_path: PathBuf,
    state: InterceptState,
    json_parser: JsonStreamParser,
}

#[derive(Debug, Clone, PartialEq)]
enum InterceptState {
    /// Not tracking any tool call (wrong tool name or path mismatch confirmed).
    Inactive,
    /// Tracking a write_file/edit_file call; path not yet confirmed.
    AwaitingPath,
    /// Path confirmed to match plan file; actively extracting content deltas.
    Extracting,
    /// Path confirmed to NOT match plan file; ignoring this tool call.
    Rejected,
}

impl PlanArgInterceptor {
    pub fn new(plan_file_path: PathBuf) -> Self {
        Self {
            plan_file_path,
            state: InterceptState::Inactive,
            json_parser: JsonStreamParser::new(),
        }
    }

    /// Notify the interceptor that a new tool call has started with the given name.
    /// Returns true if this tool is potentially a plan file write.
    pub fn on_tool_start(&mut self, tool_name: &str) -> bool {
        let dominated = matches!(tool_name, "write_file" | "create_file" | "edit_file");
        if dominated {
            self.state = InterceptState::AwaitingPath;
            self.json_parser = JsonStreamParser::new();
            true
        } else {
            self.state = InterceptState::Inactive;
            false
        }
    }

    /// Feed an argument chunk to the interceptor.
    /// Returns extracted content deltas (unescaped text) if any.
    pub fn feed(&mut self, chunk: &str) -> Vec<String> {
        match self.state {
            InterceptState::Inactive | InterceptState::Rejected => return Vec::new(),
            InterceptState::AwaitingPath | InterceptState::Extracting => {}
        }

        let events = self.json_parser.feed(chunk);
        let mut deltas = Vec::new();

        for event in events {
            match event {
                JsonEvent::KeyValue { key, value } if key == "file_path" => {
                    if self.path_matches(&value) {
                        self.state = InterceptState::Extracting;
                        // Flush any buffered content deltas
                        deltas.extend(self.json_parser.flush_content_buffer());
                    } else {
                        self.state = InterceptState::Rejected;
                        self.json_parser.clear_content_buffer();
                        return Vec::new();
                    }
                }
                JsonEvent::ContentDelta(text) => {
                    match self.state {
                        InterceptState::Extracting => deltas.push(text),
                        InterceptState::AwaitingPath => {
                            self.json_parser.buffer_content(text);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        deltas
    }

    /// Reset state for the next tool call.
    pub fn reset(&mut self) {
        self.state = InterceptState::Inactive;
        self.json_parser = JsonStreamParser::new();
    }

    fn path_matches(&self, value: &str) -> bool {
        let candidate = Path::new(value);
        if candidate == self.plan_file_path {
            return true;
        }
        // Tail match: the candidate's file name matches the plan file's file name
        // and contains a suffix of the plan path components.
        if let (Some(plan_name), Some(cand_name)) =
            (self.plan_file_path.file_name(), candidate.file_name())
        {
            if plan_name == cand_name {
                let plan_str = self.plan_file_path.to_string_lossy();
                let cand_str = candidate.to_string_lossy();
                if plan_str.ends_with(cand_str.as_ref())
                    || cand_str.ends_with(plan_str.as_ref())
                {
                    return true;
                }
            }
        }
        false
    }
}

// ─── JSON Stream Parser ──────────────────────────────────────────────────────

#[derive(Debug)]
enum JsonEvent {
    /// A complete key-value pair where value is a string.
    KeyValue { key: String, value: String },
    /// An incremental content delta (unescaped text from the "content" string).
    ContentDelta(String),
}

/// Incremental JSON parser that emits events for specific keys.
///
/// Handles the case where JSON arguments arrive in arbitrary chunks across
/// network boundaries. Tracks depth (objects/arrays), string state, and
/// escape sequences to correctly identify key-value pairs.
#[derive(Debug)]
struct JsonStreamParser {
    /// Current parsing state
    parse_state: ParseState,
    /// Current JSON nesting depth (for objects/arrays)
    depth: u32,
    /// Buffer for the current key being parsed
    key_buf: String,
    /// Buffer for the current value being parsed (non-content keys)
    value_buf: String,
    /// Whether we're inside the "content" value (streaming mode)
    in_content_value: bool,
    /// Escape state for JSON string parsing
    escape: EscapeState,
    /// Content delta buffer (for when path arrives after content)
    content_buffer: Vec<String>,
    /// Cap on buffered content (chars)
    content_buffer_chars: usize,
}

#[derive(Debug, Clone, PartialEq)]
enum ParseState {
    /// Outside any significant context
    Root,
    /// Inside a top-level object, waiting for a key
    ObjectKey,
    /// Parsing a key string (between quotes)
    InKey,
    /// After key, expecting colon
    AfterKey,
    /// After colon, expecting value
    BeforeValue,
    /// Parsing a string value
    InStringValue,
    /// Parsing a non-string value (number, bool, null, nested)
    InOtherValue,
}

#[derive(Debug, Clone, PartialEq)]
enum EscapeState {
    Normal,
    /// Just saw a backslash
    Backslash,
    /// In \uXXXX, collecting hex digits
    Unicode(String),
}

const CONTENT_BUFFER_CAP: usize = 200;

impl JsonStreamParser {
    fn new() -> Self {
        Self {
            parse_state: ParseState::Root,
            depth: 0,
            key_buf: String::new(),
            value_buf: String::new(),
            in_content_value: false,
            escape: EscapeState::Normal,
            content_buffer: Vec::new(),
            content_buffer_chars: 0,
        }
    }

    fn feed(&mut self, chunk: &str) -> Vec<JsonEvent> {
        let mut events = Vec::new();
        for ch in chunk.chars() {
            if let Some(ev) = self.process_char(ch) {
                events.push(ev);
            }
        }
        events
    }

    fn process_char(&mut self, ch: char) -> Option<JsonEvent> {
        match self.parse_state {
            ParseState::Root => {
                if ch == '{' {
                    self.depth = 1;
                    self.parse_state = ParseState::ObjectKey;
                }
                None
            }
            ParseState::ObjectKey => {
                match ch {
                    '"' => {
                        self.key_buf.clear();
                        self.parse_state = ParseState::InKey;
                        self.escape = EscapeState::Normal;
                    }
                    '}' => {
                        self.depth -= 1;
                        if self.depth == 0 {
                            self.parse_state = ParseState::Root;
                        }
                    }
                    _ => {} // whitespace, commas
                }
                None
            }
            ParseState::InKey => {
                match &self.escape {
                    EscapeState::Normal => {
                        if ch == '\\' {
                            self.escape = EscapeState::Backslash;
                        } else if ch == '"' {
                            self.parse_state = ParseState::AfterKey;
                        } else {
                            self.key_buf.push(ch);
                        }
                    }
                    EscapeState::Backslash => {
                        // In key names we don't need full unescape; just record
                        self.key_buf.push(unescape_char(ch));
                        self.escape = EscapeState::Normal;
                    }
                    EscapeState::Unicode(_) => {
                        // Unlikely in keys but handle gracefully
                        self.key_buf.push(ch);
                        self.escape = EscapeState::Normal;
                    }
                }
                None
            }
            ParseState::AfterKey => {
                if ch == ':' {
                    self.parse_state = ParseState::BeforeValue;
                }
                None
            }
            ParseState::BeforeValue => {
                match ch {
                    '"' => {
                        self.in_content_value = self.key_buf == "content";
                        self.value_buf.clear();
                        self.parse_state = ParseState::InStringValue;
                        self.escape = EscapeState::Normal;
                    }
                    '{' | '[' => {
                        // Nested object/array — skip entirely by depth tracking
                        self.depth += 1;
                        self.parse_state = ParseState::InOtherValue;
                        self.in_content_value = false;
                    }
                    c if c.is_whitespace() => {}
                    _ => {
                        // number, bool, null literal
                        self.parse_state = ParseState::InOtherValue;
                        self.in_content_value = false;
                        self.value_buf.clear();
                        self.value_buf.push(ch);
                    }
                }
                None
            }
            ParseState::InStringValue => self.process_string_char(ch),
            ParseState::InOtherValue => {
                match ch {
                    ',' => {
                        self.parse_state = ParseState::ObjectKey;
                    }
                    '}' => {
                        self.depth -= 1;
                        if self.depth == 0 {
                            self.parse_state = ParseState::Root;
                        } else {
                            self.parse_state = ParseState::ObjectKey;
                        }
                    }
                    '{' | '[' => {
                        self.depth += 1;
                    }
                    ']' => {
                        self.depth -= 1;
                    }
                    _ => {}
                }
                None
            }
        }
    }

    fn process_string_char(&mut self, ch: char) -> Option<JsonEvent> {
        match &self.escape {
            EscapeState::Normal => {
                if ch == '\\' {
                    self.escape = EscapeState::Backslash;
                    return None;
                }
                if ch == '"' {
                    // End of string value
                    let event = if self.in_content_value {
                        self.in_content_value = false;
                        None // Content streaming already emitted deltas
                    } else {
                        let key = self.key_buf.clone();
                        let value = self.value_buf.clone();
                        Some(JsonEvent::KeyValue { key, value })
                    };
                    self.parse_state = ParseState::ObjectKey;
                    return event;
                }
                // Regular character
                if self.in_content_value {
                    return Some(JsonEvent::ContentDelta(ch.to_string()));
                }
                self.value_buf.push(ch);
                None
            }
            EscapeState::Backslash => {
                if ch == 'u' {
                    self.escape = EscapeState::Unicode(String::new());
                    return None;
                }
                let unescaped = unescape_char(ch);
                self.escape = EscapeState::Normal;
                if self.in_content_value {
                    return Some(JsonEvent::ContentDelta(unescaped.to_string()));
                }
                self.value_buf.push(unescaped);
                None
            }
            EscapeState::Unicode(ref hex) => {
                let mut hex = hex.clone();
                hex.push(ch);
                if hex.len() == 4 {
                    let unescaped = unicode_unescape(&hex);
                    self.escape = EscapeState::Normal;
                    if self.in_content_value {
                        return Some(JsonEvent::ContentDelta(unescaped));
                    }
                    self.value_buf.push_str(&unescaped);
                } else {
                    self.escape = EscapeState::Unicode(hex);
                }
                None
            }
        }
    }

    fn buffer_content(&mut self, text: String) {
        let len = text.len();
        if self.content_buffer_chars + len <= CONTENT_BUFFER_CAP {
            self.content_buffer_chars += len;
            self.content_buffer.push(text);
        }
        // Exceeding cap: silently drop (path-first is the common case)
    }

    fn flush_content_buffer(&mut self) -> Vec<String> {
        self.content_buffer_chars = 0;
        std::mem::take(&mut self.content_buffer)
    }

    fn clear_content_buffer(&mut self) {
        self.content_buffer.clear();
        self.content_buffer_chars = 0;
    }
}

fn unescape_char(ch: char) -> char {
    match ch {
        'n' => '\n',
        't' => '\t',
        'r' => '\r',
        '"' => '"',
        '\\' => '\\',
        '/' => '/',
        _ => ch,
    }
}

fn unicode_unescape(hex: &str) -> String {
    u32::from_str_radix(hex, 16)
        .ok()
        .and_then(char::from_u32)
        .map(|c| c.to_string())
        .unwrap_or_else(|| format!("\\u{hex}"))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_interceptor(path: &str) -> PlanArgInterceptor {
        PlanArgInterceptor::new(PathBuf::from(path))
    }

    #[test]
    fn basic_write_file_extraction() {
        let mut interceptor = make_interceptor("/tmp/plan.md");
        interceptor.on_tool_start("write_file");

        let json = r##"{"file_path": "/tmp/plan.md", "content": "# Plan\n\nStep 1"}"##;
        let deltas = interceptor.feed(json);

        let combined: String = deltas.into_iter().collect();
        assert_eq!(combined, "# Plan\n\nStep 1");
    }

    #[test]
    fn chunked_input() {
        let mut interceptor = make_interceptor("/tmp/plan.md");
        interceptor.on_tool_start("write_file");

        let mut all_deltas = Vec::new();
        // Simulate chunks arriving
        all_deltas.extend(interceptor.feed(r#"{"file_pa"#));
        all_deltas.extend(interceptor.feed(r#"th": "/tmp/plan.md", "#));
        all_deltas.extend(interceptor.feed(r#""conte"#));
        all_deltas.extend(interceptor.feed(r#"nt": "Hello"#));
        all_deltas.extend(interceptor.feed(r#" World"}"#));

        let combined: String = all_deltas.into_iter().collect();
        assert_eq!(combined, "Hello World");
    }

    #[test]
    fn escape_sequences() {
        let mut interceptor = make_interceptor("/tmp/plan.md");
        interceptor.on_tool_start("write_file");

        let json = r#"{"file_path": "/tmp/plan.md", "content": "line1\nline2\ttab\""}"#;
        let deltas = interceptor.feed(json);

        let combined: String = deltas.into_iter().collect();
        assert_eq!(combined, "line1\nline2\ttab\"");
    }

    #[test]
    fn unicode_escape() {
        let mut interceptor = make_interceptor("/tmp/plan.md");
        interceptor.on_tool_start("write_file");

        let json = r#"{"file_path": "/tmp/plan.md", "content": "\u4e2d\u6587"}"#;
        let deltas = interceptor.feed(json);

        let combined: String = deltas.into_iter().collect();
        assert_eq!(combined, "中文");
    }

    #[test]
    fn content_before_path() {
        let mut interceptor = make_interceptor("/tmp/plan.md");
        interceptor.on_tool_start("write_file");

        // Content arrives first, then path
        let json = r#"{"content": "buffered text", "file_path": "/tmp/plan.md"}"#;
        let deltas = interceptor.feed(json);

        let combined: String = deltas.into_iter().collect();
        assert_eq!(combined, "buffered text");
    }

    #[test]
    fn content_before_path_mismatch() {
        let mut interceptor = make_interceptor("/tmp/plan.md");
        interceptor.on_tool_start("write_file");

        let json = r#"{"content": "should be discarded", "file_path": "/tmp/other.md"}"#;
        let deltas = interceptor.feed(json);

        assert!(deltas.is_empty(), "non-matching path should discard buffered content");
    }

    #[test]
    fn non_plan_file_ignored() {
        let mut interceptor = make_interceptor("/tmp/plan.md");
        interceptor.on_tool_start("write_file");

        let json = r#"{"file_path": "/tmp/other.rs", "content": "fn main() {}"}"#;
        let deltas = interceptor.feed(json);

        assert!(deltas.is_empty());
    }

    #[test]
    fn wrong_tool_ignored() {
        let mut interceptor = make_interceptor("/tmp/plan.md");
        interceptor.on_tool_start("shell_exec");

        let json = r#"{"command": "echo hello"}"#;
        let deltas = interceptor.feed(json);

        assert!(deltas.is_empty());
    }

    #[test]
    fn escape_across_chunks() {
        let mut interceptor = make_interceptor("/tmp/plan.md");
        interceptor.on_tool_start("write_file");

        let mut all_deltas = Vec::new();
        all_deltas.extend(interceptor.feed(r#"{"file_path": "/tmp/plan.md", "content": "a\"#));
        // The backslash came at end of chunk, 'n' comes in next chunk
        all_deltas.extend(interceptor.feed(r#"nb"}"#));

        let combined: String = all_deltas.into_iter().collect();
        assert_eq!(combined, "a\nb");
    }

    #[test]
    fn unicode_across_chunks() {
        let mut interceptor = make_interceptor("/tmp/plan.md");
        interceptor.on_tool_start("write_file");

        let mut all_deltas = Vec::new();
        all_deltas.extend(interceptor.feed(r#"{"file_path": "/tmp/plan.md", "content": "\u4e"#));
        all_deltas.extend(interceptor.feed(r#"2d"}"#));

        let combined: String = all_deltas.into_iter().collect();
        assert_eq!(combined, "中");
    }

    #[test]
    fn relative_path_tail_match() {
        let mut interceptor = make_interceptor("/home/user/project/.xiaolin/sessions/abc/plan.md");
        interceptor.on_tool_start("write_file");

        let json = r#"{"file_path": ".xiaolin/sessions/abc/plan.md", "content": "works"}"#;
        let deltas = interceptor.feed(json);

        let combined: String = deltas.into_iter().collect();
        assert_eq!(combined, "works");
    }

    #[test]
    fn reset_allows_reuse() {
        let mut interceptor = make_interceptor("/tmp/plan.md");

        // First tool call — non-matching
        interceptor.on_tool_start("write_file");
        interceptor.feed(r#"{"file_path": "/tmp/other.md", "content": "x"}"#);

        // Second tool call — matching
        interceptor.on_tool_start("write_file");
        let deltas = interceptor.feed(r#"{"file_path": "/tmp/plan.md", "content": "y"}"#);

        let combined: String = deltas.into_iter().collect();
        assert_eq!(combined, "y");
    }
}
