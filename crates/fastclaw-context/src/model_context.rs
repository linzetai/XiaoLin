const DEFAULT_CONTEXT_WINDOW: u32 = 128_000;
const DEFAULT_OUTPUT_LIMIT: u32 = 32_000;

/// Robust model name normalizer: strips provider prefixes, date/version suffixes,
/// quantization info, pipes, colons, etc. to give a canonical form for pattern matching.
pub fn normalize_model_name(model: &str) -> String {
    let mut s = model.to_lowercase();

    // strip provider prefix (e.g. "google/gemini-..." -> "gemini-...")
    if let Some(pos) = s.rfind('/') {
        s = s[pos + 1..].to_string();
    }
    // handle pipe separator
    if let Some(pos) = s.rfind('|') {
        s = s[pos + 1..].to_string();
    }
    // handle colon separator
    if let Some(pos) = s.rfind(':') {
        s = s[pos + 1..].to_string();
    }

    s = s.trim().to_string();

    // collapse whitespace to hyphens
    let mut out = String::with_capacity(s.len());
    let mut prev_ws = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_ws {
                out.push('-');
            }
            prev_ws = true;
        } else {
            prev_ws = false;
            out.push(ch);
        }
    }
    s = out;

    // remove -preview
    s = s.replace("-preview", "");

    // remove trailing date/version/tag suffixes unless it's a known exception
    let is_qwen_latest = s.starts_with("qwen-plus-latest")
        || s.starts_with("qwen-flash-latest")
        || s.starts_with("qwen-vl-max-latest");
    let is_kimi_dated = {
        // kimi-k2-0905 style
        s.starts_with("kimi-k2-") && s.len() >= 12 && s[8..].chars().all(|c| c.is_ascii_digit())
    };

    if !is_qwen_latest && !is_kimi_dated {
        // Remove trailing: -20250219, -0528 (4+ digit dates), -7b, -70b, -4x8b,
        // -v1, -v1.2, -latest, -exp
        let patterns: &[&str] = &["-latest", "-exp"];
        for pat in patterns {
            if s.ends_with(pat) {
                s.truncate(s.len() - pat.len());
            }
        }

        // strip trailing date suffix like -20250514
        loop {
            let trimmed = strip_trailing_suffix(&s);
            if trimmed.len() == s.len() {
                break;
            }
            s = trimmed;
        }
    }

    // remove quantization suffixes: -4bit, -int4, -int8, -bf16, -fp16, -q4, -q5, -quantized
    let quant_suffixes = [
        "-4bit",
        "-8bit",
        "-int4",
        "-int8",
        "-bf16",
        "-fp16",
        "-q4",
        "-q5",
        "-quantized",
    ];
    for q in &quant_suffixes {
        if s.ends_with(q) {
            s.truncate(s.len() - q.len());
        }
    }

    // Re-run trailing suffix stripping after quant removal
    // (e.g. "llama-3-8b-int4" -> "llama-3-8b" -> "llama-3")
    loop {
        let trimmed = strip_trailing_suffix(&s);
        if trimmed.len() == s.len() {
            break;
        }
        s = trimmed;
    }

    s
}

/// Strip one trailing suffix pattern (date, version, size). Returns shorter
/// string if matched, same length otherwise.
fn strip_trailing_suffix(s: &str) -> String {
    // Try to find a trailing "-NNNN..." (4+ digits = date)
    if let Some(dash) = s.rfind('-') {
        let tail = &s[dash + 1..];
        if tail.len() >= 4 && tail.chars().all(|c| c.is_ascii_digit()) {
            return s[..dash].to_string();
        }
        // -v1, -v1.2, -v2.1.3
        if tail.starts_with('v') && tail.len() >= 2 {
            let rest = &tail[1..];
            if rest.chars().all(|c| c.is_ascii_digit() || c == '.')
                && rest.contains(|c: char| c.is_ascii_digit())
            {
                return s[..dash].to_string();
            }
        }
        // -7b, -70b, -4x8b
        if tail
            .chars()
            .all(|c| c.is_ascii_digit() || c == 'x' || c == 'b')
            && tail.ends_with('b')
            && tail.len() >= 2
        {
            return s[..dash].to_string();
        }
    }
    s.to_string()
}

/// Model limit type: input context window or output token limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenLimitType {
    Input,
    Output,
}

struct PatternEntry {
    prefix: &'static str,
    input: u32,
    output: u32,
}

static MODEL_LIMITS: &[PatternEntry] = &[
    // Google Gemini
    PatternEntry {
        prefix: "gemini-3",
        input: 1_000_000,
        output: 64_000,
    },
    PatternEntry {
        prefix: "gemini-",
        input: 1_000_000,
        output: 8_192,
    },
    // OpenAI
    PatternEntry {
        prefix: "gpt-5",
        input: 272_000,
        output: 128_000,
    },
    PatternEntry {
        prefix: "gpt-4.1",
        input: 1_000_000,
        output: 32_768,
    },
    PatternEntry {
        prefix: "gpt-4o",
        input: 128_000,
        output: 16_384,
    },
    PatternEntry {
        prefix: "gpt-4",
        input: 128_000,
        output: 16_384,
    },
    PatternEntry {
        prefix: "o4-mini",
        input: 200_000,
        output: 128_000,
    },
    PatternEntry {
        prefix: "o3",
        input: 200_000,
        output: 128_000,
    },
    PatternEntry {
        prefix: "o1",
        input: 200_000,
        output: 128_000,
    },
    // Anthropic Claude
    PatternEntry {
        prefix: "claude-opus-4-6",
        input: 200_000,
        output: 128_000,
    },
    PatternEntry {
        prefix: "claude-sonnet-4-6",
        input: 200_000,
        output: 64_000,
    },
    PatternEntry {
        prefix: "claude-",
        input: 200_000,
        output: 64_000,
    },
    // Qwen
    PatternEntry {
        prefix: "qwen3-coder-plus",
        input: 1_000_000,
        output: 64_000,
    },
    PatternEntry {
        prefix: "qwen3-coder-flash",
        input: 1_000_000,
        output: 64_000,
    },
    PatternEntry {
        prefix: "qwen3.5-plus",
        input: 1_000_000,
        output: 64_000,
    },
    PatternEntry {
        prefix: "qwen3.5",
        input: 1_000_000,
        output: 64_000,
    },
    PatternEntry {
        prefix: "qwen3-max",
        input: 256_000,
        output: 32_000,
    },
    PatternEntry {
        prefix: "qwen-plus-latest",
        input: 1_000_000,
        output: 32_000,
    },
    PatternEntry {
        prefix: "qwen-flash-latest",
        input: 1_000_000,
        output: 32_000,
    },
    PatternEntry {
        prefix: "qwen-plus",
        input: 1_000_000,
        output: 32_000,
    },
    PatternEntry {
        prefix: "qwen-max",
        input: 256_000,
        output: 32_000,
    },
    PatternEntry {
        prefix: "qwen",
        input: 256_000,
        output: 32_000,
    },
    // DeepSeek
    PatternEntry {
        prefix: "deepseek-v4-pro",
        input: 128_000,
        output: 64_000,
    },
    PatternEntry {
        prefix: "deepseek-v4-flash",
        input: 128_000,
        output: 64_000,
    },
    PatternEntry {
        prefix: "deepseek-reasoner",
        input: 128_000,
        output: 64_000,
    },
    PatternEntry {
        prefix: "deepseek-r1",
        input: 128_000,
        output: 64_000,
    },
    PatternEntry {
        prefix: "deepseek-chat",
        input: 128_000,
        output: 8_192,
    },
    PatternEntry {
        prefix: "deepseek",
        input: 128_000,
        output: 8_192,
    },
    // GLM
    PatternEntry {
        prefix: "glm-5",
        input: 202_752,
        output: 16_384,
    },
    PatternEntry {
        prefix: "glm-4",
        input: 128_000,
        output: 16_384,
    },
    // MiniMax
    PatternEntry {
        prefix: "minimax-m2.5",
        input: 196_608,
        output: 64_000,
    },
    PatternEntry {
        prefix: "minimax-",
        input: 200_000,
        output: 32_000,
    },
    // Kimi
    PatternEntry {
        prefix: "kimi-",
        input: 256_000,
        output: 32_000,
    },
    // Seed
    PatternEntry {
        prefix: "seed-oss",
        input: 524_288,
        output: 32_000,
    },
];

fn find_limit(model: &str, limit_type: TokenLimitType) -> Option<u32> {
    let norm = normalize_model_name(model);
    for entry in MODEL_LIMITS {
        if norm.starts_with(entry.prefix) || norm.contains(entry.prefix) {
            return Some(match limit_type {
                TokenLimitType::Input => entry.input,
                TokenLimitType::Output => entry.output,
            });
        }
    }
    None
}

/// Infer context window size (input tokens) from model name using known patterns.
/// Returns a sensible default (128K) for unknown models.
pub fn infer_context_window_from_model(model: &str) -> u32 {
    find_limit(model, TokenLimitType::Input).unwrap_or(DEFAULT_CONTEXT_WINDOW)
}

/// Infer maximum output tokens from model name.
/// Returns a sensible default (32K) for unknown models.
pub fn infer_output_limit_from_model(model: &str) -> u32 {
    find_limit(model, TokenLimitType::Output).unwrap_or(DEFAULT_OUTPUT_LIMIT)
}

/// Check if a model has an explicitly known output limit (vs. just using default).
pub fn has_explicit_output_limit(model: &str) -> bool {
    find_limit(model, TokenLimitType::Output).is_some()
}

/// Minimal last-resort heuristic for models that are definitely text-only.
/// Prefer declaring `ModelCapabilities` in config/plugin definitions instead
/// of adding entries here.
static TEXT_ONLY_PREFIXES: &[&str] = &["deepseek", "seed-oss"];

/// Heuristic fallback: returns `true` when the model is likely to accept
/// `image_url` content parts based on name patterns. Only used when no
/// explicit `ModelCapabilities` is configured for the model.
pub fn model_supports_vision(model: &str) -> bool {
    let norm = normalize_model_name(model);
    !TEXT_ONLY_PREFIXES.iter().any(|p| norm.starts_with(p))
}

/// Like `model_supports_vision` but respects an explicit `ModelCapabilities`
/// override. When `caps` is `Some`, its declaration takes precedence over the
/// heuristic prefix list.
pub fn model_supports_vision_with_caps(
    model: &str,
    caps: Option<&fastclaw_core::types::ModelCapabilities>,
) -> bool {
    if let Some(c) = caps {
        return c.supports_vision();
    }
    model_supports_vision(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── normalize tests ─────────────────────────────────────────

    #[test]
    fn strip_provider_prefix() {
        assert_eq!(
            normalize_model_name("google/gemini-2.0-flash"),
            "gemini-2.0-flash"
        );
        assert_eq!(
            normalize_model_name("anthropic/claude-3.5-sonnet"),
            "claude-3.5-sonnet"
        );
    }

    #[test]
    fn strip_date_suffix() {
        assert_eq!(
            normalize_model_name("claude-sonnet-4-6-20250514"),
            "claude-sonnet-4-6"
        );
        // Compact 8-digit date suffix
        assert_eq!(normalize_model_name("gpt-4o-20250528"), "gpt-4o");
    }

    #[test]
    fn strip_version_suffix() {
        assert_eq!(normalize_model_name("some-model-v1"), "some-model");
        assert_eq!(normalize_model_name("some-model-v2.1"), "some-model");
    }

    #[test]
    fn strip_quantization() {
        assert_eq!(normalize_model_name("llama-3-8b-int4"), "llama-3");
        assert_eq!(normalize_model_name("model-bf16"), "model");
    }

    #[test]
    fn strip_latest_and_preview() {
        assert_eq!(normalize_model_name("gpt-4o-preview"), "gpt-4o");
        assert_eq!(normalize_model_name("some-model-latest"), "some-model");
    }

    #[test]
    fn preserve_qwen_latest_exceptions() {
        assert_eq!(normalize_model_name("qwen-plus-latest"), "qwen-plus-latest");
        assert_eq!(
            normalize_model_name("qwen-flash-latest"),
            "qwen-flash-latest"
        );
    }

    #[test]
    fn pipe_and_colon() {
        assert_eq!(normalize_model_name("provider|gpt-4o"), "gpt-4o");
        assert_eq!(normalize_model_name("ns:gpt-4o"), "gpt-4o");
    }

    #[test]
    fn strip_size_suffix() {
        assert_eq!(normalize_model_name("qwen-3-70b"), "qwen-3");
        assert_eq!(normalize_model_name("llama-3-4x8b"), "llama-3");
    }

    // ─── context window inference tests ──────────────────────────

    #[test]
    fn known_models_context_window() {
        assert_eq!(infer_context_window_from_model("gpt-4o-mini"), 128_000);
        assert_eq!(
            infer_context_window_from_model("claude-sonnet-4-6-20250514"),
            200_000
        );
        assert_eq!(
            infer_context_window_from_model("qwen3-coder-plus"),
            1_000_000
        );
        assert_eq!(infer_context_window_from_model("deepseek-r1"), 128_000);
        assert_eq!(
            infer_context_window_from_model("google/gemini-2.0-flash"),
            1_000_000
        );
        assert_eq!(infer_context_window_from_model("gpt-5-preview"), 272_000);
    }

    #[test]
    fn unknown_model_gets_default() {
        assert_eq!(
            infer_context_window_from_model("some-random-model"),
            DEFAULT_CONTEXT_WINDOW
        );
    }

    // ─── output limit inference tests ────────────────────────────

    #[test]
    fn known_models_output_limit() {
        assert_eq!(infer_output_limit_from_model("gpt-5"), 128_000);
        assert_eq!(infer_output_limit_from_model("gpt-4o"), 16_384);
        assert_eq!(infer_output_limit_from_model("claude-opus-4-6"), 128_000);
        assert_eq!(
            infer_output_limit_from_model("claude-sonnet-4-6-20250514"),
            64_000
        );
        assert_eq!(infer_output_limit_from_model("deepseek-r1"), 64_000);
        assert_eq!(infer_output_limit_from_model("deepseek-chat"), 8_192);
        assert_eq!(infer_output_limit_from_model("gemini-3.5-pro"), 64_000);
    }

    #[test]
    fn unknown_model_output_default() {
        assert_eq!(
            infer_output_limit_from_model("some-random-model"),
            DEFAULT_OUTPUT_LIMIT
        );
    }

    #[test]
    fn has_explicit_vs_not() {
        assert!(has_explicit_output_limit("gpt-4o"));
        assert!(!has_explicit_output_limit("some-random-model"));
    }

    // ─── vision support tests ─────────────────────────────────────

    #[test]
    fn deepseek_no_vision() {
        assert!(!model_supports_vision("deepseek-r1"));
        assert!(!model_supports_vision("deepseek-chat"));
        assert!(!model_supports_vision("deepseek-v4-pro"));
        assert!(!model_supports_vision("deepseek-v4-flash"));
    }

    #[test]
    fn vision_models() {
        assert!(model_supports_vision("gpt-4o"));
        assert!(model_supports_vision("gpt-5"));
        assert!(model_supports_vision("claude-sonnet-4-6"));
        assert!(model_supports_vision("gemini-2.0-flash"));
        assert!(model_supports_vision("qwen-vl-max-latest"));
    }

    #[test]
    fn unknown_model_assumes_vision() {
        assert!(model_supports_vision("some-random-model"));
    }

    #[test]
    fn caps_override_heuristic() {
        use fastclaw_core::types::{InputModality, ModelCapabilities, OutputModality};

        let text_only = ModelCapabilities {
            input: vec![InputModality::Text],
            output: vec![OutputModality::Text, OutputModality::ToolCalls],
        };
        // Explicit caps override heuristic: gpt-4o normally has vision,
        // but config says text-only.
        assert!(!model_supports_vision_with_caps("gpt-4o", Some(&text_only)));

        let multimodal = ModelCapabilities {
            input: vec![InputModality::Text, InputModality::Image],
            output: vec![OutputModality::Text],
        };
        // Explicit caps override heuristic: deepseek is normally text-only,
        // but config declares image support.
        assert!(model_supports_vision_with_caps(
            "deepseek-chat",
            Some(&multimodal)
        ));

        // None falls back to heuristic.
        assert!(!model_supports_vision_with_caps("deepseek-chat", None));
        assert!(model_supports_vision_with_caps("gpt-4o", None));
    }
}
