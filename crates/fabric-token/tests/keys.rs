//! Keys (task T3.3): created once, loaded from the secret store, and published to the other
//! processes in the key directory.

use std::fs;
use std::path::PathBuf;

use fabric_policy::time::parse_utc;
use fabric_policy::{Action, ActionVerdict, CheckKind, CheckReport, Verdict};
use fabric_token::{
    BindingKey, KeyDirectory, MemoryStore, PersonKeys, ReplayCache, SecretStore, Service,
    TokenError, Verifier, hex,
};

const SECRETS: [&str; 3] = ["alice/gate", "alice/identity", "alice/noise"];

fn scratch(name: &str) -> PathBuf {
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("fabric-token-keys");
    fs::create_dir_all(&directory).expect("the scratch directory can be created");
    directory.join(name)
}

fn permit(action: &Action) -> CheckReport {
    CheckReport {
        kind: CheckKind::Authorize,
        theory: "Check_000001".to_owned(),
        proof_id: "ab".repeat(32),
        policy_sha256: "cd".repeat(32),
        isabelle: "Isabelle2025-2".to_owned(),
        seconds: 1.0,
        verdicts: vec![ActionVerdict {
            action: action.clone(),
            verdict: Verdict::Permit,
            rules: Vec::new(),
        }],
        theory_text: String::new(),
    }
}

fn secret_hex(store: &MemoryStore, name: &str) -> String {
    hex::encode(&store.get(name).unwrap().expect("the secret exists"))
}

#[test]
fn keys_are_created_once_and_then_loaded() {
    let store = MemoryStore::new();
    let first = PersonKeys::load_or_create(&store, "alice").unwrap();
    let again = PersonKeys::load_or_create(&store, "alice").unwrap();
    let loaded = PersonKeys::load(&store, "alice").unwrap();
    assert_eq!(first.public(), again.public());
    assert_eq!(first.public(), loaded.public());

    let public = first.public();
    assert_ne!(public.gate_ed25519, public.identity_ed25519);
    let bob = PersonKeys::load_or_create(&store, "bob").unwrap();
    assert_ne!(bob.public(), public);
}

#[test]
fn loading_needs_every_key_at_its_full_length() {
    let store = MemoryStore::new();
    assert!(matches!(
        PersonKeys::load(&store, "alice"),
        Err(TokenError::Key(_))
    ));

    PersonKeys::load_or_create(&store, "alice").unwrap();
    store.put("alice/noise", &[1; 31]).unwrap();
    assert!(matches!(
        PersonKeys::load(&store, "alice"),
        Err(TokenError::Key(_))
    ));

    PersonKeys::delete(&store, "alice").unwrap();
    for name in SECRETS {
        assert!(store.get(name).unwrap().is_none(), "{name} was deleted");
    }
    PersonKeys::delete(&store, "alice").expect("deleting twice is fine");

    // Person IDs follow the identifier rule, so `alice:admin` cannot name a person.
    assert!(PersonKeys::load_or_create(&store, "alice:admin").is_err());
}

#[test]
fn debug_output_shows_no_secret() {
    let store = MemoryStore::new();
    let keys = PersonKeys::load_or_create(&store, "alice").unwrap();
    let text = format!("{keys:?} {:?} {store:?}", keys.minter());
    for name in SECRETS {
        assert!(!text.contains(&secret_hex(&store, name)), "{name} leaked");
    }
    assert!(text.contains(&hex::encode(&keys.public().gate_ed25519)));
}

#[test]
fn the_directory_carries_public_keys_to_the_verifiers() {
    let store = MemoryStore::new();
    let alice = PersonKeys::load_or_create(&store, "alice").unwrap();
    let bob = PersonKeys::load_or_create(&store, "bob").unwrap();
    let mut directory = KeyDirectory::new();
    directory.insert(&alice);
    directory.insert(&bob);

    let path = scratch("keys.json");
    directory.write(&path).unwrap();
    let read = KeyDirectory::read(&path).unwrap();
    assert_eq!(read, directory);

    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"format\": \"fabric-demo-keys-v1\""));
    for name in SECRETS {
        assert!(
            !text.contains(&secret_hex(&store, name)),
            "{name} in the file"
        );
    }

    // A token from Alice's gate verifies with the key the directory lists for her.
    let now = parse_utc("2027-01-11T08:00:00Z").unwrap();
    let agent = BindingKey::generate().unwrap();
    let action = Action::Handshake {
        peer: "bob".to_owned(),
    };
    let minted = alice
        .minter()
        .mint(&permit(&action), &agent.verifying_key(), now)
        .unwrap();
    let gate_key = read.person("alice").unwrap().gate().unwrap();
    let gateway = Verifier::new(Service::Gateway, "alice", gate_key);
    let body = serde_json::to_vec(&action).unwrap();
    let presentation = agent.present(&minted.bytes, &body, now).unwrap();
    gateway
        .check(&presentation, &action, &body, now, &ReplayCache::new())
        .unwrap();

    assert!(read.person("carol").is_err());
}

/// Changes one thing in a good directory file; the second argument is a weak key.
type Edit = fn(&mut serde_json::Value, &str);

#[test]
fn malformed_directories_are_refused() {
    let store = MemoryStore::new();
    let mut directory = KeyDirectory::new();
    directory.insert(&PersonKeys::load_or_create(&store, "alice").unwrap());
    let good = serde_json::to_value(&directory).unwrap();

    // The identity point is a valid encoding but a weak key.
    let weak = format!("01{}", "00".repeat(31));
    let edits: [(&str, Edit); 5] = [
        ("an old format", |json, _| {
            json["format"] = "fabric-demo-keys-v0".into();
        }),
        ("an unknown field", |json, _| {
            json["people"]["alice"]["extra"] = "x".into();
        }),
        ("a person ID with a space", |json, _| {
            let keys = json["people"]["alice"].clone();
            json["people"]["alice smith"] = keys;
        }),
        ("a short key", |json, _| {
            json["people"]["alice"]["gate_ed25519"] = "abcd".into();
        }),
        ("a weak gate key", |json, weak| {
            json["people"]["alice"]["gate_ed25519"] = weak.into();
        }),
    ];
    for (what, edit) in edits {
        let mut json = good.clone();
        edit(&mut json, &weak);
        let path = scratch("bad-keys.json");
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        assert!(KeyDirectory::read(&path).is_err(), "accepted {what}");
    }
}

/// Removes the test's credentials even when an assertion fails.
#[cfg(windows)]
struct Cleanup<'a> {
    store: &'a fabric_token::CredentialStore,
    person: String,
}

#[cfg(windows)]
impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        let _ = PersonKeys::delete(self.store, &self.person);
    }
}

#[cfg(windows)]
#[test]
#[ignore = "writes to and deletes from Windows Credential Manager; run with --ignored"]
fn windows_credential_manager_keeps_the_keys() {
    use fabric_token::CredentialStore;

    let store = CredentialStore::open("fabric-demo-test").unwrap();
    let person = format!(
        "test-{}",
        hex::encode(&fabric_token::random::<4>().unwrap())
    );
    let cleanup = Cleanup {
        store: &store,
        person: person.clone(),
    };

    let created = PersonKeys::load_or_create(&store, &person).unwrap();
    let reopened = CredentialStore::open("fabric-demo-test").unwrap();
    let loaded = PersonKeys::load(&reopened, &person).unwrap();
    assert_eq!(created.public(), loaded.public());

    drop(cleanup);
    assert!(matches!(
        PersonKeys::load(&store, &person),
        Err(TokenError::Key(_))
    ));
}
