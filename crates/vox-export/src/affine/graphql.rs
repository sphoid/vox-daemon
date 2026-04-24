//! GraphQL client for `AFFiNE` metadata: listing workspaces and docs.
//!
//! Only the two queries we need are modelled here; the rest of the `AFFiNE`
//! schema is ignored. Full schema reference:
//! <https://github.com/toeverything/AFFiNE/blob/canary/packages/backend/server/src/schema.gql>

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::debug;

use super::auth::AuthHeaders;
use crate::{error::ExportError, traits::Workspace};

const WORKSPACES_QUERY: &str = r"
query Workspaces {
  workspaces {
    id
    public
  }
}";

/// GraphQL request envelope.
#[derive(Serialize)]
struct GqlRequest<'a, V: Serialize> {
    query: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    variables: Option<V>,
}

/// GraphQL response envelope — we parse `data` into a type-specific struct
/// and surface `errors[0].message` as an [`ExportError::ApiError`].
#[derive(Deserialize)]
struct GqlResponse<D> {
    data: Option<D>,
    #[serde(default = "Vec::new")]
    errors: Vec<GqlError>,
}

#[derive(Deserialize)]
struct GqlError {
    message: String,
}

/// Send a GraphQL query and decode its `data` field.
async fn send<V, D>(
    http: &Client,
    base_url: &str,
    headers: &AuthHeaders,
    query: &str,
    variables: Option<V>,
) -> Result<D, ExportError>
where
    V: Serialize,
    D: for<'de> Deserialize<'de>,
{
    let url = format!("{base_url}/graphql");
    debug!(%url, "POST graphql");

    let req = http.post(&url).json(&GqlRequest { query, variables });
    let res = headers.apply(req).send().await?;
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(ExportError::ApiError {
            status: status.as_u16(),
            body,
        });
    }

    let parsed: GqlResponse<D> =
        serde_json::from_str(&body).map_err(|e| ExportError::ParseError {
            reason: format!("graphql response body did not match expected shape: {e}"),
        })?;

    if let Some(first) = parsed.errors.into_iter().next() {
        return Err(ExportError::ApiError {
            status: 200,
            body: first.message,
        });
    }

    parsed.data.ok_or(ExportError::ParseError {
        reason: "graphql response had no `data` field".to_owned(),
    })
}

#[derive(Deserialize)]
struct WorkspacesData {
    workspaces: Vec<WorkspaceNode>,
}

#[derive(Deserialize)]
struct WorkspaceNode {
    id: String,
}

/// Fetch the ids of all workspaces visible to the authenticated user.
///
/// The `AFFiNE` GraphQL schema intentionally does not expose workspace
/// names — they live in each workspace's root Yjs doc. Callers that want
/// human-readable names should follow up with a realtime fetch (see
/// [`super::AffineTarget::list_workspaces`]).
///
/// # Errors
///
/// Returns [`ExportError`] on transport failure or when the GraphQL server
/// returns an error.
pub async fn list_workspace_ids(
    http: &Client,
    base_url: &str,
    headers: &AuthHeaders,
) -> Result<Vec<String>, ExportError> {
    let data: WorkspacesData =
        send::<(), _>(http, base_url, headers, WORKSPACES_QUERY, None).await?;
    Ok(data.workspaces.into_iter().map(|w| w.id).collect())
}

/// Short placeholder name for a workspace whose real name could not be
/// resolved — e.g. the realtime fetch timed out or failed.
#[must_use]
pub fn fallback_workspace_name(id: &str) -> String {
    let short: String = id.chars().take(8).collect();
    format!("Workspace {short}")
}

/// Build a [`Workspace`] with a placeholder name derived from its id.
#[must_use]
pub fn workspace_with_fallback_name(id: String) -> Workspace {
    Workspace {
        name: fallback_workspace_name(&id),
        id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gql_response_parses_errors() {
        let body = r#"{"data":null,"errors":[{"message":"forbidden"}]}"#;
        let parsed: GqlResponse<WorkspacesData> = serde_json::from_str(body).expect("parse");
        assert!(parsed.data.is_none());
        assert_eq!(parsed.errors.len(), 1);
        assert_eq!(parsed.errors[0].message, "forbidden");
    }

    #[test]
    fn gql_response_parses_workspaces() {
        let body = r#"{"data":{"workspaces":[{"id":"abc","public":false}]}}"#;
        let parsed: GqlResponse<WorkspacesData> = serde_json::from_str(body).expect("parse");
        let ws = parsed.data.expect("data");
        assert_eq!(ws.workspaces.len(), 1);
        assert_eq!(ws.workspaces[0].id, "abc");
    }
}
