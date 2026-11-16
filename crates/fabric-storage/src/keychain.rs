//! The macOS Keychain calls behind [`crate::KeychainKey`]: one generic password, read and added
//! through `security-framework` (copied from `crates/fabric-token/src/keys.rs`, not a
//! dependency). Only the status code of a failure leaves this module (D37).

use security_framework::base::Error;
use security_framework::os::macos::keychain::SecKeychain;
use security_framework::os::macos::passwords::find_generic_password;

use crate::key::{KeyError, StoreKey};

/// `errSecItemNotFound`: there is no key.
const NOT_FOUND: i32 = -25300;
/// `errSecUserCanceled`: the owner pressed Deny.
const USER_CANCELED: i32 = -128;
/// `errSecAuthFailed`: the owner gave a wrong password or refused.
const AUTH_FAILED: i32 = -25293;
/// `errSecInteractionNotAllowed`: the Keychain is locked and may not ask now.
const INTERACTION_NOT_ALLOWED: i32 = -25308;
/// `errSecDecode`: the item exists but does not hold 32 bytes.
const DECODE: i32 = -26275;

/// The key in the generic password `service`/`account` of the default keychain, or `None`.
pub(crate) fn read(service: &str, account: &str) -> Result<Option<StoreKey>, KeyError> {
    let keychain = SecKeychain::default().map_err(key_error)?;
    let keychains = std::slice::from_ref(&keychain);
    match find_generic_password(Some(keychains), service, account) {
        Ok((password, _)) => {
            // Copied straight into the key's zeroed-on-drop memory. The framework's own buffer is
            // freed by SecKeychainItemFreeContent and cannot be wiped under forbid(unsafe_code).
            StoreKey::from_slice(&password)
                .map(Some)
                .ok_or(KeyError::Failed { status: DECODE })
        }
        Err(error) if error.code() == NOT_FOUND => Ok(None),
        Err(error) => Err(key_error(error)),
    }
}

/// Adds the generic password `service`/`account` holding the key. An item that already exists
/// is a failure: a key is never replaced.
pub(crate) fn add(service: &str, account: &str, key: &StoreKey) -> Result<(), KeyError> {
    let keychain = SecKeychain::default().map_err(key_error)?;
    keychain
        .add_generic_password(service, account, key.bytes())
        .map_err(key_error)
}

/// The status mapping of design 3.2. The error's text is never kept.
fn key_error(error: Error) -> KeyError {
    status_error(error.code())
}

/// Maps a Keychain status that is not "not found" to a [`KeyError`].
fn status_error(status: i32) -> KeyError {
    match status {
        USER_CANCELED | AUTH_FAILED => KeyError::Refused { status },
        INTERACTION_NOT_ALLOWED => KeyError::Locked { status },
        _ => KeyError::Failed { status },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Unavailable;

    #[test]
    fn a_denial_is_refused_and_never_retried() {
        for status in [USER_CANCELED, AUTH_FAILED] {
            let error = status_error(status);
            assert_eq!(error, KeyError::Refused { status });
            assert_eq!(error.unavailable(), Unavailable::KeychainRefused);
            assert!(!error.unavailable().retry_may_help());
        }
    }

    #[test]
    fn a_locked_keychain_and_any_other_status_are_retried() {
        assert_eq!(
            status_error(INTERACTION_NOT_ALLOWED),
            KeyError::Locked {
                status: INTERACTION_NOT_ALLOWED
            }
        );
        for status in [INTERACTION_NOT_ALLOWED, -25299, DECODE, -4] {
            let unavailable = status_error(status).unavailable();
            assert_eq!(unavailable, Unavailable::KeychainLocked);
            assert!(unavailable.retry_may_help());
        }
    }
}
