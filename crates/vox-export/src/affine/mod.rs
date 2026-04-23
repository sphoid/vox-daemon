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
use tokio::task::JoinSet;
use tracing::{debug, info, instrument, warn};
use vox_core::config::AffineExportConfig;

use self::auth::AuthHeaders;
use crate::{
    error::ExportError,
    traits::{ExportRequest, ExportResult, ExportTarget, Folder, Workspace},
};

/// Per-workspace timeout for the realtime name fetch.  Keeps a single slow
/// workspace from blocking the rest of the list; callers fall back to the
/// short-id placeholder on timeout.
const WORKSPACE_NAME_FETCH_TIMEOUT: Duration = Duration::from_secs(5);

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

        // `AFFiNE` does not expose workspace names via GraphQL — they live in
        // each workspace's root Yjs doc under `meta.name`. Fan out the
        // realtime fetches so the picker doesn't stall on a slow workspace;
        // individual failures / timeouts fall back to the short-id name.
        let ws_url = self.ws_url();
        let mut set: JoinSet<Workspace> = JoinSet::new();
        for id in ids {
            let ws_url = ws_url.clone();
            let headers = headers.clone();
            set.spawn(async move {
                let name = tokio::time::timeout(
                    WORKSPACE_NAME_FETCH_TIMEOUT,
                    fetch_workspace_name(&ws_url, &headers, &id),
                )
                .await;

                let resolved = match name {
                    Ok(Ok(Some(n))) => n,
                    Ok(Ok(None)) => graphql::fallback_workspace_name(&id),
                    Ok(Err(e)) => {
                        warn!(workspace = %id, error = %e,
                            "realtime workspace-name fetch failed; using id");
                        graphql::fallback_workspace_name(&id)
                    }
                    Err(_) => {
                        warn!(workspace = %id, "workspace-name fetch timed out; using id");
                        graphql::fallback_workspace_name(&id)
                    }
                };
                Workspace { id, name: resolved }
            });
        }

        let mut workspaces = Vec::new();
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok(ws) => workspaces.push(ws),
                Err(e) => warn!(error = %e, "workspace-name task panicked"),
            }
        }
        // Sort by display name so the picker order is stable across calls.
        workspaces.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(workspaces)
    }

    #[instrument(skip(self))]
    async fn list_folders(&self, workspace_id: &str) -> Result<Vec<Folder>, ExportError> {
        let headers = self.auth.ensure_headers(&self.http, &self.base_url).await?;
        graphql::list_docs(&self.http, &self.base_url, &headers, workspace_id).await
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

/// Open a short-lived Socket.IO connection, load the workspace's root Yjs
/// doc, and extract `meta.name`.  Returns `Ok(None)` when the doc exists
/// but has no name set (e.g. fresh workspace).
async fn fetch_workspace_name(
    ws_url: &str,
    headers: &AuthHeaders,
    workspace_id: &str,
) -> Result<Option<String>, ExportError> {
    let client = socket::WorkspaceSocket::connect(ws_url, headers).await?;
    client.join(workspace_id).await?;
    let bytes = client.load_doc(workspace_id, workspace_id).await?;
    client.disconnect().await;
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    Ok(ydoc::extract_workspace_name(&bytes))
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
