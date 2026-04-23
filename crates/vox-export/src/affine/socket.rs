//! Socket.IO client wrapper for `AFFiNE`'s realtime doc-sync endpoint.
//!
//! Connects to `wss://<host>/socket.io/`, then emits Socket.IO events
//! (`space:join`, `space:push-doc-update`) to push Yjs CRDT updates.

use std::time::Duration;

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
const ACK_TIMEOUT: Duration = Duration::from_secs(15);

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
        self.emit_with_ack("space:join", payload).await?;
        Ok(())
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
        self.emit_with_ack("space:push-doc-update", payload).await?;
        Ok(())
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

        if let Some(err_obj) = ack.get("error") {
            let msg = err_obj
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            return Err(ExportError::Transport(format!("{event}: {msg}")));
        }
        Ok(ack)
    }
}
