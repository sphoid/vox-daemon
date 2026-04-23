//! Core trait and DTOs for third-party export targets.

use async_trait::async_trait;
use vox_core::session::Session;

use crate::error::ExportError;

/// A top-level container on the remote service (e.g. an `AFFiNE` workspace,
/// a Notion workspace, a Google Drive account).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    /// Opaque identifier used by the target to refer to this container.
    pub id: String,
    /// Human-readable display name for UI pickers.
    pub name: String,
}

impl std::fmt::Display for Workspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

/// A nested destination within a [`Workspace`] (e.g. an `AFFiNE` parent doc,
/// a Notion page, a Drive folder).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    /// Opaque identifier used by the target to refer to this folder.
    pub id: String,
    /// Human-readable title shown in UI pickers.
    pub title: String,
}

impl std::fmt::Display for Folder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.title)
    }
}

/// Arguments for [`ExportTarget::export`].
///
/// Borrows the session and pre-rendered Markdown rather than owning them so
/// callers can reuse the same data for multiple targets without clones.
pub struct ExportRequest<'a> {
    /// Workspace to write into.
    pub workspace_id: String,
    /// Optional parent folder inside the workspace. `None` means the workspace
    /// root.
    pub parent_id: Option<String>,
    /// User-chosen title for the new remote document.
    pub title: String,
    /// Markdown body, pre-rendered via `vox_storage::render_export`.
    pub content_markdown: &'a str,
    /// Full session, available if the target wants to include metadata (date,
    /// duration, participants, etc.) beyond the rendered Markdown.
    pub session: &'a Session,
}

/// Result of a successful export.
#[derive(Debug, Clone)]
pub struct ExportResult {
    /// Opaque identifier of the newly created remote document.
    pub remote_id: String,
    /// Clickable URL, if the target can construct one. Surfaced in the
    /// success notification.
    pub remote_url: Option<String>,
}

/// Trait that every export plugin implements.
///
/// Implementations must be `Send + Sync` to be usable across Tokio task
/// boundaries and shared from the GUI state.
#[async_trait]
pub trait ExportTarget: Send + Sync {
    /// Stable, machine-friendly identifier for this target (e.g. `"affine"`).
    fn id(&self) -> &'static str;

    /// Human-readable display name for UI pickers (e.g. `"AFFiNE"`).
    fn display_name(&self) -> &'static str;

    /// List top-level containers (workspaces) available to the authenticated
    /// user.
    ///
    /// # Errors
    ///
    /// Returns [`ExportError`] on auth failure, transport failure, or when the
    /// remote response cannot be parsed.
    async fn list_workspaces(&self) -> Result<Vec<Workspace>, ExportError>;

    /// List folders (parent docs / pages / directories) within the given
    /// workspace.
    ///
    /// # Errors
    ///
    /// Returns [`ExportError`] on auth failure, transport failure, or when the
    /// workspace id is unknown.
    async fn list_folders(&self, workspace_id: &str) -> Result<Vec<Folder>, ExportError>;

    /// Create a new folder inside the given workspace, optionally under an
    /// existing parent folder.
    ///
    /// # Errors
    ///
    /// Returns [`ExportError`] on auth failure or if the parent cannot be
    /// located.
    async fn create_folder(
        &self,
        workspace_id: &str,
        parent_id: Option<&str>,
        title: &str,
    ) -> Result<Folder, ExportError>;

    /// Push a session to the remote service as a new document.
    ///
    /// # Errors
    ///
    /// Returns [`ExportError`] if auth fails, the document cannot be created,
    /// or the remote rejects the content.
    async fn export(&self, request: ExportRequest<'_>) -> Result<ExportResult, ExportError>;
}
