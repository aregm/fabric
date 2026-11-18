//! Token storage through the crate's public surface (section 4), with the scripted store that
//! later tests (g17) use.

mod common;

use common::ScriptedTokenStore;
use fabric_desktop_contracts::scalar::Id;
use fabric_google_calendar::{AccessCache, Secret, TokenStore, TokenStoreError, forget};

fn account() -> Id {
    Id::new("account-0123456789abcdef0123456789abcdef").expect("a valid id")
}

#[test]
fn the_scripted_store_works_until_told_to_fail_and_counts_its_calls() {
    let mut store = ScriptedTokenStore::new();
    let a = account();
    store.write(&a, &Secret::from("1//r")).expect("write");
    assert_eq!(
        store.read(&a).expect("read").expect("present").expose(),
        "1//r"
    );
    for error in [
        TokenStoreError::Locked,
        TokenStoreError::Refused,
        TokenStoreError::Failed { status: -25_308 },
    ] {
        store.fail_with(Some(error));
        assert_eq!(store.read(&a).err(), Some(error));
        assert_eq!(store.write(&a, &Secret::from("x")).err(), Some(error));
        assert_eq!(store.delete(&a).err(), Some(error));
    }
    store.fail_with(None);
    assert_eq!(
        store.read(&a).expect("read").expect("kept").expose(),
        "1//r"
    );
    assert_eq!(store.calls(), 12);
}

#[test]
fn forget_with_a_locked_store_still_drops_the_access_token() {
    let mut store = ScriptedTokenStore::new();
    let cache = AccessCache::new();
    let a = account();
    store.write(&a, &Secret::from("1//r")).expect("write");
    cache.put(&a, Secret::from("ya29.a"), u64::MAX);
    store.fail_with(Some(TokenStoreError::Locked));
    assert_eq!(forget(&mut store, &cache, &a), Err(TokenStoreError::Locked));
    assert!(cache.get_valid(&a, 0, 0).is_none());
    store.fail_with(None);
    forget(&mut store, &cache, &a).expect("forget");
    assert!(store.read(&a).expect("read").is_none());
}
