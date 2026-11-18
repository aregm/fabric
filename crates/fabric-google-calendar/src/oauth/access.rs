//! The access tokens of Calendar calls, refreshed when needed (3.8, section 4).
//!
//! [`GoogleAccess`] implements [`AccessTokens`]: the cached access token when it has at least
//! [`MARGIN_MS`] left; otherwise one refresh inside the call's deadline (`POST` to the token
//! endpoint with `grant_type=refresh_token`, `refresh_token`, `client_id`, `client_secret`).
//!
//! | Answer | Result |
//! |---|---|
//! | 2xx with `access_token` and `expires_in` (held to 60..86 400 s) | the token; a new `refresh_token` replaces the stored one; a `scope` member other than the five is `AuthLost(GrantEnded)` |
//! | `invalid_grant` (any subtype), `admin_policy_enforced` | `AuthLost(GrantEnded)`, the account forgotten |
//! | `invalid_client`, `unauthorized_client` | `AuthLost(ClientRejected)`, the account forgotten |
//! | anything else, or no answer | `Transient` |
//!
//! Token-store failures (section 4): `Locked` is `Transient`; `Refused` and `Failed` are
//! `Store(..)`. No stored refresh token is `AuthLost(GrantEnded)`. A failed write of a rotated
//! refresh token is not the call's failure (the new access token is kept). In every case
//! nothing was sent to the Calendar host.
//!
//! A refresh reads the account's generation in the [`AccessCache`] before it reads the
//! refresh token. When [`crate::tokens::forget`] raised it while the refresh was under way,
//! the refresh keeps nothing (no access token, no rotated refresh token) and the call ends
//! `AuthLost(GrantEnded)` (section 4).

use std::fmt;
use std::sync::Arc;

use fabric_desktop_contracts::scalar::Id;

use crate::client::ClientConfig;
use crate::clock::SharedClock;
use crate::http::{BodyBound, Method, Request, Target, Transport};
use crate::secret::Secret;
use crate::tokens::{
    AccessCache, AccessFailure, AccessTokens, AuthLostCause, TokenStore, TokenStoreError, forget,
};

use super::exchange::{self, AnswerProblem, OAuthError};
use super::scopes::{self, Grant};

/// An access token is used from the cache only when it has at least this long left (3.8).
pub const MARGIN_MS: u64 = 60_000;

/// The access tokens of every account, over one token store and one cache (3.8).
pub struct GoogleAccess<S: TokenStore> {
    transport: Arc<Transport>,
    client: Arc<ClientConfig>,
    store: S,
    cache: AccessCache,
    clock: SharedClock,
}

impl<S: TokenStore> GoogleAccess<S> {
    /// Access tokens refreshed through `transport` with `client`'s id and secret, kept in
    /// `cache`, with refresh tokens from `store`.
    #[must_use]
    pub fn new(
        transport: Arc<Transport>,
        client: Arc<ClientConfig>,
        store: S,
        cache: AccessCache,
        clock: SharedClock,
    ) -> GoogleAccess<S> {
        GoogleAccess {
            transport,
            client,
            store,
            cache,
            clock,
        }
    }

    /// The token store.
    pub fn store(&self) -> &S {
        &self.store
    }

    /// The cache.
    pub fn cache(&self) -> &AccessCache {
        &self.cache
    }

    fn lost(&mut self, account: &Id, generation: u64, cause: AuthLostCause) -> AccessFailure {
        // The cached token is dropped whatever the store answers (section 4). An account
        // forgotten while the refresh was under way is not forgotten again: a sign-in may
        // have stored a new token for it since.
        if self.cache.generation(account) == generation {
            let _ = forget(&mut self.store, &self.cache, account);
        }
        AccessFailure::AuthLost(cause)
    }

    fn store_failure(error: TokenStoreError) -> AccessFailure {
        match error {
            TokenStoreError::Locked => AccessFailure::Transient,
            other => AccessFailure::Store(other),
        }
    }
}

impl<S: TokenStore> AccessTokens for GoogleAccess<S> {
    fn access_token(&mut self, account: &Id, deadline_ms: u64) -> Result<Secret, AccessFailure> {
        if let Some(token) = self
            .cache
            .get_valid(account, self.clock.now_ms(), MARGIN_MS)
        {
            return Ok(token);
        }
        // Read before the refresh token: a forget after this point makes the refresh store
        // nothing (section 4).
        let generation = self.cache.generation(account);
        let refresh_token = match self.store.read(account) {
            Ok(Some(token)) => token,
            Ok(None) => {
                self.cache.drop_account(account);
                return Err(AccessFailure::AuthLost(AuthLostCause::GrantEnded));
            }
            Err(error) => return Err(Self::store_failure(error)),
        };
        let request = Request::new(Method::Post, Target::Token, BodyBound::Small)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token.expose()),
                ("client_id", self.client.client_id()),
                ("client_secret", self.client.client_secret().expose()),
            ])
            .map_err(|_| AccessFailure::Transient)?;
        let answer = match exchange::read_token_exchange(self.transport.send(request, deadline_ms))
        {
            Ok(answer) => answer,
            Err(AnswerProblem::Error(
                OAuthError::InvalidGrant | OAuthError::AdminPolicyEnforced,
            )) => return Err(self.lost(account, generation, AuthLostCause::GrantEnded)),
            Err(AnswerProblem::Error(
                OAuthError::InvalidClient | OAuthError::UnauthorizedClient,
            )) => {
                return Err(self.lost(account, generation, AuthLostCause::ClientRejected));
            }
            Err(_) => return Err(AccessFailure::Transient),
        };
        // The account was forgotten while the refresh was under way: nothing is kept, and the
        // account is gone for this call (section 4).
        if self.cache.generation(account) != generation {
            return Err(AccessFailure::AuthLost(AuthLostCause::GrantEnded));
        }
        if let Some(scope) = answer.scope.as_deref()
            && scopes::classify(scope) != Grant::Complete
        {
            return Err(self.lost(account, generation, AuthLostCause::GrantEnded));
        }
        let ends_at = self
            .clock
            .now_ms()
            .saturating_add(answer.expires_in_s.saturating_mul(1000));
        let store = &mut self.store;
        let kept = self.cache.put_if_current(
            account,
            generation,
            Secret::from(answer.access_token.expose()),
            ends_at,
            || {
                if let Some(rotated) = answer.refresh_token.as_ref() {
                    // A failed write is not this call's failure: the new access token is
                    // kept and the call goes on. The rotation is then lost: the store keeps
                    // the old refresh token, and if Google ended it with the rotation, the
                    // next refresh ends `AuthLost(GrantEnded)` and the owner signs in again.
                    let _ = store.write(account, rotated);
                }
            },
        );
        if !kept {
            return Err(AccessFailure::AuthLost(AuthLostCause::GrantEnded));
        }
        Ok(Secret::from(answer.access_token.expose()))
    }

    fn drop_cached(&mut self, account: &Id) {
        // A 401 is not a forget (3.8): the generation stays.
        self.cache.drop_token(account);
    }
}

/// Prints the store's and the cache's own `Debug` (counts only), never a token.
impl<S: TokenStore + fmt::Debug> fmt::Debug for GoogleAccess<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GoogleAccess")
            .field("store", &self.store)
            .field("cache", &self.cache)
            .finish_non_exhaustive()
    }
}
