# Contributing to Vox Daemon

Thank you for your interest in contributing! This document covers the development workflow, coding standards, and how to submit changes.

## Getting Started

1. Fork and clone the repository
2. Install Rust 1.85.0+ via [rustup](https://rustup.rs)
3. Install system dependencies (see README.md)
4. Run `cargo test` to verify your setup

## Development Workflow

1. Create a feature branch from `main`
2. Make your changes
3. Ensure all checks pass:
   ```bash
   cargo fmt --check
   cargo clippy -- -D warnings
   cargo test
   ```
4. Commit with a conventional commit message
5. Open a pull request against `main`

## Coding Standards

### Rust Conventions

- Use `rustfmt` with default settings
- Use `clippy` with `#![warn(clippy::all, clippy::pedantic)]`
- All public types and functions must have doc comments (`///`)
- Use `#[must_use]` on functions whose return values should not be discarded
- Prefer `Result<T, E>` over panics — never use `.unwrap()` in library code
- Use `.expect("reason")` only in binary/test code where the invariant is documented

### Error Handling

- Library crates use `thiserror` with crate-specific error enums
- The binary crate (`vox-daemon`) uses `anyhow` for top-level error reporting
- Each crate defines its own error type in an `error.rs` module

### Module Organization

- One module per logical unit
- Keep files under 500 lines
- Keep `lib.rs` thin — use it only for re-exports and module declarations
- Place shared types in `vox-core`

### Commit Messages

Use [conventional commits](https://www.conventionalcommits.org/):

```
feat: add GPU backend selection to settings window
fix: handle empty transcript in markdown export
refactor: extract prompt builder into separate module
docs: add man page for vox-daemon
test: add integration tests for session storage
chore: update dependencies
```

## Architecture

The project is a Cargo workspace with 8 library crates and 1 binary crate. Each library crate exposes a trait-based API:

| Crate | Primary Trait | Purpose |
|-------|--------------|---------|
| `vox-capture` | `AudioSource` | PipeWire audio capture |
| `vox-transcribe` | `Transcriber` | Whisper speech-to-text |
| `vox-summarize` | `Summarizer` | LLM-powered summaries |
| `vox-storage` | `SessionStore` | JSON persistence |
| `vox-tray` | `Tray` | System tray icon |
| `vox-notify` | `Notifier` | Desktop notifications |

Feature flags gate native dependencies so the project compiles without PipeWire, Whisper, GTK, etc. Always test with the default feature set (no flags) before pushing.

## Testing

- Write unit tests in each module (`#[cfg(test)] mod tests { ... }`)
- Integration tests go in the workspace-level `tests/` directory
- Use mock/stub implementations for testing without hardware
- PipeWire integration tests are gated behind `#[cfg(feature = "integration")]`

## Pull Request Guidelines

- Keep PRs focused — one logical change per PR
- Include tests for new functionality
- Update doc comments for any changed public API
- Ensure CI passes before requesting review

## Reporting Issues

Use the GitHub issue templates:
- **Bug reports**: Include your environment details, logs (`-vv`), and reproduction steps
- **Feature requests**: Describe the problem and your proposed solution

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
