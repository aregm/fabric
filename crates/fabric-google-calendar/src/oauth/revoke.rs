//! Revocation at disconnect (3.9, FA14).
//!
//! `POST` to the revocation endpoint, form encoded, `token=<refresh token>`. One attempt.
//! 200 is [`Revocation::Revoked`]; 400 with `error` `invalid_token` is `Revoked` too (Google no
//! longer knows the token; LV5); anything else is [`Revocation::NotConfirmed`]. It never
//! deletes the stored token: the caller does, in every case (3.9, CO4).

use fabric_desktop_contracts::scalar::Id;

use crate::http::{BodyBound, Exchange, Method, Request, Target, Transport};
use crate::tokens::TokenStore;

use super::exchange::{OAuthError, oauth_error};

/// What a revocation came to (3.9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Revocation {
    /// Google confirmed it, or no longer knows the token.
    Revoked,
    /// Anything else: an error, no answer, no stored token, or a token store that failed.
    /// The owner is told to remove the access in the Google account (V3-3).
    NotConfirmed,
}

/// Revokes the account's stored refresh token: one attempt with the deadline `deadline_ms`
/// (cut to 30 s by the transport). Does not delete it.
pub fn revoke(
    transport: &Transport,
    store: &dyn TokenStore,
    account: &Id,
    deadline_ms: u64,
) -> Revocation {
    let Ok(Some(token)) = store.read(account) else {
        return Revocation::NotConfirmed;
    };
    let Ok(request) = Request::new(Method::Post, Target::Revocation, BodyBound::Small)
        .form(&[("token", token.expose())])
    else {
        return Revocation::NotConfirmed;
    };
    match transport.send(request, deadline_ms) {
        Exchange::Answer(answer) if answer.status() == 200 => Revocation::Revoked,
        Exchange::Answer(answer)
            if answer.status() == 400 && oauth_error(&answer) == OAuthError::InvalidToken =>
        {
            Revocation::Revoked
        }
        _ => Revocation::NotConfirmed,
    }
}
