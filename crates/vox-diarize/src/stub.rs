//! Stub diarizer that passes segments through unchanged.
//!
//! Always compiled (no feature gate). Used when diarization is disabled
//! or as a fallback when the ONNX feature is not available.

use vox_core::session::{SpeakerMapping, SpeakerSource};

use crate::error::DiarizeError;
use crate::traits::{DiarizationRequest, DiarizationResult, Diarizer};

/// A no-op [`Diarizer`] that returns segments with their original labels.
#[derive(Debug, Default)]
pub struct StubDiarizer;

impl StubDiarizer {
    /// Create a new stub diarizer.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Diarizer for StubDiarizer {
    fn diarize(&self, request: &DiarizationRequest<'_>) -> Result<DiarizationResult, DiarizeError> {
        Ok(DiarizationResult {
            segments: request.segments.to_vec(),
            speakers: vec![SpeakerMapping {
                id: "Speaker".to_owned(),
                friendly_name: "Speaker".to_owned(),
                source: SpeakerSource::Unknown,
            }],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vox_core::session::TranscriptSegment;

    #[test]
    fn stub_passes_through_segments() {
        let segments = vec![
            TranscriptSegment {
                start_time: 0.0,
                end_time: 5.0,
                speaker: "Speaker".to_owned(),
                text: "Hello".to_owned(),
            },
            TranscriptSegment {
                start_time: 5.0,
                end_time: 10.0,
                speaker: "Speaker".to_owned(),
                text: "World".to_owned(),
            },
        ];

        let diarizer = StubDiarizer::new();
        let request = DiarizationRequest {
            segments: &segments,
            audio: &[0.0; 160_000],
            enrollment: None,
        };

        let result = diarizer.diarize(&request).expect("stub should not fail");
        assert_eq!(result.segments.len(), 2);
        assert_eq!(result.segments[0].text, "Hello");
        assert_eq!(result.segments[1].text, "World");
        assert_eq!(result.speakers.len(), 1);
        assert_eq!(result.speakers[0].id, "Speaker");
    }

    #[test]
    fn stub_handles_empty_input() {
        let diarizer = StubDiarizer::new();
        let request = DiarizationRequest {
            segments: &[],
            audio: &[],
            enrollment: None,
        };

        let result = diarizer.diarize(&request).expect("stub should not fail");
        assert!(result.segments.is_empty());
    }
}
