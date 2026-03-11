//! Core [`Tray`] trait.

use crate::{DaemonStatus, TrayError, TrayEvent};

/// Trait for a system tray icon implementation.
///
/// Implementors manage an OS-level tray icon and send [`TrayEvent`]s back to
/// the daemon when the user interacts with the popup menu.
///
/// The trait is `Send + Sync` so implementors can be shared across threads.
/// The actual event loop may live on a dedicated OS thread (required for GTK
/// on Linux), and this trait allows the main Tokio runtime to call
/// [`Tray::set_status`] from any async context.
pub trait Tray: Send + Sync {
    /// Update the tray icon and menu to reflect the given daemon status.
    ///
    /// For example, when `status` is [`DaemonStatus::Recording`], the icon
    /// should switch to the red-dot variant and "Stop Recording" should become
    /// enabled in the menu.
    ///
    /// # Errors
    ///
    /// Returns [`TrayError`] if the icon or menu update fails.
    fn set_status(&self, status: DaemonStatus) -> Result<(), TrayError>;

    /// Receive the next [`TrayEvent`] from the tray popup menu.
    ///
    /// This call **blocks** until an event is available.  Callers should invoke
    /// it from a dedicated thread rather than an async context.
    ///
    /// Returns `None` when the tray is shutting down and no more events will be
    /// produced.
    fn recv_event(&self) -> Option<TrayEvent>;

    /// Attempt to receive the next [`TrayEvent`] without blocking.
    ///
    /// Returns `None` when no event is currently available or the tray is
    /// shutting down.
    fn try_recv_event(&self) -> Option<TrayEvent>;
}
