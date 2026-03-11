//! Error types for the `vox-capture` crate.
//!
//! [`CaptureError`] is the single error type exposed by this crate. The
//! variants cover every failure mode that can occur in the capture layer:
//! `PipeWire` connection issues, stream lifecycle problems, format/resampling
//! errors, and channel communication failures.

use thiserror::Error;

/// All errors that can be produced by the `vox-capture` crate.
#[derive(Debug, Error)]
pub enum CaptureError {
    /// Failed to connect to the `PipeWire` daemon.
    #[error("failed to connect to PipeWire: {0}")]
    Connection(String),

    /// A stream-level error occurred during capture.
    #[error("stream error: {0}")]
    Stream(String),

    /// The requested audio source or node was not found.
    #[error("audio source not found: {0}")]
    SourceNotFound(String),

    /// An error occurred during audio format conversion or resampling.
    #[error("audio format error: {0}")]
    Format(String),

    /// The `PipeWire` event loop thread terminated unexpectedly.
    #[error("PipeWire thread panicked or exited: {0}")]
    ThreadPanic(String),

    /// A channel send/receive operation failed (receiver dropped).
    #[error("internal channel error: {0}")]
    Channel(String),

    /// Capture was already started (or not yet started for stop).
    #[error("invalid state: {0}")]
    InvalidState(String),
}
