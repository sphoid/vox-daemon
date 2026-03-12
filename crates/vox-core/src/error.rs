//! Error types for each domain in the Vox Daemon workspace.
//!
//! Each crate defines its own error enum using `thiserror`.
//! This module defines the core error types shared across crates.

use thiserror::Error;

/// Errors related to configuration loading and validation.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Failed to read the configuration file from disk.
    #[error("failed to read config file: {0}")]
    ReadFile(#[from] std::io::Error),

    /// Failed to parse the TOML configuration.
    #[error("failed to parse config: {0}")]
    Parse(#[from] toml::de::Error),

    /// Failed to serialize configuration to TOML.
    #[error("failed to serialize config: {0}")]
    Serialize(#[from] toml::ser::Error),

    /// A configuration value is invalid.
    #[error("invalid config value: {0}")]
    InvalidValue(String),
}

/// Errors related to audio capture.
#[derive(Debug, Error)]
pub enum CaptureError {
    /// Failed to connect to the `PipeWire` daemon.
    #[error("failed to connect to PipeWire: {0}")]
    Connection(String),

    /// A stream-level error occurred during capture.
    #[error("stream error: {0}")]
    Stream(String),

    /// The requested audio source was not found.
    #[error("audio source not found: {0}")]
    SourceNotFound(String),

    /// An error occurred during audio format conversion or resampling.
    #[error("format error: {0}")]
    Format(String),
}

/// Errors related to transcription.
#[derive(Debug, Error)]
pub enum TranscribeError {
    /// Failed to load the Whisper model.
    #[error("failed to load model: {0}")]
    ModelLoad(String),

    /// An error occurred during transcription inference.
    #[error("transcription failed: {0}")]
    Inference(String),

    /// The provided audio data is invalid.
    #[error("invalid audio data: {0}")]
    InvalidAudio(String),

    /// Failed to download a Whisper model file.
    #[error("model download failed: {0}")]
    ModelDownload(String),
}

/// Errors related to session storage.
#[derive(Debug, Error)]
pub enum StorageError {
    /// An I/O error occurred during file operations.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Failed to serialize or deserialize JSON.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// The requested session was not found.
    #[error("session not found: {0}")]
    NotFound(String),
}
