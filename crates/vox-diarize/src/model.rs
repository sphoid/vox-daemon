//! ONNX model management — download, caching, and path resolution.
//!
//! Follows the same pattern as `vox-transcribe/src/model.rs`: models are
//! cached under `$XDG_CACHE_HOME/vox-daemon/models/` and auto-downloaded
//! from Hugging Face on first use.

use std::path::{Path, PathBuf};

use tracing::info;

use crate::error::DiarizeError;

/// Default Hugging Face URL for the `SpeechBrain` ECAPA-TDNN model exported
/// to ONNX (~6 MB).
const DEFAULT_MODEL_URL: &str =
    "https://huggingface.co/speechbrain/spkrec-ecapa-voxceleb/resolve/main/embedding_model.onnx";

/// File name used in the local cache directory.
const MODEL_FILENAME: &str = "ecapa_tdnn.onnx";

/// Resolve the path to the ONNX speaker embedding model.
///
/// If `custom_path` is non-empty, uses that path directly (the file must
/// exist).  Otherwise, checks the XDG cache directory for a previously
/// downloaded model and downloads it if absent.
///
/// # Errors
///
/// Returns [`DiarizeError::ModelLoad`] if a custom path does not exist, or
/// [`DiarizeError::ModelDownload`] if the automatic download fails.
pub fn resolve_model_path(custom_path: &str) -> Result<PathBuf, DiarizeError> {
    if !custom_path.is_empty() {
        let p = PathBuf::from(custom_path);
        if p.exists() {
            return Ok(p);
        }
        return Err(DiarizeError::ModelLoad(format!(
            "custom model path does not exist: {custom_path}"
        )));
    }

    let cache_path = default_cache_path();

    if cache_path.exists() {
        info!("using cached diarization model at {}", cache_path.display());
        return Ok(cache_path);
    }

    info!("diarization model not found; downloading from Hugging Face...");
    download_model(DEFAULT_MODEL_URL, &cache_path)?;
    Ok(cache_path)
}

/// Returns `true` if the model file exists at the expected location.
#[must_use]
pub fn is_model_downloaded(custom_path: &str) -> bool {
    if !custom_path.is_empty() {
        return Path::new(custom_path).exists();
    }
    default_cache_path().exists()
}

/// Default cache path: `$XDG_CACHE_HOME/vox-daemon/models/ecapa_tdnn.onnx`.
fn default_cache_path() -> PathBuf {
    vox_core::paths::cache_dir()
        .join("models")
        .join(MODEL_FILENAME)
}

/// Download the model from `url` to `dest`, using an atomic write pattern
/// (write to `.tmp` then rename) to avoid leaving partial files on failure.
fn download_model(url: &str, dest: &Path) -> Result<(), DiarizeError> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            DiarizeError::ModelDownload(format!(
                "failed to create cache directory {}: {e}",
                parent.display()
            ))
        })?;
    }

    let tmp_path = dest.with_extension("onnx.tmp");

    let response = reqwest::blocking::get(url).map_err(|e| {
        DiarizeError::ModelDownload(format!("HTTP request failed: {e}"))
    })?;

    if !response.status().is_success() {
        return Err(DiarizeError::ModelDownload(format!(
            "HTTP {} for {url}",
            response.status()
        )));
    }

    let bytes = response.bytes().map_err(|e| {
        DiarizeError::ModelDownload(format!("failed to read response body: {e}"))
    })?;

    std::fs::write(&tmp_path, &bytes).map_err(|e| {
        DiarizeError::ModelDownload(format!(
            "failed to write temp file {}: {e}",
            tmp_path.display()
        ))
    })?;

    std::fs::rename(&tmp_path, dest).map_err(|e| {
        DiarizeError::ModelDownload(format!(
            "failed to rename {} → {}: {e}",
            tmp_path.display(),
            dest.display()
        ))
    })?;

    info!("diarization model downloaded to {}", dest.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_path_missing_returns_error() {
        let result = resolve_model_path("/nonexistent/ecapa.onnx");
        assert!(result.is_err());
    }

    #[test]
    fn custom_path_existing_returns_ok() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let model_file = dir.path().join("test.onnx");
        std::fs::write(&model_file, b"fake onnx").expect("write");

        let result = resolve_model_path(model_file.to_str().unwrap());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), model_file);
    }

    #[test]
    fn is_model_downloaded_false_for_missing() {
        assert!(!is_model_downloaded("/nonexistent/model.onnx"));
    }

    #[test]
    fn is_model_downloaded_true_for_existing() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let model_file = dir.path().join("model.onnx");
        std::fs::write(&model_file, b"data").expect("write");
        assert!(is_model_downloaded(model_file.to_str().unwrap()));
    }
}
