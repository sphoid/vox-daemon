//! Socket.IO client wrapper for `AFFiNE`'s realtime doc-sync endpoint.
//!
//! Connects to `wss://<host>/socket.io/`, then emits Socket.IO events
//! (`space:join`, `space:push-doc-update`) to push Yjs CRDT updates.

use std::time::Duration;

use base64::Engine as _;
use rust_socketio::{
    Payload,
    asynchronous::{Client as SioClient, ClientBuilder},
};
use serde_json::{Value, json};
use tracing::debug;

use super::auth::AuthHeaders;
use crate::error::ExportError;

/// Client-version string echoed to `space:join`. Must be recent enough that
/// the `AFFiNE` server does not reject us as an outdated client.
const CLIENT_VERSION: &str = "0.26.0";

/// Timeout for each Socket.IO ack round-trip.
///
/// Set generously: `AFFiNE` servers can take 15–25 s to ack the first
/// `space:join` on a fresh socket (cold-load of the workspace's Yjs state
/// from persistent storage). Subsequent emits are typically sub-10 ms.
const ACK_TIMEOUT: Duration = Duration::from_secs(45);

/// Connected Socket.IO client scoped to one `AFFiNE` workspace session.
pub struct WorkspaceSocket {
    client: SioClient,
}

impl WorkspaceSocket {
    /// Open a Socket.IO connection to `ws_url`, injecting auth headers.
    ///
    /// # Errors
    ///
    /// Returns [`ExportError::Transport`] when the socket cannot be opened.
    pub async fn connect(ws_url: &str, auth: &AuthHeaders) -> Result<Self, ExportError> {
        let mut builder =
            ClientBuilder::new(ws_url).transport_type(rust_socketio::TransportType::Websocket);
        if let Some(b) = &auth.bearer {
            builder = builder.opening_header("Authorization", format!("Bearer {b}"));
        }
        if let Some(c) = &auth.cookie {
            builder = builder.opening_header("Cookie", c.clone());
        }

        let client = builder
            .connect()
            .await
            .map_err(|e| ExportError::Transport(format!("socket connect failed: {e}")))?;
        Ok(Self { client })
    }

    /// Emit `space:join` for the given workspace and await the ack.
    ///
    /// # Errors
    ///
    /// Returns [`ExportError::Transport`] if the server acks with an error
    /// or the round-trip times out.
    pub async fn join(&self, workspace_id: &str) -> Result<(), ExportError> {
        let payload = json!({
            "spaceType": "workspace",
            "spaceId": workspace_id,
            "clientVersion": CLIENT_VERSION,
        });
        let ack = self.emit_with_ack("space:join", payload).await?;
        check_ack_error("space:join", &ack)
    }

    /// Emit `space:leave` so the server releases its adapter binding for
    /// this space. Call between `join`s when reusing one socket across
    /// multiple workspaces.
    ///
    /// # Errors
    ///
    /// Returns [`ExportError::Transport`] if the server acks with an error.
    pub async fn leave(&self, workspace_id: &str) -> Result<(), ExportError> {
        let payload = json!({
            "spaceType": "workspace",
            "spaceId": workspace_id,
        });
        let ack = self.emit_with_ack("space:leave", payload).await?;
        check_ack_error("space:leave", &ack)
    }

    /// Emit `space:push-doc-update` with a base64-encoded Yjs update.
    ///
    /// # Errors
    ///
    /// Returns [`ExportError::Transport`] if the server acks with an error.
    pub async fn push_doc_update(
        &self,
        workspace_id: &str,
        doc_id: &str,
        update_base64: &str,
    ) -> Result<(), ExportError> {
        let payload = json!({
            "spaceType": "workspace",
            "spaceId": workspace_id,
            "docId": doc_id,
            "update": update_base64,
        });
        let ack = self.emit_with_ack("space:push-doc-update", payload).await?;
        check_ack_error("space:push-doc-update", &ack)
    }

    /// Emit `space:load-doc` and decode the server's Yjs update payload.
    ///
    /// Returns `Ok(None)` when the doc does not yet exist (server replies
    /// with `error.name = "DOC_NOT_FOUND"`) — that is a valid response for a
    /// fresh workspace whose root doc has never been persisted.
    ///
    /// # Errors
    ///
    /// Returns [`ExportError::Transport`] on any other server error or
    /// [`ExportError::ParseError`] if the base64 payload is malformed.
    pub async fn load_doc(
        &self,
        workspace_id: &str,
        doc_id: &str,
    ) -> Result<Option<Vec<u8>>, ExportError> {
        let payload = json!({
            "spaceType": "workspace",
            "spaceId": workspace_id,
            "docId": doc_id,
        });
        let raw = self.emit_with_ack("space:load-doc", payload).await?;
        let ack = unwrap_ack_body(&raw);

        if let Some(err_obj) = ack.get("error") {
            let name = err_obj.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if name == "DOC_NOT_FOUND" {
                return Ok(None);
            }
            let msg = err_obj
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            return Err(ExportError::Transport(format!("space:load-doc: {msg}")));
        }

        // Accept both server shapes observed in the wild:
        //   { data: { missing: "…" } }   (documented)
        //   { missing: "…" }             (un-wrapped)
        let missing = ack
            .get("data")
            .and_then(|d| d.get("missing"))
            .or_else(|| ack.get("missing"))
            .and_then(|v| v.as_str());
        let Some(b64) = missing else {
            debug!(
                workspace_id,
                doc_id,
                ack = %raw,
                "space:load-doc: no `missing` field in ack"
            );
            return Ok(None);
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| ExportError::ParseError {
                reason: format!("space:load-doc base64 decode: {e}"),
            })?;
        Ok(Some(bytes))
    }

    /// Gracefully close the Socket.IO connection. Errors are logged and
    /// swallowed — a failed disconnect should not mask a successful push.
    pub async fn disconnect(self) {
        if let Err(e) = self.client.disconnect().await {
            debug!(error = %e, "socket disconnect failed");
        }
    }

    /// Emit an event and await its server ack, returning the raw JSON of the
    /// ack payload.
    async fn emit_with_ack(
        &self,
        event: &'static str,
        payload: Value,
    ) -> Result<Value, ExportError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let tx = std::sync::Arc::new(std::sync::Mutex::new(Some(tx)));

        let callback_tx = tx.clone();
        self.client
            .emit_with_ack(
                event,
                Payload::Text(vec![payload]),
                ACK_TIMEOUT,
                move |ack, _| {
                    let callback_tx = callback_tx.clone();
                    Box::pin(async move {
                        let value = match ack {
                            Payload::Text(v) => v.into_iter().next().unwrap_or(Value::Null),
                            _ => Value::Null,
                        };
                        if let Some(sender) = callback_tx.lock().ok().and_then(|mut g| g.take()) {
                            let _ = sender.send(value);
                        }
                    })
                },
            )
            .await
            .map_err(|e| ExportError::Transport(format!("{event} emit failed: {e}")))?;

        let ack = tokio::time::timeout(ACK_TIMEOUT + Duration::from_secs(1), rx)
            .await
            .map_err(|_| ExportError::Transport(format!("{event} ack timeout")))?
            .map_err(|_| ExportError::Transport(format!("{event} ack channel dropped")))?;
        if tracing::enabled!(tracing::Level::DEBUG) {
            let shape = ack_debug_shape(&ack);
            debug!(event, ack_shape = %shape, "ack received");
        }
        Ok(ack)
    }
}

/// Summarise the ack JSON for diagnostic logs without dumping huge base64
/// blobs into the terminal.
fn ack_debug_shape(ack: &Value) -> String {
    match ack {
        Value::Null => "null".to_owned(),
        Value::Object(map) => {
            let keys: Vec<String> = map
                .iter()
                .map(|(k, v)| match v {
                    Value::Object(inner) => {
                        let inner_keys: Vec<&str> = inner.keys().map(String::as_str).collect();
                        format!("{k}: {{ {} }}", inner_keys.join(", "))
                    }
                    Value::String(s) if s.len() > 40 => format!("{k}: <string len={}>", s.len()),
                    other => format!("{k}: {other}"),
                })
                .collect();
            format!("{{ {} }}", keys.join(", "))
        }
        Value::Array(a) => format!("array len={}", a.len()),
        other => other.to_string(),
    }
}

/// Treat the `error` field (if present) of an ack as a transport failure.
/// Used by events that have no graceful "not found" path.
fn check_ack_error(event: &'static str, ack: &Value) -> Result<(), ExportError> {
    let body = unwrap_ack_body(ack);
    if let Some(err_obj) = body.get("error") {
        let msg = err_obj
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        return Err(ExportError::Transport(format!("{event}: {msg}")));
    }
    Ok(())
}

/// Unwrap a single-element JSON array wrapper around an ack body.
///
/// `AFFiNE`'s `NestJS` gateway sometimes sends acks as `cb([{…}])` rather
/// than `cb({…})` — same semantic payload wrapped in a one-element array.
/// Peel it so downstream `.get("data")` / `.get("error")` calls work
/// regardless of the wrapper shape.
fn unwrap_ack_body(ack: &Value) -> &Value {
    if let Value::Array(items) = ack {
        if items.len() == 1 {
            return &items[0];
        }
    }
    ack
}
