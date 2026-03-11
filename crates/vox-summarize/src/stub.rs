//! [`StubSummarizer`] — test-friendly summarizer that returns a fixed summary.

use async_trait::async_trait;
use chrono::Utc;
use vox_core::session::{Summary, TranscriptSegment};

use crate::{SummarizeError, Summarizer};

/// A [`Summarizer`] that returns a fixed summary without calling any LLM.
///
/// Useful in tests, CI environments, and for offline development when no
/// LLM server is available.
#[derive(Debug, Default)]
pub struct StubSummarizer;

impl StubSummarizer {
    /// Create a new `StubSummarizer`.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Summarizer for StubSummarizer {
    async fn summarize(&self, transcript: &[TranscriptSegment]) -> Result<Summary, SummarizeError> {
        if transcript.is_empty() {
            return Err(SummarizeError::EmptyTranscript);
        }

        tracing::info!(
            segments = transcript.len(),
            "[StubSummarizer] generating stub summary"
        );

        Ok(Summary {
            generated_at: Utc::now(),
            backend: "stub".to_owned(),
            model: "none".to_owned(),
            overview: "This is a stub summary generated for testing purposes.".to_owned(),
            key_points: vec!["Stub key point".to_owned()],
            action_items: vec![],
            decisions: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_segment(text: &str) -> TranscriptSegment {
        TranscriptSegment {
            start_time: 0.0,
            end_time: 5.0,
            speaker: "You".to_owned(),
            text: text.to_owned(),
        }
    }

    #[tokio::test]
    async fn stub_summarizer_returns_summary() {
        let summarizer = StubSummarizer::new();
        let segments = vec![make_segment("Hello world")];
        let result = summarizer.summarize(&segments).await;
        assert!(result.is_ok());
        let summary = result.expect("should succeed");
        assert_eq!(summary.backend, "stub");
        assert!(!summary.overview.is_empty());
    }

    #[tokio::test]
    async fn stub_summarizer_empty_transcript_errors() {
        let summarizer = StubSummarizer::new();
        let result = summarizer.summarize(&[]).await;
        assert!(matches!(result, Err(SummarizeError::EmptyTranscript)));
    }

    #[tokio::test]
    async fn stub_summarizer_is_object_safe() {
        let summarizer: Box<dyn Summarizer> = Box::new(StubSummarizer::new());
        let segments = vec![make_segment("Test")];
        assert!(summarizer.summarize(&segments).await.is_ok());
    }
}
