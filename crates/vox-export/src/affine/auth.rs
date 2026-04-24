//! `AFFiNE` authentication strategies: bearer token (cloud + self-hosted) or
//! email/password exchanged for a session cookie (self-hosted only).

use std::sync::Arc;

use reqwest::Client;
use serde::Serialize;
use tokio::sync::Mutex;
use tracing::debug;

use crate::error::ExportError;

/// HTTP / Socket.IO headers produced by [`AuthState::ensure_headers`].
///
/// Exactly one of `bearer` / `cookie` is populated at a time.
#[derive(Debug, Clone, Default)]
pub struct AuthHeaders {
    /// Bearer token for `Authorization: Bearer …`.
    pub bearer: Option<String>,
    /// Session cookie for `Cookie: …`.
    pub cookie: Option<String>,
}

impl AuthHeaders {
    /// Apply these headers to an outgoing [`reqwest::RequestBuilder`].
    pub fn apply(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(b) = &self.bearer {
            req = req.header("Authorization", format!("Bearer {b}"));
        }
        if let Some(c) = &self.cookie {
            req = req.header("Cookie", c);
        }
        req
    }
}

#[derive(Debug, Clone)]
enum AuthMode {
    Token(String),
    Password { email: String, password: String },
}

/// Cached, shared authentication state.
///
/// Cloning an [`AuthState`] is cheap and produces a clone that shares the
/// underlying session cache, so multiple concurrent exports reuse a single
/// login.
#[derive(Debug, Clone)]
pub struct AuthState {
    mode: AuthMode,
    /// Cached session cookie for password-based auth. Populated on the first
    /// [`AuthState::ensure_headers`] call.
    cookie: Arc<Mutex<Option<String>>>,
}

impl AuthState {
    /// Token-based auth — works against cloud and self-hosted.
    #[must_use]
    pub fn token(token: String) -> Self {
        Self {
            mode: AuthMode::Token(token),
            cookie: Arc::new(Mutex::new(None)),
        }
    }

    /// Email + password auth — self-hosted only.
    #[must_use]
    pub fn password(email: String, password: String) -> Self {
        Self {
            mode: AuthMode::Password { email, password },
            cookie: Arc::new(Mutex::new(None)),
        }
    }

    /// Return auth headers usable against any endpoint under `base_url`.
    ///
    /// For token auth this is trivial; for password auth this performs a
    /// sign-in round-trip on first use and caches the resulting cookie.
    ///
    /// # Errors
    ///
    /// - [`ExportError::Auth`] if the sign-in request does not return a
    ///   `Set-Cookie` header.
    /// - [`ExportError::Http`] on network failure.
    /// - [`ExportError::ApiError`] when the server returns a non-2xx status.
    pub async fn ensure_headers(
        &self,
        http: &Client,
        base_url: &str,
    ) -> Result<AuthHeaders, ExportError> {
        match &self.mode {
            AuthMode::Token(t) => Ok(AuthHeaders {
                bearer: Some(t.clone()),
                cookie: None,
            }),
            AuthMode::Password { email, password } => {
                let mut guard = self.cookie.lock().await;
                if let Some(c) = &*guard {
                    return Ok(AuthHeaders {
                        bearer: None,
                        cookie: Some(c.clone()),
                    });
                }

                let cookie = sign_in_password(http, base_url, email, password).await?;
                *guard = Some(cookie.clone());
                Ok(AuthHeaders {
                    bearer: None,
                    cookie: Some(cookie),
                })
            }
        }
    }

    /// Force a re-login on next use — call after a 401 response.
    pub async fn invalidate(&self) {
        *self.cookie.lock().await = None;
    }
}

#[derive(Serialize)]
struct SignInBody<'a> {
    email: &'a str,
    password: &'a str,
}

/// `POST {base}/api/auth/sign-in` with JSON body; returns the distilled
/// `Cookie` header string suitable for subsequent requests.
async fn sign_in_password(
    http: &Client,
    base_url: &str,
    email: &str,
    password: &str,
) -> Result<String, ExportError> {
    let url = format!("{base_url}/api/auth/sign-in");
    debug!(%url, "POST sign-in");
    let res = http
        .post(&url)
        .json(&SignInBody { email, password })
        .send()
        .await?;
    let status = res.status();
    if !status.is_success() {
        let body = res.text().await.unwrap_or_default();
        return Err(ExportError::Auth(format!(
            "sign-in returned {}: {}",
            status.as_u16(),
            sanitize(&body),
        )));
    }

    let cookies: Vec<String> = res
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .map(str::to_owned)
        .collect();
    if cookies.is_empty() {
        return Err(ExportError::Auth(
            "sign-in returned no Set-Cookie headers".to_owned(),
        ));
    }

    let pairs: Vec<String> = cookies
        .iter()
        .filter_map(|sc| sc.split(';').next())
        .map(|p| p.trim().to_owned())
        .filter(|p| !p.is_empty())
        .collect();
    let header = pairs.join("; ");
    if header.contains(['\r', '\n']) {
        return Err(ExportError::Auth(
            "sign-in cookie contained illegal CR/LF characters".to_owned(),
        ));
    }
    Ok(header)
}

/// Trim HTML and excess whitespace from an API error body so it is safe to
/// surface in logs / error messages.
fn sanitize(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut in_tag = false;
    for ch in body.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    let collapsed: String = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.len() > 200 {
        format!("{}…", &collapsed[..200])
    } else {
        collapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn token_headers_are_bearer() {
        let state = AuthState::token("ut_xyz".to_owned());
        let http = Client::new();
        let h = state
            .ensure_headers(&http, "https://example.com")
            .await
            .expect("headers");
        assert_eq!(h.bearer.as_deref(), Some("ut_xyz"));
        assert!(h.cookie.is_none());
    }

    #[test]
    fn sanitize_strips_tags_and_collapses_ws() {
        let s = sanitize("<html>  something <b>went</b>\nwrong </html>");
        assert_eq!(s, "something went wrong");
    }

    #[test]
    fn sanitize_truncates_long_bodies() {
        let long = "x".repeat(500);
        let s = sanitize(&long);
        assert!(s.ends_with('…'));
        assert!(s.len() < long.len());
    }

    #[test]
    fn auth_headers_apply_sets_bearer() {
        let h = AuthHeaders {
            bearer: Some("tok".to_owned()),
            cookie: None,
        };
        let http = Client::new();
        let req = h.apply(http.get("https://example.com"));
        let built = req.build().expect("build");
        let auth = built.headers().get("Authorization").expect("header");
        assert_eq!(auth.to_str().unwrap(), "Bearer tok");
    }

    #[test]
    fn auth_headers_apply_sets_cookie() {
        let h = AuthHeaders {
            bearer: None,
            cookie: Some("sid=abc".to_owned()),
        };
        let http = Client::new();
        let req = h.apply(http.get("https://example.com"));
        let built = req.build().expect("build");
        assert_eq!(
            built.headers().get("Cookie").unwrap().to_str().unwrap(),
            "sid=abc"
        );
    }
}
