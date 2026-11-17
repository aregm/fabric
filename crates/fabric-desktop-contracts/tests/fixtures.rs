//! The fixtures of `contracts/desktop/v1/fixtures/` (README section 13), every entry computed
//! in Rust and compared with its expectation.

use std::path::{Path, PathBuf};

use fabric_desktop_contracts::digest::{Domain, digest};
use fabric_desktop_contracts::suite::{
    self, Judgment, RECORD_REFERENCES, lifecycle_first_invalid, scalar_valid, source_bytes,
};
use fabric_desktop_contracts::{Entry, canonical, parse_strict};
use serde_json::Value;

fn contract() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/desktop/v1")
}

fn fixture(name: &str) -> Value {
    suite::read_json(&contract().join("fixtures").join(name)).unwrap()
}

fn list<'a>(value: &'a Value, key: &str) -> &'a Vec<Value> {
    value[key].as_array().unwrap()
}

#[test]
fn strict_json_accepts_and_canonicalizes() {
    let file = fixture("strict-json.json");
    let mut failures = Vec::new();
    for e in list(&file, "accept") {
        let name = e["name"].as_str().unwrap();
        let bytes = source_bytes(e.as_object().unwrap()).unwrap();
        match parse_strict(&bytes) {
            Ok(v) if Some(canonical(&v).as_str()) == e["canonical"].as_str() => {}
            Ok(v) => failures.push(format!("{name}: canonical {}", canonical(&v))),
            Err(err) => failures.push(format!("{name}: refused, {err}")),
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn strict_json_refuses() {
    let file = fixture("strict-json.json");
    let accepted: Vec<&str> = list(&file, "reject")
        .iter()
        .filter(|e| parse_strict(&source_bytes(e.as_object().unwrap()).unwrap()).is_ok())
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert!(accepted.is_empty(), "accepted: {accepted:?}");
}

#[test]
fn scalars_match_their_lists() {
    let file = fixture("scalars.json");
    let mut failures = Vec::new();
    for (reference, lists) in file.as_object().unwrap() {
        for (kind, expected) in [("accept", true), ("reject", false)] {
            for (i, v) in list(lists, kind).iter().enumerate() {
                match scalar_valid(reference, v) {
                    None => failures.push(format!("{reference}: no Rust scalar type")),
                    Some(ok) if ok != expected => {
                        failures.push(format!("{reference} {kind}[{i}] {v}"))
                    }
                    Some(_) => {}
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn digests_match() {
    let mut failures = Vec::new();
    for e in fixture("digests.json").as_array().unwrap() {
        let name = e["name"].as_str().unwrap();
        let domain = Domain::from_name(e["domain"].as_str().unwrap()).expect(name);
        if Some(canonical(&e["subject"]).as_str()) != e["canonical"].as_str() {
            failures.push(format!("{name}: canonical"));
        }
        if Some(digest(domain, &e["subject"]).as_str()) != e["digest"].as_str() {
            failures.push(format!("{name}: digest"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn lifecycle_sequences_match() {
    let mut failures = Vec::new();
    for e in fixture("lifecycle.json").as_array().unwrap() {
        let name = e["name"].as_str().unwrap();
        let first = lifecycle_first_invalid(list(e, "states"));
        let expected = e["first_invalid"].as_u64().map(|i| i as usize);
        if first != expected || e["valid"].as_bool() != Some(first.is_none()) {
            failures.push(format!("{name}: computed {first:?}"));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

fn known_reference(reference: &str) -> bool {
    Entry::from_reference(reference).is_some() || RECORD_REFERENCES.contains(&reference)
}

#[test]
fn message_fixtures_are_at_least_one_hundred() {
    let files = suite::message_files(&contract()).expect("fixtures/messages must exist");
    assert!(files.len() >= 100, "only {} message fixtures", files.len());
}

#[test]
fn message_fixtures_match_their_expectations() {
    let dir = contract().join("fixtures/messages");
    let mut failures = Vec::new();
    for file in suite::message_files(&contract()).unwrap() {
        let e = suite::read_json(&dir.join(&file)).unwrap();
        let reference = e["schema"].as_str().unwrap();
        assert!(
            known_reference(reference),
            "{file}: unknown schema reference {reference}"
        );
        let bytes = source_bytes(e.as_object().unwrap()).unwrap();
        let x = &e["expect"];
        match suite::judge(reference, &bytes).unwrap() {
            Judgment::Accept {
                canonical: c,
                parsed_canonical,
                command_digest,
                action_digest,
            } => {
                if x["verdict"] != "accept" {
                    failures.push(format!("{file}: accepted, expected {x}"));
                    continue;
                }
                if c != parsed_canonical {
                    failures.push(format!(
                        "{file}: does not round-trip:\n{parsed_canonical}\n{c}"
                    ));
                }
                if x["canonical"].as_str() != Some(c.as_str()) {
                    failures.push(format!("{file}: canonical differs"));
                }
                if x["command_digest"].as_str() != command_digest.as_ref().map(|d| d.as_str()) {
                    failures.push(format!("{file}: command digest {command_digest:?}"));
                }
                if x["action_digest"].as_str() != action_digest.as_ref().map(|d| d.as_str()) {
                    failures.push(format!("{file}: action digest {action_digest:?}"));
                }
            }
            Judgment::Reject { layer, error } => {
                if x["verdict"] != "reject" || x["layer"] != layer.as_str() || x["error"] != error {
                    failures.push(format!(
                        "{file}: rejected at {} with {error}, expected {x}",
                        layer.as_str()
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn an_unknown_reference_fails_loudly() {
    let err = suite::judge("owner-channel.schema.json#/$defs/session", b"{}").unwrap_err();
    assert!(err.contains("session"), "{err}");
}

#[test]
fn every_accepted_mutant_round_trips() {
    let mut failures = Vec::new();
    let mutants = suite::mutants(&contract()).unwrap();
    assert!(!mutants.is_empty());
    for m in &mutants {
        if let Judgment::Accept {
            canonical: c,
            parsed_canonical,
            ..
        } = suite::judge(&m.schema, m.text.as_bytes()).unwrap()
            && c != parsed_canonical
        {
            failures.push(m.id.clone());
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn mutant_lines_are_json_with_members_in_order() {
    let m = suite::Mutant {
        id: "a.json#1".into(),
        schema: "s".into(),
        text: r#"{"k":"\u0001"}"#.into(),
    };
    assert_eq!(
        m.to_line(),
        r#"{"id": "a.json#1", "schema": "s", "text": "{\"k\":\"\\u0001\"}"}"#
    );
}

#[test]
fn mutations_follow_the_generation_order() {
    let value = serde_json::json!({"b": 7, "a": ["s"]});
    let texts: Vec<String> = suite::mutations(&value).iter().map(canonical).collect();
    let expected = [
        // the root object: unknown member, the ten replacements, then written as an array
        r#"{"a":["s"],"b":7,"zz_unknown":null}"#,
        "null",
        "true",
        "0",
        "-1",
        "1",
        "9007199254740991",
        r#""""#,
        r#""x""#,
        "[]",
        "{}",
        r#"[["s"],7]"#,
        // member "a" first: removed, replaced (skipping []), appended, first removed
        r#"{"b":7}"#,
    ];
    assert_eq!(&texts[..expected.len()], &expected);
    assert!(texts.contains(&r#"{"a":["s"],"b":8}"#.to_owned()));
    assert!(texts.contains(&r#"{"a":["S"],"b":7}"#.to_owned()));
    assert!(texts.contains(&r#"{"a":[""],"b":7}"#.to_owned()));
}
