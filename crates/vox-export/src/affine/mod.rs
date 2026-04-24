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

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use reqwest::Client;
use tokio::sync::{Mutex as TokioMutex, MutexGuard};
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
    /// Lazily-connected Socket.IO client shared across all realtime
    /// operations on this target.  Kept alive for the lifetime of the
    /// `AffineTarget` so every call after the first pays no cold-start
    /// cost (the first `space:join` on a fresh socket takes 20-45 s).
    socket: Arc<TokioMutex<Option<socket::WorkspaceSocket>>>,
    /// Folders (doc metadata) indexed by workspace id, populated as a
    /// side-effect of [`Self::list_workspaces`] and consumed by
    /// [`Self::list_folders`].  Both methods need the workspace root
    /// doc; caching lets the folder picker render instantly.
    folders_cache: Arc<TokioMutex<HashMap<String, Vec<Folder>>>>,
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
            socket: Arc::new(TokioMutex::new(None)),
            folders_cache: Arc::new(TokioMutex::new(HashMap::new())),
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

    /// Acquire the shared Socket.IO client, opening a fresh connection on
    /// first use.  Holding the returned guard serialises realtime ops,
    /// which is required anyway — `AFFiNE` refuses concurrent emits from
    /// the same user session.
    ///
    /// The `.as_ref().expect(…)` pattern at call-sites is safe: this
    /// method installs `Some(…)` before returning.
    async fn acquire_socket(
        &self,
    ) -> Result<MutexGuard<'_, Option<socket::WorkspaceSocket>>, ExportError> {
        let mut guard = self.socket.lock().await;
        if guard.is_none() {
            let headers = self.auth.ensure_headers(&self.http, &self.base_url).await?;
            let ws_url = self.ws_url();
            let client = socket::WorkspaceSocket::connect(&ws_url, &headers).await?;
            *guard = Some(client);
        }
        Ok(guard)
    }

    /// Forget the cached socket.  Next `acquire_socket` will reconnect.
    /// Called on transport errors so a dead socket is not reused.
    async fn reset_socket(&self) {
        let mut guard = self.socket.lock().await;
        if let Some(sock) = guard.take() {
            sock.disconnect().await;
        }
    }
}

// No explicit Drop impl: when `AffineTarget` goes out of scope, the
// `Arc<Mutex<Option<WorkspaceSocket>>>` drops, which drops the inner
// `rust_socketio` client and closes the connection.

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

        // `AFFiNE` exposes neither workspace names nor doc titles via
        // GraphQL — both live in each workspace's root Yjs doc.  Load
        // that once per workspace on the shared socket: cache `meta.name`
        // on the `Workspace` we return, and stash `meta.pages` in
        // `folders_cache` so the subsequent `list_folders` call is
        // instant.
        let mut workspaces = Vec::new();
        let guard = match self.acquire_socket().await {
            Ok(g) => g,
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
        let client = guard.as_ref().expect("acquire_socket returned Some");

        // First pass — the first `space:join` on a freshly-opened socket
        // can take 20-45 s while the server cold-loads workspace state.
        // Track anything that timed out or errored so we can retry once
        // the socket is warm.
        let mut retry_ids = Vec::new();
        for id in &ids {
            match resolve_workspace_data(client, &self.folders_cache, id).await {
                Ok(Some(name)) => workspaces.push(Workspace {
                    id: id.clone(),
                    name,
                }),
                Ok(None) => {
                    workspaces.push(graphql::workspace_with_fallback_name(id.clone()));
                }
                Err(()) => retry_ids.push(id.clone()),
            }
        }

        for id in retry_ids {
            let name = match resolve_workspace_data(client, &self.folders_cache, &id).await {
                Ok(Some(n)) => n,
                Ok(None) | Err(()) => graphql::fallback_workspace_name(&id),
            };
            workspaces.push(Workspace { id, name });
        }

        drop(guard);
        workspaces.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(workspaces)
    }

    #[instrument(skip(self))]
    async fn list_folders(&self, workspace_id: &str) -> Result<Vec<Folder>, ExportError> {
        // Fast path: `list_workspaces` already populated the cache while
        // fetching the workspace's name.
        if let Some(folders) = self.folders_cache.lock().await.get(workspace_id) {
            debug!(
                workspace_id,
                count = folders.len(),
                "list_folders: cache hit"
            );
            return Ok(folders.clone());
        }

        // Slow path (e.g. user opens the picker without `list_workspaces`
        // having been called first): fetch the workspace root doc on the
        // shared socket and extract `meta.pages`.
        let guard = self.acquire_socket().await?;
        let client = guard.as_ref().expect("acquire_socket returned Some");
        let bytes = match join_load_leave(client, workspace_id).await {
            Ok(b) => b,
            Err(e) => {
                drop(guard);
                // Reset so a dead socket is reopened next call.
                self.reset_socket().await;
                return Err(e);
            }
        };
        drop(guard);

        let folders = match bytes {
            Some(b) => pages_to_folders(ydoc::extract_workspace_pages(&b)),
            None => Vec::new(),
        };
        self.folders_cache
            .lock()
            .await
            .insert(workspace_id.to_owned(), folders.clone());
        Ok(folders)
    }

    #[instrument(skip(self, title))]
    async fn create_folder(
        &self,
        workspace_id: &str,
        parent_id: Option<&str>,
        title: &str,
    ) -> Result<Folder, ExportError> {
        let doc_id = uuid::Uuid::new_v4().to_string();
        let doc_update = ydoc::build_doc_update(&doc_id, title, "")?;
        let workspace_meta_update = ydoc::build_workspace_meta_append(&doc_id, title);

        let guard = self.acquire_socket().await?;
        let client = guard.as_ref().expect("acquire_socket returned Some");
        client.join(workspace_id).await?;
        client
            .push_doc_update(workspace_id, &doc_id, &encode(&doc_update))
            .await?;
        client
            .push_doc_update(workspace_id, workspace_id, &encode(&workspace_meta_update))
            .await?;
        if let Some(parent) = parent_id {
            let embed_update = ydoc::build_embed_linked_doc_block(parent, &doc_id)?;
            client
                .push_doc_update(workspace_id, parent, &encode(&embed_update))
                .await?;
        }
        let _ = client.leave(workspace_id).await;
        drop(guard);

        let folder = Folder {
            id: doc_id.clone(),
            title: title.to_owned(),
        };
        // Keep the cache consistent with what the user just created.
        self.folders_cache
            .lock()
            .await
            .entry(workspace_id.to_owned())
            .or_default()
            .push(folder.clone());

        info!(doc_id, title, "created affine folder doc");
        Ok(folder)
    }

    #[instrument(skip(self, request), fields(title = %request.title))]
    async fn export(&self, request: ExportRequest<'_>) -> Result<ExportResult, ExportError> {
        let doc_id = uuid::Uuid::new_v4().to_string();

        debug!(doc_id, "building yjs update for new affine doc");
        let doc_update = ydoc::build_doc_update(&doc_id, &request.title, request.content_markdown)?;
        let workspace_meta_update = ydoc::build_workspace_meta_append(&doc_id, &request.title);

        let guard = self.acquire_socket().await?;
        let client = guard.as_ref().expect("acquire_socket returned Some");
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

        if let Some(parent) = &request.parent_id {
            let embed_update = ydoc::build_embed_linked_doc_block(parent, &doc_id)?;
            client
                .push_doc_update(&request.workspace_id, parent, &encode(&embed_update))
                .await?;
        }

        let _ = client.leave(&request.workspace_id).await;
        drop(guard);

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

/// Load the workspace root doc once and populate both the returned name
/// and the per-target folders cache.
///
/// Return values:
/// - `Ok(Some(name))`: name resolved successfully; folders cached.
/// - `Ok(None)`: the root doc loaded but has no `meta.name` (retry
///   won't help; caller keeps the placeholder).
/// - `Err(())`: timeout or transport error; caller may retry on the now-
///   warm socket.
async fn resolve_workspace_data(
    client: &socket::WorkspaceSocket,
    folders_cache: &TokioMutex<HashMap<String, Vec<Folder>>>,
    workspace_id: &str,
) -> Result<Option<String>, ()> {
    match tokio::time::timeout(
        WORKSPACE_NAME_FETCH_TIMEOUT,
        join_load_leave(client, workspace_id),
    )
    .await
    {
        Ok(Ok(Some(bytes))) => {
            let name = ydoc::extract_workspace_name(&bytes);
            let folders = pages_to_folders(ydoc::extract_workspace_pages(&bytes));
            folders_cache
                .lock()
                .await
                .insert(workspace_id.to_owned(), folders);
            Ok(name)
        }
        Ok(Ok(None)) => {
            // Doc exists but is empty — no name, no pages.
            folders_cache
                .lock()
                .await
                .insert(workspace_id.to_owned(), Vec::new());
            Ok(None)
        }
        Ok(Err(e)) => {
            warn!(workspace = %workspace_id, error = %e,
                "realtime workspace fetch failed; will retry");
            Err(())
        }
        Err(_) => {
            warn!(workspace = %workspace_id, "workspace fetch timed out; will retry");
            Err(())
        }
    }
}

/// Join a workspace on a pre-opened socket, load its root Yjs doc, and
/// leave. Returns `Ok(None)` if the server reports no existing state.
async fn join_load_leave(
    client: &socket::WorkspaceSocket,
    workspace_id: &str,
) -> Result<Option<Vec<u8>>, ExportError> {
    let overall = std::time::Instant::now();
    client.join(workspace_id).await?;
    let bytes = client.load_doc(workspace_id, workspace_id).await?;
    // Leave is best-effort: we don't want a leave failure to mask an
    // otherwise-successful load.
    if let Err(e) = client.leave(workspace_id).await {
        debug!(workspace_id, error = %e, "workspace root: leave failed (non-fatal)");
    }
    debug!(
        workspace_id,
        total_ms = u64::try_from(overall.elapsed().as_millis()).unwrap_or(u64::MAX),
        bytes = bytes.as_ref().map_or(0, Vec::len),
        "workspace root: fetched"
    );
    Ok(bytes)
}

/// Convert extracted `meta.pages` entries to sorted [`Folder`]s.
fn pages_to_folders(pages: Vec<ydoc::WorkspacePage>) -> Vec<Folder> {
    let mut folders: Vec<Folder> = pages
        .into_iter()
        .map(|p| Folder {
            id: p.id,
            title: p.title.unwrap_or_else(|| "Untitled".to_owned()),
        })
        .collect();
    folders.sort_by(|a, b| a.title.cmp(&b.title));
    folders
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
