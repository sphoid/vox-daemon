//! Error types for the `vox-export` crate.

use thiserror::Error;

/// Errors that can occur while exporting a session to an external target.
#[derive(Debug, Error)]
pub enum ExportError {
    /// An unknown target id was requested.
    #[error("unknown export target '{0}'")]
    UnknownTarget(String),

    /// The target is not configured (e.g. `enabled = false` or missing fields).
    #[error("export target '{0}' is not configured")]
    NotConfigured(String),

    /// Authentication against the remote service failed.
    #[error("authentication failed: {0}")]
    Auth(String),

    /// A required configuration field is missing or invalid.
    #[error("configuration error: {0}")]
    Config(String),

    /// An HTTP request to the remote service failed.
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    /// The remote API returned a non-success status code.
    #[error("API returned status {status}: {body}")]
    ApiError {
        /// HTTP status code.
        status: u16,
        /// Response body text (truncated / sanitized).
        body: String,
    },

    /// The API response could not be parsed as expected.
    #[error("failed to parse API response: {reason}")]
    ParseError {
        /// Human-readable explanation of what failed.
        reason: String,
    },

    /// The named destination (workspace / folder / parent doc) was not found.
    #[error("destination not found: {0}")]
    DestinationNotFound(String),

    /// A realtime (WebSocket / Socket.IO) transport error.
    #[error("realtime transport error: {0}")]
    Transport(String),

    /// JSON serialization or deserialization failed.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}
