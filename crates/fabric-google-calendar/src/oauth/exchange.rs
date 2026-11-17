//! The token request and its answer (3.4), and the reading of token answers shared with the
//! refresh (3.8) and the revocation (3.9).
//!
//! `POST` to the token endpoint, form encoded: `grant_type=authorization_code`, `code`,
//! `code_verifier`, `client_id`, `client_secret`, `redirect_uri`. One attempt; never repeated,
//! since a code is single use and a request that may have reached Google has used it. Tokens
//! are taken into [`Secret`] at once, and the parsed answer is wiped (`WipedJson`).

use std::fmt;

use serde_json::Value;

use crate::client::ClientConfig;
use crate::http::{Answer, BodyBound, Exchange, Method, Request, Target, Transport};
use crate::secret::Secret;

use super::wipe::WipedJson;

/// The bounds `expires_in` is held to, in seconds (3.4, 3.8).
pub const EXPIRES_IN_BOUNDS: (u64, u64) = (60, 86_400);

/// A token answer as read (3.4, 3.8). Required members are checked by the caller, since the
/// code exchange and the refresh require different ones.
pub struct TokenAnswer {
    /// `access_token`.
    pub access_token: Secret,
    /// `expires_in`, held to [`EXPIRES_IN_BOUNDS`].
    pub expires_in_s: u64,
    /// `token_type`, when present: whether it is `Bearer` without regard to case.
    pub bearer: Option<bool>,
    /// `scope`, when present.
    pub scope: Option<String>,
    /// `id_token`, when present.
    pub id_token: Option<Secret>,
    /// `refresh_token`, when present.
    pub refresh_token: Option<Secret>,
    /// `refresh_token_expires_in`, when present (time-based access, 3.4).
    pub refresh_token_expires_in_s: Option<u64>,
}

/// Prints which members were present, never a value.
impl fmt::Debug for TokenAnswer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenAnswer")
            .field("expires_in_s", &self.expires_in_s)
            .field("bearer", &self.bearer)
            .field("scope", &self.scope.is_some())
            .field("id_token", &self.id_token.is_some())
            .field("refresh_token", &self.refresh_token.is_some())
            .field(
                "refresh_token_expires_in_s",
                &self.refresh_token_expires_in_s,
            )
            .finish()
    }
}

/// Why a token answer is not one. Fixed labels (PD27).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnswerProblem {
    /// Nothing usable came back: nothing sent, sent and unanswered, or a 2xx that is not
    /// JSON (5.2, 6.10).
    NoAnswer,
    /// A readable 2xx JSON answer of the wrong shape: not an object, `access_token` or
    /// `expires_in` missing or of the wrong type, another member of the wrong type.
    Shape,
    /// An error answer (not 2xx), with Google's `error` read against the closed list.
    Error(OAuthError),
}

/// Google's OAuth `error` codes the adapter tells apart (3.4, 3.8, 3.9). Any other is
/// [`OAuthError::Other`]; `error_description` is never read (PD27).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OAuthError {
    /// `invalid_grant` (any `error_subtype`).
    InvalidGrant,
    /// `admin_policy_enforced`.
    AdminPolicyEnforced,
    /// `invalid_client`.
    InvalidClient,
    /// `unauthorized_client`.
    UnauthorizedClient,
    /// `invalid_token` (revocation).
    InvalidToken,
    /// Any other code, or none, or an answer that is not JSON.
    Other,
}

impl OAuthError {
    /// Whether Google refuses the client itself (3.4, 3.8).
    #[must_use]
    pub const fn rejects_client(self) -> bool {
        matches!(
            self,
            OAuthError::InvalidClient | OAuthError::UnauthorizedClient
        )
    }
}

/// Reads the `error` code of an error answer against the closed list.
#[must_use]
pub fn oauth_error(answer: &Answer) -> OAuthError {
    let Ok(value) = answer.json() else {
        return OAuthError::Other;
    };
    let value = WipedJson(value);
    match value.value().get("error").and_then(Value::as_str) {
        Some("invalid_grant") => OAuthError::InvalidGrant,
        Some("admin_policy_enforced") => OAuthError::AdminPolicyEnforced,
        Some("invalid_client") => OAuthError::InvalidClient,
        Some("unauthorized_client") => OAuthError::UnauthorizedClient,
        Some("invalid_token") => OAuthError::InvalidToken,
        _ => OAuthError::Other,
    }
}

/// Reads the result of a token or refresh request.
pub fn read_token_exchange(exchange: Exchange) -> Result<TokenAnswer, AnswerProblem> {
    let answer = match exchange {
        Exchange::Answer(answer) => answer,
        Exchange::NothingSent(_) | Exchange::SentUnanswered(_) => {
            return Err(AnswerProblem::NoAnswer);
        }
    };
    if !answer.is_success() {
        return Err(AnswerProblem::Error(oauth_error(&answer)));
    }
    let value = WipedJson(answer.json().map_err(|_| AnswerProblem::NoAnswer)?);
    read_token_json(value.value())
}

/// Reads the members of a 2xx token answer (3.4, 3.8). Unknown members are ignored.
pub fn read_token_json(value: &Value) -> Result<TokenAnswer, AnswerProblem> {
    let object = value.as_object().ok_or(AnswerProblem::Shape)?;
    let text = |name: &str| -> Result<Option<&str>, AnswerProblem> {
        match object.get(name) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.as_str())),
            Some(_) => Err(AnswerProblem::Shape),
        }
    };
    let number = |name: &str| -> Result<Option<u64>, AnswerProblem> {
        match object.get(name) {
            None => Ok(None),
            Some(v) => v.as_u64().map(Some).ok_or(AnswerProblem::Shape),
        }
    };
    let secret = |name: &str| -> Result<Option<Secret>, AnswerProblem> {
        Ok(text(name)?.filter(|s| !s.is_empty()).map(Secret::from))
    };
    let access_token = secret("access_token")?.ok_or(AnswerProblem::Shape)?;
    let expires_in = number("expires_in")?.ok_or(AnswerProblem::Shape)?;
    let (low, high) = EXPIRES_IN_BOUNDS;
    Ok(TokenAnswer {
        access_token,
        expires_in_s: expires_in.clamp(low, high),
        bearer: text("token_type")?.map(|t| t.eq_ignore_ascii_case("bearer")),
        scope: text("scope")?.map(str::to_owned),
        id_token: secret("id_token")?,
        refresh_token: secret("refresh_token")?,
        refresh_token_expires_in_s: number("refresh_token_expires_in")?,
    })
}

/// The code exchange of 3.4: one attempt with the deadline `deadline_ms`.
pub fn exchange_code(
    transport: &Transport,
    client: &ClientConfig,
    code: &Secret,
    verifier: &Secret,
    redirect_uri: &str,
    deadline_ms: u64,
) -> Result<TokenAnswer, AnswerProblem> {
    let request = Request::new(Method::Post, Target::Token, BodyBound::Small).form(&[
        ("grant_type", "authorization_code"),
        ("code", code.expose()),
        ("code_verifier", verifier.expose()),
        ("client_id", client.client_id()),
        ("client_secret", client.client_secret().expose()),
        ("redirect_uri", redirect_uri),
    ]);
    read_token_exchange(transport.send(request, deadline_ms))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_full_answer_is_read_and_expires_in_is_held_to_its_bounds() {
        let answer = read_token_json(&json!({
            "access_token": "ya29.a", "expires_in": 3599, "token_type": "Bearer",
            "scope": "openid", "id_token": "h.p.s", "refresh_token": "1//r",
            "refresh_token_expires_in": 604_800, "unknown": [1, 2]
        }))
        .expect("an answer");
        assert_eq!(answer.access_token.expose(), "ya29.a");
        assert_eq!(answer.expires_in_s, 3599);
        assert_eq!(answer.bearer, Some(true));
        assert_eq!(answer.scope.as_deref(), Some("openid"));
        assert_eq!(answer.id_token.expect("id").expose(), "h.p.s");
        assert_eq!(answer.refresh_token.expect("refresh").expose(), "1//r");
        assert_eq!(answer.refresh_token_expires_in_s, Some(604_800));

        for (given, held) in [
            (0, 60),
            (59, 60),
            (60, 60),
            (86_400, 86_400),
            (86_401, 86_400),
        ] {
            let a = read_token_json(&json!({"access_token": "t", "expires_in": given}))
                .expect("an answer");
            assert_eq!(a.expires_in_s, held);
            assert!(a.bearer.is_none() && a.refresh_token.is_none() && a.id_token.is_none());
        }
        let lower =
            read_token_json(&json!({"access_token": "t", "expires_in": 1, "token_type": "bearer"}))
                .expect("an answer");
        assert_eq!(lower.bearer, Some(true));
        let mac =
            read_token_json(&json!({"access_token": "t", "expires_in": 1, "token_type": "MAC"}))
                .expect("an answer");
        assert_eq!(mac.bearer, Some(false));
    }

    #[test]
    fn missing_or_mistyped_members_are_a_shape_problem() {
        for bad in [
            json!([]),
            json!({"expires_in": 1}),
            json!({"access_token": "", "expires_in": 1}),
            json!({"access_token": "t"}),
            json!({"access_token": "t", "expires_in": -1}),
            json!({"access_token": "t", "expires_in": 1.5}),
            json!({"access_token": "t", "expires_in": "3600"}),
            json!({"access_token": 7, "expires_in": 1}),
            json!({"access_token": "t", "expires_in": 1, "scope": ["openid"]}),
            json!({"access_token": "t", "expires_in": 1, "refresh_token_expires_in": -5}),
        ] {
            assert_eq!(
                read_token_json(&bad).err(),
                Some(AnswerProblem::Shape),
                "{bad}"
            );
        }
        let empty =
            read_token_json(&json!({"access_token": "t", "expires_in": 1, "refresh_token": ""}))
                .expect("an answer");
        assert!(empty.refresh_token.is_none());
    }

    #[test]
    fn debug_prints_no_token() {
        let answer = read_token_json(&json!({
            "access_token": "ya29.secret", "expires_in": 100, "refresh_token": "1//secret",
            "id_token": "h.secret.s", "scope": "openid"
        }))
        .expect("an answer");
        let text = format!("{answer:?}");
        assert!(!text.contains("secret"), "{text}");
    }
}
