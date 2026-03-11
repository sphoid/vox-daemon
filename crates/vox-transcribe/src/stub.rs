//! No-op transcriber used when the `whisper` feature is disabled.
//!
//! [`StubTranscriber`] always returns an empty [`TranscriptionResult`] and
//! emits a warning-level log message so callers are aware that no real
//! inference is happening.  This makes it possible to compile and test the
//! rest of the workspace without the native whisper.cpp libraries.

use vox_core::error::TranscribeError;

use crate::Transcriber;
use crate::transcriber::{TranscriptionRequest, TranscriptionResult};

/// A no-op [`Transcriber`] that returns empty results without performing any
/// speech recognition.
///
/// This implementation is always available regardless of feature flags.  It is
/// intended for use in development, testing, and builds where the whisper.cpp
/// native libraries are not present.
///
/// When the `whisper` feature is enabled, prefer [`WhisperTranscriber`] instead.
///
/// [`WhisperTranscriber`]: crate::WhisperTranscriber
#[derive(Debug, Default, Clone)]
pub struct StubTranscriber;

impl StubTranscriber {
    /// Creates a new [`StubTranscriber`].
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Transcriber for StubTranscriber {
    /// Always returns an empty [`TranscriptionResult`].
    ///
    /// Emits a `tracing::warn` message on every call to signal that no real
    /// inference is taking place.
    ///
    /// # Errors
    ///
    /// Returns [`TranscribeError::InvalidAudio`] if the audio buffer is empty.
    fn transcribe(
        &self,
        request: &TranscriptionRequest,
    ) -> Result<TranscriptionResult, TranscribeError> {
        if request.is_empty() {
            return Err(TranscribeError::InvalidAudio(
                "audio buffer is empty".to_owned(),
            ));
        }

        tracing::warn!(
            duration_secs = request.duration_secs(),
            source = ?request.source,
            "StubTranscriber: returning empty result — enable the `whisper` feature for real inference"
        );

        Ok(TranscriptionResult::new(vec![]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcriber::AudioSourceRole;

    #[test]
    fn test_stub_returns_empty_result_for_valid_audio() {
        let transcriber = StubTranscriber::new();
        let request = TranscriptionRequest::new(vec![0.0_f32; 16_000], AudioSourceRole::Microphone);
        let result = transcriber.transcribe(&request).expect("should succeed");
        assert!(result.is_empty());
    }

    #[test]
    fn test_stub_errors_on_empty_audio() {
        let transcriber = StubTranscriber::new();
        let request = TranscriptionRequest::new(vec![], AudioSourceRole::Application);
        let err = transcriber.transcribe(&request).unwrap_err();
        match err {
            crate::TranscribeError::InvalidAudio(msg) => {
                assert!(msg.contains("empty"), "error message: {msg}");
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn test_stub_implements_transcriber_trait_object() {
        let transcriber: Box<dyn Transcriber> = Box::new(StubTranscriber::new());
        let request = TranscriptionRequest::new(vec![0.1_f32; 8_000], AudioSourceRole::Application);
        let result = transcriber.transcribe(&request).expect("should succeed");
        assert!(result.segments.is_empty());
    }
}
