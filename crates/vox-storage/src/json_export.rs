//! JSON export for [`Session`] data.
//!
//! Exports a selectively filtered version of the session as pretty-printed
//! JSON, controlled by [`RenderOptions`].

use serde_json::{Map, Value, json};
use vox_core::session::Session;

use crate::markdown::RenderOptions;

/// Render a [`Session`] as a pretty-printed JSON string.
///
/// Always includes session metadata (id, date, duration, participants).
/// Transcript and summary sections are included based on `options`.
///
/// # Errors
///
/// Returns an error if JSON serialization fails.
pub fn render(session: &Session, options: &RenderOptions) -> Result<String, serde_json::Error> {
    let mut map = Map::new();

    // Always include metadata.
    map.insert("id".to_owned(), json!(session.id.to_string()));
    map.insert(
        "created_at".to_owned(),
        json!(session.created_at.to_rfc3339()),
    );
    map.insert(
        "duration_seconds".to_owned(),
        json!(session.duration_seconds),
    );
    map.insert(
        "speakers".to_owned(),
        serde_json::to_value(&session.speakers)?,
    );

    if options.include_transcript {
        map.insert(
            "transcript".to_owned(),
            serde_json::to_value(&session.transcript)?,
        );
    }

    if options.include_summary {
        if let Some(ref summary) = session.summary {
            map.insert("summary".to_owned(), serde_json::to_value(summary)?);
        }
    }

    serde_json::to_string_pretty(&Value::Object(map))
}

#[cfg(test)]
mod tests {
    use vox_core::session::{
        AudioRole, AudioSourceInfo, ConfigSnapshot, Session, TranscriptSegment,
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
    fn test_json_export_metadata_always_present() {
        let session = base_session();
        let opts = RenderOptions {
            include_transcript: false,
            include_summary: false,
        };
        let json = render(&session, &opts).expect("serialize");
        let parsed: Value = serde_json::from_str(&json).expect("parse");
        assert!(parsed.get("id").is_some());
        assert!(parsed.get("created_at").is_some());
        assert!(parsed.get("duration_seconds").is_some());
        assert!(parsed.get("transcript").is_none());
        assert!(parsed.get("summary").is_none());
    }

    #[test]
    fn test_json_export_with_transcript() {
        let mut session = base_session();
        session.transcript = vec![TranscriptSegment {
            start_time: 0.0,
            end_time: 5.0,
            speaker: "You".to_owned(),
            text: "Hello.".to_owned(),
        }];
        let opts = RenderOptions {
            include_transcript: true,
            include_summary: false,
        };
        let json = render(&session, &opts).expect("serialize");
        let parsed: Value = serde_json::from_str(&json).expect("parse");
        let transcript = parsed.get("transcript").expect("transcript key");
        assert!(transcript.is_array());
        assert_eq!(transcript.as_array().unwrap().len(), 1);
    }
}
