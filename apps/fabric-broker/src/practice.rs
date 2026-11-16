//! The practice calendar of a debug build: an in-memory calendar on the
//! synthetic account that carries out an authorized request, so a person can see one go the
//! whole way without a Google account.
//!
//! It exists only in a debug build with the synthetic agent and a configured Isabelle
//! (`Config::practice` checks these three). The synthetic agent needs the test key, which the
//! configuration checks; the scratch data directory is the host's and the harness's rule, not
//! the broker's. It acts only on a dispatch order,
//! which the core makes only after a consumed authorization resting on an accepted Isabelle
//! permit. Its state lives in memory and is never written to a file. No event title reaches
//! stderr or its `Debug` form.
//!
//! "PW" numbers, such as PW 4.3, refer to
//! `docs/plans/calendar-desktop-prover-wiring-design.md`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use fabric_broker_core::format_timestamp;
use fabric_broker_core::ports::{
    AttemptDone, AttemptReport, DispatchOrder, ObservedEvent, Provider,
};
use fabric_desktop_contracts::agent::{self, Action};
use fabric_desktop_contracts::common::{CalendarRef, EventRef, EventTime, FailureCode};
use fabric_desktop_contracts::scalar::{
    EventTitle, Explanation, IanaZone, Id, OffsetDateTime, ProviderRef,
};

use crate::synthetic::{ACCOUNT_ID, CALENDAR_ID};

/// The id of the seeded event that Fabric did not create.
pub const FOREIGN_EVENT_ID: &str = "practice-foreign-1";
/// The title of the seeded event that Fabric did not create.
pub const FOREIGN_TITLE: &str = "Practice: dentist (not made by Fabric)";

/// The rejection of an order whose parts disagree.
const NOT_TOGETHER: &str = "the order does not hold together";
/// The rejection of another calendar.
const ONLY_SYNTHETIC: &str = "the practice calendar has only the synthetic calendar";
/// The rejection of a tool other than create, update and delete.
const ONLY_CHANGES: &str = "the practice calendar carries out create, update and delete only";
/// The rejection of a create whose reserved id is taken.
const ID_TAKEN: &str = "an event with this id exists";
/// The rejection of an update or delete of an event that is not there.
const NO_SUCH_EVENT: &str = "no such event";
/// The rejection of an update or delete written against another revision.
const CHANGED: &str = "the event changed";

const HOUR_MS: u64 = 3_600_000;
const DAY_MS: u64 = 24 * HOUR_MS;

/// Faults the practice calendar injects, for in-process tests only; the shell always passes the
/// default, which injects none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Faults {
    /// How many of the next applied changes are kept but answered with no answer.
    pub lose_answers: u32,
    /// How many of the next dispatches fail with a transient failure and change nothing.
    pub transient: u32,
}

/// One event of the practice calendar, as tests see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PracticeEvent {
    /// The event's id.
    pub event_id: String,
    /// Its revision, a decimal string.
    pub revision: String,
    /// True when Fabric created it.
    pub fabric_created: bool,
    /// The request that created it, if Fabric did.
    pub request_id: Option<Id>,
    /// True once deleted.
    pub deleted: bool,
}

/// One stored event. Title and times are the contract values as received, never parsed.
#[derive(Clone)]
struct Stored {
    event_id: String,
    revision: u64,
    title: EventTitle,
    start: EventTime,
    end: EventTime,
    fabric_created: bool,
    request_id: Option<Id>,
    deleted: bool,
}

impl Stored {
    fn view(&self) -> PracticeEvent {
        PracticeEvent {
            event_id: self.event_id.clone(),
            revision: self.revision.to_string(),
            fabric_created: self.fabric_created,
            request_id: self.request_id.clone(),
            deleted: self.deleted,
        }
    }

    /// The reference an applied answer names; `None` only for an id that is not a provider
    /// reference, which the checks before a change rule out.
    fn reference(&self) -> Option<EventRef> {
        Some(EventRef {
            event_id: ProviderRef::new(self.event_id.as_str()).ok()?,
            revision: ProviderRef::new(self.revision.to_string()).ok()?,
        })
    }
}

/// The practice calendar's state: the events in insertion order, the applied effects, the
/// faults still to inject and the dispatch counter.
#[derive(Default)]
struct State {
    events: Vec<Stored>,
    /// Effect id -> the event reference its applied answer named.
    effects: HashMap<Id, EventRef>,
    faults: Faults,
    dispatches: u32,
}

impl std::fmt::Debug for State {
    /// Counts only: no title, no time.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State")
            .field("events", &self.events.len())
            .field("effects", &self.effects.len())
            .field("faults", &self.faults)
            .field("dispatches", &self.dispatches)
            .finish()
    }
}

impl State {
    fn find(&self, event_id: &str) -> Option<&Stored> {
        self.events.iter().find(|e| e.event_id == event_id)
    }

    fn find_mut(&mut self, event_id: &str) -> Option<&mut Stored> {
        self.events.iter_mut().find(|e| e.event_id == event_id)
    }
}

/// The practice calendar: shared state behind a mutex, cloned into the provider worker, the
/// shell's observation hook and the synthetic agent. `Debug` because `SyntheticAgent` derives
/// it and holds one.
#[derive(Clone, Debug)]
pub struct PracticeCalendar {
    /// The events, the applied effects and the dispatch counter.
    state: Arc<Mutex<State>>,
}

impl PracticeCalendar {
    /// Seeds the foreign event seven days after `now_ms` at 09:00 UTC.
    #[must_use]
    pub fn new(now_ms: u64, faults: Faults) -> PracticeCalendar {
        let start_ms = (now_ms / DAY_MS + 7) * DAY_MS + 9 * HOUR_MS;
        let foreign = Stored {
            event_id: FOREIGN_EVENT_ID.to_owned(),
            revision: 1,
            title: EventTitle::new(FOREIGN_TITLE).expect("a valid practice title"),
            start: utc_time(start_ms),
            end: utc_time(start_ms + HOUR_MS),
            fabric_created: false,
            request_id: None,
            deleted: false,
        };
        PracticeCalendar {
            state: Arc::new(Mutex::new(State {
                events: vec![foreign],
                faults,
                ..State::default()
            })),
        }
    }

    /// The provider that carries out dispatch orders on this calendar.
    #[must_use]
    pub fn provider(&self) -> PracticeProvider {
        PracticeProvider {
            calendar: self.clone(),
        }
    }

    /// The observed event of an update or delete of a live practice event; `None` otherwise.
    #[must_use]
    pub fn observation(&self, action: &agent::Action) -> Option<ObservedEvent> {
        let (account_id, calendar_id, event) = match action {
            Action::ProposeUpdate(a) => (&a.account_id, &a.calendar_id, &a.event),
            Action::ProposeDelete(a) => (&a.account_id, &a.calendar_id, &a.event),
            _ => return None,
        };
        if !is_synthetic(account_id, calendar_id) {
            return None;
        }
        let state = self.lock()?;
        let stored = state.find(event.event_id.as_str())?;
        if stored.deleted {
            return None;
        }
        Some(ObservedEvent {
            calendar: CalendarRef {
                account_id: account_id.clone(),
                calendar_id: calendar_id.clone(),
            },
            event_id: stored.event_id.clone(),
            revision: stored.revision.to_string(),
            fabric_created: stored.fabric_created,
            recurring: false,
            default_type: true,
            timed: true,
            invitation: false,
            attendees: Vec::new(),
            occurrence_start_ms: None,
        })
    }

    /// The event with `event_id`, deleted or not.
    #[must_use]
    pub fn event(&self, event_id: &str) -> Option<PracticeEvent> {
        self.lock()?.find(event_id).map(Stored::view)
    }

    /// The event the request `request_id` created.
    #[must_use]
    pub fn event_of_request(&self, request_id: &Id) -> Option<PracticeEvent> {
        self.lock()?
            .events
            .iter()
            .find(|e| e.request_id.as_ref() == Some(request_id))
            .map(Stored::view)
    }

    /// Calls of `dispatch` so far, for tests that assert zero dispatches.
    #[must_use]
    pub fn dispatches(&self) -> u32 {
        self.lock().map_or(0, |state| state.dispatches)
    }

    /// The state, or `None` when a holder panicked.
    fn lock(&self) -> Option<MutexGuard<'_, State>> {
        self.state.lock().ok()
    }

    /// PW 4.3: the report of one order, at wall time `now_ms`; the rows below are the rows of
    /// its table.
    fn carry_out(&self, order: &DispatchOrder, now_ms: u64) -> AttemptReport {
        let Some(mut state) = self.lock() else {
            return AttemptReport::NoAnswer;
        };
        state.dispatches = state.dispatches.saturating_add(1);
        // Row 1.
        if now_ms >= order.deadline_ms {
            return AttemptReport::NoAnswer;
        }
        // Row 2.
        if !holds_together(order) {
            return rejected(FailureCode::ProviderRejected, NOT_TOGETHER);
        }
        // Row 3.
        if !is_synthetic(&order.account_id, &order.calendar_id) {
            return rejected(FailureCode::ProviderRejected, ONLY_SYNTHETIC);
        }
        // Row 4.
        if let Some(event) = state.effects.get(&order.effect_id) {
            return AttemptReport::Applied {
                event: Some(event.clone()),
            };
        }
        // Row 5.
        if !matches!(
            order.action,
            Action::ProposeCreate(_) | Action::ProposeUpdate(_) | Action::ProposeDelete(_)
        ) {
            return rejected(FailureCode::ProviderRejected, ONLY_CHANGES);
        }
        // Row 6.
        if state.faults.transient > 0 {
            state.faults.transient -= 1;
            return AttemptReport::Transient {
                retry_after_ms: None,
            };
        }
        // Row 7.
        let reference = match change(&mut state, order) {
            Ok(reference) => reference,
            Err(report) => return report,
        };
        state
            .effects
            .insert(order.effect_id.clone(), reference.clone());
        // Row 8.
        if state.faults.lose_answers > 0 {
            state.faults.lose_answers -= 1;
            return AttemptReport::NoAnswer;
        }
        // Row 9.
        AttemptReport::Applied {
            event: Some(reference),
        }
    }
}

/// Makes the change of a create, update or delete, or the rejection.
fn change(state: &mut State, order: &DispatchOrder) -> Result<EventRef, AttemptReport> {
    let stored = match &order.action {
        Action::ProposeCreate(a) => {
            // `holds_together` checked that a create carries a valid reserved id.
            let Some(event_id) = order.provider_event_id.as_deref() else {
                return Err(rejected(FailureCode::ProviderRejected, NOT_TOGETHER));
            };
            if state.find(event_id).is_some() {
                return Err(rejected(FailureCode::ProviderRejected, ID_TAKEN));
            }
            state.events.push(Stored {
                event_id: event_id.to_owned(),
                revision: 1,
                title: a.event.title.clone(),
                start: a.event.start.clone(),
                end: a.event.end.clone(),
                fabric_created: true,
                request_id: Some(order.request_id.clone()),
                deleted: false,
            });
            state.events.last().cloned()
        }
        Action::ProposeUpdate(a) => {
            let stored = live(state, &a.event)?;
            if let Some(title) = &a.changes.title {
                stored.title = title.clone();
            }
            if let Some(start) = &a.changes.start {
                stored.start = start.clone();
            }
            if let Some(end) = &a.changes.end {
                stored.end = end.clone();
            }
            stored.revision += 1;
            Some(stored.clone())
        }
        Action::ProposeDelete(a) => {
            let stored = live(state, &a.event)?;
            stored.deleted = true;
            stored.revision += 1;
            Some(stored.clone())
        }
        _ => None,
    };
    stored
        .as_ref()
        .and_then(Stored::reference)
        .ok_or_else(|| rejected(FailureCode::ProviderRejected, ONLY_CHANGES))
}

/// The live event an update or delete names, at the revision it was written against.
fn live<'s>(state: &'s mut State, event: &EventRef) -> Result<&'s mut Stored, AttemptReport> {
    let Some(stored) = state
        .find_mut(event.event_id.as_str())
        .filter(|s| !s.deleted)
    else {
        return Err(rejected(FailureCode::InvalidArguments, NO_SUCH_EVENT));
    };
    if stored.revision.to_string() != event.revision.as_str() {
        return Err(rejected(FailureCode::RevisionConflict, CHANGED));
    }
    Ok(stored)
}

/// Whether the order holds together: the authorization is consumed and names this request and
/// this action, the order names the action's calendar, and a create carries a reserved id that
/// is a provider reference.
fn holds_together(order: &DispatchOrder) -> bool {
    let authorization = &order.authorization;
    if authorization.request_id != order.request_id
        || authorization.action_digest != order.action.digest()
        || authorization.consumed_at.is_none()
    {
        return false;
    }
    let target = match &order.action {
        Action::ProposeCreate(a) => Some((&a.account_id, &a.calendar_id)),
        Action::ProposeUpdate(a) => Some((&a.account_id, &a.calendar_id)),
        Action::ProposeInvitation(a) => Some((&a.account_id, &a.calendar_id)),
        Action::ProposeResponse(a) => Some((&a.account_id, &a.calendar_id)),
        Action::ProposeDelete(a) => Some((&a.account_id, &a.calendar_id)),
        _ => None,
    };
    if target.is_some_and(|(account, calendar)| {
        *account != order.account_id || *calendar != order.calendar_id
    }) {
        return false;
    }
    match &order.action {
        Action::ProposeCreate(_) => order
            .provider_event_id
            .as_deref()
            .is_some_and(ProviderRef::is_valid),
        _ => true,
    }
}

/// True for the synthetic account's one calendar.
fn is_synthetic(account_id: &Id, calendar_id: &Id) -> bool {
    account_id.as_str() == ACCOUNT_ID && calendar_id.as_str() == CALENDAR_ID
}

fn rejected(code: FailureCode, text: &'static str) -> AttemptReport {
    AttemptReport::Rejected {
        code,
        explanation: Explanation::new(text).expect("a valid fixed explanation"),
    }
}

/// An event time at `ms` in UTC: `YYYY-MM-DDTHH:MM:SS+00:00`, zone `Etc/UTC`.
fn utc_time(ms: u64) -> EventTime {
    let text = format_timestamp(ms);
    EventTime {
        date_time: OffsetDateTime::new(format!("{}+00:00", &text.as_str()[..19]))
            .expect("a valid practice time"),
        zone: IanaZone::new("Etc/UTC").expect("a valid zone"),
    }
}

/// The provider of the practice calendar; runs on the provider worker.
pub struct PracticeProvider {
    /// The calendar it changes.
    calendar: PracticeCalendar,
}

impl Provider for PracticeProvider {
    /// The first check of the order that fails decides; every call counts.
    fn dispatch(&mut self, order: &DispatchOrder) -> AttemptDone {
        let report = self.calendar.carry_out(order, crate::shell::now_ms());
        AttemptDone {
            request_id: order.request_id.clone(),
            effect_id: order.effect_id.clone(),
            attempt: order.attempt,
            report,
            finished_at_ms: crate::shell::now_ms(),
        }
    }
}

/// Orders for the unit tests of this module and of the synthetic agent.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use fabric_desktop_contracts::decode_record;
    use fabric_desktop_contracts::owner::Authorization;
    use serde_json::json;

    /// The reserved id of every create these orders carry: 26 base32hex digits.
    pub(crate) const RESERVED: &str = "0123456789abcdefghijklmnop";

    pub(crate) fn id(text: &str) -> Id {
        Id::new(text).expect("a valid test id")
    }

    pub(crate) fn authorization(request: &str, action: &Action, consumed: bool) -> Authorization {
        let bytes = serde_json::to_vec(&json!({
            "profile": "fabric-broker-authorization/1",
            "authorization_id": "authorization-1",
            "request_id": request,
            "agent_id": "synthetic-agent",
            "action_digest": action.digest(),
            "proof_id": "proof-1",
            "grant_id": "grant-1",
            "issued_at": "2026-11-20T10:00:00.000Z",
            "expires_at": "2026-11-20T10:01:00.000Z",
            "lifetime_seconds": 60,
            "consumed_at": consumed.then_some("2026-11-20T10:00:01.000Z")
        }))
        .expect("a JSON value serializes");
        decode_record(&bytes).expect("a valid test authorization")
    }

    /// A consistent order for `action` from request `request`, effect `effect`.
    pub(crate) fn order(request: &str, effect: &str, action: Action) -> DispatchOrder {
        let provider_event_id =
            matches!(action, Action::ProposeCreate(_)).then(|| RESERVED.to_owned());
        DispatchOrder {
            request_id: id(request),
            effect_id: id(effect),
            attempt: 1,
            authorization: authorization(request, &action, true),
            action,
            account_id: id(ACCOUNT_ID),
            calendar_id: id(CALENDAR_ID),
            provider_event_id,
            deadline_ms: u64::MAX,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{RESERVED, authorization, id, order};
    use super::*;
    use serde_json::{Value, json};

    const NOW_MS: u64 = 1_795_190_400_123;

    fn action(tool: &str, arguments: Value) -> Action {
        serde_json::from_value(json!({"tool": tool, "arguments": arguments}))
            .expect("a valid test action")
    }

    fn time(text: &str) -> Value {
        json!({"date_time": text, "zone": "Etc/UTC"})
    }

    fn create_on(account: &str, calendar: &str) -> Action {
        action(
            "fabric_calendar_propose_create",
            json!({
                "account_id": account,
                "calendar_id": calendar,
                "event": {
                    "title": "Practice test block",
                    "start": time("2026-11-27T16:00:00+00:00"),
                    "end": time("2026-11-27T17:00:00+00:00"),
                    "description": null,
                    "location": null
                },
                "task": "test"
            }),
        )
    }

    fn create() -> Action {
        create_on(ACCOUNT_ID, CALENDAR_ID)
    }

    fn update(event_id: &str, revision: &str) -> Action {
        action(
            "fabric_calendar_propose_update",
            json!({
                "account_id": ACCOUNT_ID,
                "calendar_id": CALENDAR_ID,
                "event": {"event_id": event_id, "revision": revision},
                "changes": {
                    "title": null,
                    "start": time("2026-11-27T16:30:00+00:00"),
                    "end": time("2026-11-27T17:30:00+00:00"),
                    "description": null,
                    "location": null
                },
                "send_updates": "none",
                "task": "test"
            }),
        )
    }

    fn delete(event_id: &str, revision: &str) -> Action {
        action(
            "fabric_calendar_propose_delete",
            json!({
                "account_id": ACCOUNT_ID,
                "calendar_id": CALENDAR_ID,
                "event": {"event_id": event_id, "revision": revision},
                "send_updates": "none",
                "task": "test"
            }),
        )
    }

    fn calendar() -> PracticeCalendar {
        PracticeCalendar::new(NOW_MS, Faults::default())
    }

    fn applied(event_id: &str, revision: &str) -> AttemptReport {
        AttemptReport::Applied {
            event: Some(EventRef {
                event_id: ProviderRef::new(event_id).expect("a valid reference"),
                revision: ProviderRef::new(revision).expect("a valid reference"),
            }),
        }
    }

    fn rejection(code: FailureCode, text: &str) -> AttemptReport {
        AttemptReport::Rejected {
            code,
            explanation: Explanation::new(text).expect("a valid explanation"),
        }
    }

    fn dispatch(calendar: &PracticeCalendar, order: &DispatchOrder) -> AttemptReport {
        calendar.provider().dispatch(order).report
    }

    /// A calendar holding the event of request `request-a`, created by effect `effect-a`.
    fn with_created() -> PracticeCalendar {
        let calendar = calendar();
        let report = dispatch(&calendar, &order("request-a", "effect-a", create()));
        assert_eq!(report, applied(RESERVED, "1"));
        calendar
    }

    #[test]
    fn the_seed_holds_one_foreign_event_a_week_ahead_at_nine_utc() {
        let calendar = calendar();
        let foreign = calendar.event(FOREIGN_EVENT_ID).expect("seeded");
        assert_eq!(foreign.revision, "1");
        assert!(!foreign.fabric_created);
        assert_eq!(foreign.request_id, None);
        assert!(!foreign.deleted);
        let state = calendar.lock().expect("not poisoned");
        assert_eq!(state.events.len(), 1);
        assert_eq!(
            state.events[0].start.date_time.as_str(),
            "2026-11-27T09:00:00+00:00"
        );
        assert_eq!(
            state.events[0].end.date_time.as_str(),
            "2026-11-27T10:00:00+00:00"
        );
        assert_eq!(state.events[0].title.as_str(), FOREIGN_TITLE);
    }

    #[test]
    fn the_debug_form_names_no_title() {
        let text = format!("{:?}", with_created());
        assert!(!text.contains(FOREIGN_TITLE), "{text}");
        assert!(!text.contains("Practice test block"), "{text}");
        assert!(!text.contains("2026-11-27"), "{text}");
    }

    #[test]
    fn create_uses_the_reserved_id_at_revision_1() {
        let calendar = with_created();
        let event = calendar.event(RESERVED).expect("created");
        assert_eq!(
            event,
            PracticeEvent {
                event_id: RESERVED.to_owned(),
                revision: "1".to_owned(),
                fabric_created: true,
                request_id: Some(id("request-a")),
                deleted: false,
            }
        );
        assert_eq!(calendar.event_of_request(&id("request-a")), Some(event));
        assert_eq!(calendar.event_of_request(&id("request-b")), None);
        // A second create with the same reserved id under another effect is refused.
        let report = dispatch(&calendar, &order("request-b", "effect-b", create()));
        assert_eq!(report, rejection(FailureCode::ProviderRejected, ID_TAKEN));
        assert_eq!(calendar.dispatches(), 2);
    }

    #[test]
    fn a_retry_of_an_applied_effect_is_applied_once() {
        let calendar = with_created();
        let mut retry = order("request-a", "effect-a", create());
        retry.attempt = 2;
        let done = calendar.provider().dispatch(&retry);
        assert_eq!(done.attempt, 2);
        assert_eq!(done.report, applied(RESERVED, "1"));
        assert_eq!(calendar.lock().expect("not poisoned").events.len(), 2);

        let update = order("request-c", "effect-c", update(RESERVED, "1"));
        assert_eq!(dispatch(&calendar, &update), applied(RESERVED, "2"));
        assert_eq!(dispatch(&calendar, &update), applied(RESERVED, "2"));
        assert_eq!(calendar.event(RESERVED).expect("there").revision, "2");
        assert_eq!(calendar.dispatches(), 4);
    }

    #[test]
    fn update_needs_the_current_revision() {
        let calendar = with_created();
        let stale = order("request-c", "effect-c", update(RESERVED, "2"));
        assert_eq!(
            dispatch(&calendar, &stale),
            rejection(FailureCode::RevisionConflict, CHANGED)
        );
        assert_eq!(calendar.event(RESERVED).expect("there").revision, "1");

        let missing = order("request-d", "effect-d", update("no-such-event", "1"));
        assert_eq!(
            dispatch(&calendar, &missing),
            rejection(FailureCode::InvalidArguments, NO_SUCH_EVENT)
        );

        let current = order("request-e", "effect-e", update(RESERVED, "1"));
        assert_eq!(dispatch(&calendar, &current), applied(RESERVED, "2"));
        let state = calendar.lock().expect("not poisoned");
        let stored = state.find(RESERVED).expect("there");
        assert_eq!(stored.start.date_time.as_str(), "2026-11-27T16:30:00+00:00");
        assert_eq!(stored.end.date_time.as_str(), "2026-11-27T17:30:00+00:00");
        assert_eq!(stored.title.as_str(), "Practice test block");
    }

    #[test]
    fn delete_leaves_a_tombstone() {
        let calendar = with_created();
        let first = order("request-d", "effect-d", delete(RESERVED, "1"));
        assert_eq!(dispatch(&calendar, &first), applied(RESERVED, "2"));
        let event = calendar.event(RESERVED).expect("kept as a tombstone");
        assert!(event.deleted);
        assert_eq!(event.revision, "2");
        assert_eq!(calendar.event_of_request(&id("request-a")), Some(event));

        let again = order("request-e", "effect-e", delete(RESERVED, "2"));
        assert_eq!(
            dispatch(&calendar, &again),
            rejection(FailureCode::InvalidArguments, NO_SUCH_EVENT)
        );
        let update = order("request-f", "effect-f", update(RESERVED, "2"));
        assert_eq!(
            dispatch(&calendar, &update),
            rejection(FailureCode::InvalidArguments, NO_SUCH_EVENT)
        );
        // The tombstone keeps its id taken.
        let create = order("request-g", "effect-g", create());
        assert_eq!(
            dispatch(&calendar, &create),
            rejection(FailureCode::ProviderRejected, ID_TAKEN)
        );
    }

    #[test]
    fn other_calendars_and_other_tools_are_rejected() {
        let calendar = calendar();
        let mut other = order("request-a", "effect-a", create_on(ACCOUNT_ID, "other"));
        other.calendar_id = id("other");
        assert_eq!(
            dispatch(&calendar, &other),
            rejection(FailureCode::ProviderRejected, ONLY_SYNTHETIC)
        );
        let mut account = order("request-b", "effect-b", create_on("other", CALENDAR_ID));
        account.account_id = id("other");
        assert_eq!(
            dispatch(&calendar, &account),
            rejection(FailureCode::ProviderRejected, ONLY_SYNTHETIC)
        );

        let invitation = action(
            "fabric_calendar_propose_invitation",
            json!({
                "account_id": ACCOUNT_ID,
                "calendar_id": CALENDAR_ID,
                "event": {"event_id": FOREIGN_EVENT_ID, "revision": "1"},
                "attendees": ["someone@example.com"],
                "send_updates": "none",
                "task": "test"
            }),
        );
        let list = action("fabric_calendar_list", json!({}));
        for tool in [invitation, list] {
            assert_eq!(
                dispatch(&calendar, &order("request-c", "effect-c", tool)),
                rejection(FailureCode::ProviderRejected, ONLY_CHANGES)
            );
        }
        assert_eq!(calendar.lock().expect("not poisoned").events.len(), 1);
        assert_eq!(
            calendar.event(FOREIGN_EVENT_ID).expect("there").revision,
            "1"
        );
    }

    #[test]
    fn an_order_that_does_not_hold_together_is_rejected() {
        let calendar = calendar();
        let not_together = rejection(FailureCode::ProviderRejected, NOT_TOGETHER);

        let mut unconsumed = order("request-a", "effect-a", create());
        unconsumed.authorization = authorization("request-a", &create(), false);
        let mut other_request = order("request-a", "effect-a", create());
        other_request.authorization = authorization("request-b", &create(), true);
        let mut other_action = order("request-a", "effect-a", create());
        other_action.authorization =
            authorization("request-a", &delete(FOREIGN_EVENT_ID, "1"), true);
        let mut other_calendar = order("request-a", "effect-a", create());
        other_calendar.calendar_id = id("other");
        let mut no_reserved_id = order("request-a", "effect-a", create());
        no_reserved_id.provider_event_id = None;
        let mut bad_reserved_id = order("request-a", "effect-a", create());
        bad_reserved_id.provider_event_id = Some("has space".to_owned());

        for order in [
            unconsumed,
            other_request,
            other_action,
            other_calendar,
            no_reserved_id,
            bad_reserved_id,
        ] {
            assert_eq!(dispatch(&calendar, &order), not_together);
        }
        assert_eq!(calendar.lock().expect("not poisoned").events.len(), 1);
        assert_eq!(calendar.dispatches(), 6);
    }

    #[test]
    fn past_the_deadline_is_no_answer_and_no_change() {
        let calendar = calendar();
        let mut late = order("request-a", "effect-a", create());
        late.deadline_ms = 0;
        assert_eq!(dispatch(&calendar, &late), AttemptReport::NoAnswer);
        assert_eq!(calendar.event(RESERVED), None);
        assert_eq!(calendar.dispatches(), 1);

        let mut late_delete = order("request-b", "effect-b", delete(FOREIGN_EVENT_ID, "1"));
        late_delete.deadline_ms = crate::shell::now_ms();
        assert_eq!(dispatch(&calendar, &late_delete), AttemptReport::NoAnswer);
        assert!(!calendar.event(FOREIGN_EVENT_ID).expect("there").deleted);
    }

    #[test]
    fn a_lost_answer_applies_and_answers_no_answer() {
        let calendar = PracticeCalendar::new(
            NOW_MS,
            Faults {
                lose_answers: 1,
                transient: 0,
            },
        );
        let create = order("request-a", "effect-a", create());
        assert_eq!(dispatch(&calendar, &create), AttemptReport::NoAnswer);
        assert_eq!(calendar.event(RESERVED).expect("applied").revision, "1");
        // The fault is spent, and the same effect is not applied twice.
        assert_eq!(dispatch(&calendar, &create), applied(RESERVED, "1"));
        assert_eq!(calendar.lock().expect("not poisoned").events.len(), 2);
    }

    #[test]
    fn transient_changes_nothing() {
        let calendar = PracticeCalendar::new(
            NOW_MS,
            Faults {
                lose_answers: 0,
                transient: 2,
            },
        );
        let delete = order("request-a", "effect-a", delete(FOREIGN_EVENT_ID, "1"));
        for _ in 0..2 {
            assert_eq!(
                dispatch(&calendar, &delete),
                AttemptReport::Transient {
                    retry_after_ms: None
                }
            );
            let foreign = calendar.event(FOREIGN_EVENT_ID).expect("there");
            assert!(!foreign.deleted);
            assert_eq!(foreign.revision, "1");
        }
        assert_eq!(dispatch(&calendar, &delete), applied(FOREIGN_EVENT_ID, "2"));
        assert_eq!(calendar.dispatches(), 3);
    }

    #[test]
    fn the_observation_names_live_update_and_delete_targets_only() {
        let calendar = with_created();
        let observed = calendar
            .observation(&update(RESERVED, "1"))
            .expect("a live practice event");
        assert_eq!(
            observed,
            ObservedEvent {
                calendar: CalendarRef {
                    account_id: id(ACCOUNT_ID),
                    calendar_id: id(CALENDAR_ID),
                },
                event_id: RESERVED.to_owned(),
                revision: "1".to_owned(),
                fabric_created: true,
                recurring: false,
                default_type: true,
                timed: true,
                invitation: false,
                attendees: Vec::new(),
                occurrence_start_ms: None,
            }
        );
        // The observation names the current revision, whatever the action was written against.
        let foreign = calendar
            .observation(&delete(FOREIGN_EVENT_ID, "7"))
            .expect("the foreign event");
        assert_eq!(foreign.revision, "1");
        assert!(!foreign.fabric_created);

        assert_eq!(calendar.observation(&create()), None);
        assert_eq!(calendar.observation(&update("no-such-event", "1")), None);
        let mut other = update(RESERVED, "1");
        if let Action::ProposeUpdate(a) = &mut other {
            a.calendar_id = id("other");
        }
        assert_eq!(calendar.observation(&other), None);

        let deleted = order("request-d", "effect-d", delete(RESERVED, "1"));
        assert_eq!(dispatch(&calendar, &deleted), applied(RESERVED, "2"));
        assert_eq!(calendar.observation(&delete(RESERVED, "2")), None);
        assert_eq!(calendar.observation(&update(RESERVED, "2")), None);
    }

    #[test]
    fn a_poisoned_calendar_answers_nothing_and_observes_nothing() {
        let calendar = with_created();
        let held = calendar.clone();
        let _ = std::thread::spawn(move || {
            let _guard = held.state.lock();
            panic!("poison the practice state");
        })
        .join();
        assert_eq!(
            dispatch(
                &calendar,
                &order("request-c", "effect-c", update(RESERVED, "1"))
            ),
            AttemptReport::NoAnswer
        );
        assert_eq!(calendar.observation(&update(RESERVED, "1")), None);
        assert_eq!(calendar.event(RESERVED), None);
        assert_eq!(calendar.dispatches(), 0);
    }
}
