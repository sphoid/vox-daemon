//! Full-text search across transcript segments.
//!
//! [`search_transcripts`] performs a case-insensitive substring search over
//! all [`TranscriptSegment`] text fields in a slice of [`Session`] objects.
//! Results carry the session ID and the indices of matching segments so the
//! caller can highlight them in the transcript viewer.

use uuid::Uuid;
use vox_core::session::Session;

/// A single search result, pointing to a session and its matching segments.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    /// The session that contains at least one match.
    pub session_id: Uuid,

    /// Indices into `session.transcript` of segments whose text matches the
    /// query.
    pub matching_segment_indices: Vec<usize>,
}

impl SearchResult {
    /// Returns `true` if this result has no matching segments.
    ///
    /// A `SearchResult` with zero matches should never be returned by
    /// [`search_transcripts`], but this guard is useful in assertions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.matching_segment_indices.is_empty()
    }
}

/// Search all sessions for transcript segments whose text contains `query`
/// (case-insensitive).
///
/// Returns one [`SearchResult`] per session that has at least one matching
/// segment. Sessions with no matches are omitted.
///
/// An empty `query` string matches every segment in every session, which is
/// useful for "select all" UX patterns but may be expensive for large session
/// libraries — callers should avoid triggering a search on an empty query.
///
/// # Performance
///
/// This is a linear O(n × m) scan where n is the total number of transcript
/// segments across all sessions and m is the query length. For up to a few
/// hundred sessions of typical length this is fast enough for interactive
/// search-as-you-type. For very large libraries consider building an inverted
/// index.
#[must_use]
pub fn search_transcripts(sessions: &[Session], query: &str) -> Vec<SearchResult> {
    let query_lower = query.to_lowercase();

    sessions
        .iter()
        .filter_map(|session| {
            let indices: Vec<usize> = session
                .transcript
                .iter()
                .enumerate()
                .filter(|(_, seg)| seg.text.to_lowercase().contains(&query_lower))
                .map(|(i, _)| i)
                .collect();

            if indices.is_empty() {
                None
            } else {
                Some(SearchResult {
                    session_id: session.id,
                    matching_segment_indices: indices,
                })
            }
        })
        .collect()
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use vox_core::session::{
        AudioRole, AudioSourceInfo, ConfigSnapshot, Session, TranscriptSegment,
    };

    use super::*;

    fn make_session_with_transcript(segments: Vec<(&str, &str)>) -> Session {
        let mut s = Session::new(
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
            },
        );
        s.transcript = segments
            .into_iter()
            .enumerate()
            .map(|(i, (speaker, text))| TranscriptSegment {
                #[allow(clippy::cast_precision_loss)]
                start_time: i as f64 * 5.0,
                #[allow(clippy::cast_precision_loss)]
                end_time: (i as f64 + 1.0) * 5.0,
                speaker: speaker.to_owned(),
                text: text.to_owned(),
            })
            .collect();
        s
    }

    #[test]
    fn test_search_finds_matching_segment() {
        let session = make_session_with_transcript(vec![
            ("You", "Hello world, let us begin the meeting."),
            ("Remote", "Thanks for joining today."),
        ]);
        let results = search_transcripts(&[session.clone()], "meeting");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_id, session.id);
        assert_eq!(results[0].matching_segment_indices, vec![0]);
    }

    #[test]
    fn test_search_case_insensitive() {
        let session = make_session_with_transcript(vec![("You", "Hello WORLD")]);
        let results = search_transcripts(&[session.clone()], "world");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].matching_segment_indices, vec![0]);
    }

    #[test]
    fn test_search_no_match_returns_empty() {
        let session = make_session_with_transcript(vec![("You", "Nothing relevant here.")]);
        let results = search_transcripts(&[session], "quarterly report");
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_multiple_segments_match() {
        let session = make_session_with_transcript(vec![
            ("You", "We need to discuss the budget."),
            ("Remote", "The budget looks fine to me."),
            ("You", "Great, moving on."),
        ]);
        let results = search_transcripts(&[session.clone()], "budget");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].matching_segment_indices, vec![0, 1]);
    }

    #[test]
    fn test_search_across_multiple_sessions() {
        let s1 = make_session_with_transcript(vec![("You", "First session about alpha release.")]);
        let s2 = make_session_with_transcript(vec![("You", "Second session about beta features.")]);
        let s3 = make_session_with_transcript(vec![("You", "No mention here.")]);

        let results = search_transcripts(&[s1.clone(), s2.clone(), s3.clone()], "session");
        assert_eq!(results.len(), 2);
        let ids: Vec<Uuid> = results.iter().map(|r| r.session_id).collect();
        assert!(ids.contains(&s1.id));
        assert!(ids.contains(&s2.id));
        assert!(!ids.contains(&s3.id));
    }

    #[test]
    fn test_search_empty_sessions_slice() {
        let results = search_transcripts(&[], "anything");
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_session_with_no_transcript() {
        let session = make_session_with_transcript(vec![]);
        let results = search_transcripts(&[session], "hello");
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_empty_query_matches_all_segments() {
        let session = make_session_with_transcript(vec![("You", "Hello"), ("Remote", "World")]);
        let results = search_transcripts(&[session.clone()], "");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].matching_segment_indices, vec![0, 1]);
    }

    #[test]
    fn test_search_result_is_empty_guard() {
        let r = SearchResult {
            session_id: Uuid::new_v4(),
            matching_segment_indices: vec![],
        };
        assert!(r.is_empty());

        let r2 = SearchResult {
            session_id: Uuid::new_v4(),
            matching_segment_indices: vec![0],
        };
        assert!(!r2.is_empty());
    }

    #[test]
    fn test_search_partial_word_matches() {
        let session = make_session_with_transcript(vec![("You", "The transcription is complete.")]);
        let results = search_transcripts(&[session.clone()], "transcript");
        assert_eq!(results.len(), 1);
    }
}
