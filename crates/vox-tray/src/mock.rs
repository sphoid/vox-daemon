//! [`MockTray`] — a test-friendly tray implementation that does not require GTK.

use crossbeam_channel::{Receiver, Sender, unbounded};

use crate::{DaemonStatus, Tray, TrayError, TrayEvent};

/// A system tray implementation for use in tests and CI environments.
///
/// `MockTray` does not create any real OS-level tray icon. Instead it:
///
/// - Records calls to [`set_status`](MockTray::set_status) so tests can assert
///   on them.
/// - Exposes [`inject_event`](MockTray::inject_event) so tests can simulate
///   user interactions.
/// - Implements [`Tray`] so it can be used wherever a real tray is expected.
///
/// # Thread safety
///
/// `MockTray` is `Send + Sync` and can be used across threads.
pub struct MockTray {
    event_tx: Sender<TrayEvent>,
    event_rx: Receiver<TrayEvent>,

    status_tx: Sender<DaemonStatus>,
    /// Receiver for status updates. Read with
    /// [`last_status`](MockTray::last_status).
    pub status_rx: Receiver<DaemonStatus>,
}

impl std::fmt::Debug for MockTray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockTray").finish_non_exhaustive()
    }
}

impl Default for MockTray {
    fn default() -> Self {
        Self::new()
    }
}

impl MockTray {
    /// Create a new `MockTray`.
    #[must_use]
    pub fn new() -> Self {
        let (event_tx, event_rx) = unbounded();
        let (status_tx, status_rx) = unbounded();
        Self {
            event_tx,
            event_rx,
            status_tx,
            status_rx,
        }
    }

    /// Inject a [`TrayEvent`] as if the user had clicked the corresponding menu
    /// item.  The next call to [`Tray::recv_event`] or
    /// [`Tray::try_recv_event`] will return it.
    ///
    /// # Errors
    ///
    /// Returns [`TrayError::ChannelClosed`] if the receiver has been dropped.
    pub fn inject_event(&self, event: TrayEvent) -> Result<(), TrayError> {
        self.event_tx
            .send(event)
            .map_err(|_| TrayError::ChannelClosed)
    }

    /// Return the most recent [`DaemonStatus`] passed to [`Tray::set_status`],
    /// or `None` if [`set_status`](Tray::set_status) has not been called yet.
    #[must_use]
    pub fn last_status(&self) -> Option<DaemonStatus> {
        // Drain the channel and return the last value.
        let mut last = None;
        while let Ok(s) = self.status_rx.try_recv() {
            last = Some(s);
        }
        last
    }
}

impl Tray for MockTray {
    fn set_status(&self, status: DaemonStatus) -> Result<(), TrayError> {
        tracing::debug!(?status, "MockTray::set_status");
        self.status_tx
            .send(status)
            .map_err(|_| TrayError::ChannelClosed)
    }

    fn recv_event(&self) -> Option<TrayEvent> {
        self.event_rx.recv().ok()
    }

    fn try_recv_event(&self) -> Option<TrayEvent> {
        self.event_rx.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inject_and_receive_event() {
        let tray = MockTray::new();
        tray.inject_event(TrayEvent::StartRecording).unwrap();
        assert_eq!(tray.try_recv_event(), Some(TrayEvent::StartRecording));
    }

    #[test]
    fn set_status_recorded() {
        let tray = MockTray::new();
        tray.set_status(DaemonStatus::Recording).unwrap();
        assert_eq!(tray.last_status(), Some(DaemonStatus::Recording));
    }

    #[test]
    fn try_recv_event_empty_returns_none() {
        let tray = MockTray::new();
        assert_eq!(tray.try_recv_event(), None);
    }

    #[test]
    fn multiple_status_updates_last_wins() {
        let tray = MockTray::new();
        tray.set_status(DaemonStatus::Idle).unwrap();
        tray.set_status(DaemonStatus::Recording).unwrap();
        tray.set_status(DaemonStatus::Processing).unwrap();
        assert_eq!(tray.last_status(), Some(DaemonStatus::Processing));
    }

    #[test]
    fn inject_multiple_events_fifo() {
        let tray = MockTray::new();
        tray.inject_event(TrayEvent::StartRecording).unwrap();
        tray.inject_event(TrayEvent::OpenSettings).unwrap();
        assert_eq!(tray.try_recv_event(), Some(TrayEvent::StartRecording));
        assert_eq!(tray.try_recv_event(), Some(TrayEvent::OpenSettings));
        assert_eq!(tray.try_recv_event(), None);
    }

    #[test]
    fn mock_tray_is_object_safe() {
        let tray: Box<dyn Tray> = Box::new(MockTray::new());
        assert!(tray.set_status(DaemonStatus::Idle).is_ok());
        assert_eq!(tray.try_recv_event(), None);
    }
}
