//! Audio merging utility for combining mic and app audio streams.
//!
//! Both streams are already 16 kHz mono f32 PCM after the capture layer's
//! resampling, so no format conversion is needed — just additive mixing at
//! the correct timestamp offsets.

use vox_capture::AudioChunk;

/// Merge microphone and application audio chunks into a single buffer.
///
/// Chunks are placed at their recorded timestamp offset within the output
/// buffer.  Where chunks overlap, samples are additively mixed and clamped
/// to `[-1.0, 1.0]`.
///
/// Returns an empty `Vec` if both inputs are empty.
#[must_use]
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
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
                output[idx] += sample;
            }
        }
    }

    // Clamp to [-1.0, 1.0].
    for sample in &mut output {
        *sample = sample.clamp(-1.0, 1.0);
    }

    output
}

/// Compute the total output buffer length in samples from an iterator of chunks.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
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
        assert!((result[0] - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn app_only() {
        let app = vec![make_chunk(vec![0.3; 800], 0, StreamRole::Application)];
        let result = merge_chunks(&[], &app);
        assert_eq!(result.len(), 800);
        assert!((result[0] - 0.3).abs() < f32::EPSILON);
    }

    #[test]
    fn overlapping_additive_mix() {
        let mic = vec![make_chunk(vec![0.6; 1600], 0, StreamRole::Microphone)];
        let app = vec![make_chunk(vec![0.6; 1600], 0, StreamRole::Application)];
        let result = merge_chunks(&mic, &app);
        assert_eq!(result.len(), 1600);
        // 0.6 + 0.6 = 1.2, clamped to 1.0
        assert!((result[0] - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn offset_chunks() {
        // Mic at 0ms, app at 100ms (1600 samples at 16kHz = 100ms)
        let mic = vec![make_chunk(vec![0.5; 1600], 0, StreamRole::Microphone)];
        let app = vec![make_chunk(vec![0.3; 1600], 100, StreamRole::Application)];
        let result = merge_chunks(&mic, &app);
        // Total should be 100ms + 100ms = 200ms = 3200 samples
        assert_eq!(result.len(), 3200);
        // First 1600 samples: only mic (0.5)
        assert!((result[0] - 0.5).abs() < f32::EPSILON);
        // Last 1600 samples: only app (0.3)
        assert!((result[1600] - 0.3).abs() < f32::EPSILON);
    }

    #[test]
    fn clamps_negative() {
        let mic = vec![make_chunk(vec![-0.8; 100], 0, StreamRole::Microphone)];
        let app = vec![make_chunk(vec![-0.8; 100], 0, StreamRole::Application)];
        let result = merge_chunks(&mic, &app);
        // -0.8 + -0.8 = -1.6, clamped to -1.0
        assert!((result[0] - (-1.0)).abs() < f32::EPSILON);
    }
}
