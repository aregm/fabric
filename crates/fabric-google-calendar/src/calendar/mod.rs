//! The Calendar operations (design section 6, and the read of section 7).
//!
//! [`Google`] is made from a shared [`Transport`], the [`AccessTokens`] through which every call
//! gets the token of its own account, and a clock. Its operations:
//!
//! - [`Google::list_calendars`] (6.2), every page, at most 250 calendars.
//! - [`Google::free_busy`] (6.3).
//! - [`Google::read_events`] (6.4): every page of every calendar in one order, asking
//!   `still_allowed` before each page (PD35).
//! - [`Google::observe`] (6.5).
//! - [`Google::change`] (6.6 to 6.8): at most one changing request per order (PD13).
//! - [`Google::reconcile_read`] (section 7).
//!
//! Rules every operation keeps: no request names `primary` (PD22); every calendar is named by
//! the id a calendar list returned, through a [`CalendarTarget`] (PO1); every call uses the
//! token of the account it was given and only it (6.1); a 401 drops the cached token and is
//! Transient (3.8); ids and revisions are checked before any request (5.3, PD32). Nothing an
//! operation returns carries Google's text other than event content in a read result or a
//! before-image.

mod change;
mod events;
mod freebusy;
mod list;

use std::fmt;
use std::sync::Arc;

use fabric_desktop_contracts::agent::{Action, EventView};
use fabric_desktop_contracts::common::{
    CalendarAccess, CalendarRef, EventField, FailureCode, Window,
};
use fabric_desktop_contracts::scalar::{Email, IanaZone, Id, Label, ProviderRef};
use serde_json::Value;

use crate::clock::SharedClock;
use crate::http::ids::CalendarId;
use crate::http::{Answer, Exchange, Request, Transport};
use crate::outcome::{
    ReadFailure, ReadKind, ReadRefusal, Refusal, Sentence, read_access_failure, read_refusal,
};
use crate::reconcile::Effect;
use crate::secret::Secret;
use crate::tokens::AccessTokens;

/// One calendar as an order names it (PO1): the broker's reference, Google's id from the
/// calendar list, the calendar's zone, and whether it is the account's primary calendar. The
/// core resolves it under the account the action names (section 8).
#[derive(Clone, PartialEq, Eq)]
pub struct CalendarTarget {
    /// The broker's account and calendar ids.
    pub calendar: CalendarRef,
    /// Google's calendar id, from a calendar list (5.3).
    pub provider_id: CalendarId,
    /// The calendar's zone; every events read names it (PD33).
    pub time_zone: IanaZone,
    /// Whether this is the account's primary calendar (6.1, PD31).
    pub primary: bool,
}

/// Prints whether the calendar is primary; never an id or an address (PD27).
impl fmt::Debug for CalendarTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CalendarTarget")
            .field("primary", &self.primary)
            .finish_non_exhaustive()
    }
}

/// One usable entry of an account's calendar list (6.2).
#[derive(Clone, PartialEq, Eq)]
pub struct CalendarListEntry {
    /// Google's calendar id.
    pub provider_id: CalendarId,
    /// `summaryOverride`, else `summary`, cut to 200 characters; "(no name)" when empty.
    pub name: Label,
    /// Whether it is the account's primary calendar.
    pub primary: bool,
    /// The calendar's zone.
    pub time_zone: IanaZone,
    /// The contract's access (6.2, PD26).
    pub access: CalendarAccess,
}

impl CalendarListEntry {
    /// Whether the account may change events there through Fabric: `owner` or `writer` (6.2,
    /// PD26).
    #[must_use]
    pub fn writable(&self) -> bool {
        matches!(self.access, CalendarAccess::Owner | CalendarAccess::Writer)
    }
}

/// Prints the access and the flags; never the id or the name.
impl fmt::Debug for CalendarListEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CalendarListEntry")
            .field("access", &self.access)
            .field("primary", &self.primary)
            .finish_non_exhaustive()
    }
}

/// What a read order asks for (6.4, PO3): the calendars, the window, the granted fields and the
/// events Fabric created in those calendars, as pairs of the broker's calendar id and Google's
/// event id (PD19).
#[derive(Clone)]
pub struct EventsQuery {
    /// The calendars, in the order the result lists them.
    pub targets: Vec<CalendarTarget>,
    /// The half-open window.
    pub window: Window,
    /// The granted fields.
    pub fields: Vec<EventField>,
    /// The events Fabric created: (broker calendar id, event id).
    pub fabric_created: std::collections::BTreeSet<(Id, ProviderRef)>,
}

/// Prints counts only.
impl fmt::Debug for EventsQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventsQuery")
            .field("targets", &self.targets.len())
            .field("fields", &self.fields)
            .field("fabric_created", &self.fabric_created.len())
            .finish_non_exhaustive()
    }
}

/// The observation of one event (6.5). It mirrors `ObservedEvent` of the broker core's ports,
/// which this crate does not depend on (PD1).
#[derive(Clone, PartialEq, Eq)]
pub struct ObservedEvent {
    /// The event's calendar (the broker's reference).
    pub calendar: CalendarRef,
    /// Google's `id`, verbatim.
    pub event_id: ProviderRef,
    /// Google's `etag`, verbatim.
    pub revision: ProviderRef,
    /// Whether Fabric created it (the store's record, PD19), as the order said.
    pub fabric_created: bool,
    /// `recurrence` or `recurringEventId` present.
    pub recurring: bool,
    /// `eventType` absent or `default`.
    pub default_type: bool,
    /// `start.dateTime` present.
    pub timed: bool,
    /// An own entry exists (6.1) and `organizer.self` is not true.
    pub invitation: bool,
    /// Every attendee address except the own entry's, as written, the domain in lower case.
    pub attendees: Vec<Email>,
    /// `originalStartTime.dateTime` of an occurrence, as UTC milliseconds; absent for an
    /// occurrence of an all-day series.
    pub occurrence_start_ms: Option<i64>,
}

/// Prints the flags and counts; never an id, an address or a revision.
impl fmt::Debug for ObservedEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ObservedEvent")
            .field("fabric_created", &self.fabric_created)
            .field("recurring", &self.recurring)
            .field("default_type", &self.default_type)
            .field("timed", &self.timed)
            .field("invitation", &self.invitation)
            .field("attendees", &self.attendees.len())
            .field("occurrence", &self.occurrence_start_ms.is_some())
            .finish_non_exhaustive()
    }
}

/// The answer to an observe order (6.5, PO2).
#[derive(Clone, PartialEq, Eq)]
pub enum Observed {
    /// The event, and a view with every content field for the owner's card only.
    Found {
        /// The observation.
        observation: ObservedEvent,
        /// The before-image (`private.before`), boxed: it is large beside `Absent`.
        before: Box<EventView>,
    },
    /// 404, 410, or `status: cancelled`.
    Absent,
}

/// Prints the case and the observation's flags; never content.
impl fmt::Debug for Observed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Observed::Found { observation, .. } => f
                .debug_struct("Found")
                .field("observation", observation)
                .finish_non_exhaustive(),
            Observed::Absent => f.write_str("Absent"),
        }
    }
}

/// One changing order (6.6 to 6.8, PD13): the approved effect and the attempt number. A retry
/// is a new order from the core with the next attempt number (PLAN 5).
#[derive(Clone, PartialEq, Eq)]
pub struct ChangeOrder {
    /// The approved action and, for a create, the reserved id.
    pub effect: Effect,
    /// 1 for the first attempt. The adapter does not act on it; it is carried for the record.
    pub attempt: u32,
}

impl ChangeOrder {
    /// An order for `action`, with the reserved id of a create.
    #[must_use]
    pub fn new(action: Action, reserved_id: Option<String>, attempt: u32) -> ChangeOrder {
        ChangeOrder {
            effect: Effect::new(action, reserved_id),
            attempt,
        }
    }
}

/// Prints the effect's tool and the attempt.
impl fmt::Debug for ChangeOrder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChangeOrder")
            .field("effect", &self.effect)
            .field("attempt", &self.attempt)
            .finish()
    }
}

/// The Google Calendar adapter's Calendar half (section 6).
pub struct Google {
    transport: Arc<Transport>,
    tokens: Box<dyn AccessTokens>,
    clock: SharedClock,
}

/// Prints nothing of the tokens or the endpoints.
impl fmt::Debug for Google {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Google").finish_non_exhaustive()
    }
}

/// A 2xx answer of a read, as JSON; or the refusal of an event read that means "absent".
pub(crate) enum ReadAnswer {
    /// The JSON body of a 2xx answer.
    Body(Value),
    /// 404 or 410 on an event.
    Absent,
}

impl Google {
    /// The adapter over `transport`, getting each account's access token from `tokens`, with
    /// `clock` for `observed_at` and the deadline rule of 6.10.
    #[must_use]
    pub fn new(
        transport: Arc<Transport>,
        tokens: Box<dyn AccessTokens>,
        clock: SharedClock,
    ) -> Google {
        Google {
            transport,
            tokens,
            clock,
        }
    }

    fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }

    /// The account's access token for a read.
    pub(crate) fn read_token(
        &mut self,
        account: &Id,
        deadline_ms: u64,
    ) -> Result<Secret, ReadFailure> {
        self.tokens
            .access_token(account, deadline_ms)
            .map_err(read_access_failure)
    }

    /// Sends one read with the account's token and classifies its answer as 6.10 for a read.
    /// A 2xx whose body is not readable JSON is no usable answer: Transient.
    pub(crate) fn send_read(
        &mut self,
        account: &Id,
        token: &Secret,
        request: Request,
        deadline_ms: u64,
        kind: ReadKind,
    ) -> Result<ReadAnswer, ReadFailure> {
        let Ok(request) = request.bearer(token) else {
            return Err(ReadFailure::transient());
        };
        let answer = match self.transport.send(request, deadline_ms) {
            Exchange::NothingSent(_) | Exchange::SentUnanswered(_) => {
                return Err(ReadFailure::transient());
            }
            Exchange::Answer(answer) => answer,
        };
        self.read_answer(account, &answer, kind)
    }

    fn read_answer(
        &mut self,
        account: &Id,
        answer: &Answer,
        kind: ReadKind,
    ) -> Result<ReadAnswer, ReadFailure> {
        if answer.is_success() {
            return answer
                .json()
                .map(ReadAnswer::Body)
                .map_err(|_| ReadFailure::transient());
        }
        if answer.status() == 401 {
            self.tokens.drop_cached(account);
        }
        match read_refusal(kind, Refusal::of(answer)) {
            ReadRefusal::Absent => Ok(ReadAnswer::Absent),
            ReadRefusal::Failed(failure) => Err(failure),
        }
    }
}

/// Checks that every target belongs to `account`; otherwise the order is refused before any
/// request.
pub(crate) fn check_targets(account: &Id, targets: &[&CalendarTarget]) -> Result<(), ReadFailure> {
    if targets.iter().all(|t| &t.calendar.account_id == account) {
        Ok(())
    } else {
        Err(ReadFailure::rejected(
            FailureCode::InvalidArguments,
            Sentence::OrderMismatch,
        ))
    }
}
