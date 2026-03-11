//! [`DesktopNotifier`] — real XDG desktop notification implementation.

use std::time::Duration;

use notify_rust::{Notification, Urgency};
use uuid::Uuid;
use vox_core::config::NotificationConfig;

use crate::{Notifier, NotifyError};

/// Application name shown in all notifications.
const APP_NAME: &str = "Vox Daemon";

/// A [`Notifier`] that sends real XDG desktop notifications via D-Bus.
///
/// Each notification method checks the corresponding [`NotificationConfig`]
/// flag before emitting. When `config.enabled` is `false`, no notification is
/// sent regardless of individual flags.
///
/// # Platform support
///
/// `DesktopNotifier` works on any Linux desktop that implements the
/// `org.freedesktop.Notifications` D-Bus specification, including GNOME,
/// KDE Plasma, XFCE, and compositors such as Sway that include a notification
/// daemon.
#[derive(Debug, Clone)]
pub struct DesktopNotifier {
    config: NotificationConfig,
}

impl DesktopNotifier {
    /// Create a new `DesktopNotifier` with the given notification config.
    #[must_use]
    pub fn new(config: NotificationConfig) -> Self {
        Self { config }
    }

    /// Update the notification config at runtime.
    ///
    /// For example, call this after the user saves changed preferences in the
    /// settings window.
    pub fn set_config(&mut self, config: NotificationConfig) {
        self.config = config;
    }

    /// Returns a reference to the current notification config.
    #[must_use]
    pub fn config(&self) -> &NotificationConfig {
        &self.config
    }

    /// Send a notification, wrapping any D-Bus / notify-rust error.
    fn send(notification: &mut Notification) -> Result<(), NotifyError> {
        notification.show()?;
        Ok(())
    }
}

impl Notifier for DesktopNotifier {
    fn recording_started(&self) -> Result<(), NotifyError> {
        if !self.config.enabled || !self.config.on_record_start {
            tracing::debug!("recording_started notification suppressed by config");
            return Ok(());
        }
        tracing::debug!("sending recording_started notification");
        Self::send(
            Notification::new()
                .appname(APP_NAME)
                .summary("Recording started")
                .body("Vox Daemon is now capturing audio.")
                .urgency(Urgency::Low),
        )
    }

    fn recording_stopped(&self, duration: Duration) -> Result<(), NotifyError> {
        if !self.config.enabled || !self.config.on_record_stop {
            tracing::debug!("recording_stopped notification suppressed by config");
            return Ok(());
        }
        let body = format!(
            "Recording finished. Duration: {}.",
            format_duration(duration)
        );
        tracing::debug!("sending recording_stopped notification");
        Self::send(
            Notification::new()
                .appname(APP_NAME)
                .summary("Recording stopped")
                .body(&body)
                .urgency(Urgency::Low),
        )
    }

    fn transcript_ready(&self, session_id: Uuid) -> Result<(), NotifyError> {
        if !self.config.enabled || !self.config.on_transcript_ready {
            tracing::debug!("transcript_ready notification suppressed by config");
            return Ok(());
        }
        let body = format!("Transcript is ready. Session: {session_id}.");
        tracing::debug!(%session_id, "sending transcript_ready notification");
        Self::send(
            Notification::new()
                .appname(APP_NAME)
                .summary("Transcript ready")
                .body(&body)
                .urgency(Urgency::Normal),
        )
    }

    fn summary_ready(&self, session_id: Uuid) -> Result<(), NotifyError> {
        if !self.config.enabled || !self.config.on_summary_ready {
            tracing::debug!("summary_ready notification suppressed by config");
            return Ok(());
        }
        let body = format!("AI summary is ready. Session: {session_id}.");
        tracing::debug!(%session_id, "sending summary_ready notification");
        Self::send(
            Notification::new()
                .appname(APP_NAME)
                .summary("Summary ready")
                .body(&body)
                .urgency(Urgency::Normal),
        )
    }
}

/// Format a [`Duration`] as a human-readable string (e.g., `"1h 02m 30s"`).
fn format_duration(duration: Duration) -> String {
    let total = duration.as_secs();
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let secs = total % 60;

    if hours > 0 {
        format!("{hours}h {minutes:02}m {secs:02}s")
    } else if minutes > 0 {
        format!("{minutes}m {secs:02}s")
    } else {
        format!("{secs}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_duration_seconds_only() {
        assert_eq!(format_duration(Duration::from_secs(45)), "45s");
    }

    #[test]
    fn format_duration_minutes_and_seconds() {
        assert_eq!(format_duration(Duration::from_secs(90)), "1m 30s");
    }

    #[test]
    fn format_duration_hours() {
        assert_eq!(format_duration(Duration::from_secs(3690)), "1h 01m 30s");
    }

    #[test]
    fn format_duration_zero() {
        assert_eq!(format_duration(Duration::ZERO), "0s");
    }

    #[test]
    fn desktop_notifier_suppressed_when_disabled() {
        let mut config = NotificationConfig::default();
        config.enabled = false;
        let notifier = DesktopNotifier::new(config);
        // Should succeed silently without contacting D-Bus.
        assert!(notifier.recording_started().is_ok());
        assert!(notifier.recording_stopped(Duration::from_secs(60)).is_ok());
        assert!(notifier.transcript_ready(Uuid::new_v4()).is_ok());
        assert!(notifier.summary_ready(Uuid::new_v4()).is_ok());
    }

    #[test]
    fn desktop_notifier_individual_flags_suppress() {
        let config = NotificationConfig {
            enabled: true,
            on_record_start: false,
            on_record_stop: false,
            on_transcript_ready: false,
            on_summary_ready: false,
        };
        let notifier = DesktopNotifier::new(config);
        // All suppressed — no D-Bus call made.
        assert!(notifier.recording_started().is_ok());
        assert!(notifier.recording_stopped(Duration::from_secs(10)).is_ok());
        assert!(notifier.transcript_ready(Uuid::new_v4()).is_ok());
        assert!(notifier.summary_ready(Uuid::new_v4()).is_ok());
    }

    #[test]
    fn desktop_notifier_set_config() {
        let config = NotificationConfig::default();
        let mut notifier = DesktopNotifier::new(config);
        let new_config = NotificationConfig {
            enabled: false,
            ..NotificationConfig::default()
        };
        notifier.set_config(new_config.clone());
        assert_eq!(notifier.config().enabled, false);
    }

    #[test]
    fn desktop_notifier_is_object_safe() {
        let config = NotificationConfig {
            enabled: false,
            ..NotificationConfig::default()
        };
        let n: Box<dyn Notifier> = Box::new(DesktopNotifier::new(config));
        assert!(n.recording_started().is_ok());
    }
}
