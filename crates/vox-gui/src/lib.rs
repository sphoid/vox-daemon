#![warn(clippy::all, clippy::pedantic)]
//! GUI layer for Vox Daemon — settings window and transcript browser.
//!
//! # Feature flags
//!
//! - `ui` — enables the full `iced`-based UI. Requires a GPU-capable display server
//!   (Wayland with wgpu support). Gate-free modules (data models, search) are always
//!   compiled and can be used independently.
//!
//! # Module overview
//!
//! - [`settings`] — `SettingsModel` that mirrors `AppConfig` with UI-friendly
//!   representations and round-trip conversion helpers.
//! - [`browser`] — `SessionListEntry` for the transcript list view and helpers for
//!   building the entry list from `Session` data.
//! - [`search`] — full-text search across all transcript segments.
//! - [`error`] — crate-level error type.
//!
//! The `app` and `theme` modules are only compiled when the `ui` feature is
//! active, because they depend directly on `iced`.

pub mod browser;
pub mod error;
pub mod search;
pub mod settings;

#[cfg(feature = "ui")]
pub mod app;
#[cfg(feature = "ui")]
pub mod theme;

pub use browser::SessionListEntry;
pub use error::GuiError;
pub use search::{SearchResult, search_transcripts};
pub use settings::SettingsModel;
