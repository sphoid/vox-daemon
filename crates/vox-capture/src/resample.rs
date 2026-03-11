//! Audio resampling utilities.
//!
//! Whisper requires **16 kHz mono f32 PCM**. This module provides:
//!
//! - [`to_mono`] — downmix any channel count to mono by averaging channels.
//! - [`resample_linear`] — linear interpolation resampler for converting
//!   between arbitrary integer sample rates.
//! - [`convert`] — high-level helper that applies both operations in one call.
//!
//! # Note on quality
//!
//! Linear interpolation is adequate for speech (the target use-case) and has
//! no external dependencies. For music or high-fidelity audio a sinc-based
//! resampler would be preferable, but that would require a native C library.

use crate::error::CaptureError;

/// The target sample rate required by Whisper.
pub const TARGET_SAMPLE_RATE: u32 = 16_000;

/// Downmix `channels` interleaved channels to mono by averaging.
///
/// Returns the input unchanged if `channels == 1`.
///
/// # Errors
///
/// Returns [`CaptureError::Format`] if `channels` is zero or the sample count
/// is not a multiple of `channels`.
pub fn to_mono(samples: &[f32], channels: u32) -> Result<Vec<f32>, CaptureError> {
    if channels == 0 {
        return Err(CaptureError::Format(
            "channel count must be at least 1".to_owned(),
        ));
    }
    if channels == 1 {
        return Ok(samples.to_vec());
    }
    let ch = channels as usize;
    if samples.len() % ch != 0 {
        return Err(CaptureError::Format(format!(
            "sample count {} is not divisible by channel count {}",
            samples.len(),
            ch
        )));
    }
    let frames = samples.len() / ch;
    let mut mono = Vec::with_capacity(frames);
    for frame in samples.chunks_exact(ch) {
        let sum: f32 = frame.iter().sum();
        #[allow(clippy::cast_precision_loss)]
        mono.push(sum / channels as f32);
    }
    Ok(mono)
}

/// Resample mono f32 PCM from `src_rate` to `dst_rate` using linear
/// interpolation.
///
/// This is suitable for speech audio where `src_rate` and `dst_rate` are
/// within an order of magnitude of each other.
///
/// Returns the input unchanged (cloned) when `src_rate == dst_rate`.
///
/// # Errors
///
/// Returns [`CaptureError::Format`] if either rate is zero.
pub fn resample_linear(
    samples: &[f32],
    src_rate: u32,
    dst_rate: u32,
) -> Result<Vec<f32>, CaptureError> {
    if src_rate == 0 || dst_rate == 0 {
        return Err(CaptureError::Format(
            "sample rate must be greater than zero".to_owned(),
        ));
    }
    if src_rate == dst_rate {
        return Ok(samples.to_vec());
    }
    if samples.is_empty() {
        return Ok(Vec::new());
    }

    // Ratio: for every output sample, how many input samples do we advance?
    let ratio = f64::from(src_rate) / f64::from(dst_rate);

    // Output length (ceiling to avoid clipping the last partial frame).
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation
    )]
    let out_len = (samples.len() as f64 / ratio).ceil() as usize;

    let mut out = Vec::with_capacity(out_len);
    let last = samples.len() - 1;

    for i in 0..out_len {
        #[allow(clippy::cast_precision_loss)]
        let pos = i as f64 * ratio;
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let idx = pos as usize;
        #[allow(clippy::cast_precision_loss)]
        let frac = pos - idx as f64;

        let s0 = samples[idx.min(last)];
        let s1 = samples[(idx + 1).min(last)];

        #[allow(clippy::cast_possible_truncation)]
        out.push(s0 + (s1 - s0) * frac as f32);
    }

    Ok(out)
}

/// Convert interleaved multi-channel audio at `src_rate` to mono 16 kHz f32.
///
/// This is the main entry point used by the `PipeWire` callback to prepare
/// audio chunks before pushing them into the channel.
///
/// # Errors
///
/// Propagates errors from [`to_mono`] and [`resample_linear`].
pub fn convert(samples: &[f32], src_rate: u32, channels: u32) -> Result<Vec<f32>, CaptureError> {
    let mono = to_mono(samples, channels)?;
    resample_linear(&mono, src_rate, TARGET_SAMPLE_RATE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_mono_identity_for_single_channel() {
        let input = vec![0.1, 0.2, 0.3];
        let result = to_mono(&input, 1).expect("should succeed");
        assert_eq!(result, input);
    }

    #[test]
    fn to_mono_averages_stereo() {
        // L=1.0, R=0.0 → 0.5
        let input = vec![1.0_f32, 0.0, 0.5, 0.5];
        let result = to_mono(&input, 2).expect("should succeed");
        assert_eq!(result.len(), 2);
        assert!((result[0] - 0.5).abs() < 1e-6);
        assert!((result[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn to_mono_rejects_zero_channels() {
        let err = to_mono(&[1.0], 0).unwrap_err();
        assert!(matches!(err, CaptureError::Format(_)));
    }

    #[test]
    fn to_mono_rejects_misaligned() {
        // 3 samples is not divisible by 2 channels
        let err = to_mono(&[1.0, 2.0, 3.0], 2).unwrap_err();
        assert!(matches!(err, CaptureError::Format(_)));
    }

    #[test]
    fn resample_passthrough_when_rates_equal() {
        let input = vec![0.1, 0.2, 0.3];
        let result = resample_linear(&input, 48_000, 48_000).expect("should succeed");
        assert_eq!(result, input);
    }

    #[test]
    fn resample_empty_input() {
        let result = resample_linear(&[], 48_000, 16_000).expect("should succeed");
        assert!(result.is_empty());
    }

    #[test]
    fn resample_rejects_zero_rate() {
        let err = resample_linear(&[1.0], 0, 16_000).unwrap_err();
        assert!(matches!(err, CaptureError::Format(_)));
    }

    #[test]
    fn resample_downsamples_correctly() {
        // A constant signal at 1.0 should remain 1.0 after resampling.
        let input = vec![1.0_f32; 48_000];
        let result = resample_linear(&input, 48_000, 16_000).expect("should succeed");
        // Should be approximately 16 000 samples.
        assert_eq!(result.len(), 16_000);
        for sample in &result {
            assert!(
                (sample - 1.0).abs() < 1e-5,
                "constant signal should stay constant, got {sample}"
            );
        }
    }

    #[test]
    fn convert_stereo_48k_to_mono_16k() {
        // 0.25 s of stereo 48 kHz → should give ~4 000 mono 16 kHz samples.
        let frames = 48_000 / 4; // 12 000 stereo frames = 24 000 samples
        let input: Vec<f32> = (0..frames * 2).map(|_| 0.5_f32).collect();
        let result = convert(&input, 48_000, 2).expect("should succeed");
        // 12 000 mono frames at 48 kHz → 4 000 frames at 16 kHz
        assert_eq!(result.len(), frames / 3);
        for s in &result {
            assert!((s - 0.5).abs() < 1e-5);
        }
    }
}
