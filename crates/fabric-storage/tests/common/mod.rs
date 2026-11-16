//! Helpers shared by the store's acceptance tests (design section 9.2).
//!
//! Temporary directories are made by hand under `std::env::temp_dir()` (pid and a counter) and
//! removed on drop; the data directory a test passes is a not-yet-existing child, so the store
//! creates it (design 9). Keys come from `TestKey` or a scripted `KeyProvider`; nothing here
//! touches the Keychain or the network.

#![allow(
    dead_code,
    reason = "each test binary uses a different part of the shared helpers"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use fabric_desktop_contracts::owner::{Change, Preferences};
use fabric_desktop_contracts::scalar::{Id, Label, Timestamp};
use fabric_desktop_contracts::typed::from_value;
use fabric_storage::{
    Actor, AgentRow, Committed, DATABASE_FILE, IdempotencyScope, KeyError, KeyKind, KeyProvider,
    Mutation, OpMeta, OpenError, OpenOptions, Store, StoreFault, StoreKey, TEST_KEY, TestKey,
    TxError, Unavailable, WriteTx, format_timestamp,
};
use serde_json::{Value, json};

/// The tests' base time (design 9).
pub const T0: i64 = 1_795_190_400_000;

/// A temporary directory removed on drop.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// A new empty directory named after `label`, the process id and a counter.
    pub fn new(label: &str) -> TempDir {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("fabric-storage-{label}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir { path }
    }

    /// The directory itself.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The store's data directory: a child that does not exist yet.
    pub fn data_dir(&self) -> PathBuf {
        self.path.join("data")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        restore_write_permission(&self.path);
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(unix)]
fn restore_write_permission(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700));
            restore_write_permission(&p);
        }
    }
}

#[cfg(not(unix))]
fn restore_write_permission(_path: &Path) {}

/// Creates `data_dir` the way the store would (mode 0700 on Unix), for tests that put a file
/// there before the first open.
pub fn make_data_dir(data_dir: &Path) {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(data_dir).unwrap();
}

/// A `KeyProvider` whose answers the test chooses; it counts `create` calls.
#[derive(Clone)]
pub struct ScriptedKey {
    kind: KeyKind,
    key: Option<[u8; 32]>,
    creates: Arc<AtomicUsize>,
}

impl ScriptedKey {
    /// A provider of `kind` that already holds `key`.
    pub fn holding(kind: KeyKind, key: [u8; 32]) -> ScriptedKey {
        ScriptedKey {
            kind,
            key: Some(key),
            creates: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// A provider of `kind` that holds no key.
    pub fn empty(kind: KeyKind) -> ScriptedKey {
        ScriptedKey {
            kind,
            key: None,
            creates: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// How many times the store called `create`.
    pub fn creates(&self) -> usize {
        self.creates.load(Ordering::SeqCst)
    }
}

impl KeyProvider for ScriptedKey {
    fn kind(&self) -> KeyKind {
        self.kind
    }

    fn read(&self) -> Result<Option<StoreKey>, KeyError> {
        Ok(self.key.map(StoreKey::from_bytes))
    }

    fn create(&self, _key: &StoreKey) -> Result<(), KeyError> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// Open options for `data_dir` with `key` at `now_ms`.
pub fn options(data_dir: &Path, key: impl KeyProvider + 'static, now_ms: i64) -> OpenOptions {
    OpenOptions {
        data_dir: data_dir.to_path_buf(),
        key: Box::new(key),
        now_ms,
    }
}

/// Opens the store with the test key; panics with the error if it does not open.
pub fn open(data_dir: &Path, now_ms: i64) -> Store {
    open_with(options(data_dir, TestKey, now_ms))
}

/// Opens the store with `options`; panics with the error if it does not open.
pub fn open_with(options: OpenOptions) -> Store {
    match Store::open(options) {
        Ok(store) => store,
        Err(e) => panic!("the store did not open: {e} ({e:?})"),
    }
}

/// The error of an open that must fail.
pub fn open_error(options: OpenOptions) -> OpenError {
    match Store::open(options) {
        Ok(_) => panic!("the store opened, but the test expects a refusal"),
        Err(e) => e,
    }
}

/// The reason of an open that must end in `OpenError::Unavailable`.
pub fn unavailable(options: OpenOptions) -> Unavailable {
    match open_error(options) {
        OpenError::Unavailable(reason) => reason,
        other => panic!("expected the store to be unavailable, got {other:?}"),
    }
}

/// A new store with run 1 started at `T0`.
pub fn new_store(data_dir: &Path) -> Store {
    let mut store = open(data_dir, T0);
    start_run(&mut store, 1, T0);
    store
}

/// Lowercase hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A second connection to the store's database with the test key, as the fault-injection
/// tests use it (design 7.2). On macOS it keys SQLCipher; elsewhere the store is plain SQLite.
pub fn raw(data_dir: &Path) -> rusqlite::Connection {
    let connection = rusqlite::Connection::open(data_dir.join(DATABASE_FILE)).unwrap();
    key_connection(&connection, &TEST_KEY);
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    connection
}

/// Keys `connection` with `key` (macOS only; the statement form of design 3.1).
#[cfg(target_os = "macos")]
pub fn key_connection(connection: &rusqlite::Connection, key: &[u8; 32]) {
    connection
        .execute_batch(&format!("PRAGMA key = \"x'{}'\";", hex(key)))
        .unwrap();
}

/// Elsewhere the test store is plain SQLite and takes no key.
#[cfg(not(target_os = "macos"))]
pub fn key_connection(_connection: &rusqlite::Connection, _key: &[u8; 32]) {}

/// One integer from `sql` on `connection`.
pub fn query_i64(connection: &rusqlite::Connection, sql: &str) -> i64 {
    connection.query_row(sql, [], |r| r.get(0)).unwrap()
}

/// The number of rows in `table`, read by a second connection.
pub fn count_rows(data_dir: &Path, table: &str) -> i64 {
    query_i64(&raw(data_dir), &format!("SELECT count(*) FROM {table}"))
}

/// The bytes of the store's database file.
pub fn database_bytes(data_dir: &Path) -> Vec<u8> {
    std::fs::read(data_dir.join(DATABASE_FILE)).unwrap()
}

/// An id.
pub fn id(text: &str) -> Id {
    Id::new(text).unwrap()
}

/// The contract timestamp of `ms`.
pub fn ts(ms: i64) -> Timestamp {
    format_timestamp(u64::try_from(ms).unwrap())
}

/// Reads a contract value from JSON, as the contract crate requires.
pub fn contract<T: serde::de::DeserializeOwned>(value: Value) -> T {
    from_value(&value).unwrap()
}

/// Operation metadata for a seeding write by the broker (`broker.seed`, section 4).
pub fn seed_meta(now_ms: i64) -> OpMeta {
    meta("broker.seed", Actor::broker(), now_ms)
}

/// Operation metadata of `operation_type` with no idempotency, cause or correlation.
pub fn meta(operation_type: &'static str, actor: Actor, now_ms: i64) -> OpMeta {
    OpMeta {
        operation_type,
        actor,
        idempotency: None,
        causation_id: None,
        correlation_id: None,
        now_ms,
    }
}

/// Starts run `run_id` at `now_ms` the way recovery does: `broker.started`, idempotency
/// `startup`/`<run id>` (section 4).
pub fn start_run(store: &mut Store, run_id: i64, now_ms: i64) {
    let mut m = meta("broker.started", Actor::broker(), now_ms);
    m.idempotency = Some((IdempotencyScope::Startup, run_id.to_string()));
    apply_all(
        store,
        m,
        vec![Mutation::RunStarted {
            run_id,
            at_ms: now_ms,
        }],
    )
    .unwrap_or_else(|e| panic!("run {run_id} did not start: {e:?}"));
}

/// Applies `mutations` in one write.
pub fn apply_all(
    store: &mut Store,
    meta: OpMeta,
    mutations: Vec<Mutation>,
) -> Result<Committed<()>, TxError<()>> {
    store.write(meta, |tx: &mut WriteTx<'_>| {
        for mutation in mutations {
            tx.apply(mutation)?;
        }
        Ok(())
    })
}

/// A durable agent row.
pub fn agent_row(agent_id: &str, revision: u64, name: &str) -> AgentRow {
    AgentRow {
        agent_id: id(agent_id),
        revision,
        registered_name: Label::new(name).unwrap(),
        destination: contract(json!({"kind": "cloud", "processor": "Anthropic"})),
        paused: false,
        enrolled_at_ms: T0,
    }
}

/// Writes a new agent (revision 1) in one `broker.seed` write.
pub fn seed_agent(store: &mut Store, agent_id: &str, name: &str, now_ms: i64) -> Committed<()> {
    apply_all(
        store,
        seed_meta(now_ms),
        vec![Mutation::AgentPut {
            agent: agent_row(agent_id, 1, name),
        }],
    )
    .unwrap_or_else(|e| panic!("agent {agent_id} was not written: {e:?}"))
}

/// Whether the store holds the agent `agent_id`.
pub fn has_agent(store: &Store, agent_id: &str) -> bool {
    store.read(|r| r.agent(&id(agent_id))).unwrap().is_some()
}

/// Preferences at `revision` with nothing configured.
pub fn preferences(revision: u64) -> Preferences {
    contract(json!({
        "revision": revision,
        "settings": {
            "zone": null,
            "workday": null,
            "protected_times": [],
            "uninterrupted": null,
            "buffer": null,
            "warning_calendars": []
        }
    }))
}

/// The owner notice change for `preferences`.
pub fn preferences_change(preferences: &Preferences) -> Change {
    Change::Preferences(preferences.clone())
}

/// The ledger head sequence recorded in `meta`.
pub fn head_seq(store: &Store) -> u64 {
    store.read(|r| r.meta()).unwrap().ledger_head_seq
}

/// Verifies the chain and the replay of `store`; returns the head sequence.
pub fn verify(store: &Store) -> u64 {
    let head = store
        .verify_chain()
        .unwrap_or_else(|e| panic!("the chain did not verify: {e} ({e:?})"));
    store
        .verify_replay()
        .unwrap_or_else(|e| panic!("the replay did not verify: {e} ({e:?})"));
    head
}

/// Expects a write to have failed with a store fault, and returns it.
pub fn expect_fault<T: std::fmt::Debug, E: std::fmt::Debug>(
    result: Result<Committed<T>, TxError<E>>,
) -> StoreFault {
    match result {
        Err(TxError::Fault(fault)) => fault,
        other => panic!("expected a store fault, got {other:?}"),
    }
}
