//! s15: crash tests of the store (design 7.2, 9.2), and the lock after a killed process (s06).
//! Its own harness (`harness = false`): it starts copies of itself as child processes, which
//! open the store and stop inside a write (or after one) until they abort or are killed. The
//! parent then reopens the store: an unfinished write left nothing, a committed one survived,
//! and the chain and the replay verify.
//!
//! Arguments follow the libtest conventions the runner passes: `--list`, `--exact`,
//! `--skip <filter>`, `--ignored` (there are no ignored tests here), and name filters.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use common::{
    T0, TempDir, agent_row, has_agent, head_seq, make_data_dir, new_store, open, query_i64, raw,
    seed_agent, seed_meta, unavailable, verify,
};
use fabric_storage::{
    Migration, Mutation, OpenError, Store, TestKey, TxError, Unavailable, WriteTx,
};

/// The environment variable that turns this binary into a child, and names what it does.
const CHILD_MODE: &str = "FABRIC_STORAGE_CRASH_CHILD";
/// The data directory a child opens.
const CHILD_DIR: &str = "FABRIC_STORAGE_CRASH_DIR";
/// How long the parent waits for a child's line.
const CHILD_TIMEOUT: Duration = Duration::from_secs(120);

/// The agent a child writes.
const CRASH_AGENT: &str = "agent-crash";

type Test = (&'static str, fn());

const TESTS: [Test; 5] = [
    (
        "an_abort_inside_a_write_leaves_nothing",
        an_abort_inside_a_write_leaves_nothing,
    ),
    (
        "a_sigkill_inside_a_write_leaves_nothing",
        a_sigkill_inside_a_write_leaves_nothing,
    ),
    (
        "a_committed_write_survives_sigkill",
        a_committed_write_survives_sigkill,
    ),
    (
        "a_store_left_without_tables_is_created_on_the_next_open",
        a_store_left_without_tables_is_created_on_the_next_open,
    ),
    (
        "the_lock_is_free_after_its_process_is_killed",
        the_lock_is_free_after_its_process_is_killed,
    ),
];

fn main() -> ExitCode {
    if let Ok(mode) = std::env::var(CHILD_MODE) {
        let dir = PathBuf::from(std::env::var_os(CHILD_DIR).expect("a child needs its directory"));
        child(&mode, &dir);
    }

    let mut list = false;
    let mut exact = false;
    let mut ignored = false;
    let mut filters = Vec::new();
    let mut skips = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--list" => list = true,
            "--exact" => exact = true,
            "--ignored" => ignored = true,
            "--skip" => skips.extend(args.next()),
            "--test-threads" | "--format" | "--color" | "--logfile" | "-Z" => {
                args.next();
            }
            other if other.starts_with('-') => {}
            other => filters.push(other.to_owned()),
        }
    }
    let selected = |name: &str| {
        let matches = |f: &String| {
            if exact {
                name == f
            } else {
                name.contains(f.as_str())
            }
        };
        (filters.is_empty() || filters.iter().any(matches)) && !skips.iter().any(matches)
    };
    let tests: Vec<&Test> = TESTS.iter().filter(|(name, _)| selected(name)).collect();

    if list {
        if !ignored {
            for (name, _) in &tests {
                println!("{name}: test");
            }
        }
        return ExitCode::SUCCESS;
    }
    if ignored {
        println!("\nrunning 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored\n");
        return ExitCode::SUCCESS;
    }

    println!("\nrunning {} tests", tests.len());
    let mut failed = Vec::new();
    for (name, test) in &tests {
        let result = std::panic::catch_unwind(*test);
        if result.is_ok() {
            println!("test {name} ... ok");
        } else {
            println!("test {name} ... FAILED");
            failed.push(*name);
        }
    }
    let passed = tests.len() - failed.len();
    if failed.is_empty() {
        println!("\ntest result: ok. {passed} passed; 0 failed\n");
        ExitCode::SUCCESS
    } else {
        println!("\nfailures:\n    {}", failed.join("\n    "));
        println!(
            "\ntest result: FAILED. {passed} passed; {} failed\n",
            failed.len()
        );
        ExitCode::FAILURE
    }
}

/// Prints `line` for the parent and flushes it.
fn say(line: &str) {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{line}").unwrap();
    out.flush().unwrap();
}

/// Waits until killed.
fn stall() -> ! {
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

/// A child: opens the store in `dir` and does what `mode` says. It never returns.
fn child(mode: &str, dir: &Path) -> ! {
    let mut store = open(dir, T0 + 5_000);
    match mode {
        "stall-in-write" | "abort-in-write" => {
            let abort = mode == "abort-in-write";
            let _ = store.write(
                seed_meta(T0 + 5_010),
                |tx: &mut WriteTx<'_>| -> Result<(), TxError<()>> {
                    tx.apply(Mutation::AgentPut {
                        agent: agent_row(CRASH_AGENT, 1, "Crash"),
                    })?;
                    say("STALLED");
                    if abort {
                        std::process::abort();
                    }
                    stall()
                },
            );
            unreachable!("the write in a crash child never ends");
        }
        "commit-then-stall" => {
            seed_agent(&mut store, CRASH_AGENT, "Crash", T0 + 5_010);
            say("COMMITTED");
            stall();
        }
        "hold-lock" => {
            say("OPEN");
            stall();
        }
        other => panic!("unknown child mode {other}"),
    }
}

/// A running child and the lines it prints.
struct Running {
    child: Child,
    lines: mpsc::Receiver<String>,
}

impl Running {
    fn start(mode: &str, data_dir: &Path) -> Running {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .env(CHILD_MODE, mode)
            .env(CHILD_DIR, data_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Running { child, lines }
    }

    /// Waits for the child to print `expected`.
    fn wait_for(&mut self, expected: &str) {
        loop {
            match self.lines.recv_timeout(CHILD_TIMEOUT) {
                Ok(line) if line == expected => return,
                Ok(_) => {}
                Err(e) => {
                    let _ = self.child.kill();
                    panic!("the child did not print {expected}: {e}");
                }
            }
        }
    }

    /// Kills the child (SIGKILL on Unix) and reaps it.
    fn kill(mut self) {
        self.child.kill().unwrap();
        let status = self.child.wait().unwrap();
        assert!(!status.success());
    }

    /// Waits for a child that ends by itself; it must not end successfully.
    fn wait_for_crash(mut self) {
        let status = self.child.wait().unwrap();
        assert!(!status.success(), "the child exited successfully: {status}");
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A closed store with run 1 and one agent; returns its head.
fn closed_store(data_dir: &Path) -> u64 {
    let mut store = new_store(data_dir);
    seed_agent(&mut store, "agent-before", "Before", T0 + 10);
    let head = head_seq(&store);
    store.close().unwrap();
    head
}

/// Opens the store with the test key after the process holding it ended. Windows releases a
/// killed process's file lock a short, unspecified time after the process ends, so a reopen at
/// once can meet `InUse` there: it is retried while the answer is `InUse`, for up to two
/// seconds. Any other answer is final at once.
fn open_after_kill(data_dir: &Path, now_ms: i64) -> Store {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match Store::open(common::options(data_dir, TestKey, now_ms)) {
            Ok(store) => return store,
            Err(OpenError::Unavailable(Unavailable::InUse)) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("the store did not open: {e} ({e:?})"),
        }
    }
}

/// After an unfinished write: the store opens, holds no trace of the write, and verifies.
fn assert_nothing_written(data_dir: &Path, head: u64) {
    let store = open_after_kill(data_dir, T0 + 10_000);
    assert!(has_agent(&store, "agent-before"));
    assert!(
        !has_agent(&store, CRASH_AGENT),
        "the unfinished write left its agent"
    );
    assert_eq!(head_seq(&store), head);
    assert_eq!(verify(&store), head);
}

/// s15 (7.2): a process that aborts inside a write leaves nothing of it.
fn an_abort_inside_a_write_leaves_nothing() {
    let dir = TempDir::new("s15-abort");
    let data = dir.data_dir();
    let head = closed_store(&data);
    let mut running = Running::start("abort-in-write", &data);
    running.wait_for("STALLED");
    running.wait_for_crash();
    assert_nothing_written(&data, head);
}

/// s15 (7.2): a process killed inside a write leaves nothing of it.
fn a_sigkill_inside_a_write_leaves_nothing() {
    let dir = TempDir::new("s15-kill");
    let data = dir.data_dir();
    let head = closed_store(&data);
    let mut running = Running::start("stall-in-write", &data);
    running.wait_for("STALLED");
    running.kill();
    assert_nothing_written(&data, head);
}

/// s15 (7.1, 7.2; 3.1 `synchronous = FULL`): a write that committed survives the process being
/// killed right after, with its ledger row, and the chain verifies.
fn a_committed_write_survives_sigkill() {
    let dir = TempDir::new("s15-committed");
    let data = dir.data_dir();
    let head = closed_store(&data);
    let mut running = Running::start("commit-then-stall", &data);
    running.wait_for("COMMITTED");
    running.kill();

    let store = open_after_kill(&data, T0 + 10_000);
    assert!(
        has_agent(&store, CRASH_AGENT),
        "the committed write was lost"
    );
    assert_eq!(head_seq(&store), head + 1);
    assert_eq!(verify(&store), head + 1);
}

/// A migration list whose migration 1 fails part-way.
static BROKEN: [Migration; 1] = [Migration {
    to: 1,
    sql: "CREATE TABLE half_done (x INTEGER) STRICT; CREATE TABLE half_done (x INTEGER) STRICT;",
}];

/// The store at `data_dir` opens as a new, verified version-1 store.
fn assert_created(data_dir: &Path) {
    let store = open(data_dir, T0 + 10_000);
    let meta = store.read(|r| r.meta()).unwrap();
    assert_eq!(meta.schema_version, 1);
    assert_eq!(meta.key_kind, fabric_storage::KeyKind::Test);
    verify(&store);
    store.close().unwrap();
}

/// s15 (3.4 step 7, 3.6): a file with no tables, left by a crash between
/// opening the connection and the first migration's commit, is treated as not created: the
/// next open runs migration 1. Both setups of 7.2: a raw keyed connection that creates
/// `broker.db` and closes, and a first open whose migration 1 fails.
fn a_store_left_without_tables_is_created_on_the_next_open() {
    // A raw keyed connection creates the file (in WAL mode, as the store's own would) and closes.
    let dir = TempDir::new("s15-no-tables-raw");
    let data = dir.data_dir();
    make_data_dir(&data);
    let connection = raw(&data);
    let mode: String = connection
        .query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    drop(connection);
    assert!(data.join(fabric_storage::DATABASE_FILE).exists());
    assert_created(&data);

    // A first open whose migration 1 fails rolls back to a file with no tables.
    let dir = TempDir::new("s15-no-tables-failed");
    let data = dir.data_dir();
    let options = common::options(&data, TestKey, T0);
    assert!(
        Store::open_with_migrations(options, &BROKEN).is_err(),
        "a failing migration 1 opened the store"
    );
    let connection = raw(&data);
    assert_eq!(
        query_i64(&connection, "SELECT count(*) FROM sqlite_schema"),
        0
    );
    assert_eq!(query_i64(&connection, "PRAGMA user_version"), 0);
    drop(connection);
    assert_created(&data);
}

/// s06 (3.3): `broker.lock` is released by the operating system when its process dies: while
/// a child holds the store a second open is `InUse`; after the child is killed it opens.
fn the_lock_is_free_after_its_process_is_killed() {
    let dir = TempDir::new("s06-killed");
    let data = dir.data_dir();
    let head = closed_store(&data);
    let mut running = Running::start("hold-lock", &data);
    running.wait_for("OPEN");
    assert_eq!(
        unavailable(common::options(&data, TestKey, T0 + 6_000)),
        Unavailable::InUse
    );
    running.kill();

    let store = open_after_kill(&data, T0 + 10_000);
    assert_eq!(verify(&store), head);
}
