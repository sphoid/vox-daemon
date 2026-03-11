#![warn(clippy::all, clippy::pedantic)]

//! System tray icon with popup menu for Vox Daemon.
//!
//! This crate provides:
//!
//! - [`TrayEvent`] — events emitted by the tray icon when the user interacts
//!   with the popup menu.
//! - [`DaemonStatus`] — represents the current daemon state (idle, recording,
//!   processing); used to update the tray icon appearance.
//! - [`Tray`] — the core trait. Implementors manage an OS-level tray icon.
//! - [`MockTray`] — a test-friendly implementation that captures events in a
//!   channel without requiring GTK.
//!
//! # Feature flags
//!
//! | Feature | Effect |
//! |---------|--------|
//! | `gtk`   | Enables [`SystemTray`], which uses `tray-icon` + `muda` with a GTK event loop. Requires `libayatana-appindicator3-dev` and `libgtk-3-dev` at link time. |
//!
//! When the `gtk` feature is not enabled all public types except [`SystemTray`]
//! are still available and compile cleanly.

mod error;
mod event;
mod mock;
mod tray;

#[cfg(feature = "gtk")]
mod system_tray;

pub use error::TrayError;
pub use event::{DaemonStatus, TrayEvent};
pub use mock::MockTray;
pub use tray::Tray;

#[cfg(feature = "gtk")]
pub use system_tray::SystemTray;
