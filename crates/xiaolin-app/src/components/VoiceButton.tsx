import { useState, useRef, useCallback, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";

interface VoiceButtonProps {
  onTranscription: (text: string) => void;
  disabled?: boolean;
  className?: string;
}

type RecordingState = "idle" | "recording" | "transcribing";

export function VoiceButton({ onTranscription, disabled, className }: VoiceButtonProps) {
  const [state, setState] = useState<RecordingState>("idle");
  const [sttAvailable, setSttAvailable] = useState(true);
  const mediaRecorderRef = useRef<MediaRecorder | null>(null);
  const chunksRef = useRef<Blob[]>([]);

  useEffect(() => {
    invoke<boolean>("stt_available").then(setSttAvailable).catch(() => setSttAvailable(false));
  }, []);

  const startRecording = useCallback(async () => {
    if (state !== "idle" || disabled || !sttAvailable) return;
    try {
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
        } catch (err) {
          console.error("Transcription failed:", err);
        } finally {
          setState("idle");
        }
      };

      mediaRecorderRef.current = recorder;
      recorder.start(250);
      setState("recording");
    } catch (err) {
      console.error("Microphone access denied:", err);
      setSttAvailable(false);
    }
  }, [state, disabled, sttAvailable, onTranscription]);

  const stopRecording = useCallback(() => {
    if (mediaRecorderRef.current?.state === "recording") {
      mediaRecorderRef.current.stop();
    }
  }, []);

  if (!sttAvailable) {
    return (
      <button
        className={`voice-btn voice-btn-disabled ${className ?? ""}`}
        disabled
        title="语音输入不可用"
      >
        <MicOffIcon />
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
      title={state === "recording" ? "松开停止" : state === "transcribing" ? "转录中…" : "按住说话"}
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
