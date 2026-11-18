//! What a schema version 2 broker stored stays readable: the store keeps values of some
//! contract types as they were written and re-encodes them at replay, so every value version 2
//! could write must decode under this version and encode back to the same bytes.
//!
//! The values are in `tests/data/stored-version-2.json`, taken from the accepted fixtures at
//! schema version 2. `command_answers` are the whole answer frames of the commands: the
//! broker stores each and decodes it again when the command id comes back
//! (`crates/fabric-broker-core/src/owner/dedup.rs`). A version 2 answer to `pause_agent` holds
//! agent records without the members version 3 added; the broker reads it with
//! [`decode_stored_owner_response`], which adds them, while the channel's decoders refuse it.

use std::path::Path;

use fabric_desktop_contracts::agent::Action;
use fabric_desktop_contracts::owner::{
    Account, Agent, Approval, ApprovalCard, Attempt, Authorization, Destination, Grant, GrantEnd,
    LifecycleStatus, Preferences, PreferencesSettings, Profile, ProofReport, RecipientSet, Request,
    Ruling, Scope,
};
use fabric_desktop_contracts::suite::{message_files, read_json, source_bytes};
use fabric_desktop_contracts::{
    Entry, Frame, canonical, decode, decode_record, decode_stored_agent,
    decode_stored_owner_response, encode,
};
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

/// A version 2 agent record with the members version 3 added: `enrollment` `approved`, and
/// null for the other five.
fn upgraded_agent(agent: &Value) -> Value {
    let mut agent = agent.clone();
    let record = agent.as_object_mut().unwrap();
    assert!(!record.contains_key("enrollment"));
    record.insert("enrollment".into(), "approved".into());
    for member in [
        "pairing_code",
        "added_at",
        "key_proven_at",
        "approved_at",
        "removed_at",
    ] {
        record.insert(member.into(), Value::Null);
    }
    agent
}

/// A version 2 `pause_agent` answer with every agent record upgraded.
fn upgraded_answer(text: &str) -> Value {
    let mut frame: Value = serde_json::from_str(text).unwrap();
    for agent in frame["result"]["agents"].as_array_mut().unwrap() {
        *agent = upgraded_agent(agent);
    }
    frame
}

/// The frame as the value of its canonical line.
fn frame_value(frame: &Frame) -> Value {
    serde_json::from_str(frame.to_line().trim_end()).unwrap()
}

/// The canonical text of each accepted owner response fixture whose method is `pause_agent`.
fn current_pause_agent_answers() -> Vec<String> {
    let contract = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/desktop/v1");
    let mut answers = Vec::new();
    for name in message_files(&contract).unwrap() {
        if !name.starts_with("accept-") {
            continue;
        }
        let fixture = read_json(&contract.join("fixtures/messages").join(&name)).unwrap();
        if fixture["schema"] != Entry::OwnerResponse.reference()
            || fixture["value"]["method"] != "pause_agent"
        {
            continue;
        }
        let bytes = source_bytes(fixture.as_object().unwrap()).unwrap();
        answers.push(String::from_utf8(bytes).unwrap());
    }
    assert!(
        !answers.is_empty(),
        "no pause_agent answer among the fixtures"
    );
    answers
}

/// The wire stays strict: the decoders of the channels refuse a version 2 agent record and a
/// version 2 answer to `pause_agent`.
#[test]
fn the_strict_decoders_refuse_a_version_2_agent_and_a_version_2_pause_agent_answer() {
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

#[test]
fn a_stored_version_2_agent_reads_back_with_the_members_version_3_added() {
    for text in values(&stored(), "agent") {
        let agent = decode_stored_agent(text.as_bytes()).unwrap_or_else(|e| panic!("{text}: {e}"));
        let again: Value = serde_json::from_str(&encode(&agent)).unwrap();
        let v2: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(again, upgraded_agent(&v2), "{text}");
        // What it reads back as is a valid version 3 record.
        assert_eq!(
            decode_record::<Agent>(encode(&agent).as_bytes()).unwrap(),
            agent
        );
    }
}

#[test]
fn a_stored_version_2_pause_agent_answer_reads_back_with_the_members_version_3_added() {
    let (pause, _) = command_answers(&stored());
    for text in pause {
        let frame =
            decode_stored_owner_response(text.as_bytes()).unwrap_or_else(|e| panic!("{text}: {e}"));
        assert_eq!(frame_value(&frame), upgraded_answer(&text), "{text}");
        // What it reads back as is a valid version 3 frame.
        let line = frame.to_line();
        assert_eq!(
            decode(Entry::OwnerResponse, line.trim_end().as_bytes()).unwrap(),
            frame
        );
    }
}

#[test]
fn every_other_stored_command_answer_of_version_2_reads_as_the_strict_decoder_reads_it() {
    let (_, answers) = command_answers(&stored());
    for text in answers {
        let strict = decode(Entry::OwnerResponse, text.as_bytes()).unwrap();
        let read = decode_stored_owner_response(text.as_bytes()).unwrap();
        assert_eq!(read, strict, "{text}");
    }
}

#[test]
fn a_version_3_pause_agent_answer_reads_as_the_strict_decoder_reads_it() {
    for text in current_pause_agent_answers() {
        let strict = decode(Entry::OwnerResponse, text.as_bytes()).unwrap();
        let read = decode_stored_owner_response(text.as_bytes()).unwrap();
        assert_eq!(read, strict, "{text}");
    }
}

/// Only a record without `enrollment` is upgraded: a record that has it but lacks another
/// member of version 3 is refused by both stored readers.
#[test]
fn an_agent_with_enrollment_but_without_another_member_is_refused_by_the_stored_readers() {
    let answer: Value = serde_json::from_str(&current_pause_agent_answers()[0]).unwrap();
    let agent = &answer["result"]["agents"][0];
    for member in [
        "pairing_code",
        "added_at",
        "key_proven_at",
        "approved_at",
        "removed_at",
    ] {
        let mut lacking = agent.clone();
        assert!(lacking.as_object_mut().unwrap().remove(member).is_some());
        let text = canonical(&lacking);
        assert!(decode_stored_agent(text.as_bytes()).is_err(), "{text}");
        let mut frame = answer.clone();
        frame["result"]["agents"][0] = lacking;
        let text = canonical(&frame);
        assert!(
            decode_stored_owner_response(text.as_bytes()).is_err(),
            "{text}"
        );
    }
}
