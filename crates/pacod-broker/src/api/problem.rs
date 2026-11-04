//! Errors as `application/problem+json` (RFC 9457).

use axum::Json;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use schemars::JsonSchema;
use serde::Serialize;

use crate::registry::RegistryError;

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Problem {
    /// A stable code: `invalid`, `not_found`, `forbidden`, `identity_unavailable`, …
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// For `invalid`: every problem found, each `path: message`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

impl Problem {
    pub fn new(status: StatusCode, kind: &str, detail: impl Into<String>) -> Self {
        Self {
            kind: kind.to_string(),
            title: status.canonical_reason().unwrap_or("error").to_string(),
            status: status.as_u16(),
            detail: Some(detail.into()),
            errors: Vec::new(),
        }
    }

    pub fn invalid(detail: impl Into<String>, errors: Vec<String>) -> Self {
        Self { errors, ..Self::new(StatusCode::UNPROCESSABLE_ENTITY, "invalid", detail) }
    }

    pub fn bad_request(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", detail)
    }

    pub fn not_found(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", detail)
    }

    pub fn unauthorized(detail: &str) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", detail)
    }

    pub fn forbidden(detail: &str) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", detail)
    }

    pub fn unavailable(kind: &str, detail: &str) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, kind, detail)
    }
}

impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut res = (status, Json(self)).into_response();
        res.headers_mut()
            .insert(http::header::CONTENT_TYPE, http::HeaderValue::from_static("application/problem+json"));
        if status == StatusCode::UNAUTHORIZED {
            res.headers_mut().insert(http::header::WWW_AUTHENTICATE, http::HeaderValue::from_static("Bearer"));
        }
        res
    }
}

impl From<RegistryError> for Problem {
    fn from(e: RegistryError) -> Self {
        match e {
            RegistryError::Invalid(p) => Problem::invalid("the announcement has problems", p.list().to_vec()),
            RegistryError::Forbidden(m) => Problem::new(StatusCode::FORBIDDEN, "forbidden", m),
            RegistryError::NotFound(m) => Problem::not_found(m),
            RegistryError::Conflict(m) => Problem::new(StatusCode::CONFLICT, "conflict", m),
            RegistryError::Limit(m) => Problem::new(StatusCode::TOO_MANY_REQUESTS, "limit", m),
            RegistryError::Pending(p) => Problem::new(
                StatusCode::ACCEPTED,
                "pending_approval",
                format!("provider {p} waits for an admin's approval; announce again later"),
            ),
        }
    }
}

/// A JSON body that answers with a problem, not axum's plain-text rejection, when it does not
/// parse.
pub struct Body<T>(pub T);

impl<S, T> axum::extract::FromRequest<S> for Body<T>
where
    T: serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(v)) => Ok(Body(v)),
            Err(e) => Err(match e {
                axum::extract::rejection::JsonRejection::JsonDataError(e) => {
                    Problem::invalid("the body does not fit the schema", vec![e.body_text()])
                }
                axum::extract::rejection::JsonRejection::MissingJsonContentType(_) => {
                    Problem::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported_media_type", "send application/json")
                }
                other => Problem::bad_request(other.body_text()),
            }),
        }
    }
}
