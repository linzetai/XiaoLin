use std::time::Duration;

/// Structured classification of API errors from LLM providers.
#[derive(Debug, Clone, PartialEq)]
pub enum ApiErrorKind {
    PromptTooLong {
        actual_tokens: Option<usize>,
        limit_tokens: Option<usize>,
    },
    RateLimited {
        retry_after: Option<Duration>,
    },
    Overloaded,
    AuthExpired,
    MaxOutputTokens,
    InvalidApiKey,
    ImageTooLarge {
        path: String,
        size: usize,
    },
    PdfTooLarge {
        path: String,
        pages: usize,
    },
    ConnectionTimeout,
    ConnectionReset,
    /// SSE stream interrupted mid-response (decode error, partial body, etc.).
    StreamInterrupted,
    ModelNotAvailable {
        model: String,
    },
    BudgetExhausted,
    Unknown(String),
}

pub struct ApiErrorClassifier;

impl ApiErrorClassifier {
    /// Classify an error from HTTP status code and optional response body.
    pub fn classify(
        status: Option<u16>,
        error_text: &str,
        response_body: Option<&str>,
    ) -> ApiErrorKind {
        let combined = match response_body {
            Some(body) => format!("{} {}", error_text, body),
            None => error_text.to_string(),
        };
        let lower = combined.to_lowercase();

        if let Some(st) = status {
            match st {
                401 => {
                    if lower.contains("invalid") && lower.contains("key") {
                        return ApiErrorKind::InvalidApiKey;
                    }
                    return ApiErrorKind::AuthExpired;
                }
                429 => {
                    let retry_after = Self::parse_retry_after(&combined);
                    return ApiErrorKind::RateLimited { retry_after };
                }
                529 => return ApiErrorKind::Overloaded,
                _ => {}
            }
        }

        if lower.contains("prompt_too_long")
            || lower.contains("context_length_exceeded")
            || lower.contains("maximum context length")
            || lower.contains("too many tokens")
        {
            let (actual, limit) = Self::parse_prompt_too_long_tokens(&combined);
            return ApiErrorKind::PromptTooLong {
                actual_tokens: actual,
                limit_tokens: limit,
            };
        }

        if lower.contains("max_tokens") && lower.contains("output") {
            return ApiErrorKind::MaxOutputTokens;
        }

        if lower.contains("invalid") && lower.contains("api") && lower.contains("key") {
            return ApiErrorKind::InvalidApiKey;
        }

        if lower.contains("rate_limit") || lower.contains("rate limit") {
            let retry_after = Self::parse_retry_after(&combined);
            return ApiErrorKind::RateLimited { retry_after };
        }

        if lower.contains("overloaded") || lower.contains("capacity") {
            return ApiErrorKind::Overloaded;
        }

        if lower.contains("image") && (lower.contains("too large") || lower.contains("exceeds")) {
            return ApiErrorKind::ImageTooLarge {
                path: String::new(),
                size: 0,
            };
        }

        if lower.contains("pdf")
            && (lower.contains("too large") || lower.contains("too many pages"))
        {
            return ApiErrorKind::PdfTooLarge {
                path: String::new(),
                pages: 0,
            };
        }

        if lower.contains("model")
            && (lower.contains("not found")
                || lower.contains("not available")
                || lower.contains("does not exist"))
        {
            return ApiErrorKind::ModelNotAvailable {
                model: String::new(),
            };
        }

        if lower.contains("budget") && lower.contains("exhaust") {
            return ApiErrorKind::BudgetExhausted;
        }

        if lower.contains("timeout") || lower.contains("timed out") {
            return ApiErrorKind::ConnectionTimeout;
        }

        if lower.contains("connection") && (lower.contains("reset") || lower.contains("closed")) {
            return ApiErrorKind::ConnectionReset;
        }

        if lower.contains("stream read error")
            || lower.contains("error decoding response body")
            || (lower.contains("stream") && lower.contains("interrupted"))
        {
            return ApiErrorKind::StreamInterrupted;
        }

        ApiErrorKind::Unknown(error_text.to_string())
    }

    /// Extract actual and limit token counts from prompt-too-long error messages.
    ///
    /// Handles patterns like:
    /// - "This request has 150000 tokens, limit is 128000"
    /// - "context_length_exceeded: 130000 > 128000"
    /// - "maximum context length is 128000 tokens, however you requested 150000"
    pub fn parse_prompt_too_long_tokens(raw: &str) -> (Option<usize>, Option<usize>) {
        let numbers: Vec<usize> = raw
            .split(|c: char| !c.is_ascii_digit())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse::<usize>().ok())
            .filter(|&n| n >= 1000)
            .collect();

        if numbers.len() >= 2 {
            let (a, b) = (numbers[0], numbers[1]);
            if a > b {
                return (Some(a), Some(b));
            }
            return (Some(b), Some(a));
        }

        if numbers.len() == 1 {
            let lower = raw.to_lowercase();
            if lower.contains("limit") || lower.contains("maximum") {
                return (None, Some(numbers[0]));
            }
            return (Some(numbers[0]), None);
        }

        (None, None)
    }

}

#[cfg(test)]
impl ApiErrorClassifier {
    /// Generate a human-readable recovery message for the given error kind.
    pub fn recovery_message(kind: &ApiErrorKind) -> String {
        match kind {
            ApiErrorKind::PromptTooLong {
                actual_tokens,
                limit_tokens,
            } => {
                let actual = actual_tokens.unwrap_or(0);
                let limit = limit_tokens.unwrap_or(0);
                format!(
                    "Prompt is too long ({} tokens > {} limit). Running auto-compact...",
                    actual, limit
                )
            }
            ApiErrorKind::RateLimited { retry_after } => {
                let secs = retry_after.map_or(30, |d| d.as_secs());
                format!("Rate limited. Retrying in {}s...", secs)
            }
            ApiErrorKind::Overloaded => "Server is overloaded. Waiting before retry...".to_string(),
            ApiErrorKind::AuthExpired => {
                "Authentication expired. Please refresh your API credentials.".to_string()
            }
            ApiErrorKind::MaxOutputTokens => {
                "Output was truncated due to max_tokens limit. Continuing with escalated limit..."
                    .to_string()
            }
            ApiErrorKind::InvalidApiKey => {
                "Invalid API key. Please check your configuration.".to_string()
            }
            ApiErrorKind::ImageTooLarge { path, size } => {
                format!(
                    "Image {} too large ({}). Consider resizing or using a smaller image.",
                    if path.is_empty() {
                        "<unknown>"
                    } else {
                        path.as_str()
                    },
                    format_file_size(*size)
                )
            }
            ApiErrorKind::PdfTooLarge { path, pages } => {
                format!(
                    "PDF {} too large ({} pages). Consider splitting the document.",
                    if path.is_empty() {
                        "<unknown>"
                    } else {
                        path.as_str()
                    },
                    pages
                )
            }
            ApiErrorKind::ConnectionTimeout => {
                "Connection timed out. Retrying with backoff...".to_string()
            }
            ApiErrorKind::ConnectionReset => "Connection was reset. Retrying...".to_string(),
            ApiErrorKind::StreamInterrupted => {
                "Stream was interrupted. Attempting to resume...".to_string()
            }
            ApiErrorKind::ModelNotAvailable { model } => {
                format!(
                    "Model '{}' is not available. Check your model configuration.",
                    if model.is_empty() {
                        "unknown"
                    } else {
                        model.as_str()
                    }
                )
            }
            ApiErrorKind::BudgetExhausted => {
                "Token budget exhausted. Please increase your budget limit or start a new session."
                    .to_string()
            }
            ApiErrorKind::Unknown(msg) => {
                format!("Unexpected error: {}. Please try again.", msg)
            }
        }
    }
}

#[cfg(test)]
fn format_file_size(bytes: usize) -> String {
    if bytes == 0 {
        return "0 B".to_string();
    }
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;
    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{} B", bytes)
    } else {
        format!("{:.1} {}", size, UNITS[unit_idx])
    }
}

impl ApiErrorClassifier {
    /// Parse `retry-after` value from error text (seconds).
    fn parse_retry_after(text: &str) -> Option<Duration> {
        let lower = text.to_lowercase();
        if let Some(pos) = lower.find("retry-after") {
            let after = &text[pos + 11..];
            let num_str: String = after
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(secs) = num_str.parse::<u64>() {
                return Some(Duration::from_secs(secs));
            }
        }
        if let Some(pos) = lower.find("retry in") {
            let after = &text[pos + 8..];
            let num_str: String = after
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(secs) = num_str.parse::<f64>() {
                return Some(Duration::from_secs_f64(secs));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── classify() tests ────────────────────────────────────────────

    #[test]
    fn classify_401_as_auth_expired() {
        let kind = ApiErrorClassifier::classify(Some(401), "Unauthorized", None);
        assert_eq!(kind, ApiErrorKind::AuthExpired);
    }

    #[test]
    fn classify_401_invalid_key() {
        let kind = ApiErrorClassifier::classify(Some(401), "Invalid API key provided", None);
        assert_eq!(kind, ApiErrorKind::InvalidApiKey);
    }

    #[test]
    fn classify_429_rate_limited() {
        let kind = ApiErrorClassifier::classify(Some(429), "Too many requests", None);
        assert!(matches!(kind, ApiErrorKind::RateLimited { .. }));
    }

    #[test]
    fn classify_429_with_retry_after() {
        let kind = ApiErrorClassifier::classify(Some(429), "Rate limited. Retry-After: 60", None);
        match kind {
            ApiErrorKind::RateLimited { retry_after } => {
                assert_eq!(retry_after, Some(Duration::from_secs(60)));
            }
            _ => panic!("expected RateLimited"),
        }
    }

    #[test]
    fn classify_529_overloaded() {
        let kind = ApiErrorClassifier::classify(Some(529), "overloaded", None);
        assert_eq!(kind, ApiErrorKind::Overloaded);
    }

    #[test]
    fn classify_prompt_too_long_from_body() {
        let kind = ApiErrorClassifier::classify(
            Some(400),
            "Bad Request",
            Some("prompt_too_long: 150000 tokens > 128000 limit"),
        );
        match kind {
            ApiErrorKind::PromptTooLong {
                actual_tokens,
                limit_tokens,
            } => {
                assert_eq!(actual_tokens, Some(150000));
                assert_eq!(limit_tokens, Some(128000));
            }
            _ => panic!("expected PromptTooLong, got {:?}", kind),
        }
    }

    #[test]
    fn classify_context_length_exceeded() {
        let kind =
            ApiErrorClassifier::classify(None, "context_length_exceeded: 130000 > 128000", None);
        assert!(matches!(kind, ApiErrorKind::PromptTooLong { .. }));
    }

    #[test]
    fn classify_maximum_context_length() {
        let kind = ApiErrorClassifier::classify(
            None,
            "This model's maximum context length is 128000 tokens, however you requested 150000",
            None,
        );
        match kind {
            ApiErrorKind::PromptTooLong {
                actual_tokens,
                limit_tokens,
            } => {
                assert_eq!(actual_tokens, Some(150000));
                assert_eq!(limit_tokens, Some(128000));
            }
            _ => panic!("expected PromptTooLong"),
        }
    }

    #[test]
    fn classify_max_output_tokens() {
        let kind = ApiErrorClassifier::classify(None, "max_tokens output limit reached", None);
        assert_eq!(kind, ApiErrorKind::MaxOutputTokens);
    }

    #[test]
    fn classify_invalid_api_key_from_text() {
        let kind = ApiErrorClassifier::classify(None, "Invalid API key: sk-xxx is not valid", None);
        assert_eq!(kind, ApiErrorKind::InvalidApiKey);
    }

    #[test]
    fn classify_rate_limit_from_text() {
        let kind =
            ApiErrorClassifier::classify(None, "rate_limit_exceeded: retry in 30 seconds", None);
        match kind {
            ApiErrorKind::RateLimited { retry_after } => {
                assert_eq!(retry_after, Some(Duration::from_secs(30)));
            }
            _ => panic!("expected RateLimited"),
        }
    }

    #[test]
    fn classify_overloaded_from_text() {
        let kind = ApiErrorClassifier::classify(None, "The server is currently overloaded", None);
        assert_eq!(kind, ApiErrorKind::Overloaded);
    }

    #[test]
    fn classify_capacity_overloaded() {
        let kind = ApiErrorClassifier::classify(None, "No capacity available for this model", None);
        assert_eq!(kind, ApiErrorKind::Overloaded);
    }

    #[test]
    fn classify_image_too_large() {
        let kind = ApiErrorClassifier::classify(None, "Image is too large to process", None);
        assert!(matches!(kind, ApiErrorKind::ImageTooLarge { .. }));
    }

    #[test]
    fn classify_pdf_too_large() {
        let kind = ApiErrorClassifier::classify(None, "PDF has too many pages to process", None);
        assert!(matches!(kind, ApiErrorKind::PdfTooLarge { .. }));
    }

    #[test]
    fn classify_model_not_found() {
        let kind = ApiErrorClassifier::classify(Some(404), "Model gpt-5 not found", None);
        assert!(matches!(kind, ApiErrorKind::ModelNotAvailable { .. }));
    }

    #[test]
    fn classify_model_not_available() {
        let kind = ApiErrorClassifier::classify(None, "Model claude-99 is not available", None);
        assert!(matches!(kind, ApiErrorKind::ModelNotAvailable { .. }));
    }

    #[test]
    fn classify_budget_exhausted() {
        let kind =
            ApiErrorClassifier::classify(None, "Token budget exhausted for this session", None);
        assert_eq!(kind, ApiErrorKind::BudgetExhausted);
    }

    #[test]
    fn classify_timeout() {
        let kind = ApiErrorClassifier::classify(None, "Request timed out after 30s", None);
        assert_eq!(kind, ApiErrorKind::ConnectionTimeout);
    }

    #[test]
    fn classify_connection_reset() {
        let kind = ApiErrorClassifier::classify(None, "Connection reset by peer", None);
        assert_eq!(kind, ApiErrorKind::ConnectionReset);
    }

    #[test]
    fn classify_connection_closed() {
        let kind = ApiErrorClassifier::classify(None, "Connection closed unexpectedly", None);
        assert_eq!(kind, ApiErrorKind::ConnectionReset);
    }

    #[test]
    fn classify_unknown() {
        let kind = ApiErrorClassifier::classify(Some(500), "Internal server error", None);
        assert!(matches!(kind, ApiErrorKind::Unknown(_)));
    }

    // ── parse_prompt_too_long_tokens tests ──────────────────────────

    #[test]
    fn parse_tokens_two_numbers_larger_first() {
        let (actual, limit) =
            ApiErrorClassifier::parse_prompt_too_long_tokens("150000 tokens exceeds 128000 limit");
        assert_eq!(actual, Some(150000));
        assert_eq!(limit, Some(128000));
    }

    #[test]
    fn parse_tokens_two_numbers_smaller_first() {
        let (actual, limit) = ApiErrorClassifier::parse_prompt_too_long_tokens(
            "maximum context length is 128000 tokens, you requested 150000",
        );
        assert_eq!(actual, Some(150000));
        assert_eq!(limit, Some(128000));
    }

    #[test]
    fn parse_tokens_single_number_with_limit_keyword() {
        let (actual, limit) =
            ApiErrorClassifier::parse_prompt_too_long_tokens("maximum context limit is 128000");
        assert_eq!(actual, None);
        assert_eq!(limit, Some(128000));
    }

    #[test]
    fn parse_tokens_no_numbers() {
        let (actual, limit) =
            ApiErrorClassifier::parse_prompt_too_long_tokens("prompt is too long");
        assert_eq!(actual, None);
        assert_eq!(limit, None);
    }

    // ── recovery_message tests ──────────────────────────────────────

    #[test]
    fn recovery_message_prompt_too_long() {
        let msg = ApiErrorClassifier::recovery_message(&ApiErrorKind::PromptTooLong {
            actual_tokens: Some(150000),
            limit_tokens: Some(128000),
        });
        assert!(msg.contains("150000"));
        assert!(msg.contains("128000"));
        assert!(msg.contains("auto-compact"));
    }

    #[test]
    fn recovery_message_rate_limited_with_duration() {
        let msg = ApiErrorClassifier::recovery_message(&ApiErrorKind::RateLimited {
            retry_after: Some(Duration::from_secs(45)),
        });
        assert!(msg.contains("45s"));
    }

    #[test]
    fn recovery_message_rate_limited_default() {
        let msg =
            ApiErrorClassifier::recovery_message(&ApiErrorKind::RateLimited { retry_after: None });
        assert!(msg.contains("30s"));
    }

    #[test]
    fn recovery_message_all_kinds_non_empty() {
        let kinds = vec![
            ApiErrorKind::PromptTooLong {
                actual_tokens: None,
                limit_tokens: None,
            },
            ApiErrorKind::RateLimited { retry_after: None },
            ApiErrorKind::Overloaded,
            ApiErrorKind::AuthExpired,
            ApiErrorKind::MaxOutputTokens,
            ApiErrorKind::InvalidApiKey,
            ApiErrorKind::ImageTooLarge {
                path: "test.png".into(),
                size: 10_000_000,
            },
            ApiErrorKind::PdfTooLarge {
                path: "doc.pdf".into(),
                pages: 500,
            },
            ApiErrorKind::ConnectionTimeout,
            ApiErrorKind::ConnectionReset,
            ApiErrorKind::StreamInterrupted,
            ApiErrorKind::ModelNotAvailable {
                model: "gpt-99".into(),
            },
            ApiErrorKind::BudgetExhausted,
            ApiErrorKind::Unknown("something".into()),
        ];
        for kind in &kinds {
            let msg = ApiErrorClassifier::recovery_message(kind);
            assert!(!msg.is_empty(), "empty message for {:?}", kind);
        }
    }

    // ── format_file_size tests ──────────────────────────────────────

    #[test]
    fn format_file_size_zero() {
        assert_eq!(format_file_size(0), "0 B");
    }

    #[test]
    fn format_file_size_bytes() {
        assert_eq!(format_file_size(512), "512 B");
    }

    #[test]
    fn format_file_size_megabytes() {
        let size = 5 * 1024 * 1024;
        let result = format_file_size(size);
        assert!(result.contains("5.0 MB"));
    }
}
