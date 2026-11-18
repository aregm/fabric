//! The classes of an answer and the retry rule (design 6.10, 6.11; PD17, PD18, PD27).
//!
//! A change ends in one [`ChangeOutcome`]: `Applied`, `Transient` (it certainly did not happen:
//! nothing was sent, or Google refused before acting), `Rejected` (a plain refusal, with a
//! contract failure code) or `NoAnswer` (the outcome is unknown and is reconciled, never
//! retried; section 7). A read ends in its result or one [`ReadFailure`]; a read has no
//! `NoAnswer`: an unanswered read and a 5xx are Transient (6.10).
//!
//! Beside the outcome a call may report a [`SideResult`] for the account: `AuthLost` (the
//! sign-in is gone) or `AccessChanged` (the core lists the account's calendars again) (PO6).
//!
//! Every explanation is one of the fixed sentences of [`Sentence`]. Google's error `message`
//! is never kept; its `reason` is read against the closed list of 6.10.
//!
//! The retry rule of 6.11 is [`next_attempt`] with [`RetryPolicy::PLAN`]. The adapter itself
//! never waits and never repeats (PD13).

use std::fmt;

use fabric_desktop_contracts::common::EventRef;
use fabric_desktop_contracts::common::FailureCode;
use fabric_desktop_contracts::scalar::Explanation;
use serde_json::Value;

use crate::http::Answer;
use crate::tokens::{AccessFailure, AuthLostCause, TokenStoreError};

/// The longest wait taken from Google's `Retry-After`, in seconds (6.10).
pub const RETRY_AFTER_CAP_S: u64 = 120;

/// Whether the people of an event were told of an applied change (PO6, section 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Notification {
    /// `sendUpdates` was `all` or `externalOnly` and Google answered the change itself.
    Sent,
    /// `sendUpdates` was `none` (a create always is, PD16).
    NotRequested,
    /// The change was found applied by reading; whether Google sent messages is not known.
    Unknown,
}

/// A result for the account beside the outcome of a call (PO6, 3.8, 6.10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideResult {
    /// The account's sign-in is gone; the account must connect again (CO11).
    AuthLost(AuthLostCause),
    /// Google refused for a reason that may mean the account's access changed; the core lists
    /// the account's calendars again (CO7).
    AccessChanged,
}

/// The first member of the event Google returned that is not what was approved (6.12; 18.7
/// point 2). A fixed label; it carries no content.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Differs {
    /// The title (`summary`).
    Title,
    /// The instant of the start.
    Start,
    /// The instant of the end.
    End,
    /// The zone of the start or the end, when Google returns one.
    Zone,
    /// The description.
    Description,
    /// The place (`location`).
    Location,
    /// The attendees: a create's (none), or an invitation's set of addresses.
    Attendees,
    /// The answer of the account's own entry (6.1), or the own entry itself.
    OwnResponse,
    /// Another attendee's entry, which an answer must leave as it was read.
    OtherAttendee,
    /// The original start of an occurrence (CD-Q05b).
    OriginalStart,
}

impl Differs {
    /// Every member, for the tests.
    pub const ALL: [Differs; 10] = [
        Differs::Title,
        Differs::Start,
        Differs::End,
        Differs::Zone,
        Differs::Description,
        Differs::Location,
        Differs::Attendees,
        Differs::OwnResponse,
        Differs::OtherAttendee,
        Differs::OriginalStart,
    ];

    /// The fixed label, in snake case.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Differs::Title => "title",
            Differs::Start => "start",
            Differs::End => "end",
            Differs::Zone => "zone",
            Differs::Description => "description",
            Differs::Location => "location",
            Differs::Attendees => "attendees",
            Differs::OwnResponse => "own_response",
            Differs::OtherAttendee => "other_attendee",
            Differs::OriginalStart => "original_start",
        }
    }
}

/// Prints the label only.
impl fmt::Debug for Differs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Prints the label only.
impl fmt::Display for Differs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// The outcome of one changing order (6.10, PD17, PD18).
#[derive(Clone, PartialEq, Eq)]
pub enum ChangeOutcome {
    /// Google answered the change with a plain yes (6.6 to 6.8). A 200 whose event has the
    /// right id is always this, whatever the event holds (18.7 point 2).
    Applied {
        /// The event as Google now has it: its id and new revision. `None` after a deletion.
        event: Option<EventRef>,
        /// Whether Google's answer equals what was approved (6.12). For a deletion, whether
        /// the read after it found the event gone (18.7 point 1).
        confirmed: bool,
        /// The first member that is not what was approved; `None` exactly when `confirmed`
        /// is true, and always `None` for a deletion (18.7 point 2).
        differs: Option<Differs>,
        /// Whether the people of the event were told.
        notification: Notification,
    },
    /// It certainly did not happen: nothing was sent, or Google refused before acting. The
    /// core may retry by 6.11.
    Transient {
        /// Google's wait hint (`Retry-After`, at most 120 s), in milliseconds.
        retry_after_ms: Option<u64>,
    },
    /// A plain refusal. The request ends with this code.
    Rejected {
        /// The contract's failure code.
        code: FailureCode,
        /// One of the fixed sentences of [`Sentence`].
        explanation: Explanation,
    },
    /// The request may have been applied: it was sent and no complete, readable answer
    /// arrived, or Google answered 5xx. Reconciled by reading, never sent again (section 7).
    NoAnswer,
}

/// Prints the class and fixed parts only; never an event id or revision (PD27, g16).
impl fmt::Debug for ChangeOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChangeOutcome::Applied {
                event,
                confirmed,
                differs,
                notification,
            } => f
                .debug_struct("Applied")
                .field("event", &event.as_ref().map(|_| "present"))
                .field("confirmed", confirmed)
                .field("differs", differs)
                .field("notification", notification)
                .finish(),
            ChangeOutcome::Transient { retry_after_ms } => f
                .debug_struct("Transient")
                .field("retry_after_ms", retry_after_ms)
                .finish(),
            ChangeOutcome::Rejected { code, explanation } => f
                .debug_struct("Rejected")
                .field("code", code)
                .field("explanation", &explanation.as_str())
                .finish(),
            ChangeOutcome::NoAnswer => f.write_str("NoAnswer"),
        }
    }
}

impl ChangeOutcome {
    /// `Rejected` with `code` and the sentence's text.
    #[must_use]
    pub fn rejected(code: FailureCode, sentence: Sentence) -> ChangeOutcome {
        ChangeOutcome::Rejected {
            code,
            explanation: sentence.explanation(),
        }
    }

    /// `Transient` with no wait hint.
    #[must_use]
    pub const fn transient() -> ChangeOutcome {
        ChangeOutcome::Transient {
            retry_after_ms: None,
        }
    }
}

/// What a changing order reports: the outcome and, when there is one, a result for the account
/// (PO6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeReport {
    /// The outcome of the order.
    pub outcome: ChangeOutcome,
    /// `AuthLost` or `AccessChanged`, beside the outcome.
    pub side: Option<SideResult>,
}

impl ChangeReport {
    /// A report with no side result.
    #[must_use]
    pub const fn plain(outcome: ChangeOutcome) -> ChangeReport {
        ChangeReport {
            outcome,
            side: None,
        }
    }
}

impl From<ChangeOutcome> for ChangeReport {
    fn from(outcome: ChangeOutcome) -> ChangeReport {
        ChangeReport::plain(outcome)
    }
}

/// Why a read (6.2 to 6.5, and the reads of section 7) gave no result (6.10).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadFailure {
    /// Nothing was learnt and nothing was changed; the core may repeat it by 6.11. An
    /// unanswered read and a 5xx are Transient (6.10).
    Transient {
        /// Google's wait hint (`Retry-After`, at most 120 s), in milliseconds.
        retry_after_ms: Option<u64>,
    },
    /// A plain refusal, with the contract's failure code, a fixed sentence and perhaps a
    /// result for the account.
    Rejected {
        /// The contract's failure code.
        code: FailureCode,
        /// One of the fixed sentences of [`Sentence`].
        explanation: Explanation,
        /// `AuthLost` or `AccessChanged`, beside the failure.
        side: Option<SideResult>,
    },
    /// The caller's `still_allowed` turned false before a page; nothing gathered is returned
    /// and no further request was made (6.4, PO3).
    Stopped,
}

impl ReadFailure {
    /// `Rejected` with `code`, the sentence's text and no side result.
    #[must_use]
    pub fn rejected(code: FailureCode, sentence: Sentence) -> ReadFailure {
        ReadFailure::Rejected {
            code,
            explanation: sentence.explanation(),
            side: None,
        }
    }

    /// `Transient` with no wait hint.
    #[must_use]
    pub const fn transient() -> ReadFailure {
        ReadFailure::Transient {
            retry_after_ms: None,
        }
    }

    /// The result for the account, when there is one.
    #[must_use]
    pub fn side(&self) -> Option<SideResult> {
        match self {
            ReadFailure::Rejected { side, .. } => *side,
            ReadFailure::Transient { .. } | ReadFailure::Stopped => None,
        }
    }
}

/// The fixed sentences of the adapter (PD27). Every explanation it gives is one of these; no
/// text of Google's or of a library is ever shown or stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sentence {
    /// 400 (6.10). Only when Google answered: a request the adapter refuses itself has a
    /// sentence of its own (PD27).
    InvalidRequest,
    /// `AuthLost(GrantEnded)` and 403 `insufficientPermissions` (3.8, 6.10).
    SignInEnded,
    /// `AuthLost(ClientRejected)` (3.8).
    ClientRejected,
    /// A token store that refused or failed (section 4).
    TokenStore,
    /// 403 `quotaExceeded` (6.10).
    QuotaExceeded,
    /// 403 `forbiddenForNonOrganizer` (6.10).
    OnlyOrganizer,
    /// 403 with any other reason, for a change (6.10).
    ChangeRefused,
    /// 403 with any other reason, for a read (6.10).
    ReadRefused,
    /// Any other status, a 3xx among them (6.10).
    UnexpectedAnswer,
    /// 409 on a create (6.6).
    IdTaken,
    /// 412, or a read before a change that shows another revision or another original start
    /// (6.7, 6.8, CD-Q05b).
    EventChanged,
    /// 404 or 410 on a change of an existing event, or at the read before it (6.7).
    EventGone,
    /// 404 or 410 on a deletion (6.8).
    EventAlreadyGone,
    /// 404 on a calendar in a read (6.10).
    CalendarUnknown,
    /// Free/busy with an error for any asked calendar, or a missing one (6.3, PD23).
    FreeBusyIncomplete,
    /// A read past its bounds, or with a cut attendee list (6.4, PD23).
    WindowTooLarge,
    /// A calendar list past 250 calendars (6.2, PD23).
    TooManyCalendars,
    /// An answer without an own entry (6.1, 6.7).
    NotInvited,
    /// A cut attendee list at an observation or at the read before a change (6.5, 6.7).
    AttendeesCut,
    /// An attendee list longer than the contract holds, at an observation (6.5).
    TooManyAttendees,
    /// An attendee address the contract cannot hold, at an observation (6.5, PD25).
    AttendeeAddress,
    /// An event id that fails 5.3.
    EventIdForm,
    /// A reserved id that fails 5.3.
    ReservedIdForm,
    /// A revision that fails 5.3.
    RevisionForm,
    /// An order whose action does not change a calendar, or names another account or calendar
    /// than its target.
    OrderMismatch,
    /// An order with no calendar, or more than 50, for a read (6.3).
    CalendarCount,
    /// An event that Google gave in a form the adapter cannot read: well-formed JSON that is
    /// not a usable event (6.4, 6.5, 6.7, section 7).
    EventUnreadable,
    /// A time in the order that the adapter cannot read (a window or an occurrence's original
    /// start); nothing was sent.
    TimeUnreadable,
    /// An invitation whose approved list names one mailbox twice (the same local part, the
    /// domain without regard to case, 6.12); nothing was sent.
    AddressTwice,
    /// An update that sets no member (6.7); nothing was sent.
    NothingToChange,
    /// Section 7: an event exists under the request's id and is not what was approved.
    ReconcileCreateDiffers,
    /// Section 7: the event was created and then deleted.
    ReconcileCreatedThenDeleted,
    /// Section 7: the event changed, and Fabric cannot tell whether its change is part of it.
    ReconcileChangedUnclear,
    /// Section 7: the event is gone, and Fabric cannot tell whether its change was made first.
    ReconcileGoneUnclear,
}

impl Sentence {
    /// Every sentence, for the tests.
    pub const ALL: [Sentence; 34] = [
        Sentence::InvalidRequest,
        Sentence::SignInEnded,
        Sentence::ClientRejected,
        Sentence::TokenStore,
        Sentence::QuotaExceeded,
        Sentence::OnlyOrganizer,
        Sentence::ChangeRefused,
        Sentence::ReadRefused,
        Sentence::UnexpectedAnswer,
        Sentence::IdTaken,
        Sentence::EventChanged,
        Sentence::EventGone,
        Sentence::EventAlreadyGone,
        Sentence::CalendarUnknown,
        Sentence::FreeBusyIncomplete,
        Sentence::WindowTooLarge,
        Sentence::TooManyCalendars,
        Sentence::NotInvited,
        Sentence::AttendeesCut,
        Sentence::TooManyAttendees,
        Sentence::AttendeeAddress,
        Sentence::EventIdForm,
        Sentence::ReservedIdForm,
        Sentence::RevisionForm,
        Sentence::OrderMismatch,
        Sentence::CalendarCount,
        Sentence::EventUnreadable,
        Sentence::TimeUnreadable,
        Sentence::AddressTwice,
        Sentence::NothingToChange,
        Sentence::ReconcileCreateDiffers,
        Sentence::ReconcileCreatedThenDeleted,
        Sentence::ReconcileChangedUnclear,
        Sentence::ReconcileGoneUnclear,
    ];

    /// The sentence.
    #[must_use]
    pub const fn text(self) -> &'static str {
        match self {
            Sentence::InvalidRequest => "Google refused the request as not valid.",
            Sentence::SignInEnded => "This account's sign-in has ended. Connect it again.",
            Sentence::ClientRejected => {
                "Google no longer accepts Fabric's sign-in client. Signing in again will not help until it is repaired."
            }
            Sentence::TokenStore => {
                "Fabric could not read this account's sign-in from the Keychain."
            }
            Sentence::QuotaExceeded => "Google's usage limit for this calendar is reached.",
            Sentence::OnlyOrganizer => "Only the organizer can change this.",
            Sentence::ChangeRefused => "Google refused the change for this account.",
            Sentence::ReadRefused => "Google refused the read for this account.",
            Sentence::UnexpectedAnswer => "Google gave an answer Fabric does not accept.",
            Sentence::IdTaken => "An event with this id already exists.",
            Sentence::EventChanged => "The event changed after it was approved.",
            Sentence::EventGone => "The event is gone.",
            Sentence::EventAlreadyGone => "The event is already gone.",
            Sentence::CalendarUnknown => "Google does not know this calendar for this account.",
            Sentence::FreeBusyIncomplete => {
                "Google could not report busy times for one of the calendars."
            }
            Sentence::WindowTooLarge => {
                "The window holds more events than one answer can carry. Ask for a shorter window."
            }
            Sentence::TooManyCalendars => {
                "The account has more than 250 calendars, more than Fabric can list."
            }
            Sentence::NotInvited => "This account is not invited to this event on this calendar.",
            Sentence::AttendeesCut => "Google sent only part of the attendee list.",
            Sentence::TooManyAttendees => "The event has more attendees than Fabric can hold.",
            Sentence::AttendeeAddress => {
                "An attendee address of the event cannot be held by Fabric."
            }
            Sentence::EventIdForm => "The event id is not of the form Google uses.",
            Sentence::ReservedIdForm => "The reserved event id is not of the form Google accepts.",
            Sentence::RevisionForm => "The revision is not one entity tag.",
            Sentence::OrderMismatch => "The order does not fit its calendar.",
            Sentence::CalendarCount => "A read names 1 to 50 calendars.",
            Sentence::EventUnreadable => "Google gave the event in a form Fabric cannot read.",
            Sentence::TimeUnreadable => "The time in the request cannot be read.",
            Sentence::AddressTwice => "The attendee list names one address twice.",
            Sentence::NothingToChange => "The update changes nothing.",
            Sentence::ReconcileCreateDiffers => {
                "An event exists under this request's id and is not what was approved."
            }
            Sentence::ReconcileCreatedThenDeleted => "The event was created and then deleted.",
            Sentence::ReconcileChangedUnclear => {
                "The event changed, and Fabric cannot tell whether its change is part of it."
            }
            Sentence::ReconcileGoneUnclear => {
                "The event is gone, and Fabric cannot tell whether its change was made first."
            }
        }
    }

    /// The sentence as the contract's `Explanation`.
    #[must_use]
    pub fn explanation(self) -> Explanation {
        // Every sentence is 1 to 1000 code points (unit test below), so this cannot fail.
        Explanation::new(self.text()).expect("a fixed sentence is a valid explanation")
    }
}

/// Google's error `reason`, read from `error.errors[0].reason` against the closed list of 6.10.
/// Every other reason, and an answer without one, is [`Reason::Other`] (PD27).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reason {
    /// `rateLimitExceeded` or `userRateLimitExceeded`.
    RateLimit,
    /// `quotaExceeded`.
    Quota,
    /// `forbiddenForNonOrganizer`.
    NonOrganizer,
    /// `insufficientPermissions`.
    InsufficientPermissions,
    /// Anything else.
    Other,
}

impl Reason {
    /// Reads the reason of an error answer. Only a JSON answer has one.
    pub(crate) fn of(answer: &Answer) -> Reason {
        answer
            .json()
            .map_or(Reason::Other, |value| Reason::from_json(&value))
    }

    /// Reads `error.errors[0].reason`.
    pub(crate) fn from_json(value: &Value) -> Reason {
        let reason = value
            .get("error")
            .and_then(|e| e.get("errors"))
            .and_then(Value::as_array)
            .and_then(|errors| errors.first())
            .and_then(|first| first.get("reason"))
            .and_then(Value::as_str);
        match reason {
            Some("rateLimitExceeded" | "userRateLimitExceeded") => Reason::RateLimit,
            Some("quotaExceeded") => Reason::Quota,
            Some("forbiddenForNonOrganizer") => Reason::NonOrganizer,
            Some("insufficientPermissions") => Reason::InsufficientPermissions,
            _ => Reason::Other,
        }
    }
}

/// A refusal as the classification reads it: the status, the reason and the wait hint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Refusal {
    /// The status code (not 2xx).
    pub(crate) status: u16,
    /// The reason of 6.10.
    pub(crate) reason: Reason,
    /// `Retry-After` in whole seconds, as Google sent it.
    pub(crate) retry_after_s: Option<u64>,
}

impl Refusal {
    /// Reads the parts of an answer that the classification needs.
    pub(crate) fn of(answer: &Answer) -> Refusal {
        Refusal {
            status: answer.status(),
            reason: Reason::of(answer),
            retry_after_s: answer.retry_after_seconds(),
        }
    }

    fn retry_after_ms(self) -> Option<u64> {
        self.retry_after_s
            .map(|s| s.min(RETRY_AFTER_CAP_S).saturating_mul(1000))
    }
}

/// Which changing request was refused; the rows for 404, 409, 410 and 412 differ (6.6 to 6.8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChangeKind {
    /// `events.insert` (6.6).
    Create,
    /// `events.patch` (6.7).
    Patch,
    /// `events.delete` (6.8).
    Delete,
}

/// Which read was refused; a 404 means another thing for a calendar than for an event (6.10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReadKind {
    /// The calendar list, free/busy and events lists: 404 names a calendar.
    Calendar,
    /// One event: 404 and 410 are `Absent`.
    Event,
}

/// The class of a refused read: the event is absent (404 or 410 on an event), or a failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReadRefusal {
    /// 404 or 410 on an event (6.5, section 7).
    Absent,
    /// Every other refusal.
    Failed(ReadFailure),
}

/// The rows of 6.10 shared by changes and reads, for the statuses whose class does not depend on
/// what was asked. `None` for 2xx, 404, 409, 410, 412 and 5xx, which the callers decide.
fn shared_row(refusal: Refusal, read: bool) -> Option<(ChangeOutcome, Option<SideResult>)> {
    let status = refusal.status;
    let rejected = |code, sentence| Some((ChangeOutcome::rejected(code, sentence), None));
    match (status, refusal.reason) {
        (200..=299, _) => None,
        (400, _) => rejected(FailureCode::InvalidArguments, Sentence::InvalidRequest),
        (401, _) => Some((ChangeOutcome::transient(), None)),
        // Every 429 is a refusal before acting, whatever its reason (6.10).
        (429, _) | (403, Reason::RateLimit) => Some((
            ChangeOutcome::Transient {
                retry_after_ms: refusal.retry_after_ms(),
            },
            None,
        )),
        (403, Reason::Quota) => rejected(FailureCode::ProviderRejected, Sentence::QuotaExceeded),
        (403, Reason::NonOrganizer) => {
            rejected(FailureCode::ProviderRejected, Sentence::OnlyOrganizer)
        }
        (403, Reason::InsufficientPermissions) => Some((
            ChangeOutcome::rejected(
                FailureCode::AccountAuthenticationRequired,
                Sentence::SignInEnded,
            ),
            Some(SideResult::AuthLost(AuthLostCause::GrantEnded)),
        )),
        (403, _) => Some((
            ChangeOutcome::rejected(
                FailureCode::ProviderRejected,
                if read {
                    Sentence::ReadRefused
                } else {
                    Sentence::ChangeRefused
                },
            ),
            Some(SideResult::AccessChanged),
        )),
        (404 | 409 | 410 | 412, _) | (500..=599, _) => None,
        _ => rejected(FailureCode::ProviderRejected, Sentence::UnexpectedAnswer),
    }
}

/// Classifies an answer that is not 2xx to a changing request, row by row as 6.10 with the
/// rows of 6.6 to 6.8 for 404, 409, 410 and 412. The caller drops the cached access token on
/// a 401 (3.8).
pub(crate) fn change_refusal(kind: ChangeKind, refusal: Refusal) -> ChangeReport {
    if let Some((outcome, side)) = shared_row(refusal, false) {
        return ChangeReport { outcome, side };
    }
    let outcome = match (kind, refusal.status) {
        // 5xx: Google may have made the change and failed afterwards (PD17). A 2xx is a yes
        // that was not read; the callers read it first, and if one comes here it is an
        // unknown outcome too (6.10).
        (_, 500..=599 | 200..=299) => ChangeOutcome::NoAnswer,
        // 6.6: an event with this id exists; no earlier attempt of this effect made it.
        (ChangeKind::Create, 409) => {
            ChangeOutcome::rejected(FailureCode::ProviderRejected, Sentence::IdTaken)
        }
        // 6.7: a conflict with another request in progress; nothing was applied. 6.8 does not
        // name 409; PD17 counts a conflict as a refusal before acting, so it is Transient too.
        (ChangeKind::Patch | ChangeKind::Delete, 409) => ChangeOutcome::transient(),
        (ChangeKind::Patch | ChangeKind::Delete, 412) => {
            ChangeOutcome::rejected(FailureCode::RevisionConflict, Sentence::EventChanged)
        }
        (ChangeKind::Patch, 404 | 410) => {
            ChangeOutcome::rejected(FailureCode::RevisionConflict, Sentence::EventGone)
        }
        (ChangeKind::Delete, 404 | 410) => {
            ChangeOutcome::rejected(FailureCode::RevisionConflict, Sentence::EventAlreadyGone)
        }
        // A create answered 404, 410 or 412: no row of 6.6 names them; "any other status".
        _ => ChangeOutcome::rejected(FailureCode::ProviderRejected, Sentence::UnexpectedAnswer),
    };
    ChangeReport::plain(outcome)
}

/// Classifies an answer that is not 2xx to a read (6.10, last paragraph): 5xx is Transient,
/// 404 on a calendar is Rejected `provider_rejected`, 404 and 410 on an event are `Absent`, the
/// other rows hold. The caller drops the cached access token on a 401 (3.8).
pub(crate) fn read_refusal(kind: ReadKind, refusal: Refusal) -> ReadRefusal {
    if let Some((outcome, side)) = shared_row(refusal, true) {
        return ReadRefusal::Failed(match outcome {
            ChangeOutcome::Transient { retry_after_ms } => {
                ReadFailure::Transient { retry_after_ms }
            }
            ChangeOutcome::Rejected { code, explanation } => ReadFailure::Rejected {
                code,
                explanation,
                side,
            },
            // `shared_row` gives only Transient and Rejected.
            ChangeOutcome::Applied { .. } | ChangeOutcome::NoAnswer => ReadFailure::transient(),
        });
    }
    match (kind, refusal.status) {
        // A 2xx is an answer that was not read; the callers read it first, and if one comes
        // here it is no usable answer (6.10).
        (_, 500..=599 | 200..=299) => ReadRefusal::Failed(ReadFailure::transient()),
        (ReadKind::Event, 404 | 410) => ReadRefusal::Absent,
        (ReadKind::Calendar, 404) => ReadRefusal::Failed(ReadFailure::rejected(
            FailureCode::ProviderRejected,
            Sentence::CalendarUnknown,
        )),
        _ => ReadRefusal::Failed(ReadFailure::rejected(
            FailureCode::ProviderRejected,
            Sentence::UnexpectedAnswer,
        )),
    }
}

/// The outcome of a change whose access token could not be had (3.8, section 4, 6.10).
/// Nothing was sent in every case.
pub(crate) fn change_access_failure(failure: AccessFailure) -> ChangeReport {
    match read_access_failure(failure) {
        ReadFailure::Rejected {
            code,
            explanation,
            side,
        } => ChangeReport {
            outcome: ChangeOutcome::Rejected { code, explanation },
            side,
        },
        ReadFailure::Transient { .. } | ReadFailure::Stopped => {
            ChangeReport::plain(ChangeOutcome::transient())
        }
    }
}

/// The failure of a read whose access token could not be had (3.8, section 4, 6.10).
pub(crate) fn read_access_failure(failure: AccessFailure) -> ReadFailure {
    let auth = FailureCode::AccountAuthenticationRequired;
    match failure {
        AccessFailure::Transient | AccessFailure::Store(TokenStoreError::Locked) => {
            ReadFailure::transient()
        }
        AccessFailure::AuthLost(cause) => ReadFailure::Rejected {
            code: auth,
            explanation: match cause {
                AuthLostCause::GrantEnded => Sentence::SignInEnded,
                AuthLostCause::ClientRejected => Sentence::ClientRejected,
            }
            .explanation(),
            side: Some(SideResult::AuthLost(cause)),
        },
        AccessFailure::Store(TokenStoreError::Refused | TokenStoreError::Failed { .. }) => {
            ReadFailure::rejected(auth, Sentence::TokenStore)
        }
    }
}

/// The retry numbers of CD-Q11e (6.11), the same as W04 D16.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Retries after the first attempt.
    pub max_retries: u32,
    /// The wait before each retry.
    pub backoff_ms: [u64; 3],
    /// The whole time a request may spend retrying, from its first attempt.
    pub budget_ms: u64,
    /// The longest one call may take (5.2).
    pub call_ms: u64,
}

impl RetryPolicy {
    /// Three retries after 2, 4 and 8 s or Google's longer hint, within 120 s; 30 s a call.
    pub const PLAN: RetryPolicy = RetryPolicy {
        max_retries: 3,
        backoff_ms: [2_000, 4_000, 8_000],
        budget_ms: 120_000,
        call_ms: crate::http::CALL_LIMIT_MS,
    };
}

/// What [`next_attempt`] decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryDecision {
    /// Wait `wait_ms`, then try again at `at_ms`.
    Retry {
        /// The wait from now.
        wait_ms: u64,
        /// When the next attempt is due (Unix ms).
        at_ms: u64,
    },
    /// The retries or the budget are spent.
    GiveUp,
}

/// The retry rule of 6.11: wait `max(backoff_ms[retries_made], hint_ms)` when fewer than
/// `max_retries` retries were made and the wait ends inside the budget counted from the first
/// attempt; otherwise give up. No random part (CD-Q11e).
#[must_use]
pub fn next_attempt(
    policy: &RetryPolicy,
    retries_made: u32,
    first_attempt_at_ms: u64,
    now_ms: u64,
    hint_ms: Option<u64>,
) -> RetryDecision {
    if retries_made >= policy.max_retries {
        return RetryDecision::GiveUp;
    }
    let Some(&base) = usize::try_from(retries_made)
        .ok()
        .and_then(|i| policy.backoff_ms.get(i))
    else {
        return RetryDecision::GiveUp;
    };
    let wait_ms = base.max(hint_ms.unwrap_or(0));
    let Some(at_ms) = now_ms.checked_add(wait_ms) else {
        return RetryDecision::GiveUp;
    };
    if at_ms <= first_attempt_at_ms.saturating_add(policy.budget_ms) {
        RetryDecision::Retry { wait_ms, at_ms }
    } else {
        RetryDecision::GiveUp
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn refusal(status: u16, reason: Reason, retry_after_s: Option<u64>) -> Refusal {
        Refusal {
            status,
            reason,
            retry_after_s,
        }
    }

    fn code_of(report: &ChangeReport) -> Option<FailureCode> {
        match &report.outcome {
            ChangeOutcome::Rejected { code, .. } => Some(*code),
            _ => None,
        }
    }

    #[test]
    fn every_sentence_is_a_valid_explanation_and_distinct() {
        let mut seen = std::collections::HashSet::new();
        for sentence in Sentence::ALL {
            assert_eq!(sentence.explanation().as_str(), sentence.text());
            assert!(seen.insert(sentence.text()), "{sentence:?}");
            assert!(sentence.text().ends_with('.'), "{sentence:?}");
        }
    }

    #[test]
    fn the_reason_is_read_from_the_first_error_against_a_closed_list() {
        let shape = |reason: &str| json!({"error": {"errors": [{"reason": reason}, {"reason": "quotaExceeded"}], "message": "x"}});
        assert_eq!(
            Reason::from_json(&shape("rateLimitExceeded")),
            Reason::RateLimit
        );
        assert_eq!(
            Reason::from_json(&shape("userRateLimitExceeded")),
            Reason::RateLimit
        );
        assert_eq!(Reason::from_json(&shape("quotaExceeded")), Reason::Quota);
        assert_eq!(
            Reason::from_json(&shape("forbiddenForNonOrganizer")),
            Reason::NonOrganizer
        );
        assert_eq!(
            Reason::from_json(&shape("insufficientPermissions")),
            Reason::InsufficientPermissions
        );
        assert_eq!(Reason::from_json(&shape("forbidden")), Reason::Other);
        assert_eq!(
            Reason::from_json(&shape("RateLimitExceeded")),
            Reason::Other
        );
        assert_eq!(
            Reason::from_json(&json!({"error": {"errors": []}})),
            Reason::Other
        );
        assert_eq!(
            Reason::from_json(&json!({"error": "rateLimitExceeded"})),
            Reason::Other
        );
        assert_eq!(
            Reason::from_json(&json!({"error": {"errors": [{"reason": 3}]}})),
            Reason::Other
        );
        assert_eq!(Reason::from_json(&json!([])), Reason::Other);
    }

    /// g37, the change half: one case per row of 6.10 and of 6.6 to 6.8.
    #[test]
    fn each_row_of_the_change_table_gives_its_class() {
        use ChangeKind::{Create, Delete, Patch};
        let r = |kind, status, reason| change_refusal(kind, refusal(status, reason, None));
        for kind in [Create, Patch, Delete] {
            assert_eq!(
                code_of(&r(kind, 400, Reason::Other)),
                Some(FailureCode::InvalidArguments)
            );
            assert_eq!(
                r(kind, 401, Reason::Other).outcome,
                ChangeOutcome::transient()
            );
            assert_eq!(
                r(kind, 403, Reason::RateLimit).outcome,
                ChangeOutcome::transient()
            );
            assert_eq!(
                r(kind, 429, Reason::RateLimit).outcome,
                ChangeOutcome::transient()
            );
            assert_eq!(
                code_of(&r(kind, 403, Reason::Quota)),
                Some(FailureCode::ProviderRejected)
            );
            assert_eq!(
                code_of(&r(kind, 403, Reason::NonOrganizer)),
                Some(FailureCode::ProviderRejected)
            );
            let lost = r(kind, 403, Reason::InsufficientPermissions);
            assert_eq!(
                code_of(&lost),
                Some(FailureCode::AccountAuthenticationRequired)
            );
            assert_eq!(
                lost.side,
                Some(SideResult::AuthLost(AuthLostCause::GrantEnded))
            );
            let other = r(kind, 403, Reason::Other);
            assert_eq!(code_of(&other), Some(FailureCode::ProviderRejected));
            assert_eq!(other.side, Some(SideResult::AccessChanged));
            for status in [500, 502, 503, 504, 599] {
                assert_eq!(
                    r(kind, status, Reason::Other).outcome,
                    ChangeOutcome::NoAnswer
                );
            }
            for status in [300, 301, 302, 304, 307, 418, 451, 100] {
                let report = r(kind, status, Reason::Other);
                assert_eq!(
                    code_of(&report),
                    Some(FailureCode::ProviderRejected),
                    "{status}"
                );
                assert_eq!(report.side, None);
            }
        }
        assert_eq!(
            code_of(&r(Create, 409, Reason::Other)),
            Some(FailureCode::ProviderRejected)
        );
        assert_eq!(
            r(Patch, 409, Reason::Other).outcome,
            ChangeOutcome::transient()
        );
        assert_eq!(
            r(Delete, 409, Reason::Other).outcome,
            ChangeOutcome::transient()
        );
        for status in [404, 410, 412] {
            assert_eq!(
                code_of(&r(Patch, status, Reason::Other)),
                Some(FailureCode::RevisionConflict)
            );
            assert_eq!(
                code_of(&r(Delete, status, Reason::Other)),
                Some(FailureCode::RevisionConflict)
            );
            assert_eq!(
                code_of(&r(Create, status, Reason::Other)),
                Some(FailureCode::ProviderRejected)
            );
        }
    }

    /// g37, the read half.
    #[test]
    fn each_row_of_the_read_table_gives_its_class() {
        use ReadKind::{Calendar, Event};
        let r = |kind, status, reason| read_refusal(kind, refusal(status, reason, None));
        let code = |refusal: ReadRefusal| match refusal {
            ReadRefusal::Failed(ReadFailure::Rejected { code, .. }) => Some(code),
            _ => None,
        };
        for kind in [Calendar, Event] {
            assert_eq!(
                code(r(kind, 400, Reason::Other)),
                Some(FailureCode::InvalidArguments)
            );
            assert_eq!(
                r(kind, 401, Reason::Other),
                ReadRefusal::Failed(ReadFailure::transient())
            );
            assert_eq!(
                r(kind, 429, Reason::RateLimit),
                ReadRefusal::Failed(ReadFailure::transient())
            );
            for status in [500, 503] {
                assert_eq!(
                    r(kind, status, Reason::Other),
                    ReadRefusal::Failed(ReadFailure::transient())
                );
            }
            let ReadRefusal::Failed(other) = r(kind, 403, Reason::Other) else {
                panic!("a failure");
            };
            assert_eq!(other.side(), Some(SideResult::AccessChanged));
            let ReadRefusal::Failed(lost) = r(kind, 403, Reason::InsufficientPermissions) else {
                panic!("a failure");
            };
            assert_eq!(
                lost.side(),
                Some(SideResult::AuthLost(AuthLostCause::GrantEnded))
            );
            assert_eq!(
                code(r(kind, 302, Reason::Other)),
                Some(FailureCode::ProviderRejected)
            );
            assert_eq!(
                code(r(kind, 409, Reason::Other)),
                Some(FailureCode::ProviderRejected)
            );
            assert_eq!(
                code(r(kind, 412, Reason::Other)),
                Some(FailureCode::ProviderRejected)
            );
        }
        assert_eq!(r(Event, 404, Reason::Other), ReadRefusal::Absent);
        assert_eq!(r(Event, 410, Reason::Other), ReadRefusal::Absent);
        assert_eq!(
            code(r(Calendar, 404, Reason::Other)),
            Some(FailureCode::ProviderRejected)
        );
        assert_eq!(
            code(r(Calendar, 410, Reason::Other)),
            Some(FailureCode::ProviderRejected)
        );
    }

    /// A 429 always means Google refused before acting: Transient whatever its reason, with
    /// `Retry-After` when present, for a change and for a read.
    #[test]
    fn every_429_is_transient_whatever_its_reason() {
        for reason in [
            Reason::Other,
            Reason::Quota,
            Reason::NonOrganizer,
            Reason::InsufficientPermissions,
            Reason::RateLimit,
        ] {
            for kind in [ChangeKind::Create, ChangeKind::Patch, ChangeKind::Delete] {
                let report = change_refusal(kind, refusal(429, reason, None));
                assert_eq!(report, ChangeReport::plain(ChangeOutcome::transient()));
                assert_eq!(
                    change_refusal(kind, refusal(429, reason, Some(9))).outcome,
                    ChangeOutcome::Transient {
                        retry_after_ms: Some(9_000)
                    }
                );
            }
            for kind in [ReadKind::Calendar, ReadKind::Event] {
                assert_eq!(
                    read_refusal(kind, refusal(429, reason, None)),
                    ReadRefusal::Failed(ReadFailure::transient())
                );
                assert_eq!(
                    read_refusal(kind, refusal(429, reason, Some(4))),
                    ReadRefusal::Failed(ReadFailure::Transient {
                        retry_after_ms: Some(4_000)
                    })
                );
            }
        }
    }

    /// A 2xx is never read as a refusal: for a change it is an unknown outcome, for a read no
    /// usable answer; never `Rejected`.
    #[test]
    fn a_2xx_is_never_classified_as_a_refusal() {
        for status in [200, 201, 204, 299] {
            assert_eq!(
                shared_row(refusal(status, Reason::Other, None), false),
                None
            );
            assert_eq!(shared_row(refusal(status, Reason::Quota, None), true), None);
            for kind in [ChangeKind::Create, ChangeKind::Patch, ChangeKind::Delete] {
                assert_eq!(
                    change_refusal(kind, refusal(status, Reason::Other, None)),
                    ChangeReport::plain(ChangeOutcome::NoAnswer)
                );
            }
            for kind in [ReadKind::Calendar, ReadKind::Event] {
                assert_eq!(
                    read_refusal(kind, refusal(status, Reason::Other, None)),
                    ReadRefusal::Failed(ReadFailure::transient())
                );
            }
        }
    }

    #[test]
    fn retry_after_is_passed_on_in_milliseconds_and_capped_at_120_seconds() {
        let wait =
            |s| change_refusal(ChangeKind::Patch, refusal(429, Reason::RateLimit, Some(s))).outcome;
        assert_eq!(
            wait(7),
            ChangeOutcome::Transient {
                retry_after_ms: Some(7_000)
            }
        );
        assert_eq!(
            wait(120),
            ChangeOutcome::Transient {
                retry_after_ms: Some(120_000)
            }
        );
        assert_eq!(
            wait(121),
            ChangeOutcome::Transient {
                retry_after_ms: Some(120_000)
            }
        );
        assert_eq!(
            wait(u64::MAX),
            ChangeOutcome::Transient {
                retry_after_ms: Some(120_000)
            }
        );
        assert_eq!(
            read_refusal(ReadKind::Event, refusal(403, Reason::RateLimit, Some(3))),
            ReadRefusal::Failed(ReadFailure::Transient {
                retry_after_ms: Some(3_000)
            })
        );
    }

    #[test]
    fn access_failures_map_as_section_4_and_3_8_say() {
        assert_eq!(
            change_access_failure(AccessFailure::Transient).outcome,
            ChangeOutcome::transient()
        );
        assert_eq!(
            change_access_failure(AccessFailure::Store(TokenStoreError::Locked)).outcome,
            ChangeOutcome::transient()
        );
        for error in [
            TokenStoreError::Refused,
            TokenStoreError::Failed { status: -1 },
        ] {
            let report = change_access_failure(AccessFailure::Store(error));
            assert_eq!(
                code_of(&report),
                Some(FailureCode::AccountAuthenticationRequired)
            );
            assert_eq!(report.side, None);
            let ChangeOutcome::Rejected { explanation, .. } = report.outcome else {
                panic!("rejected");
            };
            assert_eq!(explanation.as_str(), Sentence::TokenStore.text());
        }
        for cause in [AuthLostCause::GrantEnded, AuthLostCause::ClientRejected] {
            let report = change_access_failure(AccessFailure::AuthLost(cause));
            assert_eq!(
                code_of(&report),
                Some(FailureCode::AccountAuthenticationRequired)
            );
            assert_eq!(report.side, Some(SideResult::AuthLost(cause)));
            assert_eq!(
                read_access_failure(AccessFailure::AuthLost(cause)).side(),
                Some(SideResult::AuthLost(cause))
            );
        }
    }

    /// g44: the waits of 2, 4 and 8 s, a longer hint, the fourth failure and the budget; the
    /// numbers are W04 D16's (`crates/fabric-broker-core/src/broker.rs`, `RetryPolicy::PLAN`).
    #[test]
    fn the_retry_rule_waits_2_4_and_8_seconds_within_120_seconds() {
        let p = RetryPolicy::PLAN;
        assert_eq!(p.max_retries, 3);
        assert_eq!(p.backoff_ms, [2_000, 4_000, 8_000]);
        assert_eq!(p.budget_ms, 120_000);
        assert_eq!(p.call_ms, 30_000);
        let t0 = 1_000_000;
        assert_eq!(
            next_attempt(&p, 0, t0, t0 + 500, None),
            RetryDecision::Retry {
                wait_ms: 2_000,
                at_ms: t0 + 2_500
            }
        );
        assert_eq!(
            next_attempt(&p, 1, t0, t0 + 3_000, None),
            RetryDecision::Retry {
                wait_ms: 4_000,
                at_ms: t0 + 7_000
            }
        );
        assert_eq!(
            next_attempt(&p, 2, t0, t0 + 8_000, None),
            RetryDecision::Retry {
                wait_ms: 8_000,
                at_ms: t0 + 16_000
            }
        );
        // The fourth failure (three retries made) gives up.
        assert_eq!(
            next_attempt(&p, 3, t0, t0 + 17_000, None),
            RetryDecision::GiveUp
        );
        assert_eq!(
            next_attempt(&p, u32::MAX, t0, t0, None),
            RetryDecision::GiveUp
        );
        // A hint longer than the wait wins; a shorter one does not.
        assert_eq!(
            next_attempt(&p, 0, t0, t0, Some(30_000)),
            RetryDecision::Retry {
                wait_ms: 30_000,
                at_ms: t0 + 30_000
            }
        );
        assert_eq!(
            next_attempt(&p, 2, t0, t0, Some(1_000)),
            RetryDecision::Retry {
                wait_ms: 8_000,
                at_ms: t0 + 8_000
            }
        );
        // The budget: a wait that ends exactly at 120 s is allowed, one past it is not.
        assert_eq!(
            next_attempt(&p, 0, t0, t0 + 118_000, None),
            RetryDecision::Retry {
                wait_ms: 2_000,
                at_ms: t0 + 120_000
            }
        );
        assert_eq!(
            next_attempt(&p, 0, t0, t0 + 118_001, None),
            RetryDecision::GiveUp
        );
        assert_eq!(
            next_attempt(&p, 0, t0, t0, Some(120_001)),
            RetryDecision::GiveUp
        );
        assert_eq!(
            next_attempt(&p, 0, t0, u64::MAX, None),
            RetryDecision::GiveUp
        );
    }

    #[test]
    fn debug_of_an_applied_outcome_prints_no_event_id() {
        let outcome = ChangeOutcome::Applied {
            event: Some(EventRef {
                event_id: fabric_desktop_contracts::scalar::ProviderRef::new("secretevent")
                    .expect("valid"),
                revision: fabric_desktop_contracts::scalar::ProviderRef::new("\"1\"")
                    .expect("valid"),
            }),
            confirmed: true,
            differs: None,
            notification: Notification::Sent,
        };
        let text = format!("{outcome:?}");
        assert!(!text.contains("secretevent"), "{text}");
        assert!(text.contains("present"), "{text}");
    }

    #[test]
    fn a_differing_member_prints_only_its_fixed_snake_case_label() {
        let mut seen = std::collections::HashSet::new();
        for member in Differs::ALL {
            let label = member.label();
            assert!(seen.insert(label), "{label}");
            assert!(
                !label.is_empty() && label.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
                "{label}"
            );
            assert_eq!(format!("{member:?}"), label);
            assert_eq!(member.to_string(), label);
        }
        let outcome = ChangeOutcome::Applied {
            event: None,
            confirmed: false,
            differs: Some(Differs::Description),
            notification: Notification::NotRequested,
        };
        assert!(format!("{outcome:?}").contains("Some(description)"));
    }
}
