//! The negotiation as the gateway has seen it (section 5.2): the state the policy gate reasons
//! about, kept up to date from the messages this side sent and received.
//!
//! Isabelle checks every outgoing action against this state when it is authorized. The gateway
//! checks the same protocol rules again when the action runs, in case a message arrived in
//! between, and checks every incoming message before it changes anything.

use fabric_policy::{Candidate, CloseReason, Decision, Negotiation, Role};
use fabric_wire::{Body, Message};
use serde::{Deserialize, Serialize};

/// The most candidates an incoming proposal may carry. Isabelle limits what this side sends;
/// this only bounds what a misbehaving peer can make the gateway store.
const MAX_INCOMING_CANDIDATES: usize = 16;

/// Stands in for the negotiation ID in policy checks before the invitee has learned it.
pub const NO_NEGOTIATION: &str = "none";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NegotiationRecord {
    /// Set from the start for the coordinator; the invitee learns it from the first proposal.
    pub id: Option<String>,
    pub role: Role,
    /// The current round; 0 before the first proposal.
    pub round: u32,
    /// This round's candidates, as proposed or received.
    pub candidates: Vec<Candidate>,
    /// IDs of this round's mutual options, in proposal order.
    pub mutual: Vec<String>,
    pub sent_decisions: Vec<Decision>,
    pub received_decisions: Vec<Decision>,
    pub closed: Option<CloseReason>,
}

impl NegotiationRecord {
    #[must_use]
    pub const fn new(role: Role, id: Option<String>) -> Self {
        Self {
            id,
            role,
            round: 0,
            candidates: Vec::new(),
            mutual: Vec::new(),
            sent_decisions: Vec::new(),
            received_decisions: Vec::new(),
            closed: None,
        }
    }

    /// The negotiation as the policy gate's context holds it.
    #[must_use]
    pub fn for_policy(&self, handshake_done: bool) -> Negotiation {
        Negotiation {
            id: self.id.clone().unwrap_or_else(|| NO_NEGOTIATION.to_owned()),
            role: self.role,
            round: self.round,
            handshake_done,
            candidates: self.candidates.clone(),
            mutual: self.mutual.clone(),
        }
    }

    fn active(&self, negotiation: &str) -> Result<(), String> {
        if let Some(reason) = self.closed {
            return Err(format!("the negotiation was closed ({reason:?})"));
        }
        match &self.id {
            Some(id) if id == negotiation => Ok(()),
            Some(id) => Err(format!(
                "negotiation {negotiation} is not the active one ({id})"
            )),
            None => Err(format!(
                "no negotiation is active yet, so {negotiation} is unknown"
            )),
        }
    }

    fn candidate_ids(&self) -> Vec<&str> {
        self.candidates.iter().map(|c| c.id.as_str()).collect()
    }

    /// Checks a message this side is about to send.
    pub fn check_outgoing(&self, message: &Message) -> Result<(), String> {
        self.active(&message.negotiation)?;
        match &message.body {
            Body::Propose {
                round, candidates, ..
            } => {
                if self.role != Role::Coordinator {
                    return Err("only the coordinator proposes".to_owned());
                }
                if *round != self.round + 1 {
                    return Err(format!(
                        "round {round}, but the next round is {}",
                        self.round + 1
                    ));
                }
                distinct(candidates)
            }
            Body::Respond { round, answers, .. } => {
                if self.role != Role::Invitee {
                    return Err("only the invitee responds".to_owned());
                }
                self.current_round(*round)?;
                let answered: Vec<&str> = answers.iter().map(|a| a.candidate_id.as_str()).collect();
                if answered != self.candidate_ids() {
                    return Err(
                        "the answers do not cover the received candidates in order".to_owned()
                    );
                }
                Ok(())
            }
            Body::Decide {
                round, decisions, ..
            } => {
                self.current_round(*round)?;
                let decided: Vec<&str> =
                    decisions.iter().map(|d| d.candidate_id.as_str()).collect();
                if decided != self.mutual.iter().map(String::as_str).collect::<Vec<_>>() {
                    return Err("the decisions do not cover the mutual options in order".to_owned());
                }
                Ok(())
            }
            Body::Close { .. } => Ok(()),
        }
    }

    /// Records a message this side sent.
    pub fn record_outgoing(&mut self, message: &Message) {
        match &message.body {
            Body::Propose {
                round, candidates, ..
            } => self.start_round(*round, candidates.clone()),
            Body::Respond { answers, .. } => {
                self.mutual = answers
                    .iter()
                    .filter(|answer| answer.available)
                    .map(|answer| answer.candidate_id.clone())
                    .collect();
            }
            Body::Decide { decisions, .. } => self.sent_decisions.clone_from(decisions),
            Body::Close { reason, .. } => self.closed = Some(*reason),
        }
    }

    /// Checks a message from the peer and records it. On error nothing changes.
    pub fn receive(&mut self, message: &Message) -> Result<(), String> {
        let mut next = self.clone();
        next.apply_incoming(message)?;
        *self = next;
        Ok(())
    }

    fn apply_incoming(&mut self, message: &Message) -> Result<(), String> {
        match &message.body {
            Body::Propose {
                round, candidates, ..
            } => {
                if self.role != Role::Invitee {
                    return Err("the peer proposed, but this side coordinates".to_owned());
                }
                if self.id.is_none() && self.closed.is_none() {
                    // The first proposal tells the invitee which negotiation this is.
                    self.id = Some(message.negotiation.clone());
                }
                self.active(&message.negotiation)?;
                if *round != self.round + 1 {
                    return Err(format!(
                        "a proposal for round {round}, but the next round is {}",
                        self.round + 1
                    ));
                }
                if candidates.is_empty() || candidates.len() > MAX_INCOMING_CANDIDATES {
                    return Err(format!("{} candidates", candidates.len()));
                }
                distinct(candidates)?;
                self.start_round(*round, candidates.clone());
            }
            Body::Respond { round, answers, .. } => {
                if self.role != Role::Coordinator {
                    return Err("the peer responded, but this side is the invitee".to_owned());
                }
                self.active(&message.negotiation)?;
                self.current_round(*round)?;
                let answered: Vec<&str> = answers.iter().map(|a| a.candidate_id.as_str()).collect();
                if answered != self.candidate_ids() {
                    return Err(
                        "the answers do not cover this round's candidates in order".to_owned()
                    );
                }
                self.mutual = answers
                    .iter()
                    .filter(|answer| answer.available)
                    .map(|answer| answer.candidate_id.clone())
                    .collect();
            }
            Body::Decide {
                round, decisions, ..
            } => {
                self.active(&message.negotiation)?;
                self.current_round(*round)?;
                if decisions
                    .iter()
                    .any(|decision| !self.mutual.contains(&decision.candidate_id))
                {
                    return Err("a decision is about an option that is not mutual".to_owned());
                }
                self.received_decisions.clone_from(decisions);
            }
            Body::Close { reason, .. } => {
                self.active(&message.negotiation)?;
                self.closed = Some(*reason);
            }
        }
        Ok(())
    }

    fn current_round(&self, round: u32) -> Result<(), String> {
        if round == self.round {
            Ok(())
        } else {
            Err(format!(
                "round {round}, but the current round is {}",
                self.round
            ))
        }
    }

    fn start_round(&mut self, round: u32, candidates: Vec<Candidate>) {
        self.round = round;
        self.candidates = candidates;
        self.mutual.clear();
        self.sent_decisions.clear();
        self.received_decisions.clear();
    }
}

fn distinct(candidates: &[Candidate]) -> Result<(), String> {
    let mut ids: Vec<&str> = candidates.iter().map(|c| c.id.as_str()).collect();
    ids.sort_unstable();
    if ids.windows(2).any(|pair| pair[0] == pair[1]) {
        Err("candidate IDs repeat".to_owned())
    } else {
        Ok(())
    }
}
