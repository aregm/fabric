//! The fake's state (14.1): clients, accounts, calendars and events, codes and tokens, the
//! signing keys, the clock, and the settings where Google's behaviour is not settled.
//!
//! A test reaches it through [`crate::FakeGoogle::with_state`] (or the shortcuts on
//! `FakeGoogle`), and a [`crate::FaultAction::Before`] hook gets it before a request.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::browser::Decision;
use crate::faults::{ArmedFault, LateTrigger};
use crate::keys::KeyId;
use crate::log::LoggedRequest;
use crate::server::Request;
use crate::time::{DEFAULT_NOW_MS, format_utc_millis};

/// The scope `openid`.
pub const SCOPE_OPENID: &str = "openid";
/// The short address scope, as the adapter asks for it.
pub const SCOPE_EMAIL: &str = "email";
/// The address scope as Google writes it in a token answer.
pub const SCOPE_USERINFO_EMAIL: &str = "https://www.googleapis.com/auth/userinfo.email";
/// The short profile scope.
pub const SCOPE_PROFILE: &str = "profile";
/// The profile scope as Google writes it.
pub const SCOPE_USERINFO_PROFILE: &str = "https://www.googleapis.com/auth/userinfo.profile";
/// Read the calendar list.
pub const SCOPE_CALENDAR_LIST: &str =
    "https://www.googleapis.com/auth/calendar.calendarlist.readonly";
/// Ask for free/busy times.
pub const SCOPE_FREEBUSY: &str = "https://www.googleapis.com/auth/calendar.freebusy";
/// Read and change events.
pub const SCOPE_EVENTS: &str = "https://www.googleapis.com/auth/calendar.events";
/// The five scopes the adapter asks for (3.2.1), in its order.
pub const ASKED_SCOPES: [&str; 5] = [
    SCOPE_OPENID,
    SCOPE_EMAIL,
    SCOPE_CALENDAR_LIST,
    SCOPE_FREEBUSY,
    SCOPE_EVENTS,
];

/// The issuer the fake writes into identity tokens and the callback's `iss`.
pub const ISSUER: &str = "https://accounts.google.com";

/// An OAuth client (3.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Client {
    /// The client id.
    pub id: String,
    /// The client secret.
    pub secret: String,
    /// Whether a token request without `client_secret` is refused (FA4; true by default).
    pub secret_required: bool,
}

impl Client {
    /// A Desktop app client whose secret is required.
    pub fn new(id: &str, secret: &str) -> Client {
        Client {
            id: id.to_string(),
            secret: secret.to_string(),
            secret_required: true,
        }
    }

    /// The same client, but a token request may leave the secret out (G-DOC marks it
    /// optional).
    pub fn secret_optional(mut self) -> Client {
        self.secret_required = false;
        self
    }
}

/// A Google account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    /// The subject (`sub`), the account's identity (PD10).
    pub subject: String,
    /// The address.
    pub email: String,
    /// Whether the address is verified (`email_verified`).
    pub email_verified: bool,
    pub(crate) calendars: Vec<CalendarEntry>,
}

impl Account {
    /// An account with a verified address and no calendars.
    pub fn new(subject: &str, email: &str) -> Account {
        Account {
            subject: subject.to_string(),
            email: email.to_string(),
            email_verified: true,
            calendars: Vec::new(),
        }
    }

    /// The same account with an unverified address.
    pub fn unverified(mut self) -> Account {
        self.email_verified = false;
        self
    }

    /// The account's calendar list, in order.
    pub fn calendars(&self) -> &[CalendarEntry] {
        &self.calendars
    }
}

/// One entry of an account's calendar list (6.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalendarEntry {
    /// Google's calendar id. Any string, so that a test can list an unusable one (g18).
    pub id: String,
    /// `summary`.
    pub summary: Option<String>,
    /// `summaryOverride`.
    pub summary_override: Option<String>,
    /// `accessRole`: `owner`, `writer`, `writerWithoutPrivateAccess`, `reader`,
    /// `freeBusyReader`, or anything else a test wants to see refused.
    pub access_role: String,
    /// `primary`.
    pub primary: bool,
    /// `timeZone`.
    pub time_zone: String,
    /// `hidden`.
    pub hidden: bool,
    /// `deleted`.
    pub deleted: bool,
}

impl CalendarEntry {
    /// An entry with this id and role, its summary the id, zone `UTC`, not primary, not
    /// hidden, not deleted.
    pub fn new(id: &str, access_role: &str) -> CalendarEntry {
        CalendarEntry {
            id: id.to_string(),
            summary: Some(id.to_string()),
            summary_override: None,
            access_role: access_role.to_string(),
            primary: false,
            time_zone: "UTC".to_string(),
            hidden: false,
            deleted: false,
        }
    }

    /// Sets `summary` (`None` leaves it out).
    pub fn summary(mut self, summary: Option<&str>) -> CalendarEntry {
        self.summary = summary.map(str::to_string);
        self
    }

    /// Sets `summaryOverride`.
    pub fn summary_override(mut self, name: &str) -> CalendarEntry {
        self.summary_override = Some(name.to_string());
        self
    }

    /// Marks the entry as the account's primary calendar.
    pub fn primary(mut self) -> CalendarEntry {
        self.primary = true;
        self
    }

    /// Sets `timeZone`.
    pub fn time_zone(mut self, zone: &str) -> CalendarEntry {
        self.time_zone = zone.to_string();
        self
    }

    /// Marks the entry hidden.
    pub fn hidden(mut self) -> CalendarEntry {
        self.hidden = true;
        self
    }

    /// Marks the entry deleted.
    pub fn deleted(mut self) -> CalendarEntry {
        self.deleted = true;
        self
    }
}

/// What a GET of a deleted (cancelled) event answers (FA3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeletedEventGet {
    /// 200 with the event and `status: cancelled` (the default).
    #[default]
    Cancelled,
    /// 404 `notFound`.
    NotFound,
    /// 410 `deleted`.
    Gone,
}

/// How the token answer spells the granted scopes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScopeSpelling {
    /// As Google is reported to: `email` and `profile` as their `userinfo` addresses (the
    /// default).
    #[default]
    Google,
    /// Exactly as the authorization request asked.
    AsAsked,
}

pub(crate) struct Calendar {
    pub(crate) time_zone: String,
    pub(crate) events: Vec<StoredEvent>,
}

pub(crate) struct StoredEvent {
    pub(crate) json: Map<String, Value>,
    /// GETs that still answer 404 (a late-visible insert).
    pub(crate) hidden_reads: u32,
}

impl StoredEvent {
    pub(crate) fn id(&self) -> &str {
        self.json.get("id").and_then(Value::as_str).unwrap_or("")
    }

    pub(crate) fn cancelled(&self) -> bool {
        self.json.get("status").and_then(Value::as_str) == Some("cancelled")
    }
}

pub(crate) struct Code {
    pub(crate) code: String,
    pub(crate) client_id: String,
    pub(crate) redirect_uri: String,
    pub(crate) challenge: String,
    pub(crate) scopes: Vec<String>,
    pub(crate) nonce: Option<String>,
    pub(crate) subject: String,
    pub(crate) ends_ms: i64,
    pub(crate) used: bool,
}

pub(crate) struct RefreshToken {
    pub(crate) token: String,
    pub(crate) subject: String,
    pub(crate) client_id: String,
    pub(crate) scopes: Vec<String>,
    pub(crate) ends_ms: Option<i64>,
    pub(crate) revoked: bool,
}

pub(crate) struct AccessToken {
    pub(crate) token: String,
    pub(crate) subject: String,
    pub(crate) client_id: String,
    pub(crate) scopes: Vec<String>,
    pub(crate) ends_ms: i64,
    pub(crate) revoked: bool,
}

pub(crate) struct LateChange {
    pub(crate) request: Request,
    pub(crate) target_path: String,
    pub(crate) trigger: LateTrigger,
    pub(crate) reads: u32,
}

/// The whole state of one fake. Every method works on the fake's own clock, never the
/// system's.
pub struct State {
    pub(crate) port: u16,
    pub(crate) now_ms: i64,
    pub(crate) clients: Vec<Client>,
    pub(crate) accounts: Vec<Account>,
    pub(crate) calendars: BTreeMap<String, Calendar>,
    pub(crate) codes: Vec<Code>,
    pub(crate) refresh_tokens: Vec<RefreshToken>,
    pub(crate) access_tokens: Vec<AccessToken>,
    pub(crate) access_token_lifetime_s: i64,
    pub(crate) refresh_token_lifetime_s: Option<i64>,
    pub(crate) published_keys: Vec<(KeyId, u32)>,
    pub(crate) key_set_max_age_s: u64,
    pub(crate) key_set_fetches: u32,
    pub(crate) signing_key: KeyId,
    pub(crate) browser_account: Option<String>,
    pub(crate) pending_decision: Option<Decision>,
    pub(crate) deleted_event_get: DeletedEventGet,
    pub(crate) free_busy_merge: bool,
    pub(crate) scope_spelling: ScopeSpelling,
    pub(crate) revision: u64,
    pub(crate) counter: u64,
    pub(crate) late: Vec<LateChange>,
    pub(crate) faults: Vec<ArmedFault>,
    pub(crate) log: Vec<LoggedRequest>,
    pub(crate) handler_panics: u32,
}

impl State {
    pub(crate) fn new(port: u16) -> State {
        State {
            port,
            now_ms: DEFAULT_NOW_MS,
            clients: Vec::new(),
            accounts: Vec::new(),
            calendars: BTreeMap::new(),
            codes: Vec::new(),
            refresh_tokens: Vec::new(),
            access_tokens: Vec::new(),
            access_token_lifetime_s: 3599,
            refresh_token_lifetime_s: None,
            published_keys: vec![(KeyId::A, 0), (KeyId::B, 0)],
            key_set_max_age_s: 21_600,
            key_set_fetches: 0,
            signing_key: KeyId::A,
            browser_account: None,
            pending_decision: None,
            deleted_event_get: DeletedEventGet::default(),
            free_busy_merge: true,
            scope_spelling: ScopeSpelling::default(),
            revision: 3_400_000_000_000_000,
            counter: 0,
            late: Vec::new(),
            faults: Vec::new(),
            log: Vec::new(),
            handler_panics: 0,
        }
    }

    // ---- The clock ---------------------------------------------------------------------------

    /// The fake's clock, in Unix milliseconds. It starts at 2026-11-17T09:00:00Z and moves only
    /// when a test moves it.
    pub fn now_ms(&self) -> i64 {
        self.now_ms
    }

    /// Sets the clock.
    pub fn set_now_ms(&mut self, now_ms: i64) {
        self.now_ms = now_ms;
    }

    /// Moves the clock forward.
    pub fn advance(&mut self, by: Duration) {
        self.now_ms += i64::try_from(by.as_millis()).unwrap_or(i64::MAX / 2);
    }

    // ---- Clients and accounts ----------------------------------------------------------------

    /// Adds a client, or replaces the one with the same id.
    pub fn add_client(&mut self, client: Client) {
        self.clients.retain(|c| c.id != client.id);
        self.clients.push(client);
    }

    /// Adds an account, or replaces the one with the same subject (keeping its calendars when
    /// the new one has none). The first account added is the one the browser signs in as
    /// unless a test says otherwise.
    pub fn add_account(&mut self, mut account: Account) {
        if let Some(i) = self
            .accounts
            .iter()
            .position(|a| a.subject == account.subject)
        {
            let old = self.accounts.remove(i);
            if account.calendars.is_empty() {
                account.calendars = old.calendars;
            }
            self.accounts.insert(i, account);
        } else {
            self.accounts.push(account);
        }
    }

    /// The account with this subject.
    pub fn account(&self, subject: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.subject == subject)
    }

    pub(crate) fn account_mut(&mut self, subject: &str) -> Option<&mut Account> {
        self.accounts.iter_mut().find(|a| a.subject == subject)
    }

    /// Sets whether an account's address is verified.
    pub fn set_email_verified(&mut self, subject: &str, verified: bool) {
        if let Some(a) = self.account_mut(subject) {
            a.email_verified = verified;
        }
    }

    /// Changes an account's address (G-DOC: an address "could change").
    pub fn set_email(&mut self, subject: &str, email: &str) {
        if let Some(a) = self.account_mut(subject) {
            a.email = email.to_string();
        }
    }

    /// The account the browser is signed in to when a decision names none and the request
    /// has no `login_hint` that names a known account.
    pub fn set_browser_account(&mut self, subject: &str) {
        self.browser_account = Some(subject.to_string());
    }

    // ---- Calendars ---------------------------------------------------------------------------

    /// Adds an entry to an account's calendar list, or replaces the entry with the same id.
    /// The calendar itself is created when it does not exist yet, with the entry's zone; two accounts that list the same id share the calendar and its events.
    pub fn add_calendar(&mut self, subject: &str, entry: CalendarEntry) {
        self.calendars
            .entry(entry.id.clone())
            .or_insert_with(|| Calendar {
                time_zone: entry.time_zone.clone(),
                events: Vec::new(),
            });
        if let Some(account) = self.account_mut(subject) {
            if let Some(existing) = account.calendars.iter_mut().find(|c| c.id == entry.id) {
                *existing = entry;
            } else {
                account.calendars.push(entry);
            }
        }
    }

    /// The entry of an account's calendar list.
    pub fn calendar_entry(&self, subject: &str, calendar: &str) -> Option<&CalendarEntry> {
        self.account(subject)?
            .calendars
            .iter()
            .find(|c| c.id == calendar)
    }

    /// The entry, to change it in place.
    pub fn calendar_entry_mut(
        &mut self,
        subject: &str,
        calendar: &str,
    ) -> Option<&mut CalendarEntry> {
        self.account_mut(subject)?
            .calendars
            .iter_mut()
            .find(|c| c.id == calendar)
    }

    /// Changes an account's role on a calendar (a role lowered, 14.2). False when the account
    /// does not list it.
    pub fn set_access_role(&mut self, subject: &str, calendar: &str, role: &str) -> bool {
        match self.calendar_entry_mut(subject, calendar) {
            Some(e) => {
                e.access_role = role.to_string();
                true
            }
            None => false,
        }
    }

    /// Removes a calendar from an account's list (a vanished calendar, 14.2). Its events stay
    /// for other accounts that list it.
    pub fn remove_calendar(&mut self, subject: &str, calendar: &str) -> bool {
        match self.account_mut(subject) {
            Some(a) => {
                let before = a.calendars.len();
                a.calendars.retain(|c| c.id != calendar);
                a.calendars.len() != before
            }
            None => false,
        }
    }

    /// Sets the zone of a calendar (the `timeZone` of an events list answer).
    pub fn set_calendar_time_zone(&mut self, calendar: &str, zone: &str) {
        if let Some(c) = self.calendars.get_mut(calendar) {
            c.time_zone = zone.to_string();
        }
    }

    // ---- Events ------------------------------------------------------------------------------

    /// Stores an event as it is (a JSON object), as if made in Google's own page, and gives
    /// its id. The fake adds what is missing: an `id` (26 base32hex digits), `kind`, `status`
    /// `confirmed`, `created` and `updated`, and an `etag` (a quoted string). An `etag` the
    /// object already has is kept, so that a test can store an unusual one (FA11). An event
    /// with the same id in that calendar is replaced. The calendar is created when it does not
    /// exist. Occurrences of a recurring event are stored as events of their own, with
    /// `recurringEventId` and `originalStartTime` (no recurrence engine).
    ///
    /// Panics when `event` is not a JSON object.
    pub fn put_event(&mut self, calendar: &str, event: Value) -> String {
        let Value::Object(mut json) = event else {
            panic!("put_event takes a JSON object");
        };
        let id = match json.get("id").and_then(Value::as_str) {
            Some(id) => id.to_string(),
            None => {
                let id = self.new_event_id();
                json.insert("id".into(), Value::String(id.clone()));
                id
            }
        };
        let now = format_utc_millis(self.now_ms);
        json.entry("kind")
            .or_insert_with(|| json!("calendar#event"));
        json.entry("status").or_insert_with(|| json!("confirmed"));
        json.entry("created").or_insert_with(|| json!(now));
        json.entry("updated").or_insert_with(|| json!(now));
        if !json.contains_key("etag") {
            let etag = self.next_etag();
            json.insert("etag".into(), Value::String(etag));
        }
        let cal = self.calendar_or_create(calendar);
        cal.events.retain(|e| e.id() != id);
        cal.events.push(StoredEvent {
            json,
            hidden_reads: 0,
        });
        id
    }

    /// The event as stored (cancelled ones included), with its etag.
    pub fn event(&self, calendar: &str, id: &str) -> Option<Value> {
        self.stored(calendar, id)
            .map(|e| Value::Object(e.json.clone()))
    }

    /// The `etag` of an event.
    pub fn etag(&self, calendar: &str, id: &str) -> Option<String> {
        self.stored(calendar, id)?
            .json
            .get("etag")
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    /// Every event of a calendar as stored, cancelled ones included, in the order stored.
    pub fn events(&self, calendar: &str) -> Vec<Value> {
        self.calendars
            .get(calendar)
            .map(|c| {
                c.events
                    .iter()
                    .map(|e| Value::Object(e.json.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Changes an event as someone else would: `patch` is merged as a JSON merge patch (null
    /// removes a member, arrays replace), and the event gets a new etag and `updated`. Gives
    /// the new etag, or `None` when there is no such event.
    pub fn change_event(&mut self, calendar: &str, id: &str, patch: Value) -> Option<String> {
        let etag = self.next_etag();
        let now = format_utc_millis(self.now_ms);
        let event = self.stored_mut(calendar, id)?;
        let mut target = Value::Object(std::mem::take(&mut event.json));
        merge_patch(&mut target, &patch);
        let Value::Object(mut json) = target else {
            unreachable!("a merge into an object stays an object");
        };
        json.insert("etag".into(), Value::String(etag.clone()));
        json.insert("updated".into(), Value::String(now));
        event.json = json;
        Some(etag)
    }

    /// Deletes an event as someone else would: `status` becomes `cancelled`, with a new etag.
    pub fn cancel_event(&mut self, calendar: &str, id: &str) -> bool {
        self.change_event(calendar, id, json!({"status": "cancelled"}))
            .is_some()
    }

    /// Removes an event entirely: a GET of it answers 404 from now on.
    pub fn remove_event(&mut self, calendar: &str, id: &str) -> bool {
        match self.calendars.get_mut(calendar) {
            Some(c) => {
                let before = c.events.len();
                c.events.retain(|e| e.id() != id);
                c.events.len() != before
            }
            None => false,
        }
    }

    /// The next `reads` GETs of this event answer 404 and lists leave it out (a late-visible
    /// insert, PD20).
    pub fn hide_event_for_reads(&mut self, calendar: &str, id: &str, reads: u32) {
        if let Some(e) = self.stored_mut(calendar, id) {
            e.hidden_reads = reads;
        }
    }

    /// What a GET of a deleted event answers (FA3).
    pub fn set_deleted_event_get(&mut self, answer: DeletedEventGet) {
        self.deleted_event_get = answer;
    }

    /// Whether free/busy merges overlapping busy periods of one calendar (true by default).
    /// Periods are never cut to the window by the fake: that is the adapter's job (6.3).
    pub fn set_free_busy_merge(&mut self, merge: bool) {
        self.free_busy_merge = merge;
    }

    pub(crate) fn stored(&self, calendar: &str, id: &str) -> Option<&StoredEvent> {
        self.calendars
            .get(calendar)?
            .events
            .iter()
            .find(|e| e.id() == id)
    }

    pub(crate) fn stored_mut(&mut self, calendar: &str, id: &str) -> Option<&mut StoredEvent> {
        self.calendars
            .get_mut(calendar)?
            .events
            .iter_mut()
            .find(|e| e.id() == id)
    }

    pub(crate) fn calendar_or_create(&mut self, calendar: &str) -> &mut Calendar {
        self.calendars
            .entry(calendar.to_string())
            .or_insert_with(|| Calendar {
                time_zone: "UTC".to_string(),
                events: Vec::new(),
            })
    }

    /// A new etag: a quoted decimal number that rises with every change (FA11, FA16).
    pub(crate) fn next_etag(&mut self) -> String {
        self.revision += 1;
        format!("\"{}\"", self.revision)
    }

    pub(crate) fn new_event_id(&mut self) -> String {
        const DIGITS: &[u8; 32] = b"0123456789abcdefghijklmnopqrstuv";
        let hash = self.next_secret("event-id");
        hash.bytes()
            .take(26)
            .map(|b| char::from(DIGITS[usize::from(b) % 32]))
            .collect()
    }

    /// A unique string of 64 hex digits, different for every call and every port. Tokens and
    /// codes are made from it; it has no randomness, so runs repeat.
    pub(crate) fn next_secret(&mut self, purpose: &str) -> String {
        self.counter += 1;
        let digest = Sha256::digest(format!("{purpose}:{}:{}", self.port, self.counter));
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    // ---- Codes and tokens --------------------------------------------------------------------

    /// Issues a refresh token for an account and client with these scopes, as a finished
    /// sign-in would, so that a Calendar test needs no sign-in.
    pub fn issue_refresh_token(
        &mut self,
        subject: &str,
        client_id: &str,
        scopes: &[&str],
    ) -> String {
        let token = format!("1//fake-refresh-{}", self.next_secret("refresh"));
        let ends_ms = self
            .refresh_token_lifetime_s
            .map(|s| self.now_ms + s * 1000);
        self.refresh_tokens.push(RefreshToken {
            token: token.clone(),
            subject: subject.to_string(),
            client_id: client_id.to_string(),
            scopes: scopes.iter().map(|s| s.to_string()).collect(),
            ends_ms,
            revoked: false,
        });
        token
    }

    /// Issues an access token that ends `lifetime_s` seconds from now on the fake clock.
    pub fn issue_access_token(
        &mut self,
        subject: &str,
        client_id: &str,
        scopes: &[&str],
        lifetime_s: i64,
    ) -> String {
        let token = format!("ya29.fake-access-{}", self.next_secret("access"));
        self.access_tokens.push(AccessToken {
            token: token.clone(),
            subject: subject.to_string(),
            client_id: client_id.to_string(),
            scopes: scopes.iter().map(|s| s.to_string()).collect(),
            ends_ms: self.now_ms + lifetime_s * 1000,
            revoked: false,
        });
        token
    }

    /// The lifetime of the access tokens the token endpoint issues (`expires_in`; 3599 s by
    /// default).
    pub fn set_access_token_lifetime(&mut self, seconds: i64) {
        self.access_token_lifetime_s = seconds;
    }

    /// Time-based access: refresh tokens issued from now on end after this many seconds, and
    /// the token answer carries `refresh_token_expires_in`. `None` (the default): they do not
    /// end by time.
    pub fn set_refresh_token_lifetime(&mut self, seconds: Option<i64>) {
        self.refresh_token_lifetime_s = seconds;
    }

    /// Every access token of this account ends now.
    pub fn expire_access_tokens(&mut self, subject: &str) {
        let now = self.now_ms;
        for t in self
            .access_tokens
            .iter_mut()
            .filter(|t| t.subject == subject)
        {
            t.ends_ms = t.ends_ms.min(now);
        }
    }

    /// Ends the whole grant of an account to a client: every refresh and access token of the
    /// pair stops working (FA14; what removing access in the Google account does).
    pub fn revoke_grant(&mut self, subject: &str, client_id: &str) {
        for t in self
            .refresh_tokens
            .iter_mut()
            .filter(|t| t.subject == subject && t.client_id == client_id)
        {
            t.revoked = true;
        }
        for t in self
            .access_tokens
            .iter_mut()
            .filter(|t| t.subject == subject && t.client_id == client_id)
        {
            t.revoked = true;
        }
    }

    /// Whether a refresh token still works.
    pub fn refresh_token_valid(&self, token: &str) -> bool {
        self.refresh_tokens
            .iter()
            .any(|t| t.token == token && self.refresh_usable(t))
    }

    /// Whether an access token still works.
    pub fn access_token_valid(&self, token: &str) -> bool {
        self.access_tokens
            .iter()
            .any(|t| t.token == token && !t.revoked && t.ends_ms > self.now_ms)
    }

    /// The refresh tokens of an account that still work, oldest first.
    pub fn valid_refresh_tokens(&self, subject: &str) -> Vec<String> {
        self.refresh_tokens
            .iter()
            .filter(|t| t.subject == subject && self.refresh_usable(t))
            .map(|t| t.token.clone())
            .collect()
    }

    /// The access tokens of an account that still work, oldest first.
    pub fn valid_access_tokens(&self, subject: &str) -> Vec<String> {
        self.access_tokens
            .iter()
            .filter(|t| t.subject == subject && !t.revoked && t.ends_ms > self.now_ms)
            .map(|t| t.token.clone())
            .collect()
    }

    /// How many authorization codes were issued.
    pub fn codes_issued(&self) -> usize {
        self.codes.len()
    }

    /// Whether a code was presented at the token endpoint; `None` for an unknown code.
    pub fn code_used(&self, code: &str) -> Option<bool> {
        self.codes.iter().find(|c| c.code == code).map(|c| c.used)
    }

    /// How the token answer spells the granted scopes.
    pub fn set_scope_spelling(&mut self, spelling: ScopeSpelling) {
        self.scope_spelling = spelling;
    }

    pub(crate) fn refresh_usable(&self, t: &RefreshToken) -> bool {
        !t.revoked && t.ends_ms.is_none_or(|end| end > self.now_ms)
    }

    pub(crate) fn subject_of_token(&self, token: &str) -> Option<String> {
        self.access_tokens
            .iter()
            .find(|t| t.token == token)
            .map(|t| t.subject.clone())
            .or_else(|| {
                self.refresh_tokens
                    .iter()
                    .find(|t| t.token == token)
                    .map(|t| t.subject.clone())
            })
            .or_else(|| {
                self.codes
                    .iter()
                    .find(|c| c.code == token)
                    .map(|c| c.subject.clone())
            })
    }

    // ---- Keys --------------------------------------------------------------------------------

    /// The `max-age` of the key set's `Cache-Control`, in seconds (21 600 by default).
    pub fn set_key_set_max_age(&mut self, seconds: u64) {
        self.key_set_max_age_s = seconds;
    }

    /// Lists a key in the key set from the key set fetch with this number on (0: from the
    /// next fetch). With `publish_key(KeyId::B, 1)` after `unpublish_key(KeyId::B)`, the first
    /// fetch lacks B and every later one has it (g11).
    pub fn publish_key(&mut self, key: KeyId, from_fetch: u32) {
        self.published_keys.retain(|(k, _)| *k != key);
        self.published_keys
            .push((key, self.key_set_fetches + from_fetch));
    }

    /// Removes a key from the key set.
    pub fn unpublish_key(&mut self, key: KeyId) {
        self.published_keys.retain(|(k, _)| *k != key);
    }

    /// The key that signs identity tokens (A by default).
    pub fn set_signing_key(&mut self, key: KeyId) {
        self.signing_key = key;
    }

    /// How many times the key set was fetched.
    pub fn key_set_fetches(&self) -> u32 {
        self.key_set_fetches
    }

    // ---- Diagnostics -------------------------------------------------------------------------

    /// How many request handlers panicked (a bug in the fake; their connections were closed
    /// without an answer).
    pub fn handler_panics(&self) -> u32 {
        self.handler_panics
    }

    /// How many late changes wait for their trigger.
    pub fn late_changes_pending(&self) -> usize {
        self.late.len()
    }

    pub(crate) fn client(&self, id: &str) -> Option<&Client> {
        self.clients.iter().find(|c| c.id == id)
    }
}

/// A JSON merge patch (RFC 7396): members of an object patch are merged recursively, `null`
/// removes a member, anything else (arrays among them) replaces.
pub(crate) fn merge_patch(target: &mut Value, patch: &Value) {
    let Value::Object(patch) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = Value::Object(Map::new());
    }
    let Value::Object(target) = target else {
        return;
    };
    for (name, value) in patch {
        if value.is_null() {
            target.remove(name);
        } else if value.is_object() {
            let slot = target.entry(name.clone()).or_insert(Value::Null);
            merge_patch(slot, value);
        } else {
            target.insert(name.clone(), value.clone());
        }
    }
}
