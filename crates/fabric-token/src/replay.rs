//! Single use: the nonces of the tokens a service has accepted.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// The nonces a service has seen, each kept until its token expires. Clones share one cache.
#[derive(Clone, Debug, Default)]
pub struct ReplayCache {
    state: Arc<Mutex<State>>,
}

#[derive(Debug, Default)]
struct State {
    /// Nonce to expiry, for tokens in use or used.
    nonces: HashMap<[u8; 16], i64>,
    /// The latest time any check used. A token that expired by then is refused even if a
    /// caller's clock runs behind, so forgetting expired nonces cannot enable a replay.
    latest_ms: i64,
}

pub(crate) enum Refusal {
    Expired,
    Replayed,
}

impl ReplayCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many nonces the cache holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().nonces.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn contains(&self, nonce: &[u8; 16]) -> bool {
        self.lock().nonces.contains_key(nonce)
    }

    /// Holds `nonce` for one request in flight.
    pub(crate) fn reserve(
        &self,
        nonce: [u8; 16],
        expires_ms: i64,
        now_ms: i64,
    ) -> Result<Reservation, Refusal> {
        let mut state = self.lock();
        state.latest_ms = state.latest_ms.max(now_ms);
        let latest = state.latest_ms;
        state.nonces.retain(|_, expiry| *expiry > latest);
        if expires_ms <= latest {
            return Err(Refusal::Expired);
        }
        if state.nonces.contains_key(&nonce) {
            return Err(Refusal::Replayed);
        }
        state.nonces.insert(nonce, expires_ms);
        Ok(Reservation {
            cache: self.clone(),
            nonce,
            committed: false,
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A nonce held for one request. Committing keeps it until its token expires; dropping the
/// reservation uncommitted releases it.
#[derive(Debug)]
pub(crate) struct Reservation {
    cache: ReplayCache,
    nonce: [u8; 16],
    committed: bool,
}

impl Reservation {
    pub(crate) fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.committed {
            self.cache.lock().nonces.remove(&self.nonce);
        }
    }
}
