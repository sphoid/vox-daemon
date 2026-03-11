# Vox Daemon — Project Instructions

## Project Overview

Vox Daemon is a Linux-native background service that captures video call audio via PipeWire, transcribes it with Whisper (GPU-accelerated), performs speaker diarization, and generates AI-powered post-call summaries. It is controlled from a system tray icon with a full settings window built in iced.

**Language:** Rust (Edition 2024, stable 1.94.0+)
**Target Platform:** Linux only (Wayland-first, PipeWire-native)
**Spec:** See `PRD.md` for the full product requirements document.

---

## Architecture

The project is structured as a Cargo workspace with the following crates:

```
vox-daemon/
├── Cargo.toml              # Workspace root
├── CLAUDE.md
├── PRD.md
├── crates/
│   ├── vox-core/           # Shared types, config, error handling, XDG paths
│   ├── vox-capture/        # PipeWire audio capture
│   ├── vox-transcribe/     # Whisper integration + diarization
│   ├── vox-summarize/      # LLM client (built-in, Ollama, OpenAI-compatible)
│   ├── vox-storage/        # JSON session storage, Markdown export
│   ├── vox-gui/            # iced settings window + transcript browser
│   ├── vox-tray/           # System tray icon + popup menu
│   └── vox-notify/         # Desktop notification wrapper
├── vox-daemon/             # Binary crate — daemon entrypoint
└── tests/                  # Integration tests
```

### Key Architectural Rules

- Each crate exposes a **trait-based API** for its primary functionality (e.g., `Transcriber`, `Summarizer`, `AudioSource`). This enables testing with mocks and future swappable implementations.
- **Async runtime:** Tokio for async I/O. PipeWire's event loop runs on a dedicated OS thread with channel-based communication back to the Tokio runtime.
- **Message passing:** Use `tokio::sync::mpsc` for async channels and `crossbeam-channel` for the PipeWire thread boundary.
- **Error handling:** Use `thiserror` for library crate errors. Each crate defines its own error enum. The binary crate uses `anyhow` for top-level error reporting.
- **Configuration:** TOML-based config at `$XDG_CONFIG_HOME/vox-daemon/config.toml`, deserialized with `serde` + `toml`.
- **Logging:** Use `tracing` with `tracing-subscriber` throughout all crates.

---

## Coding Standards

### Rust Conventions

- Use `rustfmt` with default settings. All code must pass `cargo fmt --check`.
- Use `clippy` with `#![warn(clippy::all, clippy::pedantic)]`. Address all warnings.
- Prefer `&str` over `String` in function parameters where ownership is not needed.
- Use `impl Into<String>` or generics for ergonomic API boundaries.
- All public types and functions must have doc comments (`///`).
- Use `#[must_use]` on functions that return values that should not be silently discarded.
- Prefer `Result<T, E>` over panics. Never use `.unwrap()` in library code — use `.expect("reason")` only in binary/test code where the invariant is documented.
- Write unit tests in each module (`#[cfg(test)] mod tests { ... }`).
- Integration tests go in the workspace-level `tests/` directory.

### File & Module Organization

- One module per logical unit. Avoid files longer than 500 lines.
- Keep `lib.rs` files thin — use them only for re-exports and module declarations.
- Place types shared across the workspace in `vox-core`.

### Commit Conventions

- Use conventional commits: `feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`.
- One logical change per commit.

---

## Development Phases

See `PRD.md` Section 10 for full details. Implement in this order:

### Phase 1: Core Audio Pipeline (Current)
1. Workspace setup with all crate stubs
2. `vox-core`: Config, error types, XDG paths, shared types
3. `vox-capture`: PipeWire connection, stream enumeration, audio capture
4. `vox-transcribe`: Whisper integration with GPU support
5. `vox-storage`: JSON session serialization
6. `vox-daemon`: CLI-based start/stop control
7. Basic speaker separation (mic vs. app = "You" vs. "Remote")

### Phase 2: System Tray & UI
8. `vox-tray`: System tray with start/stop/status
9. `vox-gui`: Settings window (iced)
10. `vox-gui`: Transcript browser (list, view, search)
11. `vox-notify`: Desktop notifications
12. `vox-storage`: Markdown export

### Phase 3: AI Summarization
13. `vox-summarize`: LLM provider trait
14. `vox-summarize`: Ollama / OpenAI-compatible HTTP client
15. `vox-summarize`: Prompt engineering + structured output
16. Integration: auto/manual summarization trigger

### Phase 4: Polish & Release
17. CI/CD pipeline
18. Packaging (AUR, .deb, Flatpak)
19. Documentation, README, man page
20. systemd service file

---

## Agent Orchestration Rules

This project uses specialist subagents for parallel development. The orchestrator (main session) follows these rules:

### When to Dispatch Subagents

**Parallel dispatch** when tasks span independent crates or domains:
- Audio capture work → `audio-specialist`
- Whisper/LLM/diarization work → `ai-specialist`
- GUI, tray, notifications → `gui-specialist`
- Validation against PRD → `qa-reviewer`

**Sequential dispatch** when tasks have dependencies:
- Always run `qa-reviewer` AFTER an implementation agent completes a feature
- Always run the implementing agent's fix cycle BEFORE moving to the next feature

### Dispatch Protocol

When spawning a subagent, always provide:
1. **Task scope:** Which crate(s) and file(s) to work on
2. **PRD reference:** Which section/acceptance criteria apply
3. **Context:** Any relevant decisions or constraints from prior work
4. **Output expectation:** What files should be created/modified, what tests should pass

### Output Tracking

Each subagent should write a brief completion summary to a tracking file:
- Implementation agents: append to `docs/progress.md` with what was implemented and any open questions
- QA agent: append to `docs/qa-log.md` with pass/fail per acceptance criterion and any issues found

### Cost Optimization

- Use `sonnet` model for implementation agents (fast, capable for focused coding tasks)
- Use `opus` model for `qa-reviewer` (needs deeper reasoning to validate against spec)
- The orchestrator (main session) runs on `opus` for planning and coordination

---

## Dependency Quick Reference

| Purpose | Crate | Version |
|---------|-------|---------|
| PipeWire | `pipewire` | 0.9.2 |
| Whisper | `whisper-rs` | 0.15.1 |
| GUI | `iced` | 0.14.0 |
| System Tray | `tray-icon` | 0.21.2 |
| Notifications | `notify-rust` | 4.11.6 |
| HTTP Client | `reqwest` | 0.12.x |
| Async Runtime | `tokio` | 1.x |
| Serialization | `serde`, `serde_json`, `toml` | 1.x, 1.x, 0.8.x |
| CLI | `clap` | 4.x |
| Logging | `tracing`, `tracing-subscriber` | 0.1.x, 0.3.x |
| Channels | `crossbeam-channel` | 0.5.x |
| XDG Dirs | `dirs` | 6.x |
| Timestamps | `chrono` | 0.4.x |
| Errors | `thiserror` (libs), `anyhow` (bin) | 2.x, 1.x |
| UUIDs | `uuid` | 1.x |
