#![warn(clippy::all, clippy::pedantic)]

//! LLM-powered summarization for Vox Daemon.
//!
//! # Overview
//!
//! This crate implements the `vox-summarize` component described in PRD §4.4.
//! It exposes a [`Summarizer`] trait and concrete implementations that call
//! OpenAI-compatible LLM APIs (Ollama, `OpenAI`, any compatible endpoint).
//!
//! # Quick start
//!
//! ```no_run
//! use vox_core::config::SummarizationConfig;
//! use vox_summarize::factory::create_summarizer;
//! use vox_summarize::traits::Summarizer;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let config = SummarizationConfig {
//!     backend: "ollama".to_owned(),
//!     ollama_url: "http://localhost:11434".to_owned(),
//!     ollama_model: "qwen2.5:1.5b".to_owned(),
//!     ..Default::default()
//! };
//! let summarizer = create_summarizer(&config)?;
//! // let summary = summarizer.summarize(&transcript).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Module layout
//!
//! | Module | Purpose |
//! |--------|---------|
//! | [`error`] | [`SummarizeError`] enum |
//! | [`traits`] | [`Summarizer`] trait |
//! | [`prompt`] | Builds system + user prompts from transcript segments |
//! | [`parse`] | Parses LLM text responses into [`Summary`] structs |
//! | [`client`] | [`OpenAiClient`] — HTTP client for any OpenAI-compatible API |
//! | [`ollama`] | [`OllamaClient`] — native Ollama `/api/chat` client |
//! | [`factory`] | [`create_summarizer`] factory function |
//! | [`stub`] | [`StubSummarizer`] — test-friendly no-op implementation |

pub mod client;
pub mod error;
pub mod factory;
pub mod ollama;
pub mod parse;
pub mod prompt;
pub mod stub;
pub mod traits;

// Convenience re-exports for callers that just want the top-level surface.
pub use client::OpenAiClient;
pub use error::SummarizeError;
pub use factory::create_summarizer;
pub use ollama::OllamaClient;
pub use stub::StubSummarizer;
pub use traits::Summarizer;
