//! Speaker embedding extraction using ONNX Runtime.
//!
//! This module is only compiled when the `onnx` feature flag is enabled.
//! It wraps the [`ort`] crate to run an ECAPA-TDNN speaker embedding
//! model, producing a 192-dimensional embedding vector per audio segment.

use std::path::Path;

use tracing::{debug, info, instrument, warn};

use crate::error::DiarizeError;

/// Minimum segment duration (seconds) for reliable embedding extraction.
/// Segments shorter than this are skipped.
const MIN_SEGMENT_DURATION_SECS: f64 = 0.5;

/// Speaker embedding extractor backed by ONNX Runtime.
pub struct OnnxEmbedder {
    session: ort::Session,
}

impl OnnxEmbedder {
    /// Load an ONNX speaker embedding model from the given path.
    ///
    /// # Errors
    ///
    /// Returns [`DiarizeError::ModelLoad`] if the model file cannot be
    /// read or initialised.
    #[instrument(skip_all, fields(model_path = %model_path.as_ref().display()))]
    pub fn new(model_path: impl AsRef<Path>) -> Result<Self, DiarizeError> {
        let model_path = model_path.as_ref();
        info!(
            "loading ONNX speaker embedding model from {}",
            model_path.display()
        );

        let session = ort::Session::builder()
            .map_err(|e| DiarizeError::ModelLoad(format!("failed to create session builder: {e}")))?
            .with_intra_threads(1)
            .map_err(|e| DiarizeError::ModelLoad(format!("failed to set intra threads: {e}")))?
            .commit_from_file(model_path)
            .map_err(|e| DiarizeError::ModelLoad(format!("failed to load ONNX model: {e}")))?;

        info!("ONNX speaker embedding model loaded successfully");
        Ok(Self { session })
    }

    /// Extract a speaker embedding from a single audio segment.
    ///
    /// `audio` must be 16 kHz mono f32 PCM.
    ///
    /// # Errors
    ///
    /// Returns [`DiarizeError::Inference`] if the ONNX session fails.
    pub fn extract_embedding(&self, audio: &[f32]) -> Result<Vec<f32>, DiarizeError> {
        if audio.is_empty() {
            return Err(DiarizeError::InvalidAudio(
                "cannot extract embedding from empty audio".to_owned(),
            ));
        }

        // ECAPA-TDNN expects input shape [1, num_samples].
        let input_data: Vec<f32> = audio.to_vec();
        let shape = vec![1_usize, input_data.len()];

        let input_tensor = ort::value::Value::from_array(
            ndarray::Array::from_shape_vec(ndarray::IxDyn(&shape), input_data).map_err(|e| {
                DiarizeError::Inference(format!("failed to create input array: {e}"))
            })?,
        )
        .map_err(|e| DiarizeError::Inference(format!("failed to create input tensor: {e}")))?;

        let outputs = self
            .session
            .run(ort::inputs![input_tensor].map_err(|e| {
                DiarizeError::Inference(format!("failed to create session inputs: {e}"))
            })?)
            .map_err(|e| DiarizeError::Inference(format!("ONNX inference failed: {e}")))?;

        // The output is typically shape [1, embedding_dim].
        let output = outputs.first().ok_or_else(|| {
            DiarizeError::Inference("no output tensor from ONNX model".to_owned())
        })?;

        let output_tensor = output.1.try_extract_tensor::<f32>().map_err(|e| {
            DiarizeError::Inference(format!("failed to extract output tensor: {e}"))
        })?;

        let embedding: Vec<f32> = output_tensor.iter().copied().collect();
        debug!(
            embedding_dim = embedding.len(),
            "extracted speaker embedding"
        );

        Ok(embedding)
    }

    /// Extract embeddings for each qualifying segment by slicing the
    /// merged audio at segment time boundaries.
    ///
    /// Segments shorter than [`MIN_SEGMENT_DURATION_SECS`] are skipped
    /// (their embeddings are unreliable).
    ///
    /// Returns `(segment_index, embedding)` pairs.
    ///
    /// # Errors
    ///
    /// Returns [`DiarizeError::Inference`] if any embedding extraction fails.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    pub fn extract_all(
        &self,
        segments: &[vox_core::session::TranscriptSegment],
        audio: &[f32],
    ) -> Result<Vec<(usize, Vec<f32>)>, DiarizeError> {
        let mut results = Vec::new();
        let audio_len = audio.len();

        for (i, seg) in segments.iter().enumerate() {
            let duration = seg.end_time - seg.start_time;
            if duration < MIN_SEGMENT_DURATION_SECS {
                debug!(
                    segment = i,
                    duration, "skipping short segment for embedding"
                );
                continue;
            }

            let start_sample = (seg.start_time * 16_000.0) as usize;
            let end_sample = ((seg.end_time * 16_000.0) as usize).min(audio_len);

            if start_sample >= end_sample || start_sample >= audio_len {
                warn!(segment = i, "segment time out of audio bounds; skipping");
                continue;
            }

            let segment_audio = &audio[start_sample..end_sample];
            let embedding = self.extract_embedding(segment_audio)?;
            results.push((i, embedding));
        }

        info!(
            "extracted {} embeddings from {} segments",
            results.len(),
            segments.len()
        );

        Ok(results)
    }
}

impl std::fmt::Debug for OnnxEmbedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OnnxEmbedder").finish_non_exhaustive()
    }
}
