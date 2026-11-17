//! A fake Google for the tests of the Google Calendar adapter (work package CD-W05).
//!
//! [`FakeGoogle::start`] binds `127.0.0.1:0` and serves, on one port, what the adapter talks
//! to (14.1): the authorization endpoint (`GET /o/oauth2/v2/auth`), the token endpoint
//! (`POST /token`), revocation (`POST /revoke`), the key set (`GET /oauth2/v3/certs`) and the
//! Calendar API under `/calendar/v3` (calendar list, free/busy, events list, get, insert, patch
//! and delete). The adapter's loopback endpoint set is `http://127.0.0.1:<port>` with those
//! paths (5.1); [`FakeGoogle::port`] and the address methods give them.
//!
//! **It is a test double written from Google's documentation (G-DOC, appendix A of the design)
//! and from guesses where the documentation is silent. It is never evidence of how Google
//! behaves.** Every test that passes against it shows only that the adapter does what the
//! design says against what the fake assumes. The assumptions the adapter depends on are
//! FA1 to FA17 below; each has a live check (LV1 to LV14) that only the owner's run against
//! the real Google can make. It is only ever a dev-dependency (2.1).
//!
//! - [`state`]: clients, accounts, calendars, events, codes, tokens, keys and the clock, all
//!   set by the test. The clock is the fake's own; nothing reads the system clock.
//! - [`browser`]: the person at the browser, who decides and follows the redirect to the
//!   adapter's loopback listener.
//! - [`faults`]: faults armed for the next matching request (14.2).
//! - [`log`]: every request received, in order (14.1).
//! - [`client`]: a small loopback HTTP client for sending any request to the listener.
//! - [`keys`]: the fixed RSA test keys, written as numbers.
//!
//! The server (`std::net` and threads, no async runtime and no HTTP library) reads one request
//! per connection with a `Content-Length` body, answers with `Connection: close`, and bounds
//! the head (16 KiB), the body (1 MiB) and the time to send them (10 s each). A handler that
//! panics closes its connection without an answer and is counted
//! ([`State::handler_panics`]). Dropping the `FakeGoogle` stops the server, ends every
//! delayed answer, joins every thread and frees the port.
//!
//! ```
//! use fabric_google_fake::{
//!     Account, CalendarEntry, Client, FakeGoogle, Fault, FaultAction, SCOPE_EVENTS,
//! };
//! use serde_json::json;
//!
//! let fake = FakeGoogle::start();
//! fake.add_client(Client::new("client-id", "client-secret"));
//! fake.add_account(Account::new("sub-1", "alice@example.com"));
//! fake.add_calendar("sub-1", CalendarEntry::new("alice@example.com", "owner").primary());
//! fake.put_event(
//!     "alice@example.com",
//!     json!({"id": "evt1", "start": {"dateTime": "2026-11-18T10:00:00Z"},
//!            "end": {"dateTime": "2026-11-18T11:00:00Z"}}),
//! );
//! // A Calendar test needs no sign-in: it can hand the adapter a refresh token.
//! let refresh = fake.issue_refresh_token("sub-1", "client-id", &[SCOPE_EVENTS]);
//! // The next PATCH is applied, then answered 503 (14.2).
//! let fault = fake.arm(Fault::on("PATCH", "/calendar/v3/", FaultAction::apply_then_503()));
//! // ... the adapter runs against fake.base_url() ...
//! assert_eq!(fault.fired(), 0);
//! assert!(fake.log().calendar_changes().is_empty());
//! # let _ = refresh;
//! ```
//!
//! # What the fake assumes about Google (14.3)
//!
//! - **FA1** PATCH and DELETE honour `If-Match` with one revision and answer 412 on another;
//!   `*` matches any revision; a weak tag never matches.
//! - **FA2** An insert with an id that exists in the calendar (cancelled events included)
//!   answers 409 `duplicate`.
//! - **FA3** A GET of a deleted single event answers 200 with `status: cancelled` (the
//!   default), 404, or 410, as the test sets ([`DeletedEventGet`]).
//! - **FA4** The token endpoint accepts a Desktop app client with its secret and PKCE; whether
//!   the secret is required is per client ([`Client::secret_optional`]).
//! - **FA5** The consent screen lets a person untick single scopes, and the token then lists
//!   fewer ([`Decision::untick`]).
//! - **FA6** An attendee may PATCH the whole attendee list with only the own answer changed:
//!   the fake applies any attendee list a writer sends.
//! - **FA7** The event returned after insert or PATCH holds the members as written (the fake
//!   adds `self` on the attendee whose address is the calendar id, `responseStatus`
//!   `needsAction` where missing, and on insert `creator`, `organizer`, `eventType`,
//!   `iCalUID`, `sequence`).
//! - **FA8** `events.list` with `singleEvents=true` returns occurrences with `recurringEventId`
//!   and `originalStartTime`, never their master. The fake has no recurrence engine: a test
//!   stores occurrences as events. It writes every `dateTime` as stored and does no zone
//!   arithmetic, so a test stores them with the offset of the zone the adapter names.
//! - **FA9** A refresh token that ended answers `invalid_grant`.
//! - **FA10** Rate limits arrive as 403 or 429 with `rateLimitExceeded` or
//!   `userRateLimitExceeded` (armed as faults).
//! - **FA11** The etag of an event is a quoted string (here a quoted decimal number).
//! - **FA12** `freeBusy` reports a calendar it cannot read under `errors` (reason `notFound`)
//!   with an empty `busy`, not as an empty list alone.
//! - **FA13** The identity token of a code exchange carries the `nonce` of the authorization
//!   request.
//! - **FA14** A revocation ends the whole grant of that account to the client: every refresh
//!   and access token of the pair.
//! - **FA15** On the account's primary calendar the entry marked `self` has the calendar's id
//!   as its address (what the fake writes for attendees it adds; stored events are as the
//!   test wrote them).
//! - **FA16** A change the fake applies always changes the event's etag, a PATCH that changes
//!   nothing included.
//! - **FA17** Google's own event ids match `^[A-Za-z0-9_]+$` (the fake's generated ids are 26
//!   base32hex digits).
//!
//! Numbers in this crate's comments, such as (14.2) or (6.7), and ids such as PD14, FA3 or
//! g07, cite `docs/plans/calendar-desktop-w05-design.md`.

#![warn(missing_docs)]

pub mod browser;
pub mod client;
pub mod faults;
pub mod keys;
pub mod log;
pub mod state;
pub mod time;
pub mod url;

mod calendar;
mod gzip;
mod oauth;
mod server;

use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener};
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::Value;

pub use browser::{Authorization, Browser, BrowserError, Decision, Tamper, Visit};
pub use client::{ClientError, HttpResponse, RawRequest};
pub use faults::{
    Fault, FaultAction, FaultHandle, Hook, IdTokenFault, LateTrigger, Match, TokenFault,
};
pub use keys::KeyId;
pub use log::{Log, LoggedRequest};
pub use state::{
    ASKED_SCOPES, Account, CalendarEntry, Client, DeletedEventGet, ISSUER, SCOPE_CALENDAR_LIST,
    SCOPE_EMAIL, SCOPE_EVENTS, SCOPE_FREEBUSY, SCOPE_OPENID, SCOPE_PROFILE, SCOPE_USERINFO_EMAIL,
    SCOPE_USERINFO_PROFILE, ScopeSpelling, State,
};

use faults::ArmedFault;
use server::Shared;

/// The path of the authorization endpoint.
pub const AUTHORIZATION_PATH: &str = "/o/oauth2/v2/auth";
/// The path of the token endpoint.
pub const TOKEN_PATH: &str = "/token";
/// The path of the revocation endpoint.
pub const REVOCATION_PATH: &str = "/revoke";
/// The path of the key set.
pub const KEYS_PATH: &str = "/oauth2/v3/certs";
/// The path of the Calendar API.
pub const CALENDAR_PATH: &str = "/calendar/v3";

/// A running fake Google on `127.0.0.1`. Dropping it stops the server and frees its port.
pub struct FakeGoogle {
    shared: Arc<Shared>,
    addr: SocketAddr,
    accept: Option<JoinHandle<()>>,
}

impl FakeGoogle {
    /// Starts a fake with no clients, no accounts and no calendars, on a port the system
    /// picks. Panics when it cannot bind `127.0.0.1:0` (a test cannot go on without it).
    pub fn start() -> FakeGoogle {
        FakeGoogle::try_start().expect("bind 127.0.0.1:0 for the fake Google")
    }

    /// Starts a fake, or gives the error of binding the port.
    pub fn try_start() -> io::Result<FakeGoogle> {
        let listener =
            TcpListener::bind(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)))?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(Shared::new(State::new(addr.port())));
        let accept = server::spawn(listener, Arc::clone(&shared))?;
        Ok(FakeGoogle {
            shared,
            addr,
            accept: Some(accept),
        })
    }

    /// The port the fake listens on.
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// The socket address (`127.0.0.1:<port>`).
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// `http://127.0.0.1:<port>`: the base of every endpoint (5.1).
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port())
    }

    /// The authorization endpoint's address.
    pub fn authorization_url(&self) -> String {
        format!("{}{AUTHORIZATION_PATH}", self.base_url())
    }

    /// The token endpoint's address.
    pub fn token_url(&self) -> String {
        format!("{}{TOKEN_PATH}", self.base_url())
    }

    /// The revocation endpoint's address.
    pub fn revocation_url(&self) -> String {
        format!("{}{REVOCATION_PATH}", self.base_url())
    }

    /// The key set's address.
    pub fn keys_url(&self) -> String {
        format!("{}{KEYS_PATH}", self.base_url())
    }

    /// The Calendar API's base address (`.../calendar/v3`).
    pub fn calendar_url(&self) -> String {
        format!("{}{CALENDAR_PATH}", self.base_url())
    }

    /// Runs `change` on the state. The server waits meanwhile, so `change` must not make a
    /// request to the fake.
    pub fn with_state<R>(&self, change: impl FnOnce(&mut State) -> R) -> R {
        change(&mut self.shared.lock())
    }

    /// The browser that plays the person (14.1).
    pub fn browser(&self) -> Browser<'_> {
        Browser { fake: self }
    }

    /// Arms a fault (14.2).
    pub fn arm(&self, fault: Fault) -> FaultHandle {
        let fired = Arc::new(AtomicU32::new(0));
        let handle = FaultHandle {
            fired: Arc::clone(&fired),
            times: fault.times,
        };
        self.with_state(|s| s.faults.push(ArmedFault { fault, fired }));
        handle
    }

    /// Removes every armed fault that has not fired yet.
    pub fn disarm_all(&self) {
        self.with_state(|s| s.faults.clear());
    }

    /// A copy of the request log.
    pub fn log(&self) -> Log {
        self.with_state(|s| Log::new(s.log.clone()))
    }

    /// Empties the request log.
    pub fn clear_log(&self) {
        self.with_state(|s| s.log.clear());
    }

    /// How many connections the server accepted, idle ones included.
    pub fn connections(&self) -> u64 {
        self.shared.connections()
    }

    /// Applies every late change now ([`FaultAction::ApplyLate`]), whatever its trigger, and
    /// gives how many there were.
    pub fn apply_late_changes(&self) -> usize {
        self.with_state(server::apply_all_late_changes)
    }

    /// How many request handlers panicked.
    pub fn handler_panics(&self) -> u32 {
        self.with_state(|s| s.handler_panics())
    }

    // ---- Shortcuts for the most common state changes -----------------------------------------

    /// Adds a client ([`State::add_client`]).
    pub fn add_client(&self, client: Client) {
        self.with_state(|s| s.add_client(client));
    }

    /// Adds an account ([`State::add_account`]).
    pub fn add_account(&self, account: Account) {
        self.with_state(|s| s.add_account(account));
    }

    /// Adds a calendar to an account's list ([`State::add_calendar`]).
    pub fn add_calendar(&self, subject: &str, entry: CalendarEntry) {
        self.with_state(|s| s.add_calendar(subject, entry));
    }

    /// Stores an event and gives its id ([`State::put_event`]).
    pub fn put_event(&self, calendar: &str, event: Value) -> String {
        self.with_state(|s| s.put_event(calendar, event))
    }

    /// The event as stored ([`State::event`]).
    pub fn event(&self, calendar: &str, id: &str) -> Option<Value> {
        self.with_state(|s| s.event(calendar, id))
    }

    /// Every event of a calendar as stored ([`State::events`]).
    pub fn events(&self, calendar: &str) -> Vec<Value> {
        self.with_state(|s| s.events(calendar))
    }

    /// Changes an event as someone else would ([`State::change_event`]).
    pub fn change_event(&self, calendar: &str, id: &str, patch: Value) -> Option<String> {
        self.with_state(|s| s.change_event(calendar, id, patch))
    }

    /// Issues a refresh token ([`State::issue_refresh_token`]).
    pub fn issue_refresh_token(&self, subject: &str, client_id: &str, scopes: &[&str]) -> String {
        self.with_state(|s| s.issue_refresh_token(subject, client_id, scopes))
    }

    /// Issues an access token ([`State::issue_access_token`]).
    pub fn issue_access_token(
        &self,
        subject: &str,
        client_id: &str,
        scopes: &[&str],
        lifetime_s: i64,
    ) -> String {
        self.with_state(|s| s.issue_access_token(subject, client_id, scopes, lifetime_s))
    }

    /// The fake clock in Unix milliseconds ([`State::now_ms`]).
    pub fn now_ms(&self) -> i64 {
        self.with_state(|s| s.now_ms())
    }

    /// Moves the fake clock forward ([`State::advance`]).
    pub fn advance(&self, by: Duration) {
        self.with_state(|s| s.advance(by));
    }
}

impl Drop for FakeGoogle {
    fn drop(&mut self) {
        self.shared.stop(self.addr, self.accept.take());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shareable<T: Send + Sync>() {}

    #[test]
    fn a_fake_can_be_shared_with_the_threads_of_a_test() {
        shareable::<FakeGoogle>();
        shareable::<Log>();
        shareable::<Fault>();
    }
}
