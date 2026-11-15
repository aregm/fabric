//! Tokens end to end (task T3.2, acceptance criterion A6): minted only from an authorize
//! permit, accepted by one service, bound to the exact action and to the agent's key, expired
//! after 60 seconds, and usable once.

use ed25519_dalek::SigningKey;
use fabric_policy::time::parse_utc;
use fabric_policy::{
    Action, ActionVerdict, Answer, Candidate, CheckKind, CheckReport, CloseReason, Decision,
    Denial, Rule, Verdict,
};
use fabric_token::cbor::{self, CborError, Value, int};
use fabric_token::{
    Accepted, BindingKey, LIFETIME_SECONDS, Minter, POP_MAX_AGE_MS, POP_MAX_LEAD_MS, Presentation,
    Reason, Rejection, ReplayCache, Service, TokenError, Verifier, action_digest, hex,
    read_unverified,
};

const PROOF_ID: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0";
const HALF_HOUR: i64 = 30 * 60 * 1_000;

fn at(text: &str) -> i64 {
    parse_utc(text).expect("test times are valid")
}

fn report(kind: CheckKind, verdicts: Vec<(Action, Verdict)>) -> CheckReport {
    CheckReport {
        kind,
        theory: "Check_000042".to_owned(),
        proof_id: PROOF_ID.to_owned(),
        policy_sha256: "00".repeat(32),
        isabelle: "Isabelle2025-2".to_owned(),
        seconds: 1.0,
        verdicts: verdicts
            .into_iter()
            .map(|(action, verdict)| ActionVerdict {
                action,
                verdict,
                rules: Vec::new(),
            })
            .collect(),
        theory_text: String::new(),
    }
}

fn permit(action: &Action) -> CheckReport {
    report(
        CheckKind::Authorize,
        vec![(action.clone(), Verdict::Permit)],
    )
}

fn meeting(start: &str) -> Action {
    Action::AddMeeting {
        negotiation: "n1".to_owned(),
        start_ms: at(start),
        length_ms: HALF_HOUR,
    }
}

fn read(start: &str, end: &str) -> Action {
    Action::ReadFreeBusy {
        start_ms: at(start),
        end_ms: at(end),
    }
}

fn propose(start: &str, note: &str) -> Action {
    Action::SendPropose {
        negotiation: "n1".to_owned(),
        round: 1,
        candidates: vec![Candidate {
            id: "c1".to_owned(),
            start_ms: at(start),
            length_ms: HALF_HOUR,
        }],
        note: note.to_owned(),
    }
}

/// The request body a service receives for `action`.
fn body(action: &Action) -> Vec<u8> {
    serde_json::to_vec(action).expect("actions serialize")
}

fn reason<T: std::fmt::Debug>(result: Result<T, Rejection>) -> Reason {
    result.expect_err("the service refuses").reason
}

/// Alice's gate, her agent, and her two services.
struct Demo {
    gate: Minter,
    agent: BindingKey,
    calendar: Verifier,
    gateway: Verifier,
    replay: ReplayCache,
    now: i64,
}

impl Demo {
    fn new() -> Self {
        let gate = SigningKey::from_bytes(&[7; 32]);
        let key = gate.verifying_key();
        Self {
            gate: Minter::new("alice", gate).unwrap(),
            agent: BindingKey::from_bytes(&[9; 32]),
            calendar: Verifier::new(Service::Calendar, "alice", key),
            gateway: Verifier::new(Service::Gateway, "alice", key),
            replay: ReplayCache::new(),
            now: at("2027-01-11T08:00:00Z"),
        }
    }

    fn mint(&self, action: &Action) -> Vec<u8> {
        self.gate
            .mint(&permit(action), &self.agent.verifying_key(), self.now)
            .unwrap()
            .bytes
    }

    /// The agent presents `token` for `action` at `when`, and `service` checks it then.
    fn send(
        &self,
        service: &Verifier,
        token: &[u8],
        action: &Action,
        when: i64,
    ) -> Result<Accepted, Rejection> {
        let body = body(action);
        let presentation = self.agent.present(token, &body, when).unwrap();
        service.check(&presentation, action, &body, when, &self.replay)
    }
}

#[test]
fn a_permitted_action_gets_a_token_its_service_accepts_once() {
    let demo = Demo::new();
    let action = meeting("2027-01-12T14:00Z");
    let token = demo.mint(&action);

    let accepted = demo
        .send(&demo.calendar, &token, &action, demo.now + 1_000)
        .unwrap();
    assert_eq!(accepted.claims.action, "add_meeting");
    accepted.commit();

    let again = demo.send(&demo.calendar, &token, &action, demo.now + 2_000);
    assert_eq!(reason(again), Reason::TokenReplayed);
}

#[test]
fn the_claims_name_the_service_the_action_the_agent_and_the_proof() {
    let demo = Demo::new();
    let action = meeting("2027-01-12T14:00Z");
    let minted = demo
        .gate
        .mint(
            &permit(&action),
            &demo.agent.verifying_key(),
            demo.now + 500,
        )
        .unwrap();
    let claims = read_unverified(&minted.bytes).unwrap();
    assert_eq!(claims, minted.claims);
    assert_eq!(claims.issuer, "gate:alice");
    assert_eq!(claims.audience, "calendar:alice");
    assert_eq!(claims.holder, demo.agent.verifying_key().to_bytes());
    assert_eq!(claims.issued_at, demo.now / 1_000);
    assert_eq!(claims.expires_at - claims.issued_at, LIFETIME_SECONDS);
    assert_eq!(claims.action, "add_meeting");
    assert_eq!(claims.args, action_digest(&action).unwrap());
    assert_eq!(claims.negotiation.as_deref(), Some("n1"));
    assert_eq!(hex::encode(&claims.proof), PROOF_ID);

    let json = serde_json::to_value(&claims).unwrap();
    assert_eq!(json["proof"], PROOF_ID);
    assert_eq!(json["nonce"], hex::encode(&claims.nonce));

    let other = demo
        .gate
        .mint(
            &permit(&action),
            &demo.agent.verifying_key(),
            demo.now + 500,
        )
        .unwrap();
    assert_ne!(
        other.claims.nonce, claims.nonce,
        "every token has its own nonce"
    );

    let handshake = Action::Handshake {
        peer: "bob".to_owned(),
    };
    let claims = read_unverified(&demo.mint(&handshake)).unwrap();
    assert_eq!(claims.audience, "gateway:alice");
    assert_eq!(claims.negotiation, None);
}

#[test]
fn tokens_come_only_from_an_authorize_check_that_permitted_one_action() {
    let demo = Demo::new();
    let action = meeting("2027-01-12T14:00Z");
    let other = meeting("2027-01-13T15:00Z");
    let refused = [
        report(CheckKind::Screen, vec![(action.clone(), Verdict::Permit)]),
        report(
            CheckKind::Authorize,
            vec![(
                action.clone(),
                Verdict::Deny {
                    denials: vec![Denial {
                        rule: Rule::Consent,
                        detail: "no valid yes from bob".to_owned(),
                    }],
                },
            )],
        ),
        report(
            CheckKind::Authorize,
            vec![(
                action.clone(),
                Verdict::Undecided {
                    reason: "the check timed out".to_owned(),
                },
            )],
        ),
        report(
            CheckKind::Authorize,
            vec![(action, Verdict::Permit), (other, Verdict::Permit)],
        ),
        report(CheckKind::Authorize, Vec::new()),
        // Isabelle never permits this one; even if it did, no service executes it.
        report(
            CheckKind::Authorize,
            vec![(
                Action::ReadEventDetails {
                    start_ms: 0,
                    end_ms: 1,
                },
                Verdict::Permit,
            )],
        ),
    ];
    for report in refused {
        let result = demo
            .gate
            .mint(&report, &demo.agent.verifying_key(), demo.now);
        assert!(
            matches!(result, Err(TokenError::Mint(_))),
            "{:?} gave {result:?}",
            report.verdicts
        );
    }
}

#[test]
fn each_action_goes_to_its_own_service() {
    let demo = Demo::new();
    let respond = Action::SendRespond {
        negotiation: "n1".to_owned(),
        round: 1,
        answers: vec![Answer {
            candidate_id: "c1".to_owned(),
            available: true,
        }],
        note: String::new(),
    };
    let decide = Action::SendDecide {
        negotiation: "n1".to_owned(),
        round: 1,
        decisions: vec![Decision {
            candidate_id: "c1".to_owned(),
            accept: true,
        }],
        note: String::new(),
    };
    let close = Action::SendClose {
        negotiation: "n1".to_owned(),
        reason: CloseReason::Agreed,
        note: "see you then".to_owned(),
    };
    let cases = [
        (
            read("2027-01-11T08:00Z", "2027-01-18T08:00Z"),
            Service::Calendar,
        ),
        (meeting("2027-01-12T14:00Z"), Service::Calendar),
        (
            Action::Handshake {
                peer: "bob".to_owned(),
            },
            Service::Gateway,
        ),
        (
            propose("2027-01-12T14:00Z", "afternoons suit me"),
            Service::Gateway,
        ),
        (respond, Service::Gateway),
        (decide, Service::Gateway),
        (close, Service::Gateway),
    ];
    for (action, service) in cases {
        assert_eq!(Service::for_action(&action), Some(service));
        let token = demo.mint(&action);
        let (right, wrong) = match service {
            Service::Calendar => (&demo.calendar, &demo.gateway),
            Service::Gateway => (&demo.gateway, &demo.calendar),
        };
        let refused = demo.send(wrong, &token, &action, demo.now);
        assert_eq!(reason(refused), Reason::WrongAudience, "{}", action.kind());
        demo.send(right, &token, &action, demo.now)
            .unwrap_or_else(|error| panic!("{}: {error}", action.kind()))
            .commit();
    }
}

#[test]
fn only_the_persons_own_gate_can_mint_for_their_services() {
    let demo = Demo::new();
    let action = meeting("2027-01-12T14:00Z");
    let bob = Minter::new("bob", SigningKey::from_bytes(&[8; 32])).unwrap();
    let token = bob
        .mint(&permit(&action), &demo.agent.verifying_key(), demo.now)
        .unwrap()
        .bytes;
    let at_alice = demo.send(&demo.calendar, &token, &action, demo.now);
    assert_eq!(reason(at_alice), Reason::BadSignature);

    // Alice's key configured under Bob's name: the signature verifies, but the issuer is wrong.
    let misconfigured = Verifier::new(Service::Calendar, "bob", demo.gate.verifying_key());
    let token = demo.mint(&action);
    let refused = demo.send(&misconfigured, &token, &action, demo.now);
    assert_eq!(reason(refused), Reason::BadSignature);

    let at_bob = Verifier::new(Service::Calendar, "bob", bob.verifying_key());
    let token = bob
        .mint(&permit(&action), &demo.agent.verifying_key(), demo.now)
        .unwrap()
        .bytes;
    demo.send(&at_bob, &token, &action, demo.now).unwrap();
}

#[test]
fn a_token_is_bound_to_its_exact_arguments() {
    let demo = Demo::new();
    let action = meeting("2027-01-12T14:00Z");
    let token = demo.mint(&action);
    let changed = [
        // The "token for a different slot" fault switch (section 14).
        meeting("2027-01-12T14:15Z"),
        Action::AddMeeting {
            negotiation: "n1".to_owned(),
            start_ms: at("2027-01-12T14:00Z"),
            length_ms: 2 * HALF_HOUR,
        },
        Action::AddMeeting {
            negotiation: "n2".to_owned(),
            start_ms: at("2027-01-12T14:00Z"),
            length_ms: HALF_HOUR,
        },
    ];
    for other in &changed {
        let refused = demo.send(&demo.calendar, &token, other, demo.now);
        assert_eq!(reason(refused), Reason::TokenArgumentMismatch, "{other:?}");
    }
    // Refused attempts do not use up the token.
    demo.send(&demo.calendar, &token, &action, demo.now)
        .unwrap()
        .commit();

    let proposal = propose("2027-01-12T14:00Z", "afternoons suit me");
    let token = demo.mint(&proposal);
    let changed = [
        propose("2027-01-12T14:00Z", "mornings suit me"),
        propose("2027-01-12T14:30Z", "afternoons suit me"),
        Action::SendPropose {
            negotiation: "n1".to_owned(),
            round: 2,
            candidates: vec![Candidate {
                id: "c1".to_owned(),
                start_ms: at("2027-01-12T14:00Z"),
                length_ms: HALF_HOUR,
            }],
            note: "afternoons suit me".to_owned(),
        },
    ];
    for other in &changed {
        let refused = demo.send(&demo.gateway, &token, other, demo.now);
        assert_eq!(reason(refused), Reason::TokenArgumentMismatch, "{other:?}");
    }

    // The same window under another action kind.
    let window = read("2027-01-11T08:00Z", "2027-01-18T08:00Z");
    let token = demo.mint(&window);
    let details = Action::ReadEventDetails {
        start_ms: at("2027-01-11T08:00Z"),
        end_ms: at("2027-01-18T08:00Z"),
    };
    let refused = demo.send(&demo.calendar, &token, &details, demo.now);
    assert_eq!(reason(refused), Reason::TokenArgumentMismatch);
}

#[test]
fn a_token_is_bound_to_the_agent_that_asked_for_it() {
    let demo = Demo::new();
    let action = meeting("2027-01-12T14:00Z");
    let token = demo.mint(&action);
    let body = body(&action);

    let thief = BindingKey::from_bytes(&[10; 32]);
    let stolen = thief.present(&token, &body, demo.now).unwrap();
    let refused = demo
        .calendar
        .check(&stolen, &action, &body, demo.now, &demo.replay);
    assert_eq!(reason(refused), Reason::PopInvalid);

    let signed = demo.agent.present(&token, &body, demo.now).unwrap();
    let elsewhere = meeting("2027-01-12T14:15Z");
    let refused = demo.calendar.check(
        &signed,
        &elsewhere,
        &self::body(&elsewhere),
        demo.now,
        &demo.replay,
    );
    assert_eq!(
        reason(refused),
        Reason::PopInvalid,
        "signed over another body"
    );

    let mut redated = signed.clone();
    redated.timestamp_ms += 1;
    let refused = demo
        .calendar
        .check(&redated, &action, &body, demo.now, &demo.replay);
    assert_eq!(reason(refused), Reason::PopInvalid, "timestamp changed");

    let late = demo.now + POP_MAX_AGE_MS + 1;
    let refused = demo
        .calendar
        .check(&signed, &action, &body, late, &demo.replay);
    assert_eq!(reason(refused), Reason::PopInvalid, "too old");

    let early = demo.now - POP_MAX_LEAD_MS - 1;
    let refused = demo
        .calendar
        .check(&signed, &action, &body, early, &demo.replay);
    assert_eq!(reason(refused), Reason::PopInvalid, "dated in the future");

    demo.calendar
        .check(
            &signed,
            &action,
            &body,
            demo.now + POP_MAX_AGE_MS,
            &demo.replay,
        )
        .expect("the oldest proof of possession still accepted");
}

#[test]
fn tokens_expire_sixty_seconds_after_issue() {
    let demo = Demo::new();
    let action = meeting("2027-01-12T14:00Z");
    let minted = demo
        .gate
        .mint(&permit(&action), &demo.agent.verifying_key(), demo.now)
        .unwrap();
    let expiry = minted.claims.expires_at * 1_000;
    assert_eq!(expiry, demo.now + 60_000);

    let refused = demo.send(&demo.calendar, &minted.bytes, &action, expiry);
    assert_eq!(reason(refused), Reason::Expired);
    demo.send(&demo.calendar, &minted.bytes, &action, expiry - 1)
        .expect("valid until the last millisecond");
}

#[test]
fn concurrent_requests_with_one_token_admit_one() {
    let demo = Demo::new();
    let action = meeting("2027-01-12T14:00Z");
    let token = demo.mint(&action);

    let first = demo
        .send(&demo.calendar, &token, &action, demo.now)
        .unwrap();
    let second = demo.send(&demo.calendar, &token, &action, demo.now + 10);
    assert_eq!(
        reason(second),
        Reason::TokenReplayed,
        "while the first runs"
    );

    // The first request failed to execute, so its nonce is released for a retry.
    drop(first);
    demo.send(&demo.calendar, &token, &action, demo.now + 20)
        .unwrap()
        .commit();
    let after = demo.send(&demo.calendar, &token, &action, demo.now + 30);
    assert_eq!(reason(after), Reason::TokenReplayed);
}

#[test]
fn the_replay_cache_forgets_nonces_once_their_tokens_expire() {
    let demo = Demo::new();
    let first = meeting("2027-01-12T14:00Z");
    let second = meeting("2027-01-13T15:00Z");
    let first_token = demo.mint(&first);
    for (action, token) in [(&first, &first_token), (&second, &demo.mint(&second))] {
        demo.send(&demo.calendar, token, action, demo.now)
            .unwrap()
            .commit();
    }
    assert_eq!(demo.replay.len(), 2);

    let later = Demo {
        now: demo.now + (LIFETIME_SECONDS + 1) * 1_000,
        ..Demo::new()
    };
    let third = meeting("2027-01-14T14:30Z");
    let token = later.mint(&third);
    later
        .send(&demo.calendar, &token, &third, later.now)
        .unwrap()
        .commit();
    let refused = later.send(&demo.calendar, &first_token, &first, later.now);
    assert_eq!(reason(refused), Reason::Expired);

    // Share the first cache to see the purge: only the new token's nonce is left.
    let shared = Demo {
        replay: demo.replay.clone(),
        now: later.now,
        ..Demo::new()
    };
    shared
        .send(&shared.calendar, &shared.mint(&third), &third, shared.now)
        .unwrap()
        .commit();
    assert_eq!(demo.replay.len(), 1);

    // A service clock that runs behind cannot revive a forgotten nonce.
    let refused = shared.send(&shared.calendar, &first_token, &first, demo.now + 1_000);
    assert_eq!(reason(refused), Reason::Expired);
}

#[test]
fn tampered_or_re_encoded_tokens_are_refused() {
    let demo = Demo::new();
    let action = meeting("2027-01-12T14:00Z");
    let body = body(&action);
    let token = demo.mint(&action);
    // Returns the reason, after checking that the detail mentions `expected`.
    let check_with = |bytes: Vec<u8>, expected: &str| {
        let presentation = Presentation {
            token: bytes,
            timestamp_ms: demo.now,
            signature: [0; 64],
        };
        let rejection = demo
            .calendar
            .check(&presentation, &action, &body, demo.now, &demo.replay)
            .expect_err("the service refuses");
        assert!(
            rejection.detail.contains(expected),
            "{rejection} does not mention {expected:?}"
        );
        rejection.reason
    };
    let check = |bytes: Vec<u8>| check_with(bytes, "");

    for index in [0, 7, token.len() / 2, token.len() - 1] {
        let mut flipped = token.clone();
        flipped[index] ^= 0x01;
        assert_eq!(check(flipped), Reason::BadSignature, "byte {index}");
    }

    let Value::Tag(18, inner) = cbor::decode(&token).unwrap() else {
        panic!("a tagged COSE_Sign1");
    };
    let Value::Array(items) = *inner else {
        panic!("an array");
    };
    let rebuild = |items: Vec<Value>, tagged: bool| {
        let array = Value::Array(items);
        let value = if tagged {
            Value::Tag(18, Box::new(array))
        } else {
            array
        };
        cbor::encode(&value).unwrap()
    };

    // The signature does not cover the unprotected header, so it must be empty.
    let mut with_kid = items.clone();
    with_kid[1] = Value::Map(vec![(int(4), Value::Bytes(vec![1]))]);
    assert_eq!(
        check_with(rebuild(with_kid, true), "unprotected header must be empty"),
        Reason::BadSignature
    );

    // Algorithm EdDSA (-8) instead of Ed25519 (-19).
    let mut eddsa = items.clone();
    eddsa[0] = Value::Bytes(vec![0xa1, 0x01, 0x27]);
    assert_eq!(
        check_with(rebuild(eddsa, true), "protected header must be exactly"),
        Reason::BadSignature
    );

    assert_eq!(
        check_with(rebuild(items, false), "not a tagged COSE_Sign1"),
        Reason::BadSignature
    );

    let mut long_tag = vec![0xd8, 0x12];
    long_tag.extend_from_slice(&token[1..]);
    assert_eq!(
        check_with(long_tag, "not in deterministic form"),
        Reason::BadSignature
    );

    assert_eq!(check_with(Vec::new(), "malformed"), Reason::BadSignature);
    assert_eq!(
        check_with(vec![0; 5_000], "at most 4096"),
        Reason::BadSignature
    );

    // The untampered token still works, with a real proof of possession.
    demo.send(&demo.calendar, &token, &action, demo.now)
        .unwrap();
}

#[test]
fn presentations_travel_as_deterministic_cbor() {
    let demo = Demo::new();
    let action = meeting("2027-01-12T14:00Z");
    let token = demo.mint(&action);
    let presentation = demo
        .agent
        .present(&token, &body(&action), demo.now)
        .unwrap();
    let bytes = presentation.to_bytes().unwrap();
    assert_eq!(Presentation::from_bytes(&bytes).unwrap(), presentation);

    assert!(Presentation::from_bytes(&bytes[..bytes.len() - 1]).is_err());
    let mut long_head = vec![0x98, 0x03];
    long_head.extend_from_slice(&bytes[1..]);
    assert!(matches!(
        Presentation::from_bytes(&long_head),
        Err(TokenError::Cbor(CborError::NotDeterministic))
    ));
}

/// coset, an independent COSE implementation, reads the tokens as standard CWTs.
#[test]
fn tokens_are_standard_cose_sign1_cwts() {
    use coset::cwt::{ClaimName, ClaimsSet, Timestamp};
    use coset::{CborSerializable as _, CoseSign1, TaggedCborSerializable as _, iana};

    let demo = Demo::new();
    let action = meeting("2027-01-12T14:00Z");
    let minted = demo
        .gate
        .mint(&permit(&action), &demo.agent.verifying_key(), demo.now)
        .unwrap();

    let sign1 = CoseSign1::from_tagged_slice(&minted.bytes).unwrap();
    assert_eq!(
        sign1.protected.header.alg,
        Some(coset::Algorithm::Assigned(iana::Algorithm::Ed25519))
    );
    assert!(sign1.unprotected.is_empty());
    let gate = demo.gate.verifying_key();
    sign1
        .verify_signature(b"", |signature, data| {
            let signature = ed25519_dalek::Signature::from_slice(signature)?;
            gate.verify_strict(data, &signature)
        })
        .expect("coset's Sig_structure verifies");

    let claims = ClaimsSet::from_slice(sign1.payload.as_deref().unwrap()).unwrap();
    assert_eq!(claims.issuer.as_deref(), Some("gate:alice"));
    assert_eq!(claims.audience.as_deref(), Some("calendar:alice"));
    assert_eq!(
        claims.expiration_time,
        Some(Timestamp::WholeSeconds(minted.claims.expires_at))
    );
    assert_eq!(
        claims.issued_at,
        Some(Timestamp::WholeSeconds(minted.claims.issued_at))
    );
    assert_eq!(claims.cwt_id, Some(minted.claims.nonce.to_vec()));
    let names: Vec<&ClaimName> = claims.rest.iter().map(|(name, _)| name).collect();
    assert_eq!(
        names,
        [
            &ClaimName::Assigned(iana::CwtClaimName::Cnf),
            &ClaimName::PrivateUse(-65_537),
            &ClaimName::PrivateUse(-65_538),
            &ClaimName::PrivateUse(-65_539),
            &ClaimName::PrivateUse(-65_540),
        ]
    );
}
