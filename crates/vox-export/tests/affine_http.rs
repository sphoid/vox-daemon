//! HTTP-surface integration tests for the AFFiNE target.
//!
//! These tests exercise everything up to (and excluding) the Socket.IO / Yjs
//! doc-push step, which is stubbed by AFFiNE and therefore not reachable from
//! a simple mock. The realtime layer is covered by unit tests in the
//! `ydoc` module and will need real-backend QA for final verification.

use serde_json::json;
use vox_core::config::AffineExportConfig;
use vox_export::{ExportError, ExportTarget, affine::AffineTarget};
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn cfg(base_url: &str) -> AffineExportConfig {
    AffineExportConfig {
        enabled: true,
        base_url: base_url.to_owned(),
        email: "user@example.com".to_owned(),
        password: "hunter2".to_owned(),
        ..AffineExportConfig::default()
    }
}

#[tokio::test]
async fn list_workspaces_uses_bearer_token_and_parses_response() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(header("Authorization", "Bearer ut_abc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "workspaces": [
                { "id": "ws-1", "public": false },
                { "id": "ws-2", "public": true }
            ]}
        })))
        .mount(&server)
        .await;

    let target = AffineTarget::from_config(&AffineExportConfig {
        enabled: true,
        base_url: server.uri(),
        api_token: "ut_abc".to_owned(),
        ..AffineExportConfig::default()
    })
    .expect("build target");

    let workspaces = target.list_workspaces().await.expect("list");
    assert_eq!(workspaces.len(), 2);
    assert_eq!(workspaces[0].id, "ws-1");
    assert!(workspaces[0].name.contains("ws-1") || workspaces[0].name.starts_with("Workspace "));
}

#[tokio::test]
async fn list_workspaces_surfaces_graphql_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": null,
            "errors": [{ "message": "forbidden" }]
        })))
        .mount(&server)
        .await;

    let target = AffineTarget::from_config(&AffineExportConfig {
        enabled: true,
        base_url: server.uri(),
        api_token: "t".to_owned(),
        ..AffineExportConfig::default()
    })
    .expect("build");

    let err = target
        .list_workspaces()
        .await
        .expect_err("should propagate graphql error");
    match err {
        ExportError::ApiError { body, .. } => assert_eq!(body, "forbidden"),
        other => panic!("expected ApiError, got {other}"),
    }
}

#[tokio::test]
async fn password_login_exchanges_credentials_for_cookie() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/auth/sign-in"))
        .and(body_json(json!({
            "email": "user@example.com",
            "password": "hunter2"
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("Set-Cookie", "affine_session=sess_42; Path=/; HttpOnly"),
        )
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(header("Cookie", "affine_session=sess_42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "workspaces": [{ "id": "ws-only", "public": false }] }
        })))
        .mount(&server)
        .await;

    let target = AffineTarget::from_config(&cfg(&server.uri())).expect("build");
    let workspaces = target.list_workspaces().await.expect("list");
    assert_eq!(workspaces.len(), 1);
    assert_eq!(workspaces[0].id, "ws-only");
}

// list_folders previously used GraphQL but now loads the workspace root
// Yjs doc via Socket.IO (real titles live there, not in GraphQL). The
// realtime path has no HTTP-mock-equivalent; it's covered by the ydoc
// unit tests (`extract_workspace_pages_*`).

#[tokio::test]
async fn failed_sign_in_maps_to_auth_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/auth/sign-in"))
        .respond_with(ResponseTemplate::new(401).set_body_string("bad creds"))
        .mount(&server)
        .await;

    let target = AffineTarget::from_config(&cfg(&server.uri())).expect("build");
    match target.list_workspaces().await {
        Err(ExportError::Auth(msg)) => assert!(msg.contains("401"), "unexpected: {msg}"),
        other => panic!("expected Auth error, got {other:?}"),
    }
}
