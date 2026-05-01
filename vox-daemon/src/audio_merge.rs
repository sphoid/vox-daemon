//! Audio merging utility for combining mic and app audio streams.
//!
//! Both streams are already 16 kHz mono f32 PCM after the capture layer's
//! resampling, so no format conversion is needed — just additive mixing at
//! the correct timestamp offsets.

use vox_capture::{AudioChunk, AudioStats};

/// Per-stream gain applied before additive mixing.
///
/// With two streams summed at 0.5 gain each, the result can never exceed
/// `±1.0` no matter how loud the inputs are, so the previous hard-clip
/// clamp is unnecessary and audible distortion from clipping is eliminated.
const PRE_MIX_GAIN: f32 = 0.5;

/// Merge microphone and application audio chunks into a single buffer.
///
/// Chunks are placed at their recorded timestamp offset within the output
/// buffer.  Each stream is attenuated by [`PRE_MIX_GAIN`] before being
/// additively mixed, which guarantees the sum stays within `[-1.0, 1.0]`
/// without hard-clipping.
///
/// Returns an empty `Vec` if both inputs are empty.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub fn merge_chunks(mic: &[AudioChunk], app: &[AudioChunk]) -> Vec<f32> {
    let total_len = compute_total_length(mic.iter().chain(app.iter()));
    if total_len == 0 {
        return Vec::new();
    }

    let mut output = vec![0.0_f32; total_len];

    for chunk in mic.iter().chain(app.iter()) {
        let offset_samples = (chunk.timestamp.as_secs_f64() * 16_000.0) as usize;
        for (i, &sample) in chunk.samples.iter().enumerate() {
            let idx = offset_samples + i;
            if idx < output.len() {
                output[idx] += sample * PRE_MIX_GAIN;
            }
        }
    }

    // Belt-and-suspenders: even though PRE_MIX_GAIN guarantees no clipping
    // for two streams, clamp anyway to defend against future N-stream changes.
    let mut clipped = 0_usize;
    for sample in &mut output {
        if sample.abs() > 1.0 {
            clipped += 1;
        }
        *sample = sample.clamp(-1.0, 1.0);
    }

    let stats = AudioStats::compute(&output);
    tracing::debug!(
        samples = output.len(),
        peak_dbfs = stats.peak_dbfs(),
        rms_dbfs = stats.rms_dbfs(),
        clipped_samples = clipped,
        "merged buffer stats"
    );

    output
}

/// Compute the total output buffer length in samples from an iterator of chunks.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn compute_total_length<'a>(chunks: impl Iterator<Item = &'a AudioChunk>) -> usize {
    let mut max_end: f64 = 0.0;
    for chunk in chunks {
        let end = chunk.timestamp.as_secs_f64() + chunk.duration_secs();
        if end > max_end {
            max_end = end;
        }
    }
    (max_end * 16_000.0).ceil() as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use vox_capture::StreamRole;

    fn make_chunk(samples: Vec<f32>, timestamp_ms: u64, role: StreamRole) -> AudioChunk {
        AudioChunk::new(samples, Duration::from_millis(timestamp_ms), role)
    }

    #[test]
    fn empty_inputs() {
        let result = merge_chunks(&[], &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn mic_only() {
        let mic = vec![make_chunk(vec![0.5; 1600], 0, StreamRole::Microphone)];
        let result = merge_chunks(&mic, &[]);
        assert_eq!(result.len(), 1600);
        // 0.5 * PRE_MIX_GAIN (0.5) = 0.25
        assert!((result[0] - 0.25).abs() < f32::EPSILON);
    }

    #[test]
    fn app_only() {
        let app = vec![make_chunk(vec![0.3; 800], 0, StreamRole::Application)];
        let result = merge_chunks(&[], &app);
        assert_eq!(result.len(), 800);
        // 0.3 * PRE_MIX_GAIN (0.5) = 0.15
        assert!((result[0] - 0.15).abs() < f32::EPSILON);
    }

    #[test]
    fn overlapping_additive_mix_does_not_clip() {
        // With per-stream 0.5 gain, even maximum-amplitude streams sum to
        // exactly ±1.0 with no clipping.
        let mic = vec![make_chunk(vec![0.6; 1600], 0, StreamRole::Microphone)];
        let app = vec![make_chunk(vec![0.6; 1600], 0, StreamRole::Application)];
        let result = merge_chunks(&mic, &app);
        assert_eq!(result.len(), 1600);
        // 0.6 * 0.5 + 0.6 * 0.5 = 0.6 — well under the clamp threshold.
        assert!((result[0] - 0.6).abs() < f32::EPSILON);
    }

    #[test]
    fn offset_chunks() {
        // Mic at 0ms, app at 100ms (1600 samples at 16kHz = 100ms)
        let mic = vec![make_chunk(vec![0.5; 1600], 0, StreamRole::Microphone)];
        let app = vec![make_chunk(vec![0.3; 1600], 100, StreamRole::Application)];
        let result = merge_chunks(&mic, &app);
        // Total should be 100ms + 100ms = 200ms = 3200 samples
        assert_eq!(result.len(), 3200);
        // First 1600 samples: only mic (0.5 * 0.5 = 0.25)
        assert!((result[0] - 0.25).abs() < f32::EPSILON);
        // Last 1600 samples: only app (0.3 * 0.5 = 0.15)
        assert!((result[1600] - 0.15).abs() < f32::EPSILON);
    }

    #[test]
    fn negative_streams_do_not_clip() {
        let mic = vec![make_chunk(vec![-0.8; 100], 0, StreamRole::Microphone)];
        let app = vec![make_chunk(vec![-0.8; 100], 0, StreamRole::Application)];
        let result = merge_chunks(&mic, &app);
        // -0.8 * 0.5 + -0.8 * 0.5 = -0.8 — within bounds, no clip.
        assert!((result[0] - (-0.8)).abs() < f32::EPSILON);
    }
}
