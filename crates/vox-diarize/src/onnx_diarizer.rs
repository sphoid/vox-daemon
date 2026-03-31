//! Full speaker diarization pipeline using ONNX speaker embeddings.
//!
//! This module is only compiled when the `onnx` feature flag is enabled.

use tracing::{debug, info};
use vox_core::session::{SpeakerMapping, SpeakerSource, TranscriptSegment};

use crate::clustering;
use crate::embeddings::OnnxEmbedder;
use crate::error::DiarizeError;
use crate::traits::{DiarizationRequest, DiarizationResult, Diarizer};

/// Speaker diarizer backed by ONNX-based speaker embeddings and
/// agglomerative clustering.
pub struct OnnxDiarizer {
    embedder: OnnxEmbedder,
    /// Cosine distance threshold for clustering.
    threshold: f64,
}

impl OnnxDiarizer {
    /// Create a new diarizer from a loaded embedder and clustering threshold.
    #[must_use]
    pub fn new(embedder: OnnxEmbedder, threshold: f64) -> Self {
        Self { embedder, threshold }
    }

    /// Create a diarizer by loading the ONNX model from `model_path`.
    ///
    /// # Errors
    ///
    /// Returns [`DiarizeError::ModelLoad`] if the model cannot be loaded.
    pub fn from_model_path(
        model_path: impl AsRef<std::path::Path>,
        threshold: f64,
    ) -> Result<Self, DiarizeError> {
        let embedder = OnnxEmbedder::new(model_path)?;
        Ok(Self::new(embedder, threshold))
    }
}

impl Diarizer for OnnxDiarizer {
    fn diarize(
        &self,
        request: &DiarizationRequest<'_>,
    ) -> Result<DiarizationResult, DiarizeError> {
        if request.segments.is_empty() {
            return Ok(DiarizationResult {
                segments: Vec::new(),
                speakers: Vec::new(),
            });
        }

        // 1. Extract embeddings per segment.
        let indexed_embeddings = self.embedder.extract_all(request.segments, request.audio)?;

        if indexed_embeddings.is_empty() {
            // All segments were too short for embedding — return unchanged.
            return Ok(DiarizationResult {
                segments: request.segments.to_vec(),
                speakers: vec![SpeakerMapping {
                    id: "Speaker".to_owned(),
                    friendly_name: "Speaker".to_owned(),
                    source: SpeakerSource::Unknown,
                }],
            });
        }

        // Collect just the embeddings for clustering.
        let embeddings: Vec<Vec<f32>> = indexed_embeddings.iter().map(|(_, e)| e.clone()).collect();
        let segment_indices: Vec<usize> = indexed_embeddings.iter().map(|(i, _)| *i).collect();

        // 2. Cluster embeddings.
        let cluster_labels = clustering::agglomerative_cluster(&embeddings, self.threshold);
        let num_clusters = cluster_labels.iter().copied().max().map_or(0, |m| m + 1);
        info!(num_clusters, "speaker clustering complete");

        // 3. If enrollment is provided, identify which cluster is "You".
        let you_cluster = request.enrollment.and_then(|enrollment| {
            let enrollment_emb = self.embedder.extract_embedding(enrollment).ok()?;
            clustering::identify_speaker(&cluster_labels, &embeddings, &enrollment_emb)
        });

        if let Some(yc) = you_cluster {
            debug!(cluster = yc, "identified 'You' cluster from enrollment");
        }

        // 4. Build relabelled segments.
        // Create a mapping from segment index → cluster label.
        let mut seg_to_cluster: Vec<Option<usize>> = vec![None; request.segments.len()];
        for (idx_pos, &seg_idx) in segment_indices.iter().enumerate() {
            seg_to_cluster[seg_idx] = Some(cluster_labels[idx_pos]);
        }

        let mut segments: Vec<TranscriptSegment> = request.segments.to_vec();
        for (i, seg) in segments.iter_mut().enumerate() {
            seg.speaker = match seg_to_cluster[i] {
                Some(c) if you_cluster == Some(c) => "You".to_owned(),
                Some(c) => {
                    // "Speaker 2", "Speaker 3", etc. (skip the "You" label number).
                    let speaker_num = if you_cluster.is_some() && c > you_cluster.unwrap_or(0) {
                        c + 1
                    } else {
                        c + 1
                    };
                    format!("Speaker {speaker_num}")
                }
                None => "Speaker".to_owned(), // segment too short for embedding
            };
        }

        // 5. Build speaker mappings.
        let mut speakers = Vec::new();
        for cluster_id in 0..num_clusters {
            if you_cluster == Some(cluster_id) {
                speakers.push(SpeakerMapping {
                    id: "You".to_owned(),
                    friendly_name: "You".to_owned(),
                    source: SpeakerSource::Microphone,
                });
            } else {
                let num = cluster_id + 1;
                speakers.push(SpeakerMapping {
                    id: format!("Speaker {num}"),
                    friendly_name: format!("Speaker {num}"),
                    source: SpeakerSource::Remote,
                });
            }
        }

        Ok(DiarizationResult { segments, speakers })
    }
}

impl std::fmt::Debug for OnnxDiarizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OnnxDiarizer")
            .field("threshold", &self.threshold)
            .finish_non_exhaustive()
    }
}
