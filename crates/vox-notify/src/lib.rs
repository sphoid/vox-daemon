#![warn(clippy::all, clippy::pedantic)]

//! Desktop notification wrapper for Vox Daemon.
//!
//! Provides XDG-compliant desktop notifications via [`notify-rust`] using the
//! D-Bus `org.freedesktop.Notifications` interface.
//!
//! # Usage
//!
//! ```no_run
//! use vox_notify::{DesktopNotifier, Notifier};
//! use vox_core::config::NotificationConfig;
//!
//! let config = NotificationConfig::default();
//! let notifier = DesktopNotifier::new(config);
//! notifier.recording_started().ok();
//! ```
//!
//! For testing without a desktop session, use [`StubNotifier`] which logs
//! instead of sending real notifications.

mod desktop;
mod error;
mod notifier;

pub use desktop::DesktopNotifier;
pub use error::NotifyError;
pub use notifier::{Notifier, StubNotifier};
