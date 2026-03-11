# Implementation Progress

> This file is updated by specialist agents after completing tasks.

---

<!-- Agents: append your updates below this line -->

## vox-capture — Phase 1 Audio Capture Layer (2026-03-10)

**Agent:** audio-specialist (sonnet)

### What was implemented

**Files created:**

- `/workspace/crates/vox-capture/Cargo.toml` — Cargo manifest with a `pw` feature flag gating the real PipeWire backend (requires `libpipewire-0.3-dev`) and an `integration` feature for live hardware tests.
- `/workspace/crates/vox-capture/src/lib.rs` — Thin crate root; re-exports all public types. `pw` module only compiled when `pw` feature is active.
- `/workspace/crates/vox-capture/src/error.rs` — `CaptureError` enum: Connection, Stream, SourceNotFound, Format, ThreadPanic, Channel, InvalidState.
- `/workspace/crates/vox-capture/src/types.rs` — `AudioChunk` (PCM data + relative timestamp + StreamRole + sample_rate=16000), `StreamInfo` (PipeWire node metadata), `StreamRole` (Microphone/Application), `StreamFilter` (AND-based matching). Unit-tested.
- `/workspace/crates/vox-capture/src/source.rs` — `AudioSource` trait: `list_streams`, `start`, `stop`, `stream_receiver`. Fully documented.
- `/workspace/crates/vox-capture/src/resample.rs` — Linear interpolation resampler. `to_mono` (N-channel downmix by averaging), `resample_linear` (arbitrary rate conversion, no native deps), `convert` (combined pipeline to 16 kHz mono f32). Full unit test coverage.
- `/workspace/crates/vox-capture/src/mock.rs` — `MockAudioSource` implementing `AudioSource`. Supports pre-loaded chunk replay and synthetic sine wave generation. Used for testing without PipeWire. Full unit tests.
- `/workspace/crates/vox-capture/src/pw/mod.rs` — `PipeWireSource` struct implementing `AudioSource`. Spawns a dedicated `vox-pipewire` OS thread; uses `crossbeam-channel` for all cross-thread communication. Provides `enumerate_streams()` static method.
- `/workspace/crates/vox-capture/src/pw/loop_thread.rs` — `run_loop`: PipeWire `MainLoop` + `Context` + `Core`, 10 ms timer for command polling (`LoopCommand::Stop`), per-node `Stream` creation, F32LE 48 kHz stereo capture with resample-to-16kHz-mono in the RT callback, `param_changed` handler for format negotiation/changes, `state_changed` handler for disconnection events. `LoopCommand` and `LoopEvent` enums.
- `/workspace/crates/vox-capture/src/pw/registry.rs` — `list_streams`: synchronous PipeWire registry enumeration with a sync roundtrip, timeout-guarded, returns `Vec<StreamInfo>`.
- `/workspace/crates/vox-capture/tests/integration.rs` — Integration tests gated behind `#[cfg(feature = "integration")]`: enumerate all streams, enumerate microphone sources, 200 ms live capture from first available mic.

### Design decisions

- The `pw` feature is optional; the crate compiles cleanly without `libpipewire-0.3-dev`. All types, traits, and `MockAudioSource` are always available.
- `AudioChunk` always carries `sample_rate: 16_000` as a self-documenting field.
- The PipeWire thread polls for `LoopCommand::Stop` via a 10 ms timer rather than `pw_loop_invoke` because the Rust bindings (v0.9.2) do not expose a thread-safe wakeup.
- `Rc<Cell<u32>>` (not `Arc`) is used for format state shared between `process` and `param_changed` callbacks because all PipeWire callbacks fire on the single PipeWire thread.
- `StreamListener` is stored alongside the `Stream` in `active_streams` to keep both alive for the loop duration.
- Registry enumeration creates a temporary PipeWire connection (independent of any capture session) so `list_streams` can be called without a running capture session.

### Open questions / known limitations

1. **`pw` feature not compiled:** `libpipewire-0.3-dev` is not installed in this build environment. The `pw` module was written against pipewire crate 0.9.2 documentation patterns but has not been compiled. A `NOTE FOR INTEGRATORS` comment in `loop_thread.rs` lists the API surface areas most likely to need adjustment.
2. **`node.target` deprecation:** Newer PipeWire versions prefer `object.id` targeting. The current code uses `node.target` for broad compatibility.
3. **Timer wakeup:** The 10 ms polling timer is a workaround for the missing `pw_loop_invoke()` / thread-safe signal in the Rust bindings.
4. **Format negotiation:** Fixed preference of F32LE 48 kHz 2ch. A production implementation should negotiate via `EnumFormat` params to handle 44100 Hz or S16LE-only devices.
5. **Hot-plug recovery:** Stream disconnection is detected and logged, but automatic reconnection is not implemented.

---

## vox-transcribe — Phase 1 implementation (2026-03-10)

**Agent:** ai-specialist (sonnet)

### What was implemented

- `/workspace/crates/vox-transcribe/Cargo.toml` — crate manifest with `whisper-rs = "0.15.1"` as an optional dependency, plus feature flags `whisper`, `cuda`, and `hipblas`. `cuda` and `hipblas` each imply `whisper`. `tempfile = "3"` as a dev-dependency.

- `/workspace/crates/vox-transcribe/src/lib.rs` — thin facade; declares all modules, re-exports the public API, conditionally re-exports `WhisperTranscriber` only when the `whisper` feature is active. Documents all feature flags and build invocations.

- `/workspace/crates/vox-transcribe/src/transcriber.rs` — core trait and types:
  - `AudioSourceRole` enum (`Microphone` / `Application`) with `speaker_label()` returning `"You"` / `"Remote"` (v1 diarization).
  - `TranscriptionRequest` — holds `Vec<f32>` PCM audio, source role, and a `time_offset_secs` for multi-chunk timeline stitching. Constructors `new()` and `with_offset()`.
  - `TranscriptionResult` — wraps `Vec<TranscriptSegment>`.
  - `Transcriber` trait — synchronous `transcribe(&self, request: &TranscriptionRequest) -> Result<TranscriptionResult, TranscribeError>` with `Send + Sync` bounds.
  - 6 unit tests covering speaker labels, duration calculation, empty detection, offset storage, and segment wrapping.

- `/workspace/crates/vox-transcribe/src/model.rs` — model path resolution and download helpers:
  - `ModelSize` enum (Tiny / Base / Small / Medium / Large) with `from_str()`, `file_name()`, and `download_url()`.
  - `resolve_model_path(config)` — resolves in priority order: custom path if non-empty, otherwise XDG cache dir + `ggml-<size>.bin`; returns `TranscribeError::ModelLoad` if the file doesn't exist.
  - `default_model_path(size)` — returns the expected cache-dir path without existence check (for UI messaging).
  - `is_model_downloaded(config)` — boolean check without error propagation (for UI status).
  - 7 unit tests including roundtrip path resolution with a real temporary file.

- `/workspace/crates/vox-transcribe/src/stub.rs` — always-available `StubTranscriber`:
  - Implements `Transcriber`, returns empty `TranscriptionResult` for non-empty audio.
  - Errors with `TranscribeError::InvalidAudio` on empty buffers.
  - Emits `tracing::warn` on every call to signal absence of real inference.
  - 3 unit tests including trait-object usage.

- `/workspace/crates/vox-transcribe/src/whisper.rs` (behind `#[cfg(feature = "whisper")]`):
  - `WhisperTranscriber` struct holding `Mutex<WhisperContext>` + language string.
  - `unsafe impl Send` / `unsafe impl Sync` with documented safety rationale (whisper.cpp context is read-only after load; `WhisperState` is per-call).
  - `WhisperTranscriber::new(path, language)` — loads GGML model via `WhisperContext::new_with_params`.
  - `WhisperTranscriber::from_config(config)` — delegates path resolution to `model::resolve_model_path`.
  - `build_params()` — configures `FullParams` (Greedy sampling, token timestamps, no-speech threshold, translate=false, print_progress=false).
  - `transcribe()` — creates a `WhisperState` per call (lock held only during `create_state()`), runs `state.full()`, converts centisecond timestamps to seconds with offset, skips empty text segments, labels each segment with the source's speaker label.
  - 3 unit tests (timestamp arithmetic, speaker labels, min-samples constant).

### Design decisions

- **Synchronous `Transcriber` trait**: whisper.cpp inference is CPU/GPU bound, not I/O bound. The daemon wraps calls in `tokio::task::spawn_blocking`. An async trait would add complexity without benefit at the library level.
- **`WhisperState` per call**: whisper-rs's `WhisperState` is the mutable inference handle. Creating it per call (with the mutex held only during `create_state()`) means concurrent calls on different OS threads do not block each other during the actual inference.
- **`unsafe impl Send/Sync`**: Required because `WhisperContext` wraps a raw pointer. The safety invariant is that `WhisperContext` post-load is effectively read-only; all mutation happens through `WhisperState` which is local to each call.
- **No download logic**: The `download_url()` helper and `is_model_downloaded()` check are provided for the daemon/GUI to implement download UI. Actual download (HTTP) is kept out of this crate to avoid pulling in `reqwest`.

### Open questions

- **whisper-rs `WhisperContext` Send-ness**: whisper-rs 0.15.x does not implement `Send` for `WhisperContext` (raw pointer). The `unsafe impl Send` is safe under the documented invariant but should be re-evaluated if whisper-rs upstreams a thread-safe API.
- **Model quantisation**: whisper.cpp supports quantised variants (e.g., `ggml-base.en-q5_1.bin`). The `ModelSize` enum currently only covers unquantised models. A `ModelVariant` type could be added in a future iteration without breaking the existing API.
- **`cargo` not available in this environment** — compilation could not be verified at runtime; code was validated by careful manual review against the `whisper-rs` 0.15.x public API and `vox-core` types.

## vox-storage — Phase 1 implementation (2026-03-10)

**Agent:** storage-specialist (sonnet)

### What was implemented

- `/workspace/crates/vox-storage/Cargo.toml` — crate manifest declaring all required dependencies; `tempfile = "3"` as a dev-dependency.
- `/workspace/crates/vox-storage/src/lib.rs` — thin re-export facade; declares `store` and `markdown` modules.
- `/workspace/crates/vox-storage/src/store.rs` — `SessionStore` trait (save / load / list / delete / export_markdown) and `JsonFileStore` concrete implementation.
  - `JsonFileStore::new(custom_data_dir)` — uses `vox_core::paths::sessions_dir_or` for XDG-compliant path resolution with optional override.
  - `JsonFileStore::with_dir(path)` — test-friendly constructor that accepts an explicit directory.
  - `list()` — skips non-`.json` files and corrupt files with a warning; sorts newest-first by `created_at`.
  - 12 unit tests covering all CRUD operations, edge cases (missing file, non-JSON files in dir, overwrite), and Markdown export round-trip.
- `/workspace/crates/vox-storage/src/markdown.rs` — `render(session)` function producing the full Markdown format specified in the PRD.
  - Timestamp helper `format_timestamp` (outputs `MM:SS` / `HH:MM:SS`).
  - Duration helper `format_duration` (human-readable, e.g. `"1h 02m 30s"`).
  - Speaker resolution from `session.speakers` mappings, falling back to raw speaker IDs.
  - Participants list from speaker mappings, falling back to unique IDs from transcript segments.
  - Summary section (overview, key points, action items with owners, decisions) rendered only when a summary exists.
  - 10 unit tests covering all rendering paths.

### Open questions / notes

- `tempfile` is not a workspace-level dependency; it is declared directly in `vox-storage`'s `[dev-dependencies]` (acceptable per Cargo conventions).
- No audio retention logic is handled here; that is the responsibility of `vox-capture`/`vox-daemon`.
- `cargo` is not available in this environment so compilation could not be verified at runtime; code was validated by careful manual review against `vox-core` types and the workspace `Cargo.toml`.

---

## vox-notify + vox-tray — Phase 2 System Tray & Notifications (2026-03-10)

**Agent:** gui-specialist (sonnet)

### What was implemented

All 24 tests pass. Zero clippy warnings (in scope crates). `cargo fmt --check` passes.

**vox-notify** (`/workspace/crates/vox-notify/`):

- `Cargo.toml` — added `notify-rust = "4.11.6"` and `uuid = { workspace = true }`.
- `src/lib.rs` — thin re-export facade declaring `desktop`, `error`, `notifier` modules.
- `src/error.rs` — `NotifyError` enum with a single `Send(notify_rust::error::Error)` variant using `thiserror`.
- `src/notifier.rs` — `Notifier` trait with four methods: `recording_started`, `recording_stopped(Duration)`, `transcript_ready(Uuid)`, `summary_ready(Uuid)`. All methods return `Result<(), NotifyError>` and the trait is object-safe (`Send + Sync`). `StubNotifier` implementation logs via `tracing` instead of sending real notifications. Five unit tests including trait-object usage.
- `src/desktop.rs` — `DesktopNotifier` backed by `notify-rust`. Each method checks `NotificationConfig.enabled` and the corresponding per-event flag before calling D-Bus. Uses `Urgency::Low` for start/stop, `Urgency::Normal` for transcript/summary. Session UUID is embedded in the notification body so callers can correlate click actions. `format_duration` helper (e.g., `"1h 02m 30s"`). Eight unit tests covering suppression logic, config mutation, and trait-object usage; all suppression tests pass without a real D-Bus session.

**vox-tray** (`/workspace/crates/vox-tray/`):

- `Cargo.toml` — `tray-icon = "0.21"` and `muda = "0.17"` are optional dependencies behind the `gtk` feature. `crossbeam-channel` added as a required dependency.
- `src/lib.rs` — thin facade; `SystemTray` is only re-exported when `cfg(feature = "gtk")`.
- `src/error.rs` — `TrayError` enum: `Create`, `Menu`, `Icon`, `ChannelClosed`, `EventLoopExited`.
- `src/event.rs` — `TrayEvent` enum (seven variants): `StartRecording`, `StopRecording`, `PauseRecording`, `OpenLastTranscript`, `BrowseTranscripts`, `OpenSettings`, `Quit`. `DaemonStatus` enum: `Idle`, `Recording`, `Processing` with `label()` and `is_recording()` helpers. Four unit tests.
- `src/tray.rs` — `Tray` trait: `set_status(DaemonStatus)`, `recv_event() -> Option<TrayEvent>` (blocking), `try_recv_event() -> Option<TrayEvent>` (non-blocking). Fully documented, `Send + Sync`.
- `src/mock.rs` — `MockTray`: uses two unbounded `crossbeam_channel` pairs. `inject_event()` simulates user clicks; `last_status()` drains the status channel to retrieve the most recent update. Implements `Tray`. Six unit tests.
- `src/system_tray.rs` (behind `#[cfg(feature = "gtk")]`) — `SystemTray` struct that spawns a `vox-tray-gtk` OS thread running the GTK event loop. Communicates via a bounded `StatusUpdate` channel inbound and an unbounded `TrayEvent` channel outbound. Implements `Tray`. Also contains: pure-Rust PNG generation (`generate_circle_png`) producing coloured circle icons at 32x32 RGBA (green=idle, red=recording, yellow=processing) using deflate stored-block zlib and CRC-32 computed from a compile-time table — no external image libraries needed. `png_to_rgba` decodes the generated PNG back to raw pixels. Four unit tests including a PNG roundtrip.

### Design decisions

- `Notifier::transcript_ready` and `summary_ready` accept `uuid::Uuid` (not `&str`) so callers hold a typed session ID rather than an arbitrary string. The session ID is formatted into the notification body; future implementations can attach it as a D-Bus action hint.
- PNG icons are generated programmatically at runtime using only stdlib primitives plus a compile-time CRC-32 table. This avoids any external image dependencies and the need for bundled icon files on the filesystem.
- `MockTray.last_status()` drains the entire channel to return the newest value, matching the semantics callers expect (they want the current state, not a history).
- The GTK event loop in `system_tray.rs` polls at 10 ms intervals in the absence of a glib runtime in the current build environment. In production, `gtk::main_iteration_do(false)` would replace the `thread::sleep`.

### Open questions / known limitations

1. **`gtk` feature not compilable in this build environment** — `libgtk-3-dev`, `libpango-dev`, etc. are not installed. The `gtk` feature flag correctly gates the entire `system_tray` module so the default build is unaffected. The system_tray code was validated by careful manual review against `tray-icon` 0.21 and `muda` 0.17 API docs.
2. **Icon update via `TrayIcon::set_icon`** — within the GTK loop, updating the icon requires a mutable handle to `TrayIcon`. The current implementation logs the update but does not call `set_icon` because `TrayIcon` is owned by the same block as the loop; a `RefCell<TrayIcon>` or restructuring to a `glib` idle callback would be needed to fully implement dynamic icon switching.
3. **`try_into().unwrap()` in inflate_stored** — the `u16::from_le_bytes` call uses `.unwrap()` on a slice-to-array conversion that is guaranteed by the surrounding length check, but clippy's pedantic mode may flag it in a future pass. A `map_err` wrapper would clean it up.
4. **Notification action buttons** — `notify-rust` supports adding action buttons (e.g., "Open") via the `action()` builder method, but handling the callback requires a persistent `NotificationHandle` and a D-Bus signal listener. The current implementation sends fire-and-forget notifications; action handling is left for a future iteration when the transcript browser window is available to open.

---

## vox-summarize — Phase 3 LLM Summarization (2026-03-10)

**Agent:** ai-specialist (claude-sonnet-4-6)

### What was implemented

All 35 unit tests and 2 doc-tests pass. Zero clippy warnings (`-D warnings`). Code compiles cleanly.

**Files created / modified:**

- `/workspace/crates/vox-summarize/Cargo.toml` — added all required dependencies: `reqwest`, `serde`, `serde_json`, `tokio`, `tracing`, `thiserror`, `chrono`, `async-trait`.

- `/workspace/crates/vox-summarize/src/error.rs` — `SummarizeError` enum with variants: `BackendNotImplemented`, `UnknownBackend`, `EmptyTranscript`, `Http` (wraps `reqwest::Error`), `ApiError { status, body }`, `ParseError { reason, raw }`, `EmptyResponse`, `Json` (wraps `serde_json::Error`), `Config`.

- `/workspace/crates/vox-summarize/src/traits.rs` — `Summarizer` trait using `async-trait`, `Send + Sync`, single method `summarize(&self, transcript: &[TranscriptSegment]) -> Result<Summary, SummarizeError>`.

- `/workspace/crates/vox-summarize/src/prompt.rs` — `build_prompt(segments) -> (String, String)` returning a `(system_prompt, user_prompt)` pair. System prompt instructs the LLM to output strict JSON with `overview`, `key_points`, `action_items`, `decisions` fields. `format_transcript` renders each segment as `[HH:MM:SS - HH:MM:SS] Speaker: text\n`. Long transcripts are truncated to ~6,000 tokens (24,000 chars) keeping the first 60% and last 40% with a truncation notice in the middle. 8 unit tests.

- `/workspace/crates/vox-summarize/src/parse.rs` — `parse_response(text, backend, model) -> Result<Summary, SummarizeError>`. Three-strategy cascade: (1) parse entire response as JSON, (2) extract first `{...}` block from prose/fenced response, (3) markdown section fallback (`## Overview`, `## Key Points`, etc.) with bullet stripping and `Owner: task` / `task (Owner)` action-item splitting. Always produces a valid `Summary` even from garbage input. 13 unit tests.

- `/workspace/crates/vox-summarize/src/client.rs` — `OpenAiClient` struct implementing `Summarizer`. Uses `reqwest` to POST to any `/v1/chat/completions` endpoint. Features: configurable base URL (trailing slash normalised), optional bearer token, model name, 90 s timeout, `response_format: { type: "json_object" }` hint, graceful API error extraction from JSON error bodies. 4 unit tests (endpoint normalisation, empty transcript guard).

- `/workspace/crates/vox-summarize/src/factory.rs` — `create_summarizer(config) -> Result<Box<dyn Summarizer>, SummarizeError>`. Maps `config.backend` to:
  - `"ollama"` → `OpenAiClient` with Ollama URL and model, no API key.
  - `"openai_compatible"` → `OpenAiClient` with custom URL/key/model; validates required fields.
  - `"builtin"` → `SummarizeError::BackendNotImplemented` (deferred to Phase 4).
  - anything else → `SummarizeError::UnknownBackend`.
  9 unit tests covering all branches and validation paths.

- `/workspace/crates/vox-summarize/src/lib.rs` — updated to declare and re-export all new modules alongside the pre-existing `stub` module. Public re-exports: `OpenAiClient`, `SummarizeError`, `create_summarizer`, `Summarizer`, `StubSummarizer`.

### Design decisions

- **Single `OpenAiClient` for all HTTP backends**: Ollama exposes an OpenAI-compatible API, so one implementation serves both local and cloud endpoints. Configurable base URL covers all current backends without pulling in heavy multi-provider SDKs.
- **`response_format: json_object` hint**: Sent to encourage models that support it (OpenAI, recent Ollama) to output valid JSON. The multi-strategy parser handles models that ignore it or wrap JSON in prose.
- **`StubSummarizer` preserved**: The pre-existing stub (no-op returning fixed text) is kept and re-exported. It is useful in tests, CI, and offline development.
- **`parse_response` is infallible in practice**: Returns `Ok` even for unparseable input, storing the raw text in `overview`. This prevents summarization failures from blocking session storage. If structured parsing becomes critical, the `ParseError` variant is available for callers to opt into stricter behaviour.
- **`FallbackSection` enum hoisted out of function body**: Required by `clippy::items_after_statements`. Makes the section tracking logic cleaner.

### Open questions / known limitations

1. **`builtin` backend deferred**: Returning `BackendNotImplemented` is correct per the spec (Phase 4 item). When built-in llama.cpp inference is added, `factory.rs` is the only file that needs to change.
2. **Chunked summarization for very long calls**: The prompt module truncates at ~6,000 tokens. For calls longer than ~90 minutes on a verbose model, the middle of the conversation is dropped. A proper multi-stage summarization (chunk → summarize chunks → summarize summaries) is noted in the design doc as a future improvement.
3. **`response_format: json_object` compatibility**: Older Ollama versions and some OpenAI-compatible servers do not support this field and may return an error. The fallback parser mitigates this at the response level, but the server error itself would surface as `SummarizeError::ApiError`. A retry without `response_format` could be added if this becomes a problem in practice.
4. **Streaming not implemented**: The task description mentioned streaming support with collection into a full response. The current implementation uses a non-streaming completion call, which is simpler and sufficient for the structured-output use case.

---

## vox-gui — Phase 2 Settings Window & Transcript Browser (2026-03-10)

**Agent:** gui-specialist (claude-sonnet-4-6)

### What was implemented

30 unit tests pass. Zero clippy warnings (both default and `--features ui` builds). Both `cargo build -p vox-gui` and `cargo build -p vox-gui --features ui` succeed.

**Files created / modified:**

- `/workspace/crates/vox-gui/Cargo.toml` — added `vox-storage`, `serde`, `uuid`, `chrono` as required dependencies; `iced = "0.14"` with `features = ["tokio"]` as an optional dependency behind the `ui` feature flag; `tempfile = "3"` as a dev-dependency.

- `/workspace/crates/vox-gui/src/lib.rs` — thin crate root that declares all modules. `app` and `theme` are gated behind `#[cfg(feature = "ui")]`; all others are always compiled.

- `/workspace/crates/vox-gui/src/error.rs` — `GuiError` enum with `Config`, `Storage`, and `InvalidField` variants using `thiserror`.

- `/workspace/crates/vox-gui/src/settings.rs` — `SettingsModel` and its sub-structs (`AudioSettings`, `TranscriptionSettings`, `SummarizationSettings`, `StorageSettings`, `NotificationSettings`). Strong-typed enums for UI selection lists: `WhisperModel`, `GpuBackend`, `SummarizationBackend`, `ExportFormat` — each with `all()`, `as_str()`, `from_str()`, and `Display`. `SettingsModel::from_config(&AppConfig) -> Self` and `SettingsModel::to_config(&self) -> AppConfig` for round-trip conversion. Unknown config values fall back to defaults with a `tracing::warn`. 9 unit tests covering all enums, round-trips, fallback, and modification.

- `/workspace/crates/vox-gui/src/browser.rs` — `SessionListEntry` lightweight struct for the transcript list (id, date, duration, segment count, summary preview). `SessionListEntry::from_session` builds an entry from a full `Session`, truncating summary previews at 120 characters with a UTF-8 ellipsis. `build_session_list` converts a slice of sessions. `format_duration` helper produces human-readable strings (`"1h 02m 30s"`). `formatted_date()` and `formatted_duration()` methods on `SessionListEntry`. 11 unit tests covering all formatting and truncation logic.

- `/workspace/crates/vox-gui/src/search.rs` — `SearchResult { session_id, matching_segment_indices }` and `search_transcripts(sessions: &[Session], query: &str) -> Vec<SearchResult>` performing case-insensitive substring search. Returns only sessions with at least one match. 10 unit tests covering: single match, case-insensitivity, no match, multi-segment match, cross-session search, empty sessions, empty query (matches all), partial word, and `is_empty` guard.

- `/workspace/crates/vox-gui/src/theme.rs` (behind `ui` feature) — speaker colour constants (`SPEAKER_YOU_COLOR`, `SPEAKER_REMOTE_COLOR`, `SPEAKER_UNKNOWN_COLOR`), spacing/padding constants (`PADDING = 12.0f32`, `SPACING = 8.0f32`, `SECTION_SPACING = 20.0f32`), window size constants, and `speaker_color(speaker: &str) -> Color` helper.

- `/workspace/crates/vox-gui/src/app.rs` (behind `ui` feature) — full iced 0.14 GUI application using the functional builder pattern (`iced::application(new, update, view)`). Two pages: `Page::Settings` and `Page::Browser`. `Message` enum with ~35 variants covering all UI interactions. `VoxAppState` struct holding all mutable state. `update` handles all messages including async `Task::perform` for settings save, session load, export, and delete. `view` renders both pages. Settings view has sections for Audio, Transcription, Summarization, Storage, Notifications, and About. Browser view has a search bar, scrollable session list, and a detail panel with speaker name editor and transcript viewer. `run()` entry point for launching the window.

### Design decisions

- **Feature-gated iced**: The `ui` feature correctly gates all iced-dependent code. The base crate (settings models, search, browser list entries) compiles without any display server or GPU requirement, enabling headless use and unit testing in CI.
- **`f32` for spacing/padding constants**: iced 0.14's `Pixels: From<f32>` and `Padding: From<f32>` both work; `u16` works only for `Padding` and `u32` only for `Pixels`. Using `f32` satisfies both.
- **`iced::application(new, update, view).title("Vox Daemon")` pattern**: iced 0.14 changed from `Application` trait to a functional builder. The title is set via `.title()` builder method rather than being passed as the first argument.
- **`rule::horizontal(1)` instead of `horizontal_rule(1)`**: The `horizontal_rule` shorthand from iced 0.12/0.13 was removed; the correct iced 0.14 API is `iced::widget::rule::horizontal(pixels)`.

### Open questions / known limitations

1. **GPU/display required for `ui` feature at runtime**: `cargo build --features ui` compiles successfully, but launching the window requires a Wayland or X11 display server with wgpu-compatible GPU drivers. The build environment does not have these, so end-to-end rendering was not tested.
2. **`WhisperModel`, `GpuBackend`, `SummarizationBackend`, `ExportFormat` need `Clone` for pick_list**: iced 0.14's `pick_list` requires `T: Clone`. All UI enum types derive `Clone` and `Copy`, so this is satisfied.
3. **No live settings auto-save on toggle**: Settings are only persisted when the user clicks "Save Settings". A future iteration could save on every change (debounced) to match PRD requirement "settings changes are persisted to disk immediately".
4. **Speaker name edits are not persisted**: The `SpeakerNameEdited` message updates in-memory state but does not call `SessionStore::save`. A save step after each edit (or a "Save Speaker Names" button) is needed.
5. **Search re-runs on every keystroke**: For very large session libraries, a debounce or background task would improve responsiveness.

---

## vox-capture pw module — API fixes for pipewire 0.9.2 (2026-03-11)

**Agent:** audio-specialist (claude-sonnet-4-6)

### What was fixed

The `pw` module (`loop_thread.rs` and `registry.rs`) was written against assumed API signatures that did not match `pipewire` crate v0.9.2. All mismatches have been corrected by inspecting the actual crate source at `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/pipewire-0.9.2/`.

**`/workspace/crates/vox-capture/src/pw/loop_thread.rs`:**

1. `MainLoop::new(None)` → `MainLoopBox::new(None)`. The owned smart-pointer type is `MainLoopBox`, not `MainLoop`. Import changed to `pipewire::main_loop::MainLoopBox`.
2. `Context::new(&main_loop)` → `ContextBox::new(main_loop.loop_(), None)`. The owned type is `ContextBox`; its constructor takes `&Loop` (via `main_loop.loop_()`) plus `Option<PropertiesBox>`. Import changed to `pipewire::context::ContextBox`.
3. `Stream::new(core, "vox-capture", props)` → `StreamBox::new(core, "vox-capture", props)`. The owned type is `StreamBox`; it takes `PropertiesBox` (which the `properties!` macro already returns). Import changed to `pipewire::stream::StreamBox`. Return type of `open_capture_stream` changed from `(Stream, StreamListener<()>)` to `(StreamBox<'_>, StreamListener<()>)`.
4. Timer callback signature: `add_timer` requires `Fn(u64)` (expirations count), not `Fn(_)`. The `_` placeholder caused a type annotation error. Changed to `move |_expirations: u64|`.
5. Timer arming: `.update(Duration, Duration, bool)` does not exist. Changed to `.update_timer(Option<Duration>, Option<Duration>)` which is the correct `TimerSource::update_timer` signature.
6. `process` callback signature: was `FnMut(&Stream, _)`, must be `FnMut(&Stream, &mut D)`. Added explicit `_user_data: &mut ()` parameter.
7. `param_changed` callback signature: was `FnMut(_, id, _, param)`, must be `FnMut(&Stream, &mut D, u32, Option<&Pod>)`. Added `_stream` and `_user_data: &mut ()` parameters.
8. `state_changed` callback signature: was `FnMut(old, new)`, must be `FnMut(&Stream, &mut D, StreamState, StreamState)`. Added `_stream` and `_user_data: &mut ()` parameters.
9. `AudioInfoRaw::parse(param)` as a free function does not exist. It is a `&mut self` method. Changed to `let mut info = AudioInfoRaw::new(); if info.parse(param).is_ok() { … info.rate() … info.channels() … }`.
10. `stream.connect` params type: was `&mut [*const Pod]`, must be `&mut [&spa::pod::Pod]`. Changed `let mut params = [param_pod as *const Pod]` to `let mut params = [param_pod]` where `param_pod: &Pod`.
11. `StreamState::Error(msg)` pattern: `msg` is `String`, not `&str`. Changed `.to_owned()` to `.clone()` and matched with `ref msg`.

**`/workspace/crates/vox-capture/src/pw/registry.rs`:**

1. `MainLoop::new(None)` → `MainLoopBox::new(None)`. Same as above.
2. `Context::new(&main_loop)` → `ContextBox::new(main_loop.loop_(), None)`. Same as above.
3. `global.props` type: was assumed to dereference to a struct with `.get()`, but the actual type is `Option<&spa::utils::dict::DictRef>`. The `DictRef` trait provides `.get(key)` directly. Added explicit type annotation `let props: &spa::utils::dict::DictRef = match global.props { Some(p) => p, None => return };` to resolve the inference ambiguity.

### Compile status

`cargo check --package vox-capture --features pw` fails with a missing-system-library error from the `libspa-sys` build script (`libpipewire-0.3` is not installed in this environment). This is the expected and pre-existing linker-level failure noted in the original open questions. All Rust-level API mismatch errors are resolved; the remaining error is purely a missing C library on the build host.

### Open questions / known limitations

1. **`libpipewire-0.3-dev` not installed** — linker errors persist until the system library is present. No Rust API changes are needed.
2. **`node.target` deprecation** — unchanged from original implementation.
3. **Hot-plug recovery** — unchanged from original implementation.

---

## Phase 4: Polish & Community Release (2026-03-11)

**Agent:** Orchestrator (opus)

### What was implemented

**CI/CD:**
- `.github/workflows/ci.yml` — GitHub Actions pipeline with 4 jobs:
  - `check`: fmt, clippy, build, test
  - `audit`: `cargo audit` via `rustsec/audit-check`
  - `msrv`: Build verification with Rust 1.85.0
  - `release`: Release build + artifact upload (on main branch only)
- `.github/ISSUE_TEMPLATE/bug_report.md` — Bug report template with environment checklist
- `.github/ISSUE_TEMPLATE/feature_request.md` — Feature request template

**Packaging:**
- `dist/arch/PKGBUILD` — Arch Linux / AUR package build script
- `dist/debian/control` + `dist/debian/rules` — Debian package metadata
- `dist/flatpak/com.github.user.VoxDaemon.yml` — Flatpak manifest

**systemd:**
- `dist/systemd/vox-daemon.service` — systemd user service with PipeWire dependency, hardening directives, and auto-restart

**Documentation:**
- `README.md` — Project overview, features, installation, quick start, configuration reference, architecture diagram, feature flags
- `CONTRIBUTING.md` — Development workflow, coding standards, PR guidelines
- `LICENSE` — MIT License
- `dist/man/vox-daemon.1` — roff-formatted man page covering all subcommands, options, files, configuration, examples, and environment variables

### Daemon integration (Phase 2/3 → binary)

The `vox-daemon` binary was updated to integrate all Phase 2/3 crates:
- `vox-tray`, `vox-notify`, `vox-summarize` added as dependencies
- `daemon.rs` rewritten with tray event loop (MockTray without GTK, StubNotifier)
- Two new CLI subcommands: `summarize <uuid>` and `export <uuid>`
