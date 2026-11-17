//! The rule theory this crate was built with. The gate loads whatever `Fabric_Broker.thy` its
//! `rules_dir` holds and does not enforce this hash: a caller that wants the rules it was
//! built against compares [`crate::BrokerGate::theory_sha256`] with [`built_theory_sha256`].

use std::sync::OnceLock;

use crate::report::sha256_hex;

/// The text of `isabelle/Fabric_Broker/Fabric_Broker.thy` when the crate was built.
pub const THEORY_TEXT: &str = include_str!("../../../isabelle/Fabric_Broker/Fabric_Broker.thy");

/// The session's `ROOT` (`isabelle/Fabric_Broker/ROOT`) when the crate was built.
pub const SESSION_ROOT: &str = include_str!("../../../isabelle/Fabric_Broker/ROOT");

/// The SHA-256 of [`THEORY_TEXT`] in lowercase hex, computed once.
#[must_use]
pub fn built_theory_sha256() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| sha256_hex(THEORY_TEXT.as_bytes()))
}
