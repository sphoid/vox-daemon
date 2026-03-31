//! Core trait and request/response types for the transcription API.

use vox_core::error::TranscribeError;
use vox_core::session::TranscriptSegment;

/// The role of the audio stream being transcribed.
///
/// - [`Microphone`](Self::Microphone) / [`Application`](Self::Application) are
///   used for per-stream transcription (legacy stream-based diarization).
/// - [`Merged`](Self::Merged) is used when both streams have been mixed into a
///   single audio buffer for unified transcription (the default since the
///   stream-based approach proved unreliable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSourceRole {
    /// The local user's microphone.
    Microphone,
    /// Remote participant audio captured from an application stream.
    Application,
    /// Merged audio from all streams (undiarized).
    Merged,
}

impl AudioSourceRole {
    /// Returns the speaker label string used inside a [`TranscriptSegment`].
    #[must_use]
    pub fn speaker_label(self) -> &'static str {
        match self {
            Self::Microphone => "You",
            Self::Application => "Remote",
            Self::Merged => "Speaker",
        }
    }
}

/// A request to transcribe a single chunk of audio.
///
/// Audio must be 32-bit float PCM, 16 kHz, mono (one sample per frame).
/// The minimum meaningful chunk length is implementation-defined but
/// whisper.cpp works best with at least ~1 second of audio.
#[derive(Debug, Clone)]
pub struct TranscriptionRequest {
    /// Raw audio samples: `f32` PCM at 16 kHz mono.
    pub audio: Vec<f32>,

    /// The source role of this audio chunk, used to assign a speaker label.
    pub source: AudioSourceRole,

    /// Optional offset (in seconds) to add to all segment timestamps so that
    /// multiple consecutive chunks can be stitched into a single timeline.
    pub time_offset_secs: f64,
}

impl TranscriptionRequest {
    /// Creates a new [`TranscriptionRequest`] with no time offset.
    ///
    /// Use [`TranscriptionRequest::with_offset`] when concatenating multiple
    /// chunks from the same recording session.
    #[must_use]
    pub fn new(audio: Vec<f32>, source: AudioSourceRole) -> Self {
        Self {
            audio,
            source,
            time_offset_secs: 0.0,
        }
    }

    /// Creates a [`TranscriptionRequest`] with an explicit time offset.
    ///
    /// The offset is added to the `start_time` and `end_time` of every
    /// resulting [`TranscriptSegment`], enabling correct timestamps when
    /// processing audio in sequential chunks.
    #[must_use]
    pub fn with_offset(audio: Vec<f32>, source: AudioSourceRole, time_offset_secs: f64) -> Self {
        Self {
            audio,
            source,
            time_offset_secs,
        }
    }

    /// Returns `true` if the audio buffer contains no samples.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.audio.is_empty()
    }

    /// Returns the approximate duration of the audio chunk in seconds.
    ///
    /// Assumes a sample rate of 16 000 Hz.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn duration_secs(&self) -> f64 {
        self.audio.len() as f64 / 16_000.0
    }
}

/// The result of a transcription request.
#[derive(Debug, Clone)]
pub struct TranscriptionResult {
    /// Timestamped segments produced by the transcriber.
    ///
    /// May be empty if no speech was detected in the audio chunk.
    pub segments: Vec<TranscriptSegment>,
}

impl TranscriptionResult {
    /// Creates a new result wrapping the provided segments.
    #[must_use]
    pub fn new(segments: Vec<TranscriptSegment>) -> Self {
        Self { segments }
    }

    /// Returns `true` if no speech segments were detected.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }
}

/// Trait for all transcription backends.
///
/// Implementors must be [`Send`] + [`Sync`] so they can be shared across
/// Tokio tasks and OS threads.
///
/// # Speaker labelling
///
/// The transcriber is responsible for assigning speaker labels on each
/// [`TranscriptSegment`] based on the [`AudioSourceRole`] supplied in the
/// [`TranscriptionRequest`].
pub trait Transcriber: Send + Sync {
    /// Transcribes a single chunk of audio and returns timestamped segments.
    ///
    /// # Errors
    ///
    /// Returns [`TranscribeError::InvalidAudio`] if the audio data is empty or
    /// otherwise malformed, [`TranscribeError::Inference`] if the backend
    /// encounters a runtime error during processing, or
    /// [`TranscribeError::ModelLoad`] if the model has not been initialised.
    fn transcribe(
        &self,
        request: &TranscriptionRequest,
    ) -> Result<TranscriptionResult, TranscribeError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_speaker_label_microphone() {
        assert_eq!(AudioSourceRole::Microphone.speaker_label(), "You");
    }

    #[test]
    fn test_speaker_label_application() {
        assert_eq!(AudioSourceRole::Application.speaker_label(), "Remote");
    }

    #[test]
    fn test_speaker_label_merged() {
        assert_eq!(AudioSourceRole::Merged.speaker_label(), "Speaker");
    }

    #[test]
    fn test_request_duration_one_second() {
        let audio = vec![0.0_f32; 16_000];
        let req = TranscriptionRequest::new(audio, AudioSourceRole::Microphone);
        let duration = req.duration_secs();
        assert!((duration - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_request_is_empty_when_no_samples() {
        let req = TranscriptionRequest::new(vec![], AudioSourceRole::Microphone);
        assert!(req.is_empty());
    }

    #[test]
    fn test_request_with_offset_stores_offset() {
        let req =
            TranscriptionRequest::with_offset(vec![0.0; 8_000], AudioSourceRole::Application, 30.5);
        assert!((req.time_offset_secs - 30.5).abs() < f64::EPSILON);
        assert_eq!(req.source, AudioSourceRole::Application);
    }

    #[test]
    fn test_result_is_empty_when_no_segments() {
        let result = TranscriptionResult::new(vec![]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_result_not_empty_with_segments() {
        let segment = TranscriptSegment {
            start_time: 0.0,
            end_time: 1.0,
            speaker: "You".to_owned(),
            text: "Hello".to_owned(),
        };
        let result = TranscriptionResult::new(vec![segment]);
        assert!(!result.is_empty());
    }
}
