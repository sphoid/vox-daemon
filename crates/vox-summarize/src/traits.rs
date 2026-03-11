//! Core trait definitions for the summarization subsystem.

use async_trait::async_trait;
use vox_core::session::{Summary, TranscriptSegment};

use crate::error::SummarizeError;

/// Trait for LLM-backed summarization of a call transcript.
///
/// Implementations must be `Send + Sync` to allow use across async task
/// boundaries (e.g., Tokio's multi-threaded executor).
#[async_trait]
pub trait Summarizer: Send + Sync {
    /// Summarize a slice of transcript segments.
    ///
    /// # Errors
    ///
    /// Returns [`SummarizeError`] if the transcript is empty, if the
    /// underlying LLM call fails, or if the response cannot be parsed into
    /// a structured [`Summary`].
    async fn summarize(&self, transcript: &[TranscriptSegment]) -> Result<Summary, SummarizeError>;
}
