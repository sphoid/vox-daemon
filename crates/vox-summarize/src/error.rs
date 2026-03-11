//! Error types for the `vox-summarize` crate.

use thiserror::Error;

/// Errors that can occur during summarization.
#[derive(Debug, Error)]
pub enum SummarizeError {
    /// The requested backend is not yet implemented.
    #[error("backend '{0}' is not implemented")]
    BackendNotImplemented(String),

    /// An unknown or unsupported backend name was specified in the config.
    #[error("unknown backend '{0}'; expected 'ollama', 'openai_compatible', or 'builtin'")]
    UnknownBackend(String),

    /// The transcript is empty; there is nothing to summarize.
    #[error("transcript is empty")]
    EmptyTranscript,

    /// An HTTP error occurred while calling the LLM API.
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    /// The LLM API returned a non-success status code.
    #[error("LLM API returned status {status}: {body}")]
    ApiError {
        /// HTTP status code.
        status: u16,
        /// Response body text.
        body: String,
    },

    /// The LLM API response could not be parsed.
    #[error("failed to parse LLM response: {reason}")]
    ParseError {
        /// Human-readable explanation.
        reason: String,
        /// The raw response text that failed to parse.
        raw: String,
    },

    /// The API response contained no choices or content.
    #[error("LLM API returned an empty response")]
    EmptyResponse,

    /// JSON serialization or deserialization failed.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// A required configuration field is missing or empty.
    #[error("configuration error: {0}")]
    Config(String),
}
