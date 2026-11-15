//! T1.1: drive `isabelle server` from Rust on Windows and time checks shaped like the demo's.
//!
//! Run from the repository root:
//!   powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run.ps1 cargo run --manifest-path spikes/phase1/Cargo.toml --bin t1_isabelle

use std::error::Error;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use fabric_phase1_spikes::time::parse_utc;
use serde_json::{Value, json};

type Res<T> = Result<T, Box<dyn Error>>;

const POLICY_THEORY: &str = include_str!("../../theories/Spike_Policy.thy");
const SERVER_NAME: &str = "fabric-spike";
const RULES: [&str; 7] = [
    "view_details_never",
    "read_window",
    "cand_count",
    "round_limit",
    "horizon",
    "grid",
    "note_ok",
];
const NOW: &str = "2027-01-11T08:00:00Z";
const WINDOW_START: &str = "2027-01-11T08:00:00Z";
const WINDOW_END: &str = "2027-01-18T08:00:00Z";
const MEETING_MS: i64 = 30 * 60 * 1_000;
const GOOD: [&str; 3] = [
    "2027-01-12T14:00:00Z",
    "2027-01-13T15:00:00Z",
    "2027-01-14T14:30:00Z",
];
const FIVE: [&str; 5] = [
    "2027-01-12T14:00:00Z",
    "2027-01-13T15:00:00Z",
    "2027-01-14T14:30:00Z",
    "2027-01-15T13:00:00Z",
    "2027-01-12T16:00:00Z",
];

fn main() -> Res<()> {
    let root = std::env::current_dir()?;
    if !root
        .join(".tooling")
        .join("isabelle")
        .join("Isabelle2025-2")
        .exists()
    {
        return Err("run from the repository root after scripts/setup-demo.ps1".into());
    }
    let work = root.join(".local").join("spikes").join("t1_isabelle");
    fs::create_dir_all(&work)?;
    fs::write(work.join("Spike_Policy.thy"), POLICY_THEORY)?;
    let master_dir = cygdrive(&work);

    let started = Instant::now();
    let mut server = start_server(&root, &work)?;
    println!(
        "server: address printed after {:.1}s, port {}",
        secs(started),
        server.port
    );

    let mut connection = Connection::open(server.port, &server.password)?;

    // Isabelle's headless_consolidate_delay (default 2.0 s) sets a floor under every check.
    let consolidate_delay = std::env::args()
        .skip_while(|argument| argument != "--consolidate-delay")
        .nth(1);
    let mut start_arguments = json!({"session": "HOL"});
    if let Some(delay) = &consolidate_delay {
        start_arguments["options"] = json!([format!("headless_consolidate_delay={delay}")]);
    }
    println!(
        "headless_consolidate_delay: {}",
        consolidate_delay.as_deref().unwrap_or("2.0 (default)")
    );

    let started = Instant::now();
    let (session, _) = connection.task("session_start", &start_arguments)?;
    let session_id = session["session_id"]
        .as_str()
        .ok_or("session_start returned no session_id")?
        .to_owned();
    println!(
        "session_start HOL: {:.1}s (tmp_dir {})",
        secs(started),
        session["tmp_dir"]
    );

    let checks = Checks {
        connection: &mut connection,
        session_id: &session_id,
        work: &work,
        master_dir: &master_dir,
    };
    run_experiments(checks)?;

    // Always shut the server down: a failed session_stop must not leave it running.
    let started = Instant::now();
    match connection.task("session_stop", &json!({"session_id": session_id})) {
        Ok(_) => println!("\nsession_stop: {:.1}s", secs(started)),
        Err(error) => println!("\nsession_stop reported: {error}"),
    }
    connection.call("shutdown", &Value::Null)?;
    for _ in 0..100 {
        if server.child.try_wait()?.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
    if server.child.try_wait()?.is_none() {
        server.child.kill()?;
        println!("server: killed after the shutdown timeout");
    } else {
        println!("server: shut down");
    }
    Ok(())
}

fn run_experiments(mut checks: Checks<'_>) -> Res<()> {
    // E1: does checking continue after a failed proof?
    println!("\n== E1: failed proofs and later lemmas");
    let mut theory = TheoryBuilder::new("Check_E1", "Main");
    theory.lemma("first_fails", "(2::nat) + 2 = 5", "simp");
    theory.lemma("second_holds", "(2::nat) + 2 = 4", "simp");
    theory.lemma("third_fails", "(3::nat) * 3 = 10", "simp");
    theory.value("sum", "(2::nat) + 3");
    let (text, lemmas, values) = theory.finish();
    let outcome = checks.run("Check_E1", &text, 0.05)?;
    for (line, label) in &lemmas {
        let failed = outcome.failed_lines.contains(line);
        println!(
            "  line {line:>2} {label}: {}",
            if failed { "error" } else { "proven" }
        );
    }
    for (line, label) in &values {
        println!(
            "  line {line:>2} value {label}: {}",
            outcome.value_at(*line)
        );
    }
    let continued =
        outcome.failed_lines.contains(&lemmas[2].0) && !outcome.failed_lines.contains(&lemmas[1].0);
    println!(
        "  checking continued past the first failed proof: {}  (node status {})",
        if continued { "YES" } else { "NO" },
        outcome.status
    );

    // E2: one action, every rule in both polarities.
    println!("\n== E2: one action ({} lemmas)", RULES.len() * 2);
    let single = vec![(
        "propose 5 candidates",
        propose(1, &FIVE, "afternoons suit me"),
        "deny cand_count",
    )];
    let cold = checks.run_actions("Check_E2_cold", &single, 0.05)?;
    println!(
        "  cold (also loads Spike_Policy): {:.2}s, purge {:.2}s",
        cold.0.seconds, cold.0.purge_seconds
    );
    report_actions(&single, &cold.1);
    for delay in [0.5, 0.05] {
        let mut times = Vec::new();
        for run in 0..5 {
            let (outcome, _) = checks.run_actions(
                &format!("Check_E2_{}_{run}", (delay * 100.0) as u32),
                &single,
                delay,
            )?;
            times.push(outcome.seconds);
        }
        println!("  warm x5, check_delay {delay}: {}", stats(&times));
    }

    // E3: a screening batch of 9 actions, as for 3 plans of 3 actions.
    let batch = vec![
        (
            "read own window",
            read_free_busy(WINDOW_START, WINDOW_END),
            "permit",
        ),
        (
            "propose 3 good",
            propose(1, &GOOD, "afternoons suit me"),
            "permit",
        ),
        ("propose round 2", propose(2, &GOOD[..2], ""), "permit"),
        (
            "read event details",
            read_event_details(WINDOW_START, WINDOW_END),
            "deny view_details_never",
        ),
        (
            "read past the window",
            read_free_busy(WINDOW_START, "2027-01-25T08:00:00Z"),
            "deny read_window",
        ),
        (
            "propose 5 candidates",
            propose(1, &FIVE, "afternoons suit me"),
            "deny cand_count",
        ),
        (
            "note with digits",
            propose(1, &GOOD, "meet at 2pm"),
            "deny note_ok",
        ),
        (
            "off the grid",
            propose(1, &["2027-01-12T14:07:00Z"], ""),
            "deny grid",
        ),
        (
            "after the window",
            propose(1, &["2027-01-18T12:00:00Z"], ""),
            "deny horizon",
        ),
    ];
    println!(
        "\n== E3: screening batch ({} actions, {} lemmas)",
        batch.len(),
        batch.len() * RULES.len() * 2
    );
    let mut times = Vec::new();
    let mut last = None;
    for run in 0..3 {
        let (outcome, verdicts) = checks.run_actions(&format!("Check_E3_{run}"), &batch, 0.05)?;
        times.push(outcome.seconds);
        last = Some(verdicts);
    }
    println!("  x3, check_delay 0.05: {}", stats(&times));
    if let Some(verdicts) = last {
        report_actions(&batch, &verdicts);
    }
    Ok(())
}

fn report_actions(actions: &[(&str, String, &str)], verdicts: &[ActionVerdict]) {
    for ((label, _, expected), verdict) in actions.iter().zip(verdicts) {
        let got = verdict.summary();
        let marker = if got == *expected { "ok" } else { "MISMATCH" };
        println!(
            "  {marker:>8}  {label:<22} expected {expected:<24} got {got}  {}",
            verdict.value
        );
    }
}

fn stats(times: &[f64]) -> String {
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    let min = times.iter().copied().fold(f64::INFINITY, f64::min);
    let max = times.iter().copied().fold(0.0, f64::max);
    format!("mean {mean:.2}s, min {min:.2}s, max {max:.2}s")
}

struct Checks<'a> {
    connection: &'a mut Connection,
    session_id: &'a str,
    work: &'a Path,
    master_dir: &'a str,
}

impl Checks<'_> {
    fn run(&mut self, name: &str, text: &str, check_delay: f64) -> Res<Outcome> {
        fs::write(self.work.join(format!("{name}.thy")), text)?;
        let started = Instant::now();
        let (result, _) = self.connection.task(
            "use_theories",
            &json!({
                "session_id": self.session_id,
                "theories": [name],
                "master_dir": self.master_dir,
                "check_delay": check_delay,
                "watchdog_timeout": 120.0
            }),
        )?;
        let seconds = secs(started);
        let started = Instant::now();
        self.connection.call(
            "purge_theories",
            &json!({"session_id": self.session_id, "theories": [name], "master_dir": self.master_dir}),
        )?;
        let purge_seconds = secs(started);

        let mut outcome = Outcome {
            seconds,
            purge_seconds,
            failed_lines: Vec::new(),
            writeln: Vec::new(),
            status: Value::Null,
        };
        for node in result["nodes"].as_array().into_iter().flatten() {
            if !node["theory_name"].as_str().unwrap_or("").ends_with(name) {
                continue;
            }
            outcome.status = node["status"].clone();
            for message in node["messages"].as_array().into_iter().flatten() {
                let line = message["pos"]["line"].as_u64().unwrap_or(0) as usize;
                let text = message["message"].as_str().unwrap_or("").to_owned();
                match message["kind"].as_str() {
                    Some("error") => outcome.failed_lines.push(line),
                    Some("writeln") => outcome.writeln.push((line, text)),
                    _ => {}
                }
            }
        }
        if outcome.status.is_null() {
            return Err(format!("no node for {name} in the result: {result}").into());
        }
        Ok(outcome)
    }

    /// Writes one check theory with every rule in both polarities for each action.
    fn run_actions(
        &mut self,
        name: &str,
        actions: &[(&str, String, &str)],
        check_delay: f64,
    ) -> Res<(Outcome, Vec<ActionVerdict>)> {
        let mut theory = TheoryBuilder::new(name, "Spike_Policy");
        theory.line(format!(
            "definition req :: request where \"req = \\<lparr>window_start = {}, window_end = {}, duration = {MEETING_MS}\\<rparr>\"",
            at(WINDOW_START),
            at(WINDOW_END)
        ));
        theory.line(format!(
            "definition clock :: int where \"clock = {}\"",
            at(NOW)
        ));
        let mut pairs = Vec::new();
        let mut value_lines = Vec::new();
        for (index, (_, hol, _)) in actions.iter().enumerate() {
            let action = format!("a{index}");
            theory.line(format!(
                "definition {action} :: action where \"{action} = {hol}\""
            ));
            let method = format!("(simp add: {action}_def req_def clock_def policy_defs)");
            let mut rules = Vec::new();
            for rule in RULES {
                let term = rule_term(rule, &action);
                let holds = theory.lemma(&format!("{action}_{rule}_holds"), &term, &method);
                let fails = theory.lemma(
                    &format!("{action}_{rule}_fails"),
                    &format!("\\<not> {term}"),
                    &method,
                );
                rules.push((rule, holds, fails));
            }
            pairs.push(rules);
            value_lines.push(theory.value(
                &format!("{action}_count"),
                &format!("length (cands_of {action})"),
            ));
        }
        let (text, _, _) = theory.finish();
        let outcome = self.run(name, &text, check_delay)?;
        let verdicts = pairs
            .iter()
            .zip(&value_lines)
            .map(|(rules, value_line)| ActionVerdict {
                rules: rules
                    .iter()
                    .map(|(rule, holds, fails)| {
                        let verdict = match (
                            outcome.failed_lines.contains(holds),
                            outcome.failed_lines.contains(fails),
                        ) {
                            (false, true) => "holds",
                            (true, false) => "fails",
                            (true, true) => "undecided",
                            (false, false) => "both proven",
                        };
                        ((*rule).to_owned(), verdict)
                    })
                    .collect(),
                value: format!("[candidates: {}]", outcome.value_at(*value_line)),
            })
            .collect();
        Ok((outcome, verdicts))
    }
}

struct ActionVerdict {
    rules: Vec<(String, &'static str)>,
    value: String,
}

impl ActionVerdict {
    fn summary(&self) -> String {
        let failing: Vec<&str> = self
            .rules
            .iter()
            .filter(|(_, verdict)| *verdict == "fails")
            .map(|(rule, _)| rule.as_str())
            .collect();
        let odd: Vec<String> = self
            .rules
            .iter()
            .filter(|(_, verdict)| *verdict != "fails" && *verdict != "holds")
            .map(|(rule, verdict)| format!("{rule}={verdict}"))
            .collect();
        if !odd.is_empty() {
            format!("UNDECIDED {}", odd.join(","))
        } else if failing.is_empty() {
            "permit".to_owned()
        } else {
            format!("deny {}", failing.join(","))
        }
    }
}

struct Outcome {
    seconds: f64,
    purge_seconds: f64,
    failed_lines: Vec<usize>,
    writeln: Vec<(usize, String)>,
    status: Value,
}

impl Outcome {
    fn value_at(&self, line: usize) -> String {
        self.writeln
            .iter()
            .find(|(at_line, _)| *at_line == line)
            .map(|(_, text)| {
                text.replace('\n', " ")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_else(|| "(no output)".to_owned())
    }
}

/// Line numbers paired with the label of the lemma or `value` on that line.
type LabeledLines = Vec<(usize, String)>;

struct TheoryBuilder {
    lines: Vec<String>,
    lemmas: LabeledLines,
    values: LabeledLines,
}

impl TheoryBuilder {
    fn new(name: &str, imports: &str) -> Self {
        Self {
            lines: vec![
                format!("theory {name}"),
                format!("  imports {imports}"),
                "begin".to_owned(),
                String::new(),
            ],
            lemmas: Vec::new(),
            values: Vec::new(),
        }
    }

    /// Adds a line and returns its 1-based line number.
    fn line(&mut self, text: String) -> usize {
        self.lines.push(text);
        self.lines.len()
    }

    fn lemma(&mut self, label: &str, statement: &str, method: &str) -> usize {
        let line = self.line(format!("lemma {label}: \"{statement}\" by {method}"));
        self.lemmas.push((line, label.to_owned()));
        line
    }

    fn value(&mut self, label: &str, term: &str) -> usize {
        let line = self.line(format!("value \"{term}\""));
        self.values.push((line, label.to_owned()));
        line
    }

    fn finish(mut self) -> (String, LabeledLines, LabeledLines) {
        self.lines.push(String::new());
        self.lines.push("end".to_owned());
        (self.lines.join("\n") + "\n", self.lemmas, self.values)
    }
}

fn rule_term(rule: &str, action: &str) -> String {
    match rule {
        "read_window" | "grid" => format!("{rule} req {action}"),
        "horizon" => format!("{rule} req clock {action}"),
        _ => format!("{rule} {action}"),
    }
}

fn at(text: &str) -> i64 {
    parse_utc(text).expect("fixture times are valid")
}

fn candidate(start: &str) -> String {
    format!(
        "\\<lparr>cstart = {}, cdur = {MEETING_MS}\\<rparr>",
        at(start)
    )
}

fn propose(round: u32, starts: &[&str], note: &str) -> String {
    let candidates = starts
        .iter()
        .map(|start| candidate(start))
        .collect::<Vec<_>>()
        .join(", ");
    let codes = note
        .bytes()
        .map(|byte| byte.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!("SendPropose {round} [{candidates}] [{codes}]")
}

fn read_free_busy(start: &str, end: &str) -> String {
    format!("ReadFreeBusy {} {}", at(start), at(end))
}

fn read_event_details(start: &str, end: &str) -> String {
    format!("ReadEventDetails {} {}", at(start), at(end))
}

fn secs(started: Instant) -> f64 {
    started.elapsed().as_secs_f64()
}

fn cygdrive(path: &Path) -> String {
    let text = path.display().to_string().replace('\\', "/");
    match text.split_once(":/") {
        Some((drive, rest)) if drive.len() == 1 => {
            format!("/cygdrive/{}/{rest}", drive.to_ascii_lowercase())
        }
        _ => text,
    }
}

struct Server {
    child: Child,
    port: u16,
    password: String,
}

fn start_server(root: &Path, work: &Path) -> Res<Server> {
    let isabelle = root
        .join(".tooling")
        .join("isabelle")
        .join("Isabelle2025-2");
    let user_home = root.join(".tooling").join("isabelle").join("home");
    let bash = isabelle
        .join("contrib")
        .join("cygwin")
        .join("bin")
        .join("bash.exe");
    let path = format!(
        "{};{}",
        isabelle.join("bin").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut child = Command::new(bash)
        .args([
            "--login",
            "-c",
            &format!("isabelle server -n {SERVER_NAME}"),
        ])
        .env("USER_HOME", cygdrive(&user_home))
        .env("HOME", &user_home)
        .env("PATH", path)
        .env("LANG", "en_US.UTF-8")
        .env("CHERE_INVOKING", "true")
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().ok_or("no server stdout")?;
    let stderr = child.stderr.take().ok_or("no server stderr")?;
    drain(stderr, work.join("server-stderr.log"));

    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let (port, password) = loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Err("isabelle server exited before printing its address".into());
        }
        if let Some(found) = parse_server_line(&line) {
            break found;
        }
    };
    drain(reader, work.join("server-stdout.log"));
    Ok(Server {
        child,
        port,
        password,
    })
}

/// Parses `server "NAME" = 127.0.0.1:PORT (password "UUID")`.
fn parse_server_line(line: &str) -> Option<(u16, String)> {
    let rest = line.trim().strip_prefix("server \"")?;
    let (_, rest) = rest.split_once("\" = ")?;
    let (address, rest) = rest.split_once(" (password \"")?;
    let password = rest.strip_suffix("\")")?;
    let port = address.rsplit_once(':')?.1.parse().ok()?;
    Some((port, password.to_owned()))
}

fn drain(mut source: impl Read + Send + 'static, path: PathBuf) {
    thread::spawn(move || {
        if let Ok(mut file) = File::create(path) {
            let _ = io::copy(&mut source, &mut file);
        }
    });
}

struct Connection {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl Connection {
    fn open(port: u16, password: &str) -> Res<Self> {
        let stream = TcpStream::connect(("127.0.0.1", port))?;
        stream.set_read_timeout(Some(Duration::from_secs(300)))?;
        let mut connection = Self {
            reader: BufReader::new(stream.try_clone()?),
            writer: stream,
        };
        writeln!(connection.writer, "{password}")?;
        let (kind, _) = connection.read()?;
        if kind != "OK" {
            return Err(format!("the server rejected the password: {kind}").into());
        }
        Ok(connection)
    }

    fn send(&mut self, name: &str, argument: &Value) -> Res<()> {
        if argument.is_null() {
            writeln!(self.writer, "{name}")?;
        } else {
            writeln!(self.writer, "{name} {}", serde_json::to_string(argument)?)?;
        }
        Ok(())
    }

    /// Reads one message: a single line, or a decimal length line followed by that many bytes.
    fn read(&mut self) -> Res<(String, Value)> {
        let mut line = String::new();
        loop {
            line.clear();
            if self.reader.read_line(&mut line)? == 0 {
                return Err("the server closed the connection".into());
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                continue;
            }
            let message = if trimmed.bytes().all(|byte| byte.is_ascii_digit()) {
                let length: usize = trimmed.parse()?;
                let mut buffer = vec![0; length];
                self.reader.read_exact(&mut buffer)?;
                String::from_utf8(buffer)?
            } else {
                trimmed.to_owned()
            };
            let (name, argument) = message.split_once(' ').unwrap_or((message.as_str(), ""));
            let argument = argument.trim();
            let value = if argument.is_empty() {
                Value::Null
            } else {
                serde_json::from_str(argument)
                    .unwrap_or_else(|_| Value::String(argument.to_owned()))
            };
            return Ok((name.trim().to_owned(), value));
        }
    }

    fn call(&mut self, name: &str, argument: &Value) -> Res<Value> {
        self.send(name, argument)?;
        let (kind, value) = self.read()?;
        if kind == "OK" {
            Ok(value)
        } else {
            Err(format!("{name}: {kind} {value}").into())
        }
    }

    /// Runs an asynchronous command and returns its FINISHED result and the NOTE messages.
    fn task(&mut self, name: &str, argument: &Value) -> Res<(Value, Vec<Value>)> {
        let accepted = self.call(name, argument)?;
        let task = accepted["task"]
            .as_str()
            .ok_or_else(|| format!("{name}: no task id in {accepted}"))?
            .to_owned();
        let mut notes = Vec::new();
        loop {
            let (kind, value) = self.read()?;
            let ours = value["task"].as_str() == Some(task.as_str());
            match kind.as_str() {
                "FINISHED" if ours => return Ok((value, notes)),
                "FAILED" if ours => return Err(format!("{name} failed: {value}").into()),
                _ => notes.push(json!({"kind": kind, "value": value})),
            }
        }
    }
}
