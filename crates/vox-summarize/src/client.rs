//! OpenAI-compatible HTTP client.
//!
//! This single client struct works with any endpoint that implements the
//! `OpenAI` Chat Completions API, including:
//! - Ollama (`http://localhost:11434/v1/chat/completions`)
//! - `OpenAI` (`https://api.openai.com/v1/chat/completions`)
//! - Any other compatible service
//!
//! The API key is optional — Ollama does not require one.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument, warn};
use vox_core::session::{Summary, TranscriptSegment};

use crate::{
    error::SummarizeError, parse::parse_response, prompt::build_prompt, traits::Summarizer,
};

/// Default timeout for LLM HTTP requests (90 seconds).
const REQUEST_TIMEOUT_SECS: u64 = 90;

/// Approximate maximum tokens to request in the completion.
const MAX_COMPLETION_TOKENS: u32 = 1024;

// ── Request / response types ─────────────────────────────────────────────────

/// A single message in the chat completions request.
#[derive(Debug, Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

/// The body sent to `/v1/chat/completions`.
#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    /// Ask the model for JSON output (supported by many providers).
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
    temperature: f32,
}

/// OpenAI-style response format hint.
#[derive(Debug, Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    kind: String,
}

/// Top-level response from the completions endpoint.
#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
    /// Model identifier echoed back by the API.  Captured for potential
    /// future use (e.g., logging the exact model version that responded).
    #[serde(default)]
    #[allow(dead_code)]
    model: String,
}

/// A single completion choice.
#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}

/// The message inside a completion choice.
#[derive(Debug, Deserialize)]
struct ChatResponseMessage {
    content: Option<String>,
}

/// Error body returned by the API when the request fails.
#[derive(Debug, Deserialize)]
struct ApiErrorBody {
    error: ApiErrorDetail,
}

#[derive(Debug, Deserialize)]
struct ApiErrorDetail {
    message: String,
}

// ── Client ───────────────────────────────────────────────────────────────────

/// An HTTP client that calls any OpenAI-compatible chat-completions endpoint.
///
/// Works with Ollama, `OpenAI`, and any other compatible API.
pub struct OpenAiClient {
    /// The HTTP client (with built-in connection pooling).
    http: Client,
    /// Full URL to the `/v1/chat/completions` endpoint.
    endpoint: String,
    /// Optional bearer token (not required for local Ollama servers).
    api_key: Option<String>,
    /// Model identifier sent in the request body.
    model: String,
    /// Human-readable backend name for summary metadata.
    backend_name: String,
}

impl OpenAiClient {
    /// Create a new client.
    ///
    /// # Arguments
    ///
    /// * `base_url` — Base URL of the API server (e.g. `"http://localhost:11434"`).
    ///   A trailing slash is handled automatically.
    /// * `api_key` — Optional API key; pass `None` for Ollama or other
    ///   unauthenticated servers.
    /// * `model` — Model identifier (e.g. `"qwen2.5:1.5b"` or `"gpt-4o"`).
    /// * `backend_name` — Label stored in the generated [`Summary`] metadata.
    ///
    /// # Errors
    ///
    /// Returns [`SummarizeError::Config`] if the `reqwest` client cannot be built
    /// (which is extremely rare and usually indicates a TLS library issue).
    pub fn new(
        base_url: &str,
        api_key: Option<String>,
        model: impl Into<String>,
        backend_name: impl Into<String>,
    ) -> Result<Self, SummarizeError> {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build()
            .map_err(|e| SummarizeError::Config(format!("failed to build HTTP client: {e}")))?;

        let base = base_url.trim_end_matches('/');
        let endpoint = format!("{base}/v1/chat/completions");

        Ok(Self {
            http,
            endpoint,
            api_key,
            model: model.into(),
            backend_name: backend_name.into(),
        })
    }

    /// Send a chat-completions request and return the assistant's text content.
    #[instrument(skip(self, system_prompt, user_prompt), fields(model = %self.model))]
    async fn chat(&self, system_prompt: &str, user_prompt: &str) -> Result<String, SummarizeError> {
        let messages = vec![
            ChatMessage {
                role: "system".to_owned(),
                content: system_prompt.to_owned(),
            },
            ChatMessage {
                role: "user".to_owned(),
                content: user_prompt.to_owned(),
            },
        ];

        let body = ChatRequest {
            model: self.model.clone(),
            messages,
            max_tokens: MAX_COMPLETION_TOKENS,
            // Request JSON output — supported by OpenAI and recent Ollama builds.
            response_format: Some(ResponseFormat {
                kind: "json_object".to_owned(),
            }),
            temperature: 0.2,
        };

        debug!(endpoint = %self.endpoint, "sending chat completion request");

        let mut request = self
            .http
            .post(&self.endpoint)
            .header("Content-Type", "application/json");

        if let Some(key) = &self.api_key {
            request = request.header("Authorization", format!("Bearer {key}"));
        }

        let response = request.json(&body).send().await?;
        let status = response.status();

        if !status.is_success() {
            let status_u16 = status.as_u16();
            let body_text = response.text().await.unwrap_or_default();
            // Try to extract a structured error message.
            let message = serde_json::from_str::<ApiErrorBody>(&body_text)
                .map(|e| e.error.message)
                .unwrap_or(body_text.clone());
            warn!(status = status_u16, error = %message, "LLM API returned error");
            return Err(SummarizeError::ApiError {
                status: status_u16,
                body: message,
            });
        }

        let chat_response: ChatResponse = response.json().await?;

        let content = chat_response
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .ok_or(SummarizeError::EmptyResponse)?;

        debug!(chars = content.len(), "received chat completion response");
        Ok(content)
    }
}

#[async_trait]
impl Summarizer for OpenAiClient {
    /// Summarize the transcript by calling the configured LLM endpoint.
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

        parse_response(&raw, &self.backend_name, &self.model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_endpoint_normalisation_trailing_slash() {
        let client = OpenAiClient::new("http://localhost:11434/", None, "llama3", "ollama")
            .expect("build client");
        assert_eq!(
            client.endpoint,
            "http://localhost:11434/v1/chat/completions"
        );
    }

    #[test]
    fn test_endpoint_normalisation_no_slash() {
        let client = OpenAiClient::new("http://localhost:11434", None, "llama3", "ollama")
            .expect("build client");
        assert_eq!(
            client.endpoint,
            "http://localhost:11434/v1/chat/completions"
        );
    }

    #[test]
    fn test_client_stores_model_and_backend() {
        let client = OpenAiClient::new("http://localhost:11434", None, "my-model", "my-backend")
            .expect("build client");
        assert_eq!(client.model, "my-model");
        assert_eq!(client.backend_name, "my-backend");
    }

    #[tokio::test]
    async fn test_summarize_empty_transcript_returns_error() {
        let client = OpenAiClient::new("http://localhost:11434", None, "llama3", "ollama")
            .expect("build client");
        let result = client.summarize(&[]).await;
        assert!(matches!(result, Err(SummarizeError::EmptyTranscript)));
    }
}
