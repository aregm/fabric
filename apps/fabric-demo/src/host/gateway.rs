//! The messaging gateway (section 5.2, task T5.2): one per person, inside the trusted host.
//!
//! It holds the person's identity and Noise keys and talks to exactly one configured peer
//! through the relay (Q21). A handshake or a send runs only with a token for that exact action
//! (D15). Messages are signed with the identity key inside a Noise IK session (D16, D17). What
//! arrives is checked and delivered to this person's agent, which needs no token to read it
//! (D19).
//!
//! The handshake follows section 11. One side's agent places a call. The other gateway checks
//! the caller's static key against the configured peer (`peer_key_mismatch` otherwise) and tells
//! its agent the peer is calling; the session opens when that agent answers with its own
//! handshake. If both sides call at once, the side with the lower Noise key keeps its call, and
//! the other side answers it, since its agent has already asked for a session.

use std::collections::HashSet;
use std::pin::pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use axum::http::StatusCode;
use fabric_policy::{Action, Negotiation, Role};
use fabric_token::{
    Presentation, PublicKeys, ReplayCache, Service, SigningKey, Verifier, VerifyingKey, hex, random,
};
use fabric_wire::{Call, Frame, IncomingCall, Message, Session, SessionId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Notify;
use tokio::time::{Instant, timeout_at};
use zeroize::Zeroizing;

use super::negotiation::NegotiationRecord;
use crate::api::Failure;
use crate::audit::AuditLog;
use crate::clock::Clock;
use crate::relay::{RelayClient, RelayError};

/// A call that has gone unanswered this long may be replaced by a new one.
const CALL_TIMEOUT_MS: i64 = 30_000;
/// The longest an agent's inbox poll waits.
pub const MAX_WAIT_MS: u64 = 25_000;

/// Everything a gateway needs, already loaded.
pub struct GatewayParts {
    pub person: String,
    pub peer: String,
    pub role: Role,
    /// The coordinator's negotiation ID; the invitee learns it from the first proposal.
    pub negotiation: Option<String>,
    pub identity: SigningKey,
    pub noise_secret: Zeroizing<[u8; 32]>,
    /// This person's public keys: the gate key checks tokens, and the Noise key breaks ties.
    pub own_keys: PublicKeys,
    pub peer_keys: PublicKeys,
    pub relay: RelayClient,
    pub inbox: String,
    pub peer_inbox: String,
    pub clock: Arc<dyn Clock>,
    pub audit: AuditLog,
}

/// What this person's agent receives, in order.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Delivery {
    pub seq: u64,
    pub at_ms: i64,
    #[serde(flatten)]
    pub event: Event,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// The peer placed a call. This side's agent answers it with its own handshake.
    PeerCalling {
        peer: String,
    },
    SessionEstablished {
        peer: String,
    },
    /// A caller whose static key is not the configured peer's.
    PeerKeyMismatch {
        detail: String,
    },
    Message {
        message: Message,
    },
    /// A frame or message that broke the protocol. It changed nothing.
    ProtocolError {
        detail: String,
    },
}

/// The gateway's state as its agent and the dashboard see it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Status {
    /// `idle`, `calling`, `ringing` or `connected`.
    pub link: String,
    pub negotiation: NegotiationRecord,
}

enum Link {
    Idle,
    /// This side placed a call and waits for the answer.
    Calling {
        call: Call,
        placed_at_ms: i64,
    },
    /// The peer called, and this side's agent has not answered yet.
    Ringing(IncomingCall),
    Connected(Session),
}

impl Link {
    const fn name(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Calling { .. } => "calling",
            Self::Ringing(_) => "ringing",
            Self::Connected(_) => "connected",
        }
    }
}

struct State {
    link: Link,
    negotiation: NegotiationRecord,
    deliveries: Vec<Delivery>,
    /// The relay sequence number of the last frame handled.
    relay_cursor: u64,
    /// Sessions of calls already handled, so a replayed call is ignored.
    seen_calls: HashSet<SessionId>,
}

/// What a handshake did, kept until the relay has taken its frame.
enum Placed {
    Answered(Session),
    Calling(Box<Call>),
}

pub struct Gateway {
    person: String,
    peer: String,
    identity: SigningKey,
    noise_secret: Zeroizing<[u8; 32]>,
    noise_public: [u8; 32],
    peer_identity: VerifyingKey,
    peer_noise: [u8; 32],
    verifier: Verifier,
    replay: ReplayCache,
    relay: RelayClient,
    inbox: String,
    peer_inbox: String,
    clock: Arc<dyn Clock>,
    audit: AuditLog,
    state: Mutex<State>,
    /// Serializes everything that posts to the relay, so frames leave in the order decided.
    outbound: tokio::sync::Mutex<()>,
    /// Serializes inbox processing.
    inbound: tokio::sync::Mutex<()>,
    delivered: Notify,
}

impl Gateway {
    pub fn new(parts: GatewayParts) -> Result<Arc<Self>, String> {
        let gate_key = parts.own_keys.gate().map_err(|error| error.to_string())?;
        let peer_identity = parts
            .peer_keys
            .identity()
            .map_err(|error| error.to_string())?;
        Ok(Arc::new(Self {
            verifier: Verifier::new(Service::Gateway, &parts.person, gate_key),
            state: Mutex::new(State {
                link: Link::Idle,
                negotiation: NegotiationRecord::new(parts.role, parts.negotiation),
                deliveries: Vec::new(),
                relay_cursor: 0,
                seen_calls: HashSet::new(),
            }),
            person: parts.person,
            peer: parts.peer,
            identity: parts.identity,
            noise_secret: parts.noise_secret,
            noise_public: parts.own_keys.noise_x25519,
            peer_identity,
            peer_noise: parts.peer_keys.noise_x25519,
            replay: ReplayCache::new(),
            relay: parts.relay,
            inbox: parts.inbox,
            peer_inbox: parts.peer_inbox,
            clock: parts.clock,
            audit: parts.audit,
            outbound: tokio::sync::Mutex::new(()),
            inbound: tokio::sync::Mutex::new(()),
            delivered: Notify::new(),
        }))
    }

    /// The audience this gateway accepts and the name it logs under: `gateway:<person>`.
    #[must_use]
    pub fn component(&self) -> &str {
        self.verifier.audience()
    }

    #[must_use]
    pub fn peer(&self) -> &str {
        &self.peer
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[must_use]
    pub fn status(&self) -> Status {
        let state = self.state();
        Status {
            link: state.link.name().to_owned(),
            negotiation: state.negotiation.clone(),
        }
    }

    /// The negotiation as the policy gate's context holds it.
    #[must_use]
    pub fn policy_negotiation(&self) -> Negotiation {
        let state = self.state();
        state
            .negotiation
            .for_policy(matches!(state.link, Link::Connected(_)))
    }

    /// The deliveries after `after`. With none yet, waits up to `wait_ms` for one.
    pub async fn deliveries(&self, after: u64, wait_ms: u64) -> Vec<Delivery> {
        let deadline = Instant::now() + Duration::from_millis(wait_ms.min(MAX_WAIT_MS));
        loop {
            let mut delivered = pin!(self.delivered.notified());
            delivered.as_mut().enable();
            let found = self.deliveries_after(after);
            if !found.is_empty() || Instant::now() >= deadline {
                return found;
            }
            if timeout_at(deadline, delivered).await.is_err() {
                return self.deliveries_after(after);
            }
        }
    }

    fn deliveries_after(&self, after: u64) -> Vec<Delivery> {
        self.state()
            .deliveries
            .iter()
            .filter(|delivery| delivery.seq > after)
            .cloned()
            .collect()
    }

    fn record(&self, mut record: Value) {
        record["at_ms"] = self.clock.now_ms().into();
        record["component"] = self.component().into();
        self.audit.record(&record);
    }

    /// Appends a delivery for the agent and records it.
    fn deliver(&self, state: &mut State, event: Event) {
        let delivery = Delivery {
            seq: state.deliveries.len() as u64 + 1,
            at_ms: self.clock.now_ms(),
            event,
        };
        self.record(json!({ "kind": "delivery", "delivery": delivery }));
        state.deliveries.push(delivery);
        self.delivered.notify_waiters();
    }

    async fn post(&self, frame: &Frame) -> Result<(), Failure> {
        let bytes = frame
            .encode()
            .map_err(|error| Failure::internal("wire_failed", error))?;
        self.relay
            .post(&self.peer_inbox, &bytes)
            .await
            .map(|_| ())
            .map_err(|error| {
                Failure::new(StatusCode::BAD_GATEWAY, "relay_failed", error.to_string())
            })
    }

    /// Runs a `handshake` action: answers the peer's call if one is waiting, and otherwise
    /// places a call. Returns `{"link": "connected"}` or `{"link": "calling"}`.
    pub async fn handshake(
        &self,
        presentation: &Presentation,
        action: &Action,
        body: &[u8],
    ) -> Result<Value, Failure> {
        let Action::Handshake { peer } = action else {
            return Err(Failure::bad_request(
                "wrong_endpoint",
                format!(
                    "the handshake endpoint runs handshake, not {}",
                    action.kind()
                ),
            ));
        };
        if *peer != self.peer {
            return Err(Failure::bad_request(
                "unknown_peer",
                format!("this gateway talks only to {}", self.peer),
            ));
        }
        let _outbound = self.outbound.lock().await;
        let now_ms = self.clock.now_ms();
        let accepted = self
            .verifier
            .check(presentation, action, body, now_ms, &self.replay)?;

        let (placed, frame) = {
            let mut state = self.state();
            match std::mem::replace(&mut state.link, Link::Idle) {
                Link::Connected(session) => {
                    state.link = Link::Connected(session);
                    return Err(Failure::conflict(
                        "already_connected",
                        format!("a session with {} is already open", self.peer),
                    ));
                }
                Link::Calling { call, placed_at_ms } if now_ms - placed_at_ms < CALL_TIMEOUT_MS => {
                    state.link = Link::Calling { call, placed_at_ms };
                    return Err(Failure::conflict(
                        "call_pending",
                        format!("a call to {} is waiting for an answer", self.peer),
                    ));
                }
                Link::Ringing(incoming) => {
                    let (session, frame) = incoming
                        .answer()
                        .map_err(|error| Failure::internal("noise_failed", error))?;
                    (Placed::Answered(session), frame)
                }
                Link::Calling { .. } | Link::Idle => {
                    let session =
                        random::<8>().map_err(|error| Failure::internal("noise_failed", error))?;
                    let (call, frame) = Call::place(&self.noise_secret, &self.peer_noise, session)
                        .map_err(|error| Failure::internal("noise_failed", error))?;
                    (Placed::Calling(Box::new(call)), frame)
                }
            }
        };
        // If the relay fails, the link stays idle and the token is released for a retry.
        self.post(&frame).await?;
        let claims = accepted.claims.clone();
        accepted.commit();

        let link = {
            let mut state = self.state();
            match placed {
                Placed::Answered(session) => {
                    state.link = Link::Connected(session);
                    let peer = self.peer.clone();
                    self.deliver(&mut state, Event::SessionEstablished { peer });
                    "connected"
                }
                Placed::Calling(call) => {
                    state.seen_calls.insert(call.session());
                    state.link = Link::Calling {
                        call: *call,
                        placed_at_ms: now_ms,
                    };
                    "calling"
                }
            }
        };
        self.record(json!({
            "kind": "handshake",
            "outcome": "executed",
            "link": link,
            "token": claims,
        }));
        Ok(json!({ "link": link }))
    }

    /// Runs a send action: signs the message, encrypts it in the session, and posts it to the
    /// peer's inbox. Returns `{"sent": "<type>"}`.
    pub async fn send(
        &self,
        presentation: &Presentation,
        action: &Action,
        body: &[u8],
    ) -> Result<Value, Failure> {
        let now_ms = self.clock.now_ms();
        // Phase 7 attaches the owner's signed consent records to a decision here.
        let Some(message) =
            Message::for_action(action, &self.person, &self.peer, now_ms, Vec::new())
        else {
            return Err(Failure::bad_request(
                "wrong_endpoint",
                format!("the send endpoint runs send actions, not {}", action.kind()),
            ));
        };
        let _outbound = self.outbound.lock().await;
        let accepted = self.verifier.check(
            presentation,
            action,
            body,
            self.clock.now_ms(),
            &self.replay,
        )?;

        let frame = {
            let mut guard = self.state();
            let state = &mut *guard;
            let Link::Connected(session) = &mut state.link else {
                return Err(Failure::conflict(
                    "no_session",
                    format!("no session with {} is open", self.peer),
                ));
            };
            state
                .negotiation
                .check_outgoing(&message)
                .map_err(|detail| Failure::conflict("out_of_turn", detail))?;
            let signed = message
                .sign(&self.identity)
                .map_err(|error| Failure::internal("wire_failed", error))?;
            session
                .seal(&signed)
                .map_err(|error| Failure::internal("noise_failed", error))?
        };
        // If the relay fails, nothing is recorded and the token is released. The nonce the frame
        // used is simply skipped.
        self.post(&frame).await?;
        let claims = accepted.claims.clone();
        accepted.commit();
        self.state().negotiation.record_outgoing(&message);
        self.record(json!({
            "kind": "send",
            "outcome": "executed",
            "message": message,
            "token": claims,
        }));
        Ok(json!({ "sent": message.body.kind() }))
    }

    /// Handles the frames waiting in this person's inbox, waiting up to `wait_ms` for the
    /// first. Returns how many frames it handled.
    pub async fn pump(&self, wait_ms: u64) -> Result<usize, RelayError> {
        let _inbound = self.inbound.lock().await;
        let after = self.state().relay_cursor;
        let frames = self.relay.poll(&self.inbox, after, wait_ms).await?;
        for frame in &frames {
            self.handle(&frame.frame).await;
            self.state().relay_cursor = frame.seq;
        }
        Ok(frames.len())
    }

    /// Pumps the inbox for as long as the gateway runs.
    pub async fn run_inbound(self: Arc<Self>) {
        loop {
            if let Err(error) = self.pump(MAX_WAIT_MS).await {
                self.record(json!({ "kind": "relay_error", "detail": error.to_string() }));
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }

    async fn handle(&self, bytes: &[u8]) {
        match Frame::decode(bytes) {
            Ok(Frame::Hello { session, noise }) => self.on_call(session, &noise).await,
            Ok(Frame::Welcome { session, noise }) => self.on_answer(session, &noise),
            Ok(Frame::Message {
                session,
                nonce,
                noise,
            }) => self.on_message(session, nonce, &noise),
            Err(error) => self.protocol_error(format!("an unreadable frame: {error}")),
        }
    }

    fn protocol_error(&self, detail: String) {
        let mut state = self.state();
        self.deliver(&mut state, Event::ProtocolError { detail });
    }

    async fn on_call(&self, session: SessionId, noise: &[u8]) {
        if self.state().seen_calls.contains(&session) {
            self.record(
                json!({ "kind": "replayed_call_ignored", "session": hex::encode(&session) }),
            );
            return;
        }
        let incoming = match IncomingCall::receive(&self.noise_secret, session, noise) {
            Ok(incoming) => incoming,
            Err(error) => return self.protocol_error(format!("an unreadable call: {error}")),
        };
        if *incoming.caller() != self.peer_noise {
            let detail = format!(
                "the caller's Noise key {} is not {}'s",
                hex::encode(incoming.caller()),
                self.peer
            );
            let mut state = self.state();
            state.seen_calls.insert(session);
            self.deliver(&mut state, Event::PeerKeyMismatch { detail });
            return;
        }

        let _outbound = self.outbound.lock().await;
        let answer_now = {
            let mut state = self.state();
            state.seen_calls.insert(session);
            match std::mem::replace(&mut state.link, Link::Idle) {
                Link::Calling { call, placed_at_ms } if self.noise_public < self.peer_noise => {
                    // Both sides called. This side's call stands, and the peer will answer it.
                    state.link = Link::Calling { call, placed_at_ms };
                    None
                }
                // Both sides called and the peer's call stands. This side's agent has already
                // asked for a session, so the gateway answers.
                Link::Calling { .. } => Some(incoming),
                Link::Idle | Link::Ringing(_) | Link::Connected(_) => {
                    // A new call. Any older session is gone, and the agent must answer again.
                    state.link = Link::Ringing(incoming);
                    let peer = self.peer.clone();
                    self.deliver(&mut state, Event::PeerCalling { peer });
                    None
                }
            }
        };
        if let Some(incoming) = answer_now {
            match incoming.answer() {
                Ok((session, frame)) => match self.post(&frame).await {
                    Ok(()) => {
                        let mut state = self.state();
                        state.link = Link::Connected(session);
                        let peer = self.peer.clone();
                        self.deliver(&mut state, Event::SessionEstablished { peer });
                    }
                    Err(failure) => self
                        .protocol_error(format!("could not answer the call: {}", failure.detail)),
                },
                Err(error) => self.protocol_error(format!("could not answer the call: {error}")),
            }
        }
    }

    fn on_answer(&self, session: SessionId, noise: &[u8]) {
        let mut state = self.state();
        match std::mem::replace(&mut state.link, Link::Idle) {
            Link::Calling { call, .. } if call.session() == session => match call.complete(noise) {
                Ok(session) => {
                    state.link = Link::Connected(session);
                    let peer = self.peer.clone();
                    self.deliver(&mut state, Event::SessionEstablished { peer });
                }
                Err(error) => {
                    let detail = format!("the answer to this side's call failed: {error}");
                    self.deliver(&mut state, Event::ProtocolError { detail });
                }
            },
            other => {
                state.link = other;
                let detail = "an answer to no call this side placed".to_owned();
                self.deliver(&mut state, Event::ProtocolError { detail });
            }
        }
    }

    fn on_message(&self, session_id: SessionId, nonce: u64, noise: &[u8]) {
        let mut guard = self.state();
        let state = &mut *guard;
        let Link::Connected(session) = &mut state.link else {
            let detail = "a message outside any session".to_owned();
            return self.deliver(state, Event::ProtocolError { detail });
        };
        if session.id() != session_id {
            let detail = "a message for another session".to_owned();
            return self.deliver(state, Event::ProtocolError { detail });
        }
        let message = session
            .open(nonce, noise)
            .and_then(|plaintext| Message::verify(&plaintext, &self.peer_identity));
        let message = match message {
            Ok(message) => message,
            Err(error) => {
                let detail = format!("a refused message: {error}");
                return self.deliver(state, Event::ProtocolError { detail });
            }
        };
        if message.from != self.peer || message.to != self.person {
            let detail = format!("a message from {} to {}", message.from, message.to);
            return self.deliver(state, Event::ProtocolError { detail });
        }
        match state.negotiation.receive(&message) {
            Ok(()) => self.deliver(state, Event::Message { message }),
            Err(detail) => {
                let detail = format!("a refused {}: {detail}", message.body.kind());
                self.deliver(state, Event::ProtocolError { detail });
            }
        }
    }
}
