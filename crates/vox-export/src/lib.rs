#![warn(clippy::all, clippy::pedantic)]

//! Plugin-based exporter framework for Vox Daemon.
//!
//! # Overview
//!
//! This crate lets Vox Daemon push transcripts and summaries out to third-party
//! knowledgebases. The first (and currently only) implementation is
//! [`affine::AffineTarget`], which pushes sessions into an `AFFiNE` workspace.
//!
//! # Plugin shape
//!
//! An export "plugin" is any type implementing [`traits::ExportTarget`]. The
//! GUI discovers enabled targets via [`factory::build_targets`], which reads
//! [`vox_core::config::ExportConfig`] and returns one boxed trait object per
//! enabled target.
//!
//! # Module layout
//!
//! | Module | Purpose |
//! |--------|---------|
//! | [`error`] | [`ExportError`] enum shared across all targets |
//! | [`traits`] | [`ExportTarget`] trait + request/response types |
//! | [`factory`] | [`build_targets`](factory::build_targets) factory function |
//! | [`affine`] | `AFFiNE` (cloud + self-hosted) target implementation |

pub mod affine;
pub mod error;
pub mod factory;
pub mod traits;

pub use error::ExportError;
pub use factory::build_targets;
pub use traits::{ExportRequest, ExportResult, ExportTarget, Folder, Workspace};
