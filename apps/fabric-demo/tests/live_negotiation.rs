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
