//! Core [`Notifier`] trait and [`StubNotifier`] implementation.

use std::time::Duration;

use uuid::Uuid;

use crate::error::NotifyError;

/// Notification API for Vox Daemon lifecycle events.
///
/// Implementors decide whether to send real desktop notifications
/// (`DesktopNotifier`) or log them instead (`StubNotifier`).
///
/// Each method checks the relevant `NotificationConfig` flag before emitting
/// anything; callers do not need to guard the calls themselves.
pub trait Notifier: Send + Sync {
    /// Notify that recording has started.
    ///
    /// # Errors
    ///
    /// Returns [`NotifyError`] if the notification could not be sent.
    fn recording_started(&self) -> Result<(), NotifyError>;

    /// Notify that recording has stopped.
    ///
    /// `duration` is the total length of the recorded session.
    ///
    /// # Errors
    ///
    /// Returns [`NotifyError`] if the notification could not be sent.
    fn recording_stopped(&self, duration: Duration) -> Result<(), NotifyError>;

    /// Notify that a transcript is ready for viewing.
    ///
    /// `session_id` identifies the session so callers can open the right
    /// transcript when the notification is clicked.
    ///
    /// # Errors
    ///
    /// Returns [`NotifyError`] if the notification could not be sent.
    fn transcript_ready(&self, session_id: Uuid) -> Result<(), NotifyError>;

    /// Notify that an AI summary has been generated.
    ///
    /// `session_id` identifies the session so callers can open the right
    /// transcript when the notification is clicked.
    ///
    /// # Errors
    ///
    /// Returns [`NotifyError`] if the notification could not be sent.
    fn summary_ready(&self, session_id: Uuid) -> Result<(), NotifyError>;
}

/// A [`Notifier`] that logs events with [`tracing`] instead of sending real
/// desktop notifications.
///
/// Useful in tests, CI environments, and contexts where a D-Bus session is
/// unavailable.
#[derive(Debug, Default)]
pub struct StubNotifier;

impl StubNotifier {
    /// Create a new `StubNotifier`.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Notifier for StubNotifier {
    fn recording_started(&self) -> Result<(), NotifyError> {
        tracing::info!("[StubNotifier] recording started");
        Ok(())
    }

    fn recording_stopped(&self, duration: Duration) -> Result<(), NotifyError> {
        tracing::info!(
            duration_secs = duration.as_secs(),
            "[StubNotifier] recording stopped"
        );
        Ok(())
    }

    fn transcript_ready(&self, session_id: Uuid) -> Result<(), NotifyError> {
        tracing::info!(%session_id, "[StubNotifier] transcript ready");
        Ok(())
    }

    fn summary_ready(&self, session_id: Uuid) -> Result<(), NotifyError> {
        tracing::info!(%session_id, "[StubNotifier] summary ready");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_notifier_recording_started() {
        let n = StubNotifier::new();
        assert!(n.recording_started().is_ok());
    }

    #[test]
    fn stub_notifier_recording_stopped() {
        let n = StubNotifier::new();
        assert!(n.recording_stopped(Duration::from_secs(42)).is_ok());
    }

    #[test]
    fn stub_notifier_transcript_ready() {
        let n = StubNotifier::new();
        assert!(n.transcript_ready(Uuid::new_v4()).is_ok());
    }

    #[test]
    fn stub_notifier_summary_ready() {
        let n = StubNotifier::new();
        assert!(n.summary_ready(Uuid::new_v4()).is_ok());
    }

    #[test]
    fn stub_notifier_is_object_safe() {
        // Ensure the trait can be used as a trait object.
        let n: Box<dyn Notifier> = Box::new(StubNotifier::new());
        assert!(n.recording_started().is_ok());
    }
}
