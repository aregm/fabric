//! Reading events (6.4), observing one event (6.5) and the reconcile read (section 7).

use fabric_desktop_contracts::agent::ToolResult;
use fabric_desktop_contracts::common::FailureCode;
use fabric_desktop_contracts::scalar::{AnyList, Id, ProviderRef};
use serde_json::Value;

use super::{
    CalendarTarget, EventsQuery, Google, Observed, ReadAnswer, check_effect, check_targets,
};
use crate::http::ids::EventId;
use crate::http::{BodyBound, Method, Request, Target};
use crate::outcome::{ReadFailure, ReadKind, Sentence};
use crate::reconcile::{Effect, ReadBack};
use crate::secret::Secret;
use crate::translate::{
    GoogleEvent, Granted, ObserveError, ViewError, canonical_len, event_view, matches, observation,
};

/// The most events one read returns (6.4, PD23).
pub const MAX_EVENTS: usize = 2_500;

/// The most bytes of canonical result one read returns: 768 KiB (6.4, PD23). Counted over the
/// whole result as it is returned: the views, the commas between them and the wrapper with
/// `observed_at`.
pub const MAX_RESULT_BYTES: usize = 768 * 1024;

fn window_too_large() -> ReadFailure {
    ReadFailure::rejected(FailureCode::InvalidArguments, Sentence::WindowTooLarge)
}

fn event_id_form() -> ReadFailure {
    ReadFailure::rejected(FailureCode::InvalidArguments, Sentence::EventIdForm)
}

/// Well-formed JSON from Google that is not a usable event: reading it again would give the
/// same, so it is Rejected rather than Transient (6.10).
fn unreadable() -> ReadFailure {
    ReadFailure::rejected(FailureCode::InvalidArguments, Sentence::EventUnreadable)
}

/// One event read with `GET /calendars/<id>/events/<event id>?timeZone=<zone>`.
pub(crate) enum EventRead {
    /// The event as Google gave it, perhaps cancelled.
    Event(Box<GoogleEvent>),
    /// 404 or 410.
    Absent,
}

impl Google {
    /// `GET` one event in the calendar's zone (6.5, 6.7, section 7), with the large bound: one
    /// event may carry many attendees. A body that is not a JSON object, or an event whose `id`
    /// is not the one asked for, is no usable answer: Transient. A JSON object that is not a
    /// usable event (a member of the wrong type, an unknown `responseStatus`, start and end of
    /// different kinds) is Rejected `invalid_arguments` with the "event cannot be read"
    /// sentence.
    pub(crate) fn get_event(
        &mut self,
        account: &Id,
        token: &Secret,
        target: &CalendarTarget,
        event_id: &EventId,
        deadline_ms: u64,
    ) -> Result<EventRead, ReadFailure> {
        let request = Request::new(
            Method::Get,
            Target::Event(target.provider_id.clone(), event_id.clone()),
            BodyBound::Large,
        )
        .query("timeZone", target.time_zone.as_str());
        let body = match self.send_read(account, token, request, deadline_ms, ReadKind::Event)? {
            ReadAnswer::Absent => return Ok(EventRead::Absent),
            ReadAnswer::Body(body) => body,
        };
        // Another event than the one asked for is no answer to this read.
        if body.get("id").and_then(Value::as_str) != Some(event_id.as_str()) {
            return Err(ReadFailure::transient());
        }
        let event = GoogleEvent::read(&body).map_err(|_| unreadable())?;
        if !event.cancelled
            && let Err(ViewError::Shape(_)) = event_view(&event, Granted::ALL, target, false)
        {
            return Err(unreadable());
        }
        Ok(EventRead::Event(Box::new(event)))
    }

    /// Reads every page of every calendar of `query` in one order, inside one deadline (6.4,
    /// PD35). Before each page it asks `still_allowed`; when that is false the read stops with
    /// `Stopped`, makes no further request and returns nothing it gathered. A result exists
    /// only when the last page of the last calendar arrived; past 2 500 events or 768 KiB of
    /// canonical result, or with a cut or too long attendee list when attendees were asked
    /// for, the read fails with "Ask for a shorter window" (PD23). Cancelled items are skipped.
    /// One item that is not a usable event fails the whole read as Rejected `invalid_arguments`
    /// with the "event cannot be read" sentence; a page that is not an answer at all is
    /// Transient.
    pub fn read_events(
        &mut self,
        account: &Id,
        query: &EventsQuery,
        deadline_ms: u64,
        still_allowed: &dyn Fn() -> bool,
    ) -> Result<ToolResult, ReadFailure> {
        if query.targets.is_empty()
            || query.targets.len() > super::freebusy::MAX_FREE_BUSY_CALENDARS
        {
            return Err(ReadFailure::rejected(
                FailureCode::InvalidArguments,
                Sentence::CalendarCount,
            ));
        }
        check_targets(account, &query.targets.iter().collect::<Vec<_>>())?;
        let granted = Granted::of(&query.fields);
        let fields = granted.fields_param();
        let token = self.read_token(account, deadline_ms)?;
        let mut events = Vec::new();
        let mut bytes = 0usize;
        for target in &query.targets {
            let mut page_token: Option<String> = None;
            loop {
                if !still_allowed() {
                    return Err(ReadFailure::Stopped);
                }
                let mut request = Request::new(
                    Method::Get,
                    Target::Events(target.provider_id.clone()),
                    BodyBound::Large,
                )
                .query("singleEvents", "true")
                .query("orderBy", "startTime")
                .query("showDeleted", "false")
                .query("timeMin", query.window.start.as_str())
                .query("timeMax", query.window.end.as_str())
                .query("timeZone", target.time_zone.as_str())
                .query("maxResults", "250")
                .query("fields", fields.as_str());
                if let Some(next) = page_token.take() {
                    request = request.query("pageToken", next);
                }
                let ReadAnswer::Body(body) =
                    self.send_read(account, &token, request, deadline_ms, ReadKind::Calendar)?
                else {
                    return Err(ReadFailure::transient());
                };
                let items = match body.get("items") {
                    None | Some(Value::Null) => &[][..],
                    Some(Value::Array(items)) => items.as_slice(),
                    Some(_) => return Err(ReadFailure::transient()),
                };
                for item in items {
                    let Ok(event) = GoogleEvent::read(item) else {
                        return Err(unreadable());
                    };
                    if event.cancelled {
                        continue;
                    }
                    let created = ProviderRef::new(event.id.as_str()).is_ok_and(|id| {
                        query
                            .fabric_created
                            .contains(&(target.calendar.calendar_id.clone(), id))
                    });
                    let view = match event_view(&event, granted, target, created) {
                        Ok(view) => view,
                        Err(ViewError::Bound) => return Err(window_too_large()),
                        Err(ViewError::Shape(_)) => return Err(unreadable()),
                    };
                    // The view and the comma before it; the wrapper is counted at the end.
                    bytes = bytes
                        .saturating_add(canonical_len(&view).unwrap_or(usize::MAX))
                        .saturating_add(usize::from(!events.is_empty()));
                    events.push(view);
                    if events.len() > MAX_EVENTS || bytes > MAX_RESULT_BYTES {
                        return Err(window_too_large());
                    }
                }
                match body.get("nextPageToken") {
                    None | Some(Value::Null) => break,
                    Some(Value::String(next)) => page_token = Some(next.clone()),
                    Some(_) => return Err(ReadFailure::transient()),
                }
            }
        }
        let observed_at =
            crate::time::format_timestamp(i64::try_from(self.now_ms()).unwrap_or(i64::MAX))
                .map_err(|_| ReadFailure::transient())?;
        let events = AnyList::new(events).map_err(|_| ReadFailure::transient())?;
        let result = ToolResult::Events {
            observed_at,
            events,
        };
        // The bound counts the result as it really is, wrapper and all (PD23).
        if canonical_len(&result).is_none_or(|len| len > MAX_RESULT_BYTES) {
            return Err(window_too_large());
        }
        Ok(result)
    }

    /// Observes one event (6.5, PO2): `GET` it in the calendar's zone and give the observation
    /// and a before-image with every content field, or `Absent` (404, 410, or `status`
    /// `cancelled`). A cut attendee list, more attendees than the contract holds, or an
    /// attendee address the contract cannot hold fails it with `invalid_arguments`: the check
    /// of who is notified must see everyone (PD25). An event id of another form than 5.3
    /// allows makes no request.
    pub fn observe(
        &mut self,
        account: &Id,
        target: &CalendarTarget,
        event_id: &str,
        fabric_created: bool,
        deadline_ms: u64,
    ) -> Result<Observed, ReadFailure> {
        check_targets(account, &[target])?;
        let event_id = EventId::new(event_id).map_err(|_| event_id_form())?;
        let token = self.read_token(account, deadline_ms)?;
        let event = match self.get_event(account, &token, target, &event_id, deadline_ms)? {
            EventRead::Absent => return Ok(Observed::Absent),
            EventRead::Event(event) if event.cancelled => return Ok(Observed::Absent),
            EventRead::Event(event) => *event,
        };
        let failed = |sentence| ReadFailure::rejected(FailureCode::InvalidArguments, sentence);
        match observation(&event, target, fabric_created) {
            Ok((observation, before)) => Ok(Observed::Found {
                observation,
                before: Box::new(before),
            }),
            Err(ObserveError::Cut) => Err(failed(Sentence::AttendeesCut)),
            Err(ObserveError::TooMany) => Err(failed(Sentence::TooManyAttendees)),
            Err(ObserveError::Address) => Err(failed(Sentence::AttendeeAddress)),
            Err(ObserveError::Shape(_)) => Err(failed(Sentence::EventUnreadable)),
        }
    }

    /// The read of section 7 (PO4): what Google holds under the effect's event, and nothing of
    /// Google's shape. `Found` with the revision and whether it equals what was approved (6.12),
    /// `Cancelled`, or `NotFound` (404 or 410). The event is the reserved id for a create, the
    /// approved event (or occurrence instance) otherwise. An effect that names another account
    /// or calendar than the target, or ids of another form than 5.3 allows, makes no request
    /// (the check [`Google::change`] makes too). For an answer to one occurrence, an instance
    /// whose original start is not the approved instant does not match (CD-Q05b).
    pub fn reconcile_read(
        &mut self,
        account: &Id,
        target: &CalendarTarget,
        effect: &Effect,
        deadline_ms: u64,
    ) -> Result<ReadBack, ReadFailure> {
        let ids = check_effect(account, target, effect)
            .map_err(|sentence| ReadFailure::rejected(FailureCode::InvalidArguments, sentence))?;
        let token = self.read_token(account, deadline_ms)?;
        match self.get_event(account, &token, target, &ids.event_id, deadline_ms)? {
            EventRead::Absent => Ok(ReadBack::NotFound),
            EventRead::Event(event) if event.cancelled => Ok(ReadBack::Cancelled),
            EventRead::Event(event) => {
                let revision = ProviderRef::new(event.etag.as_str()).map_err(|_| unreadable())?;
                let same_occurrence = ids
                    .original_start_ms
                    .is_none_or(|approved| event.original_start_ms() == Some(approved));
                Ok(ReadBack::Found {
                    revision,
                    matches: same_occurrence && matches(&effect.action, &event, target),
                })
            }
        }
    }
}
