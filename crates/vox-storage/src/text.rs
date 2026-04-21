//! Plain-text rendering for [`Session`] transcripts and summaries.
//!
//! Produces output without any Markdown formatting — suitable for `.txt` files.

use std::fmt::Write as _;

use vox_core::session::Session;

use crate::markdown::{
    RenderOptions, collect_participants, format_duration, format_timestamp, resolve_speaker,
};

/// Render a [`Session`] as plain text with the given options.
#[must_use]
pub fn render(session: &Session, options: &RenderOptions) -> String {
    let mut out = String::with_capacity(4096);

    render_header(session, &mut out);
    if options.include_transcript {
        render_transcript(session, &mut out);
    }
    if options.include_summary {
        render_summary(session, &mut out);
    }

    out
}

/// Write a plain-text header.
fn render_header(session: &Session, out: &mut String) {
    let date = session.created_at.format("%Y-%m-%d");
    let _ = writeln!(out, "Meeting Transcript — {date}");
    let _ = writeln!(out);

    let duration = format_duration(session.duration_seconds);
    let _ = writeln!(out, "Duration: {duration}");

    let participants = collect_participants(session);
    if participants.is_empty() {
        let _ = writeln!(out, "Participants: (unknown)");
    } else {
        let _ = writeln!(out, "Participants: {}", participants.join(", "));
    }

    let _ = writeln!(out);
}

/// Write the transcript section.
fn render_transcript(session: &Session, out: &mut String) {
    let _ = writeln!(out, "Transcript");
    let _ = writeln!(out, "{}", "-".repeat(40));
    let _ = writeln!(out);

    if session.transcript.is_empty() {
        let _ = writeln!(out, "(no transcript available)");
        let _ = writeln!(out);
        return;
    }

    for segment in &session.transcript {
        let start = format_timestamp(segment.start_time);
        let end = format_timestamp(segment.end_time);
        let speaker = resolve_speaker(session, &segment.speaker);

        let _ = writeln!(out, "[{start} - {end}] {speaker}:");
        let _ = writeln!(out, "{}", segment.text.trim());
        let _ = writeln!(out);
    }
}

/// Write the summary section, if present.
fn render_summary(session: &Session, out: &mut String) {
    let Some(summary) = &session.summary else {
        return;
    };

    let _ = writeln!(out, "Summary");
    let _ = writeln!(out, "{}", "-".repeat(40));
    let _ = writeln!(out);
    let _ = writeln!(out, "{}", summary.overview);
    let _ = writeln!(out);

    if !summary.key_points.is_empty() {
        let _ = writeln!(out, "Key Points:");
        for point in &summary.key_points {
            let _ = writeln!(out, "  - {point}");
        }
        let _ = writeln!(out);
    }

    if !summary.action_items.is_empty() {
        let _ = writeln!(out, "Action Items:");
        for item in &summary.action_items {
            match &item.owner {
                Some(owner) => {
                    let _ = writeln!(out, "  [ ] {} ({})", item.description, owner);
                }
                None => {
                    let _ = writeln!(out, "  [ ] {}", item.description);
                }
            }
        }
        let _ = writeln!(out);
    }

    if !summary.decisions.is_empty() {
        let _ = writeln!(out, "Decisions:");
        for decision in &summary.decisions {
            let _ = writeln!(out, "  - {decision}");
        }
        let _ = writeln!(out);
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use vox_core::session::{
        AudioRole, AudioSourceInfo, ConfigSnapshot, Session, Summary, TranscriptSegment,
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
                diarization_mode: "none".to_owned(),
                decoding_strategy: String::new(),
                initial_prompt: String::new(),
            },
        )
    }

    #[test]
    fn test_text_render_header() {
        let session = base_session();
        let txt = render(&session, &RenderOptions::default());
        assert!(txt.contains("Meeting Transcript"));
        assert!(txt.contains("Duration:"));
        assert!(!txt.contains('#'));
        assert!(!txt.contains("**"));
    }

    #[test]
    fn test_text_render_transcript() {
        let mut session = base_session();
        session.transcript = vec![TranscriptSegment {
            start_time: 10.0,
            end_time: 20.0,
            speaker: "You".to_owned(),
            text: "Hello.".to_owned(),
        }];
        let txt = render(&session, &RenderOptions::default());
        assert!(txt.contains("[00:10 - 00:20] You:"));
        assert!(txt.contains("Hello."));
    }

    #[test]
    fn test_text_render_summary_only() {
        let mut session = base_session();
        session.summary = Some(Summary {
            generated_at: Utc::now(),
            backend: "test".to_owned(),
            model: "test".to_owned(),
            overview: "Overview text.".to_owned(),
            key_points: vec!["Point A".to_owned()],
            action_items: vec![],
            decisions: vec![],
        });
        let opts = RenderOptions {
            include_transcript: false,
            include_summary: true,
        };
        let txt = render(&session, &opts);
        // The header always contains "Meeting Transcript" as the title,
        // but the "Transcript\n---" section should be absent.
        assert!(
            !txt.contains("Transcript\n----------------------------------------"),
            "transcript section should not be present"
        );
        assert!(txt.contains("Summary"));
        assert!(txt.contains("Overview text."));
    }
}
