# Vox Daemon

Linux-native meeting transcription and summarization service.

Vox Daemon is a background service that captures video call audio via PipeWire, transcribes it with Whisper (GPU-accelerated), performs speaker diarization, and generates AI-powered post-call summaries.

## Features

- **PipeWire-native** audio capture — no PulseAudio compatibility layer
- **Local Whisper transcription** with CUDA and ROCm GPU acceleration
- **Speaker separation** — mic audio labeled "You", remote audio labeled "Remote"
- **AI summaries** via Ollama, OpenAI-compatible APIs, or any local LLM server
- **System tray** control with start/stop/status
- **Desktop notifications** for recording, transcription, and summary events
- **Settings window** (iced) and transcript browser with full-text search
- **Markdown and JSON export** for all sessions
- **Privacy-first** — everything runs locally by default

## Requirements

- Linux with PipeWire (Wayland recommended)
- Rust 1.85.0+ (for building from source)
- Optional: NVIDIA GPU (CUDA) or AMD GPU (ROCm) for accelerated transcription
- Optional: [Ollama](https://ollama.ai) for local AI summarization

### System Dependencies

```bash
# Arch Linux
sudo pacman -S pipewire dbus

# Ubuntu/Debian
sudo apt install libpipewire-0.3-dev libdbus-1-dev pkg-config

# Fedora
sudo dnf install pipewire-devel dbus-devel pkg-config
```

## Installation

### From source

```bash
git clone https://github.com/user/vox-daemon.git
cd vox-daemon
cargo build --release
sudo install -Dm755 target/release/vox-daemon /usr/local/bin/vox-daemon
```

### Arch Linux (AUR)

```bash
yay -S vox-daemon
```

### systemd user service

```bash
# Copy the service file
mkdir -p ~/.config/systemd/user
cp dist/systemd/vox-daemon.service ~/.config/systemd/user/

# Enable and start
systemctl --user enable --now vox-daemon
```

## Quick Start

```bash
# Initialize configuration
vox-daemon init-config

# List available audio sources
vox-daemon list-sources

# Record a session (Ctrl+C to stop)
vox-daemon record

# List past sessions
vox-daemon list-sessions

# Show the active configuration
vox-daemon show-config

# Export a session to Markdown
vox-daemon export <SESSION_ID> > meeting.md

# Summarize a session with your configured LLM
vox-daemon summarize <SESSION_ID>

# Re-transcribe a session using its retained audio and current settings
vox-daemon reprocess <SESSION_ID>

# Run the long-lived background service (tray + daemon)
vox-daemon start

# Launch the settings / transcript browser GUI (requires the `ui` feature)
vox-daemon gui --page browser
```

## Configuration

Configuration is stored at `$XDG_CONFIG_HOME/vox-daemon/config.toml` (typically `~/.config/vox-daemon/config.toml`).

```toml
[audio]
mic_source = "auto"
app_source = "auto"

[transcription]
model = "small"          # tiny, base, small, medium, large
language = "en"          # or "auto"
gpu_backend = "auto"     # auto, cuda, rocm, cpu
model_path = ""          # custom whisper model path, empty = cache dir

# Diarization
diarization_mode = "none"      # none | embedding (requires `diarize` feature)
diarize_model_path = ""         # empty = auto-download ECAPA-TDNN ONNX model
diarize_threshold = 0.5         # cosine distance; lower = more clusters
enrollment_seconds = 5.0        # seconds of mic-only audio to enroll "You"

# Decoding quality knobs
decoding_strategy = "beam_search"  # greedy | beam_search
beam_size = 5
best_of = 5
initial_prompt = ""      # seed names / jargon to guide Whisper
temperature = 0.0
temperature_inc = 0.2
entropy_thold = 2.4
logprob_thold = -1.0
no_speech_thold = 0.6

[summarization]
auto_summarize = true
backend = "builtin"      # builtin, ollama, openai_compatible
ollama_url = "http://localhost:11434"
ollama_model = "qwen2.5:1.5b"
api_url = ""
api_key = ""
api_model = ""

[storage]
data_dir = ""            # empty = XDG default
retain_audio = false
export_format = "markdown"

[notifications]
enabled = true
on_record_start = true
on_record_stop = true
on_transcript_ready = true
on_summary_ready = true
```

## Feature Flags

Native dependencies are gated behind optional feature flags so the project compiles without them:

| Feature | Enables | Requires |
|---------|---------|----------|
| `pw` | Real PipeWire audio capture | `libpipewire-0.3-dev` |
| `whisper` | Whisper transcription | `whisper-rs` (builds whisper.cpp) |
| `cuda` | NVIDIA GPU acceleration | CUDA toolkit |
| `hipblas` | AMD GPU acceleration | ROCm/hipBLAS |
| `diarize` | ONNX speaker-embedding diarization | `ort` (ONNX Runtime), auto-downloads an ECAPA-TDNN model |
| `gtk` | System tray icon | `libgtk-3-dev`, `libayatana-appindicator3-dev` |
| `ui` | iced settings window + transcript browser | GPU-capable display server |

Build with all features:
```bash
cargo build --release --features "pw,whisper,cuda,diarize,gtk,ui"
```

## Architecture

```
vox-daemon/
├── crates/
│   ├── vox-core/        # Shared types, config, XDG paths
│   ├── vox-capture/     # PipeWire audio capture
│   ├── vox-transcribe/  # Whisper speech-to-text
│   ├── vox-diarize/     # Speaker diarization (ONNX embeddings + clustering)
│   ├── vox-summarize/   # LLM summarization client
│   ├── vox-storage/     # JSON session storage, Markdown export
│   ├── vox-gui/         # iced settings window + transcript browser
│   ├── vox-tray/        # System tray icon + popup menu
│   └── vox-notify/      # Desktop notification wrapper
└── vox-daemon/          # Binary crate — daemon entrypoint
```

Each crate exposes a trait-based API for its primary functionality, enabling testing with mocks and future swappable implementations.

## Development

```bash
# Run tests
cargo test

# Run with verbose logging
cargo run -- -vv record

# Check formatting and lints
cargo fmt --check
cargo clippy -- -D warnings
```

## License

MIT License. See [LICENSE](LICENSE) for details.
