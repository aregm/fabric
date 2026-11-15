//! The `relay` and `host` commands end to end: two hosts whose keys live in the OS credential
//! store connect through the relay and exchange a proposal. The keys are created under the
//! test prefix `fabric-demo-test/` and deleted afterwards, so this test is local only:
//!
//!   .\scripts\run.ps1 cargo test -p fabric-demo --test messaging_commands -- --ignored
//!   sh scripts/run.sh cargo test -p fabric-demo --test messaging_commands -- --ignored
//!
//! On macOS the `fabric-demo host` processes read Keychain items this test binary created, so
//! macOS asks whether to allow it; the test waits for the clicks.

#![cfg(any(windows, target_os = "macos"))]

mod common;

use common::{at, permit, scratch, start, write_json};
use fabric_demo::clock::{Clock, ClockConfig, DemoClock, wall_now_ms};
use fabric_demo::host::{Event, HostClient};
use fabric_demo::relay::new_mailbox;
use fabric_policy::{Action, Candidate};
use fabric_token::{BindingKey, CredentialStore, KeyDirectory, PersonKeys, hex, random};
use serde_json::json;

const PREFIX: &str = "fabric-demo-test";

/// Deletes the test's keys even when an assertion fails.
struct Cleanup(CredentialStore);

impl Drop for Cleanup {
    fn drop(&mut self) {
        for person in ["alice", "bob"] {
            let _ = PersonKeys::delete(&self.0, person);
        }
    }
}

fn secret() -> String {
    hex::encode(&random::<32>().unwrap())
}

#[test]
#[ignore = "writes to the OS credential store and runs the commands; run with --ignored"]
fn two_hosts_connect_through_the_relay_and_exchange_a_proposal() {
    let directory = scratch("messaging-commands");
    let cleanup = Cleanup(CredentialStore::open(PREFIX).unwrap());
    let alice = PersonKeys::load_or_create(&cleanup.0, "alice").unwrap();
    let bob = PersonKeys::load_or_create(&cleanup.0, "bob").unwrap();
    let mut keys = KeyDirectory::new();
    keys.insert(&alice);
    keys.insert(&bob);
    keys.write(&directory.join("keys.json")).unwrap();

    let clock = ClockConfig {
        start: "2027-01-11T08:00:00Z".to_owned(),
        wall_start_ms: wall_now_ms(),
    };
    let demo = DemoClock::from_config(&clock).unwrap();
    let inboxes = [new_mailbox().unwrap(), new_mailbox().unwrap()];
    write_json(
        &directory.join("relay.json"),
        &json!({
            "listen": "127.0.0.1:0",
            "mailboxes": inboxes,
            "clock": clock,
            "audit_log": directory.join("relay.jsonl"),
        }),
    );
    let (_relay, relay_url) = start("relay", &directory.join("relay.json"), "relay");

    let secrets = [secret(), secret()];
    let mut hosts = Vec::new();
    for (index, (person, peer, role)) in
        [("alice", "bob", "coordinator"), ("bob", "alice", "invitee")]
            .into_iter()
            .enumerate()
    {
        let path = directory.join(format!("{person}-host.json"));
        write_json(
            &path,
            &json!({
                "person": person,
                "peer": peer,
                "role": role,
                "listen": "127.0.0.1:0",
                "keys": directory.join("keys.json"),
                "relay": relay_url,
                "inbox": inboxes[index],
                "peer_inbox": inboxes[1 - index],
                "agent_secret": secrets[index],
                "owner_secret": secret(),
                "clock": clock,
                "audit_log": directory.join(format!("{person}-host.jsonl")),
                "credential_prefix": PREFIX,
            }),
        );
        let (running, url) = start("host", &path, &format!("host:{person}"));
        hosts.push((running, HostClient::new(&url, &secrets[index])));
    }
    let (alice_host, bob_host) = (&hosts[0].1, &hosts[1].1);

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (alice_agent, bob_agent) = (
        BindingKey::generate().unwrap(),
        BindingKey::generate().unwrap(),
    );
    let mint = |keys: &PersonKeys, agent: &BindingKey, action: &Action| {
        keys.minter()
            .mint(&permit(action), &agent.verifying_key(), demo.now_ms())
            .unwrap()
            .bytes
    };
    runtime.block_on(async {
        let call = Action::Handshake {
            peer: "bob".to_owned(),
        };
        let reply = alice_host
            .handshake(&alice_agent, &mint(&alice, &alice_agent, &call), &call, demo.now_ms())
            .await
            .unwrap();
        assert_eq!(reply, json!({ "link": "calling" }));

        let ringing = bob_host.inbox(0, 5_000).await.unwrap();
        assert_eq!(
            ringing[0].event,
            Event::PeerCalling {
                peer: "alice".to_owned()
            }
        );
        let answer = Action::Handshake {
            peer: "alice".to_owned(),
        };
        let reply = bob_host
            .handshake(&bob_agent, &mint(&bob, &bob_agent, &answer), &answer, demo.now_ms())
            .await
            .unwrap();
        assert_eq!(reply, json!({ "link": "connected" }));
        let connected = alice_host.inbox(0, 5_000).await.unwrap();
        assert_eq!(
            connected[0].event,
            Event::SessionEstablished {
                peer: "bob".to_owned()
            }
        );

        let negotiation = alice_host
            .status()
            .await
            .unwrap()
            .negotiation
            .id
            .expect("the coordinator opened a negotiation");
        let proposal = Action::SendPropose {
            negotiation: negotiation.clone(),
            round: 1,
            candidates: vec![Candidate {
                id: "c1".to_owned(),
                start_ms: at("2027-01-12T14:00Z"),
                length_ms: 30 * 60 * 1_000,
            }],
            note: "afternoons suit me".to_owned(),
        };
        let reply = alice_host
            .send(&alice_agent, &mint(&alice, &alice_agent, &proposal), &proposal, demo.now_ms())
            .await
            .unwrap();
        assert_eq!(reply, json!({ "sent": "propose" }));

        let after = ringing.last().unwrap().seq;
        let arrived = bob_host.inbox(after + 1, 5_000).await.unwrap();
        assert!(
            arrived
                .iter()
                .any(|delivery| matches!(&delivery.event, Event::Message { message } if message.negotiation == negotiation)),
            "{arrived:?}"
        );
        let bob_status = bob_host.status().await.unwrap();
        assert_eq!(bob_status.link, "connected");
        assert_eq!(bob_status.negotiation.id, Some(negotiation));
        assert_eq!(bob_status.negotiation.round, 1);
    });
    drop(hosts);
    drop(cleanup);
    for person in ["alice", "bob"] {
        assert!(PersonKeys::load(&CredentialStore::open(PREFIX).unwrap(), person).is_err());
    }
}
