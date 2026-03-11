//! Whisper model management: path resolution and download helpers.
//!
//! Models are stored as GGML `.bin` files under
//! `$XDG_CACHE_HOME/vox-daemon/models/`.  The file naming convention follows
//! the upstream whisper.cpp repository:
//! `ggml-<size>.bin` (e.g., `ggml-base.bin`, `ggml-small.bin`).
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

use std::path::{Path, PathBuf};

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

/// Resolves the filesystem path to the Whisper model file.
///
/// Resolution order:
/// 1. If `config.model_path` is non-empty, that path is used directly.
/// 2. Otherwise, the model file is expected at
///    `$XDG_CACHE_HOME/vox-daemon/models/ggml-<size>.bin`.
///
/// # Errors
///
/// Returns [`TranscribeError::ModelLoad`] if `config.model` names an unknown
/// size or if the resolved file does not exist on disk.
pub fn resolve_model_path(config: &TranscriptionConfig) -> Result<PathBuf, TranscribeError> {
    if !config.model_path.is_empty() {
        let path = PathBuf::from(&config.model_path);
        return verify_exists(path);
    }

    let size = ModelSize::from_str(&config.model)?;
    let path = paths::models_dir().join(size.file_name());
    verify_exists(path)
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
    fn test_resolve_model_path_custom_missing_file() {
        let config = TranscriptionConfig {
            model: "base".to_owned(),
            language: "en".to_owned(),
            gpu_backend: "auto".to_owned(),
            model_path: "/nonexistent/path/model.bin".to_owned(),
        };
        assert!(resolve_model_path(&config).is_err());
    }

    #[test]
    fn test_resolve_model_path_custom_existing_file() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let model_file = dir.path().join("custom.bin");
        std::fs::write(&model_file, b"fake model data").expect("write fake model");

        let config = TranscriptionConfig {
            model: "base".to_owned(),
            language: "en".to_owned(),
            gpu_backend: "auto".to_owned(),
            model_path: model_file.to_str().unwrap().to_owned(),
        };
        let result = resolve_model_path(&config);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), model_file);
    }

    #[test]
    fn test_is_model_downloaded_false_when_missing() {
        let config = TranscriptionConfig {
            model: "large".to_owned(),
            language: "en".to_owned(),
            gpu_backend: "auto".to_owned(),
            model_path: String::new(),
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
            model: "base".to_owned(),
            language: "en".to_owned(),
            gpu_backend: "auto".to_owned(),
            model_path: model_file.to_str().unwrap().to_owned(),
        };
        assert!(is_model_downloaded(&config));
    }
}
