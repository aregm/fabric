//! Readers for values a broker stored under an earlier schema version and reads again, such as
//! the answer of a command it gives again when the command id returns
//! (`crates/fabric-broker-core/src/owner/dedup.rs`).
//!
//! Schema version 3 added six required members to the agent record. A broker that stored an
//! answer to `pause_agent` under schema version 2 holds agent records without them. These
//! readers add the members to such a record, and to nothing else, then read the value with the
//! strict decoders. The decoders of the channels, [`decode`] and [`decode_record`], stay
//! strict: a version 2 agent record on the wire is refused.

use serde_json::Value;

use crate::common::MAX_FRAME_BYTES;
use crate::frame::{Entry, Frame, Refusal, decode, decode_record};
use crate::json::{canonical, parse_strict};
use crate::owner::Agent;

/// Gives a version 2 agent record, an object without `enrollment`, the members schema
/// version 3 added: `enrollment` `approved` and null for the other five. Any other value is
/// left as it is.
fn upgrade_agent(value: &mut Value) {
    let Value::Object(record) = value else {
        return;
    };
    if record.contains_key("enrollment") {
        return;
    }
    record.insert(
        "enrollment".to_owned(),
        Value::String("approved".to_owned()),
    );
    for member in [
        "pairing_code",
        "added_at",
        "key_proven_at",
        "approved_at",
        "removed_at",
    ] {
        record.insert(member.to_owned(), Value::Null);
    }
}

/// The bytes again, or the canonical text of the upgraded value when one was upgraded. A
/// text that is not strict JSON is passed on unchanged, so the strict decoder refuses it as
/// it would.
fn upgraded(bytes: &[u8], upgrade: impl FnOnce(&mut Value) -> bool) -> Vec<u8> {
    let Ok(mut value) = parse_strict(bytes) else {
        return bytes.to_vec();
    };
    if upgrade(&mut value) {
        canonical(&value).into_bytes()
    } else {
        bytes.to_vec()
    }
}

fn upgrade_pause_agent(value: &mut Value) -> bool {
    let Value::Object(frame) = value else {
        return false;
    };
    if frame.get("method") != Some(&Value::String("pause_agent".to_owned())) {
        return false;
    }
    let Some(Value::Array(agents)) = frame
        .get_mut("result")
        .and_then(Value::as_object_mut)
        .and_then(|result| result.get_mut("agents"))
    else {
        return false;
    };
    agents.iter_mut().for_each(upgrade_agent);
    true
}

/// Decodes the answer frame of an owner command that a broker stored, possibly under schema
/// version 2. It reads strict JSON as [`decode`] does; in an answer to `pause_agent`, every
/// agent record without `enrollment` gets the members version 3 added (`enrollment`
/// `approved`, and null for `pairing_code`, `added_at`, `key_proven_at`, `approved_at` and
/// `removed_at`), and the value is then decoded as [`decode`] decodes its canonical text
/// against [`Entry::OwnerResponse`]. It accepts nothing more than the strict decoder except
/// those records. Not for the channel: frames that arrive are read with [`decode`].
///
/// # Errors
///
/// The refusal of [`decode`].
pub fn decode_stored_owner_response(bytes: &[u8]) -> Result<Frame, Refusal> {
    // The frame layer's bounds hold for the stored text itself, so the canonical text of an
    // upgraded answer is not read where the stored text would be refused.
    if bytes.len() > MAX_FRAME_BYTES || bytes.contains(&b'\n') {
        return decode(Entry::OwnerResponse, bytes);
    }
    decode(Entry::OwnerResponse, &upgraded(bytes, upgrade_pause_agent))
}

/// Decodes one agent record that a broker stored, possibly under schema version 2: a record
/// without `enrollment` gets the members version 3 added, as in
/// [`decode_stored_owner_response`], and is then read with [`decode_record`]. Not for the
/// channel.
///
/// # Errors
///
/// The refusal of [`decode_record`].
pub fn decode_stored_agent(bytes: &[u8]) -> Result<Agent, Refusal> {
    decode_record(&upgraded(bytes, |value| {
        upgrade_agent(value);
        true
    }))
}
