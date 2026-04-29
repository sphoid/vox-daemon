//! Audio amplitude statistics for diagnostic logging.
//!
//! [`AudioStats`] computes peak and RMS levels from a slice of f32 PCM samples
//! and exposes both as linear values and in dBFS (decibels relative to full
//! scale). These are used by the `PipeWire` capture callback to emit structured
//! diagnostic log lines at a controlled rate.

/// Amplitude statistics for a block of f32 PCM audio samples.
///
/// All samples are expected to be in the `[-1.0, 1.0]` range, though `peak`
/// may exceed `1.0` if the source is clipping.
#[derive(Debug, Clone, Copy)]
pub struct AudioStats {
    /// Maximum absolute sample value. Typically in `[0.0, 1.0]`; values above
    /// `1.0` indicate clipping.
    pub peak: f32,
    /// Root-mean-square level. For a full-scale sine wave this approaches
    /// `1.0 / sqrt(2) ≈ 0.707`.
    pub rms: f32,
    /// Number of samples analysed.
    pub samples: usize,
}

impl AudioStats {
    /// Compute peak (max absolute value) and RMS from a slice of f32 samples.
    ///
    /// Returns zeroed stats (`peak = 0.0`, `rms = 0.0`, `samples = 0`) for an
    /// empty slice.
    ///
    /// # Examples
    ///
    /// ```
    /// use vox_capture::AudioStats;
    ///
    /// let stats = AudioStats::compute(&[1.0_f32, -1.0, 1.0, -1.0]);
    /// assert_eq!(stats.peak, 1.0);
    /// assert!((stats.rms - 1.0).abs() < 1e-6);
    /// ```
    #[must_use]
    pub fn compute(samples: &[f32]) -> Self {
        if samples.is_empty() {
            return Self {
                peak: 0.0,
                rms: 0.0,
                samples: 0,
            };
        }

        let mut peak: f32 = 0.0;
        let mut sum_sq: f32 = 0.0;

        for &s in samples {
            let abs = s.abs();
            if abs > peak {
                peak = abs;
            }
            sum_sq += s * s;
        }

        #[allow(clippy::cast_precision_loss)]
        let rms = (sum_sq / samples.len() as f32).sqrt();

        Self {
            peak,
            rms,
            samples: samples.len(),
        }
    }

    /// Peak level in dBFS (decibels relative to full scale).
    ///
    /// Returns [`f32::NEG_INFINITY`] for silence (`peak <= 0.0`).
    ///
    /// # Examples
    ///
    /// ```
    /// use vox_capture::AudioStats;
    ///
    /// let stats = AudioStats::compute(&[1.0_f32; 64]);
    /// assert!((stats.peak_dbfs() - 0.0).abs() < 1e-5);
    /// ```
    #[must_use]
    pub fn peak_dbfs(&self) -> f32 {
        if self.peak <= 0.0 {
            f32::NEG_INFINITY
        } else {
            20.0 * self.peak.log10()
        }
    }

    /// RMS level in dBFS (decibels relative to full scale).
    ///
    /// Returns [`f32::NEG_INFINITY`] for silence (`rms <= 0.0`).
    ///
    /// # Examples
    ///
    /// ```
    /// use vox_capture::AudioStats;
    ///
    /// let stats = AudioStats::compute(&[1.0_f32; 64]);
    /// assert!((stats.rms_dbfs() - 0.0).abs() < 1e-5);
    /// ```
    #[must_use]
    pub fn rms_dbfs(&self) -> f32 {
        if self.rms <= 0.0 {
            f32::NEG_INFINITY
        } else {
            20.0 * self.rms.log10()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_returns_zeroed_stats() {
        let stats = AudioStats::compute(&[]);
        assert!(stats.peak.abs() < f32::EPSILON, "peak should be 0.0");
        assert!(stats.rms.abs() < f32::EPSILON, "rms should be 0.0");
        assert_eq!(stats.samples, 0);
    }

    #[test]
    fn empty_input_dbfs_is_neg_infinity() {
        let stats = AudioStats::compute(&[]);
        assert!(
            stats.peak_dbfs().is_infinite() && stats.peak_dbfs().is_sign_negative(),
            "peak_dbfs should be -∞ for empty input"
        );
        assert!(
            stats.rms_dbfs().is_infinite() && stats.rms_dbfs().is_sign_negative(),
            "rms_dbfs should be -∞ for empty input"
        );
    }

    #[test]
    fn all_zeros_returns_zeroed_stats() {
        let stats = AudioStats::compute(&[0.0_f32; 64]);
        assert!(stats.peak.abs() < f32::EPSILON, "peak should be 0.0");
        assert!(stats.rms.abs() < f32::EPSILON, "rms should be 0.0");
        assert!(
            stats.peak_dbfs().is_infinite() && stats.peak_dbfs().is_sign_negative(),
            "peak_dbfs should be -∞ for silence"
        );
        assert!(
            stats.rms_dbfs().is_infinite() && stats.rms_dbfs().is_sign_negative(),
            "rms_dbfs should be -∞ for silence"
        );
    }

    #[test]
    fn constant_full_scale_gives_zero_dbfs() {
        // DC signal at 1.0: peak = 1.0, rms = 1.0, both 0 dBFS.
        let samples = vec![1.0_f32; 512];
        let stats = AudioStats::compute(&samples);
        assert!((stats.peak - 1.0).abs() < 1e-6, "peak should be 1.0");
        assert!((stats.rms - 1.0).abs() < 1e-6, "rms should be 1.0");
        assert!(
            stats.peak_dbfs().abs() < 1e-5,
            "peak_dbfs should be 0 dBFS, got {}",
            stats.peak_dbfs()
        );
        assert!(
            stats.rms_dbfs().abs() < 1e-5,
            "rms_dbfs should be 0 dBFS, got {}",
            stats.rms_dbfs()
        );
    }

    #[test]
    fn constant_half_scale_gives_approx_neg6_dbfs() {
        // DC at 0.5: peak_dbfs = rms_dbfs = 20 * log10(0.5) ≈ -6.02 dBFS.
        let samples = vec![0.5_f32; 512];
        let stats = AudioStats::compute(&samples);
        assert!((stats.peak - 0.5).abs() < 1e-6, "peak should be 0.5");
        assert!((stats.rms - 0.5).abs() < 1e-6, "rms should be 0.5");

        let expected_dbfs: f32 = 20.0 * 0.5_f32.log10(); // ≈ -6.0206
        assert!(
            (stats.peak_dbfs() - expected_dbfs).abs() < 1e-4,
            "peak_dbfs should be ~-6.02 dBFS, got {}",
            stats.peak_dbfs()
        );
        assert!(
            (stats.rms_dbfs() - expected_dbfs).abs() < 1e-4,
            "rms_dbfs should be ~-6.02 dBFS, got {}",
            stats.rms_dbfs()
        );
    }

    #[test]
    fn pure_sine_at_half_amplitude() {
        // A pure sine at amplitude 0.5: peak ≈ 0.5, rms ≈ 0.5 / sqrt(2).
        // Use 4800 samples (100 ms at 48 kHz) so the last partial cycle
        // doesn't skew the stats.
        use std::f32::consts::PI;
        let n = 4800_usize;
        let freq = 1000.0_f32; // 1 kHz
        let sr = 48_000.0_f32;
        let samples: Vec<f32> = (0..n)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32 / sr;
                0.5 * (2.0 * PI * freq * t).sin()
            })
            .collect();

        let stats = AudioStats::compute(&samples);

        // Peak should be close to 0.5 (within one sample's rounding).
        assert!(
            (stats.peak - 0.5).abs() < 0.001,
            "sine peak should be ~0.5, got {}",
            stats.peak
        );

        // RMS of a sine at amplitude A is A / sqrt(2).
        let expected_rms = 0.5_f32 / 2.0_f32.sqrt();
        assert!(
            (stats.rms - expected_rms).abs() < 0.001,
            "sine rms should be ~{expected_rms:.4}, got {}",
            stats.rms
        );
    }

    #[test]
    fn negative_samples_use_absolute_value_for_peak() {
        // Ensure peak is taken from absolute value, not signed value.
        let samples = vec![-0.8_f32, 0.3, -0.6, 0.1];
        let stats = AudioStats::compute(&samples);
        assert!(
            (stats.peak - 0.8).abs() < 1e-6,
            "peak should be abs(-0.8) = 0.8, got {}",
            stats.peak
        );
    }

    #[test]
    fn sample_count_is_accurate() {
        let n = 1337_usize;
        let stats = AudioStats::compute(&vec![0.1_f32; n]);
        assert_eq!(stats.samples, n);
    }
}
