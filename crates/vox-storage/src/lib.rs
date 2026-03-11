#![warn(clippy::all, clippy::pedantic)]

//! Session storage, JSON persistence, and Markdown export for Vox Daemon.
//!
//! This crate implements the [`SessionStore`] trait and provides a
//! [`JsonFileStore`] that persists sessions as JSON files under the XDG
//! data directory (`$XDG_DATA_HOME/vox-daemon/sessions/`).

pub mod markdown;
pub mod store;

pub use store::{JsonFileStore, SessionStore};
