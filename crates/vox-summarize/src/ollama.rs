//! Native Ollama HTTP client.
//!
//! Uses Ollama's native `/api/chat` endpoint rather than the `OpenAI`
//! compatibility layer. This avoids subtle incompatibilities with
//! `response_format`, `max_tokens`, and the response structure.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument, warn};
use vox_core::session::{Summary, TranscriptSegment};

use crate::{
    error::SummarizeError, parse::parse_response, prompt::build_prompt, traits::Summarizer,
};

// ── Request / response types ─────────────────────────────────────────────────

/// A single message in the Ollama chat request.
#[derive(Debug, Serialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

/// Options for the Ollama model.
#[derive(Debug, Serialize)]
struct OllamaOptions {
    /// Maximum number of tokens to generate.
    num_predict: u32,
    /// Sampling temperature.
    temperature: f32,
}

/// The body sent to `/api/chat`.
#[derive(Debug, Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    /// Disable streaming — return a single JSON response.
    stream: bool,
    /// Request JSON output from the model.
    format: String,
    options: OllamaOptions,
}

/// Top-level response from `/api/chat` when `stream: false`.
#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    message: OllamaResponseMessage,
    /// Why generation stopped: `"stop"` (natural end) or `"length"` (hit the
    /// `num_predict` token cap — a common cause of empty `content`).
    #[serde(default)]
    done_reason: Option<String>,
}

/// The assistant message in an Ollama chat response.
#[derive(Debug, Deserialize)]
struct OllamaResponseMessage {
    #[serde(default)]
    content: String,
    /// Reasoning text emitted by "thinking" models, separate from `content`.
    /// When the token budget is spent here, `content` can come back empty.
    #[serde(default)]
    thinking: Option<String>,
}

/// Error body returned by Ollama when the request fails.
#[derive(Debug, Deserialize)]
struct OllamaErrorBody {
    error: String,
}

// ── Client ───────────────────────────────────────────────────────────────────

/// An HTTP client that calls Ollama's native `/api/chat` endpoint.
pub struct OllamaClient {
    /// The HTTP client (with built-in connection pooling).
    http: Client,
    /// Full URL to the `/api/chat` endpoint.
    endpoint: String,
    /// Model identifier sent in the request body.
    model: String,
    /// Maximum tokens to request (sent as `num_predict`).
    max_tokens: u32,
}

impl OllamaClient {
    /// Create a new Ollama client.
    ///
    /// # Arguments
    ///
    /// * `base_url` — Base URL of the Ollama server (e.g. `"http://localhost:11434"`).
    ///   A trailing slash is handled automatically.
    /// * `model` — Model identifier (e.g. `"qwen2.5:1.5b"` or `"llama3"`).
    /// * `timeout_secs` — Total request timeout in seconds.
    /// * `max_tokens` — Maximum tokens to generate (sent as `num_predict`).
    ///
    /// # Errors
    ///
    /// Returns [`SummarizeError::Config`] if the `reqwest` client cannot be built.
    pub fn new(
        base_url: &str,
        model: impl Into<String>,
        timeout_secs: u64,
        max_tokens: u32,
    ) -> Result<Self, SummarizeError> {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build()
            .map_err(|e| SummarizeError::Config(format!("failed to build HTTP client: {e}")))?;

        let base = base_url.trim_end_matches('/');
        let endpoint = format!("{base}/api/chat");

        Ok(Self {
            http,
            endpoint,
            model: model.into(),
            max_tokens,
        })
    }

    /// Send a chat request to Ollama and return the assistant's text content.
    #[instrument(skip(self, system_prompt, user_prompt), fields(model = %self.model))]
    async fn chat(&self, system_prompt: &str, user_prompt: &str) -> Result<String, SummarizeError> {
        let messages = vec![
            OllamaMessage {
                role: "system".to_owned(),
                content: system_prompt.to_owned(),
            },
            OllamaMessage {
                role: "user".to_owned(),
                content: user_prompt.to_owned(),
            },
        ];

        let body = OllamaChatRequest {
            model: self.model.clone(),
            messages,
            stream: false,
            format: "json".to_owned(),
            options: OllamaOptions {
                num_predict: self.max_tokens,
                temperature: 0.2,
            },
        };

        debug!(endpoint = %self.endpoint, "sending Ollama chat request");
        debug!(
            system_prompt = %system_prompt,
            user_prompt = %user_prompt,
            "LLM request prompt"
        );

        let response = self
            .http
            .post(&self.endpoint)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = response.status();

        if !status.is_success() {
            let status_u16 = status.as_u16();
            let body_text = response.text().await.unwrap_or_default();
            // Try to extract a structured error message.
            let message = serde_json::from_str::<OllamaErrorBody>(&body_text)
                .map(|e| e.error)
                .unwrap_or(body_text.clone());
            warn!(status = status_u16, error = %message, "Ollama API returned error");
            return Err(SummarizeError::ApiError {
                status: status_u16,
                body: message,
            });
        }

        let body_text = response.text().await?;
        debug!(body_len = body_text.len(), "raw Ollama response body");

        let chat_response: OllamaChatResponse = serde_json::from_str(&body_text).map_err(|e| {
            warn!(
                error = %e,
                body_preview = &body_text[..body_text.len().min(500)],
                "failed to parse Ollama response"
            );
            SummarizeError::ParseError {
                reason: format!("failed to deserialize Ollama response: {e}"),
                raw: body_text.clone(),
            }
        })?;

        let done_reason = chat_response.done_reason.unwrap_or_default();
        let thinking_chars = chat_response
            .message
            .thinking
            .as_ref()
            .map_or(0, String::len);
        debug!(
            done_reason = %done_reason,
            thinking_chars,
            "Ollama generation finished"
        );

        let content = chat_response.message.content;

        if content.is_empty() {
            // `done_reason = "length"` here means the token budget was spent
            // (often on `thinking`) before any content was produced — raise
            // `max_completion_tokens` in the summarization config.
            warn!(
                done_reason = %done_reason,
                thinking_chars,
                raw_body = &body_text[..body_text.len().min(500)],
                "Ollama returned empty content"
            );
            return Err(SummarizeError::EmptyResponse);
        }

        debug!(chars = content.len(), "received Ollama chat response");
        debug!(raw_response = %content, "Ollama LLM response content");
        Ok(content)
    }
}

#[async_trait]
impl Summarizer for OllamaClient {
    /// Summarize the transcript by calling the Ollama `/api/chat` endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`SummarizeError`] on empty transcript, HTTP failure, or
    /// response parse failure.
    async fn summarize(&self, transcript: &[TranscriptSegment]) -> Result<Summary, SummarizeError> {
        if transcript.is_empty() {
            return Err(SummarizeError::EmptyTranscript);
        }

        let (system_prompt, user_prompt) = build_prompt(transcript);

        let raw = self.chat(&system_prompt, &user_prompt).await?;

        parse_response(&raw, "ollama", &self.model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_endpoint_normalisation_trailing_slash() {
        let client = OllamaClient::new("http://localhost:11434/", "llama3", 300, 1024)
            .expect("build client");
        assert_eq!(client.endpoint, "http://localhost:11434/api/chat");
    }

    #[test]
    fn test_endpoint_normalisation_no_slash() {
        let client =
            OllamaClient::new("http://localhost:11434", "llama3", 300, 1024).expect("build client");
        assert_eq!(client.endpoint, "http://localhost:11434/api/chat");
    }

    #[test]
    fn test_client_stores_model() {
        let client = OllamaClient::new("http://localhost:11434", "my-model", 300, 1024)
            .expect("build client");
        assert_eq!(client.model, "my-model");
    }

    #[tokio::test]
    async fn test_summarize_empty_transcript_returns_error() {
        let client =
            OllamaClient::new("http://localhost:11434", "llama3", 300, 1024).expect("build client");
        let result = client.summarize(&[]).await;
        assert!(matches!(result, Err(SummarizeError::EmptyTranscript)));
    }
}
