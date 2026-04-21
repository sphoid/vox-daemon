//! [`SessionStore`] trait and [`JsonFileStore`] implementation.
//!
//! Sessions are persisted as individual JSON files named `{uuid}.json` under
//! the configured sessions directory.

use std::fs;
use std::path::{Path, PathBuf};

use tracing::{debug, info, warn};
use uuid::Uuid;
use vox_core::error::StorageError;
use vox_core::paths::sessions_dir_or;
use vox_core::session::Session;

use crate::markdown;

/// Trait for storing, retrieving, and exporting call sessions.
pub trait SessionStore: Send + Sync {
    /// Persist a session to storage, overwriting any existing entry with the same ID.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the file cannot be written or serialized.
    fn save(&self, session: &Session) -> Result<(), StorageError>;

    /// Load a session by its UUID.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::NotFound`] if no session with that ID exists, or
    /// [`StorageError`] for I/O and deserialization failures.
    fn load(&self, id: Uuid) -> Result<Session, StorageError>;

    /// List all stored sessions, sorted by `created_at` descending (newest first).
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the sessions directory cannot be read.
    fn list(&self) -> Result<Vec<Session>, StorageError>;

    /// Delete the session with the given UUID.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::NotFound`] if no such session exists, or
    /// [`StorageError`] for I/O failures.
    fn delete(&self, id: Uuid) -> Result<(), StorageError>;

    /// Export a session as a Markdown-formatted string.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::NotFound`] if no such session exists, or
    /// [`StorageError`] for I/O and deserialization failures.
    fn export_markdown(&self, id: Uuid) -> Result<String, StorageError>;
}

/// A [`SessionStore`] implementation that persists sessions as JSON files.
///
/// Each session is stored as `{sessions_dir}/{uuid}.json`. The sessions
/// directory is determined from the XDG data directory, optionally overridden
/// by the `data_dir` field from [`vox_core::config::StorageConfig`].
#[derive(Debug, Clone)]
pub struct JsonFileStore {
    /// Absolute path to the directory where `{uuid}.json` files are stored.
    sessions_dir: PathBuf,
}

impl JsonFileStore {
    /// Create a new [`JsonFileStore`] using the given custom data directory.
    ///
    /// If `custom_data_dir` is empty, the XDG default
    /// (`$XDG_DATA_HOME/vox-daemon/sessions/`) is used.
    ///
    /// The sessions directory is created if it does not already exist.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the directory cannot be created.
    pub fn new(custom_data_dir: &str) -> Result<Self, StorageError> {
        let sessions_dir = sessions_dir_or(custom_data_dir);
        fs::create_dir_all(&sessions_dir)?;
        info!("JsonFileStore initialised at {}", sessions_dir.display());
        Ok(Self { sessions_dir })
    }

    /// Create a [`JsonFileStore`] rooted at an explicit directory path.
    ///
    /// Useful for testing with temporary directories. The directory is created
    /// if it does not already exist.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the directory cannot be created.
    pub fn with_dir(sessions_dir: impl Into<PathBuf>) -> Result<Self, StorageError> {
        let sessions_dir = sessions_dir.into();
        fs::create_dir_all(&sessions_dir)?;
        debug!("JsonFileStore initialised at {}", sessions_dir.display());
        Ok(Self { sessions_dir })
    }

    /// Returns the path to the JSON file for a given session UUID.
    #[must_use]
    fn session_path(&self, id: Uuid) -> PathBuf {
        self.sessions_dir.join(format!("{id}.json"))
    }

    /// Read and deserialize a session from a file path.
    fn read_session(path: &Path) -> Result<Session, StorageError> {
        let data = fs::read(path)?;
        let session = serde_json::from_slice(&data)?;
        Ok(session)
    }
}

impl SessionStore for JsonFileStore {
    fn save(&self, session: &Session) -> Result<(), StorageError> {
        let path = self.session_path(session.id);
        let json = serde_json::to_vec_pretty(session)?;
        fs::write(&path, json)?;
        info!("session {} saved to {}", session.id, path.display());
        Ok(())
    }

    fn load(&self, id: Uuid) -> Result<Session, StorageError> {
        let path = self.session_path(id);
        if !path.exists() {
            return Err(StorageError::NotFound(id.to_string()));
        }
        debug!("loading session {} from {}", id, path.display());
        Self::read_session(&path)
    }

    fn list(&self) -> Result<Vec<Session>, StorageError> {
        let read_dir = match fs::read_dir(&self.sessions_dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            }
            Err(e) => return Err(StorageError::Io(e)),
        };

        let mut sessions = Vec::new();

        for entry in read_dir {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }

            match Self::read_session(&path) {
                Ok(session) => sessions.push(session),
                Err(e) => {
                    warn!("skipping unreadable session file {}: {}", path.display(), e);
                }
            }
        }

        // Sort newest-first.
        sessions.sort_by_key(|s| std::cmp::Reverse(s.created_at));
        debug!("listed {} sessions", sessions.len());
        Ok(sessions)
    }

    fn delete(&self, id: Uuid) -> Result<(), StorageError> {
        let path = self.session_path(id);
        if !path.exists() {
            return Err(StorageError::NotFound(id.to_string()));
        }
        fs::remove_file(&path)?;
        info!("session {} deleted", id);
        Ok(())
    }

    fn export_markdown(&self, id: Uuid) -> Result<String, StorageError> {
        let session = self.load(id)?;
        Ok(markdown::render(&session))
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use vox_core::session::{
        ActionItem, AudioRole, AudioSourceInfo, ConfigSnapshot, Session, SpeakerMapping,
        SpeakerSource, Summary, TranscriptSegment,
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
                diarization_mode: "none".to_owned(),
                decoding_strategy: String::new(),
                initial_prompt: String::new(),
            },
        )
    }

    fn make_store() -> (JsonFileStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("create tempdir");
        let store = JsonFileStore::with_dir(dir.path()).expect("create store");
        (store, dir)
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let (store, _dir) = make_store();
        let session = make_session();
        store.save(&session).expect("save");
        let loaded = store.load(session.id).expect("load");
        assert_eq!(loaded.id, session.id);
        assert_eq!(loaded.config_snapshot.model, "base");
    }

    #[test]
    fn test_load_missing_returns_not_found() {
        let (store, _dir) = make_store();
        let id = Uuid::new_v4();
        match store.load(id) {
            Err(StorageError::NotFound(msg)) => assert!(msg.contains(&id.to_string())),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn test_delete_session() {
        let (store, _dir) = make_store();
        let session = make_session();
        store.save(&session).expect("save");
        store.delete(session.id).expect("delete");
        assert!(matches!(
            store.load(session.id),
            Err(StorageError::NotFound(_))
        ));
    }

    #[test]
    fn test_delete_missing_returns_not_found() {
        let (store, _dir) = make_store();
        let id = Uuid::new_v4();
        assert!(matches!(store.delete(id), Err(StorageError::NotFound(_))));
    }

    #[test]
    fn test_list_empty_directory() {
        let (store, _dir) = make_store();
        let sessions = store.list().expect("list");
        assert!(sessions.is_empty());
    }

    #[test]
    fn test_list_sorted_newest_first() {
        let (store, _dir) = make_store();

        // Create two sessions with different timestamps.
        let mut older = make_session();
        older.created_at = Utc::now() - chrono::Duration::seconds(3600);
        let mut newer = make_session();
        newer.created_at = Utc::now();

        store.save(&older).expect("save older");
        store.save(&newer).expect("save newer");

        let sessions = store.list().expect("list");
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].id, newer.id, "newest should be first");
        assert_eq!(sessions[1].id, older.id, "oldest should be last");
    }

    #[test]
    fn test_save_overwrites_existing() {
        let (store, _dir) = make_store();
        let mut session = make_session();
        store.save(&session).expect("save initial");

        session.duration_seconds = 120;
        store.save(&session).expect("save updated");

        let loaded = store.load(session.id).expect("load");
        assert_eq!(loaded.duration_seconds, 120);
    }

    #[test]
    fn test_list_skips_non_json_files() {
        let (store, dir) = make_store();
        // Create a non-JSON file in the sessions directory.
        fs::write(dir.path().join("notes.txt"), b"hello").expect("write txt");
        let sessions = store.list().expect("list");
        assert!(sessions.is_empty());
    }

    #[test]
    fn test_export_markdown_contains_overview() {
        let (store, _dir) = make_store();
        let mut session = make_session();
        session.transcript = vec![TranscriptSegment {
            start_time: 10.0,
            end_time: 20.0,
            speaker: "You".to_owned(),
            text: "Hello world".to_owned(),
        }];
        session.speakers = vec![SpeakerMapping {
            id: "speaker_0".to_owned(),
            friendly_name: "You".to_owned(),
            source: SpeakerSource::Microphone,
        }];
        session.summary = Some(Summary {
            generated_at: Utc::now(),
            backend: "builtin".to_owned(),
            model: "test-model".to_owned(),
            overview: "Brief overview of the meeting.".to_owned(),
            key_points: vec!["Point A".to_owned()],
            action_items: vec![ActionItem {
                description: "Follow up".to_owned(),
                owner: Some("Alice".to_owned()),
            }],
            decisions: vec!["Go with option B".to_owned()],
        });
        store.save(&session).expect("save");

        let md = store.export_markdown(session.id).expect("export");
        assert!(md.contains("# Meeting Transcript"), "should have title");
        assert!(md.contains("Hello world"), "should contain transcript text");
        assert!(
            md.contains("Brief overview of the meeting."),
            "should contain summary overview"
        );
        assert!(md.contains("Point A"), "should contain key points");
        assert!(md.contains("Follow up"), "should contain action items");
        assert!(md.contains("Go with option B"), "should contain decisions");
    }

    #[test]
    fn test_export_markdown_missing_session() {
        let (store, _dir) = make_store();
        let id = Uuid::new_v4();
        assert!(matches!(
            store.export_markdown(id),
            Err(StorageError::NotFound(_))
        ));
    }

    #[test]
    fn test_new_creates_sessions_dir() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let sessions_path = dir.path().join("sessions");
        assert!(!sessions_path.exists(), "should not exist yet");

        let _store = JsonFileStore::with_dir(&sessions_path).expect("store init should create dir");
        assert!(sessions_path.exists(), "dir should now exist");
    }
}
