## Overview

Voice input via push-to-talk for hands-free assistant interaction.

## Requirements

- Push-to-talk activation via configurable shortcut
- Uses system STT API (Phase 1) or local Whisper (Phase 2)
- Transcribed text sent to current conversation
- Visual indicator during recording (microphone icon + waveform)
- Graceful degradation: disabled UI when STT unavailable
- Audio stays local when using system STT or Whisper
