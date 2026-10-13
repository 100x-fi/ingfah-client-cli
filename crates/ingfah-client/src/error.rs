//! Errors returned by the client.

use std::fmt;

use serde_json::Value;

use crate::routes::{Method, Route};

/// An error response from the API, or a 2xx body whose `status` is `"error"`.
#[derive(Debug, Clone, PartialEq)]
pub struct ApiError {
    /// HTTP status code.
    pub status: u16,
    /// The short, stable `error` phrase, when the body had one.
    pub error: Option<String>,
    /// Human-readable message: `error_detail`, `error`, `message`, or the start of the body.
    pub message: String,
    /// The `errors` list, when present.
    pub errors: Option<Value>,
    /// The `data` payload some errors carry (`data.agents`, `data.invalids`, ...).
    pub data: Option<Value>,
    /// What the caller can do about it.
    pub hint: Option<String>,
}

const BODY_PREVIEW_CHARS: usize = 300;

impl ApiError {
    /// Builds the error from a response. `route` names the scope in a 403 hint.
    pub fn from_response(status: u16, body: &[u8], route: &Route) -> Self {
        let json: Option<Value> = serde_json::from_slice(body).ok();
        let field = |name: &str| {
            json.as_ref()
                .and_then(|v| v.get(name))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        let error = field("error");
        let message = field("error_detail")
            .or_else(|| error.clone())
            .or_else(|| field("message"))
            .unwrap_or_else(|| {
                String::from_utf8_lossy(body)
                    .chars()
                    .take(BODY_PREVIEW_CHARS)
                    .collect()
            });
        let take = |name: &str| {
            json.as_ref()
                .and_then(|v| v.get(name))
                .filter(|v| !v.is_null())
                .cloned()
        };
        let hint = hint_for(status, &message, route);
        Self {
            status,
            error,
            message,
            errors: take("errors"),
            data: take("data"),
            hint,
        }
    }
}

fn hint_for(status: u16, message: &str, route: &Route) -> Option<String> {
    match status {
        401 => Some(
            "API key invalid or revoked; run `ingfah auth login`, or the route may be JWT-only"
                .to_owned(),
        ),
        403 if message.contains("insufficient api key scopes") => Some(format!(
            "this key lacks the `{}` scope that {} {} needs; create a key with that scope in the dashboard",
            route.scope, route.method, route.path
        )),
        403 if route.path.ends_with("/visibility") => Some(
            "only the admin who created this resource may change its visibility; use that admin's key"
                .to_owned(),
        ),
        _ => None,
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HTTP {}", self.status)?;
        match &self.error {
            Some(e) if *e != self.message => write!(f, " {e}: {}", self.message),
            _ => write!(f, ": {}", self.message),
        }
    }
}

/// Everything the client can fail with.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The server answered with an error.
    #[error("{0}")]
    Api(Box<ApiError>),
    /// A guard refused the request before it was sent.
    #[error("{0}")]
    Refused(String),
    /// The method and path are not in the route registry.
    #[error(
        "{method} {path} is not an allowed API-key route; JWT-only and unknown routes are refused"
    )]
    UnknownRoute {
        /// Requested method.
        method: Method,
        /// Requested path.
        path: String,
    },
    /// The base URL is unusable.
    #[error("invalid base URL `{0}`: use https, or http only for localhost")]
    BaseUrl(String),
    /// Network or protocol failure.
    #[error("request failed: {0}")]
    Http(#[from] reqwest::Error),
}

impl Error {
    /// True when a guard refused the request (CLI exit code 3).
    pub fn is_refusal(&self) -> bool {
        matches!(self, Self::Refused(_) | Self::UnknownRoute { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::resolve;

    fn route(method: Method, path: &str) -> &'static Route {
        resolve(method, path).unwrap_or_else(|| panic!("no route for {path}"))
    }

    #[test]
    fn message_prefers_error_detail() {
        let body = br#"{"status":"error","error":"bad request","error_detail":"name is required"}"#;
        let err = ApiError::from_response(400, body, route(Method::Get, "/client/voices"));
        assert_eq!(err.message, "name is required");
    }

    #[test]
    fn message_falls_back_to_error_then_message() {
        let r = route(Method::Get, "/client/voices");
        assert_eq!(
            ApiError::from_response(404, br#"{"error":"resource not found"}"#, r).message,
            "resource not found"
        );
        assert_eq!(
            ApiError::from_response(500, br#"{"message":"boom"}"#, r).message,
            "boom"
        );
    }

    #[test]
    fn message_falls_back_to_first_300_chars_of_body() {
        let body = "x".repeat(400);
        let err =
            ApiError::from_response(502, body.as_bytes(), route(Method::Get, "/client/voices"));
        assert_eq!(err.message.len(), 300);
    }

    #[test]
    fn scope_hint_names_the_route_scope() {
        let body = br#"{"status":"error","error":"forbidden","error_detail":"insufficient api key scopes"}"#;
        let err = ApiError::from_response(
            403,
            body,
            route(Method::Post, "/client/outbound/batches/7/pause"),
        );
        assert!(
            err.hint
                .as_deref()
                .is_some_and(|h| h.contains("client_outbound_batches:write"))
        );
    }

    #[test]
    fn visibility_403_hint_explains_creator_rule() {
        let body = br#"{"status":"error","error":"forbidden","error_detail":"permission denied"}"#;
        let err = ApiError::from_response(
            403,
            body,
            route(Method::Put, "/client/agents/mali/visibility"),
        );
        assert!(err.hint.as_deref().is_some_and(|h| h.contains("created")));
    }

    #[test]
    fn error_keeps_data_payload() {
        let body = br#"{"status":"error","error":"conflict","error_detail":"in use","data":{"agents":[{"slug":"mali"}]}}"#;
        let err = ApiError::from_response(
            409,
            body,
            route(Method::Delete, "/client/plugin-functions/3"),
        );
        assert_eq!(
            err.data,
            Some(serde_json::json!({"agents":[{"slug":"mali"}]}))
        );
    }
}
