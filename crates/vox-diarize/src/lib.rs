#![warn(clippy::all, clippy::pedantic)]

//! Speaker diarization for vox-daemon.
//!
//! This crate provides a [`Diarizer`] trait for assigning speaker labels to
//! transcript segments based on voice similarity, plus two implementations:
//!
//! - [`StubDiarizer`] — a no-op pass-through (always compiled).
//! - [`OnnxDiarizer`] — ONNX-based speaker embedding extraction + clustering
//!   (behind the `onnx` feature flag).
//!
//! # Feature flags
//!
//! - `onnx` — enables the [`OnnxDiarizer`], [`OnnxEmbedder`], and ONNX model
//!   management.  Requires the `ort` crate (ONNX Runtime bindings).

pub mod clustering;
pub mod error;
pub mod model;
pub mod stub;
pub mod traits;

#[cfg(feature = "onnx")]
pub mod embeddings;
#[cfg(feature = "onnx")]
pub mod onnx_diarizer;

// Re-exports for ergonomic use.
pub use error::DiarizeError;
pub use stub::StubDiarizer;
pub use traits::{DiarizationRequest, DiarizationResult, Diarizer};

#[cfg(feature = "onnx")]
pub use embeddings::OnnxEmbedder;
#[cfg(feature = "onnx")]
pub use onnx_diarizer::OnnxDiarizer;
