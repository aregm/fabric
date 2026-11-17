//! The gate's life with a live Isabelle server: starting from the theory alone, stale check
//! theories, time limits, a broken gate, and bounded stops. It needs Isabelle from
//! `scripts/setup-demo.sh`, so it is ignored by default:
//!
//!   sh scripts/run.sh cargo test -p fabric-broker-policy --test gate_live -- --ignored --nocapture
//!
//! The tests use an Isabelle user home of their own under `.local/gate-live`, so they neither
//! rebuild the repository's `Fabric_Broker` heap (a heap is rebuilt when its theory is loaded
//! from another directory) nor share the repository's server registry. The first run builds
//! the `Fabric_Broker` heap there, which takes about half a minute; HOL comes with Isabelle.
//!
//! Unix only: process groups, file modes and `ps` are what these tests look at.

#![cfg(unix)]

mod common;

use std::fs;
use std::io;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use fabric_broker_policy::gate::{CHECK_GRACE, SESSION};
use fabric_broker_policy::{
    BrokerGate, Fault, GateConfig, PolicyError, Verdict, built_theory_sha256,
};
use fabric_policy::server::{Server, ServerOptions};

/// The live tests run one at a time: they share one Isabelle user home.
static LIVE: Mutex<()> = Mutex::new(());

fn repository() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives two levels below the repository root")
}

fn local() -> PathBuf {
    repository().join(".local").join("gate-live")
}

/// A rules directory that holds only `Fabric_Broker/ROOT` and `Fabric_Broker/Fabric_Broker.thy`:
/// no `ROOTS` and no other session. Its path stays the same from run to run, so the heap
/// built for it is reused.
fn rules_dir() -> PathBuf {
    let rules = local().join("rules");
    let session = rules.join(SESSION);
    let _ = fs::remove_dir_all(&rules);
    fs::create_dir_all(&session).unwrap();
    for file in ["ROOT", "Fabric_Broker.thy"] {
        fs::copy(
            repository().join("isabelle").join(SESSION).join(file),
            session.join(file),
        )
        .unwrap();
    }
    rules
}

fn config(name: &str) -> GateConfig {
    let mut config = GateConfig::for_repository(repository());
    config.user_home = local().join("home");
    config.rules_dir = rules_dir();
    config.work_dir = local().join(name);
    config.server_name = format!("fabric-gate-work-{name}");
    config
}

/// Starts a gate, retrying once after a few seconds when Isabelle's server registry is busy.
fn start(config: &GateConfig) -> BrokerGate {
    let started = Instant::now();
    let gate = BrokerGate::start(config.clone()).or_else(|error| {
        eprintln!("  the first start failed ({error}); retrying once");
        thread::sleep(Duration::from_secs(5));
        BrokerGate::start(config.clone())
    });
    let gate = gate.expect("the gate starts");
    eprintln!(
        "  gate {} started in {:.1}s on port {}",
        config.server_name,
        started.elapsed().as_secs_f64(),
        gate.port()
    );
    gate
}

fn listening(port: u16) -> bool {
    TcpStream::connect(("127.0.0.1", port)).is_ok()
}

/// Every process as `(pid, ppid, line)`, from `ps`.
fn processes() -> Vec<(u32, u32, String)> {
    let output = Command::new("ps")
        .args(["-A", "-o", "pid=,ppid=,pgid=,args="])
        .output()
        .expect("ps runs");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let ppid = fields.next()?.parse().ok()?;
            Some((pid, ppid, line.trim().to_owned()))
        })
        .collect()
}

/// The processes of the named server while it runs: the `isabelle` wrapper that runs Java
/// (its arguments name the server) and every process below it, as `(pid, "pid ppid pgid command")`.
fn server_processes(name: &str) -> Vec<(u32, String)> {
    let rows = processes();
    let mut found: Vec<u32> = rows
        .iter()
        .filter(|(_, _, line)| line.contains("/lib/Tools/java ") && line.contains(name))
        .map(|(pid, _, _)| *pid)
        .collect();
    let mut next = 0;
    while next < found.len() {
        let parent = found[next];
        for (pid, ppid, _) in &rows {
            if *ppid == parent && !found.contains(pid) {
                found.push(*pid);
            }
        }
        next += 1;
    }
    rows.into_iter()
        .filter(|(pid, _, _)| found.contains(pid))
        .map(|(pid, _, line)| (pid, shorten(&line)))
        .collect()
}

fn shorten(line: &str) -> String {
    match line.char_indices().nth(150) {
        Some((end, _)) => format!("{}...", &line[..end]),
        None => line.to_owned(),
    }
}

/// Those of `seen` that still run.
fn still_running(seen: &[(u32, String)]) -> Vec<String> {
    let rows = processes();
    seen.iter()
        .filter(|(pid, _)| rows.iter().any(|(row, _, _)| row == pid))
        .map(|(_, line)| line.clone())
        .collect()
}

/// Waits up to two seconds for every process of `seen` to be gone, and returns any left.
fn processes_left(seen: &[(u32, String)]) -> Vec<String> {
    let started = Instant::now();
    loop {
        let left = still_running(seen);
        if left.is_empty() || started.elapsed() > Duration::from_secs(2) {
            return left;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[ignore = "needs Isabelle from scripts/setup-demo.sh; run with --ignored"]
fn a_gate_starts_from_the_theory_alone_and_stops_within_its_bound() {
    let _live = LIVE.lock().unwrap_or_else(PoisonError::into_inner);
    let config = config("start");
    fs::create_dir_all(&config.work_dir).unwrap();
    // A check theory a killed process left behind holds calendar data.
    let stale = config.work_dir.join("Check_999999.thy");
    fs::write(&stale, "calendar data").unwrap();

    let mut gate = start(&config);
    assert!(!stale.exists(), "the stale check theory is removed");
    let mut entries: Vec<String> = fs::read_dir(config.rules_dir.join(SESSION))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    entries.sort();
    assert_eq!(
        entries,
        ["Fabric_Broker.thy", "ROOT"],
        "nothing more is needed"
    );
    assert!(!config.rules_dir.join("ROOTS").exists());
    assert_eq!(gate.theory_sha256(), built_theory_sha256());

    // Isabelle keeps its temporary files in the user home, in a directory only its owner
    // can enter, and said so before the server started.
    let home_user = config.user_home.join(".isabelle").join("Isabelle2025-2");
    let settings = fs::read_to_string(home_user.join("etc").join("settings")).unwrap();
    assert!(
        settings
            .lines()
            .any(|line| line == "ISABELLE_TMP_PREFIX=\"$ISABELLE_HOME_USER/tmp\""),
        "{settings}"
    );
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(home_user.join("tmp"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
    }
    let started = Instant::now();
    let prefix = fabric_policy::server::getenv(
        &config.isabelle_home,
        &config.user_home,
        "ISABELLE_TMP_PREFIX",
    )
    .unwrap();
    eprintln!(
        "  isabelle getenv ISABELLE_TMP_PREFIX took {:.3}s: {prefix}",
        started.elapsed().as_secs_f64()
    );

    let check = gate
        .check(&common::context(), &common::action())
        .expect("the check runs");
    assert!(check.verdict.is_permit(), "{:?}", check.verdict);
    assert_eq!(check.theory_sha256, built_theory_sha256());

    let running = server_processes(&config.server_name);
    eprintln!("  server processes while running (pid ppid pgid command):");
    for (_, line) in &running {
        eprintln!("    {line}");
    }
    assert!(!running.is_empty(), "the server's Java process is found");

    let port = gate.port();
    let started = Instant::now();
    gate.stop_within(Duration::from_secs(5))
        .expect("an idle gate stops within five seconds");
    eprintln!(
        "  stop_within(5s) returned Ok after {:.3}s",
        started.elapsed().as_secs_f64()
    );
    assert!(!listening(port), "nothing listens on port {port}");
    let at_once = still_running(&running);
    eprintln!("  processes right after the stop: {at_once:?}");
    let left = processes_left(&running);
    assert!(left.is_empty(), "server processes left: {left:?}");
    // A second stop has nothing to do.
    gate.stop().expect("a stopped gate stops at once");
}

#[test]
#[ignore = "needs Isabelle from scripts/setup-demo.sh; run with --ignored"]
fn a_check_with_a_short_limit_is_undecided_and_the_next_is_decided() {
    let _live = LIVE.lock().unwrap_or_else(PoisonError::into_inner);
    let mut gate = start(&config("limit"));
    let limit = Duration::from_millis(50);
    for round in 0..3 {
        let started = Instant::now();
        let check = gate
            .check_within(&common::context(), &common::action(), limit)
            .expect("the check returns");
        let returned = started.elapsed();
        eprintln!(
            "  round {round}: 50 ms check returned after {:.3}s: {:?}",
            returned.as_secs_f64(),
            check.verdict
        );
        assert!(matches!(check.verdict, Verdict::Undecided { .. }));
        assert!(returned < limit + CHECK_GRACE, "{returned:?}");
        assert!(!gate.is_broken());

        let started = Instant::now();
        let next = gate
            .check(&common::context(), &common::action())
            .expect("the next check runs");
        eprintln!(
            "  round {round}: the next check returned after {:.3}s: {:?}",
            started.elapsed().as_secs_f64(),
            next.verdict
        );
        assert!(next.verdict.is_permit(), "{:?}", next.verdict);
    }

    // A zero limit is not sent to Isabelle, which would read it as no limit at all.
    let started = Instant::now();
    let zero = gate
        .check_within(&common::context(), &common::action(), Duration::ZERO)
        .expect("the check returns");
    assert!(matches!(zero.verdict, Verdict::Undecided { .. }));
    assert!(started.elapsed() < Duration::from_millis(100));
    gate.stop().expect("the gate stops");
}

#[test]
#[ignore = "needs Isabelle from scripts/setup-demo.sh; run with --ignored"]
fn a_gate_whose_connection_closed_is_broken() {
    let _live = LIVE.lock().unwrap_or_else(PoisonError::into_inner);
    let config = config("broken");
    // The test starts the server itself and connects the gate to it, so it can end the
    // server under the gate.
    fabric_broker_policy::gate::prepare_private_tmp(&config.isabelle_home, &config.user_home)
        .unwrap();
    fs::create_dir_all(&config.work_dir).unwrap();
    let mut server = Server::start_in_group(&ServerOptions {
        isabelle_home: &config.isabelle_home,
        user_home: &config.user_home,
        name: &config.server_name,
        log_dir: &config.work_dir,
    })
    .expect("the server starts");
    let mut gate = BrokerGate::connect(config.clone(), &server.address()).expect("connects");
    let before = gate
        .check(&common::context(), &common::action())
        .expect("the check runs");
    assert!(before.verdict.is_permit());

    server.shutdown();
    let error = gate
        .check(&common::context(), &common::action())
        .expect_err("a check on a closed connection fails");
    eprintln!("  the check after the server ended: {error}");
    assert_eq!(Fault::of(&error), Fault::Gate);
    assert!(gate.is_broken());

    let started = Instant::now();
    let again = gate
        .check(&common::context(), &common::action())
        .expect_err("a broken gate refuses checks");
    assert!(matches!(again, PolicyError::Protocol(_)), "{again}");
    assert_eq!(Fault::of(&again), Fault::Gate);
    assert!(started.elapsed() < Duration::from_millis(50));
    // Someone else's server: the stop only tries to end the session.
    gate.stop_within(Duration::from_secs(2))
        .expect("the stop of a connected gate ignores the session's fate");
    let _ = io::Write::flush(&mut io::stderr());
}

#[test]
#[ignore = "needs Isabelle from scripts/setup-demo.sh; run with --ignored"]
fn a_stop_out_of_time_kills_the_server() {
    let _live = LIVE.lock().unwrap_or_else(PoisonError::into_inner);
    let config = config("kill");
    let mut gate = start(&config);
    let port = gate.port();
    let running = server_processes(&config.server_name);
    assert!(running.len() >= 3, "wrapper, Java and poly: {running:?}");

    let started = Instant::now();
    let error = gate
        .stop_within(Duration::ZERO)
        .expect_err("a server cannot stop in no time");
    eprintln!(
        "  stop_within(0) after {:.3}s: {error}",
        started.elapsed().as_secs_f64()
    );
    assert!(error.to_string().contains("killed"), "{error}");
    assert!(!listening(port), "nothing listens on port {port}");
    // Isabelle runs `poly` in a process group of its own; the kill reaches it too, so
    // nothing is left even at once.
    let at_once = still_running(&running);
    eprintln!("  processes right after the kill: {at_once:?}");
    let left = processes_left(&running);
    eprintln!("  processes left after the group kill: {left:?}");
    assert!(left.is_empty(), "server processes left: {left:?}");
}
