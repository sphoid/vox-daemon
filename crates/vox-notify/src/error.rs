/// Errors that can occur when sending desktop notifications.
#[derive(Debug, thiserror::Error)]
pub enum NotifyError {
    /// The underlying `notify-rust` / D-Bus call failed.
    #[error("notification send failed: {0}")]
    Send(#[from] notify_rust::error::Error),
}
