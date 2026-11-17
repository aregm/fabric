//! Helpers of the Calendar tests (g18 to g43): a fake Google with accounts and calendars, the
//! adapter pointed at it, a test [`AccessTokens`] that hands out tokens the fake issued, action
//! builders, and the check every test ends with: the fake's whole log holds no path with
//! `primary` (PD22), and every Calendar request carried the token of its own account (6.1).
//!
//! Every operation of a test runs inside [`World::as_account`], which records which part of the
//! log belongs to which account; [`World::check_log`] then holds each request to it.

#![allow(dead_code)]

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use fabric_desktop_contracts::agent::{
    Action, Attendees, EventChanges, NewEvent, ProposeCreateArguments, ProposeDeleteArguments,
    ProposeInvitationArguments, ProposeResponseArguments, ProposeUpdateArguments, ProposedResponse,
    ResponseTarget,
};
use fabric_desktop_contracts::common::{
    CalendarRef, EventField, EventRef, EventTime, OccurrenceRef, SendUpdates, Window,
};
use fabric_desktop_contracts::scalar::{
    Email, EventDescription, EventLocation, EventTitle, IanaZone, Id, OffsetDateTime, ProviderRef,
    Task, Timestamp,
};
use fabric_google_calendar::calendar::{CalendarTarget, EventsQuery, Google};
use fabric_google_calendar::http::Transport;
use fabric_google_calendar::{
    AccessFailure, AccessTokens, CalendarId, Clock, Endpoints, ManualClock, Secret,
};
use fabric_google_fake::{ASKED_SCOPES, Account, CalendarEntry, Client, FakeGoogle};
use serde_json::Value;

/// The OAuth client the fake knows.
pub const CLIENT: &str = "fabric-test-client.apps.googleusercontent.com";

/// The zone of every calendar unless a test says otherwise. In November it is `+01:00`.
pub const ZONE: &str = "Europe/Berlin";

/// The window most reads use: 2026-11-20, UTC.
pub fn window() -> Window {
    Window {
        start: Timestamp::new("2026-11-20T00:00:00.000Z").expect("valid"),
        end: Timestamp::new("2026-11-21T00:00:00.000Z").expect("valid"),
    }
}

/// A test account.
#[derive(Clone, Debug)]
pub struct Person {
    /// The broker's account id.
    pub id: Id,
    /// The Google subject.
    pub subject: String,
    /// The address, which is also the primary calendar's id.
    pub email: String,
    /// The primary calendar.
    pub primary: CalendarTarget,
}

#[derive(Default)]
struct TokensInner {
    tokens: HashMap<Id, String>,
    failure: Option<AccessFailure>,
    dropped: Vec<Id>,
    calls: usize,
}

/// A test [`AccessTokens`]: hands out the token the fake issued for each account, or a scripted
/// failure. Clones share their state.
#[derive(Clone, Default)]
pub struct TestTokens(Arc<Mutex<TokensInner>>);

impl TestTokens {
    /// Every later call fails with `failure`, or works again with `None`.
    pub fn fail_with(&self, failure: Option<AccessFailure>) {
        self.0.lock().expect("unpoisoned").failure = failure;
    }

    /// The accounts whose cached token was dropped, in order.
    pub fn dropped(&self) -> Vec<Id> {
        self.0.lock().expect("unpoisoned").dropped.clone()
    }

    /// How many tokens were asked for.
    pub fn calls(&self) -> usize {
        self.0.lock().expect("unpoisoned").calls
    }

    /// Replaces the token of an account.
    pub fn set(&self, account: &Id, token: String) {
        self.0
            .lock()
            .expect("unpoisoned")
            .tokens
            .insert(account.clone(), token);
    }

    /// The token of an account.
    pub fn token(&self, account: &Id) -> String {
        self.0.lock().expect("unpoisoned").tokens[account].clone()
    }
}

impl AccessTokens for TestTokens {
    fn access_token(&mut self, account: &Id, _deadline_ms: u64) -> Result<Secret, AccessFailure> {
        let mut inner = self.0.lock().expect("unpoisoned");
        inner.calls += 1;
        if let Some(failure) = inner.failure {
            return Err(failure);
        }
        inner
            .tokens
            .get(account)
            .map(|t| Secret::from(t.as_str()))
            .ok_or(AccessFailure::Transient)
    }

    fn drop_cached(&mut self, account: &Id) {
        self.0
            .lock()
            .expect("unpoisoned")
            .dropped
            .push(account.clone());
    }
}

/// The fake, the adapter and the accounts of one test.
pub struct World {
    /// The fake Google.
    pub fake: FakeGoogle,
    /// The adapter, pointed at the fake.
    pub google: Google,
    /// The transport's clock (the adapter's only clock), at the fake's time.
    pub clock: ManualClock,
    /// The tokens the adapter gets.
    pub tokens: TestTokens,
    /// Which part of the log belongs to which subject.
    segments: Vec<(usize, usize, String)>,
    /// How many broker calendar ids were issued.
    calendars: u32,
    /// Formatted values of the adapter's own types that a test collected (g16). A read's
    /// `ToolResult` is the contract's type and carries content by design; it is not collected.
    pub formatted: Vec<String>,
}

impl World {
    /// A fake with the client and no account, and the adapter pointed at it.
    pub fn new() -> World {
        let fake = FakeGoogle::start();
        fake.add_client(Client::new(CLIENT, "test-client-secret"));
        let clock = ManualClock::new(u64::try_from(fake.now_ms()).expect("after 1970"));
        let transport = Arc::new(
            Transport::new(Endpoints::loopback(fake.port()), Arc::new(clock.clone()))
                .expect("a loopback transport"),
        );
        let tokens = TestTokens::default();
        let google = Google::new(transport, Box::new(tokens.clone()));
        World {
            fake,
            google,
            clock,
            tokens,
            segments: Vec::new(),
            calendars: 0,
            formatted: Vec::new(),
        }
    }

    /// A world with one account, `alice`.
    pub fn with_alice() -> (World, Person) {
        let mut world = World::new();
        let alice = world.person("alice");
        (world, alice)
    }

    /// Adds an account `<name>@example.com` with its primary calendar (owner, [`ZONE`]) and a
    /// valid access token with the five scopes.
    pub fn person(&mut self, name: &str) -> Person {
        let n = self.calendars + 1;
        let subject = format!("1000000000000000000{n:02}");
        let email = format!("{name}@example.com");
        self.fake.add_account(Account::new(&subject, &email));
        let id = Id::new(format!("account-{n:032x}")).expect("valid");
        let token = self
            .fake
            .issue_access_token(&subject, CLIENT, &ASKED_SCOPES, 3600);
        self.tokens.set(&id, token);
        let primary = self.calendar_of(
            &id,
            &subject,
            CalendarEntry::new(&email, "owner")
                .primary()
                .time_zone(ZONE),
        );
        Person {
            id,
            subject,
            email,
            primary,
        }
    }

    /// Adds a calendar entry to an account's list and gives its target, with a new broker id.
    pub fn calendar(&mut self, person: &Person, entry: CalendarEntry) -> CalendarTarget {
        self.calendar_of(&person.id, &person.subject, entry)
    }

    fn calendar_of(&mut self, account: &Id, subject: &str, entry: CalendarEntry) -> CalendarTarget {
        self.calendars += 1;
        let target = CalendarTarget {
            calendar: CalendarRef {
                account_id: account.clone(),
                calendar_id: Id::new(format!("cal-{:032x}", self.calendars)).expect("valid"),
            },
            provider_id: CalendarId::new(&entry.id).expect("a usable id"),
            time_zone: IanaZone::new(entry.time_zone.as_str()).expect("a zone"),
            primary: entry.primary,
        };
        self.fake.add_calendar(subject, entry);
        target
    }

    /// The deadline of a normal call: now + 30 s.
    pub fn deadline(&self) -> u64 {
        self.clock.now_ms() + 30_000
    }

    /// Runs `operation` as `person`'s call and records which requests it made.
    pub fn as_account<R>(
        &mut self,
        person: &Person,
        operation: impl FnOnce(&mut Google) -> R,
    ) -> R {
        let start = self.fake.log().len();
        let result = operation(&mut self.google);
        let end = self.fake.log().len();
        self.segments.push((start, end, person.subject.clone()));
        result
    }

    /// The requests a call made: the log from `start` on.
    pub fn log_since(&self, start: usize) -> Vec<fabric_google_fake::LoggedRequest> {
        self.fake.log().entries()[start..].to_vec()
    }

    /// The end check of every test that used the fake (15, PD22, 6.1): no path holds
    /// `primary`, and every request is a Calendar request made inside an operation of one
    /// account, carrying that account's token.
    pub fn check_log(&self) {
        let log = self.fake.log();
        assert!(!log.any_path_contains("primary"), "a request named primary");
        for entry in log.entries() {
            assert!(
                entry.is_calendar(),
                "request {} is not a Calendar request",
                entry.seq
            );
            let segment = self
                .segments
                .iter()
                .find(|(start, end, _)| (*start..*end).contains(&entry.seq))
                .unwrap_or_else(|| panic!("request {} was made outside an operation", entry.seq));
            assert!(entry.authorization, "request {} had no token", entry.seq);
            assert_eq!(
                entry.token_account.as_deref(),
                Some(segment.2.as_str()),
                "request {} carried another account's token",
                entry.seq
            );
        }
    }
}

/// An event time.
pub fn at(date_time: &str, zone: &str) -> EventTime {
    EventTime {
        date_time: OffsetDateTime::new(date_time).expect("valid"),
        zone: IanaZone::new(zone).expect("valid"),
    }
}

/// A reference to an event and its revision.
pub fn event_ref(id: &str, revision: &str) -> EventRef {
    EventRef {
        event_id: ProviderRef::new(id).expect("valid"),
        revision: ProviderRef::new(revision).expect("valid"),
    }
}

/// An address.
pub fn email(address: &str) -> Email {
    Email::new(address).expect("valid")
}

fn task() -> Task {
    Task::new("a test").expect("valid")
}

/// A reserved id: 26 lower-case base32hex digits.
pub fn reserved(n: u32) -> String {
    format!("fabric{n:020}")
}

/// A create of "Planning" on 2026-11-20, 10:00 to 11:00 in [`ZONE`].
pub fn create(target: &CalendarTarget) -> Action {
    create_with(target, "Planning", Some("Agenda"), Some("Room 1"))
}

/// A create with this title, description and place.
pub fn create_with(
    target: &CalendarTarget,
    title: &str,
    description: Option<&str>,
    location: Option<&str>,
) -> Action {
    Action::ProposeCreate(ProposeCreateArguments {
        account_id: target.calendar.account_id.clone(),
        calendar_id: target.calendar.calendar_id.clone(),
        event: NewEvent {
            title: EventTitle::new(title).expect("valid"),
            start: at("2026-11-20T10:00:00+01:00", ZONE),
            end: at("2026-11-20T11:00:00+01:00", ZONE),
            description: description.map(|d| EventDescription::new(d).expect("valid")),
            location: location.map(|l| EventLocation::new(l).expect("valid")),
        },
        task: task(),
    })
}

/// No changes.
pub fn no_changes() -> EventChanges {
    EventChanges {
        title: None,
        start: None,
        end: None,
        description: None,
        location: None,
    }
}

/// An update.
pub fn update(
    target: &CalendarTarget,
    event: EventRef,
    changes: EventChanges,
    send: SendUpdates,
) -> Action {
    Action::ProposeUpdate(ProposeUpdateArguments {
        account_id: target.calendar.account_id.clone(),
        calendar_id: target.calendar.calendar_id.clone(),
        event,
        changes,
        send_updates: send,
        task: task(),
    })
}

/// An invitation change.
pub fn invitation(
    target: &CalendarTarget,
    event: EventRef,
    addresses: &[&str],
    send: SendUpdates,
) -> Action {
    Action::ProposeInvitation(ProposeInvitationArguments {
        account_id: target.calendar.account_id.clone(),
        calendar_id: target.calendar.calendar_id.clone(),
        event,
        attendees: Attendees::new(addresses.iter().map(|a| email(a)).collect()).expect("valid"),
        send_updates: send,
        task: task(),
    })
}

/// An answer to a single event.
pub fn response(
    target: &CalendarTarget,
    event: EventRef,
    answer: ProposedResponse,
    send: SendUpdates,
) -> Action {
    response_to(target, ResponseTarget::Event(event), answer, send)
}

/// An answer to one occurrence.
pub fn occurrence(instance: &str, original: EventTime, revision: &str) -> ResponseTarget {
    ResponseTarget::Occurrence(OccurrenceRef {
        instance_id: ProviderRef::new(instance).expect("valid"),
        original_start_time: original,
        revision: ProviderRef::new(revision).expect("valid"),
    })
}

/// An answer to a response target.
pub fn response_to(
    target: &CalendarTarget,
    to: ResponseTarget,
    answer: ProposedResponse,
    send: SendUpdates,
) -> Action {
    Action::ProposeResponse(ProposeResponseArguments {
        account_id: target.calendar.account_id.clone(),
        calendar_id: target.calendar.calendar_id.clone(),
        target: to,
        response: answer,
        send_updates: send,
        task: task(),
    })
}

/// A deletion.
pub fn delete(target: &CalendarTarget, event: EventRef, send: SendUpdates) -> Action {
    Action::ProposeDelete(ProposeDeleteArguments {
        account_id: target.calendar.account_id.clone(),
        calendar_id: target.calendar.calendar_id.clone(),
        event,
        send_updates: send,
        task: task(),
    })
}

/// A read query of `targets` over [`window`].
pub fn query(targets: &[CalendarTarget], fields: &[EventField]) -> EventsQuery {
    EventsQuery {
        targets: targets.to_vec(),
        window: window(),
        fields: fields.to_vec(),
        fabric_created: BTreeSet::new(),
    }
}

/// Every field.
pub const ALL_FIELDS: [EventField; 7] = [
    EventField::Time,
    EventField::Title,
    EventField::Location,
    EventField::Description,
    EventField::Attendees,
    EventField::Organizer,
    EventField::OwnResponse,
];

/// A timed event in [`ZONE`] on 2026-11-20 from `start` to `end` (hours), with this id.
pub fn timed(id: &str, title: &str, start: u32, end: u32) -> Value {
    serde_json::json!({
        "id": id,
        "summary": title,
        "start": {"dateTime": format!("2026-11-20T{start:02}:00:00+01:00"), "timeZone": ZONE},
        "end": {"dateTime": format!("2026-11-20T{end:02}:00:00+01:00"), "timeZone": ZONE},
        "eventType": "default",
    })
}

/// Stores an event and gives its id and etag.
pub fn put(world: &World, calendar: &CalendarTarget, event: Value) -> (String, String) {
    let id = world.fake.put_event(calendar.provider_id.as_str(), event);
    let etag = world
        .fake
        .with_state(|s| s.etag(calendar.provider_id.as_str(), &id))
        .expect("an etag");
    (id, etag)
}
