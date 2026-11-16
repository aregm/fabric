//! Digests, proof ids, the contract's proof report and ruling, and tool-call conversion.

mod common;

use fabric_broker_policy::check::{self, NodeReport, Polarity, Statement};
use fabric_broker_policy::{
    Action, Check, Context, Outcome, Recipient, Rule, Target, Verdict, context_digest, proof_id,
    proof_report, ruling,
};
use fabric_desktop_contracts::agent;
use fabric_desktop_contracts::owner::{ProofOutcome, ProofPurpose, ProofReport};
use fabric_desktop_contracts::scalar::{Email, Id, Timestamp};
use fabric_desktop_contracts::{decode_record, encode};
use serde_json::json;

const THEORY_SHA256: &str = "9d92e8fccd34bad0570cb6e6be7649c4a7765a523e26f438a219413be90f4edf";

/// A check as the gate would build it, from a simulated Isabelle report in which exactly
/// `failing` fail. `outcome` is the outcome those results imply.
fn check_of(context: &Context, failing: &[Rule], outcome: Outcome, complete: bool) -> Check {
    let theory = check::generate("Check_000001", context, &common::action()).unwrap();
    let errors = theory
        .lemmas
        .iter()
        .filter(|lemma| match lemma.statement {
            Statement::Rule { rule, polarity } => {
                failing.contains(&rule) == (polarity == Polarity::Holds)
            }
            // The first grant, `g-invite`, authorizes; the others are refused.
            Statement::Grant { index, authorizes } => (index == 0) != authorizes,
        })
        .map(|lemma| (lemma.line, "Failed to finish proof".to_owned()))
        .collect();
    let reading = check::read(
        &theory,
        &NodeReport {
            complete,
            errors,
            writeln: Vec::new(),
        },
    );
    if complete {
        assert_eq!(reading.verdict.outcome(), Some(outcome));
    }
    Check {
        proof_id: proof_id(THEORY_SHA256, "Isabelle2025-2", &theory.text),
        theory: theory.name,
        theory_text: theory.text,
        theory_sha256: THEORY_SHA256.to_owned(),
        isabelle: "Isabelle2025-2".to_owned(),
        context_digest: context_digest(context),
        action_digest: context.digest.clone(),
        verdict: reading.verdict,
        failing_rule: reading.failing_rule,
        rules: reading.rules,
        grants: reading.grants,
        authorizing_grants: reading.authorizing_grants,
        seconds: 0.4567,
    }
}

fn strictly_read(report: &ProofReport) -> ProofReport {
    decode_record::<ProofReport>(encode(report).as_bytes()).expect("the contract accepts it")
}

#[test]
fn the_context_digest_is_the_contract_formula_with_domain_context() {
    let context = common::context();
    let digest = context_digest(&context);
    let value = serde_json::to_value(&context).unwrap();
    let text = format!(
        "fabric-desktop/1\ncontext\n{}",
        fabric_desktop_contracts::canonical(&value)
    );
    assert_eq!(
        digest,
        format!(
            "sha256:{}",
            fabric_desktop_contracts::digest::sha256_hex(&text)
        )
    );
    assert!(fabric_desktop_contracts::scalar::Digest::is_valid(&digest));
    assert_eq!(digest, context_digest(&context.clone()));
}

#[test]
fn each_epoch_changes_the_context_digest_and_the_proof_id() {
    let base = common::context();
    let mut paused = base.clone();
    paused.pause_epoch += 1;
    let mut revoked = base.clone();
    revoked.revocation_epoch += 1;
    let checks: Vec<Check> = [&base, &paused, &revoked]
        .into_iter()
        .map(|context| check_of(context, &[], Outcome::Permit, true))
        .collect();
    for (i, a) in checks.iter().enumerate() {
        assert_eq!(a.proof_id.len(), 64);
        assert!(a.proof_id.bytes().all(|b| b.is_ascii_hexdigit()));
        for b in &checks[i + 1..] {
            assert_ne!(a.context_digest, b.context_digest);
            assert_ne!(a.proof_id, b.proof_id);
        }
    }
}

#[test]
fn the_proof_id_binds_the_theory_hash_and_the_isabelle_version() {
    let text = "theory Check_1\n";
    let id = proof_id(THEORY_SHA256, "Isabelle2025-2", text);
    assert_ne!(id, proof_id(&"0".repeat(64), "Isabelle2025-2", text));
    assert_ne!(id, proof_id(THEORY_SHA256, "Isabelle2025-1", text));
    assert_ne!(
        id,
        proof_id(THEORY_SHA256, "Isabelle2025-2", "theory Check_2\n")
    );
}

#[test]
fn every_report_passes_the_contracts_strict_reader() {
    let context = common::context();
    let request = Id::new("request-1").unwrap();
    let started = Timestamp::new("2026-11-16T10:00:00.000Z").unwrap();
    let cases = [
        (vec![], Outcome::Permit, true, ProofOutcome::Permit, None),
        (
            vec![Rule::NoStrangers],
            Outcome::NeedApproval,
            true,
            ProofOutcome::NeedApproval,
            Some("no_strangers"),
        ),
        (
            vec![Rule::AgentNotPaused, Rule::Authority],
            Outcome::Block,
            true,
            ProofOutcome::Block,
            Some("agent_not_paused"),
        ),
        (
            vec![],
            Outcome::Permit,
            false,
            ProofOutcome::Undecided,
            None,
        ),
    ];
    for (failing, proven, complete, expected, failing_rule) in cases {
        let check = check_of(&context, &failing, proven, complete);
        for purpose in [ProofPurpose::Classification, ProofPurpose::FreshPermit] {
            let report = proof_report(&check, &request, purpose, &started).unwrap();
            assert_eq!(strictly_read(&report), report);
            assert_eq!(report.outcome, expected);
            assert_eq!(report.purpose, purpose);
            assert_eq!(
                report.failing_rule.as_ref().map(|r| r.as_str()),
                failing_rule
            );
            assert_eq!(report.rule_verdicts.len(), Rule::ALL.len());
            assert_eq!(report.proof_id.as_str(), check.proof_id);
            assert_eq!(
                report.theory_hash.as_str(),
                format!("sha256:{THEORY_SHA256}")
            );
            assert_eq!(report.context_digest.as_str(), check.context_digest);
            assert_eq!(
                report.action_digest.as_str(),
                format!("sha256:{}", common::DIGEST)
            );
            assert_eq!(report.elapsed_ms.get(), 457);
            assert_eq!(report.isabelle_version.as_str(), "Isabelle2025-2");
        }
    }
}

#[test]
fn a_ruling_is_given_for_block_and_need_approval_only() {
    let context = common::context();
    let blocked = check_of(&context, &[Rule::ResultHeld], Outcome::Block, true);
    let ruling_of_block = ruling(&blocked).unwrap().unwrap();
    assert_eq!(ruling_of_block.rule.as_str(), "result_held");
    assert_eq!(
        ruling_of_block.explanation.as_str(),
        Rule::ResultHeld.explanation()
    );
    assert_eq!(ruling_of_block.proof_id.as_str(), blocked.proof_id);
    let value = serde_json::to_value(&ruling_of_block).unwrap();
    let read: fabric_desktop_contracts::owner::Ruling =
        fabric_desktop_contracts::typed::from_value(&value).unwrap();
    assert_eq!(read, ruling_of_block);

    let asked = check_of(&context, &[Rule::NoStrangers], Outcome::NeedApproval, true);
    assert_eq!(
        ruling(&asked).unwrap().unwrap().explanation.as_str(),
        "Someone this change reaches is not in any recipient set you approved."
    );
    assert_eq!(
        ruling(&check_of(&context, &[], Outcome::Permit, true)).unwrap(),
        None
    );
    let undecided = check_of(&context, &[Rule::Actor], Outcome::Block, false);
    assert!(matches!(undecided.verdict, Verdict::Undecided { .. }));
    assert_eq!(ruling(&undecided).unwrap(), None);
}

fn tool_call(tool: &str, arguments: serde_json::Value) -> agent::Action {
    fabric_desktop_contracts::typed::from_value(&json!({"tool": tool, "arguments": arguments}))
        .unwrap()
}

#[test]
fn tool_calls_become_broker_actions() {
    let s = |v: &str| v.to_owned();
    let read = tool_call(
        "fabric_calendar_read",
        json!({
            "account_id": "acct-1",
            "calendar_ids": ["primary", "team"],
            "window": {"start": "1970-01-01T00:00:01.000Z", "end": "1970-01-02T00:00:00.000Z"},
            "fields": ["title", "own_response"],
            "task": "Plan the week."
        }),
    );
    assert_eq!(
        Action::from_tool_call(&read).unwrap(),
        Action::ReadEvents {
            account: s("acct-1"),
            calendars: vec![s("primary"), s("team")],
            start_ms: 1000,
            end_ms: 86_400_000,
            fields: vec![
                fabric_broker_policy::EventField::Title,
                fabric_broker_policy::EventField::OwnResponse
            ],
        }
    );

    let respond = tool_call(
        "fabric_calendar_propose_response",
        json!({
            "account_id": "acct-1",
            "calendar_id": "primary",
            "target": {"kind": "occurrence", "occurrence": {
                "instance_id": "evt_20261120T160000Z",
                "original_start_time": {"date_time": "2026-11-20T10:00:00-06:00", "zone": "America/Chicago"},
                "revision": "3181161784712000"
            }},
            "response": "declined",
            "send_updates": "external_only",
            "task": "Decline."
        }),
    );
    // 2026-11-20T16:00:00Z.
    assert_eq!(
        Action::from_tool_call(&respond).unwrap(),
        Action::Respond {
            account: s("acct-1"),
            calendar: s("primary"),
            target: Target::Occurrence {
                instance_id: s("evt_20261120T160000Z"),
                original_start_ms: 1_795_190_400_000,
                revision: s("3181161784712000"),
            },
            response: fabric_broker_policy::Response::Declined,
            send_updates: fabric_broker_policy::SendUpdates::NotifyExternal,
        }
    );

    let invite = tool_call(
        "fabric_calendar_propose_invitation",
        json!({
            "account_id": "acct-1",
            "calendar_id": "team",
            "event": {"event_id": "evt-1", "revision": "7"},
            "attendees": ["Bob.Smith@example.com", "carol@sub.example.org"],
            "send_updates": "all",
            "task": "Invite."
        }),
    );
    assert_eq!(
        Action::from_tool_call(&invite).unwrap(),
        Action::ChangeInvitation {
            account: s("acct-1"),
            calendar: s("team"),
            event_id: s("evt-1"),
            revision: s("7"),
            attendees: vec![
                common::recipient("Bob.Smith@example.com", "example.com"),
                common::recipient("carol@sub.example.org", "sub.example.org"),
            ],
            send_updates: fabric_broker_policy::SendUpdates::NotifyAll,
        }
    );

    let list = tool_call("fabric_calendar_list", json!({}));
    assert_eq!(
        Action::from_tool_call(&list).unwrap(),
        Action::ListCalendars
    );
    assert_eq!(
        Action::from_tool_call(&list).unwrap().disclosed(),
        Action::Disclose(Box::new(Action::ListCalendars))
    );

    let result = tool_call("fabric_request_result", json!({"request_id": "request-1"}));
    assert!(Action::from_tool_call(&result).is_err());
}

#[test]
fn an_original_start_time_is_converted_with_its_offset() {
    let respond = |date_time: &str| {
        let call = tool_call(
            "fabric_calendar_propose_response",
            json!({
                "account_id": "a", "calendar_id": "c",
                "target": {"kind": "occurrence", "occurrence": {
                    "instance_id": "i",
                    "original_start_time": {"date_time": date_time, "zone": "UTC"},
                    "revision": "r"
                }},
                "response": "accepted", "send_updates": "none", "task": ""
            }),
        );
        match Action::from_tool_call(&call) {
            Ok(Action::Respond {
                target:
                    Target::Occurrence {
                        original_start_ms, ..
                    },
                ..
            }) => Ok(original_start_ms),
            Ok(other) => panic!("{other:?}"),
            Err(error) => Err(error),
        }
    };
    assert_eq!(respond("1970-01-01T00:00:00+00:00"), Ok(0));
    assert_eq!(respond("1970-01-01T05:30:00+05:30"), Ok(0));
    assert_eq!(respond("1969-12-31T23:00:00-01:00"), Ok(0));
    assert_eq!(respond("1970-01-01T00:00:00+01:00"), Ok(-3_600_000));
    assert!(respond("2026-02-30T00:00:00+00:00").is_err());
}

#[test]
fn a_recipients_domain_is_after_the_last_at_sign() {
    let bob = Recipient::from_email(&Email::new("Bob@mail.example.com").unwrap());
    assert_eq!(bob.address, "Bob@mail.example.com");
    assert_eq!(bob.domain, "mail.example.com");
}
