//! The verifier, the state and the nonce of one sign-in, and the S256 challenge (3.2, 3.2.1).
//!
//! Each is 32 random bytes from the operating system (`getrandom`), written as base64url
//! without padding: 43 characters of `A-Z a-z 0-9 - _`. They are held as [`Secret`] and never
//! leave the broker's memory, except the state and the nonce in the authorization address
//! (RK12). The challenge is the base64url of the SHA-256 of the verifier's 43 characters.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::secret::Secret;

/// The length of a verifier, a state or a nonce: 32 bytes in base64url without padding.
pub const RANDOM_TEXT_LEN: usize = 43;

/// The operating system gave no random bytes. A fixed label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RandomUnavailable;

impl fmt::Display for RandomUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the operating system gave no random bytes")
    }
}

impl std::error::Error for RandomUnavailable {}

/// 32 random bytes as 43 characters of base64url without padding.
pub fn random_text() -> Result<Secret, RandomUnavailable> {
    let mut bytes = Zeroizing::new([0u8; 32]);
    getrandom::fill(bytes.as_mut()).map_err(|_| RandomUnavailable)?;
    Ok(Secret::new(URL_SAFE_NO_PAD.encode(bytes.as_ref())))
}

/// The S256 challenge of `verifier`: base64url without padding of SHA-256 of its characters
/// (3.2.1). Not secret: it is in the address; it reveals nothing of the verifier.
#[must_use]
pub fn challenge(verifier: &Secret) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.expose().as_bytes()))
}

/// The three random values of one sign-in (3.2).
pub struct FlowSecrets {
    /// The PKCE verifier, sent only in the token request (3.4).
    pub verifier: Secret,
    /// The state, in the address and compared with the callback's (3.3).
    pub state: Secret,
    /// The nonce, in the address and compared with the identity token's (3.5 item 8).
    pub nonce: Secret,
}

impl FlowSecrets {
    /// Three fresh values.
    pub fn new() -> Result<FlowSecrets, RandomUnavailable> {
        Ok(FlowSecrets {
            verifier: random_text()?,
            state: random_text()?,
            nonce: random_text()?,
        })
    }
}

/// Prints nothing of the values.
impl fmt::Debug for FlowSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FlowSecrets([redacted])")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_texts_are_43_base64url_characters_and_differ() {
        let a = random_text().expect("random");
        let b = random_text().expect("random");
        for text in [&a, &b] {
            assert_eq!(text.len(), RANDOM_TEXT_LEN);
            assert!(
                text.expose()
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
            );
        }
        assert_ne!(a.expose(), b.expose());
    }

    #[test]
    fn the_challenge_is_the_rfc_7636_example() {
        // RFC 7636 appendix B.
        let verifier = Secret::from("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        assert_eq!(
            challenge(&verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn flow_secrets_print_nothing() {
        let secrets = FlowSecrets::new().expect("random");
        let text = format!("{secrets:?}");
        assert!(!text.contains(secrets.verifier.expose()));
        assert_eq!(text, "FlowSecrets([redacted])");
    }
}
