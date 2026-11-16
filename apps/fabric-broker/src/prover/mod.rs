//! The Isabelle prover adapter: a worker thread that runs the rule gate
//! (`fabric_broker_policy::BrokerGate`) for the core's proof jobs.
//!
//! The worker emits a proof report only when the gate checked exactly the job's action and
//! context on the rule theory this broker was built with ([`built_theory_sha256`]). Every other
//! path is a result with no report, which the core turns into `undecided`. The gate seam
//! (`Gate`, `Starter`) is crate-private; test doubles of it exist only under `cfg(test)` and
//! can only fail.
//!
//! "PW" numbers refer to `docs/plans/calendar-desktop-prover-wiring-design.md`, whose
//! section 2 describes this module.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Instant;

use fabric_broker_core::ports::{ProofDone, ProofJob, ProverIdentity, ProverKind};
use fabric_broker_policy::report::sha256_hex;
use fabric_broker_policy::{Action, BrokerGate, Check, Context, GateConfig, PolicyError};
use fabric_desktop_contracts::owner::ProverHealth;

use self::map::{GateFate, JobRefused};
// The wall clock in Unix milliseconds, as the core's clock reads it before its offset.
use crate::shell::now_ms;

pub mod map;

/// The limit of one check (plan section 5).
pub const CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// The waits after the first and the second consecutive failure; after a third the prover
/// stays down for the run.
pub const RESTART_WAITS: [std::time::Duration; 2] = [
    std::time::Duration::from_secs(10),
    std::time::Duration::from_secs(60),
];

/// How long a stop at quit waits for the worker; under the relay's 2 s drain.
pub const QUIT_BOUND: std::time::Duration = std::time::Duration::from_millis(1_500);

/// The rule theory this broker was built with, to which the worker pins the theory in
/// `FABRIC_BROKER_RULES_DIR` and the gate's before anything is proved.
const BUILT_THEORY: &str = include_str!("../../../../isabelle/Fabric_Broker/Fabric_Broker.thy");

/// The session files this broker was built with: `<rules_dir>/ROOTS` and
/// `<rules_dir>/Fabric_Broker/ROOT` decide which theory Isabelle loads as the session the checks
/// import, so they are pinned with the theory. Any other `ROOT` that also defines the
/// session makes Isabelle refuse a duplicate session, so the gate does not start.
const BUILT_SESSION_FILES: [(&str, &str); 2] = [
    ("ROOTS", include_str!("../../../../isabelle/ROOTS")),
    (
        "Fabric_Broker/ROOT",
        include_str!("../../../../isabelle/Fabric_Broker/ROOT"),
    ),
];

/// SHA-256 of the rule theory this broker was built with, in lowercase hex, as
/// `BrokerGate::theory_sha256` reports a loaded theory.
#[must_use]
pub fn built_theory_sha256() -> &'static str {
    static HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HASH.get_or_init(|| sha256_hex(BUILT_THEORY.as_bytes()))
}

/// What the worker needs of a gate; `BrokerGate` in the product, failing doubles under
/// `cfg(test)`.
pub(crate) trait Gate: Send {
    /// Checks one action in one context.
    fn check(&mut self, context: &Context, action: &Action) -> Result<Check, PolicyError>;
    /// Stops the gate's Isabelle server.
    fn stop(&mut self) -> Result<(), PolicyError>;
    /// The SHA-256 of the rule theory the gate loaded, in lowercase hex.
    fn theory_sha256(&self) -> &str;
}

impl Gate for BrokerGate {
    fn check(&mut self, context: &Context, action: &Action) -> Result<Check, PolicyError> {
        BrokerGate::check(self, context, action)
    }

    fn stop(&mut self) -> Result<(), PolicyError> {
        BrokerGate::stop(self)
    }

    fn theory_sha256(&self) -> &str {
        BrokerGate::theory_sha256(self)
    }
}

/// Starts a gate from its configuration; the real starter calls `BrokerGate::start`.
pub(crate) type Starter =
    std::sync::Arc<dyn Fn(&GateConfig) -> Result<Box<dyn Gate>, PolicyError> + Send + Sync>;

/// The gate configuration: the three Isabelle paths, the work directory
/// `<data_dir>/prover`, the server name of [`server_name`] and [`CHECK_TIMEOUT`].
#[must_use]
pub fn gate_config(isabelle: &crate::config::IsabelleSettings, data_dir: &Path) -> GateConfig {
    GateConfig {
        isabelle_home: isabelle.isabelle_home.clone(),
        user_home: isabelle.user_home.clone(),
        rules_dir: isabelle.rules_dir.clone(),
        work_dir: data_dir.join("prover"),
        server_name: server_name(data_dir),
        check_timeout: CHECK_TIMEOUT,
        consolidate_delay_seconds: 0.1,
    }
}

/// `fabric-broker-` and 16 hex characters of the data directory's hash.
#[must_use]
pub fn server_name(data_dir: &Path) -> String {
    let hash = sha256_hex(data_dir.as_os_str().as_encoded_bytes());
    format!("fabric-broker-{}", &hash[..16])
}

/// How the worker starts its gate and when it starts again.
pub struct ProverOptions {
    /// The gate configuration; a live test may shorten its `check_timeout`.
    pub gate: GateConfig,
    /// The waits after consecutive failures; after the last one the prover stays unavailable.
    pub restart_waits: Vec<std::time::Duration>,
    /// Starts a gate; `BrokerGate::start` outside unit tests.
    pub(crate) starter: Starter,
}

impl ProverOptions {
    /// `gate_config(..)`, `RESTART_WAITS`, and a starter calling `BrokerGate::start`.
    #[must_use]
    pub fn new(isabelle: &crate::config::IsabelleSettings, data_dir: &Path) -> ProverOptions {
        ProverOptions {
            gate: gate_config(isabelle, data_dir),
            restart_waits: RESTART_WAITS.to_vec(),
            starter: Arc::new(|config: &GateConfig| {
                BrokerGate::start(config.clone()).map(|gate| Box::new(gate) as Box<dyn Gate>)
            }),
        }
    }
}

/// What the worker hands to the main loop.
pub enum ProverEvent {
    /// The result of one job, boxed so health events do not carry its size.
    Done(Box<ProofDone>),
    /// The prover's health changed.
    Health(ProverHealth),
}

/// Returns false when the receiver is gone; the worker then stops.
pub type Emit = Box<dyn FnMut(ProverEvent) -> bool + Send>;

/// How long [`IsabelleProver::stop`] waits for the acknowledgement between two drains.
const STOP_SLICE: std::time::Duration = std::time::Duration::from_millis(20);

/// The stderr line when the rule theory is not the one this broker was built with.
const OTHER_THEORY_LINE: &str = "fabric-broker: the rule theory or its session files in FABRIC_BROKER_RULES_DIR are not the ones this broker was built with; the rule prover does not run in this run";

/// What reaches the worker thread.
enum Msg {
    /// A proof job from the shell.
    Job(Box<ProofJob>),
    /// The starter thread's result.
    Started(Result<Box<dyn Gate>, PolicyError>),
    /// A dropper thread finished stopping and dropping a gate.
    Dropped,
    /// Stop now, then acknowledge.
    Stop(Sender<()>),
}

/// What the worker and the main loop's overrun poll share.
struct Shared {
    /// The health last sent to the main loop, by the worker or through [`IsabelleProver::poll`].
    health: ProverHealth,
    /// The deadline of the check now running.
    running: Option<u64>,
    /// True once the poll reported the running check past its deadline.
    overrun: bool,
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The handle of the Isabelle worker thread.
pub struct IsabelleProver {
    /// The worker's messages.
    sender: Sender<Msg>,
    /// The record the overrun poll shares with the worker.
    shared: Arc<Mutex<Shared>>,
    /// `<data_dir>/prover`, removed at every stop.
    work_dir: PathBuf,
}

impl IsabelleProver {
    /// The identity the core is given at start: Isabelle, starting. Never blocks.
    #[must_use]
    pub fn identity() -> ProverIdentity {
        ProverIdentity {
            kind: ProverKind::Isabelle,
            health: ProverHealth::Starting,
        }
    }

    /// Spawns the worker. Called only once the store is open.
    ///
    /// The worker starts its first gate at once. If the thread cannot be spawned, every
    /// [`prove`](Self::prove) returns false, so the shell reports the prover unavailable.
    #[must_use]
    pub fn spawn(options: ProverOptions, emit: Emit) -> IsabelleProver {
        let (sender, messages) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Shared {
            health: ProverHealth::Starting,
            running: None,
            overrun: false,
        }));
        let work_dir = options.gate.work_dir.clone();
        let worker = Worker {
            options,
            emit,
            shared: Arc::clone(&shared),
            sender: sender.clone(),
            state: State::Starting,
            failures: 0,
            droppers: 0,
            gone: false,
        };
        // A failed spawn drops the worker and its receiver: every send then fails.
        let _ = thread::Builder::new()
            .name("fabric-prover".to_owned())
            .spawn(move || worker.run(&messages));
        IsabelleProver {
            sender,
            shared,
            work_dir,
        }
    }

    /// Hands one job to the worker. Never blocks; false when the worker is gone.
    pub fn prove(&self, job: ProofJob) -> bool {
        self.sender.send(Msg::Job(Box::new(job))).is_ok()
    }

    /// The deadline of the running check while no overrun was reported for it.
    #[must_use]
    pub fn next_due_ms(&self) -> Option<u64> {
        let shared = lock(&self.shared);
        if shared.overrun { None } else { shared.running }
    }

    /// `Some(Unavailable)` once when the running check is at or past its job's deadline.
    pub fn poll(&self, now_ms: u64) -> Option<ProverHealth> {
        let mut shared = lock(&self.shared);
        match shared.running {
            Some(deadline) if !shared.overrun && now_ms >= deadline => {
                shared.overrun = true;
                shared.health = ProverHealth::Unavailable;
                Some(ProverHealth::Unavailable)
            }
            _ => None,
        }
    }

    /// Stops the worker: sends the stop and waits up to `bound`, calling `drain` between
    /// short waits. Without an acknowledgement it removes the work directory itself and returns
    /// false.
    pub fn stop(self, bound: std::time::Duration, drain: &mut dyn FnMut()) -> bool {
        let (ack, acknowledged) = mpsc::channel();
        if self.sender.send(Msg::Stop(ack)).is_ok() {
            let until = Instant::now().checked_add(bound);
            loop {
                let left = until.map_or(STOP_SLICE, |until| {
                    until.saturating_duration_since(Instant::now())
                });
                match acknowledged.recv_timeout(left.min(STOP_SLICE)) {
                    Ok(()) => return true,
                    Err(RecvTimeoutError::Timeout) => {
                        drain();
                        if until.is_none_or(|until| Instant::now() >= until) {
                            break;
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        }
        // Removes a check file a running check still holds open, too (Unix allows it).
        let _ = remove_work_dir(&self.work_dir);
        false
    }
}

impl Drop for IsabelleProver {
    /// A handle dropped without [`IsabelleProver::stop`] still stops the worker, without
    /// waiting. After a stop the worker is gone and the send fails.
    fn drop(&mut self) {
        let (ack, _) = mpsc::channel();
        let _ = self.sender.send(Msg::Stop(ack));
    }
}

/// The worker's state.
enum State {
    /// A starter thread is starting a gate.
    Starting,
    /// A gate is ready for checks.
    Ready(Box<dyn Gate>),
    /// No gate. `retry_at` is when to start again; `None` for the rest of the run.
    Down { retry_at: Option<Instant> },
}

/// Which failure line the stderr gets.
#[derive(Clone, Copy)]
enum Failure {
    /// A gate did not start.
    Start,
    /// A ready gate broke.
    Broken,
}

/// The result of one job in `Ready`.
struct Checked {
    done: ProofDone,
    fate: GateFate,
    /// The overrun poll reported this check past its deadline.
    overrun: bool,
    /// The failure category when the gate restarts.
    category: &'static str,
}

impl Checked {
    fn refused(job: &ProofJob, reason: &'static str) -> Checked {
        Checked {
            done: map::no_report(job, reason),
            fate: GateFate::Keep,
            overrun: false,
            category: "",
        }
    }
}

/// The worker thread: it owns the gate, and every gate start and drop happens off it.
struct Worker {
    options: ProverOptions,
    emit: Emit,
    shared: Arc<Mutex<Shared>>,
    /// For the starter and dropper threads.
    sender: Sender<Msg>,
    state: State,
    /// Consecutive failures since the last `Ready`.
    failures: usize,
    /// Dropper threads still stopping a gate.
    droppers: usize,
    /// The main loop no longer takes events.
    gone: bool,
}

impl Worker {
    fn run(mut self, messages: &Receiver<Msg>) {
        self.enter_starting();
        while !self.gone {
            let message = match self.retry_wait() {
                Some(wait) => match messages.recv_timeout(wait) {
                    Ok(message) => Some(message),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                },
                None => match messages.recv() {
                    Ok(message) => Some(message),
                    Err(_) => break,
                },
            };
            if let Some(message) = message
                && let Err(ack) = self.handle(message)
            {
                self.shut_down();
                let _ = ack.send(());
                return;
            }
            if self.retry_due() {
                self.enter_starting();
            }
        }
        self.shut_down();
    }

    /// Handles one message; `Err` with the acknowledgement for a stop.
    fn handle(&mut self, message: Msg) -> Result<(), Sender<()>> {
        match message {
            Msg::Job(job) => self.job(&job),
            Msg::Started(started) => self.started(started),
            Msg::Dropped => self.droppers = self.droppers.saturating_sub(1),
            Msg::Stop(ack) => return Err(ack),
        }
        Ok(())
    }

    /// While `Down` with a retry and no dropper alive: how long until the retry.
    fn retry_wait(&self) -> Option<std::time::Duration> {
        match self.state {
            State::Down { retry_at: Some(at) } if self.droppers == 0 => {
                Some(at.saturating_duration_since(Instant::now()))
            }
            _ => None,
        }
    }

    fn retry_due(&self) -> bool {
        self.retry_wait().is_some_and(|wait| wait.is_zero())
    }

    fn send(&mut self, event: ProverEvent) {
        if !self.gone && !(self.emit)(event) {
            self.gone = true;
        }
    }

    /// Sends `health` when it differs from the health last sent.
    fn set_health(&mut self, health: ProverHealth) {
        {
            let mut shared = lock(&self.shared);
            if shared.health == health {
                return;
            }
            shared.health = health;
        }
        self.send(ProverEvent::Health(health));
    }

    /// Enters `Starting`: a fresh private work directory, the theory pin on the file and on
    /// the session files, then a starter thread. Called only while no dropper is alive.
    fn enter_starting(&mut self) {
        self.state = State::Starting;
        self.set_health(ProverHealth::Starting);
        if prepare_work_dir(&self.options.gate.work_dir).is_err() {
            self.fail(Failure::Start, "I/O");
            return;
        }
        let theory = self
            .options
            .gate
            .rules_dir
            .join(fabric_broker_policy::gate::SESSION)
            .join("Fabric_Broker.thy");
        match fs::read(theory) {
            Err(_) => {
                self.fail(Failure::Start, "I/O");
                return;
            }
            Ok(bytes) if sha256_hex(&bytes) != built_theory_sha256() => {
                self.other_theory();
                return;
            }
            Ok(_) => {}
        }
        for (name, built) in BUILT_SESSION_FILES {
            match fs::read(self.options.gate.rules_dir.join(name)) {
                Err(_) => {
                    self.fail(Failure::Start, "I/O");
                    return;
                }
                Ok(bytes) if bytes != built.as_bytes() => {
                    self.other_theory();
                    return;
                }
                Ok(_) => {}
            }
        }
        let starter = Arc::clone(&self.options.starter);
        let config = self.options.gate.clone();
        let sender = self.sender.clone();
        let spawned = thread::Builder::new()
            .name("fabric-prover-start".to_owned())
            .spawn(move || {
                // After a stop the worker is gone: the starter stops its own gate.
                if let Err(mpsc::SendError(Msg::Started(Ok(mut gate)))) =
                    sender.send(Msg::Started(starter(&config)))
                {
                    let _ = gate.stop();
                }
            });
        if spawned.is_err() {
            self.fail(Failure::Start, "I/O");
        }
    }

    fn started(&mut self, started: Result<Box<dyn Gate>, PolicyError>) {
        if !matches!(self.state, State::Starting) {
            if let Ok(gate) = started {
                self.drop_gate(gate);
            }
            return;
        }
        match started {
            Ok(gate) if gate.theory_sha256() != built_theory_sha256() => {
                self.drop_gate(gate);
                self.other_theory();
            }
            Ok(gate) => {
                self.state = State::Ready(gate);
                self.failures = 0;
                self.set_health(ProverHealth::Ready);
            }
            Err(error) => self.fail(Failure::Start, category(&error)),
        }
    }

    fn job(&mut self, job: &ProofJob) {
        let checked = match &mut self.state {
            State::Starting => Checked::refused(job, map::STARTING),
            State::Down { .. } => Checked::refused(job, map::NOT_RUNNING),
            State::Ready(gate) => check_job(gate.as_mut(), &self.shared, job),
        };
        self.send(ProverEvent::Done(Box::new(checked.done)));
        match checked.fate {
            GateFate::Restart => {
                let broken = std::mem::replace(&mut self.state, State::Down { retry_at: None });
                if let State::Ready(gate) = broken {
                    self.drop_gate(gate);
                }
                self.fail(Failure::Broken, checked.category);
            }
            GateFate::Keep if checked.overrun => self.set_health(ProverHealth::Ready),
            GateFate::Keep => {}
        }
    }

    /// The failure rule: wait the n-th wait after the n-th consecutive failure, and after
    /// the last wait stay down for the run. No error text, path or calendar value is written.
    fn fail(&mut self, failure: Failure, category: &str) {
        self.failures += 1;
        let wait = self
            .options
            .restart_waits
            .get(self.failures - 1)
            .copied()
            .and_then(|wait| Instant::now().checked_add(wait).map(|at| (wait, at)));
        let what = match failure {
            Failure::Start => "could not start",
            Failure::Broken => "failed",
        };
        let line = match wait {
            Some((wait, at)) => {
                self.state = State::Down { retry_at: Some(at) };
                format!(
                    "fabric-broker: the rule prover {what} ({category}); it is tried again in {} s",
                    wait.as_secs()
                )
            }
            None => {
                self.state = State::Down { retry_at: None };
                format!(
                    "fabric-broker: the rule prover failed again ({category}); it is not tried again in this run; quit Fabric Calendar and open it again"
                )
            }
        };
        stderr_line(&line);
        self.set_health(ProverHealth::Unavailable);
    }

    /// The rule theory is not the built one: down for the run, nothing is proved.
    fn other_theory(&mut self) {
        self.state = State::Down { retry_at: None };
        stderr_line(OTHER_THEORY_LINE);
        self.set_health(ProverHealth::Unavailable);
    }

    /// Hands a gate to a dropper thread, which stops it, drops it and reports back. A gate's
    /// stop can block for over a minute, so the worker never waits for it.
    fn drop_gate(&mut self, gate: Box<dyn Gate>) {
        let sender = self.sender.clone();
        let spawned = thread::Builder::new()
            .name("fabric-prover-drop".to_owned())
            .spawn(move || {
                let mut gate = gate;
                let _ = gate.stop();
                drop(gate);
                let _ = sender.send(Msg::Dropped);
            });
        // Without a thread the closure, and the gate in it, is already dropped.
        if spawned.is_ok() {
            self.droppers += 1;
        }
    }

    /// At a stop, or when the main loop is gone: stops a ready gate here, then removes the work
    /// directory.
    fn shut_down(&mut self) {
        if let State::Ready(mut gate) =
            std::mem::replace(&mut self.state, State::Down { retry_at: None })
        {
            let _ = gate.stop();
            drop(gate);
        }
        let _ = remove_work_dir(&self.options.gate.work_dir);
    }
}

/// One job in `Ready`.
fn check_job(gate: &mut dyn Gate, shared: &Mutex<Shared>, job: &ProofJob) -> Checked {
    if now_ms() >= job.deadline_ms {
        return Checked::refused(job, map::PAST_DEADLINE);
    }
    let (context, action) = match map::context_of(job) {
        Ok(checkable) => checkable,
        Err(JobRefused(reason)) => return Checked::refused(job, reason),
    };
    {
        let mut shared = lock(shared);
        shared.running = Some(job.deadline_ms);
        shared.overrun = false;
    }
    let result = gate.check(&context, &action);
    let overrun = {
        let mut shared = lock(shared);
        shared.running = None;
        std::mem::take(&mut shared.overrun)
    };
    let category = result.as_ref().err().map_or("Isabelle", category);
    let (done, fate) = map::done_of(job, &context, built_theory_sha256(), result);
    Checked {
        done,
        fate,
        overrun,
        category,
    }
}

/// The category a failure line names; never the error's text.
fn category(error: &PolicyError) -> &'static str {
    match error {
        PolicyError::Io(_) => "I/O",
        PolicyError::Protocol(_) => "protocol",
        PolicyError::Isabelle(_) => "Isabelle",
        PolicyError::Json(_) => "JSON",
        PolicyError::Render(_) => "render",
    }
}

fn stderr_line(line: &str) {
    let _ = writeln!(io::stderr(), "{line}");
}

/// Removes the work directory; one that is already gone is fine. `remove_dir_all` removes a
/// symbolic link itself, never what it points to.
pub(crate) fn remove_work_dir(dir: &Path) -> io::Result<()> {
    match fs::remove_dir_all(dir) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        removed => removed,
    }
}

/// Empties the work directory and makes it again, private on Unix. The create is not
/// recursive, so it never makes the data directory itself.
fn prepare_work_dir(dir: &Path) -> io::Result<()> {
    remove_work_dir(dir)?;
    #[cfg_attr(not(unix), allow(unused_mut, reason = "the mode is set on Unix only"))]
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(dir)
}

#[cfg(test)]
mod tests {
    //! The worker with gate doubles. A double's `check` only fails, or blocks and then fails;
    //! no double returns a `Check`.

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Condvar, mpsc::RecvTimeoutError};
    use std::time::Duration;

    use super::*;
    use crate::config::IsabelleSettings;
    use crate::prover::map::tests::{create, job_of};

    /// How long a test waits for an event that must come.
    const PATIENCE: Duration = Duration::from_secs(10);
    /// How long a test waits to see that an event does not come.
    const QUIET: Duration = Duration::from_millis(250);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "fabric-prover-{name}-{}-{}-{n}",
                std::process::id(),
                now_ms()
            ));
            fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Opens once, for every waiter, and stays open.
    #[derive(Clone, Default)]
    struct Latch(Arc<(Mutex<bool>, Condvar)>);

    impl Latch {
        fn wait(&self) {
            let (open, changed) = &*self.0;
            let mut open = open.lock().unwrap();
            while !*open {
                open = changed.wait(open).unwrap();
            }
        }

        fn open(&self) {
            let (open, changed) = &*self.0;
            *open.lock().unwrap() = true;
            changed.notify_all();
        }
    }

    /// A gate that cannot prove: its check fails, after an optional wait.
    #[derive(Clone)]
    struct Double {
        theory: String,
        error: fn() -> PolicyError,
        /// The check waits for it before failing.
        check_latch: Option<Latch>,
        /// The stop waits for it.
        stop_latch: Option<Latch>,
        /// A file the check writes before it waits, as the gate's check theory.
        check_file: Option<PathBuf>,
        checks: Arc<AtomicUsize>,
        stops: Arc<AtomicUsize>,
    }

    impl Double {
        fn new(error: fn() -> PolicyError) -> Double {
            Double {
                theory: built_theory_sha256().to_owned(),
                error,
                check_latch: None,
                stop_latch: None,
                check_file: None,
                checks: Arc::default(),
                stops: Arc::default(),
            }
        }
    }

    impl Gate for Double {
        fn check(&mut self, _: &Context, _: &Action) -> Result<Check, PolicyError> {
            self.checks.fetch_add(1, Ordering::SeqCst);
            if let Some(file) = &self.check_file {
                fs::write(file, "calendar data").unwrap();
            }
            if let Some(latch) = &self.check_latch {
                latch.wait();
            }
            Err((self.error)())
        }

        fn stop(&mut self) -> Result<(), PolicyError> {
            if let Some(latch) = &self.stop_latch {
                latch.wait();
            }
            self.stops.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn theory_sha256(&self) -> &str {
            &self.theory
        }
    }

    fn io() -> PolicyError {
        PolicyError::Io(io::Error::other("socket"))
    }

    fn render() -> PolicyError {
        PolicyError::Render("identifier".to_owned())
    }

    fn isabelle() -> PolicyError {
        PolicyError::Isabelle("no heap".to_owned())
    }

    /// A starter that counts its calls and starts each gate with `start(call)`, `call` from 1.
    fn counting(
        calls: &Arc<AtomicUsize>,
        start: impl Fn(usize) -> Result<Box<dyn Gate>, PolicyError> + Send + Sync + 'static,
    ) -> Starter {
        let calls = Arc::clone(calls);
        Arc::new(move |_: &GateConfig| start(calls.fetch_add(1, Ordering::SeqCst) + 1))
    }

    fn repository_rules() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../isabelle")
    }

    fn settings(data: &Path) -> IsabelleSettings {
        IsabelleSettings {
            isabelle_home: data.join("no-isabelle"),
            user_home: data.to_path_buf(),
            rules_dir: repository_rules(),
        }
    }

    fn options(data: &Path, waits: &[Duration], starter: Starter) -> ProverOptions {
        ProverOptions {
            gate: gate_config(&settings(data), data),
            restart_waits: waits.to_vec(),
            starter,
        }
    }

    fn spawn(options: ProverOptions) -> (IsabelleProver, Receiver<ProverEvent>) {
        let (sender, events) = mpsc::channel();
        let emit: Emit = Box::new(move |event| sender.send(event).is_ok());
        (IsabelleProver::spawn(options, emit), events)
    }

    fn health(events: &Receiver<ProverEvent>) -> ProverHealth {
        match events.recv_timeout(PATIENCE).expect("a health event") {
            ProverEvent::Health(health) => health,
            ProverEvent::Done(done) => panic!("a result, not health: {done:?}"),
        }
    }

    fn done(events: &Receiver<ProverEvent>) -> ProofDone {
        match events.recv_timeout(PATIENCE).expect("a result") {
            ProverEvent::Done(done) => *done,
            ProverEvent::Health(health) => panic!("health {health:?}, not a result"),
        }
    }

    fn quiet(events: &Receiver<ProverEvent>) {
        match events.recv_timeout(QUIET) {
            Err(RecvTimeoutError::Timeout) => {}
            Ok(ProverEvent::Health(health)) => panic!("unexpected health {health:?}"),
            Ok(ProverEvent::Done(done)) => panic!("unexpected result {done:?}"),
            Err(RecvTimeoutError::Disconnected) => panic!("the worker is gone"),
        }
    }

    /// A job whose deadline is a minute away.
    fn job() -> ProofJob {
        let mut job = job_of(create());
        job.deadline_ms = now_ms() + 60_000;
        job
    }

    fn reason(done: &ProofDone) -> &str {
        assert!(done.report.is_none() && done.ruling.is_none());
        assert!(done.authorizing_grants.is_empty() && done.check_theory_text.is_none());
        done.undecided_reason.as_deref().unwrap()
    }

    fn ready(gate: Double) -> Starter {
        Arc::new(move |_: &GateConfig| Ok(Box::new(gate.clone()) as Box<dyn Gate>))
    }

    fn stop(prover: IsabelleProver) -> bool {
        prover.stop(QUIT_BOUND, &mut || {})
    }

    fn wait_until(what: &str, done: impl Fn() -> bool) {
        let until = Instant::now() + PATIENCE;
        while !done() {
            assert!(Instant::now() < until, "waited too long for {what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn health_is_starting_until_the_gate_starts() {
        let data = TempDir::new("w01");
        let latch = Latch::default();
        let gate = Double::new(io);
        let waiting = latch.clone();
        let starter: Starter = Arc::new(move |_: &GateConfig| {
            waiting.wait();
            Ok(Box::new(gate.clone()) as Box<dyn Gate>)
        });
        let (prover, events) = spawn(options(data.path(), &[], starter));
        assert_eq!(IsabelleProver::identity().health, ProverHealth::Starting);
        assert_eq!(IsabelleProver::identity().kind, ProverKind::Isabelle);
        quiet(&events);
        assert!(prover.prove(job()));
        assert_eq!(reason(&done(&events)), map::STARTING);
        latch.open();
        assert_eq!(health(&events), ProverHealth::Ready);
        assert!(stop(prover));
    }

    #[test]
    fn the_nth_failure_waits_the_nth_wait_and_the_third_leaves_the_prover_unavailable() {
        let data = TempDir::new("w02");
        let calls = Arc::new(AtomicUsize::new(0));
        let times = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&times);
        let starter = counting(&calls, move |_| {
            recorded.lock().unwrap().push(Instant::now());
            Err(isabelle())
        });
        let waits = [Duration::from_millis(100), Duration::from_millis(300)];
        let (prover, events) = spawn(options(data.path(), &waits, starter));
        for expected in [
            ProverHealth::Unavailable,
            ProverHealth::Starting,
            ProverHealth::Unavailable,
            ProverHealth::Starting,
            ProverHealth::Unavailable,
        ] {
            assert_eq!(health(&events), expected);
        }
        thread::sleep(Duration::from_millis(500));
        quiet(&events);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        let times = times.lock().unwrap().clone();
        assert!(times[1] - times[0] >= waits[0]);
        assert!(times[2] - times[1] >= waits[1]);
        assert!(prover.prove(job()));
        assert_eq!(reason(&done(&events)), map::NOT_RUNNING);
        assert!(stop(prover));
    }

    #[test]
    fn jobs_while_not_ready_are_answered_at_once_with_no_report() {
        let data = TempDir::new("w03");
        let latch = Latch::default();
        let waiting = latch.clone();
        let starter: Starter = Arc::new(move |_: &GateConfig| {
            waiting.wait();
            Err(isabelle())
        });
        let (prover, events) = spawn(options(data.path(), &[], starter));
        assert!(prover.prove(job()));
        assert_eq!(reason(&done(&events)), map::STARTING);
        latch.open();
        assert_eq!(health(&events), ProverHealth::Unavailable);
        let mut second = job();
        second.job_id = 8;
        assert!(prover.prove(second));
        let done = done(&events);
        assert_eq!(done.job_id, 8);
        assert_eq!(reason(&done), map::NOT_RUNNING);
        assert!(stop(prover));
    }

    #[test]
    fn a_job_past_its_deadline_is_not_checked() {
        let data = TempDir::new("w04");
        let gate = Double::new(io);
        let checks = Arc::clone(&gate.checks);
        let (prover, events) = spawn(options(data.path(), &[], ready(gate)));
        assert_eq!(health(&events), ProverHealth::Ready);
        let mut late = job();
        late.deadline_ms = now_ms();
        assert!(prover.prove(late));
        assert_eq!(reason(&done(&events)), map::PAST_DEADLINE);
        quiet(&events);
        assert_eq!(checks.load(Ordering::SeqCst), 0);
        assert!(stop(prover));
    }

    #[test]
    fn a_broken_gate_is_dropped_off_the_worker_and_restarted() {
        let data = TempDir::new("w05");
        let gate = Double::new(io);
        let stops = Arc::clone(&gate.stops);
        let calls = Arc::new(AtomicUsize::new(0));
        let starter = counting(&calls, move |_| Ok(Box::new(gate.clone()) as Box<dyn Gate>));
        let waits = [Duration::from_millis(50)];
        let (prover, events) = spawn(options(data.path(), &waits, starter));
        assert_eq!(health(&events), ProverHealth::Ready);
        assert!(prover.prove(job()));
        assert_eq!(reason(&done(&events)), map::FAILED);
        for expected in [
            ProverHealth::Unavailable,
            ProverHealth::Starting,
            ProverHealth::Ready,
        ] {
            assert_eq!(health(&events), expected);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(stops.load(Ordering::SeqCst), 1);
        assert!(stop(prover));
        assert_eq!(stops.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_render_error_keeps_the_gate() {
        let data = TempDir::new("w06");
        let gate = Double::new(render);
        let checks = Arc::clone(&gate.checks);
        let calls = Arc::new(AtomicUsize::new(0));
        let starter = counting(&calls, move |_| Ok(Box::new(gate.clone()) as Box<dyn Gate>));
        let (prover, events) = spawn(options(data.path(), &[], starter));
        assert_eq!(health(&events), ProverHealth::Ready);
        for _ in 0..2 {
            assert!(prover.prove(job()));
            assert_eq!(reason(&done(&events)), map::RENDER);
        }
        quiet(&events);
        assert_eq!(checks.load(Ordering::SeqCst), 2);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(stop(prover));
    }

    #[test]
    fn stop_returns_within_its_bound_while_starting_or_blocked() {
        let bound = Duration::from_millis(300);
        let slack = Duration::from_millis(700);

        let data = TempDir::new("w07-starting");
        let latch = Latch::default();
        let waiting = latch.clone();
        let starter: Starter = Arc::new(move |_: &GateConfig| {
            waiting.wait();
            Err(isabelle())
        });
        let (prover, _events) = spawn(options(data.path(), &[], starter));
        let started = Instant::now();
        assert!(prover.stop(bound, &mut || {}));
        assert!(started.elapsed() < bound + slack);
        latch.open();

        let data = TempDir::new("w07-blocked");
        let latch = Latch::default();
        let mut gate = Double::new(render);
        gate.check_latch = Some(latch.clone());
        let checks = Arc::clone(&gate.checks);
        let (prover, events) = spawn(options(data.path(), &[], ready(gate)));
        assert_eq!(health(&events), ProverHealth::Ready);
        assert!(prover.prove(job()));
        wait_until("the check", || checks.load(Ordering::SeqCst) == 1);
        let drains = AtomicUsize::new(0);
        let started = Instant::now();
        assert!(!prover.stop(bound, &mut || {
            drains.fetch_add(1, Ordering::SeqCst);
        }));
        let elapsed = started.elapsed();
        assert!(elapsed >= bound && elapsed < bound + slack, "{elapsed:?}");
        assert!(drains.load(Ordering::SeqCst) > 0);
        latch.open();
    }

    #[test]
    fn stop_removes_the_work_dir_in_every_state() {
        // Starting.
        let data = TempDir::new("w08-starting");
        let latch = Latch::default();
        let waiting = latch.clone();
        let starter: Starter = Arc::new(move |_: &GateConfig| {
            waiting.wait();
            Err(isabelle())
        });
        let (prover, _events) = spawn(options(data.path(), &[], starter));
        let work_dir = data.path().join("prover");
        wait_until("the work dir", || work_dir.is_dir());
        assert!(stop(prover));
        assert!(!work_dir.exists());
        latch.open();

        // Ready.
        let data = TempDir::new("w08-ready");
        let gate = Double::new(io);
        let stops = Arc::clone(&gate.stops);
        let (prover, events) = spawn(options(data.path(), &[], ready(gate)));
        assert_eq!(health(&events), ProverHealth::Ready);
        let work_dir = data.path().join("prover");
        assert!(work_dir.is_dir());
        assert!(stop(prover));
        assert!(!work_dir.exists());
        assert_eq!(stops.load(Ordering::SeqCst), 1);

        // Down.
        let data = TempDir::new("w08-down");
        let starter: Starter = Arc::new(|_: &GateConfig| Err(isabelle()));
        let (prover, events) = spawn(options(data.path(), &[], starter));
        assert_eq!(health(&events), ProverHealth::Unavailable);
        let work_dir = data.path().join("prover");
        assert!(work_dir.is_dir());
        assert!(stop(prover));
        assert!(!work_dir.exists());
        assert!(data.path().is_dir(), "the data directory stays");
    }

    #[test]
    fn the_server_name_is_stable_per_data_dir_and_valid() {
        let one = Path::new("/Users/someone/Library/Application Support/Fabric Calendar");
        let other = Path::new("/tmp/fabric scratch");
        let name = server_name(one);
        assert_eq!(name, server_name(one));
        assert_ne!(name, server_name(other));
        assert_eq!(name.len(), "fabric-broker-".len() + 16);
        assert!(name.starts_with("fabric-broker-"));
        assert!(
            name.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );

        let settings = settings(one);
        let config = gate_config(&settings, one);
        assert_eq!(config.isabelle_home, settings.isabelle_home);
        assert_eq!(config.user_home, settings.user_home);
        assert_eq!(config.rules_dir, settings.rules_dir);
        assert_eq!(config.work_dir, one.join("prover"));
        assert_eq!(config.server_name, name);
        assert_eq!(config.check_timeout, CHECK_TIMEOUT);
        let options = ProverOptions::new(&settings, one);
        assert_eq!(options.restart_waits, RESTART_WAITS);
        assert_eq!(options.gate.server_name, name);
    }

    #[test]
    fn a_real_gate_with_no_isabelle_is_unavailable() {
        let data = TempDir::new("w10");
        let home = data.path().join("empty-isabelle");
        fs::create_dir(&home).unwrap();
        let settings = IsabelleSettings {
            isabelle_home: home,
            user_home: data.path().to_path_buf(),
            rules_dir: repository_rules(),
        };
        let mut options = ProverOptions::new(&settings, data.path());
        options.restart_waits = Vec::new();
        let (prover, events) = spawn(options);
        assert_eq!(health(&events), ProverHealth::Unavailable);
        assert!(prover.prove(job()));
        assert_eq!(reason(&done(&events)), map::NOT_RUNNING);
        assert!(stop(prover));
        assert!(!data.path().join("prover").exists());
    }

    #[test]
    fn a_rules_dir_with_another_theory_leaves_the_prover_unavailable_and_starts_nothing() {
        let data = TempDir::new("w11");
        let rules = data.path().join("rules");
        fs::create_dir_all(rules.join("Fabric_Broker")).unwrap();
        fs::write(
            rules.join("Fabric_Broker").join("Fabric_Broker.thy"),
            format!("{BUILT_THEORY} "),
        )
        .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let starter = counting(&calls, |_| Err(isabelle()));
        let mut options = options(data.path(), &[Duration::from_millis(10)], starter);
        options.gate.rules_dir = rules;
        let (prover, events) = spawn(options);
        assert_eq!(health(&events), ProverHealth::Unavailable);
        quiet(&events);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(prover.prove(job()));
        assert_eq!(reason(&done(&events)), map::NOT_RUNNING);
        assert!(stop(prover));
    }

    /// The theory pin: the built theory stays in place, but `ROOTS` or the session's
    /// `ROOT` points the session at a copied, changed theory. The prover is unavailable for the
    /// run and no gate starts.
    #[test]
    fn session_files_that_point_elsewhere_leave_the_prover_unavailable_and_start_nothing() {
        let redirects: [(&str, &str); 2] = [
            ("ROOTS", "Rules2\n"),
            (
                "Fabric_Broker/ROOT",
                "session Fabric_Broker in \"../Rules2\" = HOL +\n  theories\n    Fabric_Broker\n",
            ),
        ];
        for (file, text) in redirects {
            let data = TempDir::new("w11-session");
            let rules = data.path().join("rules");
            for dir in ["Fabric_Broker", "Rules2"] {
                fs::create_dir_all(rules.join(dir)).unwrap();
            }
            fs::write(
                rules.join("Fabric_Broker").join("Fabric_Broker.thy"),
                BUILT_THEORY,
            )
            .unwrap();
            for (name, built) in BUILT_SESSION_FILES {
                fs::write(rules.join(name), built).unwrap();
            }
            fs::write(
                rules.join("Rules2").join("Fabric_Broker.thy"),
                format!("{BUILT_THEORY} "),
            )
            .unwrap();
            fs::write(rules.join("Rules2").join("ROOT"), BUILT_SESSION_FILES[1].1).unwrap();
            fs::write(rules.join(file), text).unwrap();
            let calls = Arc::new(AtomicUsize::new(0));
            let starter = counting(&calls, |_| Err(isabelle()));
            let mut options = options(data.path(), &[Duration::from_millis(10)], starter);
            options.gate.rules_dir = rules;
            let (prover, events) = spawn(options);
            assert_eq!(health(&events), ProverHealth::Unavailable, "{file}");
            quiet(&events);
            assert_eq!(calls.load(Ordering::SeqCst), 0, "{file}");
            assert!(prover.prove(job()));
            assert_eq!(reason(&done(&events)), map::NOT_RUNNING);
            assert!(stop(prover));
        }
    }

    #[test]
    fn a_gate_reporting_another_theory_is_dropped_before_any_check() {
        let data = TempDir::new("w12");
        let mut gate = Double::new(io);
        gate.theory = "0".repeat(64);
        let checks = Arc::clone(&gate.checks);
        let stops = Arc::clone(&gate.stops);
        let calls = Arc::new(AtomicUsize::new(0));
        let starter = counting(&calls, move |_| Ok(Box::new(gate.clone()) as Box<dyn Gate>));
        let (prover, events) = spawn(options(data.path(), &[Duration::from_millis(10)], starter));
        assert_eq!(health(&events), ProverHealth::Unavailable);
        assert!(prover.prove(job()));
        assert_eq!(reason(&done(&events)), map::NOT_RUNNING);
        quiet(&events);
        wait_until("the dropper", || stops.load(Ordering::SeqCst) == 1);
        assert_eq!(checks.load(Ordering::SeqCst), 0);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(stop(prover));
    }

    #[test]
    fn an_unacknowledged_stop_removes_the_check_file_from_the_calling_thread() {
        let data = TempDir::new("w13");
        let work_dir = data.path().join("prover");
        let check_file = work_dir.join("Check_000001.thy");
        let latch = Latch::default();
        let mut gate = Double::new(io);
        gate.check_latch = Some(latch.clone());
        gate.check_file = Some(check_file.clone());
        let (prover, events) = spawn(options(data.path(), &[], ready(gate)));
        assert_eq!(health(&events), ProverHealth::Ready);
        assert!(prover.prove(job()));
        wait_until("the check file", || check_file.is_file());
        assert!(!prover.stop(Duration::from_millis(100), &mut || {}));
        assert!(!check_file.exists());
        assert!(!work_dir.exists());
        latch.open();
    }

    #[test]
    fn a_restart_waits_until_the_broken_gate_is_dropped() {
        let data = TempDir::new("w14");
        let stop_latch = Latch::default();
        let mut broken = Double::new(io);
        broken.stop_latch = Some(stop_latch.clone());
        let calls = Arc::new(AtomicUsize::new(0));
        let starter = counting(&calls, move |call| {
            let gate = if call == 1 {
                broken.clone()
            } else {
                Double::new(io)
            };
            Ok(Box::new(gate) as Box<dyn Gate>)
        });
        let (prover, events) = spawn(options(data.path(), &[Duration::from_millis(10)], starter));
        assert_eq!(health(&events), ProverHealth::Ready);
        assert!(prover.prove(job()));
        assert_eq!(reason(&done(&events)), map::FAILED);
        assert_eq!(health(&events), ProverHealth::Unavailable);
        quiet(&events);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        stop_latch.open();
        assert_eq!(health(&events), ProverHealth::Starting);
        assert_eq!(health(&events), ProverHealth::Ready);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(stop(prover));
    }

    #[test]
    fn health_leaves_ready_while_a_check_runs_past_its_deadline() {
        let data = TempDir::new("w15");
        let latch = Latch::default();
        let mut gate = Double::new(render);
        gate.check_latch = Some(latch.clone());
        let (prover, events) = spawn(options(data.path(), &[], ready(gate)));
        assert_eq!(health(&events), ProverHealth::Ready);
        assert_eq!(prover.next_due_ms(), None);
        assert_eq!(prover.poll(u64::MAX), None);
        let job = job();
        let deadline = job.deadline_ms;
        assert!(prover.prove(job));
        wait_until("the running check", || {
            prover.next_due_ms() == Some(deadline)
        });
        assert_eq!(prover.poll(deadline - 1), None);
        assert_eq!(prover.poll(deadline), Some(ProverHealth::Unavailable));
        assert_eq!(prover.poll(deadline + 1), None);
        assert_eq!(prover.next_due_ms(), None);
        latch.open();
        assert_eq!(reason(&done(&events)), map::RENDER);
        assert_eq!(health(&events), ProverHealth::Ready);
        assert_eq!(prover.next_due_ms(), None);
        assert!(stop(prover));
    }

    #[test]
    fn the_work_dir_is_emptied_and_made_private_before_each_start() {
        let data = TempDir::new("w16");
        let work_dir = data.path().join("prover");
        fs::create_dir_all(work_dir.join("nested")).unwrap();
        fs::write(work_dir.join("Check_000009.thy"), "left by a killed broker").unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&seen);
        let starter: Starter = Arc::new(move |config: &GateConfig| {
            let entries = fs::read_dir(&config.work_dir).unwrap().count();
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt as _;
                fs::metadata(&config.work_dir).unwrap().permissions().mode() & 0o777
            };
            #[cfg(not(unix))]
            let mode = 0o700;
            fs::write(config.work_dir.join("isabelle-server-stderr.log"), "log").unwrap();
            recorded.lock().unwrap().push((entries, mode));
            Err(isabelle())
        });
        let waits = [Duration::from_millis(10)];
        let (prover, events) = spawn(options(data.path(), &waits, starter));
        for expected in [
            ProverHealth::Unavailable,
            ProverHealth::Starting,
            ProverHealth::Unavailable,
        ] {
            assert_eq!(health(&events), expected);
        }
        assert_eq!(*seen.lock().unwrap(), [(0, 0o700), (0, 0o700)]);
        assert!(stop(prover));
        assert!(!work_dir.exists());
    }

    #[test]
    fn the_built_theory_is_the_repository_theory() {
        let file = repository_rules()
            .join("Fabric_Broker")
            .join("Fabric_Broker.thy");
        assert_eq!(sha256_hex(&fs::read(file).unwrap()), built_theory_sha256());
        assert_eq!(built_theory_sha256().len(), 64);
        for (name, built) in BUILT_SESSION_FILES {
            assert_eq!(
                fs::read(repository_rules().join(name)).unwrap(),
                built.as_bytes()
            );
        }
    }

    #[test]
    fn a_dropped_handle_stops_the_worker() {
        let data = TempDir::new("drop");
        let gate = Double::new(io);
        let stops = Arc::clone(&gate.stops);
        let (prover, events) = spawn(options(data.path(), &[], ready(gate)));
        assert_eq!(health(&events), ProverHealth::Ready);
        drop(prover);
        wait_until("the stop", || stops.load(Ordering::SeqCst) == 1);
        wait_until("the removal", || !data.path().join("prover").exists());
    }

    #[test]
    fn a_gone_main_loop_stops_the_worker() {
        let data = TempDir::new("gone");
        let gate = Double::new(io);
        let stops = Arc::clone(&gate.stops);
        let (prover, events) = spawn(options(data.path(), &[], ready(gate)));
        assert_eq!(health(&events), ProverHealth::Ready);
        drop(events);
        assert!(prover.prove(job()));
        wait_until("the stop", || stops.load(Ordering::SeqCst) == 1);
        wait_until("the removal", || !data.path().join("prover").exists());
        wait_until("the worker's end", || !prover.prove(job()));
    }
}
