//! Long-term keys (section 5.2): generated once, kept in the operating system's credential store
//! (Windows Credential Manager, or the login Keychain on macOS), and published to the other
//! processes in the launcher's key directory.
//!
//! Each person has three keys, all held by their trusted host:
//!
//! - `gate`: Ed25519, signs access tokens.
//! - `identity`: Ed25519, signs protocol messages and consent records.
//! - `noise`: X25519, the static key of the person's Noise endpoint.
//!
//! An agent's token-binding key is never stored; the agent makes a new one when it starts
//! ([`BindingKey::generate`](crate::BindingKey::generate)).

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use ed25519_dalek::{SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

use crate::error::TokenError;
use crate::hex;
use crate::token::{Minter, check_person};

/// Fills an array from the operating system's random number generator.
pub fn random<const N: usize>() -> Result<[u8; N], TokenError> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|error| TokenError::Random(error.to_string()))?;
    Ok(bytes)
}

/// Where secrets live. Names look like `alice/gate`.
pub trait SecretStore {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, TokenError>;
    fn put(&self, name: &str, secret: &[u8]) -> Result<(), TokenError>;
    /// Removes a secret. Removing a missing secret is not an error.
    fn delete(&self, name: &str) -> Result<(), TokenError>;
}

/// Secrets in memory only: for tests, and for platforms without a supported credential store.
#[derive(Default)]
pub struct MemoryStore {
    secrets: Mutex<HashMap<String, Zeroizing<Vec<u8>>>>,
}

impl MemoryStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Zeroizing<Vec<u8>>>> {
        self.secrets.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut names: Vec<String> = self.lock().keys().cloned().collect();
        names.sort();
        f.debug_struct("MemoryStore")
            .field("names", &names)
            .finish()
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, TokenError> {
        Ok(self.lock().get(name).cloned())
    }

    fn put(&self, name: &str, secret: &[u8]) -> Result<(), TokenError> {
        self.lock()
            .insert(name.to_owned(), Zeroizing::new(secret.to_vec()));
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<(), TokenError> {
        self.lock().remove(name);
        Ok(())
    }
}

#[cfg(windows)]
pub use credential_manager::CredentialStore;

#[cfg(windows)]
mod credential_manager {
    use std::collections::HashMap;
    use std::fmt;
    use std::sync::Arc;

    use keyring_core::api::CredentialStoreApi as _;
    use keyring_core::{Entry, Error};
    use windows_native_keyring_store::Store;
    use zeroize::Zeroizing;

    use super::SecretStore;
    use crate::error::TokenError;

    /// Windows Credential Manager: one generic credential per secret, with the target name
    /// `<prefix>/<name>` and local-machine persistence, so keys never roam to other computers.
    pub struct CredentialStore {
        store: Arc<Store>,
        prefix: String,
    }

    impl CredentialStore {
        /// The prefix of the demo's keys.
        pub const DEMO: &'static str = "fabric-demo";

        pub fn open(prefix: &str) -> Result<Self, TokenError> {
            Ok(Self {
                store: Store::new().map_err(store_error)?,
                prefix: prefix.to_owned(),
            })
        }

        fn entry(&self, name: &str) -> Result<Entry, TokenError> {
            let target = format!("{}/{name}", self.prefix);
            let modifiers = HashMap::from([("target", target.as_str()), ("persistence", "Local")]);
            self.store
                .build(&self.prefix, name, Some(&modifiers))
                .map_err(store_error)
        }
    }

    impl fmt::Debug for CredentialStore {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("CredentialStore")
                .field("prefix", &self.prefix)
                .finish_non_exhaustive()
        }
    }

    impl SecretStore for CredentialStore {
        fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, TokenError> {
            match self.entry(name)?.get_secret() {
                Ok(secret) => Ok(Some(Zeroizing::new(secret))),
                Err(Error::NoEntry) => Ok(None),
                Err(error) => Err(store_error(error)),
            }
        }

        fn put(&self, name: &str, secret: &[u8]) -> Result<(), TokenError> {
            self.entry(name)?.set_secret(secret).map_err(store_error)
        }

        fn delete(&self, name: &str) -> Result<(), TokenError> {
            match self.entry(name)?.delete_credential() {
                Ok(()) | Err(Error::NoEntry) => Ok(()),
                Err(error) => Err(store_error(error)),
            }
        }
    }

    fn store_error(error: Error) -> TokenError {
        TokenError::Store(error.to_string())
    }
}

#[cfg(target_os = "macos")]
pub use keychain::CredentialStore;

#[cfg(target_os = "macos")]
mod keychain {
    use std::fmt;

    use security_framework::base::Error;
    use security_framework::os::macos::keychain::SecKeychain;
    use security_framework::os::macos::passwords::find_generic_password;
    use zeroize::Zeroizing;

    use super::SecretStore;
    use crate::error::TokenError;

    /// `errSecItemNotFound`.
    const NOT_FOUND: i32 = -25300;

    /// The macOS login Keychain: one generic password per secret, with the service `<prefix>`
    /// and the account `<name>`, so Keychain Access lists the demo's items under its prefix.
    /// Items stay in this user's login keychain on this Mac; nothing goes to iCloud Keychain.
    ///
    /// Reads and deletes use the keychain item API that the `security` command uses, so a
    /// rebuilt `fabric-demo`, or a test binary, can remove items an earlier build created.
    pub struct CredentialStore {
        keychain: SecKeychain,
        prefix: String,
    }

    impl CredentialStore {
        /// The prefix of the demo's keys.
        pub const DEMO: &'static str = "fabric-demo";

        pub fn open(prefix: &str) -> Result<Self, TokenError> {
            Ok(Self {
                keychain: SecKeychain::default().map_err(store_error)?,
                prefix: prefix.to_owned(),
            })
        }
    }

    impl fmt::Debug for CredentialStore {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("CredentialStore")
                .field("prefix", &self.prefix)
                .finish_non_exhaustive()
        }
    }

    impl SecretStore for CredentialStore {
        fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, TokenError> {
            let keychains = std::slice::from_ref(&self.keychain);
            match find_generic_password(Some(keychains), &self.prefix, name) {
                Ok((password, _)) => Ok(Some(Zeroizing::new(password.to_vec()))),
                Err(error) if error.code() == NOT_FOUND => Ok(None),
                Err(error) => Err(store_error(error)),
            }
        }

        fn put(&self, name: &str, secret: &[u8]) -> Result<(), TokenError> {
            // Replaces the password of an existing item, or adds the item.
            self.keychain
                .set_generic_password(&self.prefix, name, secret)
                .map_err(store_error)
        }

        fn delete(&self, name: &str) -> Result<(), TokenError> {
            let keychains = std::slice::from_ref(&self.keychain);
            match find_generic_password(Some(keychains), &self.prefix, name) {
                // `SecKeychainItemDelete` reports nothing through this binding, so look again.
                Ok((_, item)) => item.delete(),
                Err(error) if error.code() == NOT_FOUND => return Ok(()),
                Err(error) => return Err(store_error(error)),
            }
            match find_generic_password(Some(keychains), &self.prefix, name) {
                Err(error) if error.code() == NOT_FOUND => Ok(()),
                Ok(_) => Err(TokenError::Store(format!(
                    "{}/{name} is still in the keychain after deleting it",
                    self.prefix
                ))),
                Err(error) => Err(store_error(error)),
            }
        }
    }

    fn store_error(error: Error) -> TokenError {
        TokenError::Store(error.to_string())
    }
}

const GATE: &str = "gate";
const IDENTITY: &str = "identity";
const NOISE: &str = "noise";

fn secret_name(person: &str, kind: &str) -> String {
    format!("{person}/{kind}")
}

fn secret(
    store: &dyn SecretStore,
    person: &str,
    kind: &str,
) -> Result<Zeroizing<[u8; 32]>, TokenError> {
    let name = secret_name(person, kind);
    let stored = store.get(&name)?.ok_or_else(|| {
        TokenError::Key(format!(
            "{name} is not in the secret store; the launcher creates it on its first run"
        ))
    })?;
    let bytes = <[u8; 32]>::try_from(stored.as_slice()).map_err(|_| {
        TokenError::Key(format!("{name} holds {} bytes; expected 32", stored.len()))
    })?;
    Ok(Zeroizing::new(bytes))
}

/// One person's long-term keys, held by their trusted host.
pub struct PersonKeys {
    person: String,
    gate: SigningKey,
    identity: SigningKey,
    noise: StaticSecret,
}

impl PersonKeys {
    /// Loads `person`'s keys, first generating and storing any that are missing.
    pub fn load_or_create(store: &dyn SecretStore, person: &str) -> Result<Self, TokenError> {
        check_person(person)?;
        for kind in [GATE, IDENTITY, NOISE] {
            let name = secret_name(person, kind);
            if store.get(&name)?.is_none() {
                store.put(&name, Zeroizing::new(random::<32>()?).as_slice())?;
            }
        }
        Self::load(store, person)
    }

    /// Loads `person`'s keys, which must all exist.
    pub fn load(store: &dyn SecretStore, person: &str) -> Result<Self, TokenError> {
        check_person(person)?;
        Ok(Self {
            person: person.to_owned(),
            gate: SigningKey::from_bytes(&*secret(store, person, GATE)?),
            identity: SigningKey::from_bytes(&*secret(store, person, IDENTITY)?),
            noise: StaticSecret::from(*secret(store, person, NOISE)?),
        })
    }

    /// Removes `person`'s keys from the store.
    pub fn delete(store: &dyn SecretStore, person: &str) -> Result<(), TokenError> {
        check_person(person)?;
        for kind in [GATE, IDENTITY, NOISE] {
            store.delete(&secret_name(person, kind))?;
        }
        Ok(())
    }

    #[must_use]
    pub fn person(&self) -> &str {
        &self.person
    }

    /// The identity key, for signing protocol messages and consent records.
    #[must_use]
    pub fn identity(&self) -> &SigningKey {
        &self.identity
    }

    /// The Noise static private key, for the messaging gateway.
    #[must_use]
    pub fn noise_secret(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.noise.to_bytes())
    }

    /// The policy gate's token minter.
    #[must_use]
    pub fn minter(&self) -> Minter {
        Minter::trusted(&self.person, self.gate.clone())
    }

    #[must_use]
    pub fn public(&self) -> PublicKeys {
        PublicKeys {
            gate_ed25519: self.gate.verifying_key().to_bytes(),
            identity_ed25519: self.identity.verifying_key().to_bytes(),
            noise_x25519: PublicKey::from(&self.noise).to_bytes(),
        }
    }
}

impl fmt::Debug for PersonKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PersonKeys")
            .field("person", &self.person)
            .field("public", &self.public())
            .finish_non_exhaustive()
    }
}

/// One person's public keys, as the key directory lists them.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicKeys {
    #[serde(with = "hex::array")]
    pub gate_ed25519: [u8; 32],
    #[serde(with = "hex::array")]
    pub identity_ed25519: [u8; 32],
    #[serde(with = "hex::array")]
    pub noise_x25519: [u8; 32],
}

impl PublicKeys {
    /// The key that tokens from this person's gate verify with.
    pub fn gate(&self) -> Result<VerifyingKey, TokenError> {
        ed25519(&self.gate_ed25519, "gate")
    }

    /// The key that this person's messages and consent records verify with.
    pub fn identity(&self) -> Result<VerifyingKey, TokenError> {
        ed25519(&self.identity_ed25519, "identity")
    }
}

fn ed25519(bytes: &[u8; 32], what: &str) -> Result<VerifyingKey, TokenError> {
    let key = VerifyingKey::from_bytes(bytes)
        .map_err(|_| TokenError::Key(format!("the {what} key is not an Ed25519 public key")))?;
    if key.is_weak() {
        return Err(TokenError::Key(format!(
            "the {what} key is a weak Ed25519 key"
        )));
    }
    Ok(key)
}

/// Everyone's public keys for one run. The launcher writes it to `.local/demo/keys.json`, and
/// every process reads its peers' keys from there.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyDirectory {
    pub format: String,
    pub people: BTreeMap<String, PublicKeys>,
}

impl Default for KeyDirectory {
    fn default() -> Self {
        Self {
            format: Self::FORMAT.to_owned(),
            people: BTreeMap::new(),
        }
    }
}

impl KeyDirectory {
    pub const FORMAT: &'static str = "fabric-demo-keys-v1";

    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, keys: &PersonKeys) {
        self.people.insert(keys.person().to_owned(), keys.public());
    }

    pub fn person(&self, person: &str) -> Result<&PublicKeys, TokenError> {
        self.people
            .get(person)
            .ok_or_else(|| TokenError::Key(format!("{person} is not in the key directory")))
    }

    /// Reads and checks a directory file.
    pub fn read(path: &Path) -> Result<Self, TokenError> {
        let directory: Self = serde_json::from_slice(&fs::read(path)?)?;
        if directory.format != Self::FORMAT {
            return Err(TokenError::Key(format!(
                "{} has format {:?}; expected {:?}",
                path.display(),
                directory.format,
                Self::FORMAT
            )));
        }
        for (person, keys) in &directory.people {
            check_person(person)?;
            keys.gate()?;
            keys.identity()?;
        }
        Ok(directory)
    }

    /// Writes the directory, replacing any previous file in one step.
    pub fn write(&self, path: &Path) -> Result<(), TokenError> {
        let mut text = serde_json::to_vec_pretty(self)?;
        text.push(b'\n');
        let partial = path.with_extension("json.partial");
        fs::write(&partial, text)?;
        fs::rename(&partial, path)?;
        Ok(())
    }
}
