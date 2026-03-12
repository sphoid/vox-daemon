# QA Review Log

> This file is updated by the qa-reviewer agent after each review.

---

<!-- QA reviewer: append your review reports below this line -->

## QA Review: Phase 2 (Tray, Notifications, GUI) and Phase 3 (Summarization)

**Date:** 2026-03-10
**Crate(s):** vox-tray, vox-notify, vox-gui, vox-summarize
**PRD Sections:** 4.4 (Summarization), 4.5 (System Tray), 4.7 (Transcript Browser), 4.8 (Notifications)
**Reviewer:** qa-reviewer

### Acceptance Criteria

#### PRD 4.5 — System Tray Interface

| # | Criterion | Status | Notes |
|---|-----------|--------|-------|
| 1 | Tray icon indicates daemon status (idle, recording, processing) | PARTIAL | `DaemonStatus` enum and icon-color mapping exist in `system_tray.rs` (green=idle, red=recording, yellow=processing). However, the `set_icon` call is commented out at line 549-556 with a TODO comment ("In a real GTK loop we'd call tray_icon.set_icon(icon)"), so icons are generated but never actually swapped. |
| 2 | Quick-access controls: Start/Stop/Pause Recording | PASS | All three menu items exist (`TrayEvent::StartRecording`, `StopRecording`, `PauseRecording`). Menu items are created with correct enable/disable states (start enabled, stop/pause disabled at init). |
| 3 | Visual indicator when actively recording (icon change) | PARTIAL | Icon generation code is correct (red circle for recording) but actual icon swap is not wired up (see #1). |
| 4 | Quick link to open most recent transcript | PASS | `TrayEvent::OpenLastTranscript` menu item exists. |
| 5 | Option to open full settings window | PASS | `TrayEvent::OpenSettings` menu item exists. |
| 6 | Tray icon appears in GNOME, KDE Plasma, and Sway | UNTESTABLE | Requires a desktop session with GTK and the `gtk` feature enabled. The code uses `tray-icon` which supports all three via libayatana-appindicator. |
| 7 | All settings changes are persisted to disk immediately | PARTIAL | Settings are persisted on explicit "Save Settings" button click (`Message::SaveSettings`), not on every individual field change. The PRD says "persisted to disk immediately". |
| 8 | Recording can be started and stopped entirely from the tray menu | PASS | Events `StartRecording` and `StopRecording` are emitted. The daemon binary would need to handle these. |
| 9 | Settings window renders correctly on Wayland | UNTESTABLE | Requires a Wayland session and the `ui` feature. iced 0.14 supports Wayland via wgpu. |

#### PRD 4.5 — Settings Window Content

| # | Criterion | Status | Notes |
|---|-----------|--------|-------|
| 10 | Audio source selection (PipeWire stream picker) | PARTIAL | Text input fields for mic/app source exist, but no actual PipeWire stream enumeration/picker is provided. Users must type node IDs manually. |
| 11 | Transcription settings: model, language, GPU preference | PASS | Pick lists for `WhisperModel`, `GpuBackend`, language text input, and model path text input are all present. |
| 12 | Summarization settings: backend, API endpoint, auto/manual | PASS | Backend pick list, auto-summarize toggle, Ollama URL/model fields, API URL/key/model fields all present. API key field uses `secure(true)`. |
| 13 | Storage settings: output directory, export format | PASS | Data directory text input, export format pick list, retain audio toggle all present. |
| 14 | Notification preferences | PASS | Master toggle plus per-event toggles (record start/stop, transcript ready, summary ready). |
| 15 | About / version info | PASS | About section shows version via `env!("CARGO_PKG_VERSION")`, license, and source URL. |

#### PRD 4.7 — Transcript Browser UI

| # | Criterion | Status | Notes |
|---|-----------|--------|-------|
| 16 | List past sessions with date, duration, and summary preview | PASS | `SessionListEntry` includes `created_at`, `duration_seconds`, `segment_count`, and `summary_preview`. Browser view renders all of these. |
| 17 | Full transcript viewer with speaker-color-coded text and timestamps | PARTIAL | Timestamps are displayed in `[MM:SS]` format, speaker names are shown, but speaker-color-coding is NOT applied in the view. `theme::speaker_color()` exists but is never called from `app.rs`. |
| 18 | Text search across all transcripts (full-text) | PASS | `search_transcripts()` performs case-insensitive substring search. `visible_sessions()` filters the list based on search results. |
| 19 | Export individual sessions to Markdown | PASS | `Message::ExportSelectedSession` triggers `store.export_markdown()`. |
| 20 | Delete sessions | PASS | `Message::DeleteSelectedSession` triggers `store.delete()` and refreshes the list. |
| 21 | Ability to edit speaker friendly names | PASS | `SpeakerNameEdited` message with speaker index and new name. Text inputs rendered per speaker. |
| 22 | Accessible from tray menu or standalone | PASS | `TrayEvent::BrowseTranscripts` exists. `app::run()` provides standalone launch. |
| 23 | Transcript list loads within 1 second for up to 100 sessions | UNTESTABLE | Requires populated data store and a display server. Design is reasonable (lightweight `SessionListEntry`). |
| 24 | Search returns results across all stored transcripts | PASS | `search_transcripts` iterates all sessions and all segments. |
| 25 | Markdown export matches internal JSON content | UNTESTABLE | Depends on `vox-storage` implementation of `export_markdown`. |

#### PRD 4.8 — Desktop Notifications

| # | Criterion | Status | Notes |
|---|-----------|--------|-------|
| 26 | Notify when: recording starts, stops, transcription completes, summary generated | PASS | `Notifier` trait has all four methods; `DesktopNotifier` implements them with config-gated D-Bus calls. |
| 27 | Notifications are clickable — clicking opens the relevant transcript | FAIL | No `.action()` or click handler is set on any notification. `notify-rust` supports actions via `.action("default", "Open")` and the `NotificationHandle` callback. Session UUIDs are passed to `transcript_ready` and `summary_ready` but are only displayed in the body text, not wired to a click action. |
| 28 | Respect system DND / notification suppression | PASS | `notify-rust` uses D-Bus `org.freedesktop.Notifications` which respects the desktop's DND settings natively. |
| 29 | Follow XDG Desktop Notification Specification | PASS | `notify-rust` is an XDG-compliant notification library. |
| 30 | Notifications appear correctly on GNOME, KDE, Sway | UNTESTABLE | Requires running desktop sessions. `notify-rust` supports all three. |
| 31 | Notification content is concise and informative | PASS | Messages are brief and include duration/session info as appropriate. |

#### PRD 4.4 — AI-Powered Summarization

| # | Criterion | Status | Notes |
|---|-----------|--------|-------|
| 32 | Support three LLM backends: builtin, local server, cloud API | PARTIAL | Ollama and OpenAI-compatible backends are fully implemented via `OpenAiClient`. Built-in local inference returns `BackendNotImplemented` error. This is acceptable for Phase 3 scope (PRD 10 Phase 3 item 5 says "Built-in model loading and inference (optional, may defer)"). |
| 33 | Generate summaries: overview, key points, action items, decisions | PASS | System prompt requests JSON with all four fields. `parse_response` handles clean JSON, JSON-in-prose, markdown fallback, and total garbage. `Summary` struct contains all required fields. |
| 34 | Action items with assigned owners | PASS | `ActionItem` struct has `description` and `owner: Option<String>`. Parser extracts owners from both "Owner: task" and "task (Owner)" patterns. |
| 35 | Configurable trigger: auto on stop or manual via UI | PASS | `SummarizationConfig.auto_summarize` flag exists. Settings UI has the toggle. |
| 36 | Summary appended to transcript JSON metadata | PASS | `Summary` struct is part of `Session` (via `session.summary`). |
| 37 | Summary generated within 60 seconds | UNTESTABLE | Depends on LLM server speed. Timeout set to 90 seconds in `OpenAiClient`. |
| 38 | Summary structure is consistent and parseable | PASS | JSON schema is strictly defined in system prompt. Multi-strategy parser ensures a valid `Summary` is always produced. |
| 39 | User can re-generate with different backend or prompt | PARTIAL | Factory function allows creating any backend at runtime. However, the GUI does not expose a "re-summarize" button. The trait supports it. |

### Code Quality

| Check | Status | Notes |
|-------|--------|-------|
| Error handling | PASS (with 1 exception) | All crates use `thiserror` for error enums. Errors propagate with `?`. One `.unwrap()` in library code: `system_tray.rs:274` inside `inflate_stored()`. This is behind `#[cfg(feature = "gtk")]` so it does not compile in CI, but it is still library code and should use `.map_err()` or `.expect("reason")`. |
| Documentation | PASS | All public types and functions have `///` doc comments. Module-level `//!` docs are thorough. `#[must_use]` is applied on functions returning values that should not be discarded. |
| Testing | PASS | 88 unit tests across the 4 crates, all passing. Tests cover core logic, edge cases (empty input, unknown config values, object safety), and round-trip conversions. |
| Architecture | PASS | Trait-based APIs: `Tray`, `Notifier`, `Summarizer`. Feature flags gate native dependencies (`gtk` for tray-icon, `ui` for iced). Channel-based communication in tray (crossbeam). Correct crate boundaries. |
| Rust idioms | PASS (with 1 exception) | `Result` return types throughout. Proper use of `Option`, `impl Into<String>` for ergonomic APIs. One unsafe byte-index slice at `browser.rs:46` (`&overview[..120]`) that will panic on multi-byte UTF-8 input. |

### Build & Test Results

- `cargo check` (reviewed crates): PASS
- `cargo clippy` (reviewed crates): PASS (0 warnings with `-D warnings`)
- `cargo fmt --check --all`: **FAIL** — Formatting differences in multiple files across `vox-gui/src/app.rs`, `vox-gui/src/browser.rs`, `vox-gui/src/search.rs`, `vox-gui/src/settings.rs`, `vox-gui/src/lib.rs`, and files in other crates outside this review scope.
- `cargo test` (reviewed crates): PASS (88 passed, 0 failed)
- `cargo test --workspace`: **FAIL** — `vox-daemon` binary fails to compile due to non-exhaustive match at `vox-daemon/src/main.rs:95` (missing arms for `Command::Summarize` and `Command::Export`). This is outside the scope of the reviewed crates but blocks workspace-wide testing.

### Dependency Audit

| Crate | Dependency | Expected Version | Actual Version | Status |
|-------|-----------|-----------------|----------------|--------|
| vox-tray | tray-icon | 0.21.2 | 0.21 (Cargo.toml) | PASS (minor flexible) |
| vox-tray | muda | 0.17.x | 0.17 | PASS |
| vox-tray | crossbeam-channel | 0.5.x | workspace | PASS |
| vox-notify | notify-rust | 4.11.6 | 4.11.6 (resolved 4.12.0) | PASS |
| vox-gui | iced | 0.14.0 | 0.14 | PASS |
| vox-summarize | reqwest | 0.12.x | workspace | PASS |
| vox-summarize | async-trait | - | 0.1 | NOTE: Not in CLAUDE.md dependency table but commonly used. Consider removing if edition 2024 supports native async traits in the trait bounds needed. |

### Issues Found

1. **[SEVERITY: MEDIUM]** `cargo fmt` check fails across multiple files in the reviewed crates.
   - Location: `crates/vox-gui/src/app.rs`, `crates/vox-gui/src/browser.rs`, `crates/vox-gui/src/search.rs`, `crates/vox-gui/src/settings.rs`, `crates/vox-gui/src/lib.rs`
   - Expected: All code passes `cargo fmt --check`
   - Actual: Multiple formatting differences
   - Fix suggestion: Run `cargo fmt --all`

2. **[SEVERITY: MEDIUM]** UTF-8 panic risk in `SessionListEntry::from_session` summary preview truncation.
   - Location: `crates/vox-gui/src/browser.rs:46`
   - Expected: Safe truncation that handles multi-byte characters
   - Actual: `&overview[..120]` uses byte indexing which panics if byte 120 falls mid-character
   - Fix suggestion: Use `overview.char_indices().take_while(|(i, _)| *i < 120).last()` to find a safe truncation point, or use the `unicode-segmentation` crate.

3. **[SEVERITY: LOW]** `.unwrap()` in library code within `inflate_stored()`.
   - Location: `crates/vox-tray/src/system_tray.rs:274`
   - Expected: No `.unwrap()` in library code per CLAUDE.md coding standards
   - Actual: `data[pos..pos + 2].try_into().unwrap()` — could use `.map_err()` to return a descriptive error
   - Fix suggestion: Replace with `.map_err(|_| "truncated stored block LEN field".to_owned())?` and change the return type appropriately, or use `.expect("LEN field is exactly 2 bytes; bounds checked at line 271")` with a documented invariant.

4. **[SEVERITY: MEDIUM]** Tray icon is never actually updated when status changes.
   - Location: `crates/vox-tray/src/system_tray.rs:549-556`
   - Expected: `tray_icon.set_icon(icon)` is called to swap the icon
   - Actual: Icon is generated but discarded with a comment saying "In a real GTK loop we'd call tray_icon.set_icon(icon)"
   - Fix suggestion: Move `_tray_icon` to a mutable binding and call `set_icon()` on status change.

5. **[SEVERITY: MEDIUM]** Notifications are not clickable (PRD 4.8 requirement).
   - Location: `crates/vox-notify/src/desktop.rs:60-124`
   - Expected: Clicking a notification opens the relevant transcript
   - Actual: No `.action()` or callback handler is set on any notification
   - Fix suggestion: Use `notify-rust`'s `.action("default", "Open")` and return the `NotificationHandle` so the caller can register a callback, or use D-Bus signal matching.

6. **[SEVERITY: LOW]** Speaker-color-coded text not applied in transcript viewer.
   - Location: `crates/vox-gui/src/app.rs:829` (where `text(&seg.speaker)` is used without color)
   - Expected: Speaker names or text use color-coding per `theme::speaker_color()`
   - Actual: `speaker_color()` is defined but never called in the view
   - Fix suggestion: Apply `.color(vox_theme::speaker_color(&seg.speaker))` to the speaker text widget.

7. **[SEVERITY: HIGH]** Workspace does not compile due to `vox-daemon/src/main.rs:95` non-exhaustive match.
   - Location: `vox-daemon/src/main.rs:95`
   - Expected: All `Command` variants handled
   - Actual: `Command::Summarize` and `Command::Export` added (presumably during Phase 3) but not matched
   - Fix suggestion: Add match arms for the two new variants. This is outside the reviewed crates but blocks `cargo test --workspace`.

8. **[SEVERITY: LOW]** `async-trait` dependency used in `vox-summarize` when Rust edition 2024 supports native async traits.
   - Location: `crates/vox-summarize/Cargo.toml:18`
   - Expected: Use native `async fn` in traits (stable since Rust 1.75)
   - Actual: Uses `#[async_trait]` macro
   - Fix suggestion: Remove `async-trait` dependency. Use `-> impl Future<Output = ...> + Send` or `async fn` directly if the trait does not need to be object-safe, or use `-> Pin<Box<dyn Future<...> + Send>>` for dyn dispatch. Note: `async-trait` is still useful for object safety with `Box<dyn Summarizer>`, so this is a style preference rather than a bug.

9. **[SEVERITY: LOW]** Settings not persisted immediately on change (PRD says "immediately").
   - Location: `crates/vox-gui/src/app.rs:351`
   - Expected: Each settings field change triggers a save
   - Actual: Save only happens when user clicks "Save Settings" button
   - Fix suggestion: Either auto-save via debounced writes on each field change, or document the UX decision as intentional.

10. **[SEVERITY: LOW]** PipeWire stream picker not implemented — audio source fields are plain text inputs.
    - Location: `crates/vox-gui/src/app.rs:537-548`
    - Expected: A stream picker that enumerates active PipeWire streams
    - Actual: Users must manually type node IDs
    - Fix suggestion: Integrate with `vox-capture` stream listing to populate a pick list. This may be acceptable as a known limitation for Phase 2.

### Verdict

**NEEDS REVISION**

Blocking issues that must be fixed before approval:

1. **`cargo fmt`** — Run `cargo fmt --all` to fix all formatting violations. This is a hard requirement per CLAUDE.md.
2. **UTF-8 panic in `browser.rs:46`** — Byte-index slicing on user-facing string data is a runtime crash risk. Must use a char-boundary-safe truncation.
3. **Workspace compilation failure** — `vox-daemon/src/main.rs` must handle the new `Command::Summarize` and `Command::Export` variants to unblock workspace-wide testing and CI.

Non-blocking issues to address (recommended):

4. Tray icon swap not wired up (system_tray.rs:549-556)
5. Notifications not clickable (desktop.rs)
6. Speaker color-coding not applied in transcript viewer (app.rs:829)

---

## QA Re-verification: Integration Gap Fixes

**Date:** 2026-03-11
**Crate(s):** vox-daemon, vox-gui, vox-transcribe, vox-notify
**Reviewer:** qa-reviewer

### Summary

Re-verification of 12 previously identified integration gaps. The review checks
whether each gap has been properly resolved in the current codebase.

### Build Verification

- `cargo check -p vox-daemon`: **PASS** (compiles without features)
- `cargo check -p vox-gui`: **PASS**
- `cargo check -p vox-transcribe`: **PASS**
- `cargo test --workspace`: **PASS** (all tests pass)

### Gap Status

| # | Gap | Status | Evidence |
|---|-----|--------|----------|
| 1 | Recording uses `MockAudioSource` -- should use `PipeWireSource` with `pw` feature | **FIXED** | `vox-daemon/src/recording.rs:89-93` uses `#[cfg(feature = "pw")]` to select `PipeWireSource::new()` and falls back to `MockAudioSource` only when the feature is absent. |
| 2 | Transcription uses `StubTranscriber` -- should use `WhisperTranscriber` with `whisper` feature | **FIXED** | `vox-daemon/src/recording.rs:190-202` uses `#[cfg(feature = "whisper")]` to construct `WhisperTranscriber::from_config()` and falls back to `StubTranscriber` only without the feature. |
| 3 | Notifications use `StubNotifier` -- should use `DesktopNotifier` | **FIXED** | `vox-daemon/src/daemon.rs:37-39` unconditionally creates `DesktopNotifier::new(config.notifications.clone())`. `StubNotifier` is no longer used in production daemon code. |
| 4 | Auto-summarization never triggers -- should trigger after recording when config flag is set | **FIXED** | `vox-daemon/src/recording.rs:55-67` checks `config.summarization.auto_summarize && !session.transcript.is_empty()` and calls `auto_summarize()`, which creates a summarizer via `vox_summarize::create_summarizer()` and stores the result back to the session. The session is re-saved after summarization. |
| 5 | Recording blocks tray event loop -- should be spawned as async task | **FIXED** | `vox-daemon/src/daemon.rs:146-159` spawns the recording via `tokio::spawn(async move { recording::record_session(...).await })` and stores the `JoinHandle` in a `RecordingTask` struct. The tray event loop continues polling independently at `daemon.rs:74`. |
| 6 | Speaker name edits not persisted -- should save back to disk | **FIXED** | `crates/vox-gui/src/app.rs:471-495` handles `Message::SpeakerNameEdited` by updating the in-memory session, cloning it, and performing an async `JsonFileStore::save()` call. The `SpeakerNamesSaved` result message logs success or failure. |
| 7 | Markdown export does not save to file -- should write .md file | **FIXED** | `crates/vox-gui/src/app.rs:400-425` handles `Message::ExportSelectedSession` by calling `store.export_markdown(id)` and then writing the result to `{data_dir}/{id}.md` via `std::fs::write()`. The CLI `export` command at `vox-daemon/src/main.rs:276-290` still only prints to stdout (no file write), which is appropriate for a CLI tool. |
| 8 | Speaker colors not applied -- should use `speaker_color()` in transcript view | **FIXED** | `crates/vox-gui/src/app.rs:865` applies `.color(speaker_color)` where `speaker_color` is obtained from `vox_theme::speaker_color(&seg.speaker)` at line 865. The `speaker_color()` function at `crates/vox-gui/src/theme.rs:68-74` maps `"You"` to blue, remote/speaker labels to green, and others to grey. |
| 9 | Stop/Pause recording -- should have a stop mechanism | **FIXED (stop) / OPEN (pause)** | Stop: `RecordingTask` struct at `daemon.rs:15-22` holds an `Arc<AtomicBool>` stop flag. `TrayEvent::StopRecording` at `daemon.rs:162-185` sets the flag and awaits the task. The recording loop at `recording.rs:110-115` polls the flag every iteration. Pause: `TrayEvent::PauseRecording` at `daemon.rs:187-189` logs "not yet implemented" and does nothing. This is a known gap. |
| 10 | Model download on first use -- should auto-download from Hugging Face | **FIXED** | `crates/vox-transcribe/src/model.rs:113-129` implements `resolve_model_path()` which checks `$XDG_CACHE_HOME/vox-daemon/models/ggml-{size}.bin` and calls `download_model()` if the file is absent. `download_model()` at lines 169-303 performs a blocking HTTP GET from `huggingface.co/ggerganov/whisper.cpp`, streams to a temporary `.bin.tmp` file, reports progress via `tracing::info` every 50 MiB, and renames atomically on success. Comprehensive tests exist (lines 316-487). |
| 11 | Raw audio retention -- check if implemented | **OPEN** | The `retain_audio` config field exists in `StorageConfig` (`crates/vox-core/src/config.rs:178`) and the GUI has a toggle for it (`crates/vox-gui/src/app.rs:679`). However, the recording pipeline at `vox-daemon/src/recording.rs` never reads this flag and never saves raw audio data to disk. Audio samples are consumed during transcription and then dropped. The feature is config-plumbed but not functionally implemented. |
| 12 | Clickable notifications -- check if implemented | **OPEN** | `crates/vox-notify/src/desktop.rs` sends fire-and-forget notifications via `Notification::new().show()`. No `.action("default", "Open")` call is made, no `NotificationHandle` is retained, and no D-Bus callback is registered. Clicking a notification performs no application-specific action. This was previously identified as a gap and remains unaddressed. |

### New Issues Introduced

1. **[SEVERITY: LOW]** The `capture_audio()` function at `recording.rs:86` does not pass mic/app override arguments through to the audio source. The `_mic_override` and `_app_override` parameters of `record_session` (line 26) are prefixed with underscores and silently ignored. When a user runs `vox-daemon record --mic <id> --app <id>`, the overrides have no effect.
   - Location: `/workspace/vox-daemon/src/recording.rs:23-29`
   - Fix suggestion: Pass the override values to `PipeWireSource::new()` or use them in stream selection.

2. **[SEVERITY: LOW]** `PauseRecording` event handler at `daemon.rs:187-189` does nothing. While marked as "not yet implemented", it silently swallows the event without any user-visible feedback (no notification, no tray status update). A user clicking "Pause" in the tray menu would see no response.
   - Location: `/workspace/vox-daemon/src/daemon.rs:187-189`

### Verdict

**8 of 12 gaps are FIXED.** 2 gaps are partially fixed (stop works, pause does not). 2 gaps remain OPEN:

- **Raw audio retention** (gap 11): config plumbing exists but no audio data is ever saved to disk.
- **Clickable notifications** (gap 12): notifications are sent but not interactive.

Additionally, 2 low-severity new issues were found (mic/app overrides ignored; pause is a no-op).

None of the open gaps cause compilation failures or test regressions. They represent missing functionality rather than broken functionality.
