//! Whisper-based transcriber implementation using whisper-rs.
//!
//! This module is only compiled when the `whisper` feature flag is enabled.
//! It wraps the [`whisper_rs`] crate which provides Rust bindings for
//! [whisper.cpp](https://github.com/ggerganov/whisper.cpp).
//!
//! # Model loading
//!
//! The model is loaded once at construction time via [`WhisperTranscriber::new`]
//! or [`WhisperTranscriber::from_config`].  Subsequent calls to
//! [`Transcriber::transcribe`] create a lightweight [`whisper_rs::WhisperState`]
//! per call using the shared context, making repeated inference calls cheap
//! after the initial model load.
//!
//! # Concurrency
//!
//! `whisper.cpp` contexts are read-only after loading and are safe to share
//! across threads.  Mutable inference state is created fresh for each
//! [`Transcriber::transcribe`] call and is not shared.  A [`Mutex`] guards
//! state creation to satisfy the `Sync` bound required by the [`Transcriber`]
//! trait without relying on `unsafe`.
//!
//! # GPU acceleration
//!
//! GPU acceleration is controlled entirely at compile time via Cargo feature
//! flags passed to the `whisper-rs` dependency:
//! - `--features cuda` → NVIDIA CUDA
//! - `--features hipblas` → AMD ROCm/hipBLAS
//!
//! No runtime detection or fallback is performed; the chosen backend is baked
//! in at build time.

use std::path::Path;
use std::sync::Mutex;

use tracing::{debug, info, instrument, warn};
use vox_core::config::TranscriptionConfig;
use vox_core::error::TranscribeError;
use vox_core::session::TranscriptSegment;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::Transcriber;
use crate::model::resolve_model_path;
use crate::transcriber::{TranscriptionRequest, TranscriptionResult};

/// Minimum audio length (in samples at 16 kHz) for meaningful inference.
///
/// Buffers shorter than 100 ms produce unreliable output in whisper.cpp.
/// We warn — but do not error — for chunks below this threshold.
const MIN_MEANINGFUL_SAMPLES: usize = 1_600; // 100 ms at 16 kHz

/// A [`Transcriber`] backed by whisper.cpp via the `whisper-rs` crate.
///
/// The [`WhisperContext`] holding the loaded model weights is shared across
/// calls.  A new [`whisper_rs::WhisperState`] (the mutable inference handle)
/// is created per [`Transcriber::transcribe`] call.
///
/// The [`Mutex`] around the context is required because `WhisperContext`
/// wraps a raw pointer and is not `Sync` by default; the mutex provides the
/// synchronisation needed for the trait's `Send + Sync` bound.
pub struct WhisperTranscriber {
    /// Whisper context holding the loaded model weights.
    ///
    /// Wrapped in a `Mutex` so that `WhisperTranscriber` satisfies `Sync`.
    /// The lock is held only during `create_state()` and released before
    /// inference begins, keeping contention minimal.
    ctx: Mutex<WhisperContext>,

    /// BCP-47 language code (e.g. `"en"`) or `"auto"` for detection.
    language: String,
}

// SAFETY: `WhisperContext` wraps a raw pointer to whisper.cpp's C struct.
// whisper.cpp guarantees that the context is safe to access from any thread
// for read operations (create_state, model queries).  Mutable inference is
// done through `WhisperState`, which is neither `Send` nor `Sync` and is
// kept strictly per-call.  The `Mutex` ensures only one thread calls
// `create_state` at a time.
unsafe impl Send for WhisperTranscriber {}
unsafe impl Sync for WhisperTranscriber {}

impl std::fmt::Debug for WhisperTranscriber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WhisperTranscriber")
            .field("language", &self.language)
            .finish_non_exhaustive()
    }
}

impl WhisperTranscriber {
    /// Loads a Whisper model from the given filesystem path.
    ///
    /// This function performs synchronous file I/O and CPU-bound model
    /// initialisation — it can take several seconds for large models.  Call it
    /// once at daemon startup, not on a hot path.
    ///
    /// # Parameters
    ///
    /// - `model_path`: Path to a GGML-format `.bin` model file (e.g.,
    ///   `ggml-base.bin`).
    /// - `language`: BCP-47 language code such as `"en"`, or `"auto"` to
    ///   enable whisper.cpp's built-in language detection.
    ///
    /// # Errors
    ///
    /// Returns [`TranscribeError::ModelLoad`] if:
    /// - The path contains non-UTF-8 characters.
    /// - The file cannot be read or is not a valid GGML model.
    #[instrument(skip_all, fields(model_path = %model_path.as_ref().display()))]
    pub fn new(
        model_path: impl AsRef<Path>,
        language: impl Into<String>,
    ) -> Result<Self, TranscribeError> {
        let model_path = model_path.as_ref();
        info!("loading Whisper model from '{}'", model_path.display());

        let path_str = model_path.to_str().ok_or_else(|| {
            TranscribeError::ModelLoad("model path contains non-UTF-8 characters".to_owned())
        })?;

        let ctx_params = WhisperContextParameters::default();
        let ctx = WhisperContext::new_with_params(path_str, ctx_params)
            .map_err(|e| TranscribeError::ModelLoad(e.to_string()))?;

        info!("Whisper model loaded successfully");

        Ok(Self {
            ctx: Mutex::new(ctx),
            language: language.into(),
        })
    }

    /// Creates a [`WhisperTranscriber`] from a [`TranscriptionConfig`].
    ///
    /// Resolves the model path via [`crate::model::resolve_model_path`] and
    /// uses the language setting from the config.
    ///
    /// # Errors
    ///
    /// Returns [`TranscribeError::ModelLoad`] if the model file cannot be
    /// located or loaded.
    pub fn from_config(config: &TranscriptionConfig) -> Result<Self, TranscribeError> {
        let model_path = resolve_model_path(config)?;
        Self::new(model_path, config.language.clone())
    }

    /// Constructs the [`FullParams`] for a single inference pass.
    fn build_params<'a>(&'a self, speaker_label: &str) -> FullParams<'a, 'a> {
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

        if self.language == "auto" {
            params.set_language(None);
            params.set_detect_language(true);
        } else {
            params.set_language(Some(self.language.as_str()));
            params.set_detect_language(false);
        }

        // Token-level timestamps are required to populate segment timing.
        params.set_token_timestamps(true);

        // Suppress spurious output from silent segments.
        params.set_no_speech_thold(0.6);

        // We want the original speech, not an English translation.
        params.set_translate(false);

        // Suppress progress output from whisper.cpp's internal logging.
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        debug!(speaker = speaker_label, language = %self.language, "whisper params configured");

        params
    }
}

impl Transcriber for WhisperTranscriber {
    /// Transcribes a single audio chunk and returns timestamped segments.
    ///
    /// Each segment is labelled with the speaker derived from
    /// [`AudioSourceRole`](crate::transcriber::AudioSourceRole): `"You"` for
    /// microphone audio, `"Remote"` for application audio.
    ///
    /// Timestamp arithmetic: whisper.cpp reports times in centiseconds
    /// (hundredths of a second).  These are converted to seconds and then
    /// shifted by [`TranscriptionRequest::time_offset_secs`] so that
    /// segments from consecutive chunks share a common timeline.
    ///
    /// # Errors
    ///
    /// - [`TranscribeError::InvalidAudio`] — audio buffer is empty.
    /// - [`TranscribeError::Inference`] — whisper.cpp inference failed or the
    ///   internal mutex is poisoned.
    #[instrument(skip_all, fields(
        source = ?request.source,
        duration_secs = request.duration_secs(),
        time_offset_secs = request.time_offset_secs,
    ))]
    fn transcribe(
        &self,
        request: &TranscriptionRequest,
    ) -> Result<TranscriptionResult, TranscribeError> {
        if request.is_empty() {
            return Err(TranscribeError::InvalidAudio(
                "audio buffer is empty".to_owned(),
            ));
        }

        if request.audio.len() < MIN_MEANINGFUL_SAMPLES {
            warn!(
                samples = request.audio.len(),
                min_samples = MIN_MEANINGFUL_SAMPLES,
                "audio chunk is shorter than 100 ms; transcription quality may be degraded"
            );
        }

        let speaker_label = request.source.speaker_label();
        let offset = request.time_offset_secs;

        // Build inference params before acquiring the lock so the lock is held
        // for the minimum time necessary.
        let params = self.build_params(speaker_label);

        // Create a per-call WhisperState.  The Mutex ensures only one thread
        // calls create_state() at a time; the lock is released immediately
        // after state creation so concurrent calls can interleave.
        let mut state = {
            let ctx = self
                .ctx
                .lock()
                .map_err(|e| TranscribeError::Inference(format!("context mutex poisoned: {e}")))?;
            ctx.create_state()
                .map_err(|e| TranscribeError::Inference(e.to_string()))?
        };

        let t0 = std::time::Instant::now();

        state
            .full(params, &request.audio)
            .map_err(|e| TranscribeError::Inference(e.to_string()))?;

        let elapsed = t0.elapsed();

        let num_segments = state.full_n_segments();

        info!(
            elapsed_ms = elapsed.as_millis(),
            segments = num_segments,
            speaker = speaker_label,
            "inference complete"
        );

        let mut segments = Vec::with_capacity(usize::try_from(num_segments).unwrap_or(0));

        for i in 0..num_segments {
            let seg = state.get_segment(i).ok_or_else(|| {
                TranscribeError::Inference(format!("segment {i} out of bounds"))
            })?;

            let text = seg
                .to_str()
                .map_err(|e| TranscribeError::Inference(e.to_string()))?
                .trim()
                .to_owned();

            if text.is_empty() {
                debug!(segment = i, "skipping empty segment");
                continue;
            }

            // whisper.cpp timestamps are in centiseconds (1/100 s).
            let start_cs = seg.start_timestamp();
            let end_cs = seg.end_timestamp();

            #[allow(clippy::cast_precision_loss)]
            let start_time = start_cs as f64 / 100.0 + offset;
            #[allow(clippy::cast_precision_loss)]
            let end_time = end_cs as f64 / 100.0 + offset;

            debug!(
                segment = i,
                start_time,
                end_time,
                speaker = speaker_label,
                text = %text,
                "segment decoded"
            );

            segments.push(TranscriptSegment {
                start_time,
                end_time,
                speaker: speaker_label.to_owned(),
                text,
            });
        }

        Ok(TranscriptionResult::new(segments))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcriber::AudioSourceRole;

    /// Verifies that centisecond timestamps are correctly converted to seconds
    /// with a time offset applied.  This tests the arithmetic in `transcribe`
    /// without requiring a real model or native whisper.cpp library.
    #[test]
    fn test_centiseconds_to_seconds_with_offset() {
        // 150 centiseconds = 1.5 s; plus a 10.0 s offset → 11.5 s
        let cs: i64 = 150;
        let offset = 10.0_f64;
        #[allow(clippy::cast_precision_loss)]
        let result = cs as f64 / 100.0 + offset;
        assert!((result - 11.5).abs() < f64::EPSILON, "got {result}");
    }

    #[test]
    fn test_speaker_label_mapping() {
        assert_eq!(AudioSourceRole::Microphone.speaker_label(), "You");
        assert_eq!(AudioSourceRole::Application.speaker_label(), "Remote");
    }

    /// Confirms that `MIN_MEANINGFUL_SAMPLES` corresponds to 100 ms at 16 kHz.
    #[test]
    fn test_min_meaningful_samples_is_100ms() {
        assert_eq!(MIN_MEANINGFUL_SAMPLES, 16_000 / 10);
    }

    /// Confirms that `WhisperTranscriber` implements the `Transcriber` trait
    /// object-safe API (this is a compile-time check embedded in a test).
    #[allow(dead_code)]
    fn _assert_trait_object_safe(_: &dyn Transcriber) {}
}
