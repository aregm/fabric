//! Token storage (section 4, PD11, PD12).
//!
//! Only the refresh token is stored, one item per account, in a [`TokenStore`]. Access tokens
//! live in memory in an [`AccessCache`]. [`forget`] removes an account from both in one step,
//! so no access token outlives its refresh token. The Keychain store is phase 2 and is not in
//! this crate yet.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use fabric_desktop_contracts::scalar::Id;

use crate::secret::Secret;

/// Why the token store failed. Fixed labels only (section 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenStoreError {
    /// The person refused access (Keychain: `errSecUserCanceled`, `errSecAuthFailed`).
    /// Rejected `account_authentication_required` for a call.
    Refused,
    /// The store cannot be read now (Keychain: `errSecInteractionNotAllowed`, the Mac is
    /// locked). Transient for a call.
    Locked,
    /// Any other failure, with the store's status number. Rejected for a call.
    Failed {
        /// The store's status number.
        status: i32,
    },
}

impl fmt::Display for TokenStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenStoreError::Refused => f.write_str("the token store refused access"),
            TokenStoreError::Locked => f.write_str("the token store is locked"),
            TokenStoreError::Failed { status } => {
                write!(f, "the token store failed with status {status}")
            }
        }
    }
}

impl std::error::Error for TokenStoreError {}

/// Where refresh tokens are kept, one per account (section 4). The account id is the broker's
/// (`account-<32 hex>`), never the address.
pub trait TokenStore: Send {
    /// The account's refresh token, or `None` when the store holds none.
    fn read(&self, account: &Id) -> Result<Option<Secret>, TokenStoreError>;
    /// Stores the account's refresh token, replacing an older one.
    fn write(&mut self, account: &Id, refresh_token: &Secret) -> Result<(), TokenStoreError>;
    /// Deletes the account's refresh token. Deleting what is not there is `Ok`.
    fn delete(&mut self, account: &Id) -> Result<(), TokenStoreError>;
}

/// Locks `mutex`, recovering the map from a panic in another holder: the maps hold plain
/// values with no invariant a panic could break.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The in-memory token store of phase 1 and of every run with the test key source (section 4).
/// Clones share one map, so the sign-in worker and the provider worker see the same tokens.
/// Its tokens end with the process.
#[derive(Clone, Default)]
pub struct MemoryTokenStore {
    tokens: Arc<Mutex<HashMap<Id, Secret>>>,
}

impl MemoryTokenStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> MemoryTokenStore {
        MemoryTokenStore::default()
    }

    /// How many accounts hold a token.
    #[must_use]
    pub fn len(&self) -> usize {
        lock(&self.tokens).len()
    }

    /// Whether no account holds a token.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        lock(&self.tokens).is_empty()
    }
}

impl TokenStore for MemoryTokenStore {
    fn read(&self, account: &Id) -> Result<Option<Secret>, TokenStoreError> {
        Ok(lock(&self.tokens)
            .get(account)
            .map(|token| Secret::from(token.expose())))
    }

    fn write(&mut self, account: &Id, refresh_token: &Secret) -> Result<(), TokenStoreError> {
        lock(&self.tokens).insert(account.clone(), Secret::from(refresh_token.expose()));
        Ok(())
    }

    fn delete(&mut self, account: &Id) -> Result<(), TokenStoreError> {
        lock(&self.tokens).remove(account);
        Ok(())
    }
}

/// Prints how many accounts, never a token or an account id.
impl fmt::Debug for MemoryTokenStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MemoryTokenStore({} accounts)", self.len())
    }
}

/// Access tokens in memory: account id to the token and the time it ends, in Unix ms (3.8,
/// section 4). Clones share one map.
#[derive(Clone, Default)]
pub struct AccessCache {
    tokens: Arc<Mutex<HashMap<Id, (Secret, u64)>>>,
}

impl AccessCache {
    /// An empty cache.
    #[must_use]
    pub fn new() -> AccessCache {
        AccessCache::default()
    }

    /// The account's access token when it ends later than `now_ms + margin_ms`; otherwise
    /// `None`, and the caller refreshes (3.8: a margin of 60 s).
    #[must_use]
    pub fn get_valid(&self, account: &Id, now_ms: u64, margin_ms: u64) -> Option<Secret> {
        let tokens = lock(&self.tokens);
        let (token, ends_at_ms) = tokens.get(account)?;
        (*ends_at_ms > now_ms.saturating_add(margin_ms)).then(|| Secret::from(token.expose()))
    }

    /// Keeps `token` for the account until `ends_at_ms`, replacing an older one.
    pub fn put(&self, account: &Id, token: Secret, ends_at_ms: u64) {
        lock(&self.tokens).insert(account.clone(), (token, ends_at_ms));
    }

    /// Drops the account's access token (a 401, 3.8; or [`forget`]).
    pub fn drop_account(&self, account: &Id) {
        lock(&self.tokens).remove(account);
    }
}

/// Prints how many accounts, never a token or an account id.
impl fmt::Debug for AccessCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AccessCache({} accounts)", lock(&self.tokens).len())
    }
}

/// Why an account's sign-in is gone (3.8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthLostCause {
    /// Google no longer honours the refresh token (`invalid_grant`, `admin_policy_enforced`,
    /// a token whose scopes changed). The account must sign in again.
    GrantEnded,
    /// Google no longer accepts the client (`invalid_client`, `unauthorized_client`). Signing
    /// in again will not help until the client is repaired.
    ClientRejected,
}

/// Why no access token could be had for a call (3.8, section 4). In every case nothing was
/// sent to the Calendar host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessFailure {
    /// The refresh was not answered, or answered with a failure that may pass, or the token
    /// store is locked. Transient for the call.
    Transient,
    /// The account's sign-in is gone. Its stored refresh token and its cached access token
    /// were deleted. Rejected `account_authentication_required` for the call.
    AuthLost(AuthLostCause),
    /// The token store refused or failed (`Refused` or `Failed`, never `Locked`). Rejected
    /// `account_authentication_required` for the call.
    Store(TokenStoreError),
}

/// Where a Calendar call gets the access token of its account (3.8). `oauth::access`
/// implements it with the token store, the cache and the token endpoint; a test may implement
/// it with fixed tokens.
pub trait AccessTokens: Send {
    /// The account's access token, valid for at least 60 s more, refreshed when needed. One
    /// refresh attempt at most, inside `deadline_ms` (Unix ms).
    fn access_token(&mut self, account: &Id, deadline_ms: u64) -> Result<Secret, AccessFailure>;
    /// Drops the account's cached access token, after a 401 from a Calendar call (3.8).
    fn drop_cached(&mut self, account: &Id);
}

/// Removes the account from the token store and from the cache in one step (section 4):
/// `AuthLost`, a failed reconnect and `disconnect_google` all go through it. The cached access
/// token is dropped first and always, so it never outlives the refresh token even when the
/// store fails; the store's error is returned.
pub fn forget(
    store: &mut dyn TokenStore,
    cache: &AccessCache,
    account: &Id,
) -> Result<(), TokenStoreError> {
    cache.drop_account(account);
    store.delete(account)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(n: u32) -> Id {
        Id::new(format!("account-{n:032x}")).expect("a valid id")
    }

    /// A store that fails every call with a chosen error (g17's double, local to these tests).
    struct Failing(TokenStoreError);

    impl TokenStore for Failing {
        fn read(&self, _: &Id) -> Result<Option<Secret>, TokenStoreError> {
            Err(self.0)
        }
        fn write(&mut self, _: &Id, _: &Secret) -> Result<(), TokenStoreError> {
            Err(self.0)
        }
        fn delete(&mut self, _: &Id) -> Result<(), TokenStoreError> {
            Err(self.0)
        }
    }

    #[test]
    fn the_memory_store_reads_what_was_written_and_deletes_idempotently() {
        let mut store = MemoryTokenStore::new();
        let a = account(1);
        assert!(store.read(&a).expect("read").is_none());
        store
            .write(&a, &Secret::from("1//refresh-a"))
            .expect("write");
        assert_eq!(
            store.read(&a).expect("read").expect("present").expose(),
            "1//refresh-a"
        );
        store
            .write(&a, &Secret::from("1//refresh-b"))
            .expect("write");
        assert_eq!(
            store.read(&a).expect("read").expect("present").expose(),
            "1//refresh-b"
        );
        store.delete(&a).expect("delete");
        store.delete(&a).expect("deleting what is not there is Ok");
        assert!(store.read(&a).expect("read").is_none());
        assert!(store.is_empty());
    }

    #[test]
    fn clones_of_the_memory_store_share_one_map_across_threads() {
        let store = MemoryTokenStore::new();
        let mut writer = store.clone();
        let a = account(2);
        let a2 = a.clone();
        std::thread::spawn(move || writer.write(&a2, &Secret::from("t")).expect("write"))
            .join()
            .expect("the thread ends");
        assert_eq!(
            store.read(&a).expect("read").expect("present").expose(),
            "t"
        );
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn the_cache_gives_a_token_only_when_it_ends_after_now_plus_margin() {
        let cache = AccessCache::new();
        let a = account(3);
        assert!(cache.get_valid(&a, 0, 0).is_none());
        cache.put(&a, Secret::from("ya29.x"), 100_000);
        assert_eq!(
            cache.get_valid(&a, 39_999, 60_000).expect("valid").expose(),
            "ya29.x"
        );
        assert!(cache.get_valid(&a, 40_000, 60_000).is_none());
        assert!(cache.get_valid(&a, 100_000, 0).is_none());
        assert!(cache.get_valid(&a, u64::MAX, u64::MAX).is_none());
        assert!(cache.get_valid(&account(4), 0, 0).is_none());
        cache.drop_account(&a);
        assert!(cache.get_valid(&a, 0, 0).is_none());
    }

    #[test]
    fn forget_removes_the_account_from_store_and_cache() {
        let mut store = MemoryTokenStore::new();
        let cache = AccessCache::new();
        let (a, b) = (account(5), account(6));
        for id in [&a, &b] {
            store.write(id, &Secret::from("r")).expect("write");
            cache.put(id, Secret::from("t"), u64::MAX);
        }
        forget(&mut store, &cache, &a).expect("forget");
        assert!(store.read(&a).expect("read").is_none());
        assert!(cache.get_valid(&a, 0, 0).is_none());
        assert!(store.read(&b).expect("read").is_some());
        assert!(cache.get_valid(&b, 0, 0).is_some());
    }

    #[test]
    fn forget_drops_the_access_token_even_when_the_store_fails() {
        let cache = AccessCache::new();
        let a = account(7);
        for error in [
            TokenStoreError::Locked,
            TokenStoreError::Refused,
            TokenStoreError::Failed { status: -25_300 },
        ] {
            cache.put(&a, Secret::from("t"), u64::MAX);
            let mut store = Failing(error);
            assert_eq!(forget(&mut store, &cache, &a), Err(error));
            assert!(cache.get_valid(&a, 0, 0).is_none());
        }
    }

    #[test]
    fn debug_and_errors_print_no_token_and_no_account() {
        let mut store = MemoryTokenStore::new();
        let cache = AccessCache::new();
        let a = account(8);
        store
            .write(&a, &Secret::from("1//secret-refresh"))
            .expect("write");
        cache.put(&a, Secret::from("ya29.secret-access"), 1);
        let text = format!(
            "{store:?} {cache:?} {} {:?}",
            TokenStoreError::Failed { status: 3 },
            TokenStoreError::Locked
        );
        assert!(!text.contains("secret"), "{text}");
        assert!(!text.contains("account-"), "{text}");
    }
}
