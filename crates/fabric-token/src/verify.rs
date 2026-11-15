//! Token checks at the services that execute actions (section 9).

use std::fmt;

use ed25519_dalek::VerifyingKey;
use fabric_policy::Action;
use fabric_policy::time::format_utc;
use serde::{Deserialize, Serialize};

use crate::digest::action_digest;
use crate::hex;
use crate::pop::{self, Presentation};
use crate::replay::{Refusal, ReplayCache, Reservation};
use crate::token::{Claims, Service, issuer, open};

/// Why a service refused a token. [`Verifier::check`] runs its checks in this order.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// Not a well-formed token signed by this person's policy gate.
    BadSignature,
    /// Minted for another service.
    WrongAudience,
    Expired,
    /// Already used, or in use by another request.
    TokenReplayed,
    /// The proof of possession is stale, or not made with the token's `cnf` key over this
    /// request.
    PopInvalid,
    /// Minted for another action, or for other arguments.
    TokenArgumentMismatch,
}

impl Reason {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::BadSignature => "bad_signature",
            Self::WrongAudience => "wrong_audience",
            Self::Expired => "expired",
            Self::TokenReplayed => "token_replayed",
            Self::PopInvalid => "pop_invalid",
            Self::TokenArgumentMismatch => "token_argument_mismatch",
        }
    }
}

/// A refused token: the reason, and the failed condition in words.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Rejection {
    pub reason: Reason,
    pub detail: String,
}

impl fmt::Display for Rejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.reason.name(), self.detail)
    }
}

impl std::error::Error for Rejection {}

fn rejection(reason: Reason, detail: impl Into<String>) -> Rejection {
    Rejection {
        reason,
        detail: detail.into(),
    }
}

/// Checks tokens for one service of one person.
#[derive(Clone, Debug)]
pub struct Verifier {
    issuer: String,
    audience: String,
    gate_key: VerifyingKey,
}

impl Verifier {
    /// Accepts tokens for `service` of `person`, signed with that person's gate key.
    #[must_use]
    pub fn new(service: Service, person: &str, gate_key: VerifyingKey) -> Self {
        Self {
            issuer: issuer(person),
            audience: service.audience(person),
            gate_key,
        }
    }

    #[must_use]
    pub fn audience(&self) -> &str {
        &self.audience
    }

    /// Checks `presentation` for a request that asks for `action` and whose body is `body`.
    /// On success the token's nonce is reserved: call [`Accepted::commit`] once the action has
    /// executed, or drop the [`Accepted`] to release the nonce if it could not execute.
    pub fn check(
        &self,
        presentation: &Presentation,
        action: &Action,
        body: &[u8],
        now_ms: i64,
        replay: &ReplayCache,
    ) -> Result<Accepted, Rejection> {
        let claims = open(&presentation.token, &self.gate_key)
            .map_err(|detail| rejection(Reason::BadSignature, detail))?;
        if claims.issuer != self.issuer {
            return Err(rejection(
                Reason::BadSignature,
                format!("issued by {}, not {}", claims.issuer, self.issuer),
            ));
        }
        if claims.audience != self.audience {
            return Err(rejection(
                Reason::WrongAudience,
                format!(
                    "the token is for {}, and this is {}",
                    claims.audience, self.audience
                ),
            ));
        }
        let expires_ms = claims.expires_at.saturating_mul(1_000);
        let expired = || {
            rejection(
                Reason::Expired,
                format!("the token expired at {}", format_utc(expires_ms)),
            )
        };
        if now_ms >= expires_ms {
            return Err(expired());
        }
        if replay.contains(&claims.nonce) {
            return Err(rejection(
                Reason::TokenReplayed,
                "this token was already used, or another request is using it",
            ));
        }
        pop::verify(&claims, presentation, body, now_ms)
            .map_err(|detail| rejection(Reason::PopInvalid, detail))?;
        check_arguments(&claims, action)
            .map_err(|detail| rejection(Reason::TokenArgumentMismatch, detail))?;
        let reservation = replay
            .reserve(claims.nonce, expires_ms, now_ms)
            .map_err(|refusal| match refusal {
                Refusal::Expired => expired(),
                Refusal::Replayed => {
                    rejection(Reason::TokenReplayed, "another request is using this token")
                }
            })?;
        Ok(Accepted {
            claims,
            reservation,
        })
    }
}

fn check_arguments(claims: &Claims, action: &Action) -> Result<(), String> {
    if claims.action != action.kind() {
        return Err(format!(
            "the token is for {}, and the request is {}",
            claims.action,
            action.kind()
        ));
    }
    if claims.negotiation.as_deref() != action.negotiation() {
        return Err(format!(
            "the token is for negotiation {}, and the request is for {}",
            claims.negotiation.as_deref().unwrap_or("none"),
            action.negotiation().unwrap_or("none")
        ));
    }
    let digest = action_digest(action).map_err(|error| error.to_string())?;
    if digest != claims.args {
        return Err(format!(
            "the request's arguments differ from the token's (digest {}, token {})",
            hex::encode(&digest),
            hex::encode(&claims.args)
        ));
    }
    Ok(())
}

/// A token a service accepted. Its nonce stays reserved while this value lives.
#[derive(Debug)]
pub struct Accepted {
    pub claims: Claims,
    reservation: Reservation,
}

impl Accepted {
    /// Marks the token used, once its action has executed (section 5.1).
    pub fn commit(self) {
        self.reservation.commit();
    }
}
