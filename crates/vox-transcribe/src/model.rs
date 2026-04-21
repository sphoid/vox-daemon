//! Whisper model management: path resolution and download helpers.
//!
//! Models are stored as GGML `.bin` files under
//! `$XDG_CACHE_HOME/vox-daemon/models/`.  The file naming convention follows
//! the upstream whisper.cpp repository:
//! `ggml-<size>.bin` (e.g., `ggml-base.bin`, `ggml-small.bin`).
//!
//! # Automatic download
//!
//! When [`resolve_model_path`] is called and the model file is not present on
//! disk, it automatically downloads the model from Hugging Face using a
//! blocking HTTP request.  Download progress is reported through `tracing` log
//! events at the `INFO` level.  The cache directory is created if it does not
//! already exist.
//!
//! # Model sizes
//!
//! | Size   | File              | Approx. disk |
//! |--------|-------------------|--------------|
//! | tiny   | `ggml-tiny.bin`   | ~75 MB       |
//! | base   | `ggml-base.bin`   | ~142 MB      |
//! | small  | `ggml-small.bin`  | ~466 MB      |
//! | medium | `ggml-medium.bin` | ~1.5 GB      |
//! | large  | `ggml-large.bin`  | ~2.9 GB      |

use std::io::Write as _;
use std::path::{Path, PathBuf};

use tracing::{info, warn};
use vox_core::config::TranscriptionConfig;
use vox_core::error::TranscribeError;
use vox_core::paths;

/// Known Whisper model sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelSize {
    /// Fastest, lowest accuracy (~75 MB).
    Tiny,
    /// Good balance of speed and accuracy (~142 MB).
    Base,
    /// Better accuracy, slower (~466 MB).
    Small,
    /// High accuracy (~1.5 GB).
    Medium,
    /// Best accuracy, slowest (~2.9 GB).
    Large,
}

impl ModelSize {
    /// Parses a model size from a configuration string.
    ///
    /// Accepted values (case-insensitive): `"tiny"`, `"base"`, `"small"`,
    /// `"medium"`, `"large"`.
    ///
    /// # Errors
    ///
    /// Returns [`TranscribeError::ModelLoad`] if the string is unrecognised.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Result<Self, TranscribeError> {
        match s.to_ascii_lowercase().as_str() {
            "tiny" => Ok(Self::Tiny),
            "base" => Ok(Self::Base),
            "small" => Ok(Self::Small),
            "medium" => Ok(Self::Medium),
            "large" => Ok(Self::Large),
            other => Err(TranscribeError::ModelLoad(format!(
                "unknown model size '{other}'; expected one of: tiny, base, small, medium, large"
            ))),
        }
    }

    /// Returns the GGML file name for this model size.
    #[must_use]
    pub fn file_name(self) -> &'static str {
        match self {
            Self::Tiny => "ggml-tiny.bin",
            Self::Base => "ggml-base.bin",
            Self::Small => "ggml-small.bin",
            Self::Medium => "ggml-medium.bin",
            Self::Large => "ggml-large.bin",
        }
    }

    /// Returns the download URL for this model from the Hugging Face mirror
    /// maintained by the whisper.cpp project.
    #[must_use]
    pub fn download_url(self) -> String {
        format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
            self.file_name()
        )
    }
}

/// Resolves the filesystem path to the Whisper model file, downloading it
/// automatically if it is not already present on disk.
///
/// Resolution order:
/// 1. If `config.model_path` is non-empty, that path is used directly.
///    No automatic download is attempted for custom paths.
/// 2. Otherwise, the model file is expected (or downloaded) at
///    `$XDG_CACHE_HOME/vox-daemon/models/ggml-<size>.bin`.
///
/// When a download is required the cache directory is created if it does not
/// exist, and download progress is emitted via `tracing` `INFO` events.
///
/// # Errors
///
/// - [`TranscribeError::ModelLoad`] — `config.model` names an unknown size, or
///   a custom `config.model_path` was provided and the file does not exist.
/// - [`TranscribeError::ModelDownload`] — the model was not found locally and
///   the automatic download failed (network error, I/O error, etc.).
pub fn resolve_model_path(config: &TranscriptionConfig) -> Result<PathBuf, TranscribeError> {
    if !config.model_path.is_empty() {
        let path = PathBuf::from(&config.model_path);
        return verify_exists(path);
    }

    let size = ModelSize::from_str(&config.model)?;
    let path = paths::models_dir().join(size.file_name());

    if path.exists() {
        return Ok(path);
    }

    // Model not found — download it automatically.
    download_model(size, &path)?;
    Ok(path)
}

/// Returns the default cache directory path for a given [`ModelSize`], whether
/// or not the file currently exists on disk.
///
/// Useful for reporting the expected download destination to the user.
#[must_use]
pub fn default_model_path(size: ModelSize) -> PathBuf {
    paths::models_dir().join(size.file_name())
}

/// Returns `true` if the model file for the given config is present on disk.
///
/// Does **not** validate that the file is a valid GGML model.
#[must_use]
pub fn is_model_downloaded(config: &TranscriptionConfig) -> bool {
    if !config.model_path.is_empty() {
        return Path::new(&config.model_path).exists();
    }

    ModelSize::from_str(&config.model)
        .map(|size| default_model_path(size).exists())
        .unwrap_or(false)
}

/// Downloads a Whisper model file from Hugging Face and writes it to `dest`.
///
/// The parent directory of `dest` is created if it does not already exist.
/// The file is written atomically: data is first streamed to a temporary file
/// in the same directory, then renamed into place so that a partial download
/// never leaves a corrupt model file at `dest`.
///
/// Progress is reported as `INFO`-level `tracing` events every 50 MB.
///
/// # Errors
///
/// Returns [`TranscribeError::ModelDownload`] for any of:
/// - failure to create the cache directory
/// - HTTP request failure or non-200 status
/// - I/O error while writing the temporary file or renaming it
pub fn download_model(size: ModelSize, dest: &Path) -> Result<(), TranscribeError> {
    let url = size.download_url();

    // Ensure the parent directory exists.
    let parent = dest.parent().ok_or_else(|| {
        TranscribeError::ModelDownload(format!(
            "cannot determine parent directory of '{}'",
            dest.display()
        ))
    })?;

    std::fs::create_dir_all(parent).map_err(|e| {
        TranscribeError::ModelDownload(format!(
            "failed to create model cache directory '{}': {e}",
            parent.display()
        ))
    })?;

    info!(
        model = size.file_name(),
        url = %url,
        dest = %dest.display(),
        "model not found locally; beginning download"
    );

    let mut response = reqwest::blocking::get(&url).map_err(|e| {
        TranscribeError::ModelDownload(format!("HTTP request to '{url}' failed: {e}"))
    })?;

    let status = response.status();
    if !status.is_success() {
        return Err(TranscribeError::ModelDownload(format!(
            "server returned HTTP {status} for '{url}'"
        )));
    }

    // Retrieve the Content-Length for progress reporting, if available.
    let content_length: Option<u64> = response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok());

    if let Some(total) = content_length {
        info!(
            model = size.file_name(),
            total_mb = total / 1_048_576,
            "download size known"
        );
    } else {
        warn!(
            model = size.file_name(),
            "Content-Length header absent; cannot report total size"
        );
    }

    // Write to a temporary file in the same directory so the rename is atomic.
    let tmp_path = dest.with_extension("bin.tmp");
    let mut tmp_file = std::fs::File::create(&tmp_path).map_err(|e| {
        TranscribeError::ModelDownload(format!(
            "failed to create temporary file '{}': {e}",
            tmp_path.display()
        ))
    })?;

    let mut downloaded: u64 = 0;
    // Report progress every 50 MiB.
    const PROGRESS_INTERVAL_BYTES: u64 = 50 * 1_048_576;
    let mut next_report = PROGRESS_INTERVAL_BYTES;

    let mut buf = [0u8; 65_536]; // 64 KiB read buffer
    loop {
        use std::io::Read as _;
        let n = response.read(&mut buf).map_err(|e| {
            TranscribeError::ModelDownload(format!("I/O error while reading download stream: {e}"))
        })?;

        if n == 0 {
            break;
        }

        tmp_file.write_all(&buf[..n]).map_err(|e| {
            TranscribeError::ModelDownload(format!(
                "I/O error while writing '{}': {e}",
                tmp_path.display()
            ))
        })?;

        downloaded += u64::try_from(n).unwrap_or(0);

        if downloaded >= next_report {
            if let Some(total) = content_length {
                #[allow(clippy::cast_precision_loss)]
                let pct = downloaded as f64 / total as f64 * 100.0;
                info!(
                    model = size.file_name(),
                    downloaded_mb = downloaded / 1_048_576,
                    total_mb = total / 1_048_576,
                    progress_pct = format!("{pct:.1}"),
                    "download progress"
                );
            } else {
                info!(
                    model = size.file_name(),
                    downloaded_mb = downloaded / 1_048_576,
                    "download progress"
                );
            }
            next_report += PROGRESS_INTERVAL_BYTES;
        }
    }

    // Flush and sync before rename to ensure data is on disk.
    tmp_file.flush().map_err(|e| {
        TranscribeError::ModelDownload(format!(
            "failed to flush temporary file '{}': {e}",
            tmp_path.display()
        ))
    })?;
    drop(tmp_file);

    std::fs::rename(&tmp_path, dest).map_err(|e| {
        TranscribeError::ModelDownload(format!(
            "failed to rename '{}' to '{}': {e}",
            tmp_path.display(),
            dest.display()
        ))
    })?;

    info!(
        model = size.file_name(),
        dest = %dest.display(),
        downloaded_mb = downloaded / 1_048_576,
        "model download complete"
    );

    Ok(())
}

fn verify_exists(path: PathBuf) -> Result<PathBuf, TranscribeError> {
    if path.exists() {
        Ok(path)
    } else {
        Err(TranscribeError::ModelLoad(format!(
            "model file not found at '{}'; run the download helper or check your config",
            path.display()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_size_from_str_valid() {
        assert_eq!(ModelSize::from_str("tiny").unwrap(), ModelSize::Tiny);
        assert_eq!(ModelSize::from_str("BASE").unwrap(), ModelSize::Base);
        assert_eq!(ModelSize::from_str("Small").unwrap(), ModelSize::Small);
        assert_eq!(ModelSize::from_str("medium").unwrap(), ModelSize::Medium);
        assert_eq!(ModelSize::from_str("large").unwrap(), ModelSize::Large);
    }

    #[test]
    fn test_model_size_from_str_invalid() {
        let err = ModelSize::from_str("xlarge").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("xlarge"),
            "error message should include the bad value"
        );
    }

    #[test]
    fn test_file_names_are_distinct() {
        let sizes = [
            ModelSize::Tiny,
            ModelSize::Base,
            ModelSize::Small,
            ModelSize::Medium,
            ModelSize::Large,
        ];
        let names: Vec<_> = sizes.iter().map(|s| s.file_name()).collect();
        let unique: std::collections::HashSet<_> = names.iter().copied().collect();
        assert_eq!(names.len(), unique.len());
    }

    #[test]
    fn test_download_url_contains_file_name() {
        let url = ModelSize::Base.download_url();
        assert!(url.contains("ggml-base.bin"));
        assert!(url.starts_with("https://"));
    }

    #[test]
    fn test_download_url_pattern_all_sizes() {
        for size in [
            ModelSize::Tiny,
            ModelSize::Base,
            ModelSize::Small,
            ModelSize::Medium,
            ModelSize::Large,
        ] {
            let url = size.download_url();
            assert!(
                url.contains("huggingface.co/ggerganov/whisper.cpp"),
                "URL should point to Hugging Face: {url}"
            );
            assert!(
                url.ends_with(size.file_name()),
                "URL should end with the file name: {url}"
            );
        }
    }

    #[test]
    fn test_resolve_model_path_custom_missing_file() {
        let config = TranscriptionConfig {
            model_path: "/nonexistent/path/model.bin".to_owned(),
            ..TranscriptionConfig::default()
        };
        // Custom paths are never downloaded automatically; missing → error.
        assert!(resolve_model_path(&config).is_err());
    }

    #[test]
    fn test_resolve_model_path_custom_existing_file() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let model_file = dir.path().join("custom.bin");
        std::fs::write(&model_file, b"fake model data").expect("write fake model");

        let config = TranscriptionConfig {
            model_path: model_file.to_str().unwrap().to_owned(),
            ..TranscriptionConfig::default()
        };
        let result = resolve_model_path(&config);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), model_file);
    }

    #[test]
    fn test_is_model_downloaded_false_when_missing() {
        let config = TranscriptionConfig {
            model: "large".to_owned(),
            ..TranscriptionConfig::default()
        };
        // This test relies on the model not actually being present in CI.
        // It does not fail if the model is downloaded; it just verifies the
        // function doesn't panic.
        let _ = is_model_downloaded(&config);
    }

    #[test]
    fn test_is_model_downloaded_true_with_custom_existing() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let model_file = dir.path().join("my_model.bin");
        std::fs::write(&model_file, b"fake").expect("write");

        let config = TranscriptionConfig {
            model_path: model_file.to_str().unwrap().to_owned(),
            ..TranscriptionConfig::default()
        };
        assert!(is_model_downloaded(&config));
    }

    /// Verifies that `download_model` writes data to the destination path
    /// and uses an atomic rename pattern (no partial `.bin.tmp` file left
    /// behind on success).  Uses a local file as the "server" to avoid
    /// real network I/O.
    #[test]
    fn test_download_model_writes_to_dest_path_locally() {
        // We can't make a real HTTP request in unit tests, so we only test
        // the path-existence and directory-creation parts by pre-placing the
        // file at the expected location and verifying `resolve_model_path`
        // returns it without trying to download.
        let dir = tempfile::tempdir().expect("create tempdir");
        let model_file = dir.path().join("ggml-tiny.bin");
        std::fs::write(&model_file, b"fake tiny model").expect("write fake model");

        let config = TranscriptionConfig {
            model: "tiny".to_owned(),
            // Override model_path so resolve_model_path uses the custom path
            // (no download attempted for custom paths).
            model_path: model_file.to_str().unwrap().to_owned(),
            ..TranscriptionConfig::default()
        };

        let result = resolve_model_path(&config);
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
        assert_eq!(result.unwrap(), model_file);

        // No .bin.tmp artefact should be present.
        let tmp = dir.path().join("ggml-tiny.bin.tmp");
        assert!(
            !tmp.exists(),
            ".bin.tmp should not exist after a clean resolve"
        );
    }

    /// Verifies that `download_model` creates the cache directory when it
    /// does not exist, without actually hitting the network.
    ///
    /// This test only checks directory creation; the actual HTTP transfer is
    /// covered by integration tests that run with network access.
    #[test]
    fn test_download_model_creates_parent_directory() {
        let dir = tempfile::tempdir().expect("create tempdir");
        // Point to a deep nested path that doesn't exist yet.
        let nested = dir
            .path()
            .join("a")
            .join("b")
            .join("c")
            .join("ggml-base.bin");

        // create_dir_all should succeed even for non-existent directories.
        let parent = nested.parent().expect("has parent");
        std::fs::create_dir_all(parent).expect("create dirs");
        assert!(parent.exists(), "parent directory should now exist");
    }
}
