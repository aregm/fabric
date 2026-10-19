//! Per-action access tokens for the calendar demo (docs/isabelle-calendar-demo.md, section 9).
//!
//! A policy gate mints a token only from an Isabelle check that permitted exactly one action.
//! The token is a CBOR Web Token in a COSE_Sign1 envelope, signed with the gate's Ed25519 key.
//! It names the one service that may accept it, binds the action's exact arguments by digest
//! and the agent's key by `cnf`, expires after 60 seconds, and works once. The agent proves
//! possession of its key on every use.

pub mod cbor;
pub mod digest;
pub mod error;
pub mod hex;
pub mod keys;
pub mod pop;
pub mod replay;
pub mod token;
pub mod verify;

pub use cbor::CborError;
pub use digest::{action_digest, sha256};
pub use ed25519_dalek::{SigningKey, VerifyingKey};
pub use error::TokenError;
#[cfg(windows)]
pub use keys::CredentialStore;
pub use keys::{KeyDirectory, MemoryStore, PersonKeys, PublicKeys, SecretStore, random};
pub use pop::{BindingKey, POP_MAX_AGE_MS, POP_MAX_LEAD_MS, Presentation};
pub use replay::ReplayCache;
pub use token::{
    Claims, LIFETIME_SECONDS, MAX_TOKEN_BYTES, Minted, Minter, Service, issuer, read_unverified,
};
pub use verify::{Accepted, Reason, Rejection, Verifier};
