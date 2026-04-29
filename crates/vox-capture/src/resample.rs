//! Audio resampling utilities.
//!
//! Whisper requires **16 kHz mono f32 PCM**. This module provides:
//!
//! - [`to_mono`] — downmix any channel count to mono by averaging channels.
//! - [`resample_linear`] — single-shot FFT-based resampler.  Construct-and-go;
//!   useful for offline/test usage where the entire signal is known up front.
//! - [`StreamingResampler`] — **stateful** FFT-based resampler designed for the
//!   `PipeWire` callback path.  Construct **once per stream**, then call
//!   [`StreamingResampler::push`] for each incoming buffer.  The convolution
//!   state is preserved across calls, so the cold-start transient happens
//!   exactly once at session start instead of once per buffer.
//! - [`convert`] — single-shot helper combining [`to_mono`] and [`resample_linear`].
//!
//! # Why the streaming type matters
//!
//! `rubato::FftFixedIn` uses overlap-add convolution.  The first call has a
//! zero-initialised overlap buffer, which produces a leading region of garbage
//! (zero or near-zero) output.  When the resampler is constructed fresh on
//! every `PipeWire` callback (~50 calls/sec at typical period sizes), every
//! output buffer carries that transient and the concatenated stream contains
//! a discontinuity at every chunk boundary — audible as severe distortion.
//! [`StreamingResampler`] holds one resampler across calls so the transient
//! is a single, brief occurrence at session start.

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
/// **Single-shot only**.  Suitable for offline/test usage but **not** for the
/// streaming `PipeWire` callback path — see the module docs and
/// [`StreamingResampler`] for why.
///
/// # Errors
///
/// Propagates errors from [`to_mono`] and [`resample_linear`].
pub fn convert(samples: &[f32], src_rate: u32, channels: u32) -> Result<Vec<f32>, CaptureError> {
    let mono = to_mono(samples, channels)?;
    resample_linear(&mono, src_rate, TARGET_SAMPLE_RATE)
}

/// Default fixed input chunk size (in frames) used by [`StreamingResampler`].
///
/// 1024 frames matches `PipeWire`'s typical period size and keeps the FFT
/// latency low (~21 ms at 48 kHz source rate).  Smaller chunk sizes give
/// lower latency but worse filter quality; larger chunks the opposite.
const STREAMING_CHUNK_SIZE_IN: usize = 1024;

/// Stateful resampler for streaming use.  Maintains overlap-add convolution
/// state across calls so the cold-start transient is a one-time event at
/// session start, not at every buffer boundary.
///
/// Operates on **mono** input — call [`to_mono`] first if your buffer is
/// interleaved multi-channel.  The destination rate is fixed at construction;
/// the source rate is configured lazily from the first [`push`](Self::push)
/// call (since callers often don't know the negotiated `PipeWire` rate yet).
///
/// If the source rate changes mid-session (rare — happens on `PipeWire` format
/// renegotiation) the resampler is rebuilt and any pending residue is
/// discarded; a few ms of audio is lost at that boundary but the rest of the
/// stream stays consistent.
pub struct StreamingResampler {
    inner: Option<FftFixedIn<f32>>,
    configured_src_rate: u32,
    dst_rate: u32,
    chunk_size_in: usize,
    in_buffer: Vec<f32>,
}

impl StreamingResampler {
    /// Create a new resampler that produces audio at `dst_rate`.  The source
    /// rate is determined on the first [`push`](Self::push) call.
    #[must_use]
    pub fn new(dst_rate: u32) -> Self {
        Self {
            inner: None,
            configured_src_rate: 0,
            dst_rate,
            chunk_size_in: STREAMING_CHUNK_SIZE_IN,
            in_buffer: Vec::new(),
        }
    }

    /// Push a mono input chunk at `src_rate`.  Returns whatever resampled
    /// output is now available — possibly an empty `Vec` if the input did
    /// not yet accumulate to a full FFT chunk; the residue is held internally
    /// and consumed on the next call.
    ///
    /// If `src_rate == dst_rate`, the input passes through unchanged.
    /// If `src_rate` differs from a previously-configured rate, the resampler
    /// is rebuilt and any pending residue is discarded.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError::Format`] if `src_rate` is zero, or if rubato
    /// fails to construct or run the resampler.
    ///
    /// # Panics
    ///
    /// Cannot panic in practice: the inner resampler is always `Some` after
    /// the lazy-construction block at the start of this method.  The
    /// `.expect()` documents the invariant for the type checker.
    #[allow(clippy::missing_panics_doc)]
    pub fn push(&mut self, samples: &[f32], src_rate: u32) -> Result<Vec<f32>, CaptureError> {
        if src_rate == 0 {
            return Err(CaptureError::Format(
                "src_rate must be greater than zero".to_owned(),
            ));
        }
        if src_rate == self.dst_rate {
            return Ok(samples.to_vec());
        }

        if self.inner.is_none() || self.configured_src_rate != src_rate {
            self.inner = Some(
                FftFixedIn::<f32>::new(
                    src_rate as usize,
                    self.dst_rate as usize,
                    self.chunk_size_in,
                    2,
                    1, // mono
                )
                .map_err(|e| {
                    CaptureError::Format(format!("rubato: failed to create resampler: {e}"))
                })?,
            );
            self.configured_src_rate = src_rate;
            self.in_buffer.clear();
        }

        let resampler = self
            .inner
            .as_mut()
            .expect("inner is Some after construction above");

        self.in_buffer.extend_from_slice(samples);

        let mut output = Vec::new();
        while self.in_buffer.len() >= self.chunk_size_in {
            // Drain exactly chunk_size_in input frames into the
            // non-interleaved single-channel layout that rubato expects.
            let chunk: Vec<f32> = self.in_buffer.drain(..self.chunk_size_in).collect();
            let wave_in = vec![chunk];
            let res = resampler
                .process(&wave_in, None)
                .map_err(|e| CaptureError::Format(format!("rubato: resampling failed: {e}")))?;
            if let Some(out_chan) = res.into_iter().next() {
                output.extend(out_chan);
            }
        }

        Ok(output)
    }

    /// The configured destination sample rate.
    #[must_use]
    pub const fn dst_rate(&self) -> u32 {
        self.dst_rate
    }
}

impl Default for StreamingResampler {
    fn default() -> Self {
        Self::new(TARGET_SAMPLE_RATE)
    }
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

    #[test]
    fn streaming_passthrough_when_rates_equal() {
        let mut r = StreamingResampler::new(48_000);
        let out = r.push(&[0.1_f32, 0.2, 0.3], 48_000).expect("push");
        assert_eq!(out, vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn streaming_rejects_zero_rate() {
        let mut r = StreamingResampler::new(16_000);
        let err = r.push(&[1.0_f32], 0).unwrap_err();
        assert!(matches!(err, CaptureError::Format(_)));
    }

    #[test]
    fn streaming_buffers_partial_input_below_chunk_size() {
        // A first push smaller than chunk_size_in must produce no output —
        // the residue is held internally for the next call.
        let mut r = StreamingResampler::new(16_000);
        let out = r.push(&[0.5_f32; 100], 48_000).expect("push");
        assert!(
            out.is_empty(),
            "partial input should buffer, got {} samples",
            out.len()
        );
    }

    #[test]
    fn streaming_no_mid_stream_discontinuities() {
        // This is the regression test for the per-callback resampler bug.
        //
        // We feed a constant 1.0 signal in many small chunks and require the
        // concatenated output to remain at ≈1.0 after the cold-start transient.
        // With the previous (broken) per-call resampler construction, every
        // callback's output had its own zero-leading transient, which meant
        // the concatenated WAV had a zero gap at every chunk boundary.  This
        // test catches that regression: it would fail with values near 0.0
        // appearing periodically through the output array.
        let mut r = StreamingResampler::new(16_000);
        let n_chunks = 100_usize;
        let chunk_size = 1024_usize;
        let mut output: Vec<f32> = Vec::new();

        for _ in 0..n_chunks {
            let chunk = vec![1.0_f32; chunk_size];
            output.extend(r.push(&chunk, 48_000).expect("push"));
        }

        assert!(
            !output.is_empty(),
            "should produce output across {n_chunks} chunks"
        );

        // Skip the cold-start transient (≤ 50% of total output is generous).
        // After that, every sample must be near 1.0 — no zero gaps allowed.
        let skip = output.len() / 2;
        for (i, &s) in output[skip..].iter().enumerate() {
            assert!(
                (s - 1.0).abs() < 0.05,
                "discontinuity at output[{}] = {s} (expected ≈1.0); \
                 this indicates per-chunk transient leakage",
                i + skip
            );
        }
    }

    #[test]
    fn streaming_rebuilds_on_rate_change() {
        let mut r = StreamingResampler::new(16_000);
        // First feed at 48 kHz.
        let _ = r.push(&[1.0_f32; 1024], 48_000).expect("first push");
        // Now switch to 44.1 kHz — should rebuild without panicking.
        let _ = r.push(&[1.0_f32; 1024], 44_100).expect("post-rebuild push");
        assert_eq!(r.configured_src_rate, 44_100);
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
