//! What a schema version 2 broker stored stays readable: the store keeps values of some
//! contract types as they were written and re-encodes them at replay, so every value version 2
//! could write must decode under this version and encode back to the same bytes.
//!
//! The values are in `tests/data/stored-version-2.json`, taken from the accepted fixtures at
//! schema version 2. `command_answers` are the whole answer frames of the commands: the
//! broker stores each and decodes it again when the command id comes back
//! (`crates/fabric-broker-core/src/owner/dedup.rs`).

use std::path::Path;

use fabric_desktop_contracts::agent::Action;
use fabric_desktop_contracts::owner::{
    Account, Agent, Approval, ApprovalCard, Attempt, Authorization, Destination, Grant, GrantEnd,
    LifecycleStatus, Preferences, PreferencesSettings, Profile, ProofReport, RecipientSet, Request,
    Ruling, Scope,
};
use fabric_desktop_contracts::suite::read_json;
use fabric_desktop_contracts::{Entry, decode, decode_record, encode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

fn stored() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/stored-version-2.json");
    read_json(&path).unwrap()
}

fn texts(list: &Value, name: &str) -> Vec<String> {
    let texts: Vec<String> = list
        .as_array()
        .unwrap_or_else(|| panic!("no values for {name}"))
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    assert!(!texts.is_empty(), "{name} has no value");
    texts
}

fn values(stored: &Value, name: &str) -> Vec<String> {
    texts(&stored["values"][name], name)
}

/// The stored command answers, split by whether the method is `pause_agent`.
fn command_answers(stored: &Value) -> (Vec<String>, Vec<String>) {
    let all = texts(&stored["command_answers"], "command_answers");
    let (pause, other): (Vec<String>, Vec<String>) = all.into_iter().partition(|text| {
        let value: Value = serde_json::from_str(text).unwrap();
        value["method"] == "pause_agent"
    });
    assert!(!pause.is_empty(), "no pause_agent answer");
    assert!(!other.is_empty(), "no other command answer");
    (pause, other)
}

/// Each text decodes into `T` and encodes back to the same bytes.
fn reads_back<T: Serialize + DeserializeOwned>(stored: &Value, name: &str) -> Vec<String> {
    let mut failures = Vec::new();
    for text in values(stored, name) {
        match decode_record::<T>(text.as_bytes()) {
            Ok(value) => {
                let again = encode(&value);
                if again != text {
                    failures.push(format!("{name}: {text}\n  encodes as {again}"));
                }
            }
            Err(refusal) => {
                failures.push(format!("{name}: {text}\n  does not decode: {refusal:?}"))
            }
        }
    }
    failures
}

#[test]
fn every_stored_value_of_version_2_reads_back_unchanged() {
    let stored = stored();
    let failures: Vec<String> = [
        reads_back::<Grant>(&stored, "grant"),
        reads_back::<Request>(&stored, "request"),
        reads_back::<ApprovalCard>(&stored, "approval_card"),
        reads_back::<Approval>(&stored, "approval"),
        reads_back::<Account>(&stored, "account"),
        reads_back::<Attempt>(&stored, "attempt"),
        reads_back::<Authorization>(&stored, "authorization"),
        reads_back::<ProofReport>(&stored, "proof_report"),
        // The Rust type of `profile_custom` is `Profile::Custom`: the struct `CustomProfile` is
        // its body without `kind`.
        reads_back::<Profile>(&stored, "profile_custom"),
        reads_back::<Preferences>(&stored, "preferences"),
        reads_back::<RecipientSet>(&stored, "recipient_set"),
        reads_back::<Destination>(&stored, "destination"),
        reads_back::<Scope>(&stored, "scope"),
        reads_back::<Ruling>(&stored, "ruling"),
        reads_back::<LifecycleStatus>(&stored, "lifecycle_status"),
        reads_back::<GrantEnd>(&stored, "grant_end"),
        reads_back::<PreferencesSettings>(&stored, "preferences_settings"),
        reads_back::<Action>(&stored, "action"),
    ]
    .concat();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn every_stored_custom_profile_of_version_2_is_a_custom_profile() {
    let stored = stored();
    for text in values(&stored, "profile_custom") {
        let profile: Profile = decode_record(text.as_bytes()).unwrap();
        assert!(matches!(profile, Profile::Custom(_)), "{text}");
    }
}

#[test]
fn every_stored_command_answer_of_version_2_except_pause_agent_reads_back_unchanged() {
    let (_, answers) = command_answers(&stored());
    let mut failures = Vec::new();
    for text in answers {
        // As `dedup.rs` reads a stored answer.
        match decode(Entry::OwnerResponse, text.as_bytes()) {
            Ok(frame) => {
                let mut again = frame.to_line();
                again.pop();
                if again != text {
                    failures.push(format!("{text}\n  encodes as {again}"));
                }
            }
            Err(refusal) => failures.push(format!("{text}\n  does not decode: {refusal:?}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// This is an open finding of the draft, not a wanted property. A version 2 broker stores the
/// answer to `pause_agent` (`crates/fabric-broker-core/src/owner/dedup.rs`), which holds agent
/// records, and version 3 adds required members to the agent record. If the draft or the store
/// is changed so that these read back, this test must be turned around.
#[test]
fn a_version_2_agent_and_a_stored_pause_agent_answer_do_not_decode_under_version_3() {
    let stored = stored();
    for text in values(&stored, "agent") {
        assert!(decode_record::<Agent>(text.as_bytes()).is_err(), "{text}");
    }
    let (pause, _) = command_answers(&stored);
    for text in pause {
        assert!(
            decode(Entry::OwnerResponse, text.as_bytes()).is_err(),
            "{text}"
        );
    }
}
