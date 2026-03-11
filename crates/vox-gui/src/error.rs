//! Crate-level error type for `vox-gui`.

use thiserror::Error;

/// Errors that can occur in the GUI layer.
#[derive(Debug, Error)]
pub enum GuiError {
    /// Failed to load or save application configuration.
    #[error("config error: {0}")]
    Config(#[from] vox_core::error::ConfigError),

    /// Failed to access the session store.
    #[error("storage error: {0}")]
    Storage(#[from] vox_core::error::StorageError),

    /// A required field had an invalid value.
    #[error("invalid value for field '{field}': {reason}")]
    InvalidField {
        /// The name of the field that had an invalid value.
        field: &'static str,
        /// A human-readable reason for the failure.
        reason: String,
    },
}
