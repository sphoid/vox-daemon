//! Prompt engineering for the LLM summarization step.
//!
//! Builds the system and user prompts from a slice of [`TranscriptSegment`]s.
//! Handles transcript truncation to fit within typical context-window budgets.

use std::fmt::Write as _;

use vox_core::session::TranscriptSegment;

/// Approximate characters per token for a rough context-window budget.
const CHARS_PER_TOKEN: usize = 4;

/// Maximum number of tokens reserved for the transcript in the user prompt.
/// This leaves headroom for the system prompt and the model's response.
const MAX_TRANSCRIPT_TOKENS: usize = 6_000;

/// Maximum characters allowed for the transcript portion of the user prompt.
const MAX_TRANSCRIPT_CHARS: usize = MAX_TRANSCRIPT_TOKENS * CHARS_PER_TOKEN;

/// Build the system and user prompts for a summarization request.
///
/// Returns a `(system_prompt, user_prompt)` tuple.  The transcript is
/// formatted as a speaker-labelled conversation; if it exceeds the character
/// budget the first and last portions are kept and a truncation notice is
/// inserted in the middle.
///
/// # Arguments
///
/// * `segments` — Ordered transcript segments to summarise.
#[must_use]
pub fn build_prompt(segments: &[TranscriptSegment]) -> (String, String) {
    let system = build_system_prompt();
    let transcript_text = format_transcript(segments);
    let user = build_user_prompt(&transcript_text);
    (system, user)
}

/// Format transcript segments into a human-readable conversation string.
///
/// Each segment is rendered as:
/// ```text
/// [00:01:23 - 00:01:45] You: Hello, how are you?
/// ```
#[must_use]
pub fn format_transcript(segments: &[TranscriptSegment]) -> String {
    if segments.is_empty() {
        return String::new();
    }

    let raw: String = segments.iter().fold(String::new(), |mut acc, seg| {
        let start = format_seconds(seg.start_time);
        let end = format_seconds(seg.end_time);
        let _ = writeln!(
            acc,
            "[{start} - {end}] {}: {}",
            seg.speaker,
            seg.text.trim()
        );
        acc
    });

    maybe_truncate(raw)
}

/// Format a floating-point number of seconds as `HH:MM:SS`.
fn format_seconds(seconds: f64) -> String {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let total = seconds.max(0.0) as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

/// Truncate the transcript if it exceeds [`MAX_TRANSCRIPT_CHARS`].
///
/// Keeps the first 60 % and last 40 % of the budget and inserts a notice
/// in the middle so the model knows content was omitted.
fn maybe_truncate(text: String) -> String {
    if text.len() <= MAX_TRANSCRIPT_CHARS {
        return text;
    }

    let head_budget = (MAX_TRANSCRIPT_CHARS * 6) / 10;
    let tail_budget = MAX_TRANSCRIPT_CHARS - head_budget;

    let head = truncate_at_line_boundary(&text, head_budget, true);
    let tail = truncate_at_line_boundary(&text, tail_budget, false);

    format!("{head}\n\n[... transcript truncated for context-window limit ...]\n\n{tail}")
}

/// Take up to `budget` characters from either the start (`from_start = true`)
/// or the end of `text`, snapping to the nearest preceding newline so we
/// don't break lines in half.
fn truncate_at_line_boundary(text: &str, budget: usize, from_start: bool) -> &str {
    if text.len() <= budget {
        return text;
    }

    if from_start {
        let slice = &text[..budget];
        // Walk backward to find the last newline.
        if let Some(pos) = slice.rfind('\n') {
            &text[..=pos]
        } else {
            slice
        }
    } else {
        let start = text.len() - budget;
        let slice = &text[start..];
        // Walk forward to find the first newline.
        if let Some(pos) = slice.find('\n') {
            &slice[pos + 1..]
        } else {
            slice
        }
    }
}

/// Build the system prompt that instructs the LLM on output format.
fn build_system_prompt() -> String {
    r#"You are an expert meeting assistant. Your task is to analyze a call transcript and produce a structured summary.

You MUST respond with valid JSON only — no markdown code fences, no prose before or after the JSON object.

The JSON object must have exactly these fields:
{
  "overview": "<2–3 sentence summary of the entire call>",
  "key_points": ["<point 1>", "<point 2>", ...],
  "action_items": [
    {"description": "<what needs to be done>", "owner": "<person responsible or null>"},
    ...
  ],
  "decisions": ["<decision 1>", "<decision 2>", ...]
}

Guidelines:
- "overview": Concise 2–3 sentence narrative covering the purpose and outcome of the call.
- "key_points": Bulleted list of the most important topics discussed (3–8 points).
- "action_items": Every concrete task, commitment, or follow-up mentioned. Set "owner" to the speaker's name when clearly assigned, otherwise null.
- "decisions": Every conclusion, agreement, or resolved question from the call.
- If a section has no items, use an empty array [].
- Do not invent information that is not in the transcript."#.to_owned()
}

/// Build the user prompt from the formatted transcript text.
fn build_user_prompt(transcript: &str) -> String {
    format!("Please summarize the following call transcript:\n\n{transcript}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use vox_core::session::TranscriptSegment;

    fn make_segment(start: f64, end: f64, speaker: &str, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            start_time: start,
            end_time: end,
            speaker: speaker.to_owned(),
            text: text.to_owned(),
        }
    }

    #[test]
    fn test_format_seconds() {
        assert_eq!(format_seconds(0.0), "00:00:00");
        assert_eq!(format_seconds(61.0), "00:01:01");
        assert_eq!(format_seconds(3661.0), "01:01:01");
        assert_eq!(format_seconds(3600.0), "01:00:00");
    }

    #[test]
    fn test_format_transcript_empty() {
        let result = format_transcript(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_format_transcript_basic() {
        let segments = vec![
            make_segment(0.0, 5.0, "You", "Hello there."),
            make_segment(5.5, 10.0, "Remote", "Hi, how are you?"),
        ];
        let result = format_transcript(&segments);
        assert!(result.contains("[00:00:00 - 00:00:05] You: Hello there."));
        assert!(result.contains("[00:00:05 - 00:00:10] Remote: Hi, how are you?"));
    }

    #[test]
    fn test_format_transcript_trims_whitespace() {
        let segments = vec![make_segment(0.0, 1.0, "You", "  hello world  ")];
        let result = format_transcript(&segments);
        assert!(result.contains("You: hello world"));
    }

    #[test]
    fn test_build_prompt_returns_two_strings() {
        let segments = vec![make_segment(0.0, 5.0, "You", "Test.")];
        let (system, user) = build_prompt(&segments);
        assert!(!system.is_empty());
        assert!(user.contains("Test."));
    }

    #[test]
    fn test_build_prompt_empty_transcript() {
        let (system, user) = build_prompt(&[]);
        assert!(!system.is_empty());
        // User prompt should still be valid even for an empty transcript.
        assert!(user.contains("transcript"));
    }

    #[test]
    fn test_truncation_applied_for_large_input() {
        // Generate a transcript that exceeds the budget.
        let many_segments: Vec<TranscriptSegment> = (0..2000)
            .map(|i| {
                make_segment(
                    f64::from(i) * 5.0,
                    f64::from(i) * 5.0 + 4.9,
                    "You",
                    "This is a test sentence that repeats many times to fill the buffer.",
                )
            })
            .collect();
        let result = format_transcript(&many_segments);
        assert!(result.contains("truncated"));
        assert!(result.len() <= MAX_TRANSCRIPT_CHARS + 200); // +200 for the notice itself
    }

    #[test]
    fn test_system_prompt_contains_json_schema() {
        let (system, _) = build_prompt(&[]);
        assert!(system.contains("overview"));
        assert!(system.contains("key_points"));
        assert!(system.contains("action_items"));
        assert!(system.contains("decisions"));
    }
}
