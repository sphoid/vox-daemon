//! Markdown rendering for [`Session`] transcripts and summaries.
//!
//! The output format is designed to be human-readable and suitable for saving
//! to a `.md` file or pasting into a note-taking application.

use std::fmt::Write as _;

use vox_core::session::Session;

/// Render a [`Session`] as a Markdown document.
///
/// The returned string includes:
/// - A heading with the session date.
/// - Duration and participant list.
/// - The full transcript with per-segment speaker labels and timestamps.
/// - An optional AI-generated summary section if one is present.
#[must_use]
pub fn render(session: &Session) -> String {
    let mut out = String::with_capacity(4096);

    render_header(session, &mut out);
    render_transcript(session, &mut out);
    render_summary(session, &mut out);

    out
}

/// Write the document header (title, metadata).
fn render_header(session: &Session, out: &mut String) {
    let date = session.created_at.format("%Y-%m-%d");
    let _ = write!(out, "# Meeting Transcript — {date}\n\n");

    // Duration
    let duration = format_duration(session.duration_seconds);
    let _ = writeln!(out, "**Duration:** {duration}");

    // Participants — collect unique friendly names from speaker mappings, or
    // fall back to unique speaker IDs from transcript segments.
    let participants = collect_participants(session);
    if participants.is_empty() {
        out.push_str("**Participants:** *(unknown)*\n");
    } else {
        let _ = writeln!(out, "**Participants:** {}", participants.join(", "));
    }

    out.push_str("\n---\n\n");
}

/// Write the transcript section.
fn render_transcript(session: &Session, out: &mut String) {
    out.push_str("## Transcript\n\n");

    if session.transcript.is_empty() {
        out.push_str("*(no transcript available)*\n\n");
        return;
    }

    for segment in &session.transcript {
        let start = format_timestamp(segment.start_time);
        let end = format_timestamp(segment.end_time);
        let speaker = resolve_speaker(session, &segment.speaker);

        let _ = write!(
            out,
            "**[{start} - {end}] {speaker}:**\n{}\n\n",
            segment.text.trim()
        );
    }
}

/// Write the summary section, if one exists.
fn render_summary(session: &Session, out: &mut String) {
    let Some(summary) = &session.summary else {
        return;
    };

    out.push_str("---\n\n## Summary\n\n");
    out.push_str(&summary.overview);
    out.push_str("\n\n");

    if !summary.key_points.is_empty() {
        out.push_str("### Key Points\n\n");
        for point in &summary.key_points {
            let _ = writeln!(out, "- {point}");
        }
        out.push('\n');
    }

    if !summary.action_items.is_empty() {
        out.push_str("### Action Items\n\n");
        for item in &summary.action_items {
            match &item.owner {
                Some(owner) => {
                    let _ = writeln!(out, "- [ ] {} ({})", item.description, owner);
                }
                None => {
                    let _ = writeln!(out, "- [ ] {}", item.description);
                }
            }
        }
        out.push('\n');
    }

    if !summary.decisions.is_empty() {
        out.push_str("### Decisions\n\n");
        for decision in &summary.decisions {
            let _ = writeln!(out, "- {decision}");
        }
        out.push('\n');
    }
}

/// Convert seconds to a `HH:MM:SS` timestamp string.
///
/// Hours are omitted when zero, giving `MM:SS` instead.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn format_timestamp(seconds: f64) -> String {
    let total = seconds as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// Format a duration in seconds to a human-readable string such as `"1h 02m 30s"`.
#[must_use]
fn format_duration(seconds: u64) -> String {
    if seconds == 0 {
        return "0s".to_owned();
    }
    let h = seconds / 3600;
    let m = (seconds % 3600) / 60;
    let s = seconds % 60;

    let mut parts = Vec::with_capacity(3);
    if h > 0 {
        parts.push(format!("{h}h"));
    }
    if m > 0 {
        parts.push(format!("{m:02}m"));
    }
    if s > 0 || parts.is_empty() {
        parts.push(format!("{s:02}s"));
    }
    parts.join(" ")
}

/// Return the friendly display name for a speaker identifier.
///
/// Looks up `speaker_id` in the session's speaker mappings; if not found,
/// returns the raw ID as-is.
fn resolve_speaker<'a>(session: &'a Session, speaker_id: &'a str) -> &'a str {
    session
        .speakers
        .iter()
        .find(|m| m.id == speaker_id)
        .map_or(speaker_id, |m| m.friendly_name.as_str())
}

/// Collect an ordered, deduplicated list of participant display names.
///
/// Prefers friendly names from speaker mappings; falls back to speaker IDs
/// found in transcript segments.
fn collect_participants(session: &Session) -> Vec<String> {
    if !session.speakers.is_empty() {
        return session
            .speakers
            .iter()
            .map(|m| m.friendly_name.clone())
            .collect();
    }

    // Fall back to unique speaker IDs from the transcript.
    let mut seen = Vec::new();
    for segment in &session.transcript {
        if !seen.contains(&segment.speaker) {
            seen.push(segment.speaker.clone());
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use vox_core::session::{
        ActionItem, AudioRole, AudioSourceInfo, ConfigSnapshot, Session, SpeakerMapping,
        SpeakerSource, Summary, TranscriptSegment,
    };

    use super::*;

    fn base_session() -> Session {
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
    fn test_format_timestamp_minutes_and_seconds() {
        assert_eq!(format_timestamp(0.0), "00:00");
        assert_eq!(format_timestamp(15.0), "00:15");
        assert_eq!(format_timestamp(75.5), "01:15");
    }

    #[test]
    fn test_format_timestamp_with_hours() {
        assert_eq!(format_timestamp(3661.0), "01:01:01");
    }

    #[test]
    fn test_format_duration_zero() {
        assert_eq!(format_duration(0), "0s");
    }

    #[test]
    fn test_format_duration_mixed() {
        assert_eq!(format_duration(3690), "1h 01m 30s");
        assert_eq!(format_duration(60), "01m");
        assert_eq!(format_duration(45), "45s");
    }

    #[test]
    fn test_render_header_contains_date() {
        let session = base_session();
        let md = render(&session);
        // Date format YYYY-MM-DD should appear in the title.
        let today = Utc::now().format("%Y-%m-%d").to_string();
        assert!(md.contains(&today), "header should contain today's date");
        assert!(md.contains("**Duration:**"));
    }

    #[test]
    fn test_render_empty_transcript() {
        let session = base_session();
        let md = render(&session);
        assert!(md.contains("*(no transcript available)*"));
    }

    #[test]
    fn test_render_transcript_segments() {
        let mut session = base_session();
        session.transcript = vec![
            TranscriptSegment {
                start_time: 15.0,
                end_time: 32.0,
                speaker: "You".to_owned(),
                text: "Hello everyone, let's get started.".to_owned(),
            },
            TranscriptSegment {
                start_time: 33.0,
                end_time: 45.0,
                speaker: "Remote".to_owned(),
                text: "Sure, sounds good.".to_owned(),
            },
        ];
        let md = render(&session);
        assert!(md.contains("[00:15 - 00:32] You:"));
        assert!(md.contains("[00:33 - 00:45] Remote:"));
        assert!(md.contains("Hello everyone, let's get started."));
        assert!(md.contains("Sure, sounds good."));
    }

    #[test]
    fn test_render_speaker_mapping_resolution() {
        let mut session = base_session();
        session.speakers = vec![SpeakerMapping {
            id: "speaker_0".to_owned(),
            friendly_name: "Alice".to_owned(),
            source: SpeakerSource::Microphone,
        }];
        session.transcript = vec![TranscriptSegment {
            start_time: 5.0,
            end_time: 10.0,
            speaker: "speaker_0".to_owned(),
            text: "Hi there.".to_owned(),
        }];
        let md = render(&session);
        // Should resolve "speaker_0" -> "Alice"
        assert!(md.contains("] Alice:**"), "should use friendly name");
        assert!(!md.contains("speaker_0"), "raw ID should not appear");
    }

    #[test]
    fn test_render_participants_from_mappings() {
        let mut session = base_session();
        session.speakers = vec![
            SpeakerMapping {
                id: "s0".to_owned(),
                friendly_name: "You".to_owned(),
                source: SpeakerSource::Microphone,
            },
            SpeakerMapping {
                id: "s1".to_owned(),
                friendly_name: "Bob".to_owned(),
                source: SpeakerSource::Remote,
            },
        ];
        let md = render(&session);
        assert!(md.contains("You, Bob") || md.contains("Bob, You"));
    }

    #[test]
    fn test_render_participants_fallback_to_transcript() {
        let mut session = base_session();
        session.transcript = vec![
            TranscriptSegment {
                start_time: 0.0,
                end_time: 1.0,
                speaker: "You".to_owned(),
                text: "Hello.".to_owned(),
            },
            TranscriptSegment {
                start_time: 1.0,
                end_time: 2.0,
                speaker: "Remote".to_owned(),
                text: "Hi.".to_owned(),
            },
        ];
        let md = render(&session);
        assert!(md.contains("You") && md.contains("Remote"));
    }

    #[test]
    fn test_render_no_summary_section() {
        let session = base_session();
        let md = render(&session);
        assert!(!md.contains("## Summary"), "no summary section if absent");
    }

    #[test]
    fn test_render_full_summary() {
        let mut session = base_session();
        session.summary = Some(Summary {
            generated_at: Utc::now(),
            backend: "ollama".to_owned(),
            model: "qwen2.5:1.5b".to_owned(),
            overview: "Overview of meeting.".to_owned(),
            key_points: vec!["Point 1".to_owned(), "Point 2".to_owned()],
            action_items: vec![
                ActionItem {
                    description: "Send report".to_owned(),
                    owner: Some("Alice".to_owned()),
                },
                ActionItem {
                    description: "Book venue".to_owned(),
                    owner: None,
                },
            ],
            decisions: vec!["Use Rust for the backend.".to_owned()],
        });

        let md = render(&session);
        assert!(md.contains("## Summary"));
        assert!(md.contains("Overview of meeting."));
        assert!(md.contains("### Key Points"));
        assert!(md.contains("- Point 1"));
        assert!(md.contains("- Point 2"));
        assert!(md.contains("### Action Items"));
        assert!(md.contains("- [ ] Send report (Alice)"));
        assert!(md.contains("- [ ] Book venue"));
        assert!(md.contains("### Decisions"));
        assert!(md.contains("- Use Rust for the backend."));
    }

    #[test]
    fn test_render_summary_no_key_points() {
        let mut session = base_session();
        session.summary = Some(Summary {
            generated_at: Utc::now(),
            backend: "builtin".to_owned(),
            model: "test".to_owned(),
            overview: "Short call.".to_owned(),
            key_points: vec![],
            action_items: vec![],
            decisions: vec![],
        });
        let md = render(&session);
        assert!(md.contains("## Summary"));
        assert!(!md.contains("### Key Points"));
        assert!(!md.contains("### Action Items"));
        assert!(!md.contains("### Decisions"));
    }
}
