//! Core domain library for the Personal Knowledge Fabric.

pub mod calendar;
pub mod rendezvous;

pub use calendar::{CalendarError, LocalCalendar, UtcInterval};
pub use rendezvous::{
    Candidate, CandidateId, CandidateRound, DecisionResponse, Eligibility, EligibilityResponse,
    MAX_CANDIDATES, MeetingAgreement, MeetingRequest, MemberId, MutualOptions, NegotiationId,
    NegotiationState, OwnerDecision, ProjectionProfile, RendezvousError, SchedulingAgent,
    SubmissionOutcome, TwoAgentNegotiation,
};

/// Returns a diagnostic string used by the initial CLI smoke test.
#[must_use]
pub const fn status() -> &'static str {
    "Personal Knowledge Fabric Rendezvous simulator is ready."
}

#[cfg(test)]
mod tests {
    use super::status;

    #[test]
    fn status_identifies_the_project() {
        assert!(status().contains("Knowledge Fabric"));
    }
}
