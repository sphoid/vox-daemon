#![warn(clippy::all, clippy::pedantic)]

//! Audio capture layer for Vox Daemon.
//!
//! This crate connects to the `PipeWire` daemon, enumerates audio streams, and
//! captures two separate streams simultaneously:
//!
//! 1. The user's microphone (`PipeWire` source output).
//! 2. The video conferencing application's audio (`PipeWire` sink input).
//!
//! All captured audio is resampled to **16 kHz mono f32 PCM** as required by
//! Whisper before being sent to the caller via a [`crossbeam_channel`] channel.
//!
//! # Threading model
//!
//! `PipeWire`'s main loop is **not** compatible with Tokio. This crate spawns a
//! dedicated OS thread for the `PipeWire` event loop. Audio data and lifecycle
//! events are communicated back to the async Tokio context through
//! [`crossbeam_channel`] channels.
//!
//! # Feature flags
//!
//! | Flag | Description |
//! |------|-------------|
//! | `pw` | Enable the real `PipeWire` backend (requires `libpipewire-0.3-dev`). |
//! | `integration` | Enables `pw` and unlocks integration tests that run against a live `PipeWire` daemon. |
//!
//! Without the `pw` feature the full public API is still available: all types,
//! the [`AudioSource`] trait, the resampler, and the [`mock`] module compile
//! and work without libpipewire. Only [`pw::PipeWireSource`] is absent.
//! This allows CI environments without `libpipewire-0.3-dev` to build and
//! test all non-hardware code paths.

pub mod error;
pub mod resample;
pub mod source;
pub mod types;

#[cfg(feature = "pw")]
pub mod pw;

pub mod mock;

// Convenience re-exports so callers only need to import from `vox_capture`.
pub use error::CaptureError;
pub use source::AudioSource;
pub use types::{AudioChunk, StreamFilter, StreamInfo, StreamRole};

#[cfg(feature = "pw")]
pub use pw::PipeWireSource;
