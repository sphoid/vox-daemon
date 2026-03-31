#![warn(clippy::all, clippy::pedantic)]

//! Session storage, JSON persistence, and export for Vox Daemon.
//!
//! This crate implements the [`SessionStore`] trait and provides a
//! [`JsonFileStore`] that persists sessions as JSON files under the XDG
//! data directory (`$XDG_DATA_HOME/vox-daemon/sessions/`).

pub mod json_export;
pub mod markdown;
pub mod store;
pub mod text;

pub use markdown::RenderOptions;
pub use store::{JsonFileStore, SessionStore};

use vox_core::session::Session;

/// Render a session export in the given format.
///
/// `format` should be one of `"markdown"`, `"json"`, or `"text"`.
///
/// # Errors
///
/// Returns an error string if the format is unknown or serialization fails.
pub fn render_export(
    session: &Session,
    format: &str,
    options: &RenderOptions,
) -> Result<String, String> {
    match format {
        "markdown" => Ok(markdown::render_with_options(session, options)),
        "text" => Ok(text::render(session, options)),
        "json" => json_export::render(session, options).map_err(|e| e.to_string()),
        other => Err(format!("unknown export format: {other}")),
    }
}
