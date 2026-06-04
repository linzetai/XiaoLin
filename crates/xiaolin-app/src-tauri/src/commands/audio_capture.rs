use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

pub struct AudioCaptureState {
    recording: Arc<AtomicBool>,
    samples: Arc<Mutex<Vec<f32>>>,
    sample_rate: Arc<Mutex<u32>>,
    channels: Arc<Mutex<u16>>,
}

impl Default for AudioCaptureState {
    fn default() -> Self {
        Self {
            recording: Arc::new(AtomicBool::new(false)),
            samples: Arc::new(Mutex::new(Vec::new())),
            sample_rate: Arc::new(Mutex::new(44100)),
            channels: Arc::new(Mutex::new(1)),
        }
    }
}

impl AudioCaptureState {
    pub fn new() -> Self {
        Self::default()
    }
}

#[tauri::command]
pub fn native_audio_available() -> bool {
    cpal::default_host().default_input_device().is_some()
}

#[tauri::command]
pub fn start_native_recording(
    state: tauri::State<'_, AudioCaptureState>,
) -> Result<(), String> {
    if state.recording.load(Ordering::SeqCst) {
        return Err("already recording".into());
    }

    if let Ok(mut s) = state.samples.lock() {
        s.clear();
    }

    let recording = state.recording.clone();
    let samples = state.samples.clone();
    let sr_out = state.sample_rate.clone();
    let ch_out = state.channels.clone();

    recording.store(true, Ordering::SeqCst);

    std::thread::spawn(move || {
        let host = cpal::default_host();
        let device = match host.default_input_device() {
            Some(d) => d,
            None => {
                recording.store(false, Ordering::SeqCst);
                return;
            }
        };

        let config = match device.default_input_config() {
            Ok(c) => c,
            Err(_) => {
                recording.store(false, Ordering::SeqCst);
                return;
            }
        };

        let sr = config.sample_rate().0;
        let ch = config.channels();
        if let Ok(mut v) = sr_out.lock() {
            *v = sr;
        }
        if let Ok(mut v) = ch_out.lock() {
            *v = ch;
        }

        let samples_clone = samples.clone();
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &config.into(),
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if let Ok(mut buf) = samples_clone.lock() {
                        buf.extend_from_slice(data);
                    }
                },
                |err| tracing::error!("audio capture error: {err}"),
                None,
            ),
            cpal::SampleFormat::I16 => {
                let sc = samples.clone();
                device.build_input_stream(
                    &config.into(),
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        if let Ok(mut buf) = sc.lock() {
                            buf.extend(data.iter().map(|&s| s as f32 / i16::MAX as f32));
                        }
                    },
                    |err| tracing::error!("audio capture error: {err}"),
                    None,
                )
            }
            other => {
                tracing::error!("unsupported sample format: {other:?}");
                recording.store(false, Ordering::SeqCst);
                return;
            }
        };

        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("failed to build audio stream: {e}");
                recording.store(false, Ordering::SeqCst);
                return;
            }
        };

        if let Err(e) = stream.play() {
            tracing::error!("failed to start audio stream: {e}");
            recording.store(false, Ordering::SeqCst);
            return;
        }

        while recording.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        drop(stream);
    });

    Ok(())
}

#[tauri::command]
pub fn stop_native_recording(
    state: tauri::State<'_, AudioCaptureState>,
) -> Result<String, String> {
    if !state.recording.load(Ordering::SeqCst) {
        return Err("not recording".into());
    }
    state.recording.store(false, Ordering::SeqCst);

    std::thread::sleep(std::time::Duration::from_millis(100));

    let samples = state
        .samples
        .lock()
        .map_err(|_| "samples lock poisoned")?;
    let sample_rate = *state
        .sample_rate
        .lock()
        .map_err(|_| "lock poisoned")?;
    let channels = *state
        .channels
        .lock()
        .map_err(|_| "lock poisoned")?;

    if samples.is_empty() {
        return Err("no audio captured".into());
    }

    let wav_data = encode_wav(&samples, sample_rate, channels)?;
    let b64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &wav_data,
    );
    Ok(b64)
}

fn encode_wav(samples: &[f32], sample_rate: u32, channels: u16) -> Result<Vec<u8>, String> {
    let mut buf = std::io::Cursor::new(Vec::new());
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer =
        hound::WavWriter::new(&mut buf, spec).map_err(|e| format!("WAV write error: {e}"))?;

    for &s in samples {
        let val = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer
            .write_sample(val)
            .map_err(|e| format!("WAV sample error: {e}"))?;
    }
    writer
        .finalize()
        .map_err(|e| format!("WAV finalize error: {e}"))?;

    Ok(buf.into_inner())
}
