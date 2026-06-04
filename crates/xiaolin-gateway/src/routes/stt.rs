use axum::{
    body::Bytes,
    extract::State,
    http::{header, StatusCode},
    response::IntoResponse,
};
use serde::Serialize;

use crate::state::AppState;

#[derive(Serialize)]
struct SttResponse {
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    language: Option<String>,
}

#[derive(Serialize)]
struct SttError {
    error: String,
}

/// POST /v1/audio/transcriptions
///
/// Accepts multipart/form-data with a `file` part (audio) and optional `model` text part.
/// Tries the agent's configured LLM provider first; falls back to local `whisper` CLI.
pub(super) async fn audio_transcriptions(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if !content_type.starts_with("multipart/form-data") {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(SttError {
                error: "Content-Type must be multipart/form-data".into(),
            }),
        )
            .into_response();
    }

    // Try provider-based transcription first
    match try_provider_stt(&state, &content_type, &body).await {
        Ok(resp) => return axum::Json(resp).into_response(),
        Err(e) => {
            tracing::debug!(error = %e, "provider STT failed, trying local whisper");
        }
    }

    // Fall back to local whisper CLI
    match try_local_whisper(&content_type, &body).await {
        Ok(resp) => axum::Json(resp).into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "all STT backends failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                axum::Json(SttError {
                    error: format!(
                        "No STT backend available. Configure a provider API key or install whisper CLI. Detail: {e}"
                    ),
                }),
            )
                .into_response()
        }
    }
}

async fn try_provider_stt(
    state: &AppState,
    content_type: &str,
    body: &[u8],
) -> Result<SttResponse, String> {
    let credentials = state.current_credentials_snapshot();

    // Try providers that support /v1/audio/transcriptions (OpenAI-compatible)
    let providers_to_try = ["openai", "dashscope", "deepseek"];

    for provider_name in &providers_to_try {
        let api_key = match credentials.get_api_key(provider_name) {
            Some(k) if !k.is_empty() => k.to_string(),
            _ => continue,
        };

        let base_url = credentials
            .get_base_url(provider_name)
            .unwrap_or("https://api.openai.com/v1")
            .trim_end_matches('/');

        let stt_url = format!("{base_url}/audio/transcriptions");

        tracing::debug!(provider = provider_name, url = %stt_url, "trying provider STT");

        let client = reqwest::Client::new();
        let resp = match client
            .post(&stt_url)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", content_type)
            .body(body.to_vec())
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(provider = provider_name, error = %e, "provider STT request failed");
                continue;
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            tracing::debug!(
                provider = provider_name,
                status = %status,
                body = %body_text,
                "provider STT returned non-success"
            );
            continue;
        }

        #[derive(serde::Deserialize)]
        struct WhisperResp {
            text: String,
            language: Option<String>,
        }

        match resp.json::<WhisperResp>().await {
            Ok(r) => {
                return Ok(SttResponse {
                    text: r.text,
                    language: r.language,
                });
            }
            Err(e) => {
                tracing::debug!(provider = provider_name, error = %e, "failed to parse STT response");
                continue;
            }
        }
    }

    Err("no provider with valid API key supports audio transcription".into())
}

async fn try_local_whisper(content_type: &str, body: &[u8]) -> Result<SttResponse, String> {
    let boundary = extract_boundary(content_type)
        .ok_or_else(|| "cannot parse multipart boundary".to_string())?;

    let audio_data = extract_file_part(body, &boundary)
        .ok_or_else(|| "no 'file' part in multipart body".to_string())?;

    let tmp_dir = std::env::temp_dir();
    let input_path = tmp_dir.join(format!("xiaolin-stt-{}.wav", std::process::id()));

    std::fs::write(&input_path, &audio_data)
        .map_err(|e| format!("failed to write temp audio: {e}"))?;

    let result = tokio::task::spawn_blocking({
        let input = input_path.clone();
        move || run_whisper_cli(&input)
    })
    .await
    .map_err(|e| format!("whisper task panicked: {e}"))?;

    let _ = std::fs::remove_file(&input_path);

    result
}

fn run_whisper_cli(audio_path: &std::path::Path) -> Result<SttResponse, String> {
    // Try whisper (OpenAI whisper CLI / whisper.cpp main)
    let whisper_cmds = ["whisper", "whisper-cpp", "main"];

    for cmd in &whisper_cmds {
        let output = std::process::Command::new(cmd)
            .arg(audio_path.to_str().unwrap_or(""))
            .arg("--language")
            .arg("auto")
            .arg("--output_format")
            .arg("txt")
            .arg("--output_dir")
            .arg(audio_path.parent().unwrap_or(std::path::Path::new("/tmp")))
            .output();

        match output {
            Ok(o) if o.status.success() => {
                // whisper outputs to <input_name>.txt
                let txt_path = audio_path.with_extension("txt");
                let text = if txt_path.exists() {
                    let t = std::fs::read_to_string(&txt_path).unwrap_or_default();
                    let _ = std::fs::remove_file(&txt_path);
                    t.trim().to_string()
                } else {
                    String::from_utf8_lossy(&o.stdout).trim().to_string()
                };

                if !text.is_empty() {
                    return Ok(SttResponse {
                        text,
                        language: None,
                    });
                }
            }
            Ok(o) => {
                tracing::debug!(
                    cmd,
                    status = %o.status,
                    stderr = %String::from_utf8_lossy(&o.stderr),
                    "whisper CLI failed"
                );
            }
            Err(_) => {
                tracing::debug!(cmd, "whisper CLI not found");
            }
        }
    }

    Err("whisper CLI not installed (try: pip install openai-whisper)".into())
}

fn extract_boundary(content_type: &str) -> Option<String> {
    content_type
        .split(';')
        .find_map(|part| {
            let part = part.trim();
            part.strip_prefix("boundary=")
                .map(|s| s.trim_matches('"').to_string())
        })
}

fn extract_file_part(body: &[u8], boundary: &str) -> Option<Vec<u8>> {
    let body_str = String::from_utf8_lossy(body);
    let delimiter = format!("--{boundary}");

    let parts: Vec<&str> = body_str.split(&delimiter).collect();

    for part in parts {
        if part.contains("name=\"file\"") || part.contains("name=file") {
            if let Some(data_start) = part.find("\r\n\r\n") {
                let data = &part[data_start + 4..];
                let data = data.trim_end_matches("\r\n");
                return Some(data.as_bytes().to_vec());
            }
        }
    }

    None
}
