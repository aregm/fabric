//! Rust mirrors of the datatypes in `isabelle/Fabric_Policy/Fabric_Policy.thy`.
//!
//! Times are UTC milliseconds since the Unix epoch and lengths are milliseconds, as in the
//! theory. The trusted host fills a [`Context`] from its own records; an agent only ever
//! supplies an [`Action`].

use serde::{Deserialize, Serialize};

/// The owner's pre-authorization for one negotiation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// Configured key ID of the person the owner wants to meet.
    pub peer: String,
    pub duration_ms: i64,
    pub window_start_ms: i64,
    pub window_end_ms: i64,
    pub opened_at_ms: i64,
    pub expires_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub start_ms: i64,
    pub length_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    pub candidate_id: String,
    pub available: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub candidate_id: String,
    pub accept: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Coordinator,
    Invitee,
}

/// The negotiation as the messaging gateway has seen it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Negotiation {
    pub id: String,
    pub role: Role,
    /// The current round; 0 before the first proposal.
    pub round: u32,
    pub handshake_done: bool,
    /// This round's candidates, as proposed (coordinator) or received (invitee).
    pub candidates: Vec<Candidate>,
    /// IDs of this round's mutual options, in proposal order.
    pub mutual: Vec<String>,
}

/// A consent record whose signature the gate has already verified.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Consent {
    pub signer: String,
    pub negotiation: String,
    pub start_ms: i64,
    pub length_ms: i64,
    pub yes: bool,
    pub expires_ms: i64,
}

/// Every fact Isabelle reasons about for one check.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Context {
    /// This owner's key ID.
    pub owner: String,
    pub now_ms: i64,
    pub request: Request,
    pub negotiation: Negotiation,
    pub consents: Vec<Consent>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    Agreed,
    NoMatch,
    Withdrawn,
}

/// One action an agent asks to perform.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    ReadFreeBusy {
        start_ms: i64,
        end_ms: i64,
    },
    ReadEventDetails {
        start_ms: i64,
        end_ms: i64,
    },
    Handshake {
        peer: String,
    },
    SendPropose {
        negotiation: String,
        round: u32,
        candidates: Vec<Candidate>,
        note: String,
    },
    SendRespond {
        negotiation: String,
        round: u32,
        answers: Vec<Answer>,
        note: String,
    },
    SendDecide {
        negotiation: String,
        round: u32,
        decisions: Vec<Decision>,
        note: String,
    },
    SendClose {
        negotiation: String,
        reason: CloseReason,
        note: String,
    },
    AddMeeting {
        negotiation: String,
        start_ms: i64,
        length_ms: i64,
    },
}

impl Action {
    /// The action's name as used in plans and audit records.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::ReadFreeBusy { .. } => "read_free_busy",
            Self::ReadEventDetails { .. } => "read_event_details",
            Self::Handshake { .. } => "handshake",
            Self::SendPropose { .. } => "send_propose",
            Self::SendRespond { .. } => "send_respond",
            Self::SendDecide { .. } => "send_decide",
            Self::SendClose { .. } => "send_close",
            Self::AddMeeting { .. } => "add_meeting",
        }
    }

    /// The free-text note of a send action.
    #[must_use]
    pub fn note(&self) -> Option<&str> {
        match self {
            Self::SendPropose { note, .. }
            | Self::SendRespond { note, .. }
            | Self::SendDecide { note, .. }
            | Self::SendClose { note, .. } => Some(note),
            _ => None,
        }
    }
}

/// The policy theory's parameters. The gate reads them from Isabelle when it starts, so denial
/// messages always quote the limits the proofs used.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Parameters {
    pub max_candidates: u64,
    pub max_rounds: u64,
    pub horizon_ms: i64,
    pub grid_ms: i64,
    pub consent_ttl_ms: i64,
    pub note_max: u64,
}

impl Parameters {
    /// The values `Fabric_Policy.thy` defines today; a live test checks the theory against them.
    pub const DEFAULT: Self = Self {
        max_candidates: 3,
        max_rounds: 3,
        horizon_ms: 14 * 24 * 60 * 60 * 1_000,
        grid_ms: 15 * 60 * 1_000,
        consent_ttl_ms: 24 * 60 * 60 * 1_000,
        note_max: 200,
    };
}

/// The rules of the policy theory, in the order of its `all_rules` list.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    ViewDetailsNever,
    ReadWindow,
    CandCount,
    AnswerCount,
    RoundLimit,
    Horizon,
    Grid,
    ConsentLifetime,
    Note,
    HandshakeTarget,
    Consent,
    WellFormed,
}

impl Rule {
    pub const ALL: [Self; 12] = [
        Self::ViewDetailsNever,
        Self::ReadWindow,
        Self::CandCount,
        Self::AnswerCount,
        Self::RoundLimit,
        Self::Horizon,
        Self::Grid,
        Self::ConsentLifetime,
        Self::Note,
        Self::HandshakeTarget,
        Self::Consent,
        Self::WellFormed,
    ];

    /// The rule's name in the specification and in denial messages.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ViewDetailsNever => "view_details_never",
            Self::ReadWindow => "read_window",
            Self::CandCount => "cand_count",
            Self::AnswerCount => "answer_count",
            Self::RoundLimit => "round_limit",
            Self::Horizon => "horizon",
            Self::Grid => "grid",
            Self::ConsentLifetime => "consent_lifetime",
            Self::Note => "note",
            Self::HandshakeTarget => "handshake_target",
            Self::Consent => "consent",
            Self::WellFormed => "well_formed",
        }
    }

    /// The constructor of the theory's `rule` datatype.
    #[must_use]
    pub const fn hol_constructor(self) -> &'static str {
        match self {
            Self::ViewDetailsNever => "View_details_never",
            Self::ReadWindow => "Read_window",
            Self::CandCount => "Cand_count",
            Self::AnswerCount => "Answer_count",
            Self::RoundLimit => "Round_limit",
            Self::Horizon => "Horizon",
            Self::Grid => "Grid",
            Self::ConsentLifetime => "Consent_lifetime",
            Self::Note => "Note",
            Self::HandshakeTarget => "Handshake_target",
            Self::Consent => "Consent",
            Self::WellFormed => "Well_formed",
        }
    }
}
