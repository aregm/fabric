//! The reducer (5.7): the next lifecycle move of a request from its record and an event.
//!
//! [`next`] names the state an event moves a request to, or `None` when the event does not
//! apply in that state. The pipeline asks it before every move, so no driver of W04 can produce
//! a move the lifecycle refuses; `move_request` and the store's guard check the same rule again.

use fabric_desktop_contracts::RequestState;

/// What happened to a request, as the W04 table of 5.7 drives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    /// The pump takes a received request.
    Advance,
    /// The world checks of `validating` ran.
    Checked {
        /// True when the arguments name what exists.
        valid: bool,
    },
    /// A proof starts (classification after validating; a fresh permit after an approval, an
    /// allowed classification, a voided dispatch or the end of a retry wait).
    Prove,
    /// An accepted classification Permit from Isabelle.
    Allow,
    /// An accepted fresh Permit that issued an authorization.
    Authorize,
    /// An accepted NeedApproval with a ruling: a card opens.
    Ask,
    /// An accepted Block with a ruling.
    Block,
    /// No decision: no prover, the watchdog, a refused or incomplete result, a permit that
    /// cannot authorize, or the classification counter exhausted.
    Undecide,
    /// The fresh-permit counter is exhausted.
    Exhausted,
    /// The owner decided.
    Decide {
        /// True for an approval.
        approve: bool,
    },
    /// The card's 24 hours passed.
    CardExpired,
    /// The account of a pending request was disconnected.
    AccountGone,
    /// The handoff committed.
    Dispatch,
    /// The provider applied the change.
    Applied,
    /// A transient failure within the retry budget.
    Retry,
    /// A refusal, or a transient failure past the budget.
    Fail,
    /// No answer, or a dispatch left open by a dead run.
    Unknown,
}

impl Event {
    /// Every event, for the exhaustive checks.
    #[cfg(test)]
    pub(crate) const ALL: [Event; 19] = [
        Event::Advance,
        Event::Checked { valid: true },
        Event::Checked { valid: false },
        Event::Prove,
        Event::Allow,
        Event::Authorize,
        Event::Ask,
        Event::Block,
        Event::Undecide,
        Event::Exhausted,
        Event::Decide { approve: true },
        Event::Decide { approve: false },
        Event::CardExpired,
        Event::AccountGone,
        Event::Dispatch,
        Event::Applied,
        Event::Retry,
        Event::Fail,
        Event::Unknown,
    ];
}

/// True for the event whose move states its cause (`undecided`): only `pipeline::undecide`
/// makes that move, and `pipeline::advance` refuses it.
#[must_use]
pub(crate) const fn states_a_cause(event: Event) -> bool {
    matches!(event, Event::Undecide)
}

/// The state `event` moves a request in `from` to, or `None` when it does not apply.
#[must_use]
pub(crate) fn next(from: RequestState, event: Event) -> Option<RequestState> {
    use RequestState as S;
    let to = match (from, event) {
        (S::Received, Event::Advance) => S::Validating,
        (S::Validating, Event::Checked { valid: true }) => S::ProvingClassification,
        (S::Validating, Event::Checked { valid: false }) => S::TerminalFailure,
        (S::ProvingClassification, Event::Allow) => S::Allowed,
        (S::ProvingClassification | S::ProvingFreshPermit, Event::Ask) => S::PendingApproval,
        (S::ProvingClassification | S::ProvingFreshPermit, Event::Block) => S::Blocked,
        (S::ProvingClassification | S::ProvingFreshPermit, Event::Undecide) => S::Undecided,
        (S::PendingApproval, Event::Decide { approve: true }) => S::Approved,
        (S::PendingApproval, Event::Decide { approve: false }) => S::Denied,
        (S::PendingApproval, Event::CardExpired) => S::Expired,
        (S::PendingApproval, Event::AccountGone) => S::Stale,
        (S::Approved | S::Allowed | S::Queued | S::RetryWait, Event::Prove) => {
            S::ProvingFreshPermit
        }
        (S::ProvingFreshPermit, Event::Authorize) => S::Queued,
        (S::ProvingFreshPermit, Event::Exhausted) => S::Stale,
        (S::Queued, Event::Dispatch) => S::Dispatching,
        (S::Dispatching, Event::Applied) => S::Applied,
        (S::Dispatching, Event::Retry) => S::RetryWait,
        (S::Dispatching, Event::Fail) => S::TerminalFailure,
        (S::Dispatching, Event::Unknown) => S::OutcomeUnknown,
        _ => return None,
    };
    debug_assert!(from.can_transition_to(to), "{from:?} -> {to:?}");
    Some(to)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use RequestState as S;

    /// The transitions the W04 table of 5.7 drives: 25 of the contract's 39.
    const DRIVEN: [(RequestState, RequestState); 25] = [
        (S::Received, S::Validating),
        (S::Validating, S::ProvingClassification),
        (S::Validating, S::TerminalFailure),
        (S::ProvingClassification, S::Allowed),
        (S::ProvingClassification, S::PendingApproval),
        (S::ProvingClassification, S::Blocked),
        (S::ProvingClassification, S::Undecided),
        (S::PendingApproval, S::Approved),
        (S::PendingApproval, S::Denied),
        (S::PendingApproval, S::Expired),
        (S::PendingApproval, S::Stale),
        (S::Approved, S::ProvingFreshPermit),
        (S::Allowed, S::ProvingFreshPermit),
        (S::ProvingFreshPermit, S::Queued),
        (S::ProvingFreshPermit, S::Undecided),
        (S::ProvingFreshPermit, S::Blocked),
        (S::ProvingFreshPermit, S::PendingApproval),
        (S::ProvingFreshPermit, S::Stale),
        (S::Queued, S::Dispatching),
        (S::Queued, S::ProvingFreshPermit),
        (S::Dispatching, S::Applied),
        (S::Dispatching, S::RetryWait),
        (S::Dispatching, S::TerminalFailure),
        (S::Dispatching, S::OutcomeUnknown),
        (S::RetryWait, S::ProvingFreshPermit),
    ];

    fn produced() -> BTreeSet<(RequestState, RequestState)> {
        let mut moves = BTreeSet::new();
        for from in RequestState::ALL {
            for event in Event::ALL {
                if let Some(to) = next(from, event) {
                    moves.insert((from, to));
                }
            }
        }
        moves
    }

    #[test]
    fn the_reducer_never_yields_a_move_the_lifecycle_refuses() {
        for from in RequestState::ALL {
            for event in Event::ALL {
                if let Some(to) = next(from, event) {
                    assert!(
                        from.can_transition_to(to),
                        "{from:?} -> {to:?} by {event:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_reducer_drives_exactly_the_transitions_of_the_w04_table() {
        let expected: BTreeSet<_> = DRIVEN.into_iter().collect();
        assert_eq!(expected.len(), DRIVEN.len());
        for (from, to) in &expected {
            assert!(from.can_transition_to(*to), "{from:?} -> {to:?}");
        }
        assert_eq!(produced(), expected);
    }

    #[test]
    fn no_event_moves_a_terminal_request() {
        for from in RequestState::ALL.into_iter().filter(|s| s.is_terminal()) {
            for event in Event::ALL {
                assert_eq!(next(from, event), None, "{from:?} by {event:?}");
            }
        }
    }

    #[test]
    fn the_transitions_left_to_later_packages_are_not_driven() {
        let contract: Vec<(RequestState, RequestState)> = RequestState::ALL
            .into_iter()
            .flat_map(|from| {
                RequestState::ALL
                    .into_iter()
                    .filter(move |to| from.can_transition_to(*to))
                    .map(move |to| (from, to))
            })
            .collect();
        assert_eq!(contract.len(), 42);
        let driven = produced();
        let left: Vec<_> = contract
            .into_iter()
            .filter(|pair| !driven.contains(pair))
            .collect();
        // Cancellation (no message in v1), reconciliation (CD-W05), the owner's resolution of an
        // unknown outcome (resolve_outcome, not implemented in this build), and the end of a
        // request the broker completes itself (completed, not implemented in this build).
        assert!(left.iter().all(|(from, to)| *to == S::Cancelled
            || *from == S::Reconciling
            || *to == S::Reconciling
            || *to == S::Reconciled
            || *from == S::NeedsOwnerResolution
            || *to == S::Completed));
    }
}
