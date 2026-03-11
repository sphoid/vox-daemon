# Product Requirements Document: Vox Daemon

## Linux-Native Meeting Transcription & Summarization Service

**Version:** 1.0.0-draft
**Date:** March 10, 2026
**Author:** PRD Assistant
**Project Type:** Personal/Hobby → Open Source
**Target Users:** Initially the developer; eventually the Linux desktop community

---

## 1. Executive Summary

Vox Daemon is a Linux-native background service that automatically captures, transcribes, and summarizes video call audio. It operates as a system daemon that hooks into PipeWire to capture both microphone input and application audio output, providing full-conversation transcription with speaker diarization and AI-generated post-call summaries.

The project targets a clear gap in the Linux ecosystem — existing tools like Krisp and Otter.ai are Windows/macOS-focused, and none provide a first-class, desktop-environment-agnostic experience for Linux users running PipeWire on Wayland.

### Key Differentiators

- **Linux-exclusive, PipeWire-native** — no PulseAudio compatibility layer, no Electron wrappers
- **Desktop-environment agnostic** — works across GNOME, KDE Plasma, Sway, Hyprland, and others
- **No meeting bot required** — captures audio at the system level without injecting a participant into the call
- **Privacy-first** — built-in local transcription via Whisper with optional external LLM integration
- **Wayland-first** — designed for modern Linux display servers

---

## 2. Target Audience

### Primary (v1)

The developer themselves — a Linux power user running a modern desktop with PipeWire, who participates in regular video calls and wants automated transcription and summaries without relying on platform-specific tools or SaaS.

### Secondary (Post v1 — Open Source Release)

- Linux desktop users on GNOME, KDE, or tiling WM setups who use Zoom, Google Meet, Teams, or similar
- Privacy-conscious professionals who want local-only transcription
- Developers and sysadmins who prefer CLI/daemon-based tooling over GUI-heavy apps

---

## 3. Project Scope & Scale

| Attribute | Value |
|-----------|-------|
| Initial user base | 1 (developer) |
| Target post-release | 10s–100s (open source community) |
| Project type | Personal hobby → open source |
| Security sensitivity | Low — no auth, no cloud storage, local data only |
| Monetization | None |

This scoping means we prioritize simplicity, correctness, and clean architecture over enterprise concerns like multi-tenancy, authentication, or horizontal scaling.

---

## 4. Core Features & Functionality

### 4.1 Audio Capture (PipeWire Integration)

**Description:** Capture both the user's microphone output and the remote participants' audio from a video conferencing application via PipeWire.

**Requirements:**
- Connect to the PipeWire daemon and enumerate active audio streams
- Capture the user's microphone stream (source output)
- Capture the application audio output stream (sink input from the conferencing app)
- Keep streams separated to support speaker diarization (user vs. remote)
- Resample audio to 16kHz mono PCM (f32) as required by Whisper
- Buffer audio in memory during active recording sessions
- Handle PipeWire stream lifecycle events (stream creation, destruction, format changes)

**Acceptance Criteria:**
- Daemon can list available PipeWire audio sources
- User can select which audio streams to capture via the settings UI
- Audio is captured without audible artifacts or gaps
- Captured audio is correctly formatted for downstream transcription

**Technical Considerations:**
- PipeWire streams may change mid-call (e.g., user switches audio devices)
- Must handle PipeWire node hot-plug/unplug gracefully
- The two audio streams (mic + app) should be captured with synchronized timestamps

### 4.2 Speech-to-Text Transcription

**Description:** Convert captured audio to text using OpenAI's Whisper model, running locally with GPU acceleration.

**Requirements:**
- Integrate whisper.cpp via Rust bindings for local inference
- Support CUDA (NVIDIA) and ROCm/hipBLAS (AMD) GPU acceleration
- Support CPU-only fallback
- Allow model selection (tiny, base, small, medium, large) via configuration
- Process audio in chunks during recording (near-real-time internal processing)
- Produce timestamped transcript segments
- Support configurable language selection (default: English, with auto-detect option)

**Acceptance Criteria:**
- Transcription completes within a reasonable time after recording stops (< 2x audio duration for large model on GPU)
- Output includes per-segment timestamps
- GPU acceleration produces measurably faster results than CPU-only

**Technical Considerations:**
- Whisper models range from ~75MB (tiny) to ~2.9GB (large-v3); the daemon should download models on first use or allow the user to provide a path
- GPU memory usage varies significantly by model size
- whisper.cpp's Rust bindings support feature flags for `cuda` and `hipblas`

### 4.3 Speaker Diarization

**Description:** Identify and label different speakers in the transcript, distinguishing at minimum between the local user and remote participants.

**Requirements:**
- Use the separated audio streams (mic vs. app audio) as the primary diarization signal
- Label transcript segments with speaker identifiers (e.g., "You", "Speaker 1", "Speaker 2")
- Allow the user to assign friendly names to speakers post-call
- For multi-participant remote audio (single mixed stream), apply clustering-based diarization to differentiate remote speakers

**Acceptance Criteria:**
- In a two-person call, the local user and remote participant are correctly separated > 90% of the time
- Speaker labels are consistent throughout the transcript
- Friendly name assignment persists in the stored JSON

**Technical Considerations:**
- Primary diarization (you vs. them) is straightforward since we have separate PipeWire streams
- Multi-speaker diarization on the remote stream is significantly harder; pyannote.audio is the state-of-the-art but is Python-based
- For v1, multi-remote-speaker diarization can be deferred — label all remote audio as "Remote" and add granular diarization in a future phase
- If pyannote is needed later, it can be invoked as a subprocess or via an embedded Python runtime

### 4.4 AI-Powered Summarization

**Description:** Generate structured summaries of completed calls using an LLM.

**Requirements:**
- Support three LLM backends (configurable):
  1. **Built-in local inference** — integrate a small model via llama.cpp bindings or GGUF runtime
  2. **Local LLM server** — connect to Ollama, llama.cpp server, or any OpenAI-compatible local API
  3. **External cloud API** — support OpenAI, Anthropic, and other OpenAI-compatible API endpoints
- Generate summaries containing:
  - Brief overall summary (2–3 sentences)
  - Key discussion points (bulleted)
  - Action items with assigned owners (when identifiable)
  - Decisions made
- Summarization trigger is configurable: automatic on recording stop, or manual via UI
- Summary is appended to the transcript's JSON metadata

**Acceptance Criteria:**
- Summary is generated and stored alongside the transcript within 60 seconds of triggering (dependent on LLM backend speed)
- Summary structure is consistent and parseable
- User can re-generate a summary with a different backend or prompt

**Technical Considerations:**
- Built-in model should be small enough to run on modest hardware (e.g., Phi-3 mini, Qwen 2.5 0.5B/1.5B in GGUF format)
- Ollama's REST API is the simplest integration path for local LLM servers
- For cloud APIs, use a generic OpenAI-compatible HTTP client with configurable base URL and API key
- Token limits vary by model; transcript may need chunking for long calls

### 4.5 System Tray Interface

**Description:** A persistent system tray icon with a quick-access popup menu and a full settings window.

**Requirements:**

**Tray Icon & Popup Menu:**
- Display a tray icon indicating daemon status (idle, recording, processing)
- Quick-access controls: Start Recording, Stop Recording, Pause Recording
- Visual indicator when actively recording (icon change or animation)
- Quick link to open the most recent transcript
- Option to open the full settings window

**Settings Window:**
- Audio source selection (PipeWire stream picker)
- Transcription settings: model selection, language, GPU preference
- Summarization settings: backend selection (built-in/local/cloud), API endpoint configuration, auto vs. manual trigger
- Storage settings: output directory, export format preferences
- Notification preferences
- About / version info

**Acceptance Criteria:**
- Tray icon appears in GNOME, KDE Plasma, and Sway system trays
- All settings changes are persisted to disk immediately
- Recording can be started and stopped entirely from the tray menu
- Settings window renders correctly on Wayland

### 4.6 Data Storage & Export

**Description:** Store transcripts and summaries locally in a structured format, following XDG standards.

**Requirements:**
- Store session data as JSON files containing:
  - Session metadata (date, time, duration, participants)
  - Full transcript with timestamps and speaker labels
  - Summary (if generated)
  - Audio source information
  - Configuration used for transcription
- Follow XDG Base Directory Specification:
  - Config: `$XDG_CONFIG_HOME/vox-daemon/` (default: `~/.config/vox-daemon/`)
  - Data: `$XDG_DATA_HOME/vox-daemon/` (default: `~/.local/share/vox-daemon/`)
  - Cache (models, temp audio): `$XDG_CACHE_HOME/vox-daemon/` (default: `~/.cache/vox-daemon/`)
- Allow configurable override of the data directory
- Export transcripts to Markdown format with speaker labels and timestamps
- Optionally retain raw audio recordings (configurable, off by default to save disk space)

**Acceptance Criteria:**
- JSON files are well-formed and contain all required metadata
- Markdown export produces clean, readable documents
- Changing the data directory in settings migrates or repoints correctly
- XDG paths are resolved correctly when environment variables are set

### 4.7 Transcript Browser UI

**Description:** A simple interface for viewing, searching, and exporting past transcripts.

**Requirements:**
- List past sessions with date, duration, and summary preview
- Full transcript viewer with speaker-color-coded text and timestamps
- Text search across all transcripts (full-text)
- Export individual sessions to Markdown
- Delete sessions
- Ability to edit speaker friendly names
- Accessible from the system tray menu or as a standalone window

**Acceptance Criteria:**
- Transcript list loads within 1 second for up to 100 sessions
- Search returns results across all stored transcripts
- Markdown export matches the internal JSON content

### 4.8 Desktop Notifications

**Description:** Notify the user of key events via the standard Linux desktop notification system.

**Requirements:**
- Notify when: recording starts, recording stops, transcription completes, summary is generated
- Notifications are clickable — clicking opens the relevant transcript
- Respect system Do Not Disturb / notification suppression settings
- Notifications follow the XDG Desktop Notification Specification

**Acceptance Criteria:**
- Notifications appear correctly on GNOME, KDE Plasma, and Sway
- Notification content is concise and informative
- Clicking a notification opens the transcript in the browser UI

---

## 5. Technical Stack Recommendation

### 5.1 Language: Rust

**Rationale:** Rust is the recommended language for this project based on the following factors:

- **PipeWire bindings** — `pipewire-rs` provides mature, actively maintained Rust bindings for PipeWire's C API, with the PipeWire team actively investing in a native Rust protocol implementation (`pipewire-native`)
- **Whisper integration** — `whisper-rs` provides excellent Rust bindings to whisper.cpp with feature flags for both CUDA and ROCm/hipBLAS, the two GPU backends required
- **System tray support** — the `tray-icon` crate (by the Tauri team) provides cross-platform system tray functionality with Linux support via libappindicator/libayatana-appindicator
- **GUI ecosystem** — `iced` is a mature, cross-platform GUI framework used in production by System76's COSMIC desktop environment, making it well-tested on Linux/Wayland
- **Performance** — zero-cost abstractions and no garbage collector overhead are ideal for a long-running daemon processing real-time audio
- **Desktop notifications** — `notify-rust` provides XDG-compliant desktop notifications with pure Rust D-Bus implementation
- **Safety** — memory safety without GC is critical for a daemon that will run for extended periods

### 5.2 Architecture Overview

```
┌─────────────────────────────────────────────────────┐
│                    Vox Daemon                         │
│                                                       │
│  ┌──────────┐   ┌──────────────┐   ┌──────────────┐ │
│  │ PipeWire │──▶│ Audio Buffer  │──▶│   Whisper     │ │
│  │ Capture  │   │ (Ring Buffer) │   │ Transcriber   │ │
│  └──────────┘   └──────────────┘   └──────┬───────┘ │
│                                           │          │
│  ┌──────────┐   ┌──────────────┐   ┌──────▼───────┐ │
│  │  Tray    │◀─▶│   Session    │◀──│  Diarizer    │ │
│  │  Icon    │   │   Manager    │   │              │ │
│  └──────────┘   └──────┬───────┘   └──────────────┘ │
│                        │                              │
│  ┌──────────┐   ┌──────▼───────┐   ┌──────────────┐ │
│  │   GUI    │◀─▶│   Storage    │──▶│ Summarizer   │ │
│  │ (iced)   │   │   (JSON/XDG) │   │ (LLM Client) │ │
│  └──────────┘   └──────────────┘   └──────────────┘ │
│                                                       │
│  ┌──────────────────────────────────────────────────┐ │
│  │              Notification Service                 │ │
│  └──────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────┘
```

**Key Architectural Decisions:**

- **Modular crate structure** — separate crates for audio capture, transcription, diarization, summarization, GUI, and storage to allow independent testing and future extensibility
- **Async runtime** — use Tokio for async I/O (HTTP clients, file I/O), with PipeWire's own event loop running on a dedicated thread
- **Message passing** — use channels (`tokio::sync::mpsc` or `crossbeam-channel`) for communication between the PipeWire capture thread, processing pipeline, and UI thread
- **Trait-based abstractions** — define traits for `Transcriber`, `Summarizer`, and `Diarizer` to allow swappable implementations

### 5.3 Configuration Format

Use TOML for the configuration file (`$XDG_CONFIG_HOME/vox-daemon/config.toml`):

```toml
[audio]
# PipeWire source identifiers (populated via UI)
mic_source = "auto"
app_source = "auto"

[transcription]
model = "base"           # tiny | base | small | medium | large
language = "en"           # or "auto"
gpu_backend = "auto"      # auto | cuda | rocm | cpu
model_path = ""           # custom path, or empty to use cache dir

[summarization]
auto_summarize = true
backend = "builtin"       # builtin | ollama | openai_compatible
ollama_url = "http://localhost:11434"
ollama_model = "qwen2.5:1.5b"
api_url = ""
api_key = ""
api_model = ""

[storage]
data_dir = ""             # empty = XDG default
retain_audio = false
export_format = "markdown" # markdown | json

[notifications]
enabled = true
on_record_start = true
on_record_stop = true
on_transcript_ready = true
on_summary_ready = true
```

---

## 6. Dependencies and Versions

**Last verified:** March 10, 2026

### Core Rust Toolchain

| Dependency | Version | Notes |
|-----------|---------|-------|
| Rust (stable) | 1.94.0 | Latest stable as of March 2026 |
| Rust Edition | 2024 | Latest edition, stable since Rust 1.85.0 |
| Cargo | 1.94.0 | Ships with Rust |

### Audio Capture

| Crate | Version | Notes |
|-------|---------|-------|
| `pipewire` (pipewire-rs) | 0.9.2 | Stable Rust bindings for libpipewire C API |
| `pipewire-native` | — | Emerging pure-Rust PipeWire protocol implementation (watch for maturity; use `pipewire-rs` for v1) |

**System dependency:** libpipewire-0.3-dev (or equivalent for your distro)

### Transcription

| Crate | Version | Notes |
|-------|---------|-------|
| `whisper-rs` | 0.15.1 | Rust bindings for whisper.cpp; supports `cuda` and `hipblas` feature flags |
| `whisper-rs-sys` | 0.14.x | Low-level sys bindings (pulled in by whisper-rs) |

**System dependencies (optional, for GPU acceleration):**
- NVIDIA: CUDA Toolkit 12.x
- AMD: ROCm 6.x with hipBLAS

**Whisper models (GGML format):**
- tiny.en (~75MB), base.en (~142MB), small.en (~466MB), medium.en (~1.5GB), large-v3 (~2.9GB)
- Downloadable from Hugging Face (ggerganov/whisper.cpp model repository)

### GUI Framework

| Crate | Version | Notes |
|-------|---------|-------|
| `iced` | 0.14.0 | Cross-platform GUI library; Wayland support via wgpu backend; used by System76 COSMIC |
| `iced_aw` | 0.12.0+ | Additional widgets (check compatibility with iced 0.14) |

### System Tray

| Crate | Version | Notes |
|-------|---------|-------|
| `tray-icon` | 0.21.2 | By Tauri team; Linux support via libappindicator/libayatana-appindicator + GTK |
| `muda` | 0.17.x | Menu library (dependency of tray-icon) |

**System dependency:** libayatana-appindicator3-dev or libappindicator3-dev, libgtk-3-dev

### Desktop Notifications

| Crate | Version | Notes |
|-------|---------|-------|
| `notify-rust` | 4.11.6 | XDG desktop notifications; pure Rust D-Bus via zbus; works on GNOME, KDE, XFCE, Sway |

### LLM / Summarization

| Crate | Version | Notes |
|-------|---------|-------|
| `ollama-rs` | latest | Rust client for Ollama REST API |
| `genai` | 0.5.x | Multi-provider LLM client (OpenAI, Anthropic, Ollama, etc.) — alternative option |
| `llm` | 1.2.4 | Unified multi-backend LLM crate (OpenAI, Anthropic, Ollama, DeepSeek) — alternative option |
| `reqwest` | 0.12.x | HTTP client for custom API integration |

**Recommendation:** Use `reqwest` to build a simple OpenAI-compatible HTTP client. This avoids pulling in a heavy multi-provider crate and keeps the dependency tree lean. The Ollama API is OpenAI-compatible, so one client handles both local and cloud backends.

### Serialization & Configuration

| Crate | Version | Notes |
|-------|---------|-------|
| `serde` | 1.x | Serialization framework |
| `serde_json` | 1.x | JSON serialization |
| `toml` | 0.8.x | TOML configuration parsing |

### Async Runtime & Utilities

| Crate | Version | Notes |
|-------|---------|-------|
| `tokio` | 1.x | Async runtime (HTTP, file I/O, timers) |
| `crossbeam-channel` | 0.5.x | Multi-producer channel for PipeWire thread communication |
| `dirs` | 6.x | XDG base directory resolution |
| `chrono` | 0.4.x | Timestamp handling |
| `tracing` | 0.1.x | Structured logging |
| `tracing-subscriber` | 0.3.x | Log output formatting |
| `clap` | 4.x | CLI argument parsing (for daemon control) |

### Compatibility Notes

- `whisper-rs` 0.15.x requires a C/C++ toolchain and CMake for building whisper.cpp from source
- `pipewire-rs` 0.9.x requires libpipewire >= 0.3.x headers at compile time
- `iced` 0.14 uses wgpu for rendering; verify your GPU drivers support Vulkan or OpenGL 3.3+
- `tray-icon` on Linux requires a GTK event loop running on the tray thread; coordinate with iced's event loop
- The `cuda` and `hipblas` feature flags on `whisper-rs` are mutually exclusive; build separate binaries or use runtime detection with conditional compilation

---

## 7. Conceptual Data Model

### Session (stored as JSON)

```
Session
├── id: UUID
├── created_at: ISO 8601 timestamp
├── duration_seconds: u64
├── audio_sources: AudioSourceInfo[]
│   ├── name: string
│   ├── pipewire_node_id: u32
│   └── role: "microphone" | "application"
├── config_snapshot: TranscriptionConfig
│   ├── model: string
│   ├── language: string
│   └── gpu_backend: string
├── transcript: TranscriptSegment[]
│   ├── start_time: f64 (seconds)
│   ├── end_time: f64 (seconds)
│   ├── speaker: string
│   └── text: string
├── speakers: SpeakerMapping[]
│   ├── id: string (e.g., "speaker_0")
│   ├── friendly_name: string (e.g., "Alice")
│   └── source: "microphone" | "remote"
├── summary: Summary | null
│   ├── generated_at: ISO 8601 timestamp
│   ├── backend: string
│   ├── model: string
│   ├── overview: string
│   ├── key_points: string[]
│   ├── action_items: ActionItem[]
│   │   ├── description: string
│   │   └── owner: string | null
│   └── decisions: string[]
└── audio_file_path: string | null
```

---

## 8. UI Design Principles

- **Minimal and unobtrusive** — the daemon should stay out of the way; the tray icon is the primary interface during calls
- **Fast and lightweight** — the settings window and transcript browser should open instantly; no splash screens or loading spinners for local data
- **Desktop-native feel** — use system fonts and respect dark/light theme preferences where possible (iced supports theme detection)
- **Keyboard accessible** — all actions should be reachable via keyboard shortcuts
- **Information density** — transcript viewer should show speaker, timestamp, and text in a compact, scannable layout
- **Consistent with Linux desktop conventions** — left-click tray for quick menu, configurable behavior for other interactions

---

## 9. Security Considerations

Given this is a personal/hobby project with local-only data, security requirements are lightweight but not negligible:

- **No authentication** — single-user system, no login required
- **API keys stored in plaintext config** — acceptable for personal use; document that users should set appropriate file permissions (`chmod 600 config.toml`)
- **Audio data stays local** — raw audio never leaves the machine unless the user explicitly configures a cloud LLM backend
- **Cloud API traffic** — when using external LLM APIs, only transcript text is sent (not raw audio); document this clearly for the user
- **No telemetry** — zero data collection, zero phone-home behavior
- **File permissions** — data directory should be created with user-only permissions (0700)
- **Dependency auditing** — use `cargo audit` in CI to check for known vulnerabilities in dependencies

---

## 10. Development Phases

### Phase 1: Core Audio Pipeline (MVP)

**Goal:** Capture PipeWire audio, transcribe it with Whisper, and save the output.

**Deliverables:**
- Daemon process that connects to PipeWire and captures audio from specified streams
- Whisper integration with GPU support (CUDA + ROCm)
- Basic CLI interface for start/stop recording
- JSON output of timestamped transcripts
- Basic speaker separation (mic vs. app audio labeled as "You" vs. "Remote")
- Configuration via TOML file
- XDG-compliant file storage

**Technical Milestones:**
1. PipeWire connection and stream enumeration
2. Audio capture and buffering (16kHz mono PCM)
3. Whisper transcription with timestamp output
4. Session JSON serialization and storage
5. CLI start/stop control via Unix signals or D-Bus

### Phase 2: System Tray & Basic UI

**Goal:** Add graphical controls and the transcript browser.

**Deliverables:**
- System tray icon with popup menu (start/stop/status)
- Settings window (audio sources, transcription config, storage paths)
- Transcript browser (list, view, search, export to Markdown)
- Desktop notifications for session lifecycle events
- Markdown export

**Technical Milestones:**
1. tray-icon integration with GTK event loop
2. iced settings window with TOML read/write
3. Transcript list view with search
4. notify-rust notification integration
5. Markdown export formatter

### Phase 3: AI Summarization

**Goal:** Generate structured summaries using configurable LLM backends.

**Deliverables:**
- Built-in local LLM inference for summarization (small GGUF model)
- Ollama API integration
- OpenAI-compatible API client (works with OpenAI, Anthropic via proxy, local servers)
- Summary generation with structured output (overview, key points, action items, decisions)
- Auto/manual summarization toggle
- Summary display in transcript browser

**Technical Milestones:**
1. LLM provider trait and factory
2. Ollama REST client
3. OpenAI-compatible HTTP client
4. Prompt engineering for structured summary output
5. Built-in model loading and inference (optional, may defer)

### Phase 4: Polish & Community Release

**Goal:** Prepare for open-source release.

**Deliverables:**
- Comprehensive README and contribution guide
- Packaging for common Linux distributions (Arch AUR, Debian .deb, Flatpak)
- Automated CI/CD (build, test, lint, cargo audit)
- User documentation
- systemd service file for auto-start
- Man page

**Technical Milestones:**
1. CI pipeline (GitHub Actions)
2. Package build scripts
3. Documentation site or wiki
4. Issue templates and contributing guidelines

### Future Phases (Post-Release)

- **Auto-detection of calls** — monitor PipeWire stream creation events to detect when video conferencing apps start audio; prompt user via notification to begin recording
- **Multi-speaker diarization** — integrate pyannote.audio (via subprocess or embedded Python) for granular speaker identification in multi-participant remote audio
- **Real-time transcript preview** — display a live scrolling transcript during recording
- **Customizable summary prompts** — let users define their own summarization templates
- **Hotkey support** — global keyboard shortcuts for start/stop/pause
- **Audio playback** — play back audio segments from within the transcript browser, synced to transcript position
- **Plugin system** — allow community-contributed post-processing plugins (e.g., sentiment analysis, keyword extraction)

---

## 11. Potential Challenges & Mitigations

| Challenge | Description | Mitigation |
|-----------|-------------|------------|
| PipeWire stream identification | Identifying which stream belongs to a video conferencing app vs. music, system sounds, etc. | v1 uses manual source selection; v2+ adds heuristics based on application name metadata from PipeWire |
| GPU backend conflicts | CUDA and ROCm features in whisper-rs are mutually exclusive at compile time | Provide separate build profiles or binary variants; long-term, investigate runtime detection |
| iced + tray-icon event loop coordination | Both iced and tray-icon want their own event loops; iced uses wgpu/winit, tray-icon uses GTK on Linux | Run tray-icon on a separate thread with its own GTK event loop; communicate with the iced UI via channels |
| Speaker diarization accuracy | Whisper alone doesn't do diarization; stream-based separation only gives "you" vs. "them" | Acceptable for v1; multi-speaker diarization is a future phase using pyannote or similar |
| Long call transcription | Very long calls (2+ hours) may produce very large audio buffers and slow transcription | Process audio in chunks during recording; use VAD (Voice Activity Detection) to skip silence |
| Whisper model download size | Large models are multi-gigabyte downloads | Ship with tiny/base by default; provide a model manager in the UI that downloads larger models on demand |
| Wayland clipboard/interaction | Some desktop interactions behave differently on Wayland vs. X11 | Use PipeWire (Wayland-native) for audio; iced and tray-icon both support Wayland; avoid X11-specific APIs |

---

## 12. Estimated Costs

| Item | Cost | Notes |
|------|------|-------|
| Development hardware | $0 | Developer already has suitable hardware with NVIDIA + AMD GPUs |
| Cloud LLM API usage | $0–5/mo | Optional; only if user enables cloud summarization. Ollama is free. |
| Hosting (if cloud API is self-hosted) | $0 | Local only |
| CI/CD | $0 | GitHub Actions free tier is sufficient for open source |
| Distribution | $0 | AUR, Flatpak, and .deb packaging are free |

**Total estimated cost: $0** (for local-only operation)

---

## 13. Future Expansion Possibilities

- **Web-based transcript viewer** — a lightweight local web UI as an alternative to the iced viewer
- **Obsidian/Logseq integration** — export directly to user's note-taking vault
- **Calendar integration** — correlate transcripts with calendar events to auto-name sessions
- **Multi-language support** — transcribe calls in different languages and translate summaries
- **Custom wake word** — voice-activated recording start/stop
- **Meeting analytics** — track speaking time per participant, word frequency, etc.
- **Accessibility** — screen reader support in the GUI

---

## Appendix A: Reference Applications

| Application | Platform | Key Feature to Learn From |
|-------------|----------|--------------------------|
| [Krisp](https://krisp.ai/) | Windows, macOS | System-level audio capture, noise cancellation |
| [Otter.ai](https://otter.ai/) | Web, mobile | Real-time transcription, summary generation, action items |
| [Nerd Dictation](https://github.com/ideasman42/nerd-dictation) | Linux | Linux-native speech-to-text using VOSK, PipeWire integration |
| [WhisperX](https://github.com/m-bain/whisperX) | Python | Whisper + pyannote diarization pipeline, word-level timestamps |
| [COSMIC Desktop](https://github.com/pop-os/cosmic-epoch) | Linux | iced-based production Linux desktop (proves iced's viability) |

---

## Appendix B: Development Environment Setup

```bash
# System dependencies (Ubuntu/Debian)
sudo apt install \
  libpipewire-0.3-dev \
  libspa-0.2-dev \
  libayatana-appindicator3-dev \
  libgtk-3-dev \
  libxdo-dev \
  cmake \
  build-essential \
  clang \
  pkg-config

# Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup default stable  # 1.94.0+

# For NVIDIA GPU support
# Install CUDA Toolkit 12.x from NVIDIA

# For AMD GPU support
# Install ROCm 6.x from AMD

# Clone and build
git clone https://github.com/<user>/vox-daemon.git
cd vox-daemon
cargo build --release --features cuda  # or --features hipblas for AMD
```
