//! Parsing of LLM response text into a structured [`Summary`].
//!
//! Strategy (in order):
//! 1. Try to deserialise the entire response as JSON.
//! 2. Try to extract a JSON object from within the response (handles models
//!    that wrap the JSON in markdown code fences or prose).
//! 3. Fall back to a best-effort markdown-section parser.
//! 4. If all else fails, put the entire response into `overview` and leave
//!    the structured fields empty.

use chrono::Utc;
use serde::Deserialize;
use vox_core::session::{ActionItem, Summary};

use crate::error::SummarizeError;

/// Intermediate type that mirrors the JSON schema we ask the LLM to produce.
#[derive(Debug, Deserialize)]
struct LlmSummaryJson {
    #[serde(default)]
    overview: String,
    #[serde(default)]
    key_points: Vec<String>,
    #[serde(default)]
    action_items: Vec<LlmActionItem>,
    #[serde(default)]
    decisions: Vec<String>,
}

/// Intermediate action-item type from the LLM JSON.
#[derive(Debug, Deserialize)]
struct LlmActionItem {
    description: String,
    owner: Option<String>,
}

/// Parse the raw text returned by the LLM into a [`Summary`].
///
/// Tries multiple strategies from strictest to most lenient.
///
/// # Arguments
///
/// * `text`    — Raw response content from the LLM.
/// * `backend` — Name of the backend (stored in the summary metadata).
/// * `model`   — Name of the model (stored in the summary metadata).
///
/// # Errors
///
/// Currently this function does not return an error — it always produces a
/// valid (if incomplete) [`Summary`].  The `Result` wrapper is kept so the
/// calling code can evolve the contract if needed.
pub fn parse_response(text: &str, backend: &str, model: &str) -> Result<Summary, SummarizeError> {
    let llm_json = try_parse_json(text)
        .or_else(|| try_extract_json(text))
        .unwrap_or_else(|| fallback_parse(text));

    Ok(Summary {
        generated_at: Utc::now(),
        backend: backend.to_owned(),
        model: model.to_owned(),
        overview: llm_json.overview,
        key_points: llm_json.key_points,
        action_items: llm_json
            .action_items
            .into_iter()
            .map(|ai| ActionItem {
                description: ai.description,
                owner: ai.owner,
            })
            .collect(),
        decisions: llm_json.decisions,
    })
}

/// Attempt to parse the entire `text` as a JSON object.
fn try_parse_json(text: &str) -> Option<LlmSummaryJson> {
    serde_json::from_str(text.trim()).ok()
}

/// Search for the first `{…}` block in `text` and try to parse it as JSON.
///
/// This handles responses like:
/// ```text
/// Here is the summary:
/// ```json
/// { "overview": "..." }
/// ```
/// ```
fn try_extract_json(text: &str) -> Option<LlmSummaryJson> {
    // Find the first `{` and the last `}`.
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    let candidate = &text[start..=end];
    serde_json::from_str(candidate).ok()
}

/// Section discriminant for the markdown fallback parser.
#[derive(PartialEq)]
enum FallbackSection {
    None,
    Overview,
    KeyPoints,
    ActionItems,
    Decisions,
}

/// Last-resort parser that extracts sections from markdown-formatted text.
///
/// Recognises headers like `## Overview`, `## Key Points`, etc.
fn fallback_parse(text: &str) -> LlmSummaryJson {
    let mut overview = String::new();
    let mut key_points: Vec<String> = Vec::new();
    let mut action_items: Vec<LlmActionItem> = Vec::new();
    let mut decisions: Vec<String> = Vec::new();
    let mut current = FallbackSection::None;

    for line in text.lines() {
        let trimmed = line.trim();

        // Detect section headers (markdown ##, or plain text labels).
        if is_section_header(trimmed, &["overview", "summary"]) {
            current = FallbackSection::Overview;
            continue;
        } else if is_section_header(
            trimmed,
            &["key point", "key discussion", "discussion point"],
        ) {
            current = FallbackSection::KeyPoints;
            continue;
        } else if is_section_header(trimmed, &["action item", "action", "todo", "to-do", "task"]) {
            current = FallbackSection::ActionItems;
            continue;
        } else if is_section_header(trimmed, &["decision"]) {
            current = FallbackSection::Decisions;
            continue;
        }

        if trimmed.is_empty() {
            continue;
        }

        match current {
            FallbackSection::Overview => {
                if !overview.is_empty() {
                    overview.push(' ');
                }
                overview.push_str(trimmed);
            }
            FallbackSection::KeyPoints => {
                if let Some(bullet) = strip_bullet(trimmed) {
                    key_points.push(bullet.to_owned());
                }
            }
            FallbackSection::ActionItems => {
                if let Some(bullet) = strip_bullet(trimmed) {
                    // Try "owner: task" or "task (owner)" patterns.
                    let (description, owner) = split_action_item(bullet);
                    action_items.push(LlmActionItem {
                        description: description.to_owned(),
                        owner: owner.map(str::to_owned),
                    });
                }
            }
            FallbackSection::Decisions => {
                if let Some(bullet) = strip_bullet(trimmed) {
                    decisions.push(bullet.to_owned());
                }
            }
            FallbackSection::None => {
                // Before any recognised section — treat as overview.
                if !overview.is_empty() {
                    overview.push(' ');
                }
                overview.push_str(trimmed);
            }
        }
    }

    // If nothing was parsed into structured fields, put everything in overview.
    if overview.is_empty()
        && key_points.is_empty()
        && action_items.is_empty()
        && decisions.is_empty()
    {
        text.trim().clone_into(&mut overview);
    }

    LlmSummaryJson {
        overview,
        key_points,
        action_items,
        decisions,
    }
}

/// Return `true` if `line` looks like a section header containing any keyword.
fn is_section_header(line: &str, keywords: &[&str]) -> bool {
    let lower = line.to_lowercase();
    // Must look like a header: starts with `#`, ends with `:`, or is all caps.
    let looks_like_header = lower.starts_with('#')
        || lower.ends_with(':')
        || (line.len() > 3 && line == line.to_uppercase());

    looks_like_header && keywords.iter().any(|kw| lower.contains(kw))
}

/// Strip common bullet/list prefixes (`-`, `*`, `•`, `1.`) from a line.
///
/// Returns `None` if the line does not look like a list item.
fn strip_bullet(line: &str) -> Option<&str> {
    // Match `- `, `* `, `• `, `· `, or `N. ` prefixes.
    if let Some(rest) = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("• "))
        .or_else(|| line.strip_prefix("· "))
    {
        return Some(rest.trim());
    }
    // Numbered list: `1. `, `12. `, etc.
    if let Some(dot_pos) = line.find(". ") {
        let prefix = &line[..dot_pos];
        if prefix.chars().all(|c| c.is_ascii_digit()) && !prefix.is_empty() {
            return Some(line[dot_pos + 2..].trim());
        }
    }
    None
}

/// Try to split an action item into `(description, owner)`.
///
/// Recognises:
/// - `"Owner: description"` → `(description, Some("Owner"))`
/// - `"description (Owner)"` → `(description, Some("Owner"))`
/// - Otherwise → `(description, None)`
fn split_action_item(text: &str) -> (&str, Option<&str>) {
    // "Owner: description" pattern.
    if let Some(colon_pos) = text.find(": ") {
        let potential_owner = &text[..colon_pos];
        // Owner is likely a short name (< 30 chars, no digits).
        if potential_owner.len() < 30
            && potential_owner
                .chars()
                .all(|c| c.is_alphabetic() || c == ' ')
        {
            return (&text[colon_pos + 2..], Some(potential_owner));
        }
    }
    // "description (Owner)" pattern.
    if text.ends_with(')') {
        if let Some(paren_start) = text.rfind(" (") {
            let owner = &text[paren_start + 2..text.len() - 1];
            if owner.len() < 30 && !owner.is_empty() {
                return (&text[..paren_start], Some(owner));
            }
        }
    }
    (text, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_clean_json() {
        let json = r#"{
            "overview": "We discussed the Q1 roadmap.",
            "key_points": ["Feature A", "Feature B"],
            "action_items": [
                {"description": "Write spec", "owner": "Alice"},
                {"description": "Review PR", "owner": null}
            ],
            "decisions": ["Ship in March"]
        }"#;

        let summary = parse_response(json, "ollama", "qwen2.5:1.5b").expect("should parse");
        assert_eq!(summary.overview, "We discussed the Q1 roadmap.");
        assert_eq!(summary.key_points.len(), 2);
        assert_eq!(summary.action_items.len(), 2);
        assert_eq!(summary.action_items[0].owner.as_deref(), Some("Alice"));
        assert!(summary.action_items[1].owner.is_none());
        assert_eq!(summary.decisions.len(), 1);
        assert_eq!(summary.backend, "ollama");
        assert_eq!(summary.model, "qwen2.5:1.5b");
    }

    #[test]
    fn test_parse_json_wrapped_in_prose() {
        let response = r#"Here is the summary you requested:

```json
{
    "overview": "The team aligned on priorities.",
    "key_points": ["Deploy next week"],
    "action_items": [],
    "decisions": ["Use Rust"]
}
```

Let me know if you need anything else."#;

        let summary =
            parse_response(response, "openai_compatible", "gpt-4o").expect("should extract JSON");
        assert_eq!(summary.overview, "The team aligned on priorities.");
        assert_eq!(summary.key_points.len(), 1);
        assert_eq!(summary.decisions, vec!["Use Rust"]);
    }

    #[test]
    fn test_parse_markdown_fallback() {
        let response = r#"## Overview
The call covered project timelines and risks.

## Key Points:
- Timeline is tight
- Risk of scope creep

## Action Items:
- Alice: Draft the risk register
- Update the Gantt chart (Bob)

## Decisions:
- Proceed with phase 1
"#;

        let summary =
            parse_response(response, "builtin", "phi-3").expect("should parse with fallback");
        assert!(summary.overview.contains("timelines"));
        assert!(summary.key_points.len() >= 1);
    }

    #[test]
    fn test_parse_total_garbage_puts_text_in_overview() {
        let garbage = "No structure here at all. Just some random text.";
        let summary = parse_response(garbage, "ollama", "llama3").expect("should not error");
        assert!(!summary.overview.is_empty());
    }

    #[test]
    fn test_strip_bullet_dash() {
        assert_eq!(strip_bullet("- Hello"), Some("Hello"));
    }

    #[test]
    fn test_strip_bullet_star() {
        assert_eq!(strip_bullet("* World"), Some("World"));
    }

    #[test]
    fn test_strip_bullet_numbered() {
        assert_eq!(strip_bullet("3. Third item"), Some("Third item"));
    }

    #[test]
    fn test_strip_bullet_no_match() {
        assert_eq!(strip_bullet("plain text"), None);
    }

    #[test]
    fn test_split_action_item_owner_prefix() {
        let (desc, owner) = split_action_item("Alice: Write the spec");
        assert_eq!(desc, "Write the spec");
        assert_eq!(owner, Some("Alice"));
    }

    #[test]
    fn test_split_action_item_owner_suffix() {
        let (desc, owner) = split_action_item("Write the spec (Alice)");
        assert_eq!(desc, "Write the spec");
        assert_eq!(owner, Some("Alice"));
    }

    #[test]
    fn test_split_action_item_no_owner() {
        let (desc, owner) = split_action_item("Write the spec");
        assert_eq!(desc, "Write the spec");
        assert!(owner.is_none());
    }

    #[test]
    fn test_parse_response_sets_timestamp() {
        let json = r#"{"overview":"ok","key_points":[],"action_items":[],"decisions":[]}"#;
        let before = Utc::now();
        let summary = parse_response(json, "test", "model").expect("parse");
        let after = Utc::now();
        assert!(summary.generated_at >= before);
        assert!(summary.generated_at <= after);
    }
}
