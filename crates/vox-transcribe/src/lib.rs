#![warn(clippy::all, clippy::pedantic)]

//! `vox-transcribe` — Whisper-based speech-to-text transcription for Vox Daemon.
//!
//! # Overview
//!
//! This crate provides the [`Transcriber`] trait and supporting types for converting
//! raw PCM audio into timestamped [`TranscriptSegment`]s.
//!
//! Audio is expected as 32-bit float PCM at 16 kHz, mono channel — the format
//! produced by `vox-capture`.
//!
//! # Speaker diarization (v1)
//!
//! The v1 diarization strategy is stream-based: audio arriving from the
//! microphone is labelled `"You"`, while audio from the application stream is
//! labelled `"Remote"`. No clustering-based or neural diarization is performed.
//! Callers indicate the source via [`AudioSourceRole`] inside
//! [`TranscriptionRequest`].
//!
//! # Feature flags
//!
//! | Feature   | Description |
//! |-----------|-------------|
//! | `whisper` | Enables the real [`WhisperTranscriber`] via whisper-rs. Requires whisper.cpp native libraries. |
//! | `cuda`    | Enables NVIDIA CUDA GPU acceleration (implies `whisper`). Mutually exclusive with `hipblas`. |
//! | `hipblas` | Enables AMD ROCm/hipBLAS GPU acceleration (implies `whisper`). Mutually exclusive with `cuda`. |
//!
//! When neither `whisper`, `cuda`, nor `hipblas` is enabled the crate still
//! compiles and exposes a [`StubTranscriber`] that always returns an empty
//! result set — useful for development builds without native dependencies.
//!
//! # Build notes
//!
//! ```text
//! # CPU-only (no native libs required beyond a C compiler):
//! cargo build -p vox-transcribe --features whisper
//!
//! # NVIDIA CUDA:
//! cargo build -p vox-transcribe --features cuda
//!
//! # AMD ROCm:
//! cargo build -p vox-transcribe --features hipblas
//! ```

pub mod model;
pub mod transcriber;

#[cfg(feature = "whisper")]
pub mod whisper;

mod stub;

pub use transcriber::{AudioSourceRole, Transcriber, TranscriptionRequest, TranscriptionResult};

#[cfg(feature = "whisper")]
pub use whisper::WhisperTranscriber;

pub use stub::StubTranscriber;

pub use vox_core::error::TranscribeError;
pub use vox_core::session::TranscriptSegment;
