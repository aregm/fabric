//! Deterministic two-agent meeting agreement semantics.
//!
//! This module is the executable `fabric-schedule-sim/0` profile. It deliberately
//! stops at an unsigned [`MeetingAgreement`]. Network transport, E2EE, signed
//! consent, persistence, holds, provider writes, and the commit saga remain
//! production work described in `docs/calendar.md`.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{self, Write as _};

use crate::calendar::{LocalCalendar, UtcInterval};

/// Maximum disclosure in the one-round simulator profile.
pub const MAX_CANDIDATES: usize = 3;

macro_rules! identifier {
    ($name:ident, $kind:literal) => {
        #[doc = concat!("Protocol-scoped ", $kind, " identifier in the simulator profile.")]
        #[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            /// Creates a bounded, log-safe simulator identifier.
            pub fn new(value: impl Into<String>) -> Result<Self, RendezvousError> {
                let value = value.into();
                if value.is_empty()
                    || value.len() > 64
                    || !value.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
                    })
                {
                    return Err(RendezvousError::InvalidIdentifier($kind));
                }
                Ok(Self(value))
            }

            /// Returns the identifier text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

identifier!(NegotiationId, "negotiation");
identifier!(MemberId, "member");
identifier!(CandidateId, "candidate");

/// Provider projection authorized for the eventual meeting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionProfile {
    /// Keep the agreed meeting in Fabric only.
    FabricOnly,
    /// A later commit may create an owner-local generic busy block.
    OpaqueBusy,
}

impl ProjectionProfile {
    const fn as_str(self) -> &'static str {
        match self {
            Self::FabricOnly => "fabric_only",
            Self::OpaqueBusy => "opaque_busy",
        }
    }
}

/// One disclosed candidate interval.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Candidate {
    id: CandidateId,
    interval: UtcInterval,
}

impl Candidate {
    /// Creates a candidate from an opaque ID and exact UTC interval.
    #[must_use]
    pub const fn new(id: CandidateId, interval: UtcInterval) -> Self {
        Self { id, interval }
    }

    /// Candidate ID, scoped to this negotiation.
    #[must_use]
    pub const fn id(&self) -> &CandidateId {
        &self.id
    }

    /// Exact disclosed interval.
    #[must_use]
    pub const fn interval(&self) -> UtcInterval {
        self.interval
    }
}

/// The exact owner-approved request context for one two-person negotiation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MeetingRequest {
    negotiation_id: NegotiationId,
    participants: [MemberId; 2],
    horizon: UtcInterval,
    duration_seconds: u32,
    projection: ProjectionProfile,
    opened_at_utc_ms: i64,
    expires_at_utc_ms: i64,
}

impl MeetingRequest {
    /// Creates the request context used by the semantic simulator.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        negotiation_id: NegotiationId,
        coordinator: MemberId,
        invitee: MemberId,
        horizon: UtcInterval,
        duration_seconds: u32,
        projection: ProjectionProfile,
        opened_at_utc_ms: i64,
        expires_at_utc_ms: i64,
    ) -> Result<Self, RendezvousError> {
        if coordinator == invitee {
            return Err(RendezvousError::SameParticipant);
        }
        if duration_seconds == 0 {
            return Err(RendezvousError::ZeroDuration);
        }
        if expires_at_utc_ms <= opened_at_utc_ms {
            return Err(RendezvousError::InvalidExpiry);
        }

        Ok(Self {
            negotiation_id,
            participants: [coordinator, invitee],
            horizon,
            duration_seconds,
            projection,
            opened_at_utc_ms,
            expires_at_utc_ms,
        })
    }

    /// Negotiation identifier.
    #[must_use]
    pub const fn negotiation_id(&self) -> &NegotiationId {
        &self.negotiation_id
    }

    /// Coordinator first, invitee second.
    #[must_use]
    pub const fn participants(&self) -> &[MemberId; 2] {
        &self.participants
    }

    /// Requested duration.
    #[must_use]
    pub const fn duration_seconds(&self) -> u32 {
        self.duration_seconds
    }

    /// Projection profile covered by an eventual owner decision.
    #[must_use]
    pub const fn projection(&self) -> ProjectionProfile {
        self.projection
    }

    fn participant_index(&self, member: &MemberId) -> Option<usize> {
        self.participants
            .iter()
            .position(|participant| participant == member)
    }

    fn ensure_active(&self, now_utc_ms: i64) -> Result<(), RendezvousError> {
        if now_utc_ms < self.opened_at_utc_ms || now_utc_ms >= self.expires_at_utc_ms {
            return Err(RendezvousError::Expired);
        }
        Ok(())
    }
}

/// One bounded candidate round. The simulator supports round one only.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidateRound {
    request: MeetingRequest,
    round: u8,
    candidates: Vec<Candidate>,
}

impl CandidateRound {
    /// Validates and opens the single simulator round.
    pub fn new(
        request: MeetingRequest,
        candidates: Vec<Candidate>,
    ) -> Result<Self, RendezvousError> {
        if candidates.is_empty() || candidates.len() > MAX_CANDIDATES {
            return Err(RendezvousError::InvalidCandidateCount(candidates.len()));
        }

        let mut ids = BTreeSet::new();
        let mut intervals = Vec::new();
        for candidate in &candidates {
            if !ids.insert(candidate.id.clone()) {
                return Err(RendezvousError::DuplicateCandidateId(
                    candidate.id.to_string(),
                ));
            }
            if candidate.interval.duration_seconds() != request.duration_seconds {
                return Err(RendezvousError::CandidateDurationMismatch(
                    candidate.id.to_string(),
                ));
            }
            if !request.horizon.contains(candidate.interval) {
                return Err(RendezvousError::CandidateOutsideHorizon(
                    candidate.id.to_string(),
                ));
            }
            if candidate.interval.start_utc_ms() <= request.opened_at_utc_ms {
                return Err(RendezvousError::CandidateAlreadyStarted(
                    candidate.id.to_string(),
                ));
            }
            if intervals.contains(&candidate.interval) {
                return Err(RendezvousError::DuplicateCandidateInterval(
                    candidate.id.to_string(),
                ));
            }
            intervals.push(candidate.interval);
        }

        Ok(Self {
            request,
            round: 1,
            candidates,
        })
    }

    /// Approved request context.
    #[must_use]
    pub const fn request(&self) -> &MeetingRequest {
        &self.request
    }

    /// Round number. It is always one in this profile.
    #[must_use]
    pub const fn round(&self) -> u8 {
        self.round
    }

    /// Exact disclosed candidates in deterministic selection order.
    #[must_use]
    pub fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }

    fn ensure_active(&self, now_utc_ms: i64) -> Result<(), RendezvousError> {
        self.request.ensure_active(now_utc_ms)?;
        if let Some(candidate) = self
            .candidates
            .iter()
            .find(|candidate| candidate.interval.start_utc_ms() <= now_utc_ms)
        {
            return Err(RendezvousError::CandidateAlreadyStarted(
                candidate.id.to_string(),
            ));
        }
        Ok(())
    }
}

/// One candidate-specific eligibility bit. It carries no reason or calendar data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Eligibility {
    candidate_id: CandidateId,
    available: bool,
}

impl Eligibility {
    /// Candidate to which this bit applies.
    #[must_use]
    pub const fn candidate_id(&self) -> &CandidateId {
        &self.candidate_id
    }

    /// Whether the exact candidate is locally eligible.
    #[must_use]
    pub const fn is_available(&self) -> bool {
        self.available
    }
}

/// Candidate-specific output produced by one agent's private local evaluation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EligibilityResponse {
    scope: CandidateRound,
    member_id: MemberId,
    values: Vec<Eligibility>,
}

impl EligibilityResponse {
    /// Responding member.
    #[must_use]
    pub const fn member_id(&self) -> &MemberId {
        &self.member_id
    }

    /// Fixed-order candidate bits with no explanations.
    #[must_use]
    pub fn values(&self) -> &[Eligibility] {
        &self.values
    }
}

/// The candidates that both local agents found eligible.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutualOptions {
    scope: CandidateRound,
    candidates: Vec<Candidate>,
}

impl MutualOptions {
    /// Negotiation-scoped mutually eligible candidates.
    #[must_use]
    pub fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }
}

/// An exact yes/no decision for a candidate already disclosed as mutual.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnerDecision {
    candidate_id: CandidateId,
    accepted: bool,
}

impl OwnerDecision {
    /// Creates one explicit owner decision.
    #[must_use]
    pub const fn new(candidate_id: CandidateId, accepted: bool) -> Self {
        Self {
            candidate_id,
            accepted,
        }
    }

    /// Candidate to which this decision applies.
    #[must_use]
    pub const fn candidate_id(&self) -> &CandidateId {
        &self.candidate_id
    }

    /// The owner's explicit decision.
    #[must_use]
    pub const fn is_accepted(&self) -> bool {
        self.accepted
    }
}

/// One owner's decisions, bound structurally to the exact mutual-options scope.
///
/// Production `fabric-schedule/1` replaces the private structural scope with a
/// deterministic digest plus signature. This simulator makes no authority claim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionResponse {
    scope: MutualOptions,
    member_id: MemberId,
    values: Vec<OwnerDecision>,
}

impl DecisionResponse {
    /// Responding member.
    #[must_use]
    pub const fn member_id(&self) -> &MemberId {
        &self.member_id
    }

    /// Fixed-order yes/no decisions.
    #[must_use]
    pub fn values(&self) -> &[OwnerDecision] {
        &self.values
    }
}

/// One local scheduler endpoint with an intentionally opaque private calendar.
#[derive(Clone)]
pub struct SchedulingAgent {
    member_id: MemberId,
    calendar: LocalCalendar,
}

impl SchedulingAgent {
    /// Creates an agent for one required participant.
    #[must_use]
    pub const fn new(member_id: MemberId, calendar: LocalCalendar) -> Self {
        Self {
            member_id,
            calendar,
        }
    }

    /// Evaluates only the exact disclosed candidates against local state.
    pub fn evaluate(
        &self,
        round: &CandidateRound,
        now_utc_ms: i64,
    ) -> Result<EligibilityResponse, RendezvousError> {
        round.ensure_active(now_utc_ms)?;
        if round.request.participant_index(&self.member_id).is_none() {
            return Err(RendezvousError::UnknownParticipant(
                self.member_id.to_string(),
            ));
        }

        let values = round
            .candidates
            .iter()
            .map(|candidate| Eligibility {
                candidate_id: candidate.id.clone(),
                available: self.calendar.is_available(candidate.interval),
            })
            .collect();

        Ok(EligibilityResponse {
            scope: round.clone(),
            member_id: self.member_id.clone(),
            values,
        })
    }

    /// Records explicit owner choices; it does not infer or manufacture consent.
    pub fn record_owner_decisions(
        &self,
        options: &MutualOptions,
        decisions: &[OwnerDecision],
        now_utc_ms: i64,
    ) -> Result<DecisionResponse, RendezvousError> {
        options.scope.ensure_active(now_utc_ms)?;
        if options
            .scope
            .request
            .participant_index(&self.member_id)
            .is_none()
        {
            return Err(RendezvousError::UnknownParticipant(
                self.member_id.to_string(),
            ));
        }

        if !same_candidate_order(
            &options.candidates,
            decisions.iter().map(|decision| &decision.candidate_id),
        ) {
            return Err(RendezvousError::CandidateVectorMismatch);
        }

        Ok(DecisionResponse {
            scope: options.clone(),
            member_id: self.member_id.clone(),
            values: decisions.to_vec(),
        })
    }
}

/// Observable state of the one-round semantic state machine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NegotiationState {
    /// Waiting for one eligibility vector from each participant.
    AwaitingEligibility,
    /// Waiting for one explicit decision vector from each participant.
    AwaitingDecisions,
    /// An exact option was unanimously accepted.
    Agreed,
    /// No mutually eligible and unanimously accepted option exists.
    NoMatch,
}

impl fmt::Display for NegotiationState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AwaitingEligibility => formatter.write_str("awaiting_eligibility"),
            Self::AwaitingDecisions => formatter.write_str("awaiting_decisions"),
            Self::Agreed => formatter.write_str("agreed"),
            Self::NoMatch => formatter.write_str("no_match"),
        }
    }
}

/// Result of submitting an envelope to the idempotent in-memory state machine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmissionOutcome {
    /// The envelope was new and recorded.
    Recorded,
    /// The exact same participant envelope was already recorded.
    DuplicateIgnored,
}

/// Exact result shared by both simulated agents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MeetingAgreement {
    round: CandidateRound,
    selected_candidate: Candidate,
    agreement_key: String,
}

impl MeetingAgreement {
    /// Approved request context.
    #[must_use]
    pub const fn request(&self) -> &MeetingRequest {
        &self.round.request
    }

    /// Full candidate round and original deterministic selection order.
    #[must_use]
    pub const fn candidate_round(&self) -> &CandidateRound {
        &self.round
    }

    /// First unanimously accepted candidate in original proposal order.
    #[must_use]
    pub const fn selected_candidate(&self) -> &Candidate {
        &self.selected_candidate
    }

    /// Stable length-prefixed key used to compare two local simulator records.
    ///
    /// This is not a cryptographic digest or signature.
    #[must_use]
    pub fn agreement_key(&self) -> &str {
        &self.agreement_key
    }
}

/// Deterministic transcript reducer for exactly two required agents.
///
/// The simulator may run one instance per endpoint to verify convergence. It
/// does not enforce the production protocol's participant-specific visibility.
pub struct TwoAgentNegotiation {
    round: CandidateRound,
    state: NegotiationState,
    eligibility: [Option<EligibilityResponse>; 2],
    mutual_options: Option<MutualOptions>,
    decisions: [Option<DecisionResponse>; 2],
    agreement: Option<MeetingAgreement>,
}

impl TwoAgentNegotiation {
    /// Opens a validated one-round negotiation.
    pub fn new(round: CandidateRound, now_utc_ms: i64) -> Result<Self, RendezvousError> {
        round.ensure_active(now_utc_ms)?;
        Ok(Self {
            round,
            state: NegotiationState::AwaitingEligibility,
            eligibility: [None, None],
            mutual_options: None,
            decisions: [None, None],
            agreement: None,
        })
    }

    /// Current state.
    #[must_use]
    pub const fn state(&self) -> NegotiationState {
        self.state
    }

    /// Mutual options once both eligibility responses have arrived.
    #[must_use]
    pub const fn mutual_options(&self) -> Option<&MutualOptions> {
        self.mutual_options.as_ref()
    }

    /// Final unsigned agreement when state is [`NegotiationState::Agreed`].
    #[must_use]
    pub const fn agreement(&self) -> Option<&MeetingAgreement> {
        self.agreement.as_ref()
    }

    /// Records one candidate-specific eligibility vector.
    pub fn submit_eligibility(
        &mut self,
        response: EligibilityResponse,
        now_utc_ms: i64,
    ) -> Result<SubmissionOutcome, RendezvousError> {
        self.validate_eligibility(&response)?;
        let index = self.participant_index(&response.member_id)?;

        if let Some(existing) = &self.eligibility[index] {
            return if existing == &response {
                Ok(SubmissionOutcome::DuplicateIgnored)
            } else {
                Err(RendezvousError::ConflictingReplay(
                    response.member_id.to_string(),
                ))
            };
        }
        self.round.ensure_active(now_utc_ms)?;
        if self.state != NegotiationState::AwaitingEligibility {
            return Err(RendezvousError::InvalidState {
                expected: NegotiationState::AwaitingEligibility,
                actual: self.state,
            });
        }

        self.eligibility[index] = Some(response);
        if let [Some(first), Some(second)] = &self.eligibility {
            let candidates = self
                .round
                .candidates
                .iter()
                .zip(&first.values)
                .zip(&second.values)
                .filter(|(_, second_value)| second_value.available)
                .filter(|((_, first_value), _)| first_value.available)
                .map(|((candidate, _), _)| candidate.clone())
                .collect::<Vec<_>>();

            if candidates.is_empty() {
                self.state = NegotiationState::NoMatch;
            } else {
                self.mutual_options = Some(MutualOptions {
                    scope: self.round.clone(),
                    candidates,
                });
                self.state = NegotiationState::AwaitingDecisions;
            }
        }

        Ok(SubmissionOutcome::Recorded)
    }

    /// Records one owner's exact decisions and selects deterministically when complete.
    pub fn submit_decisions(
        &mut self,
        response: DecisionResponse,
        now_utc_ms: i64,
    ) -> Result<SubmissionOutcome, RendezvousError> {
        let options = self
            .mutual_options
            .as_ref()
            .ok_or(RendezvousError::InvalidState {
                expected: NegotiationState::AwaitingDecisions,
                actual: self.state,
            })?;
        self.validate_decisions(&response, options)?;
        let index = self.participant_index(&response.member_id)?;

        if let Some(existing) = &self.decisions[index] {
            return if existing == &response {
                Ok(SubmissionOutcome::DuplicateIgnored)
            } else {
                Err(RendezvousError::ConflictingReplay(
                    response.member_id.to_string(),
                ))
            };
        }
        self.round.ensure_active(now_utc_ms)?;
        if self.state != NegotiationState::AwaitingDecisions {
            return Err(RendezvousError::InvalidState {
                expected: NegotiationState::AwaitingDecisions,
                actual: self.state,
            });
        }

        self.decisions[index] = Some(response);
        if let [Some(first), Some(second)] = &self.decisions {
            let selected = options
                .candidates
                .iter()
                .zip(&first.values)
                .zip(&second.values)
                .find(|((_, first_value), second_value)| {
                    first_value.accepted && second_value.accepted
                })
                .map(|((candidate, _), _)| candidate.clone());

            if let Some(selected_candidate) = selected {
                let agreement_key = canonical_agreement_key(&self.round, &selected_candidate);
                self.agreement = Some(MeetingAgreement {
                    round: self.round.clone(),
                    selected_candidate,
                    agreement_key,
                });
                self.state = NegotiationState::Agreed;
            } else {
                self.state = NegotiationState::NoMatch;
            }
        }

        Ok(SubmissionOutcome::Recorded)
    }

    fn participant_index(&self, member: &MemberId) -> Result<usize, RendezvousError> {
        self.round
            .request
            .participant_index(member)
            .ok_or_else(|| RendezvousError::UnknownParticipant(member.to_string()))
    }

    fn validate_eligibility(&self, response: &EligibilityResponse) -> Result<(), RendezvousError> {
        if response.scope != self.round {
            return Err(RendezvousError::EnvelopeScopeMismatch);
        }
        self.participant_index(&response.member_id)?;
        if !same_candidate_order(
            &self.round.candidates,
            response.values.iter().map(|value| &value.candidate_id),
        ) {
            return Err(RendezvousError::CandidateVectorMismatch);
        }
        Ok(())
    }

    fn validate_decisions(
        &self,
        response: &DecisionResponse,
        options: &MutualOptions,
    ) -> Result<(), RendezvousError> {
        if response.scope != *options {
            return Err(RendezvousError::EnvelopeScopeMismatch);
        }
        self.participant_index(&response.member_id)?;
        if !same_candidate_order(
            &options.candidates,
            response.values.iter().map(|value| &value.candidate_id),
        ) {
            return Err(RendezvousError::CandidateVectorMismatch);
        }
        Ok(())
    }
}

fn same_candidate_order<'a>(
    candidates: &[Candidate],
    ids: impl Iterator<Item = &'a CandidateId>,
) -> bool {
    candidates.iter().map(|candidate| &candidate.id).eq(ids)
}

fn canonical_agreement_key(round: &CandidateRound, candidate: &Candidate) -> String {
    fn field(output: &mut String, value: &str) {
        let _ = write!(output, "{}:{value};", value.len());
    }

    let request = &round.request;
    let mut output = String::from("fabric-schedule-sim/0;");
    field(&mut output, request.negotiation_id.as_str());
    for participant in &request.participants {
        field(&mut output, participant.as_str());
    }
    field(&mut output, request.projection.as_str());
    field(&mut output, "first_unanimous_in_proposal_order");
    let _ = write!(
        output,
        "{};{};{};{};{};{};{};",
        request.horizon.start_utc_ms(),
        request.horizon.duration_seconds(),
        request.duration_seconds,
        request.opened_at_utc_ms,
        request.expires_at_utc_ms,
        round.round,
        round.candidates.len()
    );
    for proposed in &round.candidates {
        field(&mut output, proposed.id.as_str());
        let _ = write!(
            output,
            "{};{};",
            proposed.interval.start_utc_ms(),
            proposed.interval.duration_seconds()
        );
    }
    field(&mut output, candidate.id.as_str());
    output
}

/// Semantic validation failures for `fabric-schedule-sim/0`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RendezvousError {
    /// A protocol-scoped identifier was malformed.
    InvalidIdentifier(&'static str),
    /// The coordinator and invitee must be different people.
    SameParticipant,
    /// The request duration was zero.
    ZeroDuration,
    /// Consent expiry must be later than request opening.
    InvalidExpiry,
    /// The round disclosed zero or more than three candidates.
    InvalidCandidateCount(usize),
    /// Candidate IDs must be unique within the round.
    DuplicateCandidateId(String),
    /// Different candidate IDs may not alias the same exact interval.
    DuplicateCandidateInterval(String),
    /// Every candidate must have the request's exact duration.
    CandidateDurationMismatch(String),
    /// Every candidate must be inside the approved horizon.
    CandidateOutsideHorizon(String),
    /// A candidate must still be in the future when a stage executes.
    CandidateAlreadyStarted(String),
    /// The request was not yet valid or had expired.
    Expired,
    /// A sender was not one of the two required participants.
    UnknownParticipant(String),
    /// An envelope was produced for different exact request/options data.
    EnvelopeScopeMismatch,
    /// A response did not contain the exact candidate IDs in exact order.
    CandidateVectorMismatch,
    /// A participant resent a different envelope for the same stage.
    ConflictingReplay(String),
    /// The message was not allowed in the current state.
    InvalidState {
        /// State required by this message.
        expected: NegotiationState,
        /// Actual state when it arrived.
        actual: NegotiationState,
    },
}

impl fmt::Display for RendezvousError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentifier(kind) => write!(
                formatter,
                "{kind} identifier must be 1..=64 ASCII letters, digits, '.', '_', or '-'"
            ),
            Self::SameParticipant => formatter.write_str("the two participants must be distinct"),
            Self::ZeroDuration => formatter.write_str("meeting duration must be non-zero"),
            Self::InvalidExpiry => formatter.write_str("expiry must be later than opening time"),
            Self::InvalidCandidateCount(count) => {
                write!(
                    formatter,
                    "candidate count must be 1..={MAX_CANDIDATES}, got {count}"
                )
            }
            Self::DuplicateCandidateId(id) => write!(formatter, "duplicate candidate ID: {id}"),
            Self::DuplicateCandidateInterval(id) => {
                write!(formatter, "candidate {id} duplicates an exact interval")
            }
            Self::CandidateDurationMismatch(id) => {
                write!(formatter, "candidate {id} has the wrong duration")
            }
            Self::CandidateOutsideHorizon(id) => {
                write!(formatter, "candidate {id} is outside the approved horizon")
            }
            Self::CandidateAlreadyStarted(id) => {
                write!(formatter, "candidate {id} has already started")
            }
            Self::Expired => formatter.write_str("the negotiation is not currently active"),
            Self::UnknownParticipant(id) => write!(formatter, "unknown participant: {id}"),
            Self::EnvelopeScopeMismatch => {
                formatter.write_str("response is bound to a different request or option set")
            }
            Self::CandidateVectorMismatch => {
                formatter.write_str("response candidate vector does not match exact proposal order")
            }
            Self::ConflictingReplay(id) => {
                write!(formatter, "participant {id} sent a conflicting replay")
            }
            Self::InvalidState { expected, actual } => {
                write!(
                    formatter,
                    "expected state {expected}, current state is {actual}"
                )
            }
        }
    }
}

impl Error for RendezvousError {}
