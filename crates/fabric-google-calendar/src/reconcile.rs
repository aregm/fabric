//! The decision after an unknown outcome (design section 7, ADR-113, PD20).
//!
//! A change whose outcome is unknown is never sent again. The broker reads the event
//! ([`crate::calendar::Google::reconcile_read`], which answers a [`ReadBack`] that names
//! nothing of Google's shape) and decides with [`decide`], the table of section 7 as plain
//! code. The lead's answer of 2026-11-17 is that the core owns this table (18.5); this copy is
//! for the core to take over, with its tests.

use std::fmt;

use fabric_desktop_contracts::agent::{Action, ResponseTarget};
use fabric_desktop_contracts::scalar::ProviderRef;

use crate::outcome::{Notification, Sentence};

/// The earliest a first reconcile read may run after the attempt ended (PD20).
pub const FIRST_READ_DELAY_MS: u64 = 10_000;

/// The least time between the two reads that confirm an absence (PD20).
pub const SECOND_READ_DELAY_MS: u64 = 60_000;

/// The changing tools (6.6 to 6.8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChangeTool {
    /// `fabric_calendar_propose_create` (6.6).
    Create,
    /// `fabric_calendar_propose_update` (6.7).
    Update,
    /// `fabric_calendar_propose_invitation` (6.7).
    Invitation,
    /// `fabric_calendar_propose_response` (6.7).
    Response,
    /// `fabric_calendar_propose_delete` (6.8).
    Delete,
}

impl ChangeTool {
    /// The changing tool of `action`, or `None` for a tool that changes no calendar.
    #[must_use]
    pub fn of(action: &Action) -> Option<ChangeTool> {
        match action {
            Action::ProposeCreate(_) => Some(ChangeTool::Create),
            Action::ProposeUpdate(_) => Some(ChangeTool::Update),
            Action::ProposeInvitation(_) => Some(ChangeTool::Invitation),
            Action::ProposeResponse(_) => Some(ChangeTool::Response),
            Action::ProposeDelete(_) => Some(ChangeTool::Delete),
            Action::CalendarList(_)
            | Action::CalendarAvailability(_)
            | Action::CalendarRead(_)
            | Action::RequestResult(_) => None,
        }
    }
}

/// One approved change: the action and, for a create, the id the core reserved (PD16). An
/// [`crate::calendar::ChangeOrder`] carries one; a reconcile order names one (PO4).
#[derive(Clone, PartialEq, Eq)]
pub struct Effect {
    /// The approved action, exactly as proven.
    pub action: Action,
    /// The reserved id of a create, unchecked here; it is checked before any request (5.3).
    pub reserved_id: Option<String>,
}

impl Effect {
    /// An effect of `action`, with the reserved id of a create.
    #[must_use]
    pub fn new(action: Action, reserved_id: Option<String>) -> Effect {
        Effect {
            action,
            reserved_id,
        }
    }

    /// The changing tool, or `None` when the action changes no calendar.
    #[must_use]
    pub fn tool(&self) -> Option<ChangeTool> {
        ChangeTool::of(&self.action)
    }

    /// The revision the approval names; `None` for a create and for an action that changes
    /// nothing.
    #[must_use]
    pub fn approved_revision(&self) -> Option<&ProviderRef> {
        match &self.action {
            Action::ProposeUpdate(a) => Some(&a.event.revision),
            Action::ProposeInvitation(a) => Some(&a.event.revision),
            Action::ProposeDelete(a) => Some(&a.event.revision),
            Action::ProposeResponse(a) => Some(match &a.target {
                ResponseTarget::Event(event) => &event.revision,
                ResponseTarget::Occurrence(occurrence) => &occurrence.revision,
            }),
            _ => None,
        }
    }
}

/// Prints the tool and whether a reserved id is present; never content or an id (PD27).
impl fmt::Debug for Effect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Effect")
            .field("tool", &self.tool())
            .field("reserved_id", &self.reserved_id.as_ref().map(|_| "present"))
            .finish()
    }
}

/// What Google holds under the effect's event, and nothing of Google's shape (section 7, PO4).
#[derive(Clone, PartialEq, Eq)]
pub enum ReadBack {
    /// An event that is not cancelled.
    Found {
        /// Its revision (the etag, verbatim).
        revision: ProviderRef,
        /// Whether it equals what was approved (6.12).
        matches: bool,
    },
    /// The event exists with `status: cancelled`.
    Cancelled,
    /// 404 or 410.
    NotFound,
}

/// Prints the case and `matches`; never the revision.
impl fmt::Debug for ReadBack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadBack::Found { matches, .. } => {
                f.debug_struct("Found").field("matches", matches).finish()
            }
            ReadBack::Cancelled => f.write_str("Cancelled"),
            ReadBack::NotFound => f.write_str("NotFound"),
        }
    }
}

/// Why the owner must look at a request (section 7). Each has a fixed sentence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NeedsOwnerReason {
    /// An event exists under the request's id and is not what was approved.
    CreateDiffers,
    /// The event was created and then deleted.
    CreatedThenDeleted,
    /// The event changed, and Fabric cannot tell whether its change is part of it.
    ChangedUnclear,
    /// The event is gone, and Fabric cannot tell whether its change was made first.
    GoneUnclear,
}

impl NeedsOwnerReason {
    /// The fixed sentence.
    #[must_use]
    pub const fn sentence(self) -> Sentence {
        match self {
            NeedsOwnerReason::CreateDiffers => Sentence::ReconcileCreateDiffers,
            NeedsOwnerReason::CreatedThenDeleted => Sentence::ReconcileCreatedThenDeleted,
            NeedsOwnerReason::ChangedUnclear => Sentence::ReconcileChangedUnclear,
            NeedsOwnerReason::GoneUnclear => Sentence::ReconcileGoneUnclear,
        }
    }
}

/// The decision of section 7.
#[derive(Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The change is in effect (`reconciling -> applied`).
    Applied {
        /// The revision Google holds now; `None` after a deletion. The event id is the
        /// effect's own (the reserved id of a create, the approved event or instance id
        /// otherwise), which the caller has.
        revision: Option<ProviderRef>,
        /// True: the read showed exactly what was approved (or, for a deletion, that the event
        /// is gone).
        confirmed: bool,
        /// `NotRequested` for a create (`sendUpdates=none`, PD16); `Unknown` for every other
        /// tool: whether Google sent messages is not known (PLAN 11).
        notification: Notification,
    },
    /// The change is not there and cannot appear any more (`reconciling ->
    /// absence_confirmed`).
    Absent,
    /// Not there now; read again no sooner than `not_before_ms` (PD20).
    ReadAgain {
        /// The earliest time of the next read (Unix ms).
        not_before_ms: u64,
    },
    /// The owner must look (`needs_owner_resolution`).
    NeedsOwner(NeedsOwnerReason),
}

/// Prints the case and fixed parts; never the revision.
impl fmt::Debug for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Verdict::Applied {
                revision,
                confirmed,
                notification,
            } => f
                .debug_struct("Applied")
                .field("revision", &revision.as_ref().map(|_| "present"))
                .field("confirmed", confirmed)
                .field("notification", notification)
                .finish(),
            Verdict::Absent => f.write_str("Absent"),
            Verdict::ReadAgain { not_before_ms } => f
                .debug_struct("ReadAgain")
                .field("not_before_ms", not_before_ms)
                .finish(),
            Verdict::NeedsOwner(reason) => f.debug_tuple("NeedsOwner").field(reason).finish(),
        }
    }
}

/// The table of section 7. `approved_revision` is the revision the approval names (`None` for
/// a create); `reads_before` counts the reconcile reads of this effect before this one;
/// `read_back` is what this read found, at `read_at_ms`.
///
/// `NotYet` (PD20): on the first read it is `ReadAgain`, no sooner than 60 s after this read;
/// on a later read it is `Absent`.
#[must_use]
pub fn decide(
    tool: ChangeTool,
    approved_revision: Option<&ProviderRef>,
    reads_before: u32,
    read_back: &ReadBack,
    read_at_ms: u64,
) -> Verdict {
    let not_yet = || {
        if reads_before == 0 {
            Verdict::ReadAgain {
                not_before_ms: read_at_ms.saturating_add(SECOND_READ_DELAY_MS),
            }
        } else {
            Verdict::Absent
        }
    };
    let same_revision = |found: &ProviderRef| approved_revision == Some(found);
    match (tool, read_back) {
        (ChangeTool::Create, ReadBack::Found { revision, matches }) => {
            if *matches {
                Verdict::Applied {
                    revision: Some(revision.clone()),
                    confirmed: true,
                    notification: Notification::NotRequested,
                }
            } else {
                Verdict::NeedsOwner(NeedsOwnerReason::CreateDiffers)
            }
        }
        (ChangeTool::Create, ReadBack::Cancelled) => {
            Verdict::NeedsOwner(NeedsOwnerReason::CreatedThenDeleted)
        }
        (ChangeTool::Create, ReadBack::NotFound) => not_yet(),
        (
            ChangeTool::Update | ChangeTool::Invitation | ChangeTool::Response,
            ReadBack::Found { revision, matches },
        ) => {
            if *matches {
                Verdict::Applied {
                    revision: Some(revision.clone()),
                    confirmed: true,
                    notification: Notification::Unknown,
                }
            } else if same_revision(revision) {
                not_yet()
            } else {
                Verdict::NeedsOwner(NeedsOwnerReason::ChangedUnclear)
            }
        }
        (
            ChangeTool::Update | ChangeTool::Invitation | ChangeTool::Response,
            ReadBack::Cancelled | ReadBack::NotFound,
        ) => Verdict::NeedsOwner(NeedsOwnerReason::GoneUnclear),
        (ChangeTool::Delete, ReadBack::Cancelled | ReadBack::NotFound) => Verdict::Applied {
            revision: None,
            confirmed: true,
            notification: Notification::Unknown,
        },
        (ChangeTool::Delete, ReadBack::Found { revision, .. }) => {
            if same_revision(revision) {
                not_yet()
            } else {
                // It exists and changed: the conditional delete cannot have been made and cannot
                // be made later.
                Verdict::Absent
            }
        }
    }
}

/// The earliest time of the first reconcile read: the attempt's end plus 10 s (PD20). After a
/// restart the core passes the start of its recovery (CO8).
#[must_use]
pub const fn earliest_read_ms(attempt_finished_ms: u64) -> u64 {
    attempt_finished_ms.saturating_add(FIRST_READ_DELAY_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rev(n: u32) -> ProviderRef {
        ProviderRef::new(format!("\"{n}\"")).expect("valid")
    }

    fn found(n: u32, matches: bool) -> ReadBack {
        ReadBack::Found {
            revision: rev(n),
            matches,
        }
    }

    const T: u64 = 5_000_000;

    #[test]
    fn create_rows() {
        let d = |back: &ReadBack, reads| decide(ChangeTool::Create, None, reads, back, T);
        assert_eq!(
            d(&found(7, true), 0),
            Verdict::Applied {
                revision: Some(rev(7)),
                confirmed: true,
                notification: Notification::NotRequested
            }
        );
        assert_eq!(
            d(&found(7, false), 0),
            Verdict::NeedsOwner(NeedsOwnerReason::CreateDiffers)
        );
        assert_eq!(
            d(&ReadBack::Cancelled, 0),
            Verdict::NeedsOwner(NeedsOwnerReason::CreatedThenDeleted)
        );
        assert_eq!(
            d(&ReadBack::NotFound, 0),
            Verdict::ReadAgain {
                not_before_ms: T + 60_000
            }
        );
        assert_eq!(d(&ReadBack::NotFound, 1), Verdict::Absent);
        assert_eq!(d(&ReadBack::NotFound, 5), Verdict::Absent);
    }

    #[test]
    fn update_invitation_and_response_rows() {
        for tool in [
            ChangeTool::Update,
            ChangeTool::Invitation,
            ChangeTool::Response,
        ] {
            let approved = rev(1);
            let d = |back: &ReadBack, reads| decide(tool, Some(&approved), reads, back, T);
            // Found and equal, whatever the revision.
            for n in [1, 2] {
                assert_eq!(
                    d(&found(n, true), 0),
                    Verdict::Applied {
                        revision: Some(rev(n)),
                        confirmed: true,
                        notification: Notification::Unknown
                    }
                );
            }
            // Found, the approved revision, not equal: not yet.
            assert_eq!(
                d(&found(1, false), 0),
                Verdict::ReadAgain {
                    not_before_ms: T + 60_000
                }
            );
            assert_eq!(d(&found(1, false), 1), Verdict::Absent);
            // Found, another revision, not equal.
            assert_eq!(
                d(&found(2, false), 0),
                Verdict::NeedsOwner(NeedsOwnerReason::ChangedUnclear)
            );
            for back in [ReadBack::Cancelled, ReadBack::NotFound] {
                assert_eq!(
                    d(&back, 0),
                    Verdict::NeedsOwner(NeedsOwnerReason::GoneUnclear)
                );
                assert_eq!(
                    d(&back, 1),
                    Verdict::NeedsOwner(NeedsOwnerReason::GoneUnclear)
                );
            }
        }
    }

    #[test]
    fn delete_rows() {
        let approved = rev(1);
        let d =
            |back: &ReadBack, reads| decide(ChangeTool::Delete, Some(&approved), reads, back, T);
        for back in [ReadBack::Cancelled, ReadBack::NotFound] {
            assert_eq!(
                d(&back, 0),
                Verdict::Applied {
                    revision: None,
                    confirmed: true,
                    notification: Notification::Unknown
                }
            );
        }
        for matches in [true, false] {
            assert_eq!(
                d(&found(1, matches), 0),
                Verdict::ReadAgain {
                    not_before_ms: T + 60_000
                }
            );
            assert_eq!(d(&found(1, matches), 1), Verdict::Absent);
            assert_eq!(d(&found(2, matches), 0), Verdict::Absent);
        }
    }

    #[test]
    fn the_first_read_waits_10_seconds_after_the_attempt() {
        assert_eq!(earliest_read_ms(1_000), 11_000);
        assert_eq!(earliest_read_ms(u64::MAX), u64::MAX);
        assert_eq!(
            decide(ChangeTool::Create, None, 0, &ReadBack::NotFound, u64::MAX),
            Verdict::ReadAgain {
                not_before_ms: u64::MAX
            }
        );
    }

    #[test]
    fn every_reason_has_its_own_sentence() {
        let reasons = [
            NeedsOwnerReason::CreateDiffers,
            NeedsOwnerReason::CreatedThenDeleted,
            NeedsOwnerReason::ChangedUnclear,
            NeedsOwnerReason::GoneUnclear,
        ];
        let texts: std::collections::HashSet<_> =
            reasons.iter().map(|r| r.sentence().text()).collect();
        assert_eq!(texts.len(), 4);
    }

    #[test]
    fn debug_prints_no_revision() {
        let text = format!(
            "{:?} {:?}",
            found(987_654, true),
            decide(ChangeTool::Create, None, 0, &found(987_654, true), T)
        );
        assert!(!text.contains("987654"), "{text}");
    }
}
