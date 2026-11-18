//! Helpers of the sign-in tests (g01 to g17): a fake Google with one client and two accounts,
//! the adapter's sign-in part on the loopback endpoints, and a test clock kept equal to the
//! fake's clock.
//!
//! Every wait is on a condition with a generous bound; nothing sleeps and hopes.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use fabric_desktop_contracts::scalar::Id;
use fabric_google_calendar::oauth::Supersede;
use fabric_google_calendar::oauth::{
    ConnectJob, ConnectOutcome, Flow, GoogleAccess, PendingFlow, RecordingBrowser,
};
use fabric_google_calendar::{
    AccessCache, ClientConfig, Clock, Endpoints, ManualClock, MemoryTokenStore, Secret, TokenStore,
    TokenStoreError, http::Transport,
};
use fabric_google_fake::{Account, Client, Decision, FakeGoogle, Visit};

/// The client of the tests.
pub const CLIENT_ID: &str = "fabric-test-client.apps.googleusercontent.com";
/// Its secret.
pub const CLIENT_SECRET: &str = "GOCSPX-fabric-test-secret-value";
/// The first account: its subject and its address as Google writes it.
pub const SUB_A: &str = "100000000000000000001";
/// The first account's address, domain in capitals to see it lowered.
pub const EMAIL_A: &str = "ann@Example.COM";
/// The second account.
pub const SUB_B: &str = "100000000000000000002";
/// The second account's address.
pub const EMAIL_B: &str = "bob@example.org";

/// A broker account id.
pub fn account(n: u32) -> Id {
    Id::new(format!("account-{n:032x}")).expect("a valid id")
}

/// A token store that counts writes and deletes, over a shared memory store. It can be told to
/// refuse writes while reads go on working.
#[derive(Clone, Default)]
pub struct CountingStore {
    pub inner: MemoryTokenStore,
    counts: Arc<Mutex<(usize, usize)>>,
    refuse_writes: Arc<AtomicBool>,
}

impl CountingStore {
    /// Every later write fails with `Failed`, or works again with `false`.
    pub fn refuse_writes(&self, refuse: bool) {
        self.refuse_writes.store(refuse, Ordering::SeqCst);
    }

    /// How many writes were made.
    pub fn writes(&self) -> usize {
        self.counts.lock().expect("unpoisoned").0
    }

    /// How many deletes were made.
    pub fn deletes(&self) -> usize {
        self.counts.lock().expect("unpoisoned").1
    }

    /// The stored token, as text.
    pub fn token(&self, account: &Id) -> Option<String> {
        self.inner
            .read(account)
            .expect("read")
            .map(|t| t.expose().to_owned())
    }
}

impl TokenStore for CountingStore {
    fn read(&self, account: &Id) -> Result<Option<Secret>, TokenStoreError> {
        self.inner.read(account)
    }
    fn write(&mut self, account: &Id, token: &Secret) -> Result<(), TokenStoreError> {
        self.counts.lock().expect("unpoisoned").0 += 1;
        if self.refuse_writes.load(Ordering::SeqCst) {
            return Err(TokenStoreError::Failed { status: -1 });
        }
        self.inner.write(account, token)
    }
    fn delete(&mut self, account: &Id) -> Result<(), TokenStoreError> {
        self.counts.lock().expect("unpoisoned").1 += 1;
        self.inner.delete(account)
    }
}

impl std::fmt::Debug for CountingStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CountingStore({:?})", self.inner)
    }
}

/// A started flow with the browser its `wait` opens the address in.
pub struct Pending {
    flow: PendingFlow,
    browser: RecordingBrowser,
}

impl Pending {
    /// [`PendingFlow::wait`] with this flow's recording browser.
    pub fn wait(self, store: &mut dyn TokenStore, cache: &AccessCache) -> ConnectOutcome {
        let Pending { flow, mut browser } = self;
        flow.wait(&mut browser, store, cache)
    }

    /// [`PendingFlow::generation`].
    pub fn generation(&self) -> u64 {
        self.flow.generation()
    }

    /// [`PendingFlow::supersede_handle`].
    pub fn supersede_handle(&self) -> Supersede {
        self.flow.supersede_handle()
    }

    /// [`PendingFlow::port`].
    pub fn port(&self) -> Option<u16> {
        self.flow.port()
    }
}

/// Prints the flow's own `Debug`.
impl std::fmt::Debug for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.flow.fmt(f)
    }
}

/// The address a flow's browser is given. The flow opens it in `wait` (3.2.1), so
/// [`Address::expose`] waits, with a generous bound, until `wait` has opened it.
pub struct Address {
    browser: RecordingBrowser,
    /// The fake's port, when the flow sends its requests through a proxy.
    at_fake: Option<u16>,
    taken: OnceLock<Secret>,
}

impl Address {
    fn new(browser: RecordingBrowser, at_fake: Option<u16>) -> Address {
        Address {
            browser,
            at_fake,
            taken: OnceLock::new(),
        }
    }

    fn rewrite(&self, address: Secret) -> Secret {
        let Some(port) = self.at_fake else {
            return address;
        };
        let (_, rest) = address
            .expose()
            .split_once("/o/oauth2/v2/auth")
            .expect("the authorization path");
        Secret::new(format!("http://127.0.0.1:{port}/o/oauth2/v2/auth{rest}"))
    }

    /// The address, once the flow's `wait` has opened it; panics when it is not opened in time.
    pub fn expose(&self) -> &str {
        self.taken
            .get_or_init(|| {
                let address = self
                    .browser
                    .wait_for_address(GUARD_WAIT)
                    .expect("the browser was given an address");
                self.rewrite(address)
            })
            .expose()
    }

    /// Whether the browser was given the address by now; it does not wait.
    pub fn opened(&self) -> bool {
        if self.taken.get().is_some() {
            return true;
        }
        match self.browser.take_address() {
            Some(address) => {
                let _ = self.taken.set(self.rewrite(address));
                true
            }
            None => false,
        }
    }
}

/// Prints nothing of the address.
impl std::fmt::Debug for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Address")
    }
}

/// The fake, the adapter's sign-in part and its state.
pub struct Rig {
    pub fake: FakeGoogle,
    pub clock: ManualClock,
    pub transport: Arc<Transport>,
    pub client: Arc<ClientConfig>,
    pub flow: Flow,
    pub store: CountingStore,
    pub cache: AccessCache,
}

impl Rig {
    /// A fake with the client and the accounts A and B (A signs in by default), and the
    /// adapter on its loopback endpoints with a test clock at the fake's time.
    pub fn new() -> Rig {
        let fake = FakeGoogle::start();
        fake.add_client(Client::new(CLIENT_ID, CLIENT_SECRET));
        fake.add_account(Account::new(SUB_A, EMAIL_A));
        fake.add_account(Account::new(SUB_B, EMAIL_B));
        let clock = ManualClock::new(u64::try_from(fake.now_ms()).expect("positive"));
        let shared: fabric_google_calendar::SharedClock = Arc::new(clock.clone());
        let transport = Arc::new(
            Transport::new(Endpoints::loopback(fake.port()), Arc::clone(&shared))
                .expect("a transport"),
        );
        let client =
            Arc::new(ClientConfig::new(CLIENT_ID, Secret::from(CLIENT_SECRET)).expect("a client"));
        let flow = Flow::new(Arc::clone(&transport), Arc::clone(&client), shared);
        Rig {
            fake,
            clock,
            transport,
            client,
            flow,
            store: CountingStore::default(),
            cache: AccessCache::new(),
        }
    }

    /// Moves the test clock and the fake's clock forward together.
    pub fn advance_ms(&self, ms: u64) {
        self.clock.advance(ms);
        self.fake.advance(Duration::from_millis(ms));
        assert_eq!(
            self.clock.now_ms(),
            u64::try_from(self.fake.now_ms()).expect("positive")
        );
    }

    /// Starts a flow and gives it with the address its `wait` will give the browser.
    pub fn start(&self, job: ConnectJob) -> (Pending, Address) {
        let browser = RecordingBrowser::new();
        let flow = self.flow.start(job);
        let address = Address::new(browser.clone(), None);
        (Pending { flow, browser }, address)
    }

    /// A whole sign-in: the flow waits while the browser plays `decision`.
    pub fn sign_in(&self, job: ConnectJob, decision: Decision) -> (ConnectOutcome, Visit, Address) {
        let (pending, address) = self.start(job);
        let (outcome, visit) = self.finish(pending, &address, decision);
        (outcome, visit, address)
    }

    /// Waits for `pending` while the browser plays `decision` on `address`.
    pub fn finish(
        &self,
        pending: Pending,
        address: &Address,
        decision: Decision,
    ) -> (ConnectOutcome, Visit) {
        let mut store = self.store.clone();
        std::thread::scope(|scope| {
            let visit = scope.spawn(|| {
                self.fake
                    .browser()
                    .visit(address.expose(), decision)
                    .expect("the browser reached the fake")
            });
            let outcome = pending.wait(&mut store, &self.cache);
            (outcome, visit.join().expect("the browser thread ends"))
        })
    }

    /// A new account `n` signed in as the browser's account (A unless `decision` says
    /// otherwise), which must connect.
    pub fn connect(&self, n: u32, decision: Decision) -> ConnectOutcome {
        let (outcome, _, _) = self.sign_in(ConnectJob::new_account(account(n), []), decision);
        assert!(outcome.is_connected(), "{outcome:?}");
        outcome
    }

    /// The access-token part over this rig's store and cache.
    pub fn access(&self) -> GoogleAccess<CountingStore> {
        GoogleAccess::new(
            Arc::clone(&self.transport),
            Arc::clone(&self.client),
            self.store.clone(),
            self.cache.clone(),
            Arc::new(self.clock.clone()),
        )
    }

    /// A second sign-in part over the same client and clock whose requests go to `port` (a
    /// [`HoldingProxy`] in front of the fake). Its key set and its latest flow are its own.
    pub fn flow_through(&self, port: u16) -> Flow {
        let shared: fabric_google_calendar::SharedClock = Arc::new(self.clock.clone());
        let transport = Arc::new(
            Transport::new(Endpoints::loopback(port), Arc::clone(&shared)).expect("a transport"),
        );
        Flow::new(transport, Arc::clone(&self.client), shared)
    }

    /// Starts a flow on `flow` and gives it with the address its `wait` will give the
    /// browser, pointed at the fake when `flow` sends its requests through a proxy.
    pub fn start_on(&self, flow: &Flow, job: ConnectJob) -> (Pending, Address) {
        let browser = RecordingBrowser::new();
        let started = flow.start(job);
        let address = Address::new(browser.clone(), Some(self.fake.port()));
        (
            Pending {
                flow: started,
                browser,
            },
            address,
        )
    }

    /// A deadline 30 s ahead on the test clock.
    pub fn deadline(&self) -> u64 {
        self.clock.now_ms() + 30_000
    }

    /// The checks every test that used the fake ends with (15): no path holds `primary`
    /// (PD22), every Calendar request carried a token, and no request carried a token of an
    /// account the fake does not know.
    pub fn check_log(&self) {
        let log = self.fake.log();
        assert!(!log.any_path_contains("primary"));
        for entry in log.calendar().iter() {
            assert!(entry.authorization, "a Calendar request without a token");
        }
        assert_eq!(self.fake.handler_panics(), 0);
    }

    /// How many token requests with this grant type the fake received.
    pub fn token_requests(&self, grant_type: &str) -> usize {
        self.fake
            .log()
            .token_requests()
            .filter(|e| e.form_value("grant_type") == Some(grant_type))
            .len()
    }
}

/// The decoded query pairs of an address.
pub fn query_pairs(address: &str) -> Vec<(String, String)> {
    let (_, query) = address.split_once('?').expect("a query");
    fabric_google_fake::url::parse_pairs(query).expect("a well-formed query")
}

/// The first value of a query parameter of an address.
pub fn query_value(address: &str, name: &str) -> Option<String> {
    query_pairs(address)
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v)
}

/// Waits until `done` holds, at most `bound`; panics when it does not.
pub fn wait_until(bound: Duration, mut done: impl FnMut() -> bool) {
    let end = Instant::now() + bound;
    while !done() {
        assert!(Instant::now() < end, "the condition did not hold in time");
        std::thread::yield_now();
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// What the proxy's gate is doing.
#[derive(Default)]
struct Gate {
    /// The next token request is to be held.
    armed: bool,
    /// A token request is held now.
    holding: bool,
    /// The held request may go on.
    released: bool,
}

/// A forwarder on `127.0.0.1` between the adapter and the fake that can hold the next token
/// request before the fake sees it, until the test releases it: a flow can be kept inside its
/// token request for exactly as long as a test needs, with no timing. One request per
/// connection, as the transport sends them (`Connection: close`).
pub struct HoldingProxy {
    port: u16,
    gate: Arc<(Mutex<Gate>, Condvar)>,
    stop: Arc<AtomicBool>,
}

impl HoldingProxy {
    /// Starts forwarding to the fake on `fake_port`.
    pub fn start(fake_port: u16) -> HoldingProxy {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let port = listener.local_addr().expect("an address").port();
        let gate: Arc<(Mutex<Gate>, Condvar)> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_gate, thread_stop) = (Arc::clone(&gate), Arc::clone(&stop));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if thread_stop.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let gate = Arc::clone(&thread_gate);
                std::thread::spawn(move || forward(stream, fake_port, &gate));
            }
        });
        HoldingProxy { port, gate, stop }
    }

    /// The port the adapter sends to.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Holds the next `POST /token` until [`HoldingProxy::release`].
    pub fn hold_next_token_request(&self) {
        let (gate, _) = &*self.gate;
        gate.lock().expect("unpoisoned").armed = true;
    }

    /// Waits until a token request is held, at most `GUARD_WAIT`.
    pub fn wait_until_holding(&self) {
        let (gate, signal) = &*self.gate;
        let guard = gate.lock().expect("unpoisoned");
        let (guard, timeout) = signal
            .wait_timeout_while(guard, GUARD_WAIT, |g| !g.holding)
            .expect("unpoisoned");
        assert!(
            !timeout.timed_out() && guard.holding,
            "no token request was held"
        );
    }

    /// Lets the held request go on to the fake.
    pub fn release(&self) {
        let (gate, signal) = &*self.gate;
        gate.lock().expect("unpoisoned").released = true;
        signal.notify_all();
    }
}

impl Drop for HoldingProxy {
    fn drop(&mut self) {
        self.release();
        self.stop.store(true, Ordering::SeqCst);
        // Wakes the accept loop so that it sees the stop.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

/// A guard on the proxy's waits: far longer than any step needs.
const GUARD_WAIT: Duration = Duration::from_secs(20);

/// Reads one request (head and `Content-Length` body), holds it when it is the token request
/// the gate waits for, sends it to the fake and copies the fake's answer back.
fn forward(mut client: TcpStream, fake_port: u16, gate: &(Mutex<Gate>, Condvar)) {
    let _ = client.set_read_timeout(Some(GUARD_WAIT));
    let mut request = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
            break end + 4;
        }
        match client.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => request.extend_from_slice(&chunk[..n]),
        }
    };
    let head = String::from_utf8_lossy(&request[..head_end]).to_ascii_lowercase();
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);
    while request.len() < head_end + length {
        match client.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => request.extend_from_slice(&chunk[..n]),
        }
    }
    if head.starts_with("post /token ") {
        let (lock, signal) = gate;
        let mut state = lock.lock().expect("unpoisoned");
        if state.armed {
            state.armed = false;
            state.holding = true;
            signal.notify_all();
            let (_state, _) = signal
                .wait_timeout_while(state, GUARD_WAIT, |g| !g.released)
                .expect("unpoisoned");
        }
    }
    let Ok(mut fake) = TcpStream::connect(("127.0.0.1", fake_port)) else {
        return;
    };
    let _ = fake.set_read_timeout(Some(GUARD_WAIT));
    if fake.write_all(&request).is_err() {
        return;
    }
    let mut answer = Vec::new();
    let _ = fake.read_to_end(&mut answer);
    let _ = client.write_all(&answer);
    let _ = client.flush();
    let _ = client.shutdown(Shutdown::Write);
}
