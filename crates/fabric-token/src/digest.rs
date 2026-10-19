//! SHA-256 digests of request bodies and actions.

use fabric_policy::Action;
use sha2::{Digest, Sha256};

use crate::cbor::{self, CborError};

#[must_use]
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// The token's `args` claim: SHA-256 of the action's deterministic CBOR, a map with the
/// action's `kind` and its exact arguments under the same names as in its JSON form.
pub fn action_digest(action: &Action) -> Result<[u8; 32], CborError> {
    Ok(sha256(&cbor::to_vec(action)?))
}
