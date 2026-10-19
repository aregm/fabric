//! Negotiations through live Isabelle, with scripted planners (tasks T6.1 and T7.2). They need
//! `scripts/setup-demo.ps1`, so they are ignored by default:
//!
//!   .\scripts\run.ps1 cargo test -p fabric-demo --test live_negotiation -- --ignored --nocapture

mod live;

use std::time::Duration;

use fabric_demo::agent::{Faults, Outcome};
use fabric_policy::time::format_utc;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Isabelle from scripts/setup-demo.ps1; run with --ignored"]
async fn scripted_agents_reach_the_mutual_options_through_isabelle() {
    let demo = live::Demo::start("live-mutual").await;
    demo.file_requests();
    let mut alice = demo
        .alice
        .agent(
            live::scripted(),
            demo.audit("alice-agent"),
            demo.clock.clone(),
        )
        .await;
    let mut bob = demo
        .bob
        .agent(
            live::scripted(),
            demo.audit("bob-agent"),
            demo.clock.clone(),
        )
        .await;
    let reached = live::run_until(&mut alice, &mut bob, Duration::from_secs(240), || {
        !demo.alice.host.agent_state().negotiation.mutual.is_empty()
            && !demo.bob.host.agent_state().negotiation.mutual.is_empty()
    })
    .await;
    assert!(reached, "both sides know the mutual options");

    let negotiation = demo.alice.host.agent_state().negotiation;
    let proposed: Vec<String> = negotiation
        .candidates
        .iter()
        .map(|candidate| format_utc(candidate.start_ms))
        .collect();
    assert_eq!(
        proposed,
        [
            "2027-01-12T14:00Z",
            "2027-01-13T15:00Z",
            "2027-01-14T14:30Z"
        ]
    );
    assert_eq!(negotiation.mutual, ["c1-1", "c1-3"]);
    assert_eq!(
        demo.bob.host.agent_state().negotiation.mutual,
        ["c1-1", "c1-3"]
    );

    // The agents only ever saw free/busy: no title from either calendar is in their memory.
    for (person, agent) in [(&demo.alice, &alice.memory()), (&demo.bob, &bob.memory())] {
        let memory = format!("{:?} {:?}", agent.free_busy, agent.past_actions);
        for event in person.owner_events() {
            assert!(!memory.contains(&event.title), "{} leaked", event.title);
        }
    }
}

/// Task T7.2: the section 14 happy path, with scripted planners and live Isabelle.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Isabelle from scripts/setup-demo.ps1; run with --ignored"]
async fn scripted_agents_book_the_meeting_both_owners_accepted() {
    let demo = live::Demo::start("live-meeting").await;
    demo.file_requests();
    let mut alice = demo
        .alice
        .agent(
            live::scripted(),
            demo.audit("alice-agent"),
            demo.clock.clone(),
        )
        .await;
    let mut bob = demo
        .bob
        .agent(
            live::scripted(),
            demo.audit("bob-agent"),
            demo.clock.clone(),
        )
        .await;
    let mutual = live::run_until(&mut alice, &mut bob, Duration::from_secs(240), || {
        !demo.alice.host.agent_state().negotiation.mutual.is_empty()
            && !demo.bob.host.agent_state().negotiation.mutual.is_empty()
    })
    .await;
    assert!(mutual);

    // The rehearsal script: Bob says no to the first option, and both say yes to the second.
    demo.alice.host.consent("c1-1", true).unwrap();
    demo.alice.host.consent("c1-3", true).unwrap();
    demo.bob.host.consent("c1-1", false).unwrap();
    demo.bob.host.consent("c1-3", true).unwrap();

    let booked = |person: &live::Person, title: &str| {
        person
            .owner_events()
            .into_iter()
            .any(|event| event.title == title && format_utc(event.start_ms) == "2027-01-14T14:30Z")
    };
    let done = live::run_until(&mut alice, &mut bob, Duration::from_secs(240), || {
        booked(&demo.alice, "Meeting with Bob (Fabric)")
            && booked(&demo.bob, "Meeting with Alice (Fabric)")
            && demo.alice.host.agent_state().negotiation.closed.is_some()
            && demo.bob.host.agent_state().negotiation.closed.is_some()
    })
    .await;
    assert!(
        done,
        "both calendars have the meeting and both sides closed"
    );

    for person in [&demo.alice, &demo.bob] {
        let meetings: Vec<_> = person
            .owner_events()
            .into_iter()
            .filter(|event| event.added_by.is_some())
            .collect();
        assert_eq!(
            meetings.len(),
            1,
            "{} has one agent-added meeting",
            person.name
        );
        assert_eq!(meetings[0].length_ms, 30 * 60 * 1_000);
        let added_by = meetings[0].added_by.as_ref().unwrap();
        assert_eq!(
            added_by.proof.len(),
            64,
            "the token carried an Isabelle proof ID"
        );
    }
    assert_eq!(
        demo.alice.host.agent_state().negotiation.closed,
        Some(fabric_policy::CloseReason::Agreed)
    );

    // Every executed action was authorized by a permit with a token (acceptance criterion A2).
    for name in ["alice", "bob"] {
        let log =
            std::fs::read_to_string(demo.directory.join(format!("{name}-host.jsonl"))).unwrap();
        let authorizations: Vec<serde_json::Value> = log
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .filter(|record: &serde_json::Value| record["kind"] == "authorize")
            .collect();
        assert!(
            authorizations.len() >= 5,
            "{name}: {} authorizations",
            authorizations.len()
        );
        for record in &authorizations {
            if record["token"].is_object() {
                assert_eq!(record["verdicts"][0]["verdict"]["verdict"], "permit");
                assert_eq!(record["token"]["proof"], record["proof_id"]);
            }
        }
    }
}

fn records(demo: &live::Demo, name: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(demo.directory.join(format!("{name}.jsonl")))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// The rules Isabelle denied, when screening and when authorizing, from an agent's audit log.
fn denied_rules(log: &[serde_json::Value]) -> Vec<String> {
    let screening = log
        .iter()
        .filter(|record| record["kind"] == "plans")
        .flat_map(|record| {
            record["screen"]["verdicts"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        });
    let authorizing = log
        .iter()
        .filter(|record| record["kind"] == "authorize")
        .map(|record| record["verdict"].clone());
    screening
        .chain(authorizing)
        .flat_map(|verdict| verdict["denials"].as_array().cloned().unwrap_or_default())
        .filter_map(|denial| denial["rule"].as_str().map(str::to_owned))
        .collect()
}

/// The `fault` records: which switch fired, and where or with what refusal.
fn fired(log: &[serde_json::Value]) -> Vec<(String, String)> {
    log.iter()
        .filter(|record| record["kind"] == "fault")
        .map(|record| {
            let result = record["stage"].as_str().or(record["outcome"].as_str());
            (
                record["fault"].as_str().unwrap_or_default().to_owned(),
                result.unwrap_or("not refused").to_owned(),
            )
        })
        .collect()
}

/// Acceptance criterion A3: every fault switch is denied by Isabelle or refused by a service,
/// and the run still books the meeting both owners accepted.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Isabelle from scripts/setup-demo.ps1; run with --ignored"]
async fn every_fault_switch_is_refused_and_the_run_still_books_the_meeting() {
    let demo = live::Demo::start("live-faults").await;
    demo.file_requests();
    let mut alice = demo
        .alice
        .agent(
            live::scripted(),
            demo.audit("alice-agent"),
            demo.clock.clone(),
        )
        .await;
    let mut bob = demo
        .bob
        .agent(
            live::scripted(),
            demo.audit("bob-agent"),
            demo.clock.clone(),
        )
        .await;
    // Each switch waits for its action: Alice's first read and her proposal, and Bob's first
    // calendar action.
    alice.inject(Faults {
        over_broad_read: true,
        too_many_candidates: true,
        ..Faults::default()
    });
    bob.inject(Faults {
        token_for_different_slot: true,
        ..Faults::default()
    });
    let mutual = live::run_until(&mut alice, &mut bob, Duration::from_secs(300), || {
        !demo.alice.host.agent_state().negotiation.mutual.is_empty()
            && !demo.bob.host.agent_state().negotiation.mutual.is_empty()
    })
    .await;
    assert!(mutual, "the negotiation reached the mutual options");
    assert_eq!(
        demo.alice.host.agent_state().negotiation.mutual,
        ["c1-1", "c1-3"]
    );

    // Both wait for their owners' decisions: Alice's decision, and Bob's next request.
    alice.inject(Faults {
        write_without_consent: true,
        ..Faults::default()
    });
    bob.inject(Faults {
        replay_token: true,
        ..Faults::default()
    });
    demo.alice.host.consent("c1-1", true).unwrap();
    demo.alice.host.consent("c1-3", true).unwrap();
    demo.bob.host.consent("c1-1", false).unwrap();
    demo.bob.host.consent("c1-3", true).unwrap();
    let booked = |person: &live::Person| {
        person.owner_events().into_iter().any(|event| {
            event.added_by.is_some() && format_utc(event.start_ms) == "2027-01-14T14:30Z"
        })
    };
    let done = live::run_until(&mut alice, &mut bob, Duration::from_secs(300), || {
        booked(&demo.alice)
            && booked(&demo.bob)
            && demo.alice.host.agent_state().negotiation.closed.is_some()
            && demo.bob.host.agent_state().negotiation.closed.is_some()
    })
    .await;
    assert!(done, "the run recovered and booked the meeting");
    assert_eq!(
        alice.pending_faults(),
        Faults::default(),
        "every switch fired"
    );
    assert_eq!(
        bob.pending_faults(),
        Faults::default(),
        "every switch fired"
    );

    let alice_log = records(&demo, "alice-agent");
    let bob_log = records(&demo, "bob-agent");
    assert_eq!(
        fired(&alice_log),
        [
            ("over_broad_read".to_owned(), "screening".to_owned()),
            ("too_many_candidates".to_owned(), "screening".to_owned()),
            ("write_without_consent".to_owned(), "authorize".to_owned()),
        ]
    );
    assert_eq!(
        fired(&bob_log),
        [
            (
                "token_for_different_slot".to_owned(),
                "token_argument_mismatch".to_owned()
            ),
            ("replay_token".to_owned(), "token_replayed".to_owned()),
        ]
    );
    let denied = denied_rules(&alice_log);
    for rule in ["view_details_never", "read_window", "cand_count", "consent"] {
        assert!(
            denied.iter().any(|denied| denied == rule),
            "{rule} in {denied:?}"
        );
    }
    assert!(
        denied_rules(&bob_log).is_empty(),
        "Bob's switches never reach Isabelle"
    );

    // The services logged the misuse they refused.
    let refused = |name: &str, error: &str| {
        records(&demo, name)
            .iter()
            .any(|record| record["outcome"] == "refused" && record["error"] == error)
    };
    assert!(refused("bob-calendar", "token_argument_mismatch"));
    assert!(refused("bob-gateway", "token_replayed"));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Isabelle from scripts/setup-demo.ps1; run with --ignored"]
async fn isabelle_denies_an_over_broad_read_and_the_agent_recovers() {
    let demo = live::Demo::start("live-broad-read").await;
    demo.file_requests();
    let mut alice = demo
        .alice
        .agent(
            live::scripted(),
            demo.audit("alice-agent"),
            demo.clock.clone(),
        )
        .await;
    alice.inject(Faults {
        over_broad_read: true,
        ..Faults::default()
    });
    assert_eq!(alice.step(0).await.unwrap(), Some(Outcome::ReadMore));
    let denials = format!("{:?}", alice.memory().denials);
    assert!(denials.contains("view_details_never"), "{denials}");
    assert!(denials.contains("read_window"), "{denials}");
    assert!(
        alice.memory().free_busy.is_some(),
        "the re-plan read the window"
    );
}
