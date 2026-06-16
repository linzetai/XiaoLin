/// Remove invisible/dangerous Unicode characters from MCP server-supplied strings.
///
/// Strips: bidi controls, zero-width chars, and C0 controls except `\t\n\r`.
/// Preserves all visible characters including CJK, emoji, and punctuation.
pub fn sanitize_unicode(s: &str) -> String {
    s.chars()
        .filter(|&c| {
            !matches!(c,
                // Bidi directional marks
                '\u{200E}' | '\u{200F}' |
                // Bidi embedding/override/isolate
                '\u{202A}'..='\u{202E}' |
                '\u{2066}'..='\u{2069}' |
                // Zero-width chars
                '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}'
            ) && !is_stripped_control(c)
        })
        .collect()
}

fn is_stripped_control(c: char) -> bool {
    matches!(c, '\u{0000}'..='\u{001F}') && !matches!(c, '\t' | '\n' | '\r')
}

/// Recursively sanitize all `"description"` string values in a JSON Schema.
pub fn sanitize_json_schema_descriptions(schema: &mut serde_json::Value) {
    match schema {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(desc)) = map.get_mut("description") {
                *desc = sanitize_unicode(desc);
            }
            for (_, v) in map.iter_mut() {
                sanitize_json_schema_descriptions(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                sanitize_json_schema_descriptions(v);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_normal_text() {
        assert_eq!(sanitize_unicode("hello world"), "hello world");
        assert_eq!(sanitize_unicode("你好世界"), "你好世界");
        assert_eq!(sanitize_unicode("emoji 🚀✨"), "emoji 🚀✨");
        assert_eq!(sanitize_unicode("「引用」【括号】"), "「引用」【括号】");
    }

    #[test]
    fn preserves_allowed_whitespace() {
        assert_eq!(sanitize_unicode("a\tb\nc\r\n"), "a\tb\nc\r\n");
    }

    #[test]
    fn strips_bidi_override() {
        let input = "safe \u{202E}evil\u{202C} text";
        assert_eq!(sanitize_unicode(input), "safe evil text");
    }

    #[test]
    fn strips_bidi_marks_and_isolates() {
        let input = "\u{200E}ltr\u{200F}\u{2066}isolate\u{2069}";
        assert_eq!(sanitize_unicode(input), "ltrisolate");
    }

    #[test]
    fn strips_zero_width_chars() {
        let input = "a\u{200B}b\u{200C}c\u{200D}d\u{FEFF}e";
        assert_eq!(sanitize_unicode(input), "abcde");
    }

    #[test]
    fn strips_c0_controls() {
        let input = "a\x00b\x01c\x1Fd";
        assert_eq!(sanitize_unicode(input), "abcd");
    }

    #[test]
    fn schema_description_sanitized() {
        let mut schema = serde_json::json!({
            "type": "object",
            "description": "outer \u{202E}rtl\u{202C} desc",
            "properties": {
                "field": {
                    "type": "string",
                    "description": "inner \u{200B}zwsp"
                }
            }
        });
        sanitize_json_schema_descriptions(&mut schema);
        assert_eq!(
            schema["description"].as_str().unwrap(),
            "outer rtl desc"
        );
        assert_eq!(
            schema["properties"]["field"]["description"].as_str().unwrap(),
            "inner zwsp"
        );
    }

    #[test]
    fn schema_array_items_sanitized() {
        let mut schema = serde_json::json!({
            "type": "array",
            "items": {
                "type": "string",
                "description": "\u{FEFF}bom prefix"
            }
        });
        sanitize_json_schema_descriptions(&mut schema);
        assert_eq!(
            schema["items"]["description"].as_str().unwrap(),
            "bom prefix"
        );
    }

    #[test]
    fn schema_without_description_unchanged() {
        let mut schema = serde_json::json!({"type": "number"});
        let original = schema.clone();
        sanitize_json_schema_descriptions(&mut schema);
        assert_eq!(schema, original);
    }
}
