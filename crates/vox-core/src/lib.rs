#![warn(clippy::all, clippy::pedantic)]

//! Shared types, configuration, error handling, and XDG path utilities
//! for the Vox Daemon workspace.

pub mod config;
pub mod error;
pub mod paths;
pub mod session;
