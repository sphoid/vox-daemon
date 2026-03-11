//! Transcript browser data model.
//!
//! [`SessionListEntry`] is a lightweight summary of a session suitable for
//! displaying in a scrollable list — it avoids holding the full transcript in
//! memory for every row.
//!
//! [`build_session_list`] converts a slice of full [`Session`] objects (as
//! returned by [`vox_storage::store::SessionStore::list`]) into a `Vec` of
//! list entries. The iced browser view can then load a full session only when
//! the user selects one.

use chrono::{DateTime, Utc};
use uuid::Uuid;
use vox_core::session::Session;

/// Lightweight entry for the session list view.
///
/// All expensive data (transcript text, audio path, etc.) is omitted so the
/// list can be rendered quickly even with hundreds of sessions.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionListEntry {
    /// Unique session identifier — used to load the full session on demand.
    pub id: Uuid,

    /// When the session was created (UTC).
    pub created_at: DateTime<Utc>,

    /// Session duration in seconds.
    pub duration_seconds: u64,

    /// Number of transcript segments.
    pub segment_count: usize,

    /// Short preview of the AI summary (first 120 characters of the overview),
    /// or `None` if no summary has been generated yet.
    pub summary_preview: Option<String>,
}

impl SessionListEntry {
    /// Build a [`SessionListEntry`] from a full [`Session`].
    #[must_use]
    pub fn from_session(session: &Session) -> Self {
        let summary_preview = session.summary.as_ref().map(|s| {
            let overview = s.overview.trim();
            if overview.chars().count() > 120 {
                let truncated: String = overview.chars().take(120).collect();
                format!("{truncated}…")
            } else {
                overview.to_owned()
            }
        });

        Self {
            id: session.id,
            created_at: session.created_at,
            duration_seconds: session.duration_seconds,
            segment_count: session.transcript.len(),
            summary_preview,
        }
    }

    /// Returns a human-readable duration string (e.g. `"1h 02m 30s"`).
    #[must_use]
    pub fn formatted_duration(&self) -> String {
        format_duration(self.duration_seconds)
    }

    /// Returns the creation date formatted as `"YYYY-MM-DD HH:MM"` (UTC).
    #[must_use]
    pub fn formatted_date(&self) -> String {
        self.created_at.format("%Y-%m-%d %H:%M").to_string()
    }
}

/// Convert a slice of full sessions into a `Vec` of lightweight list entries.
///
/// The order of the output matches the order of the input. Callers that need
/// the list sorted should sort the sessions first (e.g. via
/// [`vox_storage::store::SessionStore::list`], which already sorts
/// newest-first).
#[must_use]
pub fn build_session_list(sessions: &[Session]) -> Vec<SessionListEntry> {
    sessions
        .iter()
        .map(SessionListEntry::from_session)
        .collect()
}

/// Format a duration in seconds as a human-readable string.
///
/// Examples:
/// - `0` → `"0s"`
/// - `90` → `"1m 30s"`
/// - `3750` → `"1h 02m 30s"`
#[must_use]
pub fn format_duration(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;

    if hours > 0 {
        format!("{hours}h {minutes:02}m {secs:02}s")
    } else if minutes > 0 {
        format!("{minutes}m {secs:02}s")
    } else {
        format!("{secs}s")
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use vox_core::session::{
        ActionItem, AudioRole, AudioSourceInfo, ConfigSnapshot, Session, Summary, TranscriptSegment,
    };

    use super::*;

    fn make_session() -> Session {
        Session::new(
            vec![AudioSourceInfo {
                name: "Mic".to_owned(),
                pipewire_node_id: 1,
                role: AudioRole::Microphone,
            }],
            ConfigSnapshot {
                model: "base".to_owned(),
                language: "en".to_owned(),
                gpu_backend: "auto".to_owned(),
            },
        )
    }

    #[test]
    fn test_entry_from_session_no_summary() {
        let session = make_session();
        let entry = SessionListEntry::from_session(&session);
        assert_eq!(entry.id, session.id);
        assert_eq!(entry.duration_seconds, 0);
        assert_eq!(entry.segment_count, 0);
        assert!(entry.summary_preview.is_none());
    }

    #[test]
    fn test_entry_from_session_with_summary() {
        let mut session = make_session();
        session.summary = Some(Summary {
            generated_at: Utc::now(),
            backend: "builtin".to_owned(),
            model: "test".to_owned(),
            overview: "This was a productive meeting about Q1 planning.".to_owned(),
            key_points: vec![],
            action_items: vec![ActionItem {
                description: "Send slides".to_owned(),
                owner: None,
            }],
            decisions: vec![],
        });

        let entry = SessionListEntry::from_session(&session);
        assert_eq!(
            entry.summary_preview.as_deref(),
            Some("This was a productive meeting about Q1 planning.")
        );
    }

    #[test]
    fn test_entry_summary_preview_truncated_at_120_chars() {
        let mut session = make_session();
        let long_overview = "A".repeat(200);
        session.summary = Some(Summary {
            generated_at: Utc::now(),
            backend: "builtin".to_owned(),
            model: "test".to_owned(),
            overview: long_overview,
            key_points: vec![],
            action_items: vec![],
            decisions: vec![],
        });

        let entry = SessionListEntry::from_session(&session);
        let preview = entry.summary_preview.expect("should have preview");
        assert!(preview.ends_with('…'), "should end with ellipsis");
        // 120 chars of content + 1 ellipsis = 121 chars total.
        assert_eq!(preview.chars().count(), 121);
    }

    #[test]
    fn test_entry_segment_count() {
        let mut session = make_session();
        session.transcript = vec![
            TranscriptSegment {
                start_time: 0.0,
                end_time: 5.0,
                speaker: "You".to_owned(),
                text: "Hello".to_owned(),
            },
            TranscriptSegment {
                start_time: 5.0,
                end_time: 10.0,
                speaker: "Remote".to_owned(),
                text: "Hi there".to_owned(),
            },
        ];
        let entry = SessionListEntry::from_session(&session);
        assert_eq!(entry.segment_count, 2);
    }

    #[test]
    fn test_formatted_duration_zero() {
        assert_eq!(format_duration(0), "0s");
    }

    #[test]
    fn test_formatted_duration_seconds_only() {
        assert_eq!(format_duration(45), "45s");
    }

    #[test]
    fn test_formatted_duration_minutes_and_seconds() {
        assert_eq!(format_duration(90), "1m 30s");
    }

    #[test]
    fn test_formatted_duration_hours_minutes_seconds() {
        assert_eq!(format_duration(3750), "1h 02m 30s");
    }

    #[test]
    fn test_formatted_duration_exact_hour() {
        assert_eq!(format_duration(3600), "1h 00m 00s");
    }

    #[test]
    fn test_formatted_date_format() {
        let entry = SessionListEntry {
            id: Uuid::new_v4(),
            created_at: chrono::DateTime::parse_from_rfc3339("2026-03-10T14:30:00Z")
                .expect("parse")
                .with_timezone(&Utc),
            duration_seconds: 0,
            segment_count: 0,
            summary_preview: None,
        };
        assert_eq!(entry.formatted_date(), "2026-03-10 14:30");
    }

    #[test]
    fn test_build_session_list_preserves_order() {
        let mut s1 = make_session();
        let mut s2 = make_session();
        s1.duration_seconds = 100;
        s2.duration_seconds = 200;

        let list = build_session_list(&[s1.clone(), s2.clone()]);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, s1.id);
        assert_eq!(list[1].id, s2.id);
    }

    #[test]
    fn test_build_session_list_empty() {
        let list = build_session_list(&[]);
        assert!(list.is_empty());
    }
}
