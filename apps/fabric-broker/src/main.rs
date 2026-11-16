//! The Fabric Calendar broker executable (work package CD-W04): see the library documentation
//! of `fabric_broker`.
//!
//! The process shell (design 10.6): it refuses arguments (exit 64) and a refused configuration
//! (exit 78) before it reads stdin or starts a thread; it opens the store through the authority
//! core, reads stdin as bounded lines on a thread, runs the prover (only the synthetic classifier
//! in this build) on a worker thread, runs due work, and writes frames to stdout one at a time.
//! stdout carries frames only; diagnostics go to stderr. End of input on stdin is the order to
//! stop (exit 0). A failed read of stdin or write of stdout is exit 74; a failed write to the
//! store ends the run with exit 75.

use std::io::{self, Write};
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fabric_broker::config::{self, Config, EXIT_CONFIG, EXIT_HALT, KeySource};
use fabric_broker::{Line, LineReader};
use fabric_broker_core::ports::{ProofDone, ProofJob, Prover};
use fabric_broker_core::{Attachments, Broker, Input, Job, Opener, Output, Settings, StartError};
use fabric_desktop_contracts::MAX_FRAME_BYTES;
use fabric_storage::{OpenOptions, Store};

/// The exit code for a usage error (`EX_USAGE`).
const EXIT_USAGE: u8 = 64;
/// The exit code when stdin or stdout fails (`EX_IOERR`).
const EXIT_IO: u8 = 74;

/// How many read lines and worker results may wait for the main loop. The reader blocks beyond
/// this, so a fast host cannot make the broker hold more than a few frames in memory; a worker
/// blocks too, until the main loop takes its result.
const QUEUED_LINES: usize = 4;

/// The raw wall clock in Unix milliseconds. The core keeps its own time from it, which never
/// goes back (design 5.1, D34).
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// What the reader thread and the workers pass to the main loop.
enum Event {
    Line(Vec<u8>),
    Oversized,
    /// The input ended; the main loop stops.
    End,
    /// Reading failed; the main loop stops with an error.
    Failed(io::Error),
    /// The prover's result for one job, boxed so lines do not carry its size.
    Proof(Box<ProofDone>),
}

fn spawn_reader(events: SyncSender<Event>) {
    thread::spawn(move || {
        let mut reader = LineReader::new(io::stdin().lock(), MAX_FRAME_BYTES);
        loop {
            let event = match reader.next_line() {
                Ok(Line::Frame(bytes)) => Event::Line(bytes),
                Ok(Line::Oversized) => Event::Oversized,
                Ok(Line::Unterminated(length)) => {
                    eprintln!(
                        "fabric-broker: stdin ended inside a line of {length} bytes with no line feed; it is not a frame and is not answered"
                    );
                    Event::End
                }
                Ok(Line::End) => Event::End,
                Err(e) => Event::Failed(e),
            };
            let last = matches!(event, Event::End | Event::Failed(_));
            if events.send(event).is_err() || last {
                return;
            }
        }
    });
}

/// Runs `prover` on its own thread. Jobs go to it over an unbounded channel, so the main loop
/// never waits for the worker; the worker may wait on the bounded result channel until the main
/// loop drains it, which cannot deadlock.
fn spawn_prover(mut prover: Box<dyn Prover>, events: SyncSender<Event>) -> Sender<ProofJob> {
    let (jobs, received) = mpsc::channel::<ProofJob>();
    thread::spawn(move || {
        for job in received {
            if events
                .send(Event::Proof(Box::new(prover.prove(&job))))
                .is_err()
            {
                return;
            }
        }
    });
    jobs
}

/// Opens the store in the configured data directory with the configured key. Called at start
/// and for every reopen attempt.
fn opener(config: &Config) -> Opener {
    match config.key_source {
        #[cfg(target_os = "macos")]
        KeySource::Keychain => {
            let data_dir = config.data_dir.clone();
            Box::new(move |now_ms| {
                Store::open(OpenOptions {
                    data_dir: data_dir.clone(),
                    key: Box::new(fabric_storage::KeychainKey::product()),
                    now_ms,
                })
            })
        }
        #[cfg(debug_assertions)]
        KeySource::Test => {
            let data_dir = config.data_dir.clone();
            Box::new(move |now_ms| {
                Store::open(OpenOptions {
                    data_dir: data_dir.clone(),
                    key: Box::new(fabric_storage::TestKey),
                    now_ms,
                })
            })
        }
    }
}

/// The build named in `hello`; a synthetic run says so.
fn build_label(synthetic: bool) -> String {
    let suffix = if synthetic { " synthetic" } else { "" };
    format!("{} CD-W04{suffix}", env!("CARGO_PKG_VERSION"))
}

/// The synthetic agent of a debug build, when the configuration asks for it.
#[cfg(debug_assertions)]
mod demo {
    use fabric_broker::synthetic::{SyntheticAgent, SyntheticClassifier};
    use fabric_broker_core::ports::Prover;
    use fabric_broker_core::{Broker, Output};

    /// The synthetic agent, or nothing.
    pub struct Synthetic(Option<SyntheticAgent>);

    impl Synthetic {
        pub fn new(enabled: bool) -> Synthetic {
            Synthetic(enabled.then(SyntheticAgent::new))
        }

        /// The synthetic classifier, the prover of a synthetic run.
        pub fn classifier(&self) -> Option<Box<dyn Prover>> {
            self.0
                .as_ref()
                .map(|_| Box::new(SyntheticClassifier) as Box<dyn Prover>)
        }

        pub fn next_due_ms(&self) -> Option<u64> {
            self.0.as_ref().and_then(SyntheticAgent::next_due_ms)
        }

        pub fn tick(&mut self, broker: &mut Broker, now_ms: u64) -> Output {
            match &mut self.0 {
                Some(agent) => agent.tick(broker, now_ms),
                None => Output::default(),
            }
        }

        pub fn take_replies(&mut self, output: &mut Output) {
            if let Some(agent) = &mut self.0 {
                agent.take_replies(output);
            }
        }
    }
}

/// A release build has no synthetic agent; the configuration refuses one.
#[cfg(not(debug_assertions))]
mod demo {
    use fabric_broker_core::ports::Prover;
    use fabric_broker_core::{Broker, Output};

    /// Nothing.
    pub struct Synthetic;

    impl Synthetic {
        pub fn new(enabled: bool) -> Synthetic {
            let _ = enabled;
            Synthetic
        }

        pub fn classifier(&self) -> Option<Box<dyn Prover>> {
            None
        }

        pub fn next_due_ms(&self) -> Option<u64> {
            None
        }

        pub fn tick(&mut self, broker: &mut Broker, now_ms: u64) -> Output {
            let _ = (broker, now_ms);
            Output::default()
        }

        pub fn take_replies(&mut self, output: &mut Output) {
            let _ = output;
        }
    }
}

/// Where the broker's output goes: stdout for frames, stderr for diagnostics, the prover's
/// channel for proof jobs.
struct Shell<W: Write> {
    stdout: W,
    prover: Option<Sender<ProofJob>>,
}

impl<W: Write> Shell<W> {
    /// Writes what one broker call produced and hands out its jobs. `Err` is the exit code the
    /// process ends with.
    fn emit(&mut self, output: Output) -> Result<(), ExitCode> {
        for line in &output.stderr {
            eprintln!("fabric-broker: {line}");
        }
        for frame in output.owner {
            let written = self
                .stdout
                .write_all(frame.to_line().as_bytes())
                .and_then(|()| self.stdout.flush());
            if let Err(e) = written {
                eprintln!("fabric-broker: writing stdout failed, so the host is gone: {e}");
                return Err(ExitCode::from(EXIT_IO));
            }
        }
        if let Some(fault) = output.halt {
            eprintln!("fabric-broker: a write to the store failed, so this run is over: {fault}");
            return Err(ExitCode::from(EXIT_HALT));
        }
        for job in output.jobs {
            match job {
                Job::Prove(job) => match &self.prover {
                    // The worker lives as long as the process; a send fails only after it
                    // panicked, and the core's watchdog then ends the proof as undecided.
                    Some(prover) => {
                        let _ = prover.send(job);
                    }
                    None => eprintln!(
                        "fabric-broker: a proof job came with no prover attached; its watchdog ends it"
                    ),
                },
                Job::Dispatch(_) => eprintln!(
                    "fabric-broker: a dispatch came with no provider attached; it is not carried out"
                ),
            }
        }
        Ok(())
    }
}

/// The earlier of two due times.
fn earliest(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Waits for the next event, or until `due`; `None` when the wait ended at `due`.
fn next_event(events: &Receiver<Event>, due: Option<u64>) -> Option<Event> {
    let Some(due) = due else {
        return Some(events.recv().unwrap_or(Event::End));
    };
    let now = now_ms();
    if due <= now {
        return None;
    }
    match events.recv_timeout(Duration::from_millis(due - now)) {
        Ok(event) => Some(event),
        Err(RecvTimeoutError::Timeout) => None,
        Err(RecvTimeoutError::Disconnected) => Some(Event::End),
    }
}

fn main() -> ExitCode {
    if std::env::args_os().len() > 1 {
        eprintln!(
            "usage: fabric-broker (no arguments; it speaks the owner channel of fabric-desktop/1 on stdin and stdout, and reads {}, {} and {})",
            config::DATA_DIR,
            config::KEY_SOURCE,
            config::SYNTHETIC_AGENT
        );
        return ExitCode::from(EXIT_USAGE);
    }
    let config = match config::config_from(std::env::vars_os()) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(EXIT_CONFIG);
        }
    };
    let mut synthetic = demo::Synthetic::new(config.synthetic_agent);
    let classifier = synthetic.classifier();
    let attachments = Attachments {
        prover: classifier.as_ref().map(|prover| prover.identity()),
        provider_present: false,
    };
    let settings = Settings::cd_w04(build_label(config.synthetic_agent));
    let (mut broker, started) =
        match Broker::start(opener(&config), settings, attachments, now_ms()) {
            Ok(started) => started,
            Err(error) => {
                eprintln!("fabric-broker: {error}");
                let code = match error {
                    StartError::Config(_) => EXIT_CONFIG,
                    StartError::Halt(_) => EXIT_HALT,
                };
                return ExitCode::from(code);
            }
        };
    let (sender, events) = mpsc::sync_channel(QUEUED_LINES);
    let prover = classifier.map(|classifier| spawn_prover(classifier, sender.clone()));
    spawn_reader(sender);
    let mut shell = Shell {
        stdout: io::stdout().lock(),
        prover,
    };
    if let Err(code) = shell.emit(started) {
        return code;
    }
    loop {
        let due = earliest(broker.next_due_ms(), synthetic.next_due_ms());
        let input = match next_event(&events, due) {
            None => Input::Tick,
            Some(Event::Line(bytes)) => Input::OwnerLine(bytes),
            Some(Event::Oversized) => Input::OwnerOversized,
            Some(Event::Proof(done)) => Input::ProofDone(*done),
            Some(Event::End) => {
                let mut output = broker.quit(now_ms());
                synthetic.take_replies(&mut output);
                return match shell.emit(output) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(code) => code,
                };
            }
            Some(Event::Failed(e)) => {
                eprintln!("fabric-broker: reading stdin failed: {e}");
                return ExitCode::from(EXIT_IO);
            }
        };
        let mut output = broker.handle(input, now_ms());
        synthetic.take_replies(&mut output);
        if let Err(code) = shell.emit(output) {
            return code;
        }
        let output = synthetic.tick(&mut broker, now_ms());
        if let Err(code) = shell.emit(output) {
            return code;
        }
    }
}
