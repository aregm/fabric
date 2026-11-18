//! The `fabric-desktop/1` contract in Rust: strict JSON, the canonical form, the digests, the
//! transport types of the owner and agent channels, decoding by layer, and the request
//! lifecycle (`contracts/desktop/v1/README.md`, work package CD-W01).
//!
//! The schemas and data files in `contracts/desktop/v1/` are the contract. The types here
//! accept exactly the language the schemas describe: every member is required (a nullable one
//! must be present as `null`), no unknown member is accepted, and every pattern, bound,
//! `const` and `enum` is enforced when a value is read. Writing a value and taking its
//! canonical form gives back the canonical form of the text it was read from.
//!
//! Read contract values only with [`decode`], [`decode_record`] or [`typed::from_value`].
//! The types implement `serde::Deserialize`, but `serde_json`'s own readers are more lenient
//! than the schemas: they accept a struct written as an array of its fields.

pub mod agent;
pub mod common;
pub mod digest;
pub mod error;
pub mod frame;
pub mod handshake;
pub mod json;
pub mod lifecycle;
pub mod owner;
pub mod scalar;
pub mod stored;
pub mod suite;
mod tagged;
pub mod typed;

pub use common::{
    AUTHORIZATION_LIFETIME_SECONDS, Limits, MAX_FRAME_BYTES, PROTOCOL, SCHEMA_VERSION,
};
pub use digest::{
    Decision, Domain, action_digest, approval_digest, command_digest, digest, expansion_digest,
    request_digest,
};
pub use frame::{Entry, Frame, Layer, Refusal, decode, decode_record, encode};
pub use handshake::{handshake_text, pairing_code};
pub use json::{StrictJsonError, canonical, parse_strict};
pub use lifecycle::{PlainRequestState, RequestState, first_invalid};
pub use owner::AUTHORIZATION_PROFILE;
pub use stored::{decode_stored_agent, decode_stored_owner_response};
