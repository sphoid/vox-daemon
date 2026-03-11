//! Session data model types matching the PRD Section 7.
//!
//! A `Session` represents a single recorded meeting, containing metadata,
//! transcript segments, speaker mappings, and an optional summary.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A complete recording session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Unique session identifier.
    pub id: Uuid,

    /// When the session was created.
    pub created_at: DateTime<Utc>,

    /// Session duration in seconds.
    pub duration_seconds: u64,

    /// Audio sources used in this session.
    pub audio_sources: Vec<AudioSourceInfo>,

    /// Transcription configuration snapshot at time of recording.
    pub config_snapshot: ConfigSnapshot,

    /// Timestamped transcript segments.
    pub transcript: Vec<TranscriptSegment>,

    /// Speaker label mappings.
    pub speakers: Vec<SpeakerMapping>,

    /// AI-generated summary, if available.
    pub summary: Option<Summary>,

    /// Path to the retained audio file, if audio retention is enabled.
    pub audio_file_path: Option<String>,
}

impl Session {
    /// Creates a new session with the given audio sources and config.
    #[must_use]
    pub fn new(audio_sources: Vec<AudioSourceInfo>, config_snapshot: ConfigSnapshot) -> Self {
        Self {
            id: Uuid::new_v4(),
            created_at: Utc::now(),
            duration_seconds: 0,
            audio_sources,
            config_snapshot,
            transcript: Vec::new(),
            speakers: Vec::new(),
            summary: None,
            audio_file_path: None,
        }
    }
}

/// Information about an audio source used during recording.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioSourceInfo {
    /// Human-readable name of the audio source.
    pub name: String,

    /// `PipeWire` node ID.
    pub pipewire_node_id: u32,

    /// Role of this audio source.
    pub role: AudioRole,
}

/// The role of an audio source in the recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioRole {
    /// The user's microphone input.
    Microphone,
    /// Application audio output (remote participants).
    Application,
}

/// Snapshot of the transcription configuration used for this session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSnapshot {
    /// Whisper model size used.
    pub model: String,
    /// Language setting.
    pub language: String,
    /// GPU backend used.
    pub gpu_backend: String,
}

/// A single segment of the transcript with timing and speaker info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptSegment {
    /// Start time in seconds from the beginning of the recording.
    pub start_time: f64,
    /// End time in seconds from the beginning of the recording.
    pub end_time: f64,
    /// Speaker identifier (e.g., `"speaker_0"` or `"You"`).
    pub speaker: String,
    /// Transcribed text content.
    pub text: String,
}

/// Maps a speaker ID to a friendly name and source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakerMapping {
    /// Internal speaker identifier (e.g., `"speaker_0"`).
    pub id: String,
    /// User-assigned friendly name (e.g., `"Alice"`).
    pub friendly_name: String,
    /// Whether this speaker comes from the microphone or remote stream.
    pub source: SpeakerSource,
}

/// Indicates whether a speaker is local or remote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpeakerSource {
    /// The local user's microphone.
    Microphone,
    /// A remote participant's audio.
    Remote,
}

/// An AI-generated summary of a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    /// When the summary was generated.
    pub generated_at: DateTime<Utc>,
    /// LLM backend that generated the summary.
    pub backend: String,
    /// Model name/identifier used.
    pub model: String,
    /// Brief overall summary (2-3 sentences).
    pub overview: String,
    /// Key discussion points.
    pub key_points: Vec<String>,
    /// Action items with optional owner.
    pub action_items: Vec<ActionItem>,
    /// Decisions made during the call.
    pub decisions: Vec<String>,
}

/// An action item extracted from the call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionItem {
    /// Description of the action item.
    pub description: String,
    /// The person responsible, if identifiable.
    pub owner: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_new() {
        let sources = vec![AudioSourceInfo {
            name: "Built-in Mic".to_owned(),
            pipewire_node_id: 42,
            role: AudioRole::Microphone,
        }];
        let config = ConfigSnapshot {
            model: "base".to_owned(),
            language: "en".to_owned(),
            gpu_backend: "auto".to_owned(),
        };
        let session = Session::new(sources, config);
        assert_eq!(session.duration_seconds, 0);
        assert!(session.transcript.is_empty());
        assert!(session.summary.is_none());
    }

    #[test]
    fn test_session_json_roundtrip() {
        let session = Session::new(
            vec![
                AudioSourceInfo {
                    name: "Mic".to_owned(),
                    pipewire_node_id: 1,
                    role: AudioRole::Microphone,
                },
                AudioSourceInfo {
                    name: "Zoom".to_owned(),
                    pipewire_node_id: 2,
                    role: AudioRole::Application,
                },
            ],
            ConfigSnapshot {
                model: "small".to_owned(),
                language: "auto".to_owned(),
                gpu_backend: "cuda".to_owned(),
            },
        );

        let json = serde_json::to_string_pretty(&session).expect("serialize");
        let parsed: Session = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.id, session.id);
        assert_eq!(parsed.audio_sources.len(), 2);
    }

    #[test]
    fn test_audio_role_serde() {
        let role = AudioRole::Microphone;
        let json = serde_json::to_string(&role).expect("serialize");
        assert_eq!(json, "\"microphone\"");
        let parsed: AudioRole = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, AudioRole::Microphone);
    }

    #[test]
    fn test_speaker_source_serde() {
        let source = SpeakerSource::Remote;
        let json = serde_json::to_string(&source).expect("serialize");
        assert_eq!(json, "\"remote\"");
    }
}
