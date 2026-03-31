//! Core trait and request/response types for the diarization API.

use vox_core::session::{SpeakerMapping, TranscriptSegment};

use crate::error::DiarizeError;

/// A request to diarize (assign speaker labels to) transcript segments.
pub struct DiarizationRequest<'a> {
    /// Transcript segments to relabel with speaker identities.
    pub segments: &'a [TranscriptSegment],

    /// Merged 16 kHz mono f32 PCM audio corresponding to the segments.
    pub audio: &'a [f32],

    /// Optional enrollment audio (mic-only, first N seconds) used to
    /// identify the local user's voice.  When `Some`, the diarizer will
    /// attempt to label one cluster as `"You"`.
    pub enrollment: Option<&'a [f32]>,
}

/// The result of a diarization pass.
pub struct DiarizationResult {
    /// Transcript segments with updated `speaker` labels.
    pub segments: Vec<TranscriptSegment>,

    /// Speaker mappings discovered during diarization.
    pub speakers: Vec<SpeakerMapping>,
}

/// Trait for all speaker diarization backends.
///
/// Implementors must be [`Send`] + [`Sync`] so they can be shared across
/// Tokio tasks and OS threads.
pub trait Diarizer: Send + Sync {
    /// Diarize the given segments, assigning speaker labels based on
    /// voice similarity.
    ///
    /// # Errors
    ///
    /// Returns [`DiarizeError`] if embedding extraction, clustering, or
    /// model inference fails.
    fn diarize(&self, request: &DiarizationRequest<'_>) -> Result<DiarizationResult, DiarizeError>;
}
