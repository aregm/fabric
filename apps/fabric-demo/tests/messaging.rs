//! Two gateways through a real relay on loopback (task T5.2): the Noise IK handshake with a
//! token on each side, the peer key check, signed messages that update both negotiations,
//! token checks on every send, and acceptance criterion A5, "the relay capture contains no
//! plaintext candidate times, notes or names".

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{Request, StatusCode};
use fabric_demo::api::authorization;
use fabric_demo::audit::AuditLog;
use fabric_demo::clock::{Clock, ManualClock};
use fabric_demo::host::{Event, Gateway, GatewayParts, Host, HostParts, router};
use fabric_demo::net::bind_loopback;
use fabric_demo::relay::{Relay, RelayClient, new_mailbox, serve};
use fabric_policy::time::parse_utc;
use fabric_policy::{
    Action, ActionVerdict, Answer, Candidate, CheckKind, CheckReport, CloseReason, Decision, Role,
    Verdict,
};
use fabric_token::{BindingKey, MemoryStore, Minter, PersonKeys, PublicKeys, SigningKey};
use fabric_wire::{Call, Message};
use serde_json::{Value, json};
use tower::ServiceExt as _;

const AGENT_SECRET: &str = "agent-secret-for-tests-0123456789abcdef";
const NEGOTIATION: &str = "n-test";
const HALF_HOUR: i64 = 30 * 60 * 1_000;

fn at(text: &str) -> i64 {
    parse_utc(text).expect("test times are valid")
}

fn scratch(test: &str) -> PathBuf {
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("messaging")
        .join(test);
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    directory
}

fn permit(action: &Action) -> CheckReport {
    CheckReport {
        kind: CheckKind::Authorize,
        theory: "Check_000005".to_owned(),
        proof_id: "ab".repeat(32),
        policy_sha256: "cd".repeat(32),
        isabelle: "Isabelle2025-2".to_owned(),
        seconds: 1.0,
        verdicts: vec![ActionVerdict {
            action: action.clone(),
            verdict: Verdict::Permit,
            rules: Vec::new(),
        }],
        theory_text: String::new(),
    }
}

fn handshake(peer: &str) -> Action {
    Action::Handshake {
        peer: peer.to_owned(),
    }
}

fn candidates() -> Vec<Candidate> {
    [
        ("slot-tuesday-afternoon", "2027-01-12T14:00Z"),
        ("slot-wednesday-afternoon", "2027-01-13T15:00Z"),
        ("slot-thursday-afternoon", "2027-01-14T14:30Z"),
    ]
    .into_iter()
    .map(|(id, start)| Candidate {
        id: id.to_owned(),
        start_ms: at(start),
        length_ms: HALF_HOUR,
    })
    .collect()
}

fn propose(note: &str) -> Action {
    Action::SendPropose {
        negotiation: NEGOTIATION.to_owned(),
        round: 1,
        candidates: candidates(),
        note: note.to_owned(),
    }
}

fn respond(note: &str) -> Action {
    Action::SendRespond {
        negotiation: NEGOTIATION.to_owned(),
        round: 1,
        answers: candidates()
            .iter()
            .map(|candidate| Answer {
                candidate_id: candidate.id.clone(),
                available: candidate.id != "slot-wednesday-afternoon",
            })
            .collect(),
        note: note.to_owned(),
    }
}

fn decide(first: bool, second: bool) -> Action {
    Action::SendDecide {
        negotiation: NEGOTIATION.to_owned(),
        round: 1,
        decisions: vec![
            Decision {
                candidate_id: "slot-tuesday-afternoon".to_owned(),
                accept: first,
            },
            Decision {
                candidate_id: "slot-thursday-afternoon".to_owned(),
                accept: second,
            },
        ],
        note: String::new(),
    }
}

struct Reply {
    status: StatusCode,
    body: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|_| panic!("JSON body: {}", self.body))
    }

    fn error(&self) -> String {
        self.json()["error"].as_str().unwrap_or_default().to_owned()
    }
}

/// One person: their host, their gate's minter, and their agent's key.
struct Side {
    host: Arc<Host>,
    gate: Minter,
    agent: BindingKey,
    clock: Arc<ManualClock>,
}

impl Side {
    fn gateway(&self) -> &Arc<Gateway> {
        self.host.gateway()
    }

    fn mint(&self, action: &Action) -> Vec<u8> {
        self.gate
            .mint(
                &permit(action),
                &self.agent.verifying_key(),
                self.clock.now_ms(),
            )
            .unwrap()
            .bytes
    }

    async fn act(&self, action: &Action) -> Reply {
        self.act_with(&self.mint(action), action).await
    }

    async fn act_with(&self, token: &[u8], action: &Action) -> Reply {
        self.act_at(endpoint(action), token, action).await
    }

    async fn act_at(&self, path: &str, token: &[u8], action: &Action) -> Reply {
        let body = serde_json::to_vec(action).unwrap();
        let presentation = self
            .agent
            .present(token, &body, self.clock.now_ms())
            .unwrap();
        let request = Request::post(path)
            .header(AUTHORIZATION, authorization(&presentation).unwrap())
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap();
        self.request(request).await
    }

    async fn request(&self, request: Request<Body>) -> Reply {
        let response = router(self.host.clone()).oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        Reply {
            status,
            body: String::from_utf8(bytes.to_vec()).unwrap(),
        }
    }

    async fn pump(&self) -> usize {
        self.gateway().pump(0).await.unwrap()
    }

    fn events(&self) -> Vec<Event> {
        futures_now(self.gateway().deliveries(0, 0))
            .into_iter()
            .map(|delivery| delivery.event)
            .collect()
    }

    fn link(&self) -> String {
        self.gateway().status().link
    }

    /// The owner's answers on Tuesday and Thursday, signed by the consent signer.
    fn answer(&self, tuesday: bool, thursday: bool) {
        let expires = at("2027-01-12T08:00Z");
        let gateway = self.gateway();
        gateway
            .record_consent("slot-tuesday-afternoon", tuesday, expires)
            .unwrap();
        gateway
            .record_consent("slot-thursday-afternoon", thursday, expires)
            .unwrap();
    }
}

/// Runs a future that is ready at once.
fn futures_now<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(future))
}

fn endpoint(action: &Action) -> &'static str {
    match action {
        Action::Handshake { .. } => "/v1/gateway/handshake",
        _ => "/v1/gateway/send",
    }
}

struct Net {
    relay: RelayClient,
    inboxes: [String; 2],
    alice: Side,
    bob: Side,
    keys: [PersonKeys; 2],
    clock: Arc<ManualClock>,
    directory: PathBuf,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl Drop for Net {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Net {
    async fn start(test: &str) -> Self {
        Self::start_with(test, |_, peer| peer, None).await
    }

    /// Starts the relay and both gateways. `peer_keys` may change the keys a gateway is
    /// configured with for its peer.
    /// `alice_lower` picks whose Noise key is lower, which decides who wins simultaneous calls.
    async fn start_with(
        test: &str,
        peer_keys: fn(&str, PublicKeys) -> PublicKeys,
        alice_lower: Option<bool>,
    ) -> Self {
        let directory = scratch(test);
        let clock = Arc::new(ManualClock::new(at("2027-01-11T08:00Z")));
        let inboxes = [new_mailbox().unwrap(), new_mailbox().unwrap()];
        let relay = Relay::new(
            &inboxes,
            clock.clone(),
            AuditLog::open(&directory.join("relay.jsonl")).unwrap(),
        );
        let listener = bind_loopback("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(serve(listener, relay, std::future::pending()));

        let (alice, bob) = loop {
            let alice = PersonKeys::load_or_create(&MemoryStore::new(), "alice").unwrap();
            let bob = PersonKeys::load_or_create(&MemoryStore::new(), "bob").unwrap();
            let lower = alice.public().noise_x25519 < bob.public().noise_x25519;
            if alice_lower.is_none_or(|wanted| wanted == lower) {
                break (alice, bob);
            }
        };
        let side = |me: &PersonKeys, peer: &PersonKeys, role, inbox: usize, agent: u8| {
            let gateway = Gateway::new(GatewayParts {
                person: me.person().to_owned(),
                peer: peer.person().to_owned(),
                role,
                negotiation: (role == Role::Coordinator).then(|| NEGOTIATION.to_owned()),
                identity: me.identity().clone(),
                noise_secret: me.noise_secret(),
                own_keys: me.public(),
                peer_keys: peer_keys(me.person(), peer.public()),
                relay: RelayClient::new(&url),
                inbox: inboxes[inbox].clone(),
                peer_inbox: inboxes[1 - inbox].clone(),
                clock: clock.clone(),
                audit: AuditLog::open(&directory.join(format!("{}.jsonl", me.person()))).unwrap(),
            })
            .unwrap();
            Side {
                host: Host::new(HostParts {
                    gateway,
                    policy: None,
                    agent_secret: AGENT_SECRET.to_owned(),
                    owner_secret: AGENT_SECRET.to_owned(),
                    clock: clock.clone(),
                    audit: AuditLog::disabled(),
                }),
                gate: me.minter(),
                agent: BindingKey::from_bytes(&[agent; 32]),
                clock: clock.clone(),
            }
        };
        Self {
            alice: side(&alice, &bob, Role::Coordinator, 0, 9),
            bob: side(&bob, &alice, Role::Invitee, 1, 10),
            relay: RelayClient::new(&url),
            inboxes,
            keys: [alice, bob],
            clock,
            directory,
            server,
        }
    }

    /// Alice calls, Bob answers, and both sides see the session.
    async fn connect(&self) {
        assert_eq!(
            self.alice.act(&handshake("bob")).await.json(),
            json!({"link": "calling"})
        );
        self.bob.pump().await;
        let answer = self.bob.act(&handshake("alice")).await;
        assert_eq!(
            answer.json(),
            json!({"link": "connected"}),
            "{}",
            answer.body
        );
        self.alice.pump().await;
        assert_eq!(
            (self.alice.link(), self.bob.link()),
            ("connected".into(), "connected".into())
        );
    }

    /// Every frame either inbox ever held.
    async fn capture(&self) -> Vec<Vec<u8>> {
        let mut frames = Vec::new();
        for inbox in &self.inboxes {
            for frame in self.relay.poll(inbox, 0, 0).await.unwrap() {
                frames.push(frame.frame);
            }
        }
        frames
    }
}

fn messages(events: &[Event]) -> Vec<Message> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Message { message } => Some(message.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_invitee_answers_a_call_with_its_own_handshake() {
    let net = Net::start("handshake").await;
    let calling = net.alice.act(&handshake("bob")).await;
    assert_eq!(calling.json(), json!({ "link": "calling" }));

    // Before Bob answers, a proposal is refused and its token stays usable.
    let proposal = propose("afternoons suit me best");
    let token = net.alice.mint(&proposal);
    let early = net.alice.act_with(&token, &proposal).await;
    assert_eq!(
        (early.status, early.error().as_str()),
        (StatusCode::CONFLICT, "no_session")
    );

    assert_eq!(net.bob.pump().await, 1);
    assert_eq!(
        net.bob.events(),
        [Event::PeerCalling {
            peer: "alice".to_owned()
        }]
    );
    assert_eq!(net.bob.link(), "ringing");
    assert_eq!(net.alice.link(), "calling");

    let answer = net.bob.act(&handshake("alice")).await;
    assert_eq!(answer.json(), json!({ "link": "connected" }));
    net.alice.pump().await;
    assert_eq!(
        net.alice.events(),
        [Event::SessionEstablished {
            peer: "bob".to_owned()
        }]
    );
    assert!(net.alice.gateway().policy_negotiation().handshake_done);
    assert!(net.bob.gateway().policy_negotiation().handshake_done);

    let sent = net.alice.act_with(&token, &proposal).await;
    assert_eq!(sent.json(), json!({ "sent": "propose" }), "{}", sent.body);
    let again = net.alice.act(&handshake("bob")).await;
    assert_eq!(again.error(), "already_connected");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_round_reaches_the_other_agent_and_updates_both_negotiations() {
    let net = Net::start("round").await;
    net.connect().await;

    let proposal = propose("afternoons suit me best");
    assert_eq!(
        net.alice.act(&proposal).await.json(),
        json!({ "sent": "propose" })
    );
    net.bob.pump().await;
    let received = messages(&net.bob.events());
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].from, "alice");
    assert_eq!(received[0].negotiation, NEGOTIATION);
    assert_eq!(
        received[0],
        Message::for_action(
            &proposal,
            "alice",
            "bob",
            received[0].sent_at_ms,
            Vec::new()
        )
        .unwrap()
    );
    let bob_view = net.bob.gateway().policy_negotiation();
    assert_eq!(
        (
            bob_view.id.as_str(),
            bob_view.round,
            bob_view.candidates.len()
        ),
        (NEGOTIATION, 1, 3)
    );

    assert_eq!(
        net.bob.act(&respond("wednesday is taken")).await.json(),
        json!({ "sent": "respond" })
    );
    net.alice.pump().await;
    let mutual = vec![
        "slot-tuesday-afternoon".to_owned(),
        "slot-thursday-afternoon".to_owned(),
    ];
    assert_eq!(net.alice.gateway().policy_negotiation().mutual, mutual);
    assert_eq!(net.bob.gateway().policy_negotiation().mutual, mutual);

    for (side, other) in [(&net.alice, &net.bob), (&net.bob, &net.alice)] {
        side.answer(false, true);
        let differs = side.act(&decide(true, true)).await;
        assert_eq!(
            differs.error(),
            "no_consent_record",
            "the owner said no to Tuesday"
        );
        assert_eq!(
            side.act(&decide(false, true)).await.json(),
            json!({ "sent": "decide" })
        );
        other.pump().await;
    }
    assert_eq!(
        net.alice.gateway().consent_facts().len(),
        4,
        "both owners' records"
    );
    assert_eq!(net.bob.gateway().consent_facts().len(), 4);
    let status = net.alice.gateway().status();
    assert_eq!(status.negotiation.received_decisions.len(), 2);

    let close = Action::SendClose {
        negotiation: NEGOTIATION.to_owned(),
        reason: CloseReason::Agreed,
        note: "see you then".to_owned(),
    };
    assert_eq!(
        net.alice.act(&close).await.json(),
        json!({ "sent": "close" })
    );
    net.bob.pump().await;
    assert_eq!(
        net.bob.gateway().status().negotiation.closed,
        Some(CloseReason::Agreed)
    );
    let late = net.bob.act(&respond("too late")).await;
    assert_eq!(
        (late.status, late.error().as_str()),
        (StatusCode::CONFLICT, "out_of_turn")
    );

    let kinds: Vec<&str> = messages(&net.bob.events())
        .iter()
        .map(|message| message.body.kind())
        .collect();
    assert_eq!(kinds, ["propose", "decide", "close"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_caller_with_another_key_is_refused() {
    let net = Net::start("mismatch").await;
    let mallory = PersonKeys::load_or_create(&MemoryStore::new(), "mallory").unwrap();
    let (_call, hello) = Call::place(
        &mallory.noise_secret(),
        &net.keys[1].public().noise_x25519,
        [5; 8],
    )
    .unwrap();
    net.relay
        .post(&net.inboxes[1], &hello.encode().unwrap())
        .await
        .unwrap();
    net.bob.pump().await;
    let events = net.bob.events();
    assert!(
        matches!(&events[..], [Event::PeerKeyMismatch { detail }] if detail.contains("not alice's")),
        "{events:?}"
    );
    assert_eq!(net.bob.link(), "idle");
}

/// Whoever's Noise key is lower keeps their call; the test runs both ways round.
#[tokio::test(flavor = "multi_thread")]
async fn calls_placed_at_once_settle_on_one_session() {
    for alice_lower in [true, false] {
        let test = format!("simultaneous-{alice_lower}");
        let net = Net::start_with(&test, |_, peer| peer, Some(alice_lower)).await;
        assert_eq!(
            net.alice.act(&handshake("bob")).await.json(),
            json!({"link": "calling"})
        );
        assert_eq!(
            net.bob.act(&handshake("alice")).await.json(),
            json!({"link": "calling"})
        );
        for _ in 0..3 {
            net.alice.pump().await;
            net.bob.pump().await;
        }
        assert_eq!(
            (net.alice.link(), net.bob.link()),
            ("connected".into(), "connected".into()),
            "alice_lower = {alice_lower}"
        );
        let proposal = propose("afternoons suit me best");
        assert_eq!(
            net.alice.act(&proposal).await.json(),
            json!({ "sent": "propose" })
        );
        net.bob.pump().await;
        assert_eq!(messages(&net.bob.events()).len(), 1);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn every_send_needs_a_token_for_that_exact_message() {
    let net = Net::start("tokens").await;
    net.connect().await;
    let proposal = propose("afternoons suit me best");

    let request = Request::post("/v1/gateway/send")
        .body(Body::from(serde_json::to_vec(&proposal).unwrap()))
        .unwrap();
    let no_token = net.alice.request(request).await;
    let other_note = net
        .alice
        .act_with(&net.alice.mint(&propose("mornings suit me")), &proposal)
        .await;
    let bobs_gate = net
        .alice
        .act_with(&net.bob.mint(&proposal), &proposal)
        .await;
    let meeting = Action::AddMeeting {
        negotiation: NEGOTIATION.to_owned(),
        start_ms: at("2027-01-12T14:00Z"),
        length_ms: HALF_HOUR,
    };
    let calendar_action = net
        .alice
        .act_at("/v1/gateway/send", &net.alice.mint(&meeting), &meeting)
        .await;
    let invitee_proposes = net.bob.act(&proposal).await;
    let wrong_negotiation = net
        .alice
        .act(&Action::SendClose {
            negotiation: "n-other".to_owned(),
            reason: CloseReason::Withdrawn,
            note: String::new(),
        })
        .await;
    let stranger = net.alice.act(&handshake("carol")).await;
    let cases = [
        (no_token, StatusCode::UNAUTHORIZED, "bad_signature"),
        (other_note, StatusCode::FORBIDDEN, "token_argument_mismatch"),
        (bobs_gate, StatusCode::UNAUTHORIZED, "bad_signature"),
        (calendar_action, StatusCode::BAD_REQUEST, "wrong_endpoint"),
        (invitee_proposes, StatusCode::CONFLICT, "out_of_turn"),
        (wrong_negotiation, StatusCode::CONFLICT, "out_of_turn"),
        (stranger, StatusCode::BAD_REQUEST, "unknown_peer"),
    ];
    for (reply, status, error) in cases {
        assert_eq!(
            (reply.status, reply.error().as_str()),
            (status, error),
            "{}",
            reply.body
        );
    }

    let token = net.alice.mint(&proposal);
    assert_eq!(
        net.alice.act_with(&token, &proposal).await.status,
        StatusCode::OK
    );
    let replayed = net.alice.act_with(&token, &proposal).await;
    assert_eq!(replayed.error(), "token_replayed");
    net.bob.pump().await;
    assert_eq!(
        messages(&net.bob.events()).len(),
        1,
        "only one proposal arrived"
    );

    let request = Request::get("/v1/gateway/inbox")
        .body(Body::empty())
        .unwrap();
    let unauthorized = net.alice.request(request).await;
    assert_eq!(unauthorized.error(), "agent_unauthorized");
    let request = Request::get("/v1/gateway/inbox?after=0")
        .header(AUTHORIZATION, format!("Bearer {AGENT_SECRET}"))
        .body(Body::empty())
        .unwrap();
    let inbox = net.bob.request(request).await.json();
    let events: Vec<&str> = inbox["deliveries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|delivery| delivery["event"].as_str().unwrap())
        .collect();
    assert_eq!(events, ["peer_calling", "session_established", "message"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn replayed_altered_and_forged_frames_change_nothing() {
    let net = Net::start("replay").await;
    net.connect().await;
    net.alice.act(&propose("afternoons suit me best")).await;
    net.bob.pump().await;
    let frames = net.relay.poll(&net.inboxes[1], 0, 0).await.unwrap();
    let hello = frames.first().unwrap().frame.clone();
    let proposal_frame = frames.last().unwrap().frame.clone();

    for frame in [proposal_frame, b"not a frame".to_vec(), hello] {
        net.relay.post(&net.inboxes[1], &frame).await.unwrap();
    }
    net.bob.pump().await;
    let events = net.bob.events();
    let errors = events
        .iter()
        .filter(|event| matches!(event, Event::ProtocolError { .. }))
        .count();
    assert_eq!(
        errors, 2,
        "the replayed message and the garbage: {events:?}"
    );
    assert_eq!(net.bob.link(), "connected", "a replayed call is ignored");
    assert_eq!(net.bob.gateway().policy_negotiation().round, 1);
    assert_eq!(messages(&events).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_signed_with_another_identity_key_is_refused() {
    // Bob's gateway expects Alice's messages to verify with Mallory's identity key.
    let net = Net::start_with(
        "forged",
        |me, peer| {
            if me == "bob" {
                let mallory = SigningKey::from_bytes(&[42; 32]);
                PublicKeys {
                    identity_ed25519: mallory.verifying_key().to_bytes(),
                    ..peer
                }
            } else {
                peer
            }
        },
        None,
    )
    .await;
    net.connect().await;
    net.alice.act(&propose("afternoons suit me best")).await;
    net.bob.pump().await;
    let events = net.bob.events();
    assert!(
        matches!(events.last(), Some(Event::ProtocolError { detail }) if detail.contains("signature")),
        "{events:?}"
    );
    assert_eq!(net.bob.gateway().policy_negotiation().round, 0);
}

/// The encodings in which a value could show up if it crossed the relay in the clear.
fn encodings(text: &str) -> Vec<Vec<u8>> {
    let mut cbor =
        fabric_token::cbor::encode(&fabric_token::cbor::Value::Text(text.to_owned())).unwrap();
    let json = serde_json::to_vec(text).unwrap();
    let mut forms = vec![json];
    if text.len() >= 6 {
        forms.push(text.as_bytes().to_vec());
    }
    forms.push(std::mem::take(&mut cbor));
    forms
}

#[tokio::test(flavor = "multi_thread")]
async fn the_relay_carries_only_ciphertext() {
    let net = Net::start("ciphertext").await;
    net.connect().await;
    let notes = [
        "afternoons suit me best",
        "wednesday is taken",
        "see you then",
    ];
    net.alice.act(&propose(notes[0])).await;
    net.bob.pump().await;
    net.bob.act(&respond(notes[1])).await;
    net.alice.pump().await;
    net.alice.answer(false, true);
    net.alice.act(&decide(false, true)).await;
    net.bob.pump().await;
    net.alice
        .act(&Action::SendClose {
            negotiation: NEGOTIATION.to_owned(),
            reason: CloseReason::Agreed,
            note: notes[2].to_owned(),
        })
        .await;
    net.bob.pump().await;
    assert_eq!(messages(&net.bob.events()).len(), 3);

    let mut markers: Vec<Vec<u8>> = Vec::new();
    for text in notes
        .iter()
        .copied()
        .chain(["alice", "bob", "Alice", "Bob", NEGOTIATION])
        .chain(candidates().iter().map(|candidate| candidate.id.as_str()))
    {
        markers.extend(encodings(text));
    }
    for candidate in candidates() {
        let start = candidate.start_ms;
        markers.push(fabric_token::cbor::encode(&fabric_token::cbor::int(start)).unwrap());
        markers.push(start.to_string().into_bytes());
        markers.push(fabric_policy::time::format_utc(start).into_bytes());
    }

    let frames = net.capture().await;
    assert!(frames.len() >= 6, "{} frames", frames.len());
    for frame in &frames {
        for marker in &markers {
            assert!(
                !frame
                    .windows(marker.len())
                    .any(|window| window == marker.as_slice()),
                "{:?} crossed the relay in the clear",
                String::from_utf8_lossy(marker)
            );
        }
    }
    let log = fs::read_to_string(net.directory.join("relay.jsonl")).unwrap();
    for text in notes.iter().copied().chain(["alice", "bob"]) {
        assert!(!log.contains(text), "{text} in the relay log");
    }
    let _ = &net.clock;
}
