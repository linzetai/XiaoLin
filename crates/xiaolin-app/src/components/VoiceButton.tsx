import { useState, useRef, useCallback, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";

interface VoiceButtonProps {
  onTranscription: (text: string) => void;
  disabled?: boolean;
  className?: string;
}

type RecordingState = "idle" | "recording" | "transcribing";
type CaptureBackend = "webrtc" | "native" | null;

export function VoiceButton({ onTranscription, disabled, className }: VoiceButtonProps) {
  const [state, setState] = useState<RecordingState>("idle");
  const [sttAvailable, setSttAvailable] = useState(true);
  const [micError, setMicError] = useState<string | null>(null);
  const [backend, setBackend] = useState<CaptureBackend>(null);
  const mediaRecorderRef = useRef<MediaRecorder | null>(null);
  const chunksRef = useRef<Blob[]>([]);

  useEffect(() => {
    invoke<boolean>("stt_available").then(setSttAvailable).catch(() => setSttAvailable(false));

    (async () => {
      try {
        const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
        stream.getTracks().forEach((t) => t.stop());
        setBackend("webrtc");
      } catch {
        const nativeOk = await invoke<boolean>("native_audio_available").catch(() => false);
        if (nativeOk) {
          setBackend("native");
        } else {
          setMicError("无可用的音频输入设备");
        }
      }
    })();
  }, []);

  const startWebRtcRecording = useCallback(async () => {
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    const mimeType = MediaRecorder.isTypeSupported("audio/webm;codecs=opus")
      ? "audio/webm;codecs=opus"
      : "audio/webm";

    const recorder = new MediaRecorder(stream, { mimeType });
    chunksRef.current = [];

    recorder.ondataavailable = (e) => {
      if (e.data.size > 0) chunksRef.current.push(e.data);
    };

    recorder.onstop = async () => {
      stream.getTracks().forEach((t) => t.stop());
      const blob = new Blob(chunksRef.current, { type: mimeType });
      if (blob.size < 1000) {
        setState("idle");
        return;
      }
      setState("transcribing");
      try {
        const buf = await blob.arrayBuffer();
        const base64 = btoa(
          new Uint8Array(buf).reduce((s, b) => s + String.fromCharCode(b), ""),
        );
        const result = await invoke<{ text: string }>("transcribe_audio", {
          audioBase64: base64,
          mimeType,
        });
        if (result.text.trim()) {
          onTranscription(result.text.trim());
        }
      } catch (err: unknown) {
        const msg = err instanceof Error ? err.message : String(err);
        console.error("Transcription failed:", msg);
        setMicError(msg);
      } finally {
        setState("idle");
      }
    };

    mediaRecorderRef.current = recorder;
    recorder.start(250);
    setState("recording");
  }, [onTranscription]);

  const startNativeRecording = useCallback(async () => {
    await invoke("start_native_recording");
    setState("recording");
  }, []);

  const stopNativeRecording = useCallback(async () => {
    setState("transcribing");
    try {
      const wavBase64 = await invoke<string>("stop_native_recording");
      const result = await invoke<{ text: string }>("transcribe_audio", {
        audioBase64: wavBase64,
        mimeType: "audio/wav",
      });
      if (result.text.trim()) {
        onTranscription(result.text.trim());
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      console.error("Native transcription failed:", msg);
      setMicError(msg);
    } finally {
      setState("idle");
    }
  }, [onTranscription]);

  const startRecording = useCallback(async () => {
    if (state !== "idle" || disabled || !sttAvailable || !backend) return;
    try {
      if (backend === "webrtc") {
        await startWebRtcRecording();
      } else {
        await startNativeRecording();
      }
    } catch (err) {
      console.error("Recording failed:", err);
      setMicError(err instanceof Error ? err.message : "录音失败");
    }
  }, [state, disabled, sttAvailable, backend, startWebRtcRecording, startNativeRecording]);

  const stopRecording = useCallback(() => {
    if (state !== "recording") return;
    if (backend === "webrtc") {
      if (mediaRecorderRef.current?.state === "recording") {
        mediaRecorderRef.current.stop();
      }
    } else {
      stopNativeRecording();
    }
  }, [state, backend, stopNativeRecording]);

  if (!sttAvailable) {
    return (
      <button
        className={`voice-btn voice-btn-disabled ${className ?? ""}`}
        disabled
        title="语音输入不可用（STT 服务未就绪）"
      >
        <MicOffIcon />
      </button>
    );
  }

  if (micError) {
    return (
      <button
        className={`voice-btn voice-btn-disabled ${className ?? ""}`}
        onClick={() => setMicError(null)}
        title={`${micError}（点击重试）`}
      >
        <MicOffIcon />
      </button>
    );
  }

  if (!backend) {
    return (
      <button
        className={`voice-btn voice-btn-disabled ${className ?? ""}`}
        disabled
        title="检测音频设备中…"
      >
        <SpinnerIcon />
      </button>
    );
  }

  return (
    <button
      className={`voice-btn ${state !== "idle" ? "voice-btn-active" : ""} ${className ?? ""}`}
      onMouseDown={startRecording}
      onMouseUp={stopRecording}
      onMouseLeave={stopRecording}
      disabled={disabled || state === "transcribing"}
      title={
        state === "recording"
          ? "松开停止"
          : state === "transcribing"
            ? "转录中…"
            : `按住说话 (${backend === "native" ? "原生录音" : "WebRTC"})`
      }
    >
      {state === "transcribing" ? <SpinnerIcon /> : state === "recording" ? <WaveIcon /> : <MicIcon />}
    </button>
  );
}

function MicIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z" />
      <path d="M19 10v2a7 7 0 0 1-14 0v-2" />
      <line x1="12" x2="12" y1="19" y2="22" />
    </svg>
  );
}

function MicOffIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <line x1="2" x2="22" y1="2" y2="22" />
      <path d="M18.89 13.23A7.12 7.12 0 0 0 19 12v-2" />
      <path d="M5 10v2a7 7 0 0 0 12 5.29" />
      <path d="M15 9.34V5a3 3 0 0 0-5.94-.6" />
      <path d="M9 9v3a3 3 0 0 0 5.12 2.12" />
      <line x1="12" x2="12" y1="19" y2="22" />
    </svg>
  );
}

function WaveIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="voice-wave">
      <line x1="4" x2="4" y1="8" y2="16" />
      <line x1="8" x2="8" y1="5" y2="19" />
      <line x1="12" x2="12" y1="3" y2="21" />
      <line x1="16" x2="16" y1="5" y2="19" />
      <line x1="20" x2="20" y1="8" y2="16" />
    </svg>
  );
}

function SpinnerIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" className="voice-spinner">
      <path d="M21 12a9 9 0 1 1-6.219-8.56" />
    </svg>
  );
}
