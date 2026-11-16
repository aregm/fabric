//! Process tests of the relay, with a test target that is its own broker child.
//!
//! Run without the variable `FABRIC_RELAY_TEST_CHILD`, `main` runs the cases. Each case starts
//! this same executable as the broker child through `BrokerConfig` and `Session`, with
//! `FABRIC_RELAY_TEST_CHILD` set to the mode the child plays. The relay starts a broker with no
//! arguments, so the mode travels in the environment, through the allowlist of the test (the
//! product's `ENVIRONMENT_ALLOWLIST` does not hold it).
//!
//! The output imitates Cargo's harness: one line per case and a final `test result:` line.

use std::ffi::OsString;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use fabric_desktop_contracts::{Frame, Layer, MAX_FRAME_BYTES, canonical, parse_strict};
use fabric_desktop_relay::{
    BROKER_ISABELLE_HOME, BROKER_ISABELLE_USER_HOME, BROKER_RULES_DIR, BrokerConfig, ConfigError,
    ENVIRONMENT_ALLOWLIST, Ending, Event, InstanceError, InstanceLock, QUIT_DRAIN, SendError,
    Session,
};

/// The mode the re-executed child plays.
const MODE: &str = "FABRIC_RELAY_TEST_CHILD";
/// A file the child writes what it saw to.
const OUT: &str = "FABRIC_RELAY_TEST_OUT";
/// A variable the environment case allowlists.
const ALLOWED: &str = "FABRIC_RELAY_TEST_ALLOWED";
/// A variable the environment case sets and does not allowlist.
const SECRET: &str = "FABRIC_RELAY_TEST_SECRET";

/// How long a case waits for an event before it fails.
const EVENT_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a scripted child that waits to be killed lives at most, so none outlives a crashed
/// test run for long.
const CHILD_LIFETIME: Duration = Duration::from_secs(30);

const REQUEST_HELLO: &str = r#"{"id":"hello-1","method":"hello","params":{"client":{"build":"0.1.0-dev+20261115","name":"Fabric Calendar"},"schema_version":1},"protocol":"fabric-desktop/1"}"#;
const REQUEST_SNAPSHOT: &str =
    r#"{"id":"snapshot-1","method":"get_snapshot","params":{},"protocol":"fabric-desktop/1"}"#;
const REQUEST_SUBSCRIBE: &str = r#"{"id":"subscribe-1","method":"subscribe","params":{"cursor":0},"protocol":"fabric-desktop/1"}"#;

fn main() {
    if let Some(mode) = std::env::var_os(MODE) {
        child(&mode.to_string_lossy());
    }
    std::process::exit(run_cases());
}

// ---------------------------------------------------------------------------------------------
// The scripted child.

fn child(mode: &str) -> ! {
    match mode {
        "echo" => child_echo(),
        "env" => child_env(),
        "garbage" => {
            write_out(&notice(1));
            write_out("this is not a frame\n");
            write_out(&notice(2));
            linger()
        }
        "oversized" => {
            write_out(&padded_notice(1, MAX_FRAME_BYTES));
            write_out(&padded_notice(2, MAX_FRAME_BYTES + 1));
            write_out(&notice(3));
            linger()
        }
        "exit" => std::process::exit(3),
        "deaf" => {
            // Reads to the end of input, then ignores it.
            let _ = std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink());
            linger()
        }
        "sleep" => linger(),
        other => {
            eprintln!("relay test child: unknown mode {other}");
            std::process::exit(70)
        }
    }
}

/// Answers every request line with an `internal` error response that echoes its id and method,
/// then a `snapshot_required` notice with the next sequence. Appends each line it received, as
/// received, to the file named by `FABRIC_RELAY_TEST_OUT`. Exits 0 at the end of input.
fn child_echo() -> ! {
    let mut record = std::env::var_os(OUT).map(|path| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("open the record file")
    });
    let mut sequence = 0u64;
    for line in std::io::stdin().lock().split(b'\n') {
        let Ok(line) = line else { break };
        if let Some(file) = record.as_mut() {
            file.write_all(&line).expect("record");
            file.write_all(b"\n").expect("record");
            file.flush().expect("record");
        }
        let value = parse_strict(&line).expect("the relay sends strict JSON");
        sequence += 1;
        write_out(&response(
            &canonical(&value["id"]),
            &canonical(&value["method"]),
        ));
        write_out(&notice(sequence));
    }
    std::process::exit(0)
}

/// Writes the names of its environment, one per line, to the record file, and exits 0. `PATH`
/// is written with its value, as `PATH=<value>`, so the test can see it is the relay's fixed one.
fn child_env() -> ! {
    let path = std::env::var_os(OUT).expect("the record file is allowlisted");
    let mut text = String::new();
    for (name, value) in std::env::vars_os() {
        text.push_str(&name.to_string_lossy());
        if name == "PATH" {
            text.push('=');
            text.push_str(&value.to_string_lossy());
        }
        text.push('\n');
    }
    std::fs::write(path, text).expect("write the record file");
    std::process::exit(0)
}

fn write_out(text: &str) {
    let mut stdout = std::io::stdout().lock();
    if stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
        .is_err()
    {
        std::process::exit(71);
    }
}

fn linger() -> ! {
    std::thread::sleep(CHILD_LIFETIME);
    std::process::exit(72)
}

// ---------------------------------------------------------------------------------------------
// Frames the child writes.

fn response(id_json: &str, method_json: &str) -> String {
    format!(
        r#"{{"error":{{"code":"internal","correlation_id":null,"detail":null,"message":"Something went wrong in the broker.","recovery":"report_problem","retryable":false}},"id":{id_json},"method":{method_json},"protocol":"fabric-desktop/1"}}"#
    ) + "\n"
}

fn notice(sequence: u64) -> String {
    format!(
        r#"{{"change":{{"kind":"snapshot_required"}},"protocol":"fabric-desktop/1","sequence":{sequence}}}"#
    ) + "\n"
}

/// A valid notice padded with spaces before its closing brace to `length` bytes, plus the line
/// feed.
fn padded_notice(sequence: u64, length: usize) -> String {
    let mut text = notice(sequence);
    text.pop(); // the line feed
    text.pop(); // the closing brace
    let padding = length - text.len() - 1;
    text.extend(std::iter::repeat_n(' ', padding));
    text.push_str("}\n");
    text
}

// ---------------------------------------------------------------------------------------------
// The cases.

type Outcome = Result<(), String>;
type Case = (&'static str, fn(&Path) -> Outcome);

macro_rules! ensure {
    ($cond:expr, $($message:tt)+) => {
        if !$cond {
            return Err(format!($($message)+));
        }
    };
}

fn run_cases() -> i32 {
    let mut cases: Vec<Case> = vec![
        (
            "environment_holds_only_allowlisted_names",
            environment_holds_only_allowlisted_names,
        ),
        (
            "configuration_errors_start_nothing",
            configuration_errors_start_nothing,
        ),
        (
            "refused_request_is_not_written",
            refused_request_is_not_written,
        ),
        (
            "valid_request_reaches_child_canonical_on_one_line",
            valid_request_reaches_child_canonical_on_one_line,
        ),
        ("frames_come_back_in_order", frames_come_back_in_order),
        (
            "line_bound_is_max_frame_bytes",
            line_bound_is_max_frame_bytes,
        ),
        (
            "invalid_frame_kills_the_child",
            invalid_frame_kills_the_child,
        ),
        ("exit_code_is_reported", exit_code_is_reported),
        ("stop_with_clean_exit", stop_with_clean_exit),
        ("stop_with_kill_after_drain", stop_with_kill_after_drain),
        (
            "stop_is_idempotent_and_ended_comes_once",
            stop_is_idempotent_and_ended_comes_once,
        ),
        ("stop_from_inside_the_sink", stop_from_inside_the_sink),
        (
            "send_after_end_is_not_running",
            send_after_end_is_not_running,
        ),
        (
            "stop_kills_a_child_that_does_not_read",
            stop_kills_a_child_that_does_not_read,
        ),
        ("drop_stops_the_child", drop_stops_the_child),
        ("program_path_with_spaces", program_path_with_spaces),
        (
            "instance_lock_held_and_released",
            instance_lock_held_and_released,
        ),
        (
            "prover_names_are_exported_and_allowlisted",
            prover_names_are_exported_and_allowlisted,
        ),
    ];
    cases.extend(platform_cases());

    let root = std::env::temp_dir().join(format!(
        "fabric-desktop-relay-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let started = Instant::now();
    println!();
    println!("running {} tests", cases.len());
    let mut failures = Vec::new();
    for (index, (name, case)) in cases.iter().enumerate() {
        let directory = root.join(format!("case-{index}"));
        let outcome = std::fs::create_dir_all(&directory)
            .map_err(|e| format!("creating {}: {e}", directory.display()))
            .and_then(|()| {
                std::panic::catch_unwind(|| case(&directory))
                    .unwrap_or_else(|panic| Err(format!("panicked: {}", panic_text(&*panic))))
            });
        match outcome {
            Ok(()) => println!("test {name} ... ok"),
            Err(message) => {
                println!("test {name} ... FAILED");
                failures.push((*name, message));
            }
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    if !failures.is_empty() {
        println!();
        println!("failures:");
        for (name, message) in &failures {
            println!("    {name}: {message}");
        }
    }
    let passed = cases.len() - failures.len();
    println!();
    println!(
        "test result: {}. {passed} passed; {} failed; 0 ignored; 0 measured; 0 filtered out; finished in {:.2}s",
        if failures.is_empty() { "ok" } else { "FAILED" },
        failures.len(),
        started.elapsed().as_secs_f64()
    );
    println!();
    i32::from(!failures.is_empty())
}

/// The cases that only run on some platforms.
fn platform_cases() -> Vec<Case> {
    #[cfg(unix)]
    {
        vec![
            (
                "killed_by_signal_from_outside",
                killed_by_signal_from_outside,
            ),
            (
                "closed_stdout_but_alive_is_killed_after_drain",
                closed_stdout_but_alive_is_killed_after_drain,
            ),
        ]
    }
    #[cfg(not(unix))]
    {
        Vec::new()
    }
}

fn panic_text(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = panic.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = panic.downcast_ref::<String>() {
        text.clone()
    } else {
        "a panic with no message".to_owned()
    }
}

// ---------------------------------------------------------------------------------------------
// Helpers.

fn this_executable() -> PathBuf {
    std::env::current_exe().expect("the test executable's path")
}

/// A configuration that starts `program` in `mode`, with `extra` added to the host's
/// environment and the test's names allowlisted on top of the product's.
fn config_for(program: &Path, mode: &str, extra: &[(&str, OsString)]) -> BrokerConfig {
    let mut allowlist: Vec<&str> = ENVIRONMENT_ALLOWLIST.to_vec();
    allowlist.extend([MODE, OUT, ALLOWED]);
    let host = std::env::vars_os()
        .chain([(OsString::from(MODE), OsString::from(mode))])
        .chain(extra.iter().map(|(k, v)| (OsString::from(k), v.clone())));
    BrokerConfig::check(program, &allowlist, host).expect("the test executable is a valid broker")
}

fn start(mode: &str, extra: &[(&str, OsString)]) -> (Session, Receiver<Event>) {
    start_program(&this_executable(), mode, extra)
}

fn start_program(
    program: &Path,
    mode: &str,
    extra: &[(&str, OsString)],
) -> (Session, Receiver<Event>) {
    let (sender, receiver) = mpsc::channel();
    let session = Session::start(&config_for(program, mode, extra), move |event| {
        let _ = sender.send(event);
    })
    .expect("start the child");
    (session, receiver)
}

fn next_event(events: &Receiver<Event>) -> Result<Event, String> {
    events
        .recv_timeout(EVENT_TIMEOUT)
        .map_err(|e| format!("no event: {e}"))
}

/// Reads every remaining event until the reader thread drops the sink, and checks that exactly
/// one of them is `Ended` and that it is the last. Returns the frames before it and the ending.
fn finish(events: &Receiver<Event>) -> Result<(Vec<Frame>, Ending), String> {
    let deadline = Instant::now() + EVENT_TIMEOUT;
    let mut all = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(left) {
            Ok(event) => all.push(event),
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                return Err(format!("the sink was not dropped; events so far: {all:?}"));
            }
        }
    }
    let ended = all.iter().filter(|e| matches!(e, Event::Ended(_))).count();
    ensure!(ended == 1, "Ended came {ended} times: {all:?}");
    let Some(Event::Ended(ending)) = all.pop() else {
        return Err(format!("Ended is not the last event: {all:?}"));
    };
    let frames = all
        .into_iter()
        .map(|e| match e {
            Event::Frame(frame) => frame,
            Event::Ended(_) => unreachable!("counted above"),
        })
        .collect();
    Ok((frames, ending))
}

fn lines_of(frames: &[Frame]) -> Vec<String> {
    frames.iter().map(Frame::to_line).collect()
}

fn path_value(path: &Path) -> OsString {
    path.as_os_str().to_owned()
}

// ---------------------------------------------------------------------------------------------
// Cases.

fn environment_holds_only_allowlisted_names(directory: &Path) -> Outcome {
    let record = directory.join("environment.txt");
    let (_session, events) = start(
        "env",
        &[
            (OUT, path_value(&record)),
            (ALLOWED, OsString::from("yes")),
            (SECRET, OsString::from("must not pass")),
        ],
    );
    let (frames, ending) = finish(&events)?;
    ensure!(frames.is_empty(), "unexpected frames: {frames:?}");
    ensure!(
        matches!(ending, Ending::Exited { code: Some(0), .. }),
        "ending: {ending:?}"
    );
    let text = std::fs::read_to_string(&record).map_err(|e| e.to_string())?;
    let names: Vec<&str> = text.lines().collect();
    let mut allowed: Vec<&str> = ENVIRONMENT_ALLOWLIST.to_vec();
    allowed.extend([MODE, OUT, ALLOWED]);
    let same = |a: &str, b: &str| {
        if cfg!(windows) {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    };
    for name in [MODE, OUT, ALLOWED] {
        ensure!(
            names.iter().any(|n| same(n, name)),
            "{name} is missing in the child: {names:?}"
        );
    }
    ensure!(
        !names.iter().any(|n| same(n, SECRET)),
        "{SECRET} reached the child"
    );
    // On Unix the relay gives the child one fixed PATH, never the host's (the test's own PATH is
    // in the host environment above).
    let fixed_path = "PATH=/usr/bin:/bin:/usr/sbin:/sbin";
    if cfg!(unix) {
        ensure!(
            names.contains(&fixed_path),
            "the child's PATH is not the relay's fixed one: {names:?}"
        );
    }
    for name in &names {
        // Windows may list hidden per-drive variables whose names begin with `=`.
        ensure!(
            name.starts_with('=')
                || allowed.iter().any(|a| same(name, a))
                || (cfg!(unix) && *name == fixed_path),
            "{name} reached the child but is not allowlisted"
        );
    }
    Ok(())
}

fn configuration_errors_start_nothing(directory: &Path) -> Outcome {
    let check = |path: &Path| BrokerConfig::check(path, &[], Vec::new());
    let relative = Path::new("fabric-broker");
    ensure!(
        check(relative) == Err(ConfigError::NotAbsolute(relative.to_path_buf())),
        "relative path"
    );
    let missing = directory.join("no-such-broker");
    ensure!(
        check(&missing) == Err(ConfigError::Missing(missing.clone())),
        "missing path"
    );
    ensure!(
        check(directory) == Err(ConfigError::NotAFile(directory.to_path_buf())),
        "directory"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let plain = directory.join("not-executable");
        std::fs::write(&plain, "#!/bin/sh\nexit 0\n").map_err(|e| e.to_string())?;
        std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644))
            .map_err(|e| e.to_string())?;
        ensure!(
            check(&plain) == Err(ConfigError::NotExecutable(plain.clone())),
            "file without an execute bit"
        );
    }
    Ok(())
}

fn refused_request_is_not_written(directory: &Path) -> Outcome {
    let record = directory.join("received.txt");
    let (session, events) = start("echo", &[(OUT, path_value(&record))]);
    let refusals = [
        (&b"not json"[..], Layer::Frame),
        (
            br#"{"id":"x","method":"get_snapshot","params":{},"protocol":"fabric-desktop/2"}"#,
            Layer::Protocol,
        ),
        (
            br#"{"id":"x","method":"get_snapshot","params":{"extra":1},"protocol":"fabric-desktop/1"}"#,
            Layer::Schema,
        ),
        (
            br#"{"id":"x","method":"launch","params":{},"protocol":"fabric-desktop/1"}"#,
            Layer::Schema,
        ),
        // A response is not a request.
        (
            br#"{"change":{"kind":"snapshot_required"},"protocol":"fabric-desktop/1","sequence":1}"#,
            Layer::Schema,
        ),
    ];
    for (bytes, layer) in refusals {
        match session.send(bytes) {
            Err(SendError::Refused(refusal)) if refusal.layer == layer => {}
            other => return Err(format!("expected a refusal at {layer:?}, got {other:?}")),
        }
    }
    session.stop(QUIT_DRAIN);
    let (frames, ending) = finish(&events)?;
    ensure!(frames.is_empty(), "frames for refused requests: {frames:?}");
    ensure!(
        ending == Ending::Stopped { killed: false },
        "ending: {ending:?}"
    );
    let received = std::fs::read(&record).unwrap_or_default();
    ensure!(
        received.is_empty(),
        "the child received {:?}",
        String::from_utf8_lossy(&received)
    );
    Ok(())
}

fn valid_request_reaches_child_canonical_on_one_line(directory: &Path) -> Outcome {
    let record = directory.join("received.txt");
    let (session, events) = start("echo", &[(OUT, path_value(&record))]);
    // Members out of order, with whitespace, including line feeds between tokens: decode
    // refuses a line feed in a frame, so use spaces and tabs here.
    let sent = b"{ \"protocol\" : \"fabric-desktop/1\",\t\"params\": {}, \"method\": \"get_snapshot\", \"id\": \"snapshot-1\" }";
    session.send(sent).map_err(|e| e.to_string())?;
    next_event(&events)?;
    next_event(&events)?;
    session.stop(QUIT_DRAIN);
    finish(&events)?;
    let received = std::fs::read_to_string(&record).map_err(|e| e.to_string())?;
    ensure!(
        received == format!("{REQUEST_SNAPSHOT}\n"),
        "the child received {received:?}"
    );
    Ok(())
}

fn frames_come_back_in_order(_: &Path) -> Outcome {
    let (session, events) = start("echo", &[]);
    for request in [REQUEST_HELLO, REQUEST_SNAPSHOT, REQUEST_SUBSCRIBE] {
        session
            .send(request.as_bytes())
            .map_err(|e| e.to_string())?;
    }
    let mut frames = Vec::new();
    for _ in 0..6 {
        match next_event(&events)? {
            Event::Frame(frame) => frames.push(frame),
            Event::Ended(ending) => return Err(format!("ended early: {ending:?}")),
        }
    }
    session.stop(QUIT_DRAIN);
    let (rest, ending) = finish(&events)?;
    ensure!(rest.is_empty(), "frames after the six: {rest:?}");
    ensure!(
        ending == Ending::Stopped { killed: false },
        "ending: {ending:?}"
    );
    let expected = vec![
        response(r#""hello-1""#, r#""hello""#),
        notice(1),
        response(r#""snapshot-1""#, r#""get_snapshot""#),
        notice(2),
        response(r#""subscribe-1""#, r#""subscribe""#),
        notice(3),
    ];
    ensure!(
        lines_of(&frames) == expected,
        "frames: {:?}",
        lines_of(&frames)
    );
    ensure!(
        matches!(frames[0], Frame::OwnerResponse(_)) && matches!(frames[1], Frame::OwnerNotice(_)),
        "responses and notices are told apart"
    );
    Ok(())
}

fn line_bound_is_max_frame_bytes(_: &Path) -> Outcome {
    let (_session, events) = start("oversized", &[]);
    let (frames, ending) = finish(&events)?;
    ensure!(
        lines_of(&frames) == vec![notice(1)],
        "only the frame of exactly {MAX_FRAME_BYTES} bytes passes: {} frames",
        frames.len()
    );
    match ending {
        Ending::ProtocolViolation { reason } if reason.contains("longer than") => Ok(()),
        other => Err(format!("ending: {other:?}")),
    }
}

fn invalid_frame_kills_the_child(_: &Path) -> Outcome {
    let started = Instant::now();
    let (session, events) = start("garbage", &[]);
    let (frames, ending) = finish(&events)?;
    ensure!(
        lines_of(&frames) == vec![notice(1)],
        "nothing after the invalid line is forwarded: {:?}",
        lines_of(&frames)
    );
    ensure!(
        matches!(ending, Ending::ProtocolViolation { .. }),
        "ending: {ending:?}"
    );
    // The child lingers for 30 s; ending well before means the relay killed it.
    ensure!(
        started.elapsed() < Duration::from_secs(10),
        "the child was not killed"
    );
    ensure!(
        session.send(REQUEST_SNAPSHOT.as_bytes()) == Err(SendError::NotRunning),
        "send after a violation"
    );
    Ok(())
}

fn exit_code_is_reported(_: &Path) -> Outcome {
    let (_session, events) = start("exit", &[]);
    let (frames, ending) = finish(&events)?;
    ensure!(frames.is_empty(), "frames: {frames:?}");
    ensure!(
        ending
            == Ending::Exited {
                code: Some(3),
                description: "exit code 3".to_owned()
            },
        "ending: {ending:?}"
    );
    Ok(())
}

fn stop_with_clean_exit(_: &Path) -> Outcome {
    let (session, events) = start("echo", &[]);
    session
        .send(REQUEST_HELLO.as_bytes())
        .map_err(|e| e.to_string())?;
    next_event(&events)?;
    session.stop(QUIT_DRAIN);
    ensure!(
        session.send(REQUEST_HELLO.as_bytes()) == Err(SendError::NotRunning),
        "send after stop"
    );
    let (_, ending) = finish(&events)?;
    ensure!(
        ending == Ending::Stopped { killed: false },
        "ending: {ending:?}"
    );
    Ok(())
}

fn stop_with_kill_after_drain(_: &Path) -> Outcome {
    let (session, events) = start("deaf", &[]);
    let drain = Duration::from_millis(300);
    let started = Instant::now();
    session.stop(drain);
    let took = started.elapsed();
    ensure!(took >= drain, "stop returned after {took:?}");
    ensure!(took < Duration::from_secs(10), "stop took {took:?}");
    let (_, ending) = finish(&events)?;
    ensure!(
        ending == Ending::Stopped { killed: true },
        "ending: {ending:?}"
    );
    Ok(())
}

fn stop_is_idempotent_and_ended_comes_once(_: &Path) -> Outcome {
    let (session, events) = start("deaf", &[]);
    let session = Arc::new(session);
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let session = Arc::clone(&session);
            std::thread::spawn(move || session.stop(Duration::from_millis(100)))
        })
        .collect();
    session.stop(Duration::from_millis(100));
    for thread in threads {
        thread.join().map_err(|_| "a stop panicked".to_owned())?;
    }
    session.stop(Duration::ZERO);
    drop(session);
    let (_, ending) = finish(&events)?;
    ensure!(
        ending == Ending::Stopped { killed: true },
        "ending: {ending:?}"
    );
    Ok(())
}

fn stop_from_inside_the_sink(_: &Path) -> Outcome {
    let slot: Arc<OnceLock<Session>> = Arc::new(OnceLock::new());
    let (sender, events) = mpsc::channel();
    let sink_slot = Arc::clone(&slot);
    let session = Session::start(&config_for(&this_executable(), "echo", &[]), move |event| {
        let is_frame = matches!(event, Event::Frame(_));
        let _ = sender.send(event);
        if is_frame && let Some(session) = sink_slot.get() {
            session.stop(QUIT_DRAIN);
        }
    })
    .map_err(|e| e.to_string())?;
    let _ = slot.set(session);
    let session = slot.get().expect("set above");
    session
        .send(REQUEST_HELLO.as_bytes())
        .map_err(|e| e.to_string())?;
    let (frames, ending) = finish(&events)?;
    ensure!(!frames.is_empty(), "no frame before the end");
    ensure!(
        ending == Ending::Stopped { killed: false },
        "ending: {ending:?}"
    );
    Ok(())
}

fn send_after_end_is_not_running(_: &Path) -> Outcome {
    let (session, events) = start("exit", &[]);
    // Sends while the child exits: each must be accepted or refused as not running (or an
    // I/O error), never panic.
    for _ in 0..200 {
        match session.send(REQUEST_SNAPSHOT.as_bytes()) {
            Ok(()) | Err(SendError::NotRunning | SendError::Io(_)) => {}
            Err(other) => return Err(format!("send: {other:?}")),
        }
    }
    finish(&events)?;
    ensure!(
        session.send(REQUEST_SNAPSHOT.as_bytes()) == Err(SendError::NotRunning),
        "send after Ended"
    );
    Ok(())
}

fn stop_kills_a_child_that_does_not_read(_: &Path) -> Outcome {
    let (session, events) = start("sleep", &[]);
    let session = Arc::new(session);
    let writer_session = Arc::clone(&session);
    let (done_sender, done) = mpsc::channel();
    // Fills the pipe until a write blocks, then waits there until the child is killed.
    std::thread::spawn(move || {
        let result = loop {
            if let Err(e) = writer_session.send(REQUEST_HELLO.as_bytes()) {
                break e;
            }
        };
        let _ = done_sender.send(result);
    });
    std::thread::sleep(Duration::from_millis(200));
    session.stop(Duration::from_millis(100));
    let result = done
        .recv_timeout(EVENT_TIMEOUT)
        .map_err(|_| "the blocked send never returned".to_owned())?;
    ensure!(
        matches!(result, SendError::NotRunning | SendError::Io(_)),
        "the blocked send ended with {result:?}"
    );
    let (_, ending) = finish(&events)?;
    ensure!(
        ending == Ending::Stopped { killed: true },
        "ending: {ending:?}"
    );
    Ok(())
}

fn drop_stops_the_child(_: &Path) -> Outcome {
    let (session, events) = start("echo", &[]);
    drop(session);
    let (_, ending) = finish(&events)?;
    ensure!(
        ending == Ending::Stopped { killed: false },
        "ending: {ending:?}"
    );
    Ok(())
}

fn program_path_with_spaces(directory: &Path) -> Outcome {
    let spaced = directory.join("a directory with spaces");
    std::fs::create_dir_all(&spaced).map_err(|e| e.to_string())?;
    let program = spaced.join(format!("relay child{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(this_executable(), &program).map_err(|e| e.to_string())?;
    let (session, events) = start_program(&program, "echo", &[]);
    session
        .send(REQUEST_HELLO.as_bytes())
        .map_err(|e| e.to_string())?;
    let first = next_event(&events)?;
    ensure!(
        matches!(&first, Event::Frame(frame) if frame.to_line() == response(r#""hello-1""#, r#""hello""#)),
        "first event: {first:?}"
    );
    session.stop(QUIT_DRAIN);
    let (_, ending) = finish(&events)?;
    ensure!(
        ending == Ending::Stopped { killed: false },
        "ending: {ending:?}"
    );
    Ok(())
}

/// The host takes the three prover names from the relay's crate root instead of spelling them
/// again, so the root exports them, with the broker's spelling, and the allowlist holds them.
fn prover_names_are_exported_and_allowlisted(_: &Path) -> Outcome {
    let names = [
        BROKER_ISABELLE_HOME,
        BROKER_ISABELLE_USER_HOME,
        BROKER_RULES_DIR,
    ];
    ensure!(
        names
            == [
                "FABRIC_BROKER_ISABELLE_HOME",
                "FABRIC_BROKER_ISABELLE_USER_HOME",
                "FABRIC_BROKER_RULES_DIR",
            ],
        "the exported prover names are {names:?}"
    );
    for name in names {
        ensure!(
            ENVIRONMENT_ALLOWLIST.contains(&name),
            "{name} is not in the allowlist"
        );
    }
    Ok(())
}

fn instance_lock_held_and_released(directory: &Path) -> Outcome {
    let lock_directory = directory.join("data").join("instance");
    let first = InstanceLock::acquire(&lock_directory).map_err(|e| e.to_string())?;
    ensure!(
        lock_directory.join("instance.lock").is_file(),
        "no lock file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&lock_directory)
            .map_err(|e| e.to_string())?
            .permissions()
            .mode();
        ensure!(mode & 0o777 == 0o700, "directory mode {:o}", mode & 0o777);
    }
    match InstanceLock::acquire(&lock_directory) {
        Err(InstanceError::Held) => {}
        other => return Err(format!("second acquire: {other:?}")),
    }
    drop(first);
    let again = InstanceLock::acquire(&lock_directory).map_err(|e| e.to_string())?;
    drop(again);
    Ok(())
}

#[cfg(unix)]
fn killed_by_signal_from_outside(_: &Path) -> Outcome {
    let (session, events) = start("echo", &[]);
    let status = std::process::Command::new("kill")
        .args(["-9", &session.pid().to_string()])
        .status()
        .map_err(|e| e.to_string())?;
    ensure!(status.success(), "kill failed: {status}");
    let (_, ending) = finish(&events)?;
    ensure!(
        ending
            == Ending::Exited {
                code: None,
                description: "killed by signal 9".to_owned()
            },
        "ending: {ending:?}"
    );
    Ok(())
}

#[cfg(unix)]
fn closed_stdout_but_alive_is_killed_after_drain(directory: &Path) -> Outcome {
    use std::os::unix::fs::PermissionsExt;
    // A child that closes its stdout and stays alive. Rust's standard library cannot close
    // its own stdout without unsafe code, so a shell script plays this child.
    let script = directory.join("close-stdout.sh");
    std::fs::write(&script, "#!/bin/sh\nexec 1>&-\nexec /bin/sleep 30\n")
        .map_err(|e| e.to_string())?;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| e.to_string())?;
    let started = Instant::now();
    let (_session, events) = start_program(&script, "unused", &[]);
    let (frames, ending) = finish(&events)?;
    let took = started.elapsed();
    ensure!(frames.is_empty(), "frames: {frames:?}");
    ensure!(
        took >= QUIT_DRAIN,
        "ended after {took:?}, before the drain bound"
    );
    ensure!(
        ending
            == Ending::Exited {
                code: None,
                description: "killed by signal 9; the relay killed it after its output closed"
                    .to_owned()
            },
        "ending: {ending:?}"
    );
    Ok(())
}
