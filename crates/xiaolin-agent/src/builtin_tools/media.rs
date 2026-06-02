use std::collections::HashMap;

use async_trait::async_trait;
use xiaolin_core::tool::{Tool, ToolParameterSchema, ToolResult};

// --- Media Generation Tools ---

/// Generate images using an AI model API (OpenAI DALL-E compatible).
pub struct ImageGenerateTool {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl ImageGenerateTool {
    pub fn new(base_url: &str, api_key: &str) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .unwrap_or_default(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
        }
    }
}

#[async_trait]
impl Tool for ImageGenerateTool {
    fn name(&self) -> &str {
        "image_generate"
    }

    fn description(&self) -> &str {
        "Call an OpenAI-compatible POST /images/generations endpoint to create one new raster image from a text prompt. JSON result includes url or b64 data (depending on API), optional revised_prompt, model, and size. \
         Use when the user wants generated artwork, mockups, or illustrative bitmaps—not for inpainting/editing existing files (unsupported) and not when SVG/HTML would be clearer. \
         Credentials and base_url come from deployment config; typical failures are auth, quota, content policy, or invalid model/size pairs. \
         Strong prompts name subject, composition, palette, lighting, and negatives (e.g. 'no legible text'). \
         size must be an allowed enum for the provider; model defaults to dall-e-3 unless your proxy expects another id. \
         Anti-pattern: one-word prompts like 'logo' with no style context. \
         Example: {\"prompt\": \"Flat illustration of a red panda coding on a laptop, pastel background, no text\", \"size\": \"1024x1024\"}."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "prompt".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Full natural-language brief for the image. Examples: 'Flat vector icons for settings, mail, calendar, pastel blue on white'; 'Photorealistic oak desk, 35mm, shallow depth of field'. Include negatives the user cares about (e.g. 'no watermark')."
            }),
        );
        props.insert(
            "size".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["256x256", "512x512", "1024x1024", "1792x1024", "1024x1792"],
                "description": "Output dimensions the upstream API accepts. Default when omitted: 1024x1024. Use wide/tall enums only when the provider supports them for the chosen model."
            }),
        );
        props.insert(
            "model".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Image model id sent to the API (default 'dall-e-3'). Examples: 'dall-e-3', 'dall-e-2' if your base_url proxies them. Must match whatever the configured server exposes."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["prompt".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "image_generate: arguments are not valid JSON: {e}. \
                 Pass {{\"prompt\": \"...\", \"size\": \"1024x1024\", \"model\": \"dall-e-3\"}} with double-quoted keys; size and model are optional."
            )),
        };

        let prompt = match args.get("prompt").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolResult::err(
                "image_generate is missing required string field 'prompt'. \
                 Example: {\"prompt\": \"Watercolor map of Europe, muted colors\", \"size\": \"1024x1024\"}."
                    .to_string(),
            ),
        };

        let size = args
            .get("size")
            .and_then(|v| v.as_str())
            .unwrap_or("1024x1024");
        let model = args
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("dall-e-3");

        let body = serde_json::json!({
            "model": model,
            "prompt": prompt,
            "n": 1,
            "size": size,
        });

        let resp = match self
            .client
            .post(format!("{}/images/generations", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => return ToolResult::err(format!(
                "image_generate could not reach '{}/images/generations': {e}. \
                 What to do next: verify base_url, outbound HTTPS, DNS, and firewall rules; confirm the gateway host clock is sane for TLS; shorten the prompt and retry once if the body might be too large for an intermediary.",
                self.base_url
            )),
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return ToolResult::err(format!(
                "image_generate API returned HTTP {status}. Response body (often JSON with error message): {text}. \
                 What to do next: on 401/403 fix API key or IAM; on 400 read the message for unsupported size/model or content_policy violation—rewrite the prompt without disallowed content; on 429 wait or reduce usage; on 5xx retry later or try another model."
            ));
        }

        let json: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "image_generate received success HTTP but the body was not valid JSON: {e}. \
                 What to do next: confirm the base_url points to an OpenAI-compatible images endpoint; capture a raw trace for the operator if a proxy stripped the body."
            )),
        };

        let url = json
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|a| a.first())
            .and_then(|i| i.get("url").or_else(|| i.get("b64_json")))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let revised_prompt = json
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|a| a.first())
            .and_then(|i| i.get("revised_prompt"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        ToolResult::ok(
            serde_json::json!({
                "url": url,
                "revised_prompt": revised_prompt,
                "model": model,
                "size": size,
            })
            .to_string(),
        )
    }
}

/// Text-to-speech synthesis using an API (OpenAI TTS compatible).
pub struct TtsTool {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl TtsTool {
    pub fn new(base_url: &str, api_key: &str) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
        }
    }
}

#[async_trait]
impl Tool for TtsTool {
    fn name(&self) -> &str {
        "text_to_speech"
    }

    fn description(&self) -> &str {
        "Call an OpenAI-compatible POST /audio/speech with model tts-1, synthesize speech from plain text, write an audio file on the gateway disk, and return JSON {path, bytes, voice}. \
         For narration, accessibility readouts, or short announcements—not music composition, sfx design, or multi-voice drama (single preset voice per call). \
         voice is one of alloy, echo, fable, onyx, nova, shimmer; default alloy. output_path sets the destination (mp3); omit for /tmp/xiaolin_tts_<timestamp>.mp3—pick a writable directory. \
         Model is fixed to tts-1 in this tool; changing speech engines requires deployment configuration, not parameters. \
         Very long inputs may hit provider limits—split into chunks if needed. \
         Anti-pattern: feeding raw Markdown tables—preprocess to speakable sentences unless literal reading was requested. \
         Example: {\"text\": \"Deployment succeeded. All health checks are green.\", \"voice\": \"nova\", \"output_path\": \"/tmp/deploy_ok.mp3\"}."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "text".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "UTF-8 text to speak. Examples: a two-sentence status update; a short paragraph for voicemail. Very long text may hit provider limits—split into multiple calls if needed."
            }),
        );
        props.insert(
            "voice".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["alloy", "echo", "fable", "onyx", "nova", "shimmer"],
                "description": "Preset voice id (default alloy). Pick nova/shimmer for brighter tone, onyx for deeper—match user preference when they named a style."
            }),
        );
        props.insert(
            "output_path".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Filesystem path for the written audio file (typically .mp3). Example: '/tmp/xiaolin_summary.mp3'. Parent directories are created when possible. Omit to use /tmp/xiaolin_tts_<timestamp>.mp3."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["text".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "text_to_speech: arguments are not valid JSON: {e}. \
                 Pass {{\"text\": \"...\", \"voice\": \"alloy\", \"output_path\": \"/tmp/out.mp3\"}} with double-quoted keys; voice and output_path are optional."
            )),
        };

        let text = match args.get("text").and_then(|v| v.as_str()) {
            Some(t) if !t.is_empty() => t,
            _ => {
                return ToolResult::err(
                    "text_to_speech is missing or empty required string field 'text'. \
                 Example: {\"text\": \"Your deployment is live.\"}. \
                 Strip to spoken content if you were given Markdown-only scaffolding."
                        .to_string(),
                )
            }
        };

        let voice = args
            .get("voice")
            .and_then(|v| v.as_str())
            .unwrap_or("alloy");
        let output_path = args
            .get("output_path")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_else(|| {
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0);
                format!("/tmp/xiaolin_tts_{ts}.mp3")
            });

        let body = serde_json::json!({
            "model": "tts-1",
            "input": text,
            "voice": voice,
        });

        let resp = match self
            .client
            .post(format!("{}/audio/speech", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => return ToolResult::err(format!(
                "text_to_speech could not reach '{}/audio/speech': {e}. \
                 What to do next: verify base_url, API key, outbound HTTPS, and DNS; retry with shorter text if a proxy timed out.",
                self.base_url
            )),
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return ToolResult::err(format!(
                "text_to_speech API returned HTTP {status}. Response body: {text}. \
                 What to do next: on 401/403 fix credentials; on 400 validate voice enum and input length; on 429 back off; on 5xx retry later."
            ));
        }

        let bytes = match resp.bytes().await {
            Ok(b) => b,
            Err(e) => return ToolResult::err(format!(
                "text_to_speech got HTTP success but failed while reading the audio bytes: {e}. \
                 What to do next: retry once; if it persists, check for proxies that mishandle binary bodies."
            )),
        };

        if let Some(parent) = std::path::Path::new(&output_path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        match std::fs::write(&output_path, &bytes) {
            Ok(()) => ToolResult::ok(
                serde_json::json!({
                    "path": output_path,
                    "bytes": bytes.len(),
                    "voice": voice,
                })
                .to_string(),
            ),
            Err(e) => ToolResult::err(format!(
                "text_to_speech could not write audio to '{output_path}': {e}. \
                 What to do next: choose a writable path (often under /tmp), ensure parent directories exist or omit output_path to use the default under /tmp, and check disk quota."
            )),
        }
    }
}
