use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct SttResult {
    pub text: String,
    pub language: Option<String>,
}

/// Transcribe audio data using the gateway's configured STT endpoint.
/// The audio should be base64-encoded WAV/WebM data from the frontend MediaRecorder.
#[tauri::command]
pub async fn transcribe_audio(
    audio_base64: String,
    mime_type: String,
    state: tauri::State<'_, crate::AppData>,
) -> Result<SttResult, String> {
    let watch = state.startup_watch.clone();
    let gateway_info = match &*watch.borrow() {
        crate::GatewayStartupState::Running { info } => info.clone(),
        _ => return Err("Gateway is not running yet".to_string()),
    };

    let audio_data = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        &audio_base64,
    )
    .map_err(|e| format!("Invalid base64 audio data: {e}"))?;

    let ext = match mime_type.as_str() {
        "audio/webm" | "audio/webm;codecs=opus" => "webm",
        "audio/ogg" | "audio/ogg;codecs=opus" => "ogg",
        "audio/wav" | "audio/wave" => "wav",
        "audio/mp4" => "m4a",
        _ => "webm",
    };
    let filename = format!("recording.{ext}");

    let stt_url = format!(
        "http://127.0.0.1:{}/v1/audio/transcriptions",
        gateway_info.port
    );

    let part = reqwest::multipart::Part::bytes(audio_data)
        .file_name(filename)
        .mime_str(&mime_type)
        .map_err(|e| format!("Failed to create multipart: {e}"))?;

    let form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", "whisper-1");

    let client = reqwest::Client::new();
    let resp = client
        .post(&stt_url)
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("STT request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("STT API returned {status}: {body}"));
    }

    #[derive(Deserialize)]
    struct WhisperResponse {
        text: String,
        language: Option<String>,
    }

    let whisper_resp: WhisperResponse = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse STT response: {e}"))?;

    Ok(SttResult {
        text: whisper_resp.text,
        language: whisper_resp.language,
    })
}

/// Check if STT is available (i.e., gateway is running).
#[tauri::command]
pub fn stt_available(state: tauri::State<'_, crate::AppData>) -> bool {
    matches!(&*state.startup_watch.borrow(), crate::GatewayStartupState::Running { .. })
}
