//! Public event and status types for the system tray.

/// Events emitted by the tray icon when the user interacts with the popup menu.
///
/// The daemon listens for these events on the receiver end of a
/// [`crossbeam_channel`] and acts accordingly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayEvent {
    /// The user clicked "Start Recording".
    StartRecording,

    /// The user clicked "Stop Recording".
    StopRecording,

    /// The user clicked "Pause Recording".
    PauseRecording,

    /// The user clicked "Open Latest Transcript".
    OpenLastTranscript,

    /// The user clicked "Browse Transcripts…".
    BrowseTranscripts,

    /// The user clicked "Settings…".
    OpenSettings,

    /// The user clicked "Quit".
    Quit,
}

/// The current operational status of the daemon.
///
/// Pass this to [`Tray::set_status`] to update the tray icon and menu items
/// to reflect what the daemon is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DaemonStatus {
    /// The daemon is idle — not recording or processing.
    #[default]
    Idle,

    /// The daemon is actively recording audio.
    Recording,

    /// The daemon is processing (transcribing or summarising) a past session.
    Processing,
}

impl DaemonStatus {
    /// Returns a short human-readable label for this status.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Recording => "Recording",
            Self::Processing => "Processing",
        }
    }

    /// Returns `true` when the daemon is actively recording.
    #[must_use]
    pub fn is_recording(self) -> bool {
        self == Self::Recording
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_status_default_is_idle() {
        assert_eq!(DaemonStatus::default(), DaemonStatus::Idle);
    }

    #[test]
    fn daemon_status_label() {
        assert_eq!(DaemonStatus::Idle.label(), "Idle");
        assert_eq!(DaemonStatus::Recording.label(), "Recording");
        assert_eq!(DaemonStatus::Processing.label(), "Processing");
    }

    #[test]
    fn daemon_status_is_recording() {
        assert!(!DaemonStatus::Idle.is_recording());
        assert!(DaemonStatus::Recording.is_recording());
        assert!(!DaemonStatus::Processing.is_recording());
    }

    #[test]
    fn tray_event_debug() {
        // Ensure all variants are reachable and produce debug output.
        let events = [
            TrayEvent::StartRecording,
            TrayEvent::StopRecording,
            TrayEvent::PauseRecording,
            TrayEvent::OpenLastTranscript,
            TrayEvent::BrowseTranscripts,
            TrayEvent::OpenSettings,
            TrayEvent::Quit,
        ];
        for e in &events {
            assert!(!format!("{e:?}").is_empty());
        }
    }
}
