//! HTTP conventions shared by the demo's services: refusals as `{"error", "detail"}`, token
//! presentations in `Authorization: FabricToken <hex>`, and per-run bearer secrets.

use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use fabric_token::{Presentation, Reason, Rejection, hex, sha256};
use serde_json::json;

/// The `Authorization` scheme that carries a token presentation.
pub const TOKEN_SCHEME: &str = "FabricToken";
/// The shortest per-run secret accepted, in characters.
pub const MIN_SECRET: usize = 32;

/// A refused request: an HTTP status and `{"error", "detail"}`.
#[derive(Debug)]
pub struct Failure {
    pub status: StatusCode,
    pub error: &'static str,
    pub detail: String,
}

impl Failure {
    pub fn new(status: StatusCode, error: &'static str, detail: impl Into<String>) -> Self {
        Self {
            status,
            error,
            detail: detail.into(),
        }
    }

    pub fn bad_request(error: &'static str, detail: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, error, detail)
    }

    pub fn conflict(error: &'static str, detail: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, error, detail)
    }

    pub fn internal(error: &'static str, detail: impl std::fmt::Display) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, error, detail.to_string())
    }
}

impl From<Rejection> for Failure {
    /// 401 when the token cannot be trusted at all, 403 when it is genuine but does not allow
    /// this request.
    fn from(rejection: Rejection) -> Self {
        let status = match rejection.reason {
            Reason::BadSignature | Reason::Expired | Reason::PopInvalid => StatusCode::UNAUTHORIZED,
            Reason::WrongAudience | Reason::TokenReplayed | Reason::TokenArgumentMismatch => {
                StatusCode::FORBIDDEN
            }
        };
        Self::new(status, rejection.reason.name(), rejection.detail)
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let body = json!({ "error": self.error, "detail": self.detail });
        (self.status, Json(body)).into_response()
    }
}

/// Reads `Authorization: FabricToken <hex>`. A missing or unreadable token is a
/// `bad_signature` (section 9).
pub fn presentation(headers: &HeaderMap) -> Result<Presentation, Failure> {
    let unreadable = |detail: &str| {
        Failure::new(
            StatusCode::UNAUTHORIZED,
            Reason::BadSignature.name(),
            detail.to_owned(),
        )
    };
    let value = headers
        .get(AUTHORIZATION)
        .ok_or_else(|| unreadable("the request carries no token"))?
        .to_str()
        .map_err(|_| unreadable("the Authorization header is not ASCII"))?;
    let encoded = value
        .strip_prefix(TOKEN_SCHEME)
        .and_then(|rest| rest.strip_prefix(' '))
        .ok_or_else(|| unreadable("the Authorization scheme is not FabricToken"))?;
    let bytes =
        hex::decode(encoded.trim()).map_err(|_| unreadable("the token is not lowercase hex"))?;
    Presentation::from_bytes(&bytes).map_err(|error| unreadable(&error.to_string()))
}

/// The `Authorization` header value for a presentation.
pub fn authorization(presentation: &Presentation) -> Result<String, fabric_token::TokenError> {
    Ok(format!(
        "{TOKEN_SCHEME} {}",
        hex::encode(&presentation.to_bytes()?)
    ))
}

/// A per-run bearer secret, kept only as its digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BearerSecret([u8; 32]);

impl BearerSecret {
    #[must_use]
    pub fn new(secret: &str) -> Self {
        Self(sha256(secret.as_bytes()))
    }

    /// Checks `Authorization: Bearer <secret>`. Comparing digests keeps the comparison time
    /// independent of the secret.
    pub fn check(&self, headers: &HeaderMap, error: &'static str) -> Result<(), Failure> {
        let given = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .unwrap_or_default();
        if sha256(given.as_bytes()) == self.0 {
            Ok(())
        } else {
            Err(Failure::new(
                StatusCode::UNAUTHORIZED,
                error,
                "this API needs the run's secret",
            ))
        }
    }
}

/// Checks a per-run secret's length.
pub fn check_secret(name: &str, secret: &str) -> Result<(), String> {
    if secret.len() < MIN_SECRET {
        Err(format!("the {name} needs at least {MIN_SECRET} characters"))
    } else {
        Ok(())
    }
}
