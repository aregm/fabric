//! The store key and where it comes from (design 3.2).
//!
//! The key is 32 raw bytes, used without a key derivation function. Two sources exist: the
//! macOS Keychain (`KeychainKey`, macOS only) and the public [`TEST_KEY`] ([`TestKey`]),
//! which the broker accepts only in debug builds.

/// The public test key. On macOS it keys SQLCipher; elsewhere the store is plain SQLite.
pub const TEST_KEY: [u8; 32] = [0x46; 32];

/// Where a store's key comes from. The store records it at creation and refuses a key of
/// another kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyKind {
    /// The generic password in the macOS Keychain.
    Keychain,
    /// The public [`TEST_KEY`].
    Test,
}

impl KeyKind {
    /// The name stored in `meta.key_kind`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            KeyKind::Keychain => "keychain",
            KeyKind::Test => "test",
        }
    }
}

/// 32 raw bytes, zeroed on drop. No Debug, no Display.
pub struct StoreKey(zeroize::Zeroizing<[u8; 32]>);

impl StoreKey {
    /// Wraps the bytes of a key.
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> StoreKey {
        StoreKey(zeroize::Zeroizing::new(bytes))
    }

    /// A key copied straight from `bytes` into zeroed-on-drop memory; `None` unless 32 bytes.
    /// No bare array of the key is ever bound, so no copy is left unwiped on the stack.
    #[cfg_attr(
        not(target_os = "macos"),
        allow(dead_code, reason = "only the Keychain provider reads bytes")
    )]
    pub(crate) fn from_slice(bytes: &[u8]) -> Option<StoreKey> {
        if bytes.len() != 32 {
            return None;
        }
        let mut key = zeroize::Zeroizing::new([0_u8; 32]);
        key.copy_from_slice(bytes);
        Some(StoreKey(key))
    }

    /// A new key from the system random source.
    ///
    /// # Errors
    ///
    /// [`KeyError::Random`] when the system random source fails.
    pub fn generate() -> Result<StoreKey, KeyError> {
        let mut bytes = zeroize::Zeroizing::new([0_u8; 32]);
        getrandom::fill(bytes.as_mut_slice()).map_err(|_| KeyError::Random)?;
        Ok(StoreKey(bytes))
    }

    /// The raw bytes, for the Keychain item.
    #[cfg_attr(
        not(target_os = "macos"),
        allow(dead_code, reason = "only the Keychain provider stores the bytes")
    )]
    pub(crate) fn bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The SQLCipher raw-key literal `x'<64 hex>'`, built in place in zeroed-on-drop memory.
    #[cfg_attr(
        not(target_os = "macos"),
        allow(dead_code, reason = "only SQLCipher on macOS takes a key")
    )]
    pub(crate) fn sql_literal(&self) -> zeroize::Zeroizing<String> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut text = zeroize::Zeroizing::new(String::with_capacity(67));
        text.push_str("x'");
        for byte in self.0.iter() {
            text.push(char::from(HEX[usize::from(byte >> 4)]));
            text.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        text.push('\'');
        text
    }
}

/// A Keychain failure (its status code only) or a failure of the system random source (D37).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyError {
    /// The owner refused access to the Keychain item.
    Refused {
        /// The Keychain status code.
        status: i32,
    },
    /// The Keychain is locked.
    Locked {
        /// The Keychain status code.
        status: i32,
    },
    /// Any other Keychain failure.
    Failed {
        /// The Keychain status code.
        status: i32,
    },
    /// The system random source failed.
    Random,
}

impl KeyError {
    /// Refused -> KeychainRefused; Locked and Failed -> KeychainLocked (retried; 3.2); Random -> KeyGeneration.
    #[must_use]
    pub fn unavailable(self) -> crate::store::Unavailable {
        use crate::store::Unavailable;
        match self {
            KeyError::Refused { .. } => Unavailable::KeychainRefused,
            KeyError::Locked { .. } | KeyError::Failed { .. } => Unavailable::KeychainLocked,
            KeyError::Random => Unavailable::KeyGeneration,
        }
    }
}

impl std::fmt::Display for KeyError {
    /// Fixed text and the status number.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyError::Refused { status } => {
                write!(
                    f,
                    "the keychain refused access to the store key (status {status})"
                )
            }
            KeyError::Locked { status } => {
                write!(f, "the keychain is locked (status {status})")
            }
            KeyError::Failed { status } => {
                write!(f, "the keychain failed (status {status})")
            }
            KeyError::Random => f.write_str("the system random source failed"),
        }
    }
}

/// A source of the store key.
pub trait KeyProvider: Send {
    /// The kind of key this source gives.
    fn kind(&self) -> KeyKind;

    /// `Ok(None)` when no key exists.
    ///
    /// # Errors
    ///
    /// A [`KeyError`] when the source cannot be read.
    fn read(&self) -> Result<Option<StoreKey>, KeyError>;

    /// Called only when no store file exists and `read` answered `Ok(None)` (3.2).
    ///
    /// # Errors
    ///
    /// A [`KeyError`] when the key cannot be stored.
    fn create(&self, key: &StoreKey) -> Result<(), KeyError>;
}

/// The public `TEST_KEY`. On macOS it keys SQLCipher; elsewhere the store is plain SQLite.
pub struct TestKey;

impl KeyProvider for TestKey {
    /// [`KeyKind::Test`].
    fn kind(&self) -> KeyKind {
        KeyKind::Test
    }

    /// Always [`TEST_KEY`].
    fn read(&self) -> Result<Option<StoreKey>, KeyError> {
        Ok(Some(StoreKey::from_bytes(TEST_KEY)))
    }

    /// Stores nothing: the test key always exists.
    fn create(&self, key: &StoreKey) -> Result<(), KeyError> {
        let _ = key;
        Ok(())
    }
}

/// The store key as a generic password in the macOS Keychain.
#[cfg(target_os = "macos")]
pub struct KeychainKey {
    service: String,
    account: String,
}

#[cfg(target_os = "macos")]
impl KeychainKey {
    /// The Keychain service of the product's store key.
    pub const SERVICE: &'static str = "ai.paracels.fabric.calendar.store";
    /// The Keychain account of the product's store key.
    pub const ACCOUNT: &'static str = "database-key-v1";

    /// The product's key: [`Self::SERVICE`] and [`Self::ACCOUNT`].
    #[must_use]
    pub fn product() -> KeychainKey {
        KeychainKey::with_names(KeychainKey::SERVICE, KeychainKey::ACCOUNT)
    }

    /// A key under other names (the ignored Keychain test uses its own).
    #[must_use]
    pub fn with_names(service: &str, account: &str) -> KeychainKey {
        KeychainKey {
            service: service.to_owned(),
            account: account.to_owned(),
        }
    }
}

#[cfg(target_os = "macos")]
impl KeyProvider for KeychainKey {
    /// [`KeyKind::Keychain`].
    fn kind(&self) -> KeyKind {
        KeyKind::Keychain
    }

    /// Reads the generic password `service`/`account`.
    fn read(&self) -> Result<Option<StoreKey>, KeyError> {
        crate::keychain::read(&self.service, &self.account)
    }

    /// Adds the generic password `service`/`account`.
    fn create(&self, key: &StoreKey) -> Result<(), KeyError> {
        crate::keychain::add(&self.service, &self.account, key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Unavailable;

    #[test]
    fn a_generated_key_is_new_each_time() {
        let a = StoreKey::generate().unwrap();
        let b = StoreKey::generate().unwrap();
        assert_ne!(a.bytes(), b.bytes());
    }

    #[test]
    fn the_sql_literal_is_the_raw_key_in_lowercase_hex() {
        let mut bytes = [0xab_u8; 32];
        bytes[0] = 0x01;
        let literal = StoreKey::from_bytes(bytes).sql_literal();
        assert_eq!(literal.len(), 67);
        assert!(literal.starts_with("x'01ab"));
        assert!(literal.ends_with("abab'"));
    }

    #[test]
    fn a_key_from_a_slice_takes_exactly_32_bytes() {
        assert_eq!(StoreKey::from_slice(&TEST_KEY).unwrap().bytes(), &TEST_KEY);
        assert!(StoreKey::from_slice(&[0x46; 31]).is_none());
        assert!(StoreKey::from_slice(&[0x46; 33]).is_none());
        assert!(StoreKey::from_slice(&[]).is_none());
    }

    #[test]
    fn the_test_key_is_the_public_constant() {
        let key = TestKey.read().unwrap().unwrap();
        assert_eq!(key.bytes(), &TEST_KEY);
        assert_eq!(TestKey.kind(), KeyKind::Test);
        assert!(TestKey.create(&key).is_ok());
    }

    #[test]
    fn each_key_error_maps_to_its_unavailable_reason() {
        assert_eq!(
            KeyError::Refused { status: -128 }.unavailable(),
            Unavailable::KeychainRefused
        );
        assert_eq!(
            KeyError::Locked { status: -25308 }.unavailable(),
            Unavailable::KeychainLocked
        );
        assert_eq!(
            KeyError::Failed { status: -4 }.unavailable(),
            Unavailable::KeychainLocked
        );
        assert_eq!(KeyError::Random.unavailable(), Unavailable::KeyGeneration);
    }

    #[test]
    fn a_random_source_failure_has_its_own_message_and_is_retried() {
        let reason = KeyError::Random.unavailable();
        assert!(reason.retry_may_help());
        assert_eq!(
            reason.message(),
            "The broker could not get random bytes from the system to create the store key. \
             Nothing was written; it tries again when the page next asks."
        );
        assert!(!reason.message().contains("Keychain"));
        assert!(!KeyError::Random.to_string().contains("keychain"));
    }

    #[test]
    fn key_kinds_have_their_stored_names() {
        assert_eq!(KeyKind::Keychain.as_str(), "keychain");
        assert_eq!(KeyKind::Test.as_str(), "test");
    }
}
