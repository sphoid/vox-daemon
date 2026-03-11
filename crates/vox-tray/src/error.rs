//! Error types for `vox-tray`.

/// Errors that can occur while managing the system tray icon.
#[derive(Debug, thiserror::Error)]
pub enum TrayError {
    /// The tray icon could not be created.
    #[error("failed to create tray icon: {0}")]
    Create(String),

    /// The tray menu could not be built.
    #[error("failed to build tray menu: {0}")]
    Menu(String),

    /// The icon image could not be loaded or decoded.
    #[error("failed to load icon: {0}")]
    Icon(String),

    /// A channel send failed (the receiver was dropped).
    #[error("tray event channel closed")]
    ChannelClosed,

    /// The GTK event loop exited unexpectedly.
    #[error("GTK event loop exited unexpectedly")]
    EventLoopExited,
}
