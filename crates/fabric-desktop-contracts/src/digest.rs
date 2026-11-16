//! Domain-separated digests of canonical forms (README section 4).

use serde::Serialize;
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};

use crate::agent::Action;
use crate::json::canonical;
use crate::owner::{Duration, Expansion, Scope};
use crate::scalar::{Digest, Id};

/// The five digest domains of README section 4.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Domain {
    /// The `action` object `{"tool", "arguments"}`.
    Action,
    /// `{"agent_id", "action", "scope"}`: what the owner saw on the approval card.
    Request,
    /// `{"request_id", "request_digest", "decision", "scope", "duration"}`.
    Approval,
    /// `{"method", "params"}` of an owner command frame.
    Command,
    /// The `expansion` array of a custom profile save.
    Expansion,
}

impl Domain {
    pub const ALL: [Domain; 5] = [
        Domain::Action,
        Domain::Request,
        Domain::Approval,
        Domain::Command,
        Domain::Expansion,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Domain::Action => "action",
            Domain::Request => "request",
            Domain::Approval => "approval",
            Domain::Command => "command",
            Domain::Expansion => "expansion",
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Domain> {
        Domain::ALL.into_iter().find(|d| d.as_str() == name)
    }
}

/// Lowercase hexadecimal of bytes.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 of `text`'s UTF-8 bytes in lowercase hexadecimal, as the report prints it (README section 13).
#[must_use]
pub fn sha256_hex(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

/// `"sha256:" + hex(SHA-256("fabric-desktop/1\n" + domain + "\n" + canonical(value)))`.
#[must_use]
pub fn digest(domain: Domain, value: &Value) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(format!("{}\n{}\n", crate::common::PROTOCOL, domain.as_str()).as_bytes());
    hasher.update(canonical(value).as_bytes());
    let text = format!("sha256:{}", hex(&hasher.finalize()));
    Digest::new(text).expect("a SHA-256 in lowercase hex is a digest")
}

/// The JSON value of a contract type. Contract types always serialize to JSON.
pub(crate) fn to_value<T: Serialize + ?Sized>(value: &T) -> Value {
    serde_json::to_value(value).expect("contract types serialize to JSON")
}

/// The action digest that a one-request grant and an authorization bind.
#[must_use]
pub fn action_digest(action: &Action) -> Digest {
    digest(Domain::Action, &to_value(action))
}

/// `request_digest` over `{"agent_id", "action", "scope"}`; with the approved scope it is the
/// `effective_request_digest`.
#[must_use]
pub fn request_digest(agent_id: &Id, action: &Action, scope: &Scope) -> Digest {
    let subject = json!({
        "agent_id": to_value(agent_id),
        "action": to_value(action),
        "scope": to_value(scope),
    });
    digest(Domain::Request, &subject)
}

/// What the owner decided, as the approval digest binds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Approve this scope for this duration.
    Approve { scope: Scope, duration: Duration },
    /// Deny; the subject then carries `null` scope and duration.
    Deny,
}

/// `approval_digest`: the owner's exact decision.
#[must_use]
pub fn approval_digest(request_id: &Id, request_digest: &Digest, decision: &Decision) -> Digest {
    let (name, scope, duration) = match decision {
        Decision::Approve { scope, duration } => ("approve", to_value(scope), to_value(duration)),
        Decision::Deny => ("deny", Value::Null, Value::Null),
    };
    let mut subject = Map::new();
    subject.insert("request_id".into(), to_value(request_id));
    subject.insert("request_digest".into(), to_value(request_digest));
    subject.insert("decision".into(), Value::String(name.into()));
    subject.insert("scope".into(), scope);
    subject.insert("duration".into(), duration);
    digest(Domain::Approval, &Value::Object(subject))
}

/// The command digest of `{"method", "params"}`, used for command deduplication.
#[must_use]
pub fn command_digest(method: &str, params: &Value) -> Digest {
    let subject = json!({ "method": method, "params": params });
    digest(Domain::Command, &subject)
}

/// The digest the owner confirms when a custom profile widens authority (README section 7).
#[must_use]
pub fn expansion_digest(expansion: &Expansion) -> Digest {
    digest(Domain::Expansion, &to_value(expansion))
}
