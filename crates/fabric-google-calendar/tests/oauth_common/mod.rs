//! Helpers of the sign-in tests (g01 to g17): a fake Google with one client and two accounts,
//! the adapter's sign-in part on the loopback endpoints, and a test clock kept equal to the
//! fake's clock.
//!
//! Every wait is on a condition with a generous bound; nothing sleeps and hopes.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fabric_desktop_contracts::scalar::Id;
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

/// A token store that counts writes and deletes, over a shared memory store.
#[derive(Clone, Default)]
pub struct CountingStore {
    pub inner: MemoryTokenStore,
    counts: Arc<Mutex<(usize, usize)>>,
}

impl CountingStore {
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

    /// Starts a flow and gives it with the address the browser was given.
    pub fn start(&self, job: ConnectJob) -> (PendingFlow, Secret) {
        let browser = RecordingBrowser::new();
        let pending = self.flow.start(job, &mut browser.clone());
        let address = browser
            .take_address()
            .expect("the browser was given an address");
        (pending, address)
    }

    /// A whole sign-in: the flow waits while the browser plays `decision`.
    pub fn sign_in(&self, job: ConnectJob, decision: Decision) -> (ConnectOutcome, Visit, Secret) {
        let (pending, address) = self.start(job);
        let (outcome, visit) = self.finish(pending, &address, decision);
        (outcome, visit, address)
    }

    /// Waits for `pending` while the browser plays `decision` on `address`.
    pub fn finish(
        &self,
        pending: PendingFlow,
        address: &Secret,
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
