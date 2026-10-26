//! The encrypted calendar store (decision D20): one file per person, holding the deterministic
//! CBOR of the events, encrypted with XChaCha20-Poly1305 under a key that the calendar service
//! keeps in the OS credential store (Windows Credential Manager, or the login Keychain on macOS).
//!
//! File layout: the 8-byte magic `FABCAL01`, a random 24-byte nonce, then the ciphertext with
//! its 16-byte tag. The associated data is the magic followed by the person's ID, so a file
//! cannot be passed off as another person's.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{Key, KeyInit, XChaCha20Poly1305, XNonce};
use fabric_token::cbor;
use fabric_token::{SecretStore, TokenError, random};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"FABCAL01";
const NONCE_BYTES: usize = 24;
const FORMAT: &str = "fabric-demo-calendar-v1";

/// One calendar entry. Only the calendar service and its owner ever see titles and IDs.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub start_ms: i64,
    pub length_ms: i64,
    pub title: String,
    /// Events that are not busy do not block time.
    pub busy: bool,
    /// For meetings an agent added: the token that allowed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_by: Option<AddedBy>,
}

impl Event {
    #[must_use]
    pub const fn end_ms(&self) -> i64 {
        self.start_ms.saturating_add(self.length_ms)
    }
}

/// Where an agent-added meeting came from, for the owner's view and the audit log.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AddedBy {
    pub negotiation: String,
    /// The Isabelle proof ID of the check that permitted the meeting.
    pub proof: String,
    /// The token's nonce.
    pub token: String,
}

#[derive(Debug)]
pub enum StoreError {
    Io(io::Error),
    /// The file is not a calendar store for this person and key, or it was altered.
    Unreadable(String),
    Key(TokenError),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "calendar store: {error}"),
            Self::Unreadable(detail) => write!(f, "calendar store: {detail}"),
            Self::Key(error) => write!(f, "calendar store key: {error}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<io::Error> for StoreError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// The store's 32-byte key, kept as `<person>/store` in the secret store.
pub struct StoreKey(Zeroizing<[u8; 32]>);

impl StoreKey {
    /// Loads the person's store key, generating and saving it on first use.
    pub fn load_or_create(secrets: &dyn SecretStore, person: &str) -> Result<Self, StoreError> {
        let name = format!("{person}/store");
        if let Some(stored) = secrets.get(&name).map_err(StoreError::Key)? {
            let bytes = <[u8; 32]>::try_from(stored.as_slice()).map_err(|_| {
                StoreError::Key(TokenError::Key(format!(
                    "{name} holds {} bytes; expected 32",
                    stored.len()
                )))
            })?;
            return Ok(Self(Zeroizing::new(bytes)));
        }
        let key = Zeroizing::new(random::<32>().map_err(StoreError::Key)?);
        secrets
            .put(&name, key.as_slice())
            .map_err(StoreError::Key)?;
        Ok(Self(key))
    }

    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new(Key::from_slice(self.0.as_slice()))
    }
}

impl fmt::Debug for StoreKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StoreKey(..)")
    }
}

#[derive(Serialize, Deserialize)]
struct Contents {
    format: String,
    person: String,
    events: Vec<Event>,
}

/// A person's events, held in memory and written through to the encrypted file.
#[derive(Debug)]
pub struct Store {
    person: String,
    path: PathBuf,
    key: StoreKey,
    events: Vec<Event>,
}

impl Store {
    /// Opens the store at `path`, or creates it with `fixtures` if there is no file yet.
    pub fn open_or_create(
        path: &Path,
        person: &str,
        key: StoreKey,
        fixtures: &[Event],
    ) -> Result<Self, StoreError> {
        let mut store = Self {
            person: person.to_owned(),
            path: path.to_owned(),
            key,
            events: Vec::new(),
        };
        match fs::read(path) {
            Ok(bytes) => store.events = store.unseal(&bytes)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                store.events = fixtures.to_vec();
                store.write()?;
            }
            Err(error) => return Err(error.into()),
        }
        Ok(store)
    }

    #[must_use]
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Replaces every event and writes the file. On failure the store is unchanged.
    pub fn replace(&mut self, events: Vec<Event>) -> Result<(), StoreError> {
        let previous = std::mem::replace(&mut self.events, events);
        self.write().inspect_err(|_| self.events = previous)
    }

    /// Adds one event and writes the file. On failure the store is unchanged.
    pub fn add(&mut self, event: Event) -> Result<(), StoreError> {
        self.events.push(event);
        self.write().inspect_err(|_| {
            self.events.pop();
        })
    }

    fn write(&self) -> Result<(), StoreError> {
        let contents = Contents {
            format: FORMAT.to_owned(),
            person: self.person.clone(),
            events: self.events.clone(),
        };
        let plaintext = Zeroizing::new(
            cbor::to_vec(&contents).map_err(|error| StoreError::Unreadable(error.to_string()))?,
        );
        let nonce = random::<NONCE_BYTES>().map_err(StoreError::Key)?;
        let ciphertext = self
            .key
            .cipher()
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: &self.associated_data(),
                },
            )
            .map_err(|_| StoreError::Unreadable("encryption failed".to_owned()))?;
        let mut file = Vec::with_capacity(MAGIC.len() + NONCE_BYTES + ciphertext.len());
        file.extend_from_slice(MAGIC);
        file.extend_from_slice(&nonce);
        file.extend_from_slice(&ciphertext);

        let partial = self.path.with_extension("partial");
        fs::write(&partial, file)?;
        fs::rename(&partial, &self.path)?;
        Ok(())
    }

    fn unseal(&self, file: &[u8]) -> Result<Vec<Event>, StoreError> {
        let unreadable =
            |detail: &str| StoreError::Unreadable(format!("{}: {detail}", self.path.display()));
        let rest = file
            .strip_prefix(MAGIC.as_slice())
            .ok_or_else(|| unreadable("not a calendar store"))?;
        if rest.len() < NONCE_BYTES {
            return Err(unreadable("truncated"));
        }
        let (nonce, ciphertext) = rest.split_at(NONCE_BYTES);
        let plaintext = Zeroizing::new(
            self.key
                .cipher()
                .decrypt(
                    XNonce::from_slice(nonce),
                    Payload {
                        msg: ciphertext,
                        aad: &self.associated_data(),
                    },
                )
                .map_err(|_| {
                    unreadable(&format!(
                        "cannot decrypt it with {}'s store key, or it was altered",
                        self.person
                    ))
                })?,
        );
        let value = cbor::decode(&plaintext).map_err(|error| unreadable(&error.to_string()))?;
        let contents: Contents = value
            .deserialized()
            .map_err(|error| unreadable(&error.to_string()))?;
        if contents.format != FORMAT || contents.person != self.person {
            return Err(unreadable("format or person does not match"));
        }
        Ok(contents.events)
    }

    fn associated_data(&self) -> Vec<u8> {
        [MAGIC.as_slice(), self.person.as_bytes()].concat()
    }
}
