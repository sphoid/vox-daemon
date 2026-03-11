//! XDG Base Directory utilities for Vox Daemon.
//!
//! All paths follow the XDG Base Directory Specification:
//! - Config: `$XDG_CONFIG_HOME/vox-daemon/`
//! - Data: `$XDG_DATA_HOME/vox-daemon/`
//! - Cache: `$XDG_CACHE_HOME/vox-daemon/`

use std::path::PathBuf;

const APP_DIR_NAME: &str = "vox-daemon";

/// Returns the configuration directory (`$XDG_CONFIG_HOME/vox-daemon/`).
///
/// Falls back to `~/.config/vox-daemon/` if `XDG_CONFIG_HOME` is not set.
#[must_use]
pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join(APP_DIR_NAME)
}

/// Returns the data directory (`$XDG_DATA_HOME/vox-daemon/`).
///
/// Falls back to `~/.local/share/vox-daemon/` if `XDG_DATA_HOME` is not set.
/// If a custom `data_dir` is configured, returns that path instead.
#[must_use]
pub fn data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
        .join(APP_DIR_NAME)
}

/// Returns the data directory, or a custom override if provided.
///
/// If `custom_dir` is non-empty, it is used directly. Otherwise, the XDG default is used.
#[must_use]
pub fn data_dir_or(custom_dir: &str) -> PathBuf {
    if custom_dir.is_empty() {
        data_dir()
    } else {
        PathBuf::from(custom_dir)
    }
}

/// Returns the cache directory (`$XDG_CACHE_HOME/vox-daemon/`).
///
/// Falls back to `~/.cache/vox-daemon/` if `XDG_CACHE_HOME` is not set.
#[must_use]
pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("~/.cache"))
        .join(APP_DIR_NAME)
}

/// Returns the directory where Whisper models are stored.
#[must_use]
pub fn models_dir() -> PathBuf {
    cache_dir().join("models")
}

/// Returns the directory for session JSON files.
#[must_use]
pub fn sessions_dir() -> PathBuf {
    data_dir().join("sessions")
}

/// Returns the directory for session JSON files with a custom data directory override.
#[must_use]
pub fn sessions_dir_or(custom_dir: &str) -> PathBuf {
    data_dir_or(custom_dir).join("sessions")
}

/// Ensures all required directories exist, creating them with appropriate permissions.
///
/// # Errors
///
/// Returns an I/O error if directory creation fails.
pub fn ensure_dirs(custom_data_dir: &str) -> std::io::Result<()> {
    use std::fs;

    let dirs_to_create = [
        config_dir(),
        data_dir_or(custom_data_dir),
        sessions_dir_or(custom_data_dir),
        cache_dir(),
        models_dir(),
    ];

    for dir in &dirs_to_create {
        fs::create_dir_all(dir)?;
        // Set user-only permissions on data directory
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o700);
            fs::set_permissions(dir, perms)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_paths_contain_app_name() {
        assert!(config_dir().ends_with(APP_DIR_NAME));
        assert!(data_dir().ends_with(APP_DIR_NAME));
        assert!(cache_dir().ends_with(APP_DIR_NAME));
    }

    #[test]
    fn test_custom_data_dir_override() {
        let custom = "/tmp/custom-vox";
        assert_eq!(data_dir_or(custom), PathBuf::from(custom));
        assert_eq!(data_dir_or(""), data_dir());
    }

    #[test]
    fn test_sessions_dir_is_under_data() {
        let sessions = sessions_dir();
        let data = data_dir();
        assert!(sessions.starts_with(data));
    }

    #[test]
    fn test_ensure_dirs_creates_directories() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let custom = dir.path().join("vox-data");
        // SAFETY: Test runs in a single thread; no concurrent env var access.
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", dir.path().join("config"));
            std::env::set_var("XDG_CACHE_HOME", dir.path().join("cache"));
        }

        ensure_dirs(custom.to_str().unwrap()).expect("ensure_dirs");

        assert!(dir.path().join("config").join(APP_DIR_NAME).exists());
        assert!(dir.path().join("cache").join(APP_DIR_NAME).exists());
        assert!(custom.join("sessions").exists());

        // SAFETY: Test cleanup; no concurrent env var access.
        unsafe {
            std::env::remove_var("XDG_CONFIG_HOME");
            std::env::remove_var("XDG_CACHE_HOME");
        }
    }
}
