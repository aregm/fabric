//! Contract types to and from Google's resources (design 6.2, 6.4, 6.5, 6.6, 6.7, 6.9, 6.12).
//!
//! Google's JSON is read leniently (5.2): unknown members are ignored, a missing optional member
//! and `null` are absent, a member of the wrong type is a [`ShapeError`] with a fixed label. No
//! text of Google's reaches an error; event content reaches only the values a read or a
//! before-image returns.
//!
//! What this module decides:
//!
//! - The calendar-list entry (6.2): the roles, the name cut to 200 characters or "(no name)",
//!   entries with an unusable id, the id `primary`, an unknown role or an unusable zone left
//!   out.
//! - Event times (6.9, PD33): a `dateTime` is paired with the zone of the calendar the read
//!   named; `Z` becomes `+00:00`, a fraction of a second is dropped; all-day dates stay dates.
//! - The [`EventView`] of a read, built from the granted fields only (PD24), with texts cut at
//!   the contract's bounds on a character boundary (PD25) and attendee addresses with the
//!   domain in lower case; one that the contract cannot hold is left out of a read.
//! - "The account's own entry" (6.1, PD31): the attendee entry marked `self`, only on the
//!   account's primary calendar and only when its address is that calendar's id.
//! - The observation of 6.5, which fails on a cut attendee list or an address the contract
//!   cannot hold (PD25).
//! - The bodies of 6.6 and 6.7, and `matches` and `confirmed` of 6.12, with the first member
//!   that differs ([`Differs`]) and the closed list of [`NORMALISATIONS`], empty today (18.7
//!   point 2).
//! - The span of each event of a read ([`crate::calendar::Span`]; 18.7 point 4).

use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt;

use fabric_desktop_contracts::agent::{
    Action, Attendee, AttendeeList, EventChanges, EventSpan, EventView, NewEvent, Occurrence,
    ProposedResponse, ResponseStatus,
};
use fabric_desktop_contracts::canonical;
use fabric_desktop_contracts::common::{CalendarAccess, EventField, EventTime};
use fabric_desktop_contracts::scalar::{
    Date, Email, EventDescription, EventLocation, IanaZone, Label, OffsetDateTime, ProviderRef,
    ReadTitle,
};
use serde_json::{Map, Value, json};

use crate::calendar::{CalendarTarget, ObservedEvent, Span};
use crate::http::ids::{CalendarId, EventId};
use crate::outcome::Differs;
use crate::time::{parse_rfc3339_ms, read_date_time, to_offset_date_time};

/// The longest calendar name (6.2; the contract's `label`).
pub const MAX_NAME_CHARS: usize = 200;

/// The name of a calendar that has none (6.2).
pub const NO_NAME: &str = "(no name)";

/// The most attendees one event may have in a read and an observation (PD23; the contract's
/// attendee list).
pub const MAX_ATTENDEES: usize = 2_000;

/// The longest title, place and description a view holds (the contract's bounds; PD25).
const MAX_TITLE_CHARS: usize = 1_024;
const MAX_LOCATION_CHARS: usize = 1_024;
const MAX_DESCRIPTION_CHARS: usize = 8_192;

/// Google's JSON was not of the expected shape. The label is fixed and names the member, never
/// a value (PD27).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ShapeError(&'static str);

impl ShapeError {
    /// The fixed label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        self.0
    }
}

impl fmt::Debug for ShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ShapeError({})", self.0)
    }
}

impl fmt::Display for ShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for ShapeError {}

type Object = Map<String, Value>;

/// The value of a member; `null` reads as absent.
fn get<'a>(o: &'a Object, name: &str) -> Option<&'a Value> {
    o.get(name).filter(|v| !v.is_null())
}

fn as_object<'a>(value: &'a Value, label: &'static str) -> Result<&'a Object, ShapeError> {
    value.as_object().ok_or(ShapeError(label))
}

fn text<'a>(o: &'a Object, name: &str, label: &'static str) -> Result<Option<&'a str>, ShapeError> {
    get(o, name)
        .map(|v| v.as_str().ok_or(ShapeError(label)))
        .transpose()
}

fn flag(o: &Object, name: &str, label: &'static str) -> Result<bool, ShapeError> {
    get(o, name)
        .map(|v| v.as_bool().ok_or(ShapeError(label)))
        .transpose()
        .map(|b| b.unwrap_or(false))
}

fn child<'a>(
    o: &'a Object,
    name: &str,
    label: &'static str,
) -> Result<Option<&'a Object>, ShapeError> {
    get(o, name).map(|v| as_object(v, label)).transpose()
}

fn list<'a>(
    o: &'a Object,
    name: &str,
    label: &'static str,
) -> Result<Option<&'a [Value]>, ShapeError> {
    get(o, name)
        .map(|v| v.as_array().map(Vec::as_slice).ok_or(ShapeError(label)))
        .transpose()
}

/// Cuts `text` to at most `max` characters, on a character boundary (PD25).
#[must_use]
pub fn cut(text: &str, max: usize) -> &str {
    match text.char_indices().nth(max) {
        Some((at, _)) => &text[..at],
        None => text,
    }
}

// ---- Addresses -------------------------------------------------------------------------------

/// Two addresses are the same when the local parts are equal and the domains are equal without
/// regard to case (6.12). The first `@` separates them, as in the contract's `email`.
#[must_use]
pub fn same_address(a: &str, b: &str) -> bool {
    match (a.split_once('@'), b.split_once('@')) {
        (Some((la, da)), Some((lb, db))) => la == lb && da.eq_ignore_ascii_case(db),
        _ => false,
    }
}

/// An address as the contract holds it: as written, the domain in lower case (6.5). `None` when
/// the contract's `email` cannot hold it (PD25).
#[must_use]
pub fn contract_address(address: &str) -> Option<Email> {
    let (local, domain) = address.split_once('@')?;
    Email::new(format!("{local}@{}", domain.to_ascii_lowercase())).ok()
}

/// The key two addresses share when [`same_address`] holds.
fn address_key(address: &str) -> Option<String> {
    let (local, domain) = address.split_once('@')?;
    Some(format!("{local}@{}", domain.to_ascii_lowercase()))
}

// ---- The calendar list (6.2) -----------------------------------------------------------------

/// The contract's access for Google's `accessRole` (6.2, PD26). `None`: the entry is left out.
#[must_use]
pub fn access_of(role: &str) -> Option<CalendarAccess> {
    match role {
        "owner" => Some(CalendarAccess::Owner),
        "writer" => Some(CalendarAccess::Writer),
        // PD26: may write and sees private events without details; shown as `reader` and not
        // writable until the contract names it (V3-1).
        "writerWithoutPrivateAccess" | "reader" => Some(CalendarAccess::Reader),
        "freeBusyReader" => Some(CalendarAccess::FreeBusyReader),
        _ => None,
    }
}

/// One usable entry of a calendar list.
pub(crate) struct ListItem {
    pub(crate) id: CalendarId,
    pub(crate) name: Label,
    pub(crate) primary: bool,
    pub(crate) time_zone: IanaZone,
    pub(crate) access: CalendarAccess,
}

/// Reads one entry of `calendarList.list` (6.2). `Ok(None)` when it is left out: an unusable
/// id (5.3), the id `primary` (Google's keyword, which no request may name, PD22), an unknown
/// role, a missing zone or one the contract cannot hold, or a deleted entry.
pub(crate) fn calendar_list_item(value: &Value) -> Result<Option<ListItem>, ShapeError> {
    const L: &str = "a calendar list entry is not of the expected shape";
    let o = as_object(value, L)?;
    let id = text(o, "id", L)?;
    let role = text(o, "accessRole", L)?;
    let zone = text(o, "timeZone", L)?;
    let override_name = text(o, "summaryOverride", L)?;
    let summary = text(o, "summary", L)?;
    let primary = flag(o, "primary", L)?;
    let deleted = flag(o, "deleted", L)?;
    let _hidden = flag(o, "hidden", L)?;
    if deleted {
        return Ok(None);
    }
    // PD22: an entry whose id is the keyword would make a later request name `primary`.
    let Some(id) = id
        .filter(|id| *id != "primary")
        .and_then(|id| CalendarId::new(id).ok())
    else {
        return Ok(None);
    };
    let Some(access) = role.and_then(access_of) else {
        return Ok(None);
    };
    let Some(time_zone) = zone.and_then(|z| IanaZone::new(z).ok()) else {
        return Ok(None);
    };
    let raw = override_name
        .filter(|n| !n.is_empty())
        .or(summary)
        .unwrap_or("");
    let name = cut(raw, MAX_NAME_CHARS);
    let name =
        Label::new(if name.is_empty() { NO_NAME } else { name }).map_err(|_| ShapeError(L))?;
    Ok(Some(ListItem {
        id,
        name,
        primary,
        time_zone,
        access,
    }))
}

// ---- Events ----------------------------------------------------------------------------------

/// `start`, `end` or `originalStartTime` of an event.
#[derive(Clone, Default)]
pub(crate) struct GoogleTime {
    pub(crate) date_time: Option<String>,
    pub(crate) date: Option<String>,
    pub(crate) time_zone: Option<String>,
}

impl GoogleTime {
    fn read(o: &Object) -> Result<GoogleTime, ShapeError> {
        const L: &str = "an event time is not of the expected shape";
        Ok(GoogleTime {
            date_time: text(o, "dateTime", L)?.map(str::to_owned),
            date: text(o, "date", L)?.map(str::to_owned),
            time_zone: text(o, "timeZone", L)?.map(str::to_owned),
        })
    }

    /// The instant of a `dateTime`, when there is one and it parses.
    pub(crate) fn instant_ms(&self) -> Option<i64> {
        self.date_time
            .as_deref()
            .and_then(|t| parse_rfc3339_ms(t).ok())
    }
}

/// One attendee entry, with the entry as Google wrote it (kept whole in 6.7).
#[derive(Clone)]
pub(crate) struct GoogleAttendee {
    pub(crate) email: Option<String>,
    pub(crate) response: Option<String>,
    pub(crate) is_self: bool,
    pub(crate) raw: Value,
}

/// An event as Google gave it, read leniently. Holds event content: no `Debug`.
#[derive(Clone)]
pub(crate) struct GoogleEvent {
    pub(crate) id: String,
    pub(crate) etag: String,
    pub(crate) cancelled: bool,
    pub(crate) recurring_event_id: bool,
    pub(crate) recurrence: bool,
    pub(crate) event_type: Option<String>,
    pub(crate) start: Option<GoogleTime>,
    pub(crate) end: Option<GoogleTime>,
    pub(crate) original_start: Option<GoogleTime>,
    pub(crate) summary: Option<String>,
    pub(crate) location: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) attendees: Vec<GoogleAttendee>,
    pub(crate) attendees_omitted: bool,
    pub(crate) organizer_email: Option<String>,
    pub(crate) organizer_self: bool,
}

impl GoogleEvent {
    /// Reads an event. `id` is required; `etag` is required unless the event is cancelled
    /// (a cancelled item of a list may carry little more than its id).
    pub(crate) fn read(value: &Value) -> Result<GoogleEvent, ShapeError> {
        const L: &str = "an event is not of the expected shape";
        const LA: &str = "an attendee entry is not of the expected shape";
        let o = as_object(value, L)?;
        let cancelled = text(o, "status", L)? == Some("cancelled");
        let id = text(o, "id", L)?.ok_or(ShapeError("an event has no id"))?;
        let etag = match text(o, "etag", L)? {
            Some(etag) => etag.to_owned(),
            None if cancelled => String::new(),
            None => return Err(ShapeError("an event has no etag")),
        };
        let time = |name| -> Result<Option<GoogleTime>, ShapeError> {
            child(o, name, L)?.map(GoogleTime::read).transpose()
        };
        let recurrence = list(o, "recurrence", L)?.is_some_and(|r| !r.is_empty());
        let mut attendees = Vec::new();
        for entry in list(o, "attendees", L)?.unwrap_or(&[]) {
            let a = as_object(entry, LA)?;
            attendees.push(GoogleAttendee {
                email: text(a, "email", LA)?.map(str::to_owned),
                response: text(a, "responseStatus", LA)?.map(str::to_owned),
                is_self: flag(a, "self", LA)?,
                raw: entry.clone(),
            });
        }
        let organizer = child(o, "organizer", L)?;
        Ok(GoogleEvent {
            id: id.to_owned(),
            etag,
            cancelled,
            recurring_event_id: text(o, "recurringEventId", L)?.is_some(),
            recurrence,
            event_type: text(o, "eventType", L)?.map(str::to_owned),
            start: time("start")?,
            end: time("end")?,
            original_start: time("originalStartTime")?,
            summary: text(o, "summary", L)?.map(str::to_owned),
            location: text(o, "location", L)?.map(str::to_owned),
            description: text(o, "description", L)?.map(str::to_owned),
            attendees,
            attendees_omitted: flag(o, "attendeesOmitted", L)?,
            organizer_email: organizer
                .map(|org| text(org, "email", L))
                .transpose()?
                .flatten()
                .map(str::to_owned),
            organizer_self: organizer
                .map(|org| flag(org, "self", L))
                .transpose()?
                .unwrap_or(false),
        })
    }

    /// The instant of `originalStartTime.dateTime`, for an occurrence of a timed series.
    pub(crate) fn original_start_ms(&self) -> Option<i64> {
        self.original_start
            .as_ref()
            .and_then(GoogleTime::instant_ms)
    }
}

/// The index of the account's own entry (6.1, PD31): only on the account's primary calendar,
/// only an entry marked `self` whose address is that calendar's id.
pub(crate) fn own_entry(event: &GoogleEvent, target: &CalendarTarget) -> Option<usize> {
    if !target.primary {
        return None;
    }
    event.attendees.iter().position(|a| {
        a.is_self
            && a.email
                .as_deref()
                .is_some_and(|e| same_address(e, target.provider_id.as_str()))
    })
}

fn response_status(text: Option<&str>) -> Result<ResponseStatus, ShapeError> {
    match text {
        None | Some("needsAction") => Ok(ResponseStatus::NeedsAction),
        Some("accepted") => Ok(ResponseStatus::Accepted),
        Some("declined") => Ok(ResponseStatus::Declined),
        Some("tentative") => Ok(ResponseStatus::Tentative),
        Some(_) => Err(ShapeError(
            "an attendee's response is not one Google documents",
        )),
    }
}

/// A `dateTime` paired with the calendar's zone (6.9, PD33).
fn event_time(date_time: &str, zone: &IanaZone) -> Result<EventTime, ShapeError> {
    Ok(EventTime {
        date_time: to_offset_date_time(date_time)
            .map_err(|_| ShapeError("an event time is not an RFC 3339 date-time"))?,
        zone: zone.clone(),
    })
}

/// One end of a timed event, read once: the contract's form and the instant.
struct TimedEnd {
    date_time: OffsetDateTime,
    instant_ms: i64,
}

/// An event's own `start` and `end`, read once (6.4; 18.7 point 4). The one owner of their
/// shape rules and of their parsing: the view's time ([`span`]) and the [`Span`]
/// ([`event_span`]) are both derived from it, so one is never present without the other.
enum StartEnd {
    Timed { start: TimedEnd, end: TimedEnd },
    AllDay { start_date: Date, end_date: Date },
}

/// Reads an event's own `start` and `end`, never `originalStartTime`: two `dateTime`s are
/// timed, two `date`s are all day. Anything else (missing, one a date and the other a time, a
/// value that does not parse) is a [`ShapeError`].
fn start_end(event: &GoogleEvent) -> Result<StartEnd, ShapeError> {
    const L: &str = "an event's start and end are not both times or both dates";
    let (Some(start), Some(end)) = (&event.start, &event.end) else {
        return Err(ShapeError("an event has no start or no end"));
    };
    match (&start.date_time, &end.date_time, &start.date, &end.date) {
        (Some(s), Some(e), _, _) => {
            let timed = |t: &str| {
                read_date_time(t)
                    .map(|(date_time, instant_ms)| TimedEnd {
                        date_time,
                        instant_ms,
                    })
                    .map_err(|_| ShapeError("an event time is not an RFC 3339 date-time"))
            };
            Ok(StartEnd::Timed {
                start: timed(s)?,
                end: timed(e)?,
            })
        }
        (None, None, Some(s), Some(e)) => Ok(StartEnd::AllDay {
            start_date: Date::new(s.as_str()).map_err(|_| ShapeError(L))?,
            end_date: Date::new(e.as_str()).map_err(|_| ShapeError(L))?,
        }),
        _ => Err(ShapeError(L)),
    }
}

/// The view's time (6.4): [`start_end`] with the calendar's zone.
fn span(event: &GoogleEvent, zone: &IanaZone) -> Result<EventSpan, ShapeError> {
    Ok(match start_end(event)? {
        StartEnd::Timed { start, end } => EventSpan::Timed {
            start: EventTime {
                date_time: start.date_time,
                zone: zone.clone(),
            },
            end: EventTime {
                date_time: end.date_time,
                zone: zone.clone(),
            },
        },
        StartEnd::AllDay {
            start_date,
            end_date,
        } => EventSpan::AllDay {
            start_date,
            end_date,
        },
    })
}

/// The [`Span`] of an event of a read (6.4; 18.7 point 4): from the occurrence's own `start`
/// and `end`, never from `originalStartTime`. Two `dateTime`s give `Timed` in UTC
/// milliseconds; two `date`s give `AllDay` with Google's dates exactly, the end exclusive.
/// No widening and no zone arithmetic. Anything else (missing, or one a date and the other a
/// time) is a [`ShapeError`]: the read fails, it never lacks a span. Read by [`start_end`], as
/// the view's time is.
pub(crate) fn event_span(event: &GoogleEvent) -> Result<Span, ShapeError> {
    Ok(match start_end(event)? {
        StartEnd::Timed { start, end } => Span::Timed {
            start_ms: start.instant_ms,
            end_ms: end.instant_ms,
        },
        StartEnd::AllDay {
            start_date,
            end_date,
        } => Span::AllDay {
            start_date,
            end_date,
        },
    })
}

/// The occurrence identity of an occurrence of a timed series (6.4). `None` for a single event
/// and for an occurrence of an all-day series, whose original start is a date (V3-6).
fn occurrence(event: &GoogleEvent, zone: &IanaZone) -> Result<Option<Occurrence>, ShapeError> {
    if !event.recurring_event_id {
        return Ok(None);
    }
    let Some(original) = event
        .original_start
        .as_ref()
        .and_then(|o| o.date_time.as_deref())
    else {
        return Ok(None);
    };
    Ok(Some(Occurrence {
        instance_id: provider_ref(&event.id)?,
        original_start_time: event_time(original, zone)?,
    }))
}

fn provider_ref(value: &str) -> Result<ProviderRef, ShapeError> {
    ProviderRef::new(value).map_err(|_| ShapeError("an id or etag is not a provider reference"))
}

/// The fields a read discloses (6.4). Built from the grant; a field that is not granted is
/// `null` in the view even if Google sent it (PD24).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Granted {
    /// `time`.
    pub time: bool,
    /// `title`.
    pub title: bool,
    /// `location`.
    pub location: bool,
    /// `description`.
    pub description: bool,
    /// `attendees`.
    pub attendees: bool,
    /// `organizer`.
    pub organizer: bool,
    /// `own_response`.
    pub own_response: bool,
}

impl Granted {
    /// Every field: the before-image of an observation (6.5).
    pub const ALL: Granted = Granted {
        time: true,
        title: true,
        location: true,
        description: true,
        attendees: true,
        organizer: true,
        own_response: true,
    };

    /// The fields of a grant.
    #[must_use]
    pub fn of(fields: &[EventField]) -> Granted {
        let mut g = Granted::default();
        for field in fields {
            match field {
                EventField::Time => g.time = true,
                EventField::Title => g.title = true,
                EventField::Location => g.location = true,
                EventField::Description => g.description = true,
                EventField::Attendees => g.attendees = true,
                EventField::Organizer => g.organizer = true,
                EventField::OwnResponse => g.own_response = true,
            }
        }
        g
    }

    /// The `fields=` value of an events list (6.4): `nextPageToken` and, per item, `id`,
    /// `etag`, `status`, `recurringEventId`, the start and end, and the members of each
    /// granted field. The start and end are asked whatever the grant, as
    /// `start(date,dateTime),end(date,dateTime)`, for the span only (18.7 point 4); with
    /// `time` granted they are asked whole, with `originalStartTime`, for the view.
    #[must_use]
    pub fn fields_param(self) -> String {
        let mut members = vec!["id", "etag", "status", "recurringEventId"];
        if self.time {
            members.extend(["start", "end", "originalStartTime"]);
        } else {
            members.extend(["start(date,dateTime)", "end(date,dateTime)"]);
        }
        if self.title {
            members.push("summary");
        }
        if self.location {
            members.push("location");
        }
        if self.description {
            members.push("description");
        }
        if self.attendees || self.own_response {
            members.extend(["attendees(email,responseStatus,self)", "attendeesOmitted"]);
        }
        if self.organizer {
            members.push("organizer(email)");
        }
        format!("nextPageToken,items({})", members.join(","))
    }
}

/// Why an event could not become a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ViewError {
    /// Google's JSON is not of the expected shape.
    Shape(ShapeError),
    /// The attendee list was asked for and is cut or longer than 2 000 (PD23).
    Bound,
}

impl From<ShapeError> for ViewError {
    fn from(error: ShapeError) -> ViewError {
        ViewError::Shape(error)
    }
}

/// The [`EventView`] of an event (6.4): built from the granted fields only (PD24), in the
/// calendar's zone (PD33), texts cut at the contract's bounds (PD25), attendee addresses that
/// the contract cannot hold left out, `own_response` only from the own entry (6.1).
pub(crate) fn event_view(
    event: &GoogleEvent,
    granted: Granted,
    target: &CalendarTarget,
    fabric_created: bool,
) -> Result<EventView, ViewError> {
    let zone = &target.time_zone;
    let content =
        |text: &Option<String>, max: usize| cut(text.as_deref().unwrap_or(""), max).to_owned();
    let attendees = if granted.attendees {
        if event.attendees_omitted || event.attendees.len() > MAX_ATTENDEES {
            return Err(ViewError::Bound);
        }
        let mut list = Vec::new();
        for a in &event.attendees {
            let response = response_status(a.response.as_deref())?;
            if let Some(email) = a.email.as_deref().and_then(contract_address) {
                list.push(Attendee { email, response });
            }
        }
        Some(AttendeeList::new(list).map_err(|_| ViewError::Bound)?)
    } else {
        None
    };
    let own_response = if granted.own_response {
        own_entry(event, target)
            .map(|i| response_status(event.attendees[i].response.as_deref()))
            .transpose()?
    } else {
        None
    };
    let bad_text = |_| ShapeError("an event text does not fit the contract");
    Ok(EventView {
        event_id: provider_ref(&event.id)?,
        revision: provider_ref(&event.etag)?,
        calendar_id: target.calendar.calendar_id.clone(),
        fabric_created,
        occurrence: if granted.time {
            occurrence(event, zone)?
        } else {
            None
        },
        time: if granted.time {
            Some(span(event, zone)?)
        } else {
            None
        },
        title: if granted.title {
            Some(ReadTitle::new(content(&event.summary, MAX_TITLE_CHARS)).map_err(bad_text)?)
        } else {
            None
        },
        location: if granted.location {
            Some(
                EventLocation::new(content(&event.location, MAX_LOCATION_CHARS))
                    .map_err(bad_text)?,
            )
        } else {
            None
        },
        description: if granted.description {
            Some(
                EventDescription::new(content(&event.description, MAX_DESCRIPTION_CHARS))
                    .map_err(bad_text)?,
            )
        } else {
            None
        },
        attendees,
        organizer: if granted.organizer {
            event.organizer_email.as_deref().and_then(contract_address)
        } else {
            None
        },
        own_response,
    })
}

/// The size of a value in the canonical form, for the bound of 768 KiB (6.4).
pub(crate) fn canonical_len<T: serde::Serialize>(value: &T) -> Option<usize> {
    serde_json::to_value(value)
        .ok()
        .map(|v| canonical(&v).len())
}

/// Why an observation failed (6.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObserveError {
    /// Google's JSON is not of the expected shape.
    Shape(ShapeError),
    /// `attendeesOmitted` is true.
    Cut,
    /// More attendees than the contract holds.
    TooMany,
    /// An attendee address the contract cannot hold (PD25), or an entry with no address.
    Address,
}

/// The observation of 6.5 and the before-image (every content field). The event must not be
/// cancelled (the caller answers `Absent` for that).
pub(crate) fn observation(
    event: &GoogleEvent,
    target: &CalendarTarget,
    fabric_created: bool,
) -> Result<(ObservedEvent, EventView), ObserveError> {
    if event.attendees_omitted {
        return Err(ObserveError::Cut);
    }
    if event.attendees.len() > MAX_ATTENDEES {
        return Err(ObserveError::TooMany);
    }
    let own = own_entry(event, target);
    let mut attendees = Vec::new();
    for (i, a) in event.attendees.iter().enumerate() {
        if Some(i) == own {
            continue;
        }
        let email = a
            .email
            .as_deref()
            .and_then(contract_address)
            .ok_or(ObserveError::Address)?;
        attendees.push(email);
    }
    let before = event_view(event, Granted::ALL, target, fabric_created).map_err(|e| match e {
        ViewError::Shape(s) => ObserveError::Shape(s),
        ViewError::Bound => ObserveError::TooMany,
    })?;
    let observed = ObservedEvent {
        calendar: target.calendar.clone(),
        event_id: before.event_id.clone(),
        revision: before.revision.clone(),
        fabric_created,
        recurring: event.recurrence || event.recurring_event_id,
        default_type: event.event_type.as_deref().is_none_or(|t| t == "default"),
        timed: event.start.as_ref().is_some_and(|s| s.date_time.is_some()),
        invitation: own.is_some() && !event.organizer_self,
        attendees,
        occurrence_start_ms: if event.recurring_event_id {
            event.original_start_ms()
        } else {
            None
        },
    };
    Ok((observed, before))
}

// ---- Bodies (6.6, 6.7) -----------------------------------------------------------------------

fn time_body(time: &EventTime) -> Value {
    json!({"dateTime": time.date_time.as_str(), "timeZone": time.zone.as_str()})
}

/// The body of `events.insert` (6.6): the reserved id, the title, start and end with their
/// zones, and the description and place when given. Nothing else: no attendees, no
/// recurrence, no event type.
/// `id` is the reserved id, checked as 5.3 says ([`crate::reconcile::Effect`]).
#[must_use]
pub(crate) fn create_body(event: &NewEvent, id: &EventId) -> Value {
    let mut body = Map::new();
    body.insert("id".into(), json!(id.as_str()));
    body.insert("summary".into(), json!(event.title.as_str()));
    body.insert("start".into(), time_body(&event.start));
    body.insert("end".into(), time_body(&event.end));
    if let Some(d) = &event.description {
        body.insert("description".into(), json!(d.as_str()));
    }
    if let Some(l) = &event.location {
        body.insert("location".into(), json!(l.as_str()));
    }
    Value::Object(body)
}

/// Whether an update sets no member. Such an order is refused before any request: a `PATCH {}`
/// could still tell everyone and would then read as confirmed (6.7).
#[must_use]
pub(crate) fn changes_nothing(changes: &EventChanges) -> bool {
    changes.title.is_none()
        && changes.start.is_none()
        && changes.end.is_none()
        && changes.description.is_none()
        && changes.location.is_none()
}

/// Whether an approved attendee list names one mailbox twice: two addresses with the same local
/// part whose domains are equal without regard to case (6.12). Such an order is refused before
/// any request. The contract's `email` and its unique list rule this out today (a domain is
/// lower case); the check keeps the body of 6.7 right if that ever changes.
#[must_use]
pub(crate) fn names_an_address_twice<'a>(approved: impl IntoIterator<Item = &'a str>) -> bool {
    let mut seen = HashSet::new();
    approved
        .into_iter()
        .any(|a| !seen.insert(address_key(a).unwrap_or_else(|| a.to_owned())))
}

/// The body of an update (6.7): only the members that are set; a start or end as `dateTime`
/// and `timeZone`, with `date: null` so an all-day event becomes timed.
#[must_use]
pub(crate) fn update_body(changes: &EventChanges) -> Value {
    let mut body = Map::new();
    if let Some(t) = &changes.title {
        body.insert("summary".into(), json!(t.as_str()));
    }
    for (name, time) in [("start", &changes.start), ("end", &changes.end)] {
        if let Some(time) = time {
            body.insert(
                name.into(),
                json!({"dateTime": time.date_time.as_str(), "timeZone": time.zone.as_str(), "date": null}),
            );
        }
    }
    if let Some(d) = &changes.description {
        body.insert("description".into(), json!(d.as_str()));
    }
    if let Some(l) = &changes.location {
        body.insert("location".into(), json!(l.as_str()));
    }
    Value::Object(body)
}

/// The body of an invitation change (6.7): the approved list is the complete list after the
/// change. Each approved address in the approved order; an address that was an attendee keeps
/// its whole entry, a new one is `{"email": ...}`. The own entry, when there is one, is kept
/// as it was: at the place of its address when the approved list names it, else at the end.
/// Every other entry that is not approved is removed.
#[must_use]
pub(crate) fn invitation_body(read: &GoogleEvent, approved: &[Email], own: Option<usize>) -> Value {
    let own_entry = own.map(|i| &read.attendees[i]);
    let mut entries = Vec::new();
    let mut own_placed = false;
    for address in approved {
        if let Some(entry) = own_entry
            && entry
                .email
                .as_deref()
                .is_some_and(|e| same_address(e, address.as_str()))
        {
            if !own_placed {
                entries.push(entry.raw.clone());
                own_placed = true;
            }
            continue;
        }
        let kept = read.attendees.iter().enumerate().find(|(i, a)| {
            Some(*i) != own
                && a.email
                    .as_deref()
                    .is_some_and(|e| same_address(e, address.as_str()))
        });
        entries.push(match kept {
            Some((_, a)) => a.raw.clone(),
            None => json!({"email": address.as_str()}),
        });
    }
    if let Some(entry) = own_entry
        && !own_placed
    {
        entries.push(entry.raw.clone());
    }
    json!({"attendees": entries})
}

/// The body of an answer (6.7): the attendee list exactly as read, with the `responseStatus`
/// of the own entry set to the approved answer. Nothing else differs.
#[must_use]
pub(crate) fn response_body(read: &GoogleEvent, own: usize, response: ProposedResponse) -> Value {
    let status = response_text(response);
    let entries: Vec<Value> = read
        .attendees
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let mut raw = a.raw.clone();
            if i == own
                && let Some(o) = raw.as_object_mut()
            {
                o.insert("responseStatus".into(), json!(status));
            }
            raw
        })
        .collect();
    json!({"attendees": entries})
}

const fn response_text(response: ProposedResponse) -> &'static str {
    match response {
        ProposedResponse::Accepted => "accepted",
        ProposedResponse::Declined => "declined",
    }
}

// ---- Comparing (6.12) ------------------------------------------------------------------------

/// A rewriting of one member that the comparison applies to both sides before it compares them
/// (6.12; 18.7 point 2). A closed, named list.
///
/// Google may store a member in another form than it was sent; an entry is added here only
/// with a result from a live run (design LV18) and a test against the fake. The list is empty
/// today: every member is compared as 6.12 says, with no equivalence invented (PLAN S04).
///
/// It reaches only the members compared as text: the title, the zone, the description and the
/// place. Instants, attendee addresses, answers and other attendees' entries are not compared
/// through it, so an entry for one of those needs its own place in the comparison and its own
/// test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Normalisation {}

impl Normalisation {
    /// The member it applies to.
    #[must_use]
    pub const fn member(self) -> Differs {
        match self {}
    }

    /// `value` in the form the comparison uses.
    fn apply(self, _value: &str) -> String {
        match self {}
    }
}

/// Every normalisation the comparison applies: none today (LV18).
pub const NORMALISATIONS: &[Normalisation] = &[];

/// The one place where a normalisation is applied: to Google's value and to the approved value
/// alike, before they are compared. Only the members compared as text pass through it (the
/// title, the zone, the description and the place); instants, attendee addresses, answers and
/// other attendees' entries do not, so a normalisation of one of those needs its own place.
fn normalised(member: Differs, value: &str) -> Cow<'_, str> {
    NORMALISATIONS
        .iter()
        .filter(|n| n.member() == member)
        .fold(Cow::Borrowed(value), |value, n| Cow::Owned(n.apply(&value)))
}

/// Whether Google's text of `member` and the approved text are the same after the
/// normalisations.
fn same(member: Differs, google: &str, approved: &str) -> bool {
    normalised(member, google) == normalised(member, approved)
}

/// `member` when Google's time is not the approved instant (or is missing).
fn instant_differs(
    google: Option<&GoogleTime>,
    approved: &EventTime,
    member: Differs,
) -> Option<Differs> {
    let want = parse_rfc3339_ms(approved.date_time.as_str()).ok();
    let got = google.and_then(GoogleTime::instant_ms);
    (want.is_none() || got != want).then_some(member)
}

/// The zone, compared only when Google returns the event's own `timeZone` (6.12).
fn zone_differs(google: Option<&GoogleTime>, approved: &EventTime) -> Option<Differs> {
    google
        .and_then(|g| g.time_zone.as_deref())
        .is_some_and(|zone| !same(Differs::Zone, zone, approved.zone.as_str()))
        .then_some(Differs::Zone)
}

/// `member` when Google's text is not the approved one; absent and empty are equal (6.12).
fn text_differs(
    google: Option<&String>,
    approved: Option<&str>,
    member: Differs,
) -> Option<Differs> {
    (!same(
        member,
        google.map_or("", String::as_str),
        approved.unwrap_or(""),
    ))
    .then_some(member)
}

/// The first member of Google's event that is not the approved change (6.12; 18.7 point 2),
/// in this order: title, start, end, zone, description, place, attendees, own answer. `None`
/// when every compared member is as approved. Only a create, an update, an invitation and a
/// response are compared; for every other action there is nothing to compare and it gives
/// `None` ([`matches`] answers `false` for them).
pub(crate) fn differs(
    action: &Action,
    event: &GoogleEvent,
    target: &CalendarTarget,
) -> Option<Differs> {
    match action {
        Action::ProposeCreate(a) => {
            let e = &a.event;
            let title = match event.summary.as_deref() {
                Some(summary) if same(Differs::Title, summary, e.title.as_str()) => None,
                _ => Some(Differs::Title),
            };
            title
                .or_else(|| instant_differs(event.start.as_ref(), &e.start, Differs::Start))
                .or_else(|| instant_differs(event.end.as_ref(), &e.end, Differs::End))
                .or_else(|| zone_differs(event.start.as_ref(), &e.start))
                .or_else(|| zone_differs(event.end.as_ref(), &e.end))
                .or_else(|| {
                    text_differs(
                        event.description.as_ref(),
                        e.description.as_ref().map(|d| d.as_str()),
                        Differs::Description,
                    )
                })
                .or_else(|| {
                    text_differs(
                        event.location.as_ref(),
                        e.location.as_ref().map(|l| l.as_str()),
                        Differs::Location,
                    )
                })
                .or_else(|| (!event.attendees.is_empty()).then_some(Differs::Attendees))
        }
        Action::ProposeUpdate(a) => {
            let c = &a.changes;
            c.title
                .as_ref()
                .and_then(|t| {
                    text_differs(event.summary.as_ref(), Some(t.as_str()), Differs::Title)
                })
                .or_else(|| {
                    c.start
                        .as_ref()
                        .and_then(|t| instant_differs(event.start.as_ref(), t, Differs::Start))
                })
                .or_else(|| {
                    c.end
                        .as_ref()
                        .and_then(|t| instant_differs(event.end.as_ref(), t, Differs::End))
                })
                .or_else(|| {
                    c.start
                        .as_ref()
                        .and_then(|t| zone_differs(event.start.as_ref(), t))
                })
                .or_else(|| {
                    c.end
                        .as_ref()
                        .and_then(|t| zone_differs(event.end.as_ref(), t))
                })
                .or_else(|| {
                    c.description.as_ref().and_then(|d| {
                        text_differs(
                            event.description.as_ref(),
                            Some(d.as_str()),
                            Differs::Description,
                        )
                    })
                })
                .or_else(|| {
                    c.location.as_ref().and_then(|l| {
                        text_differs(event.location.as_ref(), Some(l.as_str()), Differs::Location)
                    })
                })
        }
        Action::ProposeInvitation(a) => {
            let own = own_entry(event, target);
            let own_address = own.and_then(|i| event.attendees[i].email.as_deref());
            let mut google = HashSet::new();
            for (i, attendee) in event.attendees.iter().enumerate() {
                if Some(i) == own {
                    continue;
                }
                match attendee.email.as_deref().and_then(address_key) {
                    Some(key) => {
                        google.insert(key);
                    }
                    None => return Some(Differs::Attendees),
                }
            }
            let approved: Option<HashSet<String>> = a
                .attendees
                .iter()
                .filter(|e| own_address.is_none_or(|own| !same_address(own, e.as_str())))
                .map(|e| address_key(e.as_str()))
                .collect();
            (approved.as_ref() != Some(&google)).then_some(Differs::Attendees)
        }
        Action::ProposeResponse(a) => {
            let answered = own_entry(event, target).is_some_and(|i| {
                event.attendees[i].response.as_deref() == Some(response_text(a.response))
            });
            (!answered).then_some(Differs::OwnResponse)
        }
        Action::ProposeDelete(_)
        | Action::CalendarList(_)
        | Action::CalendarAvailability(_)
        | Action::CalendarRead(_)
        | Action::RequestResult(_) => None,
    }
}

/// Whether `action` is one [`differs`] compares: a create, an update, an invitation or a
/// response.
const fn is_compared(action: &Action) -> bool {
    matches!(
        action,
        Action::ProposeCreate(_)
            | Action::ProposeUpdate(_)
            | Action::ProposeInvitation(_)
            | Action::ProposeResponse(_)
    )
}

/// `matches(action, event)` of 6.12: whether Google's event equals the approved change, as a
/// bool for the read-back of section 7. A thin wrapper of [`differs`]. An action that changes
/// no event never matches.
pub(crate) fn matches(action: &Action, event: &GoogleEvent, target: &CalendarTarget) -> bool {
    is_compared(action) && differs(action, event, target).is_none()
}

/// For the answer to the adapter's own PATCH of a response (6.12): [`differs`], and then
/// [`Differs::OtherAttendee`] when another attendee entry is not as it was read.
pub(crate) fn response_differs(
    action: &Action,
    read: &GoogleEvent,
    after: &GoogleEvent,
    target: &CalendarTarget,
) -> Option<Differs> {
    differs(action, after, target).or_else(|| {
        let others = |event: &GoogleEvent| -> Vec<Value> {
            let own = own_entry(event, target);
            event
                .attendees
                .iter()
                .enumerate()
                .filter(|(i, _)| Some(*i) != own)
                .map(|(_, a)| a.raw.clone())
                .collect()
        };
        (others(read) != others(after)).then_some(Differs::OtherAttendee)
    })
}

/// [`Differs::OriginalStart`] when an answer to one occurrence names an original start and the
/// event's is not that instant (CD-Q05b). `None` when no original start was approved.
pub(crate) fn occurrence_differs(event: &GoogleEvent, approved_ms: Option<i64>) -> Option<Differs> {
    approved_ms
        .is_some_and(|approved| event.original_start_ms() != Some(approved))
        .then_some(Differs::OriginalStart)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric_desktop_contracts::agent::{
        ProposeCreateArguments, ProposeInvitationArguments, ProposeResponseArguments,
        ProposeUpdateArguments, ResponseTarget,
    };
    use fabric_desktop_contracts::common::{CalendarRef, EventRef, SendUpdates};
    use fabric_desktop_contracts::scalar::{EventTitle, Id, Task};

    fn target(primary: bool) -> CalendarTarget {
        CalendarTarget {
            calendar: CalendarRef {
                account_id: Id::new("account-1").expect("valid"),
                calendar_id: Id::new("cal-1").expect("valid"),
            },
            provider_id: CalendarId::new("owner@example.com").expect("valid"),
            time_zone: IanaZone::new("Europe/Berlin").expect("valid"),
            primary,
        }
    }

    fn event(value: Value) -> GoogleEvent {
        GoogleEvent::read(&value).expect("an event")
    }

    fn at(date_time: &str, zone: &str) -> EventTime {
        EventTime {
            date_time: OffsetDateTime::new(date_time).expect("valid"),
            zone: IanaZone::new(zone).expect("valid"),
        }
    }

    fn email(s: &str) -> Email {
        Email::new(s).expect("valid")
    }

    fn ids() -> (Id, Id) {
        (
            Id::new("account-1").expect("valid"),
            Id::new("cal-1").expect("valid"),
        )
    }

    fn event_ref() -> EventRef {
        EventRef {
            event_id: ProviderRef::new("ev1").expect("valid"),
            revision: ProviderRef::new("\"1\"").expect("valid"),
        }
    }

    fn create_action(description: Option<&str>, location: Option<&str>) -> Action {
        let (account_id, calendar_id) = ids();
        Action::ProposeCreate(ProposeCreateArguments {
            account_id,
            calendar_id,
            event: NewEvent {
                title: EventTitle::new("Planning").expect("valid"),
                start: at("2026-11-20T10:00:00+01:00", "Europe/Berlin"),
                end: at("2026-11-20T11:00:00+01:00", "Europe/Berlin"),
                description: description.map(|d| EventDescription::new(d).expect("valid")),
                location: location.map(|l| EventLocation::new(l).expect("valid")),
            },
            task: Task::new("").expect("valid"),
        })
    }

    fn update_action(changes: EventChanges) -> Action {
        let (account_id, calendar_id) = ids();
        Action::ProposeUpdate(ProposeUpdateArguments {
            account_id,
            calendar_id,
            event: event_ref(),
            changes,
            send_updates: SendUpdates::None,
            task: Task::new("").expect("valid"),
        })
    }

    fn no_changes() -> EventChanges {
        EventChanges {
            title: None,
            start: None,
            end: None,
            description: None,
            location: None,
        }
    }

    fn invitation_action(addresses: &[&str]) -> Action {
        let (account_id, calendar_id) = ids();
        Action::ProposeInvitation(ProposeInvitationArguments {
            account_id,
            calendar_id,
            event: event_ref(),
            attendees: fabric_desktop_contracts::agent::Attendees::new(
                addresses.iter().map(|a| email(a)).collect(),
            )
            .expect("valid"),
            send_updates: SendUpdates::All,
            task: Task::new("").expect("valid"),
        })
    }

    fn response_action(response: ProposedResponse) -> Action {
        let (account_id, calendar_id) = ids();
        Action::ProposeResponse(ProposeResponseArguments {
            account_id,
            calendar_id,
            target: ResponseTarget::Event(event_ref()),
            response,
            send_updates: SendUpdates::All,
            task: Task::new("").expect("valid"),
        })
    }

    fn created(extra: Value) -> GoogleEvent {
        let mut base = json!({
            "id": "ev1", "etag": "\"2\"", "summary": "Planning",
            "start": {"dateTime": "2026-11-20T10:00:00+01:00", "timeZone": "Europe/Berlin"},
            "end": {"dateTime": "2026-11-20T11:00:00+01:00", "timeZone": "Europe/Berlin"},
        });
        crate_merge(&mut base, extra);
        event(base)
    }

    fn crate_merge(base: &mut Value, extra: Value) {
        if let (Some(b), Value::Object(e)) = (base.as_object_mut(), extra) {
            for (k, v) in e {
                if v.is_null() {
                    b.remove(&k);
                } else {
                    b.insert(k, v);
                }
            }
        }
    }

    #[test]
    fn cutting_keeps_character_boundaries() {
        assert_eq!(cut("abc", 5), "abc");
        assert_eq!(cut("abc", 3), "abc");
        assert_eq!(cut("abc", 2), "ab");
        assert_eq!(cut("\u{e9}\u{1F600}x", 2), "\u{e9}\u{1F600}");
        assert_eq!(cut("", 0), "");
        assert_eq!(cut("a", 0), "");
    }

    #[test]
    fn addresses_compare_the_local_part_exactly_and_the_domain_without_case() {
        assert!(same_address("Bob@Example.COM", "Bob@example.com"));
        assert!(!same_address("bob@example.com", "Bob@example.com"));
        assert!(!same_address("bob", "bob"));
        assert_eq!(
            contract_address("Bob@Example.COM").expect("fits").as_str(),
            "Bob@example.com"
        );
        assert!(contract_address("no-at-sign").is_none());
        assert!(contract_address("a b@example.com").is_none());
        assert!(contract_address("bob@ex\u{e4}mple.com").is_none());
        assert!(contract_address(&format!("{}@example.com", "a".repeat(65))).is_none());
    }

    #[test]
    fn calendar_roles_follow_the_table_of_6_2() {
        assert_eq!(access_of("owner"), Some(CalendarAccess::Owner));
        assert_eq!(access_of("writer"), Some(CalendarAccess::Writer));
        assert_eq!(
            access_of("writerWithoutPrivateAccess"),
            Some(CalendarAccess::Reader)
        );
        assert_eq!(access_of("reader"), Some(CalendarAccess::Reader));
        assert_eq!(
            access_of("freeBusyReader"),
            Some(CalendarAccess::FreeBusyReader)
        );
        for other in ["", "Owner", "none", "admin"] {
            assert_eq!(access_of(other), None);
        }
    }

    #[test]
    fn an_entry_named_primary_or_without_a_zone_is_left_out() {
        let base = json!({"id": "a@example.com", "accessRole": "owner", "timeZone": "UTC"});
        assert!(calendar_list_item(&base).expect("readable").is_some());
        let mut v = base.clone();
        v["id"] = json!("primary");
        assert!(calendar_list_item(&v).expect("readable").is_none());
        // Only the exact keyword: an id that merely holds the word is an ordinary id.
        let mut v = base.clone();
        v["id"] = json!("primary@example.com");
        assert!(calendar_list_item(&v).expect("readable").is_some());
        let mut v = base;
        v.as_object_mut().expect("an object").remove("timeZone");
        assert!(calendar_list_item(&v).expect("readable").is_none());
    }

    #[test]
    fn a_calendar_name_is_the_override_or_the_summary_cut_to_200_or_no_name() {
        let item = |v: Value| calendar_list_item(&v).expect("readable").expect("kept");
        let base = json!({"id": "a@example.com", "accessRole": "owner", "timeZone": "UTC"});
        let mut v = base.clone();
        v["summary"] = json!("Work");
        v["summaryOverride"] = json!("Mine");
        assert_eq!(item(v).name.as_str(), "Mine");
        let mut v = base.clone();
        v["summary"] = json!("Work");
        v["summaryOverride"] = json!("");
        assert_eq!(item(v).name.as_str(), "Work");
        assert_eq!(item(base.clone()).name.as_str(), NO_NAME);
        let mut v = base.clone();
        v["summary"] = json!("\u{1F600}".repeat(250));
        assert_eq!(item(v).name.as_str().chars().count(), 200);
        let mut v = base.clone();
        v["timeZone"] = json!("Not a zone!");
        assert!(calendar_list_item(&v).expect("readable").is_none());
        let mut v = base.clone();
        v["primary"] = json!("yes");
        assert!(calendar_list_item(&v).is_err());
        let mut v = base;
        v["deleted"] = json!(true);
        assert!(calendar_list_item(&v).expect("readable").is_none());
    }

    #[test]
    fn a_member_of_the_wrong_type_is_an_error_with_a_fixed_label_and_unknown_members_are_ignored() {
        let ok = json!({"id": "e1", "etag": "\"1\"", "colorId": 4, "conferenceData": {"x": [1]}, "summary": null});
        assert!(GoogleEvent::read(&ok).is_ok());
        for bad in [
            json!({"id": 3, "etag": "\"1\""}),
            json!({"id": "e1", "etag": "\"1\"", "summary": 5}),
            json!({"id": "e1", "etag": "\"1\"", "attendees": {}}),
            json!({"id": "e1", "etag": "\"1\"", "attendees": [{"email": "a@b.c", "self": "true"}]}),
            json!({"id": "e1", "etag": "\"1\"", "start": "2026-11-20"}),
            json!({"id": "e1"}),
            json!([]),
        ] {
            let error = GoogleEvent::read(&bad).err().expect("refused");
            let label = format!("{error:?} {error}");
            assert!(!label.contains("e1") && !label.contains("a@b.c"), "{label}");
        }
        assert!(GoogleEvent::read(&json!({"id": "e1", "status": "cancelled"})).is_ok());
    }

    #[test]
    fn a_view_shows_only_the_granted_fields_in_the_calendars_zone() {
        let e = event(json!({
            "id": "occ_20261120T090000Z", "etag": "\"5\"", "recurringEventId": "series",
            "summary": "Standup", "location": "Room 1", "description": "Notes",
            "start": {"dateTime": "2026-11-20T10:00:00.250+01:00", "timeZone": "America/New_York"},
            "end": {"dateTime": "2026-11-20T09:30:00Z"},
            "originalStartTime": {"dateTime": "2026-11-20T10:00:00+01:00"},
            "organizer": {"email": "Boss@Example.com"},
            "attendees": [
                {"email": "owner@example.com", "self": true, "responseStatus": "tentative"},
                {"email": "Ann@Example.COM", "responseStatus": "accepted"},
                {"email": "not an address", "responseStatus": "accepted"},
            ],
        }));
        let t = target(true);
        let none = event_view(&e, Granted::default(), &t, true).expect("a view");
        assert!(none.time.is_none() && none.title.is_none() && none.location.is_none());
        assert!(none.description.is_none() && none.attendees.is_none() && none.organizer.is_none());
        assert!(none.own_response.is_none() && none.occurrence.is_none());
        assert!(none.fabric_created);
        assert_eq!(none.calendar_id.as_str(), "cal-1");
        let all = event_view(&e, Granted::ALL, &t, false).expect("a view");
        let Some(EventSpan::Timed { start, end }) = &all.time else {
            panic!("timed");
        };
        assert_eq!(start.date_time.as_str(), "2026-11-20T10:00:00+01:00");
        assert_eq!(start.zone.as_str(), "Europe/Berlin");
        assert_eq!(end.date_time.as_str(), "2026-11-20T09:30:00+00:00");
        let occurrence = all.occurrence.as_ref().expect("an occurrence");
        assert_eq!(occurrence.instance_id.as_str(), "occ_20261120T090000Z");
        assert_eq!(
            occurrence.original_start_time.zone.as_str(),
            "Europe/Berlin"
        );
        assert_eq!(all.title.as_ref().expect("title").as_str(), "Standup");
        assert_eq!(
            all.organizer.as_ref().expect("organizer").as_str(),
            "Boss@example.com"
        );
        let attendees = all.attendees.as_ref().expect("attendees");
        assert_eq!(attendees.len(), 2, "the unusable address is left out");
        assert_eq!(attendees[1].email.as_str(), "Ann@example.com");
        assert_eq!(all.own_response, Some(ResponseStatus::Tentative));
        let shared = event_view(&e, Granted::ALL, &target(false), false).expect("a view");
        assert_eq!(shared.own_response, None);
    }

    #[test]
    fn all_day_events_are_dates_and_an_all_day_occurrence_has_no_occurrence() {
        let e = event(json!({
            "id": "a1", "etag": "\"1\"", "recurringEventId": "s",
            "start": {"date": "2026-11-20"}, "end": {"date": "2026-11-21"},
            "originalStartTime": {"date": "2026-11-20"},
        }));
        let view = event_view(&e, Granted::ALL, &target(true), false).expect("a view");
        assert_eq!(
            view.time,
            Some(EventSpan::AllDay {
                start_date: Date::new("2026-11-20").expect("valid"),
                end_date: Date::new("2026-11-21").expect("valid"),
            })
        );
        assert!(view.occurrence.is_none());
        let mixed = event(
            json!({"id": "m", "etag": "\"1\"", "start": {"date": "2026-11-20"}, "end": {"dateTime": "2026-11-20T10:00:00Z"}}),
        );
        assert!(matches!(
            event_view(&mixed, Granted::ALL, &target(true), false),
            Err(ViewError::Shape(_))
        ));
    }

    /// 18.7 point 4: a span is the occurrence's own start and end, `Timed` in UTC milliseconds
    /// or `AllDay` with Google's dates; never the original start; anything else is an error.
    /// Finding of review: the view's time and the span come from one read of `start` and
    /// `end`. Each `dateTime` the view shows is the instant the span holds, for timed events;
    /// all-day dates are the same in both; a value refused for one is refused for both.
    #[test]
    fn the_views_time_and_the_span_come_from_one_read_of_start_and_end() {
        let zone = IanaZone::new("Europe/Paris").expect("valid");
        for (start, end) in [
            ("2026-11-20T10:00:00.250+01:00", "2026-11-20T09:30:00Z"),
            ("2026-11-20T23:59:59.999-23:59", "2026-11-21T00:00:00+23:59"),
            ("0000-01-01T00:00:00Z", "9999-12-31T23:59:59.999Z"),
        ] {
            let e = event(json!({"id": "t", "etag": "\"1\"",
                "start": {"dateTime": start}, "end": {"dateTime": end}}));
            let (Ok(EventSpan::Timed { start: vs, end: ve }), Ok(Span::Timed { start_ms, end_ms })) =
                (span(&e, &zone), event_span(&e))
            else {
                panic!("both timed");
            };
            assert_eq!(
                parse_rfc3339_ms(vs.date_time.as_str()),
                Ok(start_ms - start_ms.rem_euclid(1000))
            );
            assert_eq!(
                parse_rfc3339_ms(ve.date_time.as_str()),
                Ok(end_ms - end_ms.rem_euclid(1000))
            );
            assert_eq!(start_ms, parse_rfc3339_ms(start).expect("valid"));
            assert_eq!(end_ms, parse_rfc3339_ms(end).expect("valid"));
        }
        let all_day = event(
            json!({"id": "a", "etag": "\"1\"", "start": {"date": "2026-11-20"}, "end": {"date": "2026-11-22"}}),
        );
        let (
            Ok(EventSpan::AllDay {
                start_date: vs,
                end_date: ve,
            }),
            Ok(Span::AllDay {
                start_date,
                end_date,
            }),
        ) = (span(&all_day, &zone), event_span(&all_day))
        else {
            panic!("both all day");
        };
        assert_eq!((vs, ve), (start_date, end_date));
        for bad in [
            "2026-02-30T10:00:00Z",
            "2026-11-20T24:00:00Z",
            "2026-11-20T10:00:00+24:00",
            "2026-11-20T10:00:60Z",
            "2026-11-20t10:00:00Z",
            "2026-11-20T10:00:00.Z",
        ] {
            let e = event(json!({"id": "b", "etag": "\"1\"",
                "start": {"dateTime": bad}, "end": {"dateTime": "2026-11-20T11:00:00Z"}}));
            assert!(span(&e, &zone).is_err() && event_span(&e).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_span_is_the_events_own_start_and_end_as_instants_or_dates() {
        let occurrence = event(json!({
            "id": "o", "etag": "\"1\"", "recurringEventId": "s",
            "originalStartTime": {"dateTime": "2026-11-20T08:00:00+01:00"},
            "start": {"dateTime": "2026-11-20T10:00:00.250+01:00", "timeZone": "America/New_York"},
            "end": {"dateTime": "2026-11-20T09:30:00Z"},
        }));
        assert_eq!(
            event_span(&occurrence),
            Ok(Span::Timed {
                start_ms: parse_rfc3339_ms("2026-11-20T09:00:00.250Z").expect("valid"),
                end_ms: parse_rfc3339_ms("2026-11-20T09:30:00Z").expect("valid"),
            })
        );
        let all_day = event(
            json!({"id": "a", "etag": "\"1\"", "start": {"date": "2026-11-20"}, "end": {"date": "2026-11-22"}}),
        );
        assert_eq!(
            event_span(&all_day),
            Ok(Span::AllDay {
                start_date: Date::new("2026-11-20").expect("valid"),
                end_date: Date::new("2026-11-22").expect("valid"),
            })
        );
        for bad in [
            json!({"id": "m", "etag": "\"1\"", "start": {"date": "2026-11-20"}, "end": {"dateTime": "2026-11-20T10:00:00Z"}}),
            json!({"id": "m", "etag": "\"1\"", "start": {"dateTime": "2026-11-20T10:00:00Z"}}),
            json!({"id": "m", "etag": "\"1\""}),
            json!({"id": "m", "etag": "\"1\"", "start": {"dateTime": "tomorrow"}, "end": {"dateTime": "2026-11-20T10:00:00Z"}}),
            json!({"id": "m", "etag": "\"1\"", "start": {"date": "20 November"}, "end": {"date": "2026-11-21"}}),
        ] {
            assert!(event_span(&event(bad)).is_err());
        }
    }

    #[test]
    fn texts_are_cut_at_the_contracts_bounds() {
        let e = event(json!({
            "id": "t", "etag": "\"1\"", "summary": "\u{1F600}".repeat(1100),
            "location": "x".repeat(2000), "description": "d".repeat(9000),
        }));
        let view = event_view(&e, Granted::ALL.no_time(), &target(true), false).expect("a view");
        assert_eq!(view.title.expect("title").as_str().chars().count(), 1024);
        assert_eq!(view.location.expect("location").as_str().len(), 1024);
        assert_eq!(view.description.expect("description").as_str().len(), 8192);
        let empty = event(json!({"id": "t", "etag": "\"1\""}));
        let view =
            event_view(&empty, Granted::ALL.no_time(), &target(true), false).expect("a view");
        assert_eq!(view.title.expect("title").as_str(), "");
        assert_eq!(view.location.expect("location").as_str(), "");
    }

    impl Granted {
        fn no_time(mut self) -> Granted {
            self.time = false;
            self
        }
    }

    #[test]
    fn a_cut_or_too_long_attendee_list_fails_a_read_that_asked_for_attendees() {
        let cut_list = event(
            json!({"id": "c", "etag": "\"1\"", "attendeesOmitted": true, "attendees": [{"email": "a@example.com"}]}),
        );
        let mut g = Granted::default();
        assert!(event_view(&cut_list, g, &target(true), false).is_ok());
        g.attendees = true;
        assert_eq!(
            event_view(&cut_list, g, &target(true), false).err(),
            Some(ViewError::Bound)
        );
        let many: Vec<Value> = (0..2001)
            .map(|i| json!({"email": format!("p{i}@example.com")}))
            .collect();
        let long = event(json!({"id": "c", "etag": "\"1\"", "attendees": many}));
        assert_eq!(
            event_view(&long, g, &target(true), false).err(),
            Some(ViewError::Bound)
        );
    }

    #[test]
    fn the_fields_parameter_asks_only_for_the_granted_members_and_always_the_span() {
        assert_eq!(
            Granted::default().fields_param(),
            "nextPageToken,items(id,etag,status,recurringEventId,start(date,dateTime),end(date,dateTime))"
        );
        assert_eq!(
            Granted::of(&[EventField::Title]).fields_param(),
            "nextPageToken,items(id,etag,status,recurringEventId,start(date,dateTime),end(date,dateTime),summary)"
        );
        assert_eq!(
            Granted::of(&[EventField::OwnResponse]).fields_param(),
            "nextPageToken,items(id,etag,status,recurringEventId,start(date,dateTime),end(date,dateTime),attendees(email,responseStatus,self),attendeesOmitted)"
        );
        assert_eq!(
            Granted::of(&[EventField::Time]).fields_param(),
            "nextPageToken,items(id,etag,status,recurringEventId,start,end,originalStartTime)"
        );
        assert_eq!(
            Granted::ALL.fields_param(),
            "nextPageToken,items(id,etag,status,recurringEventId,start,end,originalStartTime,summary,location,description,attendees(email,responseStatus,self),attendeesOmitted,organizer(email))"
        );
    }

    #[test]
    fn the_own_entry_exists_only_on_the_primary_calendar_with_the_calendars_address() {
        let e = event(json!({"id": "e", "etag": "\"1\"", "attendees": [
            {"email": "other@example.com", "self": true},
            {"email": "Owner@EXAMPLE.com", "self": true},
        ]}));
        assert_eq!(own_entry(&e, &target(false)), None);
        assert_eq!(
            own_entry(&e, &target(true)),
            None,
            "local part differs in case"
        );
        let e = event(json!({"id": "e", "etag": "\"1\"", "attendees": [
            {"email": "other@example.com", "self": true},
            {"email": "owner@EXAMPLE.com", "self": true},
        ]}));
        assert_eq!(own_entry(&e, &target(true)), Some(1));
        let unmarked = event(
            json!({"id": "e", "etag": "\"1\"", "attendees": [{"email": "owner@example.com"}]}),
        );
        assert_eq!(own_entry(&unmarked, &target(true)), None);
    }

    #[test]
    fn the_invitation_body_keeps_entries_adds_new_ones_and_keeps_the_own_entry() {
        let read = event(json!({"id": "e", "etag": "\"1\"", "attendees": [
            {"email": "owner@example.com", "self": true, "responseStatus": "accepted"},
            {"email": "Ann@Example.com", "responseStatus": "declined", "comment": "x"},
            {"email": "room@resource.example.com", "resource": true},
            {"email": "boss@example.com", "organizer": true},
        ]}));
        let own = own_entry(&read, &target(true));
        let body = invitation_body(
            &read,
            &[email("new@example.com"), email("Ann@example.com")],
            own,
        );
        assert_eq!(
            body,
            json!({"attendees": [
                {"email": "new@example.com"},
                {"email": "Ann@Example.com", "responseStatus": "declined", "comment": "x"},
                {"email": "owner@example.com", "self": true, "responseStatus": "accepted"},
            ]})
        );
        let named = invitation_body(
            &read,
            &[email("owner@example.com"), email("new@example.com")],
            own,
        );
        assert_eq!(named["attendees"][0]["responseStatus"], json!("accepted"));
        assert_eq!(named["attendees"].as_array().expect("list").len(), 2);
        let shared = invitation_body(&read, &[email("new@example.com")], None);
        assert_eq!(shared, json!({"attendees": [{"email": "new@example.com"}]}));
    }

    #[test]
    fn the_response_body_changes_only_the_own_entrys_answer() {
        let read = event(json!({"id": "e", "etag": "\"1\"", "attendees": [
            {"email": "a@example.com", "responseStatus": "accepted", "optional": true},
            {"email": "owner@example.com", "self": true, "responseStatus": "needsAction", "comment": "c"},
        ]}));
        let body = response_body(&read, 1, ProposedResponse::Declined);
        assert_eq!(
            body,
            json!({"attendees": [
                {"email": "a@example.com", "responseStatus": "accepted", "optional": true},
                {"email": "owner@example.com", "self": true, "responseStatus": "declined", "comment": "c"},
            ]})
        );
    }

    #[test]
    fn the_create_and_update_bodies_hold_exactly_the_approved_members() {
        let Action::ProposeCreate(a) = create_action(None, Some("")) else {
            panic!("create");
        };
        let id = crate::http::ids::ReservedEventId::new("abcdefghij0123456789abcdef")
            .expect("valid")
            .as_event_id();
        assert_eq!(
            create_body(&a.event, &id),
            json!({
                "id": "abcdefghij0123456789abcdef", "summary": "Planning",
                "start": {"dateTime": "2026-11-20T10:00:00+01:00", "timeZone": "Europe/Berlin"},
                "end": {"dateTime": "2026-11-20T11:00:00+01:00", "timeZone": "Europe/Berlin"},
                "location": "",
            })
        );
        let mut changes = no_changes();
        changes.start = Some(at("2026-11-20T12:00:00+01:00", "Europe/Berlin"));
        changes.title = Some(EventTitle::new("New").expect("valid"));
        assert_eq!(
            update_body(&changes),
            json!({"summary": "New", "start": {"dateTime": "2026-11-20T12:00:00+01:00", "timeZone": "Europe/Berlin", "date": null}})
        );
        // An update that sets nothing is refused before any request (6.7); a set member,
        // even an empty text, is a change.
        assert!(changes_nothing(&no_changes()));
        assert!(!changes_nothing(&changes));
        let mut empty_text = no_changes();
        empty_text.location = Some(EventLocation::new("").expect("valid"));
        assert!(!changes_nothing(&empty_text));
    }

    #[test]
    fn an_attendee_list_names_one_address_twice_only_when_local_parts_are_equal_and_domains_differ_in_case()
     {
        assert!(names_an_address_twice([
            "ann@example.com",
            "ann@EXAMPLE.com"
        ]));
        assert!(names_an_address_twice([
            "bob@example.com",
            "ann@Example.Com",
            "ann@example.COM"
        ]));
        assert!(names_an_address_twice([
            "ann@example.com",
            "ann@example.com"
        ]));
        // The local part is compared exactly: two mailboxes.
        assert!(!names_an_address_twice([
            "ann@example.com",
            "Ann@example.com"
        ]));
        assert!(!names_an_address_twice([
            "ann@example.com",
            "bob@example.com"
        ]));
        assert!(!names_an_address_twice([]));
        // The contract cannot hold an upper-case domain, so an order cannot carry such a pair.
        assert!(Email::new("ann@EXAMPLE.com").is_err());
    }

    /// g45: `matches` for each tool, each member, absent against empty, instants written with
    /// different offsets, the own address in an approved list.
    #[test]
    fn matches_compares_each_member_of_each_tool() {
        let t = target(true);
        // Create.
        let create = create_action(None, None);
        assert!(matches(&create, &created(json!({})), &t));
        assert!(matches(
            &create,
            &created(json!({"description": "", "location": ""})),
            &t
        ));
        assert!(!matches(&create, &created(json!({"description": "x"})), &t));
        assert!(!matches(&create, &created(json!({"location": "x"})), &t));
        assert!(!matches(&create, &created(json!({"summary": "Other"})), &t));
        assert!(!matches(&create, &created(json!({"summary": null})), &t));
        assert!(!matches(
            &create,
            &created(json!({"attendees": [{"email": "a@example.com"}]})),
            &t
        ));
        assert!(matches(&create, &created(json!({"attendees": []})), &t));
        // The same instants written with other offsets match; another instant does not.
        assert!(matches(
            &create,
            &created(
                json!({"start": {"dateTime": "2026-11-20T09:00:00Z"}, "end": {"dateTime": "2026-11-20T05:00:00-05:00"}})
            ),
            &t
        ));
        assert!(!matches(
            &create,
            &created(json!({"start": {"dateTime": "2026-11-20T10:00:00Z"}})),
            &t
        ));
        // The zone counts only when Google returns one.
        assert!(!matches(
            &create,
            &created(
                json!({"start": {"dateTime": "2026-11-20T10:00:00+01:00", "timeZone": "Europe/Paris"}})
            ),
            &t
        ));
        assert!(!matches(
            &create,
            &created(json!({"start": {"date": "2026-11-20"}})),
            &t
        ));
        let create_with = create_action(Some("Agenda"), Some("Room"));
        assert!(matches(
            &create_with,
            &created(json!({"description": "Agenda", "location": "Room"})),
            &t
        ));
        assert!(!matches(&create_with, &created(json!({})), &t));

        // Update: only set members are compared.
        let current = created(json!({"summary": "Old", "description": "D"}));
        assert!(matches(&update_action(no_changes()), &current, &t));
        let mut c = no_changes();
        c.title = Some(EventTitle::new("Old").expect("valid"));
        assert!(matches(&update_action(c), &current, &t));
        let mut c = no_changes();
        c.title = Some(EventTitle::new("New").expect("valid"));
        assert!(!matches(&update_action(c), &current, &t));
        let mut c = no_changes();
        c.description = Some(EventDescription::new("").expect("valid"));
        assert!(!matches(&update_action(c.clone()), &current, &t));
        assert!(matches(&update_action(c), &created(json!({})), &t));
        let mut c = no_changes();
        c.end = Some(at("2026-11-20T10:00:00+00:00", "Europe/Berlin"));
        assert!(matches(&update_action(c), &created(json!({})), &t));
        let mut c = no_changes();
        c.start = Some(at("2026-11-20T10:30:00+01:00", "Europe/Berlin"));
        assert!(!matches(&update_action(c), &created(json!({})), &t));
        let mut c = no_changes();
        c.location = Some(EventLocation::new("Hall").expect("valid"));
        assert!(matches(
            &update_action(c.clone()),
            &created(json!({"location": "Hall"})),
            &t
        ));
        assert!(!matches(&update_action(c), &created(json!({})), &t));

        // Invitation: the set without the own entry and without the own address.
        let with_people = created(json!({"attendees": [
            {"email": "owner@example.com", "self": true},
            {"email": "Ann@EXAMPLE.com"},
            {"email": "bob@example.com"},
        ]}));
        assert!(matches(
            &invitation_action(&["bob@example.com", "Ann@example.com"]),
            &with_people,
            &t
        ));
        assert!(matches(
            &invitation_action(&["owner@example.com", "bob@example.com", "Ann@example.com"]),
            &with_people,
            &t
        ));
        assert!(!matches(
            &invitation_action(&["bob@example.com"]),
            &with_people,
            &t
        ));
        assert!(!matches(
            &invitation_action(&["ann@example.com", "bob@example.com"]),
            &with_people,
            &t
        ));
        // On a shared calendar the marked entry is another person and counts.
        assert!(!matches(
            &invitation_action(&["bob@example.com", "Ann@example.com"]),
            &with_people,
            &target(false)
        ));
        assert!(matches(
            &invitation_action(&["owner@example.com", "bob@example.com", "Ann@example.com"]),
            &with_people,
            &target(false)
        ));

        // Response: the own entry has the approved status.
        let answered = |status: &str| {
            created(json!({"attendees": [
                {"email": "owner@example.com", "self": true, "responseStatus": status},
                {"email": "a@example.com", "responseStatus": "needsAction"},
            ]}))
        };
        assert!(matches(
            &response_action(ProposedResponse::Accepted),
            &answered("accepted"),
            &t
        ));
        assert!(!matches(
            &response_action(ProposedResponse::Accepted),
            &answered("declined"),
            &t
        ));
        assert!(matches(
            &response_action(ProposedResponse::Declined),
            &answered("declined"),
            &t
        ));
        assert!(!matches(
            &response_action(ProposedResponse::Accepted),
            &answered("accepted"),
            &target(false)
        ));
        let read = answered("needsAction");
        assert_eq!(
            response_differs(
                &response_action(ProposedResponse::Accepted),
                &read,
                &answered("accepted"),
                &t
            ),
            None
        );
        let mut changed_other = answered("accepted");
        changed_other.attendees[1].raw =
            json!({"email": "a@example.com", "responseStatus": "accepted"});
        assert_eq!(
            response_differs(
                &response_action(ProposedResponse::Accepted),
                &read,
                &changed_other,
                &t
            ),
            Some(Differs::OtherAttendee)
        );
        assert_eq!(
            response_differs(
                &response_action(ProposedResponse::Declined),
                &read,
                &changed_other,
                &t
            ),
            Some(Differs::OwnResponse),
            "the own answer is named before another entry"
        );
    }

    /// 18.7 point 2: the comparison names the first member that differs, as a fixed label, in
    /// the order title, start, end, zone, description, place, attendees, own answer.
    #[test]
    fn the_comparison_names_the_first_member_that_differs() {
        let t = target(true);
        let create = create_action(Some("Agenda"), Some("Room"));
        let base = json!({"description": "Agenda", "location": "Room"});
        // `null` members are kept here, so that `created` removes them.
        let with = |extra: Value| {
            let mut v = base.clone();
            if let (Some(b), Value::Object(e)) = (v.as_object_mut(), extra) {
                b.extend(e);
            }
            created(v)
        };
        assert_eq!(differs(&create, &with(json!({})), &t), None);
        let cases = [
            (json!({"summary": "Other"}), Differs::Title),
            (json!({"summary": null}), Differs::Title),
            (
                json!({"start": {"dateTime": "2026-11-20T10:30:00+01:00"}}),
                Differs::Start,
            ),
            (json!({"start": null}), Differs::Start),
            (
                json!({"end": {"dateTime": "2026-11-20T11:30:00+01:00"}}),
                Differs::End,
            ),
            (
                json!({"end": {"dateTime": "2026-11-20T11:00:00+01:00", "timeZone": "Asia/Calcutta"}}),
                Differs::Zone,
            ),
            (json!({"description": "Agenda "}), Differs::Description),
            (json!({"location": "Hall"}), Differs::Location),
            (
                json!({"attendees": [{"email": "a@example.com"}]}),
                Differs::Attendees,
            ),
            // Two members differ: the first in the order is named.
            (
                json!({"summary": "Other", "location": "Hall"}),
                Differs::Title,
            ),
            (
                json!({"description": "x", "start": {"dateTime": "2026-11-20T09:00:00+01:00"}}),
                Differs::Start,
            ),
        ];
        for (extra, want) in cases {
            let label = extra.to_string();
            let event = with(extra);
            assert_eq!(differs(&create, &event, &t), Some(want), "{label}");
            assert!(!matches(&create, &event, &t), "{label}");
        }
        let mut c = no_changes();
        c.start = Some(at("2026-11-20T10:00:00+01:00", "Asia/Kolkata"));
        assert_eq!(
            differs(&update_action(c), &created(json!({})), &t),
            Some(Differs::Zone)
        );
        assert_eq!(
            differs(
                &invitation_action(&["bob@example.com"]),
                &created(json!({})),
                &t
            ),
            Some(Differs::Attendees)
        );
        assert_eq!(
            differs(
                &response_action(ProposedResponse::Accepted),
                &created(json!({})),
                &t
            ),
            Some(Differs::OwnResponse)
        );
        // The original start of an occurrence.
        let occurrence = created(
            json!({"recurringEventId": "s", "originalStartTime": {"dateTime": "2026-11-20T10:00:00+01:00"}}),
        );
        let at_nine_utc = parse_rfc3339_ms("2026-11-20T09:00:00Z").expect("valid");
        assert_eq!(occurrence_differs(&occurrence, Some(at_nine_utc)), None);
        assert_eq!(occurrence_differs(&occurrence, None), None);
        assert_eq!(
            occurrence_differs(&occurrence, Some(at_nine_utc + 1)),
            Some(Differs::OriginalStart)
        );
        // A delete, and every action that changes no event, is never compared and never
        // matches.
        let (account_id, calendar_id) = ids();
        let delete =
            Action::ProposeDelete(fabric_desktop_contracts::agent::ProposeDeleteArguments {
                account_id,
                calendar_id,
                event: event_ref(),
                send_updates: SendUpdates::None,
                task: Task::new("").expect("valid"),
            });
        assert_eq!(differs(&delete, &created(json!({})), &t), None);
        assert!(!matches(&delete, &created(json!({})), &t));
    }

    /// 18.7 point 2: the list of normalisations is closed and empty until a live run (LV18)
    /// and a test against the fake add an entry; a trimmed description and a zone alias are
    /// differences today.
    #[test]
    fn the_list_of_normalisations_is_empty() {
        assert!(NORMALISATIONS.is_empty());
        for member in Differs::ALL {
            assert_eq!(normalised(member, "Agenda  "), "Agenda  ");
            assert!(!same(member, "Agenda", "Agenda  "));
        }
    }

    #[test]
    fn the_observation_follows_6_5() {
        let t = target(true);
        let invitation = event(json!({
            "id": "inv", "etag": "\"3\"", "eventType": "default",
            "start": {"dateTime": "2026-11-20T10:00:00+01:00"}, "end": {"dateTime": "2026-11-20T11:00:00+01:00"},
            "organizer": {"email": "boss@example.com"},
            "attendees": [
                {"email": "owner@example.com", "self": true},
                {"email": "Boss@Example.COM", "organizer": true},
                {"email": "room@resource.Example.com", "resource": true},
            ],
        }));
        let (o, before) = observation(&invitation, &t, true).expect("observed");
        assert!(o.invitation && o.timed && o.default_type && !o.recurring && o.fabric_created);
        let addresses: Vec<&str> = o.attendees.iter().map(|e| e.as_str()).collect();
        assert_eq!(addresses, ["Boss@example.com", "room@resource.example.com"]);
        assert_eq!(o.revision.as_str(), "\"3\"");
        assert!(before.title.is_some() && before.attendees.is_some());
        // On a shared calendar there is no invitation and the marked entry is an attendee.
        let (o, _) = observation(&invitation, &target(false), false).expect("observed");
        assert!(!o.invitation);
        assert_eq!(o.attendees.len(), 3);
        // The organizer's own copy is no invitation.
        let organized = event(
            json!({"id": "x", "etag": "\"1\"", "organizer": {"email": "owner@example.com", "self": true},
            "attendees": [{"email": "owner@example.com", "self": true}], "start": {"date": "2026-11-20"}, "end": {"date": "2026-11-21"}, "eventType": "focusTime"}),
        );
        let (o, _) = observation(&organized, &t, false).expect("observed");
        assert!(!o.invitation && !o.timed && !o.default_type && o.attendees.is_empty());
        let master = event(
            json!({"id": "m", "etag": "\"1\"", "recurrence": ["RRULE:FREQ=DAILY"], "start": {"dateTime": "2026-11-20T10:00:00Z"}, "end": {"dateTime": "2026-11-20T11:00:00Z"}}),
        );
        let (o, _) = observation(&master, &t, false).expect("observed");
        assert!(o.recurring && o.occurrence_start_ms.is_none());
        let occurrence = event(
            json!({"id": "m_20261120T100000Z", "etag": "\"1\"", "recurringEventId": "m",
            "originalStartTime": {"dateTime": "2026-11-20T11:00:00+01:00"}, "start": {"dateTime": "2026-11-20T11:00:00+01:00"}, "end": {"dateTime": "2026-11-20T12:00:00+01:00"}}),
        );
        let (o, _) = observation(&occurrence, &t, false).expect("observed");
        assert!(o.recurring);
        assert_eq!(
            o.occurrence_start_ms,
            Some(parse_rfc3339_ms("2026-11-20T10:00:00Z").expect("valid"))
        );
        let all_day_occurrence = event(
            json!({"id": "d_20261120", "etag": "\"1\"", "recurringEventId": "d",
            "originalStartTime": {"date": "2026-11-20"}, "start": {"date": "2026-11-20"}, "end": {"date": "2026-11-21"}}),
        );
        let (o, _) = observation(&all_day_occurrence, &t, false).expect("observed");
        assert!(o.recurring && o.occurrence_start_ms.is_none());
        // A cut list and an address the contract cannot hold fail the observation.
        let cut_list = event(
            json!({"id": "c", "etag": "\"1\"", "attendeesOmitted": true, "start": {"date": "2026-11-20"}, "end": {"date": "2026-11-21"}}),
        );
        assert_eq!(
            observation(&cut_list, &t, false).err(),
            Some(ObserveError::Cut)
        );
        let bad = event(
            json!({"id": "c", "etag": "\"1\"", "attendees": [{"email": "a b@example.com"}], "start": {"date": "2026-11-20"}, "end": {"date": "2026-11-21"}}),
        );
        assert_eq!(
            observation(&bad, &t, false).err(),
            Some(ObserveError::Address)
        );
        let no_address = event(
            json!({"id": "c", "etag": "\"1\"", "attendees": [{"responseStatus": "accepted"}], "start": {"date": "2026-11-20"}, "end": {"date": "2026-11-21"}}),
        );
        assert_eq!(
            observation(&no_address, &t, false).err(),
            Some(ObserveError::Address)
        );
    }
}
