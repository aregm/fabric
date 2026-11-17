//! The contract suite of README section 13: reading fixture sources, judging a text against
//! a schema reference with the Rust types, the report, and the mutants derived from the
//! accepted message fixtures.
//!
//! Everything here is computed by the Rust types; a fixture's expectation is only compared,
//! never echoed.

use std::fs;
use std::path::Path;

use serde_json::{Map, Value};

use crate::agent::{Action, RequestStatus};
use crate::common::Limits;
use crate::digest::{Domain, digest, sha256_hex, to_value};
use crate::frame::{Entry, Frame, Layer, decode, decode_record};
use crate::json::{MAX_SAFE_INTEGER, canonical, parse_strict, write_string};
use crate::lifecycle::{RequestState, first_invalid};
use crate::owner::{
    Agent, Authorization, Grant, Preferences, Profile, ProofReport, RecipientSet, Request, Scope,
};
use crate::scalar::{
    ConfigurationText, CustomProfileId, Date, Digest, DomainName, Email, EventTitle, Explanation,
    IanaZone, Id, Label, LocalTime, NameToken, Nonce, OffsetDateTime, PairingCode, PositiveCount,
    ProviderRef, PublicKey, Signature, Task, Timestamp,
};

/// What the Rust types decided about one text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Judgment {
    /// Accepted. `canonical` is the canonical text of the typed value written back;
    /// `parsed_canonical` is the canonical text of the strict JSON value that was read. They
    /// are equal when the type round-trips exactly.
    Accept {
        canonical: String,
        parsed_canonical: String,
        command_digest: Option<Digest>,
        action_digest: Option<Digest>,
    },
    Reject {
        layer: Layer,
        error: &'static str,
    },
}

/// The record references a message fixture may name besides the five frame entries.
pub const RECORD_REFERENCES: [&str; 12] = [
    "owner-channel.schema.json#/$defs/profile",
    "owner-channel.schema.json#/$defs/grant",
    "owner-channel.schema.json#/$defs/scope",
    "owner-channel.schema.json#/$defs/agent",
    "owner-channel.schema.json#/$defs/authorization",
    "owner-channel.schema.json#/$defs/recipient_set",
    "owner-channel.schema.json#/$defs/request",
    "owner-channel.schema.json#/$defs/proof_report",
    "owner-channel.schema.json#/$defs/preferences",
    "common.schema.json#/$defs/limits",
    "agent-channel.schema.json#/$defs/action",
    "agent-channel.schema.json#/$defs/request_status",
];

fn parsed_canonical(bytes: &[u8]) -> String {
    parse_strict(bytes)
        .map(|v| canonical(&v))
        .unwrap_or_default()
}

fn record<T: serde::Serialize + serde::de::DeserializeOwned>(bytes: &[u8]) -> Judgment {
    match decode_record::<T>(bytes) {
        Ok(value) => Judgment::Accept {
            canonical: canonical(&to_value(&value)),
            parsed_canonical: parsed_canonical(bytes),
            command_digest: None,
            action_digest: None,
        },
        Err(r) => Judgment::Reject {
            layer: r.layer,
            error: r.code.as_str(),
        },
    }
}

/// Judges `bytes` against a schema reference with the Rust type of that reference. Fails
/// for a reference the suite does not know.
pub fn judge(reference: &str, bytes: &[u8]) -> Result<Judgment, String> {
    if let Some(entry) = Entry::from_reference(reference) {
        return Ok(match decode(entry, bytes) {
            Ok(frame) => {
                let (command_digest, action_digest) = match &frame {
                    Frame::OwnerRequest(r) => (r.command_digest(), None),
                    Frame::AgentRequest(r) => (None, r.action_digest()),
                    _ => (None, None),
                };
                Judgment::Accept {
                    canonical: canonical(&frame.to_value()),
                    parsed_canonical: parsed_canonical(bytes),
                    command_digest,
                    action_digest,
                }
            }
            Err(r) => Judgment::Reject {
                layer: r.layer,
                error: r.code.as_str(),
            },
        });
    }
    let o = |name: &str| format!("owner-channel.schema.json#/$defs/{name}");
    Ok(match reference {
        r if r == o("profile") => record::<Profile>(bytes),
        r if r == o("grant") => record::<Grant>(bytes),
        r if r == o("scope") => record::<Scope>(bytes),
        r if r == o("agent") => record::<Agent>(bytes),
        r if r == o("authorization") => record::<Authorization>(bytes),
        r if r == o("recipient_set") => record::<RecipientSet>(bytes),
        r if r == o("request") => record::<Request>(bytes),
        r if r == o("proof_report") => record::<ProofReport>(bytes),
        r if r == o("preferences") => record::<Preferences>(bytes),
        "common.schema.json#/$defs/limits" => record::<Limits>(bytes),
        "agent-channel.schema.json#/$defs/action" => record::<Action>(bytes),
        "agent-channel.schema.json#/$defs/request_status" => record::<RequestStatus>(bytes),
        _ => return Err(format!("no Rust type for the reference {reference}")),
    })
}

/// Whether `value` is an instance of the scalar named by `reference`, judged by the Rust
/// type of that scalar. `None` for a reference without a scalar type.
#[must_use]
pub fn scalar_valid(reference: &str, value: &Value) -> Option<bool> {
    fn ok<T: serde::de::DeserializeOwned>(value: &Value) -> bool {
        crate::typed::from_value::<T>(value).is_ok()
    }
    Some(match reference {
        "common.schema.json#/$defs/id" => ok::<Id>(value),
        "common.schema.json#/$defs/timestamp" => ok::<Timestamp>(value),
        "common.schema.json#/$defs/digest" => ok::<Digest>(value),
        "common.schema.json#/$defs/public_key" => ok::<PublicKey>(value),
        "common.schema.json#/$defs/nonce" => ok::<Nonce>(value),
        "common.schema.json#/$defs/signature" => ok::<Signature>(value),
        "common.schema.json#/$defs/pairing_code" => ok::<PairingCode>(value),
        "common.schema.json#/$defs/configuration_text" => ok::<ConfigurationText>(value),
        "common.schema.json#/$defs/count" => ok::<crate::scalar::Count>(value),
        "common.schema.json#/$defs/positive_count" => ok::<PositiveCount>(value),
        "common.schema.json#/$defs/label" => ok::<Label>(value),
        "common.schema.json#/$defs/explanation" => ok::<Explanation>(value),
        "common.schema.json#/$defs/name_token" => ok::<NameToken>(value),
        "common.schema.json#/$defs/email" => ok::<Email>(value),
        "common.schema.json#/$defs/domain" => ok::<DomainName>(value),
        "common.schema.json#/$defs/iana_zone" => ok::<IanaZone>(value),
        "common.schema.json#/$defs/date" => ok::<Date>(value),
        "common.schema.json#/$defs/local_time" => ok::<LocalTime>(value),
        "common.schema.json#/$defs/offset_date_time" => ok::<OffsetDateTime>(value),
        "common.schema.json#/$defs/provider_ref" => ok::<ProviderRef>(value),
        "owner-channel.schema.json#/$defs/custom_profile_id" => ok::<CustomProfileId>(value),
        "agent-channel.schema.json#/$defs/task" => ok::<Task>(value),
        "agent-channel.schema.json#/$defs/event_title" => ok::<EventTitle>(value),
        _ => return None,
    })
}

/// The first invalid index of a sequence of state names (README section 13, `lifecycle.json`).
/// A name that is not a state is invalid where it stands.
#[must_use]
pub fn lifecycle_first_invalid(names: &[Value]) -> Option<usize> {
    let mut states = Vec::new();
    for (i, name) in names.iter().enumerate() {
        match name.as_str().and_then(RequestState::from_name) {
            Some(state) => states.push(state),
            None => return first_invalid(&states).or(Some(i)),
        }
    }
    first_invalid(&states)
}

fn hex_bytes(hex: &str) -> Result<Vec<u8>, String> {
    if !hex.len().is_multiple_of(2) {
        return Err("odd number of hex digits".into());
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

/// The bytes a fixture source stands for: exactly one of `value` (its canonical text),
/// `text`, `hex` and `generate` (README section 13).
pub fn source_bytes(fixture: &Map<String, Value>) -> Result<Vec<u8>, String> {
    let sources: Vec<&str> = ["value", "text", "hex", "generate"]
        .into_iter()
        .filter(|k| fixture.contains_key(*k))
        .collect();
    if sources.len() != 1 {
        return Err(format!("expected exactly one source, found {sources:?}"));
    }
    let v = &fixture[sources[0]];
    match sources[0] {
        "value" => Ok(canonical(v).into_bytes()),
        "text" => v
            .as_str()
            .map(|s| s.as_bytes().to_vec())
            .ok_or_else(|| "text is not a string".into()),
        "hex" => hex_bytes(v.as_str().ok_or("hex is not a string")?),
        _ => {
            let part = |k: &str| {
                v.get(k)
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("generate.{k} is not a string"))
            };
            let count = v
                .get("count")
                .and_then(Value::as_u64)
                .ok_or("generate.count is not a count")?;
            let count = usize::try_from(count).map_err(|e| e.to_string())?;
            Ok(format!(
                "{}{}{}",
                part("prefix")?,
                part("fill")?.repeat(count),
                part("suffix")?
            )
            .into_bytes())
        }
    }
}

/// Reads an ordinary JSON file of the fixtures.
pub fn read_json(path: &Path) -> Result<Value, String> {
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// The message fixture files in name order.
pub fn message_files(contract: &Path) -> Result<Vec<String>, String> {
    let dir = contract.join("fixtures/messages");
    let mut names: Vec<String> = fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().and_then(|e| e.file_name().into_string().ok()))
        .filter(|n| n.ends_with(".json"))
        .collect();
    names.sort();
    Ok(names)
}

fn opt(d: &Option<Digest>) -> &str {
    d.as_ref().map_or("-", Digest::as_str)
}

fn obj(v: &Value) -> Result<&Map<String, Value>, String> {
    v.as_object().ok_or_else(|| "expected an object".to_owned())
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// The report of README section 13 and the disagreements with the fixtures' expectations.
/// Lines are sorted bytewise and carry no line feed.
pub fn report(contract: &Path) -> Result<(Vec<String>, Vec<String>), String> {
    let fx = contract.join("fixtures");
    let mut lines = Vec::new();
    let mut problems = Vec::new();

    let strict = read_json(&fx.join("strict-json.json"))?;
    for (list, accept) in [("accept", true), ("reject", false)] {
        for e in strict[list].as_array().ok_or("strict-json lists")? {
            let name = str_of(e, "name");
            let bytes = source_bytes(obj(e)?)?;
            match parse_strict(&bytes) {
                Ok(v) => {
                    let c = canonical(&v);
                    lines.push(format!("strict\t{name}\taccept\t{}", sha256_hex(&c)));
                    if !accept {
                        problems.push(format!("strict {name}: accepted, expected a refusal"));
                    } else if e.get("canonical").and_then(Value::as_str) != Some(c.as_str()) {
                        problems.push(format!("strict {name}: canonical form differs"));
                    }
                }
                Err(err) => {
                    lines.push(format!("strict\t{name}\treject"));
                    if accept {
                        problems.push(format!("strict {name}: refused ({err})"));
                    }
                }
            }
        }
    }

    let scalars = read_json(&fx.join("scalars.json"))?;
    for (reference, lists) in obj(&scalars)? {
        for kind in ["accept", "reject"] {
            for (i, v) in lists[kind]
                .as_array()
                .ok_or("scalar lists")?
                .iter()
                .enumerate()
            {
                let Some(ok) = scalar_valid(reference, v) else {
                    problems.push(format!("scalar {reference}: no Rust scalar type"));
                    continue;
                };
                let got = if ok { "accept" } else { "reject" };
                lines.push(format!("scalar\t{reference}\t{kind}\t{i}\t{got}"));
                if got != kind {
                    problems.push(format!("scalar {reference} {kind}[{i}] = {v}: {got}"));
                }
            }
        }
    }

    let digests = read_json(&fx.join("digests.json"))?;
    for e in digests.as_array().ok_or("digests list")? {
        let name = str_of(e, "name");
        let Some(domain) = Domain::from_name(str_of(e, "domain")) else {
            problems.push(format!("digest {name}: unknown domain"));
            continue;
        };
        let d = digest(domain, &e["subject"]);
        lines.push(format!("digest\t{name}\t{d}"));
        if Some(d.as_str()) != e.get("digest").and_then(Value::as_str)
            || Some(canonical(&e["subject"]).as_str()) != e.get("canonical").and_then(Value::as_str)
        {
            problems.push(format!("digest {name}: differs"));
        }
    }

    let lifecycle = read_json(&fx.join("lifecycle.json"))?;
    for e in lifecycle.as_array().ok_or("lifecycle list")? {
        let name = str_of(e, "name");
        let states = e["states"].as_array().ok_or("states")?;
        let first = lifecycle_first_invalid(states);
        let (verdict, index) = match first {
            None => ("valid", "-".to_owned()),
            Some(i) => ("invalid", i.to_string()),
        };
        lines.push(format!("lifecycle\t{name}\t{verdict}\t{index}"));
        let expected_first = e["first_invalid"].as_u64().map(|i| i as usize);
        if e["valid"].as_bool() != Some(first.is_none()) || expected_first != first {
            problems.push(format!("lifecycle {name}: differs"));
        }
    }

    let handshakes = read_json(&fx.join("handshake.json"))?;
    for e in handshakes.as_array().ok_or("handshake list")? {
        let name = str_of(e, "name");
        let computed = handshake_entry(e).map_err(|err| format!("handshake {name}: {err}"))?;
        lines.push(format!(
            "handshake\t{name}\t{}\t{}",
            computed.pairing_code,
            sha256_hex(&computed.text)
        ));
        if computed.pairing_code.as_str() != str_of(e, "pairing_code") {
            problems.push(format!("handshake {name}: pairing code differs"));
        }
        if computed.text != str_of(e, "text") {
            problems.push(format!("handshake {name}: text differs"));
        }
    }

    for file in message_files(contract)? {
        let e = read_json(&fx.join("messages").join(&file))?;
        let fixture = obj(&e)?;
        let reference = str_of(&e, "schema");
        let judgment = judge(reference, &source_bytes(fixture)?)
            .map_err(|err| format!("message {file}: {err}"))?;
        let x = &e["expect"];
        match &judgment {
            Judgment::Accept {
                canonical: c,
                parsed_canonical,
                command_digest,
                action_digest,
            } => {
                lines.push(format!(
                    "message\t{file}\taccept\t{}\t{}\t{}",
                    sha256_hex(c),
                    opt(command_digest),
                    opt(action_digest)
                ));
                if c != parsed_canonical {
                    problems.push(format!(
                        "message {file}: the typed value does not round-trip"
                    ));
                }
                let expected = |k: &str| x.get(k).and_then(Value::as_str).map(str::to_owned);
                if str_of(x, "verdict") != "accept"
                    || expected("canonical").as_deref() != Some(c.as_str())
                    || expected("command_digest").as_deref()
                        != command_digest.as_ref().map(Digest::as_str)
                    || expected("action_digest").as_deref()
                        != action_digest.as_ref().map(Digest::as_str)
                {
                    problems.push(format!("message {file}: accepted, expectation {x}"));
                }
            }
            Judgment::Reject { layer, error } => {
                lines.push(format!(
                    "message\t{file}\treject\t{}\t{error}",
                    layer.as_str()
                ));
                if str_of(x, "verdict") != "reject"
                    || str_of(x, "layer") != layer.as_str()
                    || str_of(x, "error") != *error
                {
                    problems.push(format!(
                        "message {file}: rejected at {} with {error}, expectation {x}",
                        layer.as_str()
                    ));
                }
            }
        }
    }

    lines.sort();
    Ok((lines, problems))
}

/// What this side computes for one entry of `handshake.json`: the pairing code of its public
/// key and the handshake text of its four values. The signature is checked by the tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Handshake {
    pub public_key: PublicKey,
    pub pairing_code: PairingCode,
    pub text: String,
}

/// Computes a `handshake.json` entry's pairing code and text from its members.
pub fn handshake_entry(e: &Value) -> Result<Handshake, String> {
    let read = |k: &str| str_of(e, k).to_owned();
    let public_key = PublicKey::new(read("public_key")).map_err(|err| err.to_string())?;
    let enrollment_id = Id::new(read("enrollment_id")).map_err(|err| err.to_string())?;
    let broker_nonce = Nonce::new(read("broker_nonce")).map_err(|err| err.to_string())?;
    let schema_version = e["schema_version"]
        .as_u64()
        .ok_or("schema_version is not a count")?;
    Ok(Handshake {
        pairing_code: crate::handshake::pairing_code(&public_key),
        text: crate::handshake::handshake_text(
            schema_version,
            &enrollment_id,
            &public_key,
            &broker_nonce,
        ),
        public_key,
    })
}

/// One mutant: an id `<file>#<n>`, the fixture's schema, and the canonical text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mutant {
    pub id: String,
    pub schema: String,
    pub text: String,
}

impl Mutant {
    /// The JSONL line, members in the order id, schema, text, strings in canonical escaping.
    #[must_use]
    pub fn to_line(&self) -> String {
        let mut line = String::from("{\"id\": ");
        write_string(&self.id, &mut line);
        line.push_str(", \"schema\": ");
        write_string(&self.schema, &mut line);
        line.push_str(", \"text\": ");
        write_string(&self.text, &mut line);
        line.push('}');
        line
    }
}

#[derive(Clone, Debug)]
enum Step {
    Key(String),
    Index(usize),
}

fn at_mut<'a>(root: &'a mut Value, path: &[Step]) -> &'a mut Value {
    path.iter().fold(root, |node, step| match step {
        Step::Key(k) => &mut node[k.as_str()],
        Step::Index(i) => &mut node[*i],
    })
}

fn with_replaced(root: &Value, path: &[Step], new: Value) -> Value {
    let mut copy = root.clone();
    *at_mut(&mut copy, path) = new;
    copy
}

fn replacements() -> [Value; 10] {
    [
        Value::Null,
        Value::Bool(true),
        Value::from(0),
        Value::from(-1),
        Value::from(1),
        Value::from(MAX_SAFE_INTEGER),
        Value::from(""),
        Value::from("x"),
        Value::Array(Vec::new()),
        Value::Object(Map::new()),
    ]
}

fn mutate(root: &Value, path: &mut Vec<Step>, node: &Value, out: &mut Vec<Value>) {
    // 1. an object member, removed
    if let Some(Step::Key(name)) = path.last() {
        let mut copy = root.clone();
        if let Value::Object(parent) = at_mut(&mut copy, &path[..path.len() - 1]) {
            parent.remove(name.as_str());
        }
        out.push(copy);
    }
    // 2. an object, with an unknown member
    if let Value::Object(members) = node {
        let mut extended = members.clone();
        extended.insert("zz_unknown".to_owned(), Value::Null);
        out.push(with_replaced(root, path, Value::Object(extended)));
    }
    // 3. the node replaced by each fixed value that differs from it
    let own = canonical(node);
    for r in replacements() {
        if canonical(&r) != own {
            out.push(with_replaced(root, path, r));
        }
    }
    // 4. strings
    if let Value::String(s) = node {
        out.push(with_replaced(root, path, Value::String(format!("{s} "))));
        let upper = s.to_ascii_uppercase();
        if &upper != s {
            out.push(with_replaced(root, path, Value::String(upper)));
        }
        if !s.is_empty() {
            let mut shorter = s.clone();
            shorter.pop();
            out.push(with_replaced(root, path, Value::String(shorter)));
        }
    }
    // 5. non-empty arrays
    if let Value::Array(items) = node
        && let Some(first) = items.first()
    {
        let mut longer = items.clone();
        longer.push(first.clone());
        out.push(with_replaced(root, path, Value::Array(longer)));
        out.push(with_replaced(root, path, Value::Array(items[1..].to_vec())));
    }
    // 6. integers
    if let Some(n) = node.as_i64() {
        if n < MAX_SAFE_INTEGER {
            out.push(with_replaced(root, path, Value::from(n + 1)));
        }
        if n > -MAX_SAFE_INTEGER {
            out.push(with_replaced(root, path, Value::from(n - 1)));
        }
    }
    // 7. an object, written as the array of its member values in name order
    if let Value::Object(members) = node {
        let mut names: Vec<&String> = members.keys().collect();
        names.sort();
        let values = names.into_iter().map(|n| members[n].clone()).collect();
        out.push(with_replaced(root, path, Value::Array(values)));
    }
    // children: members in code point order, elements in order
    match node {
        Value::Object(members) => {
            let mut names: Vec<&String> = members.keys().collect();
            names.sort();
            for name in names {
                path.push(Step::Key(name.clone()));
                mutate(root, path, &members[name], out);
                path.pop();
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                path.push(Step::Index(i));
                mutate(root, path, item, out);
                path.pop();
            }
        }
        _ => {}
    }
}

/// The mutated copies of `value`, in generation order.
#[must_use]
pub fn mutations(value: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    mutate(value, &mut Vec::new(), value, &mut out);
    out
}

/// The mutants of every accepted message fixture with a `value` source, in file-name order.
pub fn mutants(contract: &Path) -> Result<Vec<Mutant>, String> {
    let mut all = Vec::new();
    for file in message_files(contract)? {
        let e = read_json(&contract.join("fixtures/messages").join(&file))?;
        if str_of(&e["expect"], "verdict") != "accept" {
            continue;
        }
        let Some(value) = e.get("value") else {
            continue;
        };
        let schema = str_of(&e, "schema").to_owned();
        for (n, m) in mutations(value).iter().enumerate() {
            all.push(Mutant {
                id: format!("{file}#{}", n + 1),
                schema: schema.clone(),
                text: canonical(m),
            });
        }
    }
    Ok(all)
}

/// The judgment line of one mutant: `mutant\t<id>\taccept` or `mutant\t<id>\treject\t<layer>\t<error>`.
pub fn mutant_line(mutant: &Mutant) -> Result<String, String> {
    Ok(match judge(&mutant.schema, mutant.text.as_bytes())? {
        Judgment::Accept { .. } => format!("mutant\t{}\taccept", mutant.id),
        Judgment::Reject { layer, error } => {
            format!("mutant\t{}\treject\t{}\t{error}", mutant.id, layer.as_str())
        }
    })
}

/// Reads a mutants JSONL file.
pub fn read_mutants(path: &Path) -> Result<Vec<Mutant>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let v: Value = serde_json::from_str(l).map_err(|e| e.to_string())?;
            Ok(Mutant {
                id: str_of(&v, "id").to_owned(),
                schema: str_of(&v, "schema").to_owned(),
                text: str_of(&v, "text").to_owned(),
            })
        })
        .collect()
}
