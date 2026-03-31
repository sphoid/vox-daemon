//! Error types for the diarization crate.

/// Errors that can occur during speaker diarization.
#[derive(Debug, thiserror::Error)]
pub enum DiarizeError {
    /// Failed to load the speaker embedding model.
    #[error("model load error: {0}")]
    ModelLoad(String),

    /// Failed to download the speaker embedding model.
    #[error("model download error: {0}")]
    ModelDownload(String),

    /// An error occurred during inference (embedding extraction).
    #[error("inference error: {0}")]
    Inference(String),

    /// The audio data provided is invalid or too short.
    #[error("invalid audio: {0}")]
    InvalidAudio(String),
}
