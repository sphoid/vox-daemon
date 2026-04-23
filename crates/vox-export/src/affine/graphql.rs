//! GraphQL client for `AFFiNE` metadata: listing workspaces and docs.
//!
//! Only the two queries we need are modelled here; the rest of the `AFFiNE`
//! schema is ignored. Full schema reference:
//! <https://github.com/toeverything/AFFiNE/blob/canary/packages/backend/server/src/schema.gql>

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::debug;

use super::auth::AuthHeaders;
use crate::{
    error::ExportError,
    traits::{Folder, Workspace},
};

const WORKSPACES_QUERY: &str = r"
query Workspaces {
  workspaces {
    id
    public
  }
}";

const DOCS_QUERY: &str = r"
query WorkspaceDocs($id: String!, $first: Int!, $offset: Int!) {
  workspace(id: $id) {
    id
    docs(pagination: { first: $first, offset: $offset }) {
      edges {
        node {
          id
          title
        }
      }
    }
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

/// Fetch all workspaces visible to the authenticated user.
///
/// # Errors
///
/// Returns [`ExportError`] on transport failure or when the GraphQL server
/// returns an error.
pub async fn list_workspaces(
    http: &Client,
    base_url: &str,
    headers: &AuthHeaders,
) -> Result<Vec<Workspace>, ExportError> {
    let data: WorkspacesData =
        send::<(), _>(http, base_url, headers, WORKSPACES_QUERY, None).await?;

    // `AFFiNE`'s `workspaces` query returns only ids for security; the display
    // name lives inside the workspace's own Yjs doc and is not exposed via
    // GraphQL. Use the short id as the display name so the picker is still
    // usable.
    Ok(data
        .workspaces
        .into_iter()
        .map(|w| {
            let short = w.id.chars().take(8).collect::<String>();
            Workspace {
                name: format!("Workspace {short}"),
                id: w.id,
            }
        })
        .collect())
}

#[derive(Deserialize)]
struct DocsData {
    workspace: DocsWorkspace,
}

#[derive(Deserialize)]
struct DocsWorkspace {
    docs: DocsConnection,
}

#[derive(Deserialize)]
struct DocsConnection {
    edges: Vec<DocsEdge>,
}

#[derive(Deserialize)]
struct DocsEdge {
    node: DocNode,
}

#[derive(Deserialize)]
struct DocNode {
    id: String,
    #[serde(default)]
    title: Option<String>,
}

#[derive(Serialize)]
struct DocsVariables<'a> {
    id: &'a str,
    first: i32,
    offset: i32,
}

/// Fetch the first page of docs in `workspace_id`. Returns up to 200 docs —
/// enough for the Send-to picker; full pagination is not needed.
///
/// # Errors
///
/// Returns [`ExportError`] on transport or API failure.
pub async fn list_docs(
    http: &Client,
    base_url: &str,
    headers: &AuthHeaders,
    workspace_id: &str,
) -> Result<Vec<Folder>, ExportError> {
    let vars = DocsVariables {
        id: workspace_id,
        first: 200,
        offset: 0,
    };
    let data: DocsData = send(http, base_url, headers, DOCS_QUERY, Some(vars)).await?;
    Ok(data
        .workspace
        .docs
        .edges
        .into_iter()
        .map(|e| Folder {
            id: e.node.id.clone(),
            title: e.node.title.unwrap_or_else(|| "Untitled".to_owned()),
        })
        .collect())
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

    #[test]
    fn gql_response_parses_docs() {
        let body = r#"{
          "data": {
            "workspace": {
              "docs": {
                "edges": [
                  { "node": { "id": "d1", "title": "Notes" } },
                  { "node": { "id": "d2", "title": null } }
                ]
              }
            }
          }
        }"#;
        let parsed: GqlResponse<DocsData> = serde_json::from_str(body).expect("parse");
        let data = parsed.data.expect("data");
        assert_eq!(data.workspace.docs.edges.len(), 2);
        assert_eq!(data.workspace.docs.edges[0].node.id, "d1");
        assert_eq!(
            data.workspace.docs.edges[0].node.title.as_deref(),
            Some("Notes")
        );
        assert!(data.workspace.docs.edges[1].node.title.is_none());
    }
}
