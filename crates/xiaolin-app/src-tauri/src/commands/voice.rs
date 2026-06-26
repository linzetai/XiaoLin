use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::process::Command as TokioCommand;

/// Maximum base64-encoded audio payload (~25 MiB raw audio at ~1.33x encoding overhead).
const MAX_AUDIO_BASE64_LEN: usize = 35_000_000;

#[derive(Debug, Serialize, Deserialize)]
pub struct SttResult {
    pub text: String,
    pub language: Option<String>,
}

#[tauri::command]
pub async fn transcribe_audio(
    audio_base64: String,
    mime_type: String,
    state: tauri::State<'_, crate::AppData>,
) -> Result<SttResult, String> {
    if audio_base64.len() > MAX_AUDIO_BASE64_LEN {
        return Err("Audio data too large (max 25MB)".to_string());
    }
    let audio_data =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &audio_base64)
            .map_err(|e| format!("Invalid base64 audio data: {e}"))?;

    // Try gateway API first (proxies to configured LLM provider)
    if let Some(result) = try_gateway_stt(&state, &audio_data, &mime_type).await {
        return Ok(result);
    }

    // Fall back to local whisper CLI
    try_local_whisper(&audio_data, &mime_type).await
}

async fn try_gateway_stt(
    state: &tauri::State<'_, crate::AppData>,
    audio_data: &[u8],
    mime_type: &str,
) -> Option<SttResult> {
    let watch = state.startup_watch.clone();
    let gateway_info = match &*watch.borrow() {
        crate::GatewayStartupState::Running { info } => info.clone(),
        _ => return None,
    };

    let ext = mime_ext(mime_type);
    let filename = format!("recording.{ext}");

    let stt_url = format!(
        "http://127.0.0.1:{}/v1/audio/transcriptions",
        gateway_info.port
    );

    let part = reqwest::multipart::Part::bytes(audio_data.to_vec())
        .file_name(filename)
        .mime_str(mime_type)
        .ok()?;

    let form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", "whisper-1");

    let client = reqwest::Client::new();
    let resp = client.post(&stt_url).multipart(form).send().await.ok()?;

    if !resp.status().is_success() {
        tracing::debug!(
            status = %resp.status(),
            "gateway STT returned error, falling back to local whisper"
        );
        return None;
    }

    #[derive(Deserialize)]
    struct WhisperResponse {
        text: String,
        language: Option<String>,
    }

    let r: WhisperResponse = resp.json().await.ok()?;
    Some(SttResult {
        text: r.text,
        language: r.language,
    })
}

async fn try_local_whisper(audio_data: &[u8], mime_type: &str) -> Result<SttResult, String> {
    let ext = mime_ext(mime_type);
    let tmp_path = std::env::temp_dir().join(format!("xiaolin-voice-{}.{ext}", std::process::id()));

    std::fs::write(&tmp_path, audio_data)
        .map_err(|e| format!("failed to write temp audio file: {e}"))?;

    let path = tmp_path.clone();
    let result = run_whisper_cli(&path).await;

    let _ = std::fs::remove_file(&tmp_path);
    result
}

async fn run_whisper_cli(audio_path: &std::path::Path) -> Result<SttResult, String> {
    const WHISPER_TIMEOUT: Duration = Duration::from_secs(300);
    let output_dir = audio_path.parent().unwrap_or(std::path::Path::new("/tmp"));

    // Try whisper (pip install openai-whisper) or whisper.cpp CLI
    for cmd in &["whisper", "whisper-cpp"] {
        let child = match TokioCommand::new(cmd)
            .arg(audio_path)
            .arg("--language")
            .arg("zh")
            .arg("--model")
            .arg("base")
            .arg("--output_format")
            .arg("txt")
            .arg("--output_dir")
            .arg(output_dir)
            .spawn()
        {
            Ok(c) => c,
            Err(_) => {
                tracing::debug!(cmd, "whisper CLI not found");
                continue;
            }
        };
        let pid = child.id();

        let output = match tokio::time::timeout(WHISPER_TIMEOUT, child.wait_with_output()).await {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => {
                tracing::debug!(cmd, error = %e, "whisper CLI wait failed");
                continue;
            }
            Err(_) => {
                kill_whisper_child(pid);
                tracing::warn!(cmd, "whisper CLI timed out after 300s");
                continue;
            }
        };

        if output.status.success() {
            let txt_path = audio_path.with_extension("txt");
            let text = if txt_path.exists() {
                let t = std::fs::read_to_string(&txt_path).unwrap_or_default();
                let _ = std::fs::remove_file(&txt_path);
                t.trim().to_string()
            } else {
                String::from_utf8_lossy(&output.stdout).trim().to_string()
            };

            if !text.is_empty() {
                return Ok(SttResult {
                    text,
                    language: Some("zh".into()),
                });
            }
        } else {
            tracing::debug!(
                cmd,
                status = %output.status,
                stderr = %String::from_utf8_lossy(&output.stderr),
                "whisper CLI exited with error"
            );
        }
    }

    Err(
        "语音转文字不可用。请配置 LLM 提供商 API Key，或安装 whisper CLI (pip install openai-whisper)"
            .into(),
    )
}

fn kill_whisper_child(pid: Option<u32>) {
    let Some(pid) = pid else {
        return;
    };
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .arg(pid.to_string())
            .status();
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F", "/T"])
            .status();
    }
}

fn mime_ext(mime_type: &str) -> &str {
    match mime_type {
        "audio/webm" | "audio/webm;codecs=opus" => "webm",
        "audio/ogg" | "audio/ogg;codecs=opus" => "ogg",
        "audio/wav" | "audio/wave" => "wav",
        "audio/mp4" => "m4a",
        _ => "webm",
    }
}

#[tauri::command]
pub fn stt_available(state: tauri::State<'_, crate::AppData>) -> bool {
    // STT is always "available" — we have local whisper fallback
    // The actual availability is checked at transcription time
    let gateway_up = matches!(
        &*state.startup_watch.borrow(),
        crate::GatewayStartupState::Running { .. }
    );
    if gateway_up {
        return true;
    }

    // Check if whisper CLI is installed
    std::process::Command::new("whisper")
        .arg("--help")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
