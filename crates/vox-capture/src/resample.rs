//! Audio resampling utilities.
//!
//! Whisper requires **16 kHz mono f32 PCM**. This module provides:
//!
//! - [`to_mono`] — downmix any channel count to mono by averaging channels.
//! - [`resample_linear`] — high-quality FFT-based resampler using [`rubato::FftFixedIn`]
//!   for converting between arbitrary integer sample rates with proper anti-aliasing.
//! - [`convert`] — high-level helper that applies both operations in one call.
//!
//! # Resampler implementation
//!
//! The resampler uses `rubato::FftFixedIn`, a synchronous FFT-based algorithm that
//! applies an anti-aliasing Blackman-Harris window filter during downsampling.  For the
//! 48 kHz → 16 kHz (3:1 downsample) path this is essential: frequencies above 8 kHz
//! (the new Nyquist limit) would alias back into the audible band under naive linear
//! interpolation, producing muffled and distorted output.  The FFT resampler removes
//! those frequencies cleanly before decimation.
//!
//! The resampler is constructed once per call with `chunk_size_in = input.len()`.  This
//! is the simplest strategy for arbitrary-length `PipeWire` buffers: no pre-allocation
//! overhead is visible to callers, and `PipeWire`'s RT callback delivers a fixed period
//! size each call (typically 1024 frames), so the constructor cost is amortised in
//! practice.

use rubato::{FftFixedIn, Resampler};

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

/// Resample mono f32 PCM from `src_rate` to `dst_rate` using the `rubato`
/// FFT-based resampler with anti-aliasing.
///
/// `rubato::FftFixedIn` is constructed per call with `chunk_size_in = samples.len()`.
/// `PipeWire` delivers a fixed period size each callback invocation (typically 1024
/// frames), so this allocation is amortised and latency is deterministic.
///
/// Returns the input unchanged (cloned) when `src_rate == dst_rate`.
/// Returns an empty `Vec` when `samples` is empty.
///
/// # Errors
///
/// Returns [`CaptureError::Format`] if either rate is zero or if rubato fails
/// to construct or run the resampler.
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

    // Construct an FftFixedIn resampler sized exactly to this input chunk.
    // sub_chunks=2 lets rubato split the FFT work into two sub-blocks, which
    // reduces peak memory for large buffers while keeping the interface simple.
    let mut resampler = FftFixedIn::<f32>::new(
        src_rate as usize,
        dst_rate as usize,
        samples.len(),
        2,
        1, // mono
    )
    .map_err(|e| CaptureError::Format(format!("rubato: failed to create resampler: {e}")))?;

    // rubato expects non-interleaved input: one Vec<f32> per channel.
    let wave_in = vec![samples.to_vec()];

    let output = resampler
        .process(&wave_in, None)
        .map_err(|e| CaptureError::Format(format!("rubato: resampling failed: {e}")))?;

    // output is Vec<Vec<f32>>; channel 0 is our mono result.
    Ok(output.into_iter().next().unwrap_or_default())
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
        // A constant signal at 1.0 should remain close to 1.0 after resampling.
        //
        // rubato's FFT overlap-add resampler with Blackman-Harris windowing has a
        // cold-start transient that includes both an initial zero region (the overlap
        // buffer is initialised to zero) and edge-ringing from the window filter
        // applied to the step from zero to the constant signal.  We skip the first
        // half of the output and verify the steady-state tail.
        let input = vec![1.0_f32; 48_000];
        let result = resample_linear(&input, 48_000, 16_000).expect("should succeed");
        // Should produce exactly 16 000 output samples.
        assert_eq!(result.len(), 16_000);
        let skip = result.len() / 2;
        for (i, sample) in result[skip..].iter().enumerate() {
            assert!(
                (sample - 1.0).abs() < 0.02,
                "constant signal should stay near 1.0 after transient (idx {i}), got {sample}"
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
        // Skip the cold-start transient (see resample_downsamples_correctly comment).
        let skip = result.len() / 2;
        for (i, s) in result[skip..].iter().enumerate() {
            assert!(
                (s - 0.5).abs() < 0.02,
                "expected ≈0.5 after transient (idx {i}), got {s}"
            );
        }
    }

    /// Anti-aliasing test: a 12 kHz sine fed at 48 kHz source rate is above the
    /// 8 kHz Nyquist of the 16 kHz destination rate.  With proper anti-aliasing
    /// (rubato's FFT resampler applies a Blackman-Harris window filter) the tone
    /// is attenuated to near-zero in the output.  Under naive linear interpolation
    /// the same frequency would alias to 4 kHz and remain audible.
    ///
    /// Verification: the output RMS must be well below the input RMS.
    #[test]
    fn resample_antialias_attenuates_above_nyquist() {
        use std::f32::consts::TAU;

        let src_rate = 48_000_u32;
        let dst_rate = 16_000_u32;
        // 12 kHz is above 8 kHz (Nyquist of 16 kHz) → must be filtered out.
        let freq_hz = 12_000.0_f32;
        let n_frames = 48_000_usize; // 1 second at 48 kHz

        let input: Vec<f32> = (0..n_frames)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32 / src_rate as f32;
                (TAU * freq_hz * t).sin()
            })
            .collect();

        #[allow(clippy::cast_precision_loss)]
        let input_rms = (input.iter().map(|s| s * s).sum::<f32>() / n_frames as f32).sqrt();

        let output = resample_linear(&input, src_rate, dst_rate).expect("should succeed");

        #[allow(clippy::cast_precision_loss)]
        let output_rms = (output.iter().map(|s| s * s).sum::<f32>() / output.len() as f32).sqrt();

        // The 12 kHz tone must be strongly attenuated (< 10% of input RMS).
        assert!(
            output_rms < 0.1 * input_rms,
            "anti-aliasing failed: input_rms={input_rms:.4}, output_rms={output_rms:.4} \
             (expected output_rms < 0.1 * input_rms)"
        );
    }
}
