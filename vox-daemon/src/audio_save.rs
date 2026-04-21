//! WAV file I/O for retaining and reloading session audio.
//!
//! Audio is stored as 16-bit PCM WAV (16 kHz, mono) which is compact
//! (~1.9 MB/minute) and universally playable.  The internal pipeline
//! works with `f32` samples, so conversions are performed on read/write.

use std::path::Path;

use anyhow::{Context, Result};

/// Saves `f32` audio samples as a 16-bit PCM WAV file.
///
/// Samples are expected to be in the range `[-1.0, 1.0]` at 16 kHz mono.
/// Values outside that range are clamped before conversion.
pub fn save_wav(path: &Path, samples: &[f32]) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer =
        hound::WavWriter::create(path, spec).context("failed to create WAV file")?;

    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        #[allow(clippy::cast_possible_truncation)]
        let sample_i16 = (clamped * f32::from(i16::MAX)) as i16;
        writer.write_sample(sample_i16).context("failed to write WAV sample")?;
    }

    writer.finalize().context("failed to finalize WAV file")?;
    tracing::info!("audio saved to {}", path.display());

    Ok(())
}

/// Loads a 16 kHz mono WAV file and returns `f32` samples in `[-1.0, 1.0]`.
///
/// The file must be 16-bit PCM, 16 kHz, mono — the format produced by
/// [`save_wav`].
pub fn load_wav(path: &Path) -> Result<Vec<f32>> {
    let reader = hound::WavReader::open(path).context("failed to open WAV file")?;
    let spec = reader.spec();

    anyhow::ensure!(
        spec.channels == 1,
        "expected mono WAV, got {} channels",
        spec.channels
    );
    anyhow::ensure!(
        spec.sample_rate == 16_000,
        "expected 16 kHz WAV, got {} Hz",
        spec.sample_rate
    );
    anyhow::ensure!(
        spec.sample_format == hound::SampleFormat::Int && spec.bits_per_sample == 16,
        "expected 16-bit PCM WAV, got {}-bit {:?}",
        spec.bits_per_sample,
        spec.sample_format
    );

    let samples: Vec<f32> = reader
        .into_samples::<i16>()
        .map(|s| {
            let s = s.context("failed to read WAV sample")?;
            Ok(f32::from(s) / f32::from(i16::MAX))
        })
        .collect::<Result<Vec<f32>>>()?;

    tracing::info!(
        "loaded {} samples ({:.1}s) from {}",
        samples.len(),
        samples.len() as f64 / 16_000.0,
        path.display()
    );

    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wav_roundtrip() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let path = dir.path().join("test.wav");

        // Generate a short sine wave at 440 Hz.
        let samples: Vec<f32> = (0..16_000)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 16_000.0).sin() * 0.5)
            .collect();

        save_wav(&path, &samples).expect("save");
        let loaded = load_wav(&path).expect("load");

        assert_eq!(samples.len(), loaded.len());

        // 16-bit quantization introduces error up to 1/32768 ≈ 3e-5.
        for (orig, loaded) in samples.iter().zip(loaded.iter()) {
            assert!(
                (orig - loaded).abs() < 0.001,
                "sample mismatch: {orig} vs {loaded}"
            );
        }
    }

    #[test]
    fn test_clamping() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let path = dir.path().join("clamp.wav");

        let samples = vec![-2.0, -1.0, 0.0, 1.0, 2.0];
        save_wav(&path, &samples).expect("save");
        let loaded = load_wav(&path).expect("load");

        // -2.0 and 2.0 should be clamped to -1.0 and 1.0.
        assert!((loaded[0] - (-1.0)).abs() < 0.001);
        assert!((loaded[4] - 1.0).abs() < 0.001);
    }
}
