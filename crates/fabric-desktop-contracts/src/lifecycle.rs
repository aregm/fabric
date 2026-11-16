//! The request lifecycle of `data/request-lifecycle.json` (README section 9).

use serde::{Deserialize, Serialize};

/// `common#/$defs/request_state`: the 23 states of one logical request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestState {
    Received,
    Validating,
    ProvingClassification,
    Allowed,
    PendingApproval,
    Blocked,
    Undecided,
    Approved,
    Denied,
    Expired,
    Cancelled,
    Stale,
    ProvingFreshPermit,
    Queued,
    Dispatching,
    Applied,
    Reconciled,
    RetryWait,
    OutcomeUnknown,
    TerminalFailure,
    Reconciling,
    AbsenceConfirmed,
    NeedsOwnerResolution,
}

use RequestState as S;

impl RequestState {
    /// Every state, in the order of `data/request-lifecycle.json`.
    pub const ALL: [RequestState; 23] = [
        S::Received,
        S::Validating,
        S::ProvingClassification,
        S::Allowed,
        S::PendingApproval,
        S::Blocked,
        S::Undecided,
        S::Approved,
        S::Denied,
        S::Expired,
        S::Cancelled,
        S::Stale,
        S::ProvingFreshPermit,
        S::Queued,
        S::Dispatching,
        S::Applied,
        S::Reconciled,
        S::RetryWait,
        S::OutcomeUnknown,
        S::TerminalFailure,
        S::Reconciling,
        S::AbsenceConfirmed,
        S::NeedsOwnerResolution,
    ];

    /// A request starts here.
    pub const INITIAL: RequestState = S::Received;

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            S::Received => "received",
            S::Validating => "validating",
            S::ProvingClassification => "proving_classification",
            S::Allowed => "allowed",
            S::PendingApproval => "pending_approval",
            S::Blocked => "blocked",
            S::Undecided => "undecided",
            S::Approved => "approved",
            S::Denied => "denied",
            S::Expired => "expired",
            S::Cancelled => "cancelled",
            S::Stale => "stale",
            S::ProvingFreshPermit => "proving_fresh_permit",
            S::Queued => "queued",
            S::Dispatching => "dispatching",
            S::Applied => "applied",
            S::Reconciled => "reconciled",
            S::RetryWait => "retry_wait",
            S::OutcomeUnknown => "outcome_unknown",
            S::TerminalFailure => "terminal_failure",
            S::Reconciling => "reconciling",
            S::AbsenceConfirmed => "absence_confirmed",
            S::NeedsOwnerResolution => "needs_owner_resolution",
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<RequestState> {
        RequestState::ALL.into_iter().find(|s| s.as_str() == name)
    }

    /// A request is open while its state is not terminal. `undecided`, `absence_confirmed` and
    /// `needs_owner_resolution` are terminal in version 1 (README section 14).
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            S::Blocked
                | S::Undecided
                | S::Denied
                | S::Expired
                | S::Cancelled
                | S::Stale
                | S::Reconciled
                | S::TerminalFailure
                | S::AbsenceConfirmed
                | S::NeedsOwnerResolution
        )
    }

    /// Whether `self → next` is one of the 39 listed transitions: the 24 of plan section 11
    /// and the 15 this contract adds (README section 9).
    #[must_use]
    pub const fn can_transition_to(self, next: RequestState) -> bool {
        matches!(
            (self, next),
            // plan section 11
            (S::Received, S::Validating)
                | (S::Validating, S::ProvingClassification)
                | (
                    S::ProvingClassification,
                    S::Allowed | S::PendingApproval | S::Blocked | S::Undecided
                )
                | (
                    S::PendingApproval,
                    S::Approved | S::Denied | S::Expired | S::Cancelled | S::Stale
                )
                | (S::Approved, S::ProvingFreshPermit)
                | (S::Allowed, S::ProvingFreshPermit)
                | (S::ProvingFreshPermit, S::Queued)
                | (S::Queued, S::Dispatching)
                | (
                    S::Dispatching,
                    S::Applied | S::RetryWait | S::OutcomeUnknown | S::TerminalFailure
                )
                | (S::Applied, S::Reconciled)
                | (S::OutcomeUnknown, S::Reconciling)
                | (
                    S::Reconciling,
                    S::Applied | S::AbsenceConfirmed | S::NeedsOwnerResolution
                )
                // added: cancellation before dispatch (plan 9.2)
                | (
                    S::Received
                        | S::Validating
                        | S::ProvingClassification
                        | S::Allowed
                        | S::Approved
                        | S::ProvingFreshPermit
                        | S::Queued
                        | S::RetryWait,
                    S::Cancelled
                )
                // added: invalid arguments (plan 8, step 3)
                | (S::Validating, S::TerminalFailure)
                // added: the outcomes of a fresh proof (plan 8)
                | (
                    S::ProvingFreshPermit,
                    S::Blocked | S::Undecided | S::PendingApproval | S::Stale
                )
                // added: proved again before the handoff, and per retry (plan 5, 7 and 8)
                | (S::Queued | S::RetryWait, S::ProvingFreshPermit)
        )
    }

    /// The 20 states whose status carries nothing but the state, or `None` for
    /// `pending_approval`, `blocked` and `terminal_failure`.
    #[must_use]
    pub fn plain(self) -> Option<PlainRequestState> {
        PlainRequestState::ALL
            .into_iter()
            .find(|p| p.state() == self)
    }
}

/// The index of the first state of `states` that breaks the lifecycle, or `None` when the
/// sequence starts at `received` and every step is a listed transition. An empty sequence
/// is invalid at index 0 (README section 13, `lifecycle.json`).
#[must_use]
pub fn first_invalid(states: &[RequestState]) -> Option<usize> {
    match states.first() {
        None => return Some(0),
        Some(&first) if first != RequestState::INITIAL => return Some(0),
        Some(_) => {}
    }
    states
        .windows(2)
        .position(|pair| !pair[0].can_transition_to(pair[1]))
        .map(|i| i + 1)
}

/// `common#/$defs/plain_request_state`: every state except `pending_approval`, `blocked`
/// and `terminal_failure`, whose status carries more.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlainRequestState {
    Received,
    Validating,
    ProvingClassification,
    Allowed,
    Undecided,
    Approved,
    Denied,
    Expired,
    Cancelled,
    Stale,
    ProvingFreshPermit,
    Queued,
    Dispatching,
    Applied,
    Reconciled,
    RetryWait,
    OutcomeUnknown,
    Reconciling,
    AbsenceConfirmed,
    NeedsOwnerResolution,
}

impl PlainRequestState {
    pub const ALL: [PlainRequestState; 20] = [
        PlainRequestState::Received,
        PlainRequestState::Validating,
        PlainRequestState::ProvingClassification,
        PlainRequestState::Allowed,
        PlainRequestState::Undecided,
        PlainRequestState::Approved,
        PlainRequestState::Denied,
        PlainRequestState::Expired,
        PlainRequestState::Cancelled,
        PlainRequestState::Stale,
        PlainRequestState::ProvingFreshPermit,
        PlainRequestState::Queued,
        PlainRequestState::Dispatching,
        PlainRequestState::Applied,
        PlainRequestState::Reconciled,
        PlainRequestState::RetryWait,
        PlainRequestState::OutcomeUnknown,
        PlainRequestState::Reconciling,
        PlainRequestState::AbsenceConfirmed,
        PlainRequestState::NeedsOwnerResolution,
    ];

    #[must_use]
    pub const fn state(self) -> RequestState {
        match self {
            PlainRequestState::Received => S::Received,
            PlainRequestState::Validating => S::Validating,
            PlainRequestState::ProvingClassification => S::ProvingClassification,
            PlainRequestState::Allowed => S::Allowed,
            PlainRequestState::Undecided => S::Undecided,
            PlainRequestState::Approved => S::Approved,
            PlainRequestState::Denied => S::Denied,
            PlainRequestState::Expired => S::Expired,
            PlainRequestState::Cancelled => S::Cancelled,
            PlainRequestState::Stale => S::Stale,
            PlainRequestState::ProvingFreshPermit => S::ProvingFreshPermit,
            PlainRequestState::Queued => S::Queued,
            PlainRequestState::Dispatching => S::Dispatching,
            PlainRequestState::Applied => S::Applied,
            PlainRequestState::Reconciled => S::Reconciled,
            PlainRequestState::RetryWait => S::RetryWait,
            PlainRequestState::OutcomeUnknown => S::OutcomeUnknown,
            PlainRequestState::Reconciling => S::Reconciling,
            PlainRequestState::AbsenceConfirmed => S::AbsenceConfirmed,
            PlainRequestState::NeedsOwnerResolution => S::NeedsOwnerResolution,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_sequence_is_invalid_at_zero() {
        assert_eq!(first_invalid(&[]), Some(0));
    }

    #[test]
    fn sequence_must_start_at_received() {
        assert_eq!(first_invalid(&[S::Validating]), Some(0));
    }

    #[test]
    fn allowed_path_is_valid() {
        let path = [
            S::Received,
            S::Validating,
            S::ProvingClassification,
            S::Allowed,
            S::ProvingFreshPermit,
            S::Queued,
            S::Dispatching,
            S::Applied,
            S::Reconciled,
        ];
        assert_eq!(first_invalid(&path), None);
    }

    #[test]
    fn skipping_the_fresh_proof_is_invalid() {
        let path = [
            S::Received,
            S::Validating,
            S::ProvingClassification,
            S::Allowed,
            S::Queued,
        ];
        assert_eq!(first_invalid(&path), Some(4));
    }

    #[test]
    fn terminal_states_have_no_successor() {
        for from in RequestState::ALL.into_iter().filter(|s| s.is_terminal()) {
            for to in RequestState::ALL {
                assert!(!from.can_transition_to(to), "{from:?} -> {to:?}");
            }
        }
    }

    #[test]
    fn three_states_carry_more_than_the_state() {
        let carrying: Vec<_> = RequestState::ALL
            .into_iter()
            .filter(|s| s.plain().is_none())
            .collect();
        assert_eq!(
            carrying,
            [S::PendingApproval, S::Blocked, S::TerminalFailure]
        );
    }
}
