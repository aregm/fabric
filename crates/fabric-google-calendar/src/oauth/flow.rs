//! One sign-in from start to the token store (3.2, 3.2.2, 3.7, PD5, PD8 to PD10, PD34).
//!
//! [`Flow::start`] makes the verifier, the state and the nonce, binds the loopback listener and
//! builds the authorization address; it opens nothing. It gives a [`PendingFlow`].
//! [`PendingFlow::wait`] hands the address to the [`BrowserOpener`], unless the flow was
//! superseded before, serves the listener until it ends (3.3), then makes the token request
//! (3.4), reads the granted scopes (3.6), checks the identity token (3.5)
//! and compares the subject with what the [`ConnectJob`] names (3.7), and only then writes the
//! refresh token to the token store. It gives the [`ConnectOutcome`] of 3.2.2.
//!
//! - Nothing but a `Connected` outcome writes to the token store. A flow that ends otherwise
//!   wipes what Google issued (every token is a [`Secret`], dropped on return) and asks for no
//!   revocation (PD34).
//! - A reconnect that does not end `Connected` forgets the account's old refresh token and
//!   its cached access token ([`forget`]; 3.7, OQ12).
//! - [`Supersede`], [`Flow::supersede_latest`] and [`Flow::supersede`] end a flow from another
//!   thread (PD5; 18.7 point 5(a)). A flow superseded after the listener ended, but before the
//!   token store is written, ends `Cancelled(Superseded)` too, and so does one superseded
//!   before [`PendingFlow::wait`] runs: it then opens no browser, runs no listener and makes
//!   no request.
//! - The adapter keeps PD5 itself: [`Flow::start`] supersedes the flow that the same [`Flow`]
//!   (or a clone) started before, and gives the new one the next generation. A superseded
//!   flow never writes the token store or the access cache and never forgets the account, so
//!   it cannot delete what the flow after it stored (3.3). Its last "was I superseded" check
//!   and its write happen under one lock, the turn's, that `start` and every supersede take
//!   too: once a supersede has returned `true`, that flow writes nothing; when it returned
//!   `false`, the flow had already ended, written or not.
//!
//! Order of the checks after the token answer: the shape of the answer and `token_type`, then
//! the scopes (so that an unticked address scope is a partial grant and not an identity
//! failure), then the identity token, then the refresh token's presence, then the subject.

use std::collections::BTreeSet;
use std::fmt;
#[cfg(debug_assertions)]
use std::sync::Condvar;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
#[cfg(debug_assertions)]
use std::time::Duration;
use std::time::Instant;

use fabric_desktop_contracts::scalar::{Email, Id};

use crate::client::ClientConfig;
use crate::clock::SharedClock;
use crate::http::{CALL_LIMIT_MS, Transport};
use crate::secret::Secret;
use crate::tokens::{AccessCache, TokenStore, TokenStoreError, forget};

use super::exchange::{self, AnswerProblem};
use super::id_token::{self, CheckFailure, Expected, IdentityProblem, KeyCache};
use super::listener::{CallbackEnd, Listener};
use super::pkce::{self, FlowSecrets};
use super::scopes::{self, Grant, Scope};
use super::url::{self, AddressPurpose};

/// Why a sign-in was cancelled (3.2.2). Fixed labels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelReason {
    /// The listener's 300 s passed (3.3).
    TimedOut,
    /// Another `connect_google`, or the broker quitting, superseded it (PD5).
    Superseded,
    /// The browser could not be started (3.2.1).
    BrowserNotOpened,
    /// The callback carried another `error`, both or neither of `code` and `error`, or an
    /// issuer other than Google's; or the token endpoint answered an error other than the
    /// client's, or a token answer of the wrong shape or `token_type` (3.3, 3.4).
    GoogleError,
    /// Google does not accept the client: `invalid_client`, `unauthorized_client` (3.4).
    ClientRejected,
    /// The token or key request failed: nothing or nothing usable came back. Also a sign-in
    /// that could not start: no loopback port could be bound, or the system gave no random
    /// bytes (3.2.2 names `Network` for these).
    Network,
    /// A check of the identity token failed (3.5), with its fixed label.
    IdentityCheckFailed(IdentityProblem),
    /// More scopes than the five were granted (PD9).
    UnexpectedScope,
    /// The token answer had no refresh token (3.4).
    NoRefreshToken,
    /// The token store refused or failed to write the refresh token (section 4).
    TokenStore(TokenStoreError),
}

impl CancelReason {
    /// The fixed label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            CancelReason::TimedOut => "the sign-in timed out",
            CancelReason::Superseded => "the sign-in was superseded",
            CancelReason::BrowserNotOpened => "the browser could not be opened",
            CancelReason::GoogleError => "Google answered with an error",
            CancelReason::ClientRejected => "Google does not accept this client",
            CancelReason::Network => "the request to Google failed",
            CancelReason::IdentityCheckFailed(_) => "the identity token was refused",
            CancelReason::UnexpectedScope => "Google granted more than was asked",
            CancelReason::NoRefreshToken => "Google issued no refresh token",
            CancelReason::TokenStore(_) => "the token store could not keep the sign-in",
        }
    }
}

impl fmt::Display for CancelReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CancelReason::IdentityCheckFailed(problem) => {
                write!(f, "{}: {problem}", self.label())
            }
            CancelReason::TokenStore(error) => write!(f, "{}: {error}", self.label()),
            _ => f.write_str(self.label()),
        }
    }
}

/// How a sign-in ended (3.2.2).
#[derive(Clone, PartialEq, Eq)]
pub enum ConnectOutcome {
    /// Every check held and the refresh token is in the token store.
    Connected {
        /// The Google subject (`sub`), the account's identity (PD10).
        subject: String,
        /// The verified address, its domain in lower case; display only (PD10).
        email: Email,
        /// The granted scopes: the five.
        scopes: Vec<Scope>,
        /// When the connection ends, in Unix ms, for time-based access
        /// (`refresh_token_expires_in`, 3.4); `None` when Google named no end.
        refresh_token_ends_at: Option<u64>,
    },
    /// The callback carried `error=access_denied`.
    ConsentDenied,
    /// The address scope or a calendar scope was not granted (PD8). Nothing stored.
    PartialGrant {
        /// The scopes not granted.
        missing: Vec<Scope>,
    },
    /// A reconnect signed in as another subject, or a new connect as a subject that an
    /// account record already has (3.7).
    AccountMismatch,
    /// Anything else (3.2.2).
    Cancelled(CancelReason),
}

impl ConnectOutcome {
    /// Whether this is `Connected`.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        matches!(self, ConnectOutcome::Connected { .. })
    }
}

/// Prints the kind, the scopes and the end time; never the subject or the address.
impl fmt::Debug for ConnectOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectOutcome::Connected {
                scopes,
                refresh_token_ends_at,
                ..
            } => f
                .debug_struct("Connected")
                .field("subject", &"[redacted]")
                .field("email", &"[redacted]")
                .field("scopes", scopes)
                .field("refresh_token_ends_at", refresh_token_ends_at)
                .finish(),
            ConnectOutcome::ConsentDenied => f.write_str("ConsentDenied"),
            ConnectOutcome::PartialGrant { missing } => f
                .debug_struct("PartialGrant")
                .field("missing", missing)
                .finish(),
            ConnectOutcome::AccountMismatch => f.write_str("AccountMismatch"),
            ConnectOutcome::Cancelled(reason) => f.debug_tuple("Cancelled").field(reason).finish(),
        }
    }
}

/// The browser could not be started (3.2.1). A fixed label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrowserNotOpened;

impl fmt::Display for BrowserNotOpened {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the browser could not be opened")
    }
}

impl std::error::Error for BrowserNotOpened {}

/// Opens the authorization address in the system browser (3.2.1, PD7). The real
/// `SystemBrowser` (`/usr/bin/open`) is phase 2; tests use `RecordingBrowser`, which exists
/// only in a debug build (PD2).
pub trait BrowserOpener {
    /// Opens `address`. An implementation must not log or keep it longer than it needs to.
    fn open(&mut self, address: &Secret) -> Result<(), BrowserNotOpened>;
}

/// A test double that opens nothing: it keeps each address for the test to take, or refuses
/// every one. Clones share the addresses. Only in a debug build (PD2: no test seam in
/// release).
#[cfg(debug_assertions)]
#[derive(Clone, Default)]
pub struct RecordingBrowser {
    /// The addresses not taken yet, and the signal of a new one.
    addresses: Arc<(Mutex<Vec<Secret>>, Condvar)>,
    refuse: bool,
}

#[cfg(debug_assertions)]
impl RecordingBrowser {
    /// A double that accepts every address.
    #[must_use]
    pub fn new() -> RecordingBrowser {
        RecordingBrowser::default()
    }

    /// A double that refuses every address, as a browser that cannot be started.
    #[must_use]
    pub fn refusing() -> RecordingBrowser {
        RecordingBrowser {
            addresses: Arc::default(),
            refuse: true,
        }
    }

    /// Takes the oldest address not taken yet.
    #[must_use]
    pub fn take_address(&self) -> Option<Secret> {
        let mut addresses = lock(&self.addresses.0);
        (!addresses.is_empty()).then(|| addresses.remove(0))
    }

    /// Takes the oldest address not taken yet, waiting at most `bound` for one to be opened.
    #[must_use]
    pub fn wait_for_address(&self, bound: Duration) -> Option<Secret> {
        let (addresses, opened) = &*self.addresses;
        let guard = lock(addresses);
        let (mut addresses, _) = opened
            .wait_timeout_while(guard, bound, |a| a.is_empty())
            .unwrap_or_else(PoisonError::into_inner);
        (!addresses.is_empty()).then(|| addresses.remove(0))
    }
}

#[cfg(debug_assertions)]
impl BrowserOpener for RecordingBrowser {
    fn open(&mut self, address: &Secret) -> Result<(), BrowserNotOpened> {
        if self.refuse {
            return Err(BrowserNotOpened);
        }
        let (addresses, opened) = &*self.addresses;
        lock(addresses).push(Secret::from(address.expose()));
        opened.notify_all();
        Ok(())
    }
}

/// Prints how many addresses wait, never one.
#[cfg(debug_assertions)]
impl fmt::Debug for RecordingBrowser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let waiting = lock(&self.addresses.0).len();
        write!(
            f,
            "RecordingBrowser({waiting} waiting, refuse {})",
            self.refuse
        )
    }
}

/// What the subject must be (3.7).
#[derive(Clone, PartialEq, Eq)]
enum ExpectedSubject {
    /// A reconnect: the account's stored subject.
    Reconnect(String),
    /// A new account: none of the subjects any account record has.
    New(BTreeSet<String>),
}

/// The job of one `connect_google` (3.2, 3.7): the broker's account id under which the refresh
/// token is stored, and what the subject must be.
#[derive(Clone, PartialEq, Eq)]
pub struct ConnectJob {
    account: Id,
    expected: ExpectedSubject,
}

impl ConnectJob {
    /// A reconnect of a known account: the subject must be `subject`, the one stored.
    #[must_use]
    pub fn reconnect(account: Id, subject: impl Into<String>) -> ConnectJob {
        ConnectJob {
            account,
            expected: ExpectedSubject::Reconnect(subject.into()),
        }
    }

    /// A new account record `account`: the subject must not be any of `taken`, the subjects
    /// that account records already have.
    #[must_use]
    pub fn new_account(account: Id, taken: impl IntoIterator<Item = String>) -> ConnectJob {
        ConnectJob {
            account,
            expected: ExpectedSubject::New(taken.into_iter().collect()),
        }
    }

    /// The account the token is stored under.
    #[must_use]
    pub fn account(&self) -> &Id {
        &self.account
    }

    /// Whether this is a reconnect.
    #[must_use]
    pub fn is_reconnect(&self) -> bool {
        matches!(self.expected, ExpectedSubject::Reconnect(_))
    }

    fn accepts(&self, subject: &str) -> bool {
        match &self.expected {
            ExpectedSubject::Reconnect(stored) => stored == subject,
            ExpectedSubject::New(taken) => !taken.contains(subject),
        }
    }
}

/// Prints the kind only, never an account id or a subject.
impl fmt::Debug for ConnectJob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.expected {
            ExpectedSubject::Reconnect(_) => f.write_str("ConnectJob(reconnect)"),
            ExpectedSubject::New(taken) => write!(f, "ConnectJob(new, {} taken)", taken.len()),
        }
    }
}

/// Ends one flow from another thread (PD5). Clones share the flag. A handle made with
/// `default()` belongs to no flow and supersedes nothing.
#[derive(Clone, Default)]
pub struct Supersede {
    /// Set once the flow is superseded; the listener reads it (3.3).
    flag: Arc<AtomicBool>,
    /// The turn of the [`Flow`] that started the flow, whose lock every supersede takes.
    turn: Arc<Mutex<Turn>>,
    /// The flow's generation.
    generation: u64,
}

impl Supersede {
    /// Ends the flow, as [`Flow::supersede`] with its generation does: under the lock under
    /// which the flow writes the token store. `true` when the flow had not ended and was not
    /// superseded yet; it then ends `Cancelled(Superseded)`, its listener stops within
    /// 250 ms, and it never writes the token store or the access cache. `false` when it had
    /// already ended (written or not), or was superseded before.
    pub fn supersede(&self) -> bool {
        supersede_generation(&mut lock(&self.turn), self.generation)
    }

    fn is_set(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

impl fmt::Debug for Supersede {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Supersede({})", self.is_set())
    }
}

/// Which flow is the latest (PD5): its generation, its supersede flag and whether it ended.
#[derive(Default)]
struct Turn {
    /// The generation of the latest flow started; 0 before the first.
    current: u64,
    /// The supersede flag of the latest flow started.
    latest: Option<Arc<AtomicBool>>,
    /// Whether the latest flow ended: it wrote the token store, or `wait` finished it.
    ended: bool,
}

/// Supersedes the flow of `generation` when it is the latest, has not ended and is not
/// superseded yet; the caller holds the turn's lock. A flow that is no longer the latest was
/// superseded by the start that followed it.
fn supersede_generation(turn: &mut Turn, generation: u64) -> bool {
    if generation == 0 || turn.current != generation || turn.ended {
        return false;
    }
    match &turn.latest {
        Some(flag) if !flag.load(Ordering::SeqCst) => {
            flag.store(true, Ordering::SeqCst);
            true
        }
        _ => false,
    }
}

/// The sign-in part of the adapter: the transport, the client, the clock, Google's key set in
/// memory, and which flow is the latest (PD5). One per broker; clones share the key set and
/// the latest flow, so a start on any clone supersedes a flow started on another.
#[derive(Clone)]
pub struct Flow {
    transport: Arc<Transport>,
    client: Arc<ClientConfig>,
    clock: SharedClock,
    keys: Arc<Mutex<KeyCache>>,
    turn: Arc<Mutex<Turn>>,
}

impl Flow {
    /// A sign-in part over `transport` (whose endpoint set gives the authorization address).
    #[must_use]
    pub fn new(transport: Arc<Transport>, client: Arc<ClientConfig>, clock: SharedClock) -> Flow {
        Flow {
            transport,
            client,
            clock,
            keys: Arc::new(Mutex::new(KeyCache::new())),
            turn: Arc::new(Mutex::new(Turn::default())),
        }
    }

    /// Starts a sign-in for `job` (3.2): fresh verifier, state and nonce, the listener on
    /// `127.0.0.1:0`, and the authorization address. It only prepares: no browser is opened
    /// until [`PendingFlow::wait`] runs, so a flow superseded in between shows nothing. The
    /// listener's 300 s count from here.
    ///
    /// It supersedes the flow this `Flow` (or a clone) started before, if that one is still
    /// under way, and makes the new one the latest (PD5).
    ///
    /// The 300 s run from here whether or not `wait` was called, so call it soon.
    pub fn start(&self, job: ConnectJob) -> PendingFlow {
        let flag = Arc::new(AtomicBool::new(false));
        let generation = {
            let mut turn = lock(&self.turn);
            let previous = turn.current;
            supersede_generation(&mut turn, previous);
            turn.latest = Some(Arc::clone(&flag));
            turn.ended = false;
            turn.current = turn.current.wrapping_add(1);
            turn.current
        };
        let supersede = Supersede {
            flag,
            turn: Arc::clone(&self.turn),
            generation,
        };
        let started_ms = self.clock.now_ms();
        let started_at = Instant::now();
        let stage = match self.prepare(&job) {
            Ok(listening) => Stage::Listening(Box::new(listening)),
            Err(reason) => Stage::Ended(reason),
        };
        PendingFlow {
            flow: self.clone(),
            job,
            stage,
            started_ms,
            started_at,
            supersede,
            generation,
        }
    }

    fn prepare(&self, job: &ConnectJob) -> Result<Listening, CancelReason> {
        let secrets = FlowSecrets::new().map_err(|_| CancelReason::Network)?;
        let listener = Listener::bind().map_err(|_| CancelReason::Network)?;
        let redirect_uri = url::redirect_uri(listener.port());
        let purpose = match &job.expected {
            ExpectedSubject::Reconnect(subject) => AddressPurpose::Reconnect { subject },
            ExpectedSubject::New(_) => AddressPurpose::NewAccount,
        };
        let address = url::authorization_address(
            self.transport.endpoints(),
            self.client.client_id(),
            &redirect_uri,
            &pkce::challenge(&secrets.verifier),
            &secrets.state,
            &secrets.nonce,
            purpose,
        );
        Ok(Listening {
            listener,
            secrets,
            redirect_uri,
            address,
        })
    }

    /// Supersedes the latest flow this `Flow` (or a clone) started, if it has not ended: for
    /// a new connect and for the broker quitting (18.7 point 5(a)). It takes the lock under
    /// which a flow makes its last "was I superseded" check and writes the token store, so
    /// once it has returned `true` that flow never writes the token store or the access cache
    /// and ends `Cancelled(Superseded)`. `false` when there is nothing to supersede: no flow
    /// was started, or the latest one had already ended (written or not) or was superseded
    /// before.
    pub fn supersede_latest(&self) -> bool {
        let mut turn = lock(&self.turn);
        let latest = turn.current;
        supersede_generation(&mut turn, latest)
    }

    /// Supersedes the flow of `generation` ([`PendingFlow::generation`]) and never another
    /// one, under the same lock and with the same answer as [`Flow::supersede_latest`]. A flow
    /// that is no longer the latest was superseded by the start that followed it: `false`.
    pub fn supersede(&self, generation: u64) -> bool {
        supersede_generation(&mut lock(&self.turn), generation)
    }

    fn deadline(&self) -> u64 {
        self.clock.now_ms().saturating_add(CALL_LIMIT_MS)
    }
}

/// Prints nothing but the kind.
impl fmt::Debug for Flow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Flow")
            .field("transport", &self.transport)
            .field("client", &self.client)
            .finish_non_exhaustive()
    }
}

struct Listening {
    listener: Listener,
    secrets: FlowSecrets,
    redirect_uri: String,
    /// The authorization address, handed to the browser by `wait` (3.2.1).
    address: Secret,
}

enum Stage {
    Listening(Box<Listening>),
    Ended(CancelReason),
}

/// A prepared sign-in (3.2). Dropping it closes the listener. No browser is opened until
/// [`PendingFlow::wait`] runs.
///
/// The listener's 300 s count from [`Flow::start`], whether or not `wait` was called.
pub struct PendingFlow {
    flow: Flow,
    job: ConnectJob,
    stage: Stage,
    started_ms: u64,
    started_at: Instant,
    supersede: Supersede,
    /// This flow's generation; the flow is the latest while the [`Flow`]'s is the same.
    generation: u64,
}

impl PendingFlow {
    /// A handle that ends this flow from another thread (PD5).
    #[must_use]
    pub fn supersede_handle(&self) -> Supersede {
        self.supersede.clone()
    }

    /// This flow's generation: the count of flows its [`Flow`] (and every clone) started, this
    /// one included. It names the flow for [`Flow::supersede`] (18.7 point 5(a)).
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The listener's port, while it listens.
    #[must_use]
    pub fn port(&self) -> Option<u16> {
        match &self.stage {
            Stage::Listening(listening) => Some(listening.listener.port()),
            Stage::Ended(_) => None,
        }
    }

    /// Opens the authorization address with `browser` (3.2.1), waits for the callback and
    /// finishes the sign-in (3.2 to 3.7). Writes the refresh token to `store` only for
    /// `Connected`, and then keeps the new access token in `cache`. A reconnect that does not
    /// end `Connected` forgets the account in `store` and `cache`. A browser that cannot be
    /// started ends the flow `Cancelled(BrowserNotOpened)`.
    ///
    /// A flow that is superseded, by a newer start or by a supersede (PD5; 18.7 point 5(a)),
    /// ends `Cancelled(Superseded)` without writing, and never forgets the account: the later
    /// flow owns it now. One superseded before `wait` is called ends so at once: no browser
    /// is opened, its listener is closed without being run, and no request is made.
    pub fn wait(
        self,
        browser: &mut dyn BrowserOpener,
        store: &mut dyn TokenStore,
        cache: &AccessCache,
    ) -> ConnectOutcome {
        let PendingFlow {
            flow,
            job,
            stage,
            started_ms,
            started_at,
            supersede,
            generation,
        } = self;
        let started = Started {
            ms: started_ms,
            at: started_at,
        };
        let ours = Ours {
            supersede: &supersede,
            generation,
        };
        // Superseded before it ran (18.7 point 5(a)): no browser is opened, the listener is
        // dropped, and so closed, without being run, and nothing is asked or written.
        if ours.superseded(&lock(&flow.turn)) {
            drop(stage);
            return ConnectOutcome::Cancelled(CancelReason::Superseded);
        }
        let outcome = match stage {
            Stage::Ended(reason) => ConnectOutcome::Cancelled(reason),
            // 3.2.1: the browser is opened only now, after the check above.
            Stage::Listening(listening) => match browser.open(&listening.address) {
                Ok(()) => finish(&flow, &job, *listening, started, &ours, store, cache),
                Err(BrowserNotOpened) => ConnectOutcome::Cancelled(CancelReason::BrowserNotOpened),
            },
        };
        if outcome.is_connected() {
            // Ended under the turn's lock when the token was written (`finish`).
            return outcome;
        }
        // The end of a flow that wrote nothing, under the turn's lock that every supersede
        // takes: a supersede that came first turns the outcome into `Cancelled(Superseded)`
        // and the flow forgets nothing; one that comes later finds the flow ended.
        let mut turn = lock(&flow.turn);
        if ours.superseded(&turn) {
            return ConnectOutcome::Cancelled(CancelReason::Superseded);
        }
        turn.ended = true;
        if job.is_reconnect() {
            // Fail closed (3.7, OQ12), only while this flow is the latest and not superseded:
            // a later flow may have stored a token for the account (3.3). A store that fails
            // here keeps the old token, but the account is not connected and the next connect
            // replaces it; the cached access token is dropped in every case.
            let _ = forget(store, cache, &job.account);
        }
        drop(turn);
        outcome
    }
}

/// Prints the kind and whether it still listens.
impl fmt::Debug for PendingFlow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let stage = match &self.stage {
            Stage::Listening(_) => "listening",
            Stage::Ended(_) => "ended",
        };
        f.debug_struct("PendingFlow")
            .field("job", &self.job)
            .field("stage", &stage)
            .field("superseded", &self.supersede.is_set())
            .finish()
    }
}

/// Locks `mutex`, recovering from a panic in another holder: the turn and the key set hold
/// plain values with no invariant a panic could break.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// When a flow started, on the crate's clock and on the monotonic clock (3.3).
#[derive(Clone, Copy)]
struct Started {
    ms: u64,
    at: Instant,
}

/// What tells a flow it is no longer wanted: its supersede flag and its generation (PD5).
struct Ours<'a> {
    supersede: &'a Supersede,
    generation: u64,
}

impl Ours<'_> {
    /// Whether the flow was superseded, by its handle or by a newer start; `turn` is the
    /// [`Flow`]'s, read under its lock.
    fn superseded(&self, turn: &Turn) -> bool {
        self.supersede.is_set() || turn.current != self.generation
    }
}

fn finish(
    flow: &Flow,
    job: &ConnectJob,
    listening: Listening,
    started: Started,
    ours: &Ours<'_>,
    store: &mut dyn TokenStore,
    cache: &AccessCache,
) -> ConnectOutcome {
    use ConnectOutcome::Cancelled;
    let Listening {
        listener,
        secrets,
        redirect_uri,
        address,
    } = listening;
    drop(address);
    // The listener is consumed: closed, its threads joined, before the token request (3.3).
    let code = match listener.run(
        &secrets.state,
        flow.clock.as_ref(),
        started.ms,
        started.at,
        &ours.supersede.flag,
    ) {
        CallbackEnd::Code(code) => code,
        CallbackEnd::Denied => return ConnectOutcome::ConsentDenied,
        CallbackEnd::GoogleError => return Cancelled(CancelReason::GoogleError),
        CallbackEnd::TimedOut => return Cancelled(CancelReason::TimedOut),
        CallbackEnd::Superseded => return Cancelled(CancelReason::Superseded),
    };
    if ours.superseded(&lock(&flow.turn)) {
        return Cancelled(CancelReason::Superseded);
    }
    let answer = match exchange::exchange_code(
        &flow.transport,
        &flow.client,
        &code,
        &secrets.verifier,
        &redirect_uri,
        flow.deadline(),
    ) {
        Ok(answer) => answer,
        Err(AnswerProblem::NoAnswer) => return Cancelled(CancelReason::Network),
        Err(AnswerProblem::Shape) => return Cancelled(CancelReason::GoogleError),
        Err(AnswerProblem::Error(error)) if error.rejects_client() => {
            return Cancelled(CancelReason::ClientRejected);
        }
        Err(AnswerProblem::Error(_)) => return Cancelled(CancelReason::GoogleError),
    };
    drop(code);
    // 3.4: `token_type` and `scope` are required members.
    if answer.bearer != Some(true) {
        return Cancelled(CancelReason::GoogleError);
    }
    let Some(scope) = answer.scope.as_deref() else {
        return Cancelled(CancelReason::GoogleError);
    };
    match scopes::classify(scope) {
        Grant::Complete => {}
        Grant::Unexpected => return Cancelled(CancelReason::UnexpectedScope),
        Grant::Partial(missing) => return ConnectOutcome::PartialGrant { missing },
        Grant::NoOpenId => {
            return Cancelled(CancelReason::IdentityCheckFailed(IdentityProblem::NoOpenId));
        }
    }
    let Some(token) = answer.id_token.as_ref() else {
        return Cancelled(CancelReason::IdentityCheckFailed(
            IdentityProblem::NoIdToken,
        ));
    };
    let identity = {
        let mut keys = lock(&flow.keys);
        let expected = Expected {
            client_id: flow.client.client_id(),
            nonce: &secrets.nonce,
            now_ms: flow.clock.now_ms(),
        };
        let deadline = flow.deadline();
        let mut fetch = || id_token::fetch_key_set(&flow.transport, deadline);
        match id_token::check(token, &expected, &mut keys, &mut fetch) {
            Ok(identity) => identity,
            Err(CheckFailure::Identity(problem)) => {
                return Cancelled(CancelReason::IdentityCheckFailed(problem));
            }
            Err(CheckFailure::KeysUnavailable) => return Cancelled(CancelReason::Network),
        }
    };
    let Some(refresh_token) = answer.refresh_token.as_ref() else {
        return Cancelled(CancelReason::NoRefreshToken);
    };
    // 3.7: compared before anything is written.
    if !job.accepts(&identity.subject) {
        return ConnectOutcome::AccountMismatch;
    }
    // The last check and the write under the turn's lock, which `Flow::start` takes too: a
    // newer start either comes first and this flow writes nothing, or comes after the write
    // and the newer flow owns the account from then on (PD5, 3.3).
    let now = {
        let mut turn = lock(&flow.turn);
        if ours.superseded(&turn) {
            return Cancelled(CancelReason::Superseded);
        }
        if let Err(error) = store.write(&job.account, refresh_token) {
            return Cancelled(CancelReason::TokenStore(error));
        }
        // Ended: a supersede from now on finds nothing to supersede (18.7 point 5(a)).
        turn.ended = true;
        let now = flow.clock.now_ms();
        cache.put(
            &job.account,
            Secret::from(answer.access_token.expose()),
            now.saturating_add(answer.expires_in_s.saturating_mul(1000)),
        );
        drop(turn);
        now
    };
    ConnectOutcome::Connected {
        subject: identity.subject.clone(),
        email: identity.email.clone(),
        scopes: Scope::ALL.to_vec(),
        refresh_token_ends_at: answer
            .refresh_token_expires_in_s
            .map(|s| now.saturating_add(s.saturating_mul(1000))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u32) -> Id {
        Id::new(format!("account-{n:032x}")).expect("an id")
    }

    #[test]
    fn a_job_accepts_the_stored_subject_or_any_subject_not_taken() {
        let reconnect = ConnectJob::reconnect(id(1), "111");
        assert!(reconnect.accepts("111"));
        assert!(!reconnect.accepts("222"));
        assert!(reconnect.is_reconnect());
        let new = ConnectJob::new_account(id(2), ["111".to_owned(), "222".to_owned()]);
        assert!(!new.accepts("111") && !new.accepts("222"));
        assert!(new.accepts("333"));
        assert!(!new.is_reconnect());
        assert_eq!(format!("{reconnect:?}"), "ConnectJob(reconnect)");
        assert_eq!(format!("{new:?}"), "ConnectJob(new, 2 taken)");
    }

    #[test]
    fn the_outcome_prints_no_subject_and_no_address() {
        let outcome = ConnectOutcome::Connected {
            subject: "1234567890".to_owned(),
            email: Email::new("ann@example.com").expect("an email"),
            scopes: Scope::ALL.to_vec(),
            refresh_token_ends_at: None,
        };
        let text = format!("{outcome:?}");
        assert!(
            !text.contains("1234567890") && !text.contains("ann@"),
            "{text}"
        );
        assert_eq!(
            CancelReason::IdentityCheckFailed(IdentityProblem::Nonce).to_string(),
            "the identity token was refused: the identity token's nonce is not this sign-in's"
        );
    }

    #[test]
    #[cfg(debug_assertions)]
    fn the_recording_browser_hands_out_addresses_in_order_and_can_refuse() {
        let mut browser = RecordingBrowser::new();
        let reader = browser.clone();
        browser.open(&Secret::from("a")).expect("opened");
        browser.open(&Secret::from("b")).expect("opened");
        assert_eq!(
            format!("{reader:?}"),
            "RecordingBrowser(2 waiting, refuse false)"
        );
        assert_eq!(reader.take_address().expect("a").expose(), "a");
        assert_eq!(reader.take_address().expect("b").expose(), "b");
        assert!(reader.take_address().is_none());
        assert_eq!(
            RecordingBrowser::refusing().open(&Secret::from("a")),
            Err(BrowserNotOpened)
        );
    }
}
