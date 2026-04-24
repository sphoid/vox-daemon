//! `AFFiNE` export target.
//!
//! Supports both `AFFiNE` Cloud (`app.affine.pro`, requires a personal access
//! token — email/password sign-in is blocked by Cloudflare) and self-hosted
//! instances (token or email/password).
//!
//! # Wire protocol
//!
//! `AFFiNE` does not expose a "create document with markdown" HTTP endpoint.
//! Documents are Yjs CRDT blobs synchronised over Socket.IO. Exporting is a
//! four-step dance:
//!
//! 1. Authenticate — either use the configured [`AffineExportConfig::api_token`]
//!    directly, or exchange `email`/`password` for a session cookie via
//!    `POST /api/auth/sign-in` (self-hosted only).
//! 2. Query GraphQL at `/graphql` for workspace / doc metadata.
//! 3. Connect to `wss://{host}/socket.io/`, join the workspace, and push a
//!    freshly-constructed Yjs update containing the new document's blocks.
//! 4. Amend the workspace's root Yjs doc (stored at the workspace id) to add
//!    a `pages` entry so the doc appears in the workspace sidebar.
//!
//! Nesting under a parent doc is handled by appending an
//! `affine:embed-linked-doc` block to that parent.

pub mod auth;
pub mod graphql;
pub mod socket;
pub mod ydoc;

use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use reqwest::Client;
use tracing::{debug, info, instrument, warn};
use vox_core::config::AffineExportConfig;

use crate::{
    error::ExportError,
    traits::{ExportRequest, ExportResult, ExportTarget, Folder, Workspace},
};

/// Per-workspace timeout for the realtime name fetch.  Covers join +
/// load-doc + leave.  Must be > the Socket.IO ack timeout because the
/// first `space:join` on a fresh socket can take 15–25 s while the
/// `AFFiNE` server cold-loads workspace state.
const WORKSPACE_NAME_FETCH_TIMEOUT: Duration = Duration::from_secs(60);

/// `AFFiNE` export target.
pub struct AffineTarget {
    /// Normalised base URL (no trailing slash).
    base_url: String,
    /// Shared HTTP client used for REST + GraphQL calls.
    http: Client,
    /// Authentication strategy.
    auth: auth::AuthState,
}

impl AffineTarget {
    /// Build an [`AffineTarget`] from its config section.
    ///
    /// # Errors
    ///
    /// Returns [`ExportError::Config`] when required fields are missing or
    /// when the HTTP client cannot be constructed.
    pub fn from_config(cfg: &AffineExportConfig) -> Result<Self, ExportError> {
        if cfg.base_url.trim().is_empty() {
            return Err(ExportError::Config(
                "affine.base_url must not be empty".to_owned(),
            ));
        }
        let base_url = cfg.base_url.trim_end_matches('/').to_owned();

        // Cloud blocks password sign-in; require a token.
        let is_cloud = base_url.contains("affine.pro");
        if is_cloud && cfg.api_token.trim().is_empty() {
            return Err(ExportError::Config(
                "affine.api_token is required for AFFiNE Cloud — \
                 password sign-in is blocked; generate a token under \
                 Settings → Integrations → MCP Server"
                    .to_owned(),
            ));
        }
        if cfg.api_token.trim().is_empty() && cfg.email.trim().is_empty() {
            return Err(ExportError::Config(
                "affine: set either api_token or email + password".to_owned(),
            ));
        }

        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| ExportError::Config(format!("failed to build HTTP client: {e}")))?;

        let auth = if cfg.api_token.trim().is_empty() {
            auth::AuthState::password(cfg.email.trim().to_owned(), cfg.password.clone())
        } else {
            auth::AuthState::token(cfg.api_token.trim().to_owned())
        };

        Ok(Self {
            base_url,
            http,
            auth,
        })
    }

    /// WebSocket URL derived from [`Self::base_url`].
    fn ws_url(&self) -> String {
        if let Some(rest) = self.base_url.strip_prefix("https://") {
            format!("wss://{rest}")
        } else if let Some(rest) = self.base_url.strip_prefix("http://") {
            format!("ws://{rest}")
        } else {
            self.base_url.clone()
        }
    }
}

#[async_trait]
impl ExportTarget for AffineTarget {
    fn id(&self) -> &'static str {
        "affine"
    }

    fn display_name(&self) -> &'static str {
        "AFFiNE"
    }

    #[instrument(skip(self), fields(base_url = %self.base_url))]
    async fn list_workspaces(&self) -> Result<Vec<Workspace>, ExportError> {
        let headers = self.auth.ensure_headers(&self.http, &self.base_url).await?;
        let ids = graphql::list_workspace_ids(&self.http, &self.base_url, &headers).await?;

        // `AFFiNE` does not expose workspace names via GraphQL — they live
        // in each workspace's root Yjs doc under `meta.name`.  We open a
        // single Socket.IO connection and serially join each workspace,
        // load its root doc, and leave.  Parallel fan-out was tried first
        // but `AFFiNE` does not ack `space:join` when multiple fresh
        // sockets race each other from the same user session.
        let ws_url = self.ws_url();
        let mut workspaces = Vec::new();
        let client = match socket::WorkspaceSocket::connect(&ws_url, &headers).await {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e,
                    "failed to open socket for workspace-name lookup; using id placeholders");
                for id in ids {
                    workspaces.push(graphql::workspace_with_fallback_name(id));
                }
                workspaces.sort_by(|a, b| a.name.cmp(&b.name));
                return Ok(workspaces);
            }
        };

        // First pass.  The first `space:join` on a fresh socket takes
        // 15–45s while the `AFFiNE` server cold-loads workspace state,
        // which tends to eat whichever workspace goes first.  We track
        // those so we can retry them once the socket is warm.
        let mut retry_ids = Vec::new();
        for id in &ids {
            match resolve_workspace_name(&client, id).await {
                Ok(Some(name)) => workspaces.push(Workspace {
                    id: id.clone(),
                    name,
                }),
                Ok(None) => {
                    // Doc loaded but carries no `meta.name` — no point
                    // retrying; keep the placeholder.
                    workspaces.push(graphql::workspace_with_fallback_name(id.clone()));
                }
                Err(()) => retry_ids.push(id.clone()),
            }
        }

        // Retry any that timed out / errored on the cold socket.  By now
        // the socket is warm so these typically finish in milliseconds.
        for id in retry_ids {
            let name = match resolve_workspace_name(&client, &id).await {
                Ok(Some(n)) => n,
                Ok(None) | Err(()) => graphql::fallback_workspace_name(&id),
            };
            workspaces.push(Workspace { id, name });
        }

        client.disconnect().await;
        workspaces.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(workspaces)
    }

    #[instrument(skip(self))]
    async fn list_folders(&self, workspace_id: &str) -> Result<Vec<Folder>, ExportError> {
        // `AFFiNE`'s GraphQL `workspace(id).docs` returns `title: null` on
        // self-hosted instances — the real title lives in each doc's Yjs
        // body, and the workspace's root doc mirrors a `{id, title}` map
        // of all its pages under `meta.pages`.  Load that once instead of
        // fanning out per-doc.
        let headers = self.auth.ensure_headers(&self.http, &self.base_url).await?;
        let ws_url = self.ws_url();
        let client = socket::WorkspaceSocket::connect(&ws_url, &headers).await?;
        client.join(workspace_id).await?;
        let bytes = client.load_doc(workspace_id, workspace_id).await?;
        let _ = client.leave(workspace_id).await;
        client.disconnect().await;

        let Some(bytes) = bytes else {
            return Ok(Vec::new());
        };
        let pages = ydoc::extract_workspace_pages(&bytes);
        let mut folders: Vec<Folder> = pages
            .into_iter()
            .map(|p| Folder {
                id: p.id,
                title: p.title.unwrap_or_else(|| "Untitled".to_owned()),
            })
            .collect();
        folders.sort_by(|a, b| a.title.cmp(&b.title));
        Ok(folders)
    }

    #[instrument(skip(self, title))]
    async fn create_folder(
        &self,
        workspace_id: &str,
        parent_id: Option<&str>,
        title: &str,
    ) -> Result<Folder, ExportError> {
        let headers = self.auth.ensure_headers(&self.http, &self.base_url).await?;
        let ws_url = self.ws_url();
        let doc_id = uuid::Uuid::new_v4().to_string();

        // Build an empty-bodied doc so it shows up as a usable parent.
        let doc_update = ydoc::build_doc_update(&doc_id, title, "")?;
        let workspace_meta_update = ydoc::build_workspace_meta_append(&doc_id, title);

        let client = socket::WorkspaceSocket::connect(&ws_url, &headers).await?;
        client.join(workspace_id).await?;
        client
            .push_doc_update(workspace_id, &doc_id, &encode(&doc_update))
            .await?;
        client
            .push_doc_update(workspace_id, workspace_id, &encode(&workspace_meta_update))
            .await?;

        // Nest under an existing parent if requested.
        if let Some(parent) = parent_id {
            let embed_update = ydoc::build_embed_linked_doc_block(parent, &doc_id)?;
            client
                .push_doc_update(workspace_id, parent, &encode(&embed_update))
                .await?;
        }

        client.disconnect().await;

        info!(doc_id, title, "created affine folder doc");
        Ok(Folder {
            id: doc_id,
            title: title.to_owned(),
        })
    }

    #[instrument(skip(self, request), fields(title = %request.title))]
    async fn export(&self, request: ExportRequest<'_>) -> Result<ExportResult, ExportError> {
        let headers = self.auth.ensure_headers(&self.http, &self.base_url).await?;
        let ws_url = self.ws_url();
        let doc_id = uuid::Uuid::new_v4().to_string();

        debug!(doc_id, "building yjs update for new affine doc");
        let doc_update = ydoc::build_doc_update(&doc_id, &request.title, request.content_markdown)?;
        let workspace_meta_update = ydoc::build_workspace_meta_append(&doc_id, &request.title);

        let client = socket::WorkspaceSocket::connect(&ws_url, &headers).await?;
        client.join(&request.workspace_id).await?;
        client
            .push_doc_update(&request.workspace_id, &doc_id, &encode(&doc_update))
            .await?;
        client
            .push_doc_update(
                &request.workspace_id,
                &request.workspace_id,
                &encode(&workspace_meta_update),
            )
            .await?;

        if let Some(parent) = request.parent_id {
            let embed_update = ydoc::build_embed_linked_doc_block(&parent, &doc_id)?;
            client
                .push_doc_update(&request.workspace_id, &parent, &encode(&embed_update))
                .await?;
        }

        client.disconnect().await;

        let remote_url = Some(format!(
            "{}/workspace/{}/{}",
            self.base_url, request.workspace_id, doc_id
        ));
        info!(doc_id, url = ?remote_url, "exported session to affine");

        Ok(ExportResult {
            remote_id: doc_id,
            remote_url,
        })
    }
}

fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Run one attempt at resolving a workspace's name.
///
/// Return values:
/// - `Ok(Some(name))`: name resolved successfully.
/// - `Ok(None)`: the workspace's root doc loaded but has no `meta.name`.
///   Retrying won't help — caller should keep the placeholder.
/// - `Err(())`: timeout or transport error; caller may retry.
async fn resolve_workspace_name(
    client: &socket::WorkspaceSocket,
    workspace_id: &str,
) -> Result<Option<String>, ()> {
    match tokio::time::timeout(
        WORKSPACE_NAME_FETCH_TIMEOUT,
        fetch_workspace_name_on(client, workspace_id),
    )
    .await
    {
        Ok(Ok(name)) => Ok(name),
        Ok(Err(e)) => {
            warn!(workspace = %workspace_id, error = %e,
                "realtime workspace-name fetch failed; will retry");
            Err(())
        }
        Err(_) => {
            warn!(workspace = %workspace_id, "workspace-name fetch timed out; will retry");
            Err(())
        }
    }
}

/// Join the workspace on a pre-opened socket, load its root Yjs doc,
/// extract `meta.name`, then leave again so the socket is clean for the
/// next workspace.
///
/// Returns `Ok(None)` when the doc exists but has no name set.
async fn fetch_workspace_name_on(
    client: &socket::WorkspaceSocket,
    workspace_id: &str,
) -> Result<Option<String>, ExportError> {
    let overall = std::time::Instant::now();

    let step = std::time::Instant::now();
    client.join(workspace_id).await?;
    debug!(
        workspace_id,
        elapsed_ms = u64::try_from(step.elapsed().as_millis()).unwrap_or(u64::MAX),
        "workspace-name: joined"
    );

    let step = std::time::Instant::now();
    let bytes = client.load_doc(workspace_id, workspace_id).await?;
    debug!(
        workspace_id,
        elapsed_ms = u64::try_from(step.elapsed().as_millis()).unwrap_or(u64::MAX),
        bytes = bytes.as_ref().map_or(0, Vec::len),
        "workspace-name: load-doc returned"
    );

    // Leave so the server's per-space adapter binding is released before
    // the next `join`. Errors here are non-fatal.
    if let Err(e) = client.leave(workspace_id).await {
        debug!(workspace_id, error = %e, "workspace-name: leave failed (non-fatal)");
    }

    let Some(bytes) = bytes else {
        debug!(
            workspace_id,
            total_ms = u64::try_from(overall.elapsed().as_millis()).unwrap_or(u64::MAX),
            "workspace-name: no bytes returned"
        );
        return Ok(None);
    };
    let name = ydoc::extract_workspace_name(&bytes);
    debug!(
        workspace_id,
        total_ms = u64::try_from(overall.elapsed().as_millis()).unwrap_or(u64::MAX),
        resolved = name.is_some(),
        "workspace-name: finished"
    );
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cloud_cfg(token: &str) -> AffineExportConfig {
        AffineExportConfig {
            enabled: true,
            base_url: "https://app.affine.pro".to_owned(),
            api_token: token.to_owned(),
            ..AffineExportConfig::default()
        }
    }

    #[test]
    fn from_config_rejects_empty_base_url() {
        let cfg = AffineExportConfig {
            enabled: true,
            base_url: String::new(),
            api_token: "t".to_owned(),
            ..AffineExportConfig::default()
        };
        assert!(matches!(
            AffineTarget::from_config(&cfg),
            Err(ExportError::Config(_))
        ));
    }

    #[test]
    fn from_config_rejects_cloud_without_token() {
        let cfg = AffineExportConfig {
            enabled: true,
            base_url: "https://app.affine.pro".to_owned(),
            email: "a@b".to_owned(),
            password: "p".to_owned(),
            ..AffineExportConfig::default()
        };
        match AffineTarget::from_config(&cfg) {
            Err(ExportError::Config(msg)) => {
                assert!(msg.contains("api_token"), "unexpected message: {msg}");
            }
            Err(e) => panic!("expected Config error, got {e:?}"),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }

    #[test]
    fn from_config_rejects_missing_auth() {
        let cfg = AffineExportConfig {
            enabled: true,
            base_url: "https://affine.example.com".to_owned(),
            ..AffineExportConfig::default()
        };
        assert!(matches!(
            AffineTarget::from_config(&cfg),
            Err(ExportError::Config(_))
        ));
    }

    #[test]
    fn from_config_accepts_cloud_token() {
        let t = AffineTarget::from_config(&cloud_cfg("ut_secret")).expect("build");
        assert_eq!(t.base_url, "https://app.affine.pro");
    }

    #[test]
    fn from_config_normalises_trailing_slash() {
        let cfg = AffineExportConfig {
            enabled: true,
            base_url: "https://affine.example.com/".to_owned(),
            email: "a".to_owned(),
            password: "p".to_owned(),
            ..AffineExportConfig::default()
        };
        let t = AffineTarget::from_config(&cfg).expect("build");
        assert_eq!(t.base_url, "https://affine.example.com");
    }

    #[test]
    fn ws_url_swaps_scheme() {
        let t = AffineTarget::from_config(&cloud_cfg("t")).expect("build");
        assert_eq!(t.ws_url(), "wss://app.affine.pro");
    }
}
