//! The Isabelle prover adapter: a worker thread that runs the rule gate
//! (`fabric_broker_policy::BrokerGate`) for the core's proof jobs.
//!
//! The worker emits a proof report only when the gate checked exactly the job's action and
//! context on the rule theory this broker was built with ([`built_theory_sha256`]). Every other
//! path is a result with no report, which the core turns into `undecided`. The gate seam
//! (`Gate`, `Starter`) is crate-private; test doubles of it exist only under `cfg(test)` and
//! can only fail.
//!
//! At a stop the adapter kills, on Unix, what an orderly stop cannot end in time (PW 2.5, PW K1):
//! the worker kills a gate start in progress through its kill handle before it acknowledges, and
//! the calling thread kills the server of a check still running at the stop's instant.
//!
//! "PW" numbers refer to `docs/plans/calendar-desktop-prover-wiring-design.md`, whose
//! section 2 describes this module.

use std::fs;
use std::io::{self, Write as _};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use fabric_broker_core::ports::{ProofDone, ProofJob, ProverIdentity, ProverKind, ProverStatus};
use fabric_broker_policy::report::sha256_hex;
use fabric_broker_policy::theory::SESSION_ROOT;
use fabric_broker_policy::{
    Action, BrokerGate, Check, Context, GateConfig, KillHandle, PolicyError,
};
use fabric_desktop_contracts::owner::ProverProblem;

use self::map::{GateFate, JobRefused, Lateness, Refusal};
// The wall clock in Unix milliseconds, as the core's clock reads it before its offset.
use crate::shell::now_ms;

pub mod map;

/// The longest limit of one check (plan section 5). The adapter passes each check the smaller of
/// the time left before its job's deadline and the gate configuration's `check_timeout` (this
/// value), so the gate's own cap at the same value changes nothing (PW 2.3).
pub const CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// The waits after the first and the second consecutive failure; after a third the prover
/// stays down for the run.
pub const RESTART_WAITS: [std::time::Duration; 2] = [
    std::time::Duration::from_secs(10),
    std::time::Duration::from_secs(60),
];

/// How long a stop at quit waits for the worker; under the relay's 2 s drain.
pub const QUIT_BOUND: std::time::Duration = std::time::Duration::from_millis(1_500);

/// What a stop at quit keeps back from `QUIT_BOUND` after the stop's instant (PW X9, PW 2.5): for
/// the gate's kill at its bound, or, when a check still runs at that instant, for the kill of its
/// server through the kill handle, the check's failure, the broken gate's stop (which returns at
/// once) and the acknowledgement. The kill's helper process was measured at about 0.03 s on an
/// idle Mac.
const KILL_ALLOWANCE: Duration = Duration::from_millis(300);

/// The bound of every stop of a gate: past it the gate kills its server's process groups (Unix).
const GATE_STOP_BOUND: Duration = QUIT_BOUND.saturating_sub(KILL_ALLOWANCE);

/// The policy crate's hash of the rule theory this broker was built with.
pub use fabric_broker_policy::built_theory_sha256;

/// What the worker needs of a gate; `BrokerGate` in the product, failing doubles under
/// `cfg(test)`.
pub(crate) trait Gate: Send {
    /// Checks one action in one context within `limit` (`BrokerGate::check_within`).
    fn check_within(
        &mut self,
        context: &Context,
        action: &Action,
        limit: Duration,
    ) -> Result<Check, PolicyError>;
    /// Whether a failure broke the gate; its checks then fail at once (`BrokerGate::is_broken`).
    fn is_broken(&self) -> bool;
    /// Stops the gate's Isabelle server within `bound`, then kills it (`BrokerGate::stop_within`).
    fn stop_within(&mut self, bound: Duration) -> Result<(), PolicyError>;
    /// The SHA-256 of the rule theory the gate loaded, in lowercase hex.
    fn theory_sha256(&self) -> &str;
}

impl Gate for BrokerGate {
    fn check_within(
        &mut self,
        context: &Context,
        action: &Action,
        limit: Duration,
    ) -> Result<Check, PolicyError> {
        BrokerGate::check_within(self, context, action, limit)
    }

    fn is_broken(&self) -> bool {
        BrokerGate::is_broken(self)
    }

    fn stop_within(&mut self, bound: Duration) -> Result<(), PolicyError> {
        BrokerGate::stop_within(self, bound)
    }

    fn theory_sha256(&self) -> &str {
        BrokerGate::theory_sha256(self)
    }
}

/// Starts a gate from its configuration, killable through the kill handle it is given, a new one
/// for every start (PW 2.2, PW X7); the real starter calls `BrokerGate::start_with_kill`.
pub(crate) type Starter = std::sync::Arc<
    dyn Fn(&GateConfig, &KillHandle) -> Result<Box<dyn Gate>, PolicyError> + Send + Sync,
>;

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
    /// Starts a gate; `BrokerGate::start_with_kill` outside unit tests.
    pub(crate) starter: Starter,
}

impl ProverOptions {
    /// `gate_config(..)`, `RESTART_WAITS`, and a starter calling `BrokerGate::start_with_kill`.
    #[must_use]
    pub fn new(isabelle: &crate::config::IsabelleSettings, data_dir: &Path) -> ProverOptions {
        ProverOptions {
            gate: gate_config(isabelle, data_dir),
            restart_waits: RESTART_WAITS.to_vec(),
            starter: Arc::new(|config: &GateConfig, kill: &KillHandle| {
                BrokerGate::start_with_kill(config.clone(), kill)
                    .map(|gate| Box::new(gate) as Box<dyn Gate>)
            }),
        }
    }
}

/// What the worker hands to the main loop.
pub enum ProverEvent {
    /// The result of one job, boxed so health events do not carry its size.
    Done(Box<ProofDone>),
    /// The prover's status changed: its health and, while unavailable, why.
    Health(ProverStatus),
}

/// Returns false when the receiver is gone; the worker then stops.
pub type Emit = Box<dyn FnMut(ProverEvent) -> bool + Send>;

/// How long [`IsabelleProver::stop`] waits for the acknowledgement between two drains.
const STOP_SLICE: std::time::Duration = std::time::Duration::from_millis(20);

/// The stderr line when the rule theory is not the one this broker was built with.
const OTHER_THEORY_LINE: &str = "fabric-broker: the rule theory or its session files in FABRIC_BROKER_RULES_DIR are not the ones this broker was built with; the rule prover does not run in this run";

/// The error of a gate start that panicked; never written anywhere, its category is `I/O`.
const START_PANICKED: &str = "the gate's start panicked";

/// What reaches the worker thread.
enum Msg {
    /// A proof job from the shell.
    Job(Box<ProofJob>),
    /// The starter thread's result.
    Started(Result<Box<dyn Gate>, PolicyError>),
    /// Stop now, then acknowledge.
    Stop(Sender<()>),
}

/// What the worker, the main loop's overrun poll and the handle's stop share.
///
/// The overrun poll is a backstop: the gate answers by the check's limit and its grace, but a
/// hung gate is reported at the job's deadline, the gate does not bound its theory file's write
/// and removal, and a step of the wall clock can move the deadline.
struct Shared {
    /// The status last sent to the main loop, by the worker or through [`IsabelleProver::poll`].
    health: ProverStatus,
    /// The deadline of the check now running. `check_job` sets it only while no stop was sent,
    /// in the same critical section that reads `stop_by`, so once the stop's instant has passed
    /// it is set if and only if a check that began before the stop still runs (PW 2.5).
    running: Option<u64>,
    /// True once the poll reported the running check past its deadline.
    overrun: bool,
    /// Set once by the handle before it sends `Stop`: the instant by which a gate's stop must
    /// end. While it is set, queued jobs are not checked, no check and no start begins.
    stop_by: Option<Instant>,
    /// The kill handle of the latest gate start: of the start in progress, or of the gate it
    /// gave (PW 2.2). `None` until the first start is about to spawn its starter thread. It is
    /// never cleared: the handle of a server that was stopped and reaped reaches nothing.
    kill: Option<KillHandle>,
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
    /// The worker thread; `None` when it could not be spawned (PW 2.2).
    worker: Option<JoinHandle<()>>,
}

impl IsabelleProver {
    /// The identity the core is given at start: Isabelle, starting. Never blocks.
    #[must_use]
    pub fn identity() -> ProverIdentity {
        ProverIdentity {
            kind: ProverKind::Isabelle,
            health: ProverStatus::Starting,
        }
    }

    /// Spawns the worker. Called only once the store is open.
    ///
    /// The worker starts its first gate at once, unless a stop was already sent. If the thread
    /// cannot be spawned, every [`prove`](Self::prove) returns false, so the shell reports the
    /// prover unavailable.
    #[must_use]
    pub fn spawn(options: ProverOptions, emit: Emit) -> IsabelleProver {
        let (sender, messages) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Shared {
            health: ProverStatus::Starting,
            running: None,
            overrun: false,
            stop_by: None,
            kill: None,
        }));
        let work_dir = options.gate.work_dir.clone();
        let worker = Worker {
            options,
            emit,
            shared: Arc::clone(&shared),
            sender: sender.clone(),
            state: State::Starting,
            failures: 0,
            gone: false,
        };
        // A failed spawn drops the worker and its receiver: every send then fails.
        let worker = thread::Builder::new()
            .name("fabric-prover".to_owned())
            .spawn(move || worker.run(&messages))
            .ok();
        IsabelleProver {
            sender,
            shared,
            work_dir,
            worker,
        }
    }

    /// True when the worker is lost: its thread could not be spawned, or it has ended although
    /// no stop was sent, by a panic or because the main loop no longer takes its events. The
    /// shell asks this each turn (PW 2.2).
    #[must_use]
    pub fn lost(&self) -> bool {
        let ended = self.worker.as_ref().is_none_or(JoinHandle::is_finished);
        ended && lock(&self.shared).stop_by.is_none()
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

    /// `Some(Unavailable(None))` once when the running check is at or past its job's deadline:
    /// why is not known while the check still runs (PW 2.3).
    pub fn poll(&self, now_ms: u64) -> Option<ProverStatus> {
        let mut shared = lock(&self.shared);
        match shared.running {
            Some(deadline) if !shared.overrun && now_ms >= deadline => {
                shared.overrun = true;
                shared.health = ProverStatus::Unavailable(None);
                Some(ProverStatus::Unavailable(None))
            }
            _ => None,
        }
    }

    /// Stops the worker: sends the stop and waits up to `bound`, calling `drain` between
    /// short waits. Without an acknowledgement it removes the work directory itself and returns
    /// false.
    ///
    /// Before the send it records the instant by which the gate's stop must end: `bound` less
    /// `KILL_ALLOWANCE`, at most `GATE_STOP_BOUND`, so the gate's kill goes out within `bound`.
    ///
    /// The first time it finds that instant passed without an acknowledgement, it looks once at
    /// the shared record: a check that still runs then is not ended in time by any bound of the
    /// gate's, so on Unix it kills that check's server through the kill handle (PW 2.5, PW K1).
    /// The check then fails, the worker fails the broken gate as for any broken gate (its
    /// result, a failure line, health `unavailable`, no retry once a stop was sent), its stop
    /// returns at once, and the worker acknowledges. With no check running nothing is killed
    /// here: the worker is then in its gate's stop, where the gate kills at that same instant
    /// itself, or in its own kill of a start. It keeps waiting and draining until `bound` as
    /// before.
    pub fn stop(self, bound: std::time::Duration, drain: &mut dyn FnMut()) -> bool {
        let stop_by = *lock(&self.shared).stop_by.get_or_insert(
            Instant::now() + bound.saturating_sub(KILL_ALLOWANCE).min(GATE_STOP_BOUND),
        );
        let (ack, acknowledged) = mpsc::channel();
        if self.sender.send(Msg::Stop(ack)).is_ok() {
            let until = Instant::now().checked_add(bound);
            let mut looked = false;
            loop {
                let left = until.map_or(STOP_SLICE, |until| {
                    until.saturating_duration_since(Instant::now())
                });
                match acknowledged.recv_timeout(left.min(STOP_SLICE)) {
                    Ok(()) => return true,
                    Err(RecvTimeoutError::Timeout) => {
                        if !looked && Instant::now() >= stop_by {
                            looked = true;
                            self.kill_running_check();
                        }
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

    /// On Unix, kills the server of the check that still runs, if one does and a gate start
    /// gave a kill handle (PW 2.5). The kill runs on a short-lived thread, so the calling
    /// thread keeps its slices, its drain and its bound although the kill may wait up to 2 s
    /// for its helper; only if that thread cannot be spawned is the kill made here. Outside
    /// Unix the handle reaches nothing, and its flag alone would only make the gate's later
    /// stop skip its orderly steps, so nothing is killed there.
    fn kill_running_check(&self) {
        if !cfg!(unix) {
            return;
        }
        // The guard is dropped at the end of this statement: no lock is held across the kill.
        let kill = {
            let shared = lock(&self.shared);
            if shared.running.is_some() {
                shared.kill.clone()
            } else {
                None
            }
        };
        let Some(kill) = kill else {
            return;
        };
        let killing = kill.clone();
        let spawned = thread::Builder::new()
            .name("fabric-prover-kill".to_owned())
            .spawn(move || {
                let _ = killing.kill();
            });
        if spawned.is_err() {
            let _ = kill.kill();
        }
    }
}

impl Drop for IsabelleProver {
    /// A handle dropped without [`IsabelleProver::stop`] still stops the worker, without
    /// waiting, and gives the gate's stop `GATE_STOP_BOUND`. After a stop the worker is gone and
    /// the send fails.
    fn drop(&mut self) {
        lock(&self.shared)
            .stop_by
            .get_or_insert(Instant::now() + GATE_STOP_BOUND);
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
    fn refused(job: &ProofJob, refusal: Refusal) -> Checked {
        Checked {
            done: map::no_report(job, refusal),
            fate: GateFate::Keep,
            overrun: false,
            category: "",
        }
    }
}

/// The worker thread: it owns the gate; every gate start happens off it; a gate's stop is
/// bounded and runs on it.
struct Worker {
    options: ProverOptions,
    emit: Emit,
    shared: Arc<Mutex<Shared>>,
    /// For the starter threads.
    sender: Sender<Msg>,
    state: State,
    /// Consecutive failures since the last `Ready`.
    failures: usize,
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
            Msg::Stop(ack) => return Err(ack),
        }
        Ok(())
    }

    /// While `Down` with a retry and no stop sent: how long until the retry.
    fn retry_wait(&self) -> Option<std::time::Duration> {
        match self.state {
            State::Down { retry_at: Some(at) } if lock(&self.shared).stop_by.is_none() => {
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

    /// Sends `status` when it differs from the status last sent. The whole status is compared,
    /// so a change of cause alone is sent too.
    fn set_health(&mut self, status: ProverStatus) {
        {
            let mut shared = lock(&self.shared);
            if shared.health == status {
                return;
            }
            shared.health = status;
        }
        self.send(ProverEvent::Health(status));
    }

    /// Enters `Starting`: a fresh private work directory, the pin of PW 2.1, then a starter
    /// thread with a new kill handle, which `Shared.kill` holds from then on (PW 2.2).
    fn enter_starting(&mut self) {
        // No start begins once a stop was sent (PW 2.5); the `Stop` that follows ends the worker.
        if lock(&self.shared).stop_by.is_some() {
            return;
        }
        self.state = State::Starting;
        self.set_health(ProverStatus::Starting);
        if prepare_work_dir(&self.options.gate.work_dir).is_err() {
            self.fail(Failure::Start, "I/O");
            return;
        }
        match pin(&self.options.gate.rules_dir) {
            Err(_) => {
                self.fail(Failure::Start, "I/O");
                return;
            }
            Ok(false) => {
                self.other_theory();
                return;
            }
            Ok(true) => {}
        }
        // Every start gets a new handle: a killed handle stays killed (PW X7). A stop sent during
        // the work directory's preparation or the pin starts no gate either; one sent after this
        // section finds the handle, so the worker's stop can kill the start (PW 2.5).
        let kill = KillHandle::new();
        {
            let mut shared = lock(&self.shared);
            if shared.stop_by.is_some() {
                return;
            }
            shared.kill = Some(kill.clone());
        }
        let starter = Arc::clone(&self.options.starter);
        let config = self.options.gate.clone();
        let sender = self.sender.clone();
        let shared = Arc::clone(&self.shared);
        let spawned = thread::Builder::new()
            .name("fabric-prover-start".to_owned())
            .spawn(move || {
                // After a stop the worker is gone, and on Unix it killed this start's handle
                // before it acknowledged (PW 2.2 step 3, PW K1): a server the start spawned is
                // ended, and the start returns an error. A gate it still returns is stopped here
                // within `stop_bound`, which with the killed handle returns at once and sends
                // `shutdown` to a server found already running. What is left: if the process ends
                // before the start returns, a found server outlives it. A start that panics is a
                // failed start, so the worker goes through `fail` (PW 2.2).
                let started = panic::catch_unwind(AssertUnwindSafe(|| starter(&config, &kill)))
                    .unwrap_or_else(|_| Err(PolicyError::Io(io::Error::other(START_PANICKED))));
                if let Err(mpsc::SendError(Msg::Started(Ok(mut gate)))) =
                    sender.send(Msg::Started(started))
                {
                    let _ = gate.stop_within(stop_bound(&shared));
                }
            });
        if spawned.is_err() {
            self.fail(Failure::Start, "I/O");
        }
    }

    fn started(&mut self, started: Result<Box<dyn Gate>, PolicyError>) {
        if !matches!(self.state, State::Starting) {
            if let Ok(gate) = started {
                stop_gate(gate, &self.shared);
            }
            return;
        }
        match started {
            Ok(gate) if gate.theory_sha256() != built_theory_sha256() => {
                self.other_theory();
                stop_gate(gate, &self.shared);
            }
            Ok(gate) => {
                self.state = State::Ready(gate);
                self.failures = 0;
                self.set_health(ProverStatus::Ready);
            }
            Err(error) => self.fail(Failure::Start, category(&error)),
        }
    }

    fn job(&mut self, job: &ProofJob) {
        // Read before the match: `check_job` locks the record again.
        let stop_sent = lock(&self.shared).stop_by.is_some();
        let checked = match &mut self.state {
            State::Starting => Checked::refused(job, map::STARTING),
            State::Down { .. } => Checked::refused(job, map::NOT_RUNNING),
            // A job still queued at a stop is not checked.
            State::Ready(_) if stop_sent => Checked::refused(job, map::NOT_RUNNING),
            State::Ready(gate) => check_job(
                gate.as_mut(),
                &self.shared,
                self.options.gate.check_timeout,
                job,
            ),
        };
        self.send(ProverEvent::Done(Box::new(checked.done)));
        match checked.fate {
            GateFate::Restart => {
                let broken = std::mem::replace(&mut self.state, State::Down { retry_at: None });
                self.fail(Failure::Broken, checked.category);
                if let State::Ready(gate) = broken {
                    stop_gate(gate, &self.shared);
                }
            }
            GateFate::Keep if checked.overrun => self.set_health(ProverStatus::Ready),
            GateFate::Keep => {}
        }
    }

    /// The failure rule: wait the n-th wait after the n-th consecutive failure, and after
    /// the last wait stay down for the run. No error text, path or calendar value is written.
    ///
    /// The status says whether a retry comes: `failed_will_retry` while one is scheduled and no
    /// stop was sent; `failed` after the last wait or once a stop was sent, since no retry starts
    /// then (PW 2.2).
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
        let will_retry = matches!(self.state, State::Down { retry_at: Some(_) })
            && lock(&self.shared).stop_by.is_none();
        self.set_health(ProverStatus::Unavailable(Some(if will_retry {
            ProverProblem::FailedWillRetry
        } else {
            ProverProblem::Failed
        })));
    }

    /// The rule theory is not the built one: down for the run, nothing is proved.
    fn other_theory(&mut self) {
        self.state = State::Down { retry_at: None };
        stderr_line(OTHER_THEORY_LINE);
        self.set_health(ProverStatus::Unavailable(Some(ProverProblem::WrongRules)));
    }

    /// At a stop, or when the channel is closed or the main loop is gone: stops a ready gate
    /// here, within what is left of the stop's instant; on Unix kills a start in progress
    /// through its kill handle; then removes the work directory. The stop is acknowledged only
    /// after this (PW 2.5).
    ///
    /// The kill refuses a start that has not spawned its server yet, and ends a server being
    /// started, so the start returns an error. A gate already started whose `Started` is in the
    /// channel, or that its starter cannot send any more, has the killed handle: its stop, by
    /// the channel's drop or by the starter, returns at once. The kill does not reach a server
    /// `isabelle server` found already running: its `shutdown` is sent by whoever stops its
    /// gate, after a quit during its start only after the acknowledgement (by the starter thread,
    /// or by the worker thread as it drops the channel), so it goes out only if the process is
    /// still alive (PW K1). A server this broker spawned was ended before the acknowledgement.
    /// In `Starting` `Shared.kill` is the handle of the start in progress, or, when a stop came
    /// before its starter thread, a stale one whose server was stopped, which reaches nothing,
    /// or none when that start was the first.
    fn shut_down(&mut self) {
        match std::mem::replace(&mut self.state, State::Down { retry_at: None }) {
            State::Ready(gate) => stop_gate(gate, &self.shared),
            // Outside Unix the handle reaches nothing, and its flag alone would only make the
            // gate's later stop skip its orderly steps.
            State::Starting if cfg!(unix) => {
                // Cloned out first: no lock on `Shared` is held across the kill.
                let kill = lock(&self.shared).kill.clone();
                if let Some(kill) = kill {
                    let _ = kill.kill();
                }
            }
            State::Starting | State::Down { .. } => {}
        }
        let _ = remove_work_dir(&self.options.gate.work_dir);
    }
}

/// What a stop of a gate may take now: `GATE_STOP_BOUND`, or what is left of the quit's deadline
/// if a stop was sent (zero: the gate kills its server at once).
fn stop_bound(shared: &Mutex<Shared>) -> Duration {
    lock(shared).stop_by.map_or(GATE_STOP_BOUND, |by| {
        by.saturating_duration_since(Instant::now())
            .min(GATE_STOP_BOUND)
    })
}

/// Stops a gate on the worker within [`stop_bound`] and its kill (PW 2.2), then drops it.
fn stop_gate(mut gate: Box<dyn Gate>, shared: &Mutex<Shared>) {
    let _ = gate.stop_within(stop_bound(shared));
    drop(gate);
}

/// One job in `Ready`: refused without a check once a stop was sent, past its deadline, or when
/// it cannot be mapped; otherwise checked within the time left before its deadline, at most
/// `check_timeout` (PW 2.3).
///
/// The adapter owns the limit it passes and times the call itself (PW 2.3): an error or an
/// undecided verdict states `DeadlinePassed` when the clock is then at or past the job's
/// deadline, else `CheckLimitReached` when the call took at least that limit. No text of the
/// gate is read for this.
fn check_job(
    gate: &mut dyn Gate,
    shared: &Mutex<Shared>,
    check_timeout: Duration,
    job: &ProofJob,
) -> Checked {
    // One clock read for the refusal and the limit: the time left before the job's deadline.
    let left = job.deadline_ms.saturating_sub(now_ms());
    if left == 0 {
        return Checked::refused(job, map::PAST_DEADLINE);
    }
    let limit = Duration::from_millis(left).min(check_timeout);
    let (context, action) = match map::context_of(job) {
        Ok(checkable) => checkable,
        Err(JobRefused(reason)) => return Checked::refused(job, reason),
    };
    {
        let mut shared = lock(shared);
        // No check starts once a stop was sent (PW 2.5). Read where `running` is set, so a check
        // that runs at the stop's instant had set it before, and the handle's one look at that
        // instant sees it (`IsabelleProver::stop`). `Worker::job`'s earlier read refuses queued
        // jobs first; this one closes the gap between that read and here.
        if shared.stop_by.is_some() {
            return Checked::refused(job, map::NOT_RUNNING);
        }
        shared.running = Some(job.deadline_ms);
        shared.overrun = false;
    }
    let began = Instant::now();
    let result = gate.check_within(&context, &action, limit);
    let took = began.elapsed();
    let lateness = if now_ms() >= job.deadline_ms {
        Lateness::PastDeadline
    } else if took >= limit {
        Lateness::AtLimit
    } else {
        Lateness::InTime
    };
    // A broken gate is restarted whatever the result; an `Ok` check's result stands.
    let broken = gate.is_broken();
    let overrun = {
        let mut shared = lock(shared);
        shared.running = None;
        std::mem::take(&mut shared.overrun)
    };
    let category = match &result {
        Err(error) => category(error),
        // The check returned, but the purge after it broke the gate.
        Ok(_) if broken => "connection",
        Ok(_) => "Isabelle",
    };
    let (done, fate) = map::done_of(job, &context, built_theory_sha256(), result, lateness);
    let fate = if broken { GateFate::Restart } else { fate };
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

/// The pin of PW 2.1: true when `<rules_dir>/Fabric_Broker` holds the theory this broker was
/// built with (by its SHA-256) and the built session `ROOT` (byte for byte), and no `ROOTS`,
/// which would add another directory to the session's. Nothing else under `rules_dir` is read.
fn pin(rules_dir: &Path) -> io::Result<bool> {
    let session = rules_dir.join(fabric_broker_policy::gate::SESSION);
    if sha256_hex(&fs::read(session.join("Fabric_Broker.thy"))?) != built_theory_sha256() {
        return Ok(false);
    }
    if fs::read(session.join("ROOT"))? != SESSION_ROOT.as_bytes() {
        return Ok(false);
    }
    match fs::symlink_metadata(session.join("ROOTS")) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error),
        Ok(_) => Ok(false),
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
    //! The worker with gate doubles. A double's `check_within` only fails, or blocks and then
    //! fails; no double returns a `Check`.

    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Condvar, mpsc::RecvTimeoutError};
    use std::time::Duration;

    use fabric_broker_policy::gate::CHECK_GRACE;
    use fabric_broker_policy::theory::THEORY_TEXT;

    use super::*;
    use crate::config::IsabelleSettings;
    use crate::prover::map::tests::{create, job_of};

    /// How long a test waits for an event that must come.
    const PATIENCE: Duration = Duration::from_secs(10);
    /// How long a test waits to see that an event does not come.
    const QUIET: Duration = Duration::from_millis(250);
    /// How long a double's check waits for its kill handle at most.
    const KILLED_CHECK_SAFETY: Duration = Duration::from_secs(5);

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
        /// The check breaks the gate before it fails.
        breaks: bool,
        /// What `is_broken` returns: set by a check that breaks the gate.
        broken: Arc<AtomicBool>,
        checks: Arc<AtomicUsize>,
        stops: Arc<AtomicUsize>,
        /// The limit of each check.
        limits: Arc<Mutex<Vec<Duration>>>,
        /// The bound of each stop.
        stop_bounds: Arc<Mutex<Vec<Duration>>>,
        /// The kill handle of the start that gave this gate, set by a starter that hands it
        /// on. The check then waits until it is killed (polling, up to a safety timeout),
        /// breaks the gate and fails with an I/O error, one of the errors a gate's check gives
        /// when its server is killed.
        kill: Option<KillHandle>,
        /// The stop returns only this long after its bound, as a gate that kills its server at
        /// its bound does.
        stop_overrun: Option<Duration>,
    }

    impl Double {
        fn new(error: fn() -> PolicyError) -> Double {
            Double {
                theory: built_theory_sha256().to_owned(),
                error,
                check_latch: None,
                stop_latch: None,
                check_file: None,
                breaks: false,
                broken: Arc::default(),
                checks: Arc::default(),
                stops: Arc::default(),
                limits: Arc::default(),
                stop_bounds: Arc::default(),
                kill: None,
                stop_overrun: None,
            }
        }
    }

    impl Gate for Double {
        fn check_within(
            &mut self,
            _: &Context,
            _: &Action,
            limit: Duration,
        ) -> Result<Check, PolicyError> {
            self.limits.lock().unwrap().push(limit);
            self.checks.fetch_add(1, Ordering::SeqCst);
            if let Some(file) = &self.check_file {
                fs::write(file, "calendar data").unwrap();
            }
            if self.breaks {
                self.broken.store(true, Ordering::SeqCst);
            }
            if let Some(kill) = &self.kill {
                let until = Instant::now() + KILLED_CHECK_SAFETY;
                while !kill.is_killed() && Instant::now() < until {
                    thread::sleep(Duration::from_millis(5));
                }
                self.broken.store(true, Ordering::SeqCst);
                return Err(io());
            }
            if let Some(latch) = &self.check_latch {
                latch.wait();
            }
            Err((self.error)())
        }

        fn is_broken(&self) -> bool {
            self.broken.load(Ordering::SeqCst)
        }

        fn stop_within(&mut self, bound: Duration) -> Result<(), PolicyError> {
            self.stop_bounds.lock().unwrap().push(bound);
            if let Some(overrun) = self.stop_overrun {
                thread::sleep(bound + overrun);
            }
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

    /// A starter that counts its calls and starts each gate with `start(call, kill)`, `call`
    /// from 1, `kill` the handle of that start.
    fn counting<F>(calls: &Arc<AtomicUsize>, start: F) -> Starter
    where
        F: Fn(usize, &KillHandle) -> Result<Box<dyn Gate>, PolicyError> + Send + Sync + 'static,
    {
        let calls = Arc::clone(calls);
        Arc::new(move |_: &GateConfig, kill: &KillHandle| {
            start(calls.fetch_add(1, Ordering::SeqCst) + 1, kill)
        })
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

    fn status(events: &Receiver<ProverEvent>) -> ProverStatus {
        match events.recv_timeout(PATIENCE).expect("a health event") {
            ProverEvent::Health(status) => status,
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

    /// The fixed refusal of a result with no report, found by its text; its cause must be the
    /// refusal's.
    fn refusal(done: &ProofDone) -> Refusal {
        assert!(done.report.is_none() && done.ruling.is_none());
        assert!(done.authorizing_grants.is_empty() && done.check_theory_text.is_none());
        let text = done.undecided_reason.as_deref().unwrap();
        let refusal = *map::tests::REFUSALS
            .iter()
            .find(|refusal| refusal.text == text)
            .unwrap_or_else(|| panic!("not a fixed text: {text}"));
        assert_eq!(done.cause, Some(refusal.cause), "{text}");
        refusal
    }

    fn ready(gate: Double) -> Starter {
        Arc::new(move |_: &GateConfig, _: &KillHandle| Ok(Box::new(gate.clone()) as Box<dyn Gate>))
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
        let starter: Starter = Arc::new(move |_: &GateConfig, _: &KillHandle| {
            waiting.wait();
            Ok(Box::new(gate.clone()) as Box<dyn Gate>)
        });
        let (prover, events) = spawn(options(data.path(), &[], starter));
        assert_eq!(IsabelleProver::identity().health, ProverStatus::Starting);
        assert_eq!(IsabelleProver::identity().kind, ProverKind::Isabelle);
        quiet(&events);
        assert!(prover.prove(job()));
        assert_eq!(refusal(&done(&events)), map::STARTING);
        latch.open();
        assert_eq!(status(&events), ProverStatus::Ready);
        assert!(stop(prover));
    }

    #[test]
    fn the_nth_failure_waits_the_nth_wait_and_the_third_leaves_the_prover_unavailable() {
        let data = TempDir::new("w02");
        let calls = Arc::new(AtomicUsize::new(0));
        let times = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&times);
        let starter = counting(&calls, move |_, _| {
            recorded.lock().unwrap().push(Instant::now());
            Err(isabelle())
        });
        let waits = [Duration::from_millis(100), Duration::from_millis(300)];
        let (prover, events) = spawn(options(data.path(), &waits, starter));
        for expected in [
            ProverStatus::Unavailable(Some(ProverProblem::FailedWillRetry)),
            ProverStatus::Starting,
            ProverStatus::Unavailable(Some(ProverProblem::FailedWillRetry)),
            ProverStatus::Starting,
            ProverStatus::Unavailable(Some(ProverProblem::Failed)),
        ] {
            assert_eq!(status(&events), expected);
        }
        thread::sleep(Duration::from_millis(500));
        quiet(&events);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        let times = times.lock().unwrap().clone();
        assert!(times[1] - times[0] >= waits[0]);
        assert!(times[2] - times[1] >= waits[1]);
        assert!(prover.prove(job()));
        assert_eq!(refusal(&done(&events)), map::NOT_RUNNING);
        assert!(stop(prover));
    }

    #[test]
    fn jobs_while_not_ready_are_answered_at_once_with_no_report() {
        let data = TempDir::new("w03");
        let latch = Latch::default();
        let waiting = latch.clone();
        let starter: Starter = Arc::new(move |_: &GateConfig, _: &KillHandle| {
            waiting.wait();
            Err(isabelle())
        });
        let (prover, events) = spawn(options(data.path(), &[], starter));
        assert!(prover.prove(job()));
        assert_eq!(refusal(&done(&events)), map::STARTING);
        latch.open();
        assert_eq!(
            status(&events),
            ProverStatus::Unavailable(Some(ProverProblem::Failed))
        );
        let mut second = job();
        second.job_id = 8;
        assert!(prover.prove(second));
        let done = done(&events);
        assert_eq!(done.job_id, 8);
        assert_eq!(refusal(&done), map::NOT_RUNNING);
        assert!(stop(prover));
    }

    #[test]
    fn a_job_past_its_deadline_is_not_checked() {
        let data = TempDir::new("w04");
        let gate = Double::new(io);
        let checks = Arc::clone(&gate.checks);
        let (prover, events) = spawn(options(data.path(), &[], ready(gate)));
        assert_eq!(status(&events), ProverStatus::Ready);
        let mut late = job();
        late.deadline_ms = now_ms();
        assert!(prover.prove(late));
        assert_eq!(refusal(&done(&events)), map::PAST_DEADLINE);
        quiet(&events);
        assert_eq!(checks.load(Ordering::SeqCst), 0);
        assert!(stop(prover));
    }

    #[test]
    fn a_broken_gate_is_stopped_on_the_worker_and_restarted() {
        let data = TempDir::new("w05");
        let gate = Double::new(io);
        let stops = Arc::clone(&gate.stops);
        let stop_bounds = Arc::clone(&gate.stop_bounds);
        let calls = Arc::new(AtomicUsize::new(0));
        let starter = counting(&calls, move |_, _| {
            Ok(Box::new(gate.clone()) as Box<dyn Gate>)
        });
        let waits = [Duration::from_millis(50)];
        let (prover, events) = spawn(options(data.path(), &waits, starter));
        assert_eq!(status(&events), ProverStatus::Ready);
        assert!(prover.prove(job()));
        assert_eq!(refusal(&done(&events)), map::FAILED);
        for expected in [
            ProverStatus::Unavailable(Some(ProverProblem::FailedWillRetry)),
            ProverStatus::Starting,
            ProverStatus::Ready,
        ] {
            assert_eq!(status(&events), expected);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(stops.load(Ordering::SeqCst), 1);
        assert_eq!(stop_bounds.lock().unwrap()[0], GATE_STOP_BOUND);
        assert!(stop(prover));
        assert_eq!(stops.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_render_error_keeps_the_gate() {
        let data = TempDir::new("w06");
        let gate = Double::new(render);
        let checks = Arc::clone(&gate.checks);
        let calls = Arc::new(AtomicUsize::new(0));
        let starter = counting(&calls, move |_, _| {
            Ok(Box::new(gate.clone()) as Box<dyn Gate>)
        });
        let (prover, events) = spawn(options(data.path(), &[], starter));
        assert_eq!(status(&events), ProverStatus::Ready);
        for _ in 0..2 {
            assert!(prover.prove(job()));
            assert_eq!(refusal(&done(&events)), map::RENDER);
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
        let starter: Starter = Arc::new(move |_: &GateConfig, _: &KillHandle| {
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
        assert_eq!(status(&events), ProverStatus::Ready);
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
        let starter: Starter = Arc::new(move |_: &GateConfig, _: &KillHandle| {
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
        assert_eq!(status(&events), ProverStatus::Ready);
        let work_dir = data.path().join("prover");
        assert!(work_dir.is_dir());
        assert!(stop(prover));
        assert!(!work_dir.exists());
        assert_eq!(stops.load(Ordering::SeqCst), 1);

        // Down.
        let data = TempDir::new("w08-down");
        let starter: Starter = Arc::new(|_: &GateConfig, _: &KillHandle| Err(isabelle()));
        let (prover, events) = spawn(options(data.path(), &[], starter));
        assert_eq!(
            status(&events),
            ProverStatus::Unavailable(Some(ProverProblem::Failed))
        );
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
        assert_eq!(
            status(&events),
            ProverStatus::Unavailable(Some(ProverProblem::Failed))
        );
        assert!(prover.prove(job()));
        assert_eq!(refusal(&done(&events)), map::NOT_RUNNING);
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
            format!("{THEORY_TEXT} "),
        )
        .unwrap();
        fs::write(rules.join("Fabric_Broker").join("ROOT"), SESSION_ROOT).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let starter = counting(&calls, |_, _| Err(isabelle()));
        let mut options = options(data.path(), &[Duration::from_millis(10)], starter);
        options.gate.rules_dir = rules;
        let (prover, events) = spawn(options);
        assert_eq!(
            status(&events),
            ProverStatus::Unavailable(Some(ProverProblem::WrongRules))
        );
        quiet(&events);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(prover.prove(job()));
        assert_eq!(refusal(&done(&events)), map::NOT_RUNNING);
        assert!(stop(prover));
    }

    /// A rules directory with the built theory, the built session `ROOT`, and a `Rules2/`
    /// beside them with a changed theory.
    fn rules_with_a_copy(data: &Path) -> PathBuf {
        let rules = data.join("rules");
        for dir in ["Fabric_Broker", "Rules2"] {
            fs::create_dir_all(rules.join(dir)).unwrap();
        }
        fs::write(
            rules.join("Fabric_Broker").join("Fabric_Broker.thy"),
            THEORY_TEXT,
        )
        .unwrap();
        fs::write(rules.join("Fabric_Broker").join("ROOT"), SESSION_ROOT).unwrap();
        fs::write(
            rules.join("Rules2").join("Fabric_Broker.thy"),
            format!("{THEORY_TEXT} "),
        )
        .unwrap();
        fs::write(rules.join("Rules2").join("ROOT"), SESSION_ROOT).unwrap();
        rules
    }

    /// The theory pin: the built theory stays in place, but the session's `ROOT` points the
    /// session at a copied, changed theory, or a `ROOTS` beside it adds a directory. The prover
    /// is unavailable for the run and no gate starts.
    #[test]
    fn session_files_that_point_elsewhere_leave_the_prover_unavailable_and_start_nothing() {
        let redirects: [(&str, &str); 2] = [
            (
                "Fabric_Broker/ROOT",
                "session Fabric_Broker in \"../Rules2\" = HOL +\n  theories\n    Fabric_Broker\n",
            ),
            ("Fabric_Broker/ROOTS", "../Rules2\n"),
        ];
        for (file, text) in redirects {
            let data = TempDir::new("w11-session");
            let rules = rules_with_a_copy(data.path());
            fs::write(rules.join(file), text).unwrap();
            let calls = Arc::new(AtomicUsize::new(0));
            let starter = counting(&calls, |_, _| Err(isabelle()));
            let mut options = options(data.path(), &[Duration::from_millis(10)], starter);
            options.gate.rules_dir = rules;
            let (prover, events) = spawn(options);
            assert_eq!(
                status(&events),
                ProverStatus::Unavailable(Some(ProverProblem::WrongRules)),
                "{file}"
            );
            quiet(&events);
            assert_eq!(calls.load(Ordering::SeqCst), 0, "{file}");
            assert!(prover.prove(job()));
            assert_eq!(refusal(&done(&events)), map::NOT_RUNNING);
            assert!(stop(prover));
        }
    }

    /// PW 2.1: the pin reads only `Fabric_Broker/`, so a rules directory without `ROOTS`, or
    /// with one that names another directory, reaches the starter.
    #[test]
    fn a_rules_dir_without_roots_reaches_the_starter() {
        for with_roots in [false, true] {
            let data = TempDir::new("w21");
            let rules = data.path().join("rules");
            fs::create_dir_all(rules.join("Fabric_Broker")).unwrap();
            fs::write(
                rules.join("Fabric_Broker").join("Fabric_Broker.thy"),
                THEORY_TEXT,
            )
            .unwrap();
            fs::write(rules.join("Fabric_Broker").join("ROOT"), SESSION_ROOT).unwrap();
            if with_roots {
                fs::write(rules.join("ROOTS"), "Rules2\n").unwrap();
            }
            let calls = Arc::new(AtomicUsize::new(0));
            let starter = counting(&calls, |_, _| Err(isabelle()));
            let mut options = options(data.path(), &[], starter);
            options.gate.rules_dir = rules;
            let (prover, events) = spawn(options);
            assert_eq!(
                status(&events),
                ProverStatus::Unavailable(Some(ProverProblem::Failed)),
                "{with_roots}"
            );
            assert_eq!(calls.load(Ordering::SeqCst), 1, "{with_roots}");
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
        let starter = counting(&calls, move |_, _| {
            Ok(Box::new(gate.clone()) as Box<dyn Gate>)
        });
        let (prover, events) = spawn(options(data.path(), &[Duration::from_millis(10)], starter));
        assert_eq!(
            status(&events),
            ProverStatus::Unavailable(Some(ProverProblem::WrongRules))
        );
        assert!(prover.prove(job()));
        assert_eq!(refusal(&done(&events)), map::NOT_RUNNING);
        quiet(&events);
        wait_until("the stop", || stops.load(Ordering::SeqCst) == 1);
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
        let stops = Arc::clone(&gate.stops);
        let stop_bounds = Arc::clone(&gate.stop_bounds);
        let (prover, events) = spawn(options(data.path(), &[], ready(gate)));
        assert_eq!(status(&events), ProverStatus::Ready);
        assert!(prover.prove(job()));
        wait_until("the check file", || check_file.is_file());
        assert!(!prover.stop(Duration::from_millis(100), &mut || {}));
        assert!(!check_file.exists());
        assert!(!work_dir.exists());
        latch.open();
        // The check then breaks the gate: its stop gets what is left of the stop's instant, which
        // is nothing; the handle's drop at the end of `stop` did not move that instant.
        wait_until("the stop", || stops.load(Ordering::SeqCst) == 1);
        assert_eq!(*stop_bounds.lock().unwrap(), [Duration::ZERO]);
    }

    #[test]
    fn a_restart_waits_until_the_broken_gate_is_dropped() {
        let data = TempDir::new("w14");
        let stop_latch = Latch::default();
        let mut broken = Double::new(io);
        broken.stop_latch = Some(stop_latch.clone());
        let calls = Arc::new(AtomicUsize::new(0));
        let starter = counting(&calls, move |call, _| {
            let gate = if call == 1 {
                broken.clone()
            } else {
                Double::new(io)
            };
            Ok(Box::new(gate) as Box<dyn Gate>)
        });
        let (prover, events) = spawn(options(data.path(), &[Duration::from_millis(10)], starter));
        assert_eq!(status(&events), ProverStatus::Ready);
        assert!(prover.prove(job()));
        assert_eq!(refusal(&done(&events)), map::FAILED);
        assert_eq!(
            status(&events),
            ProverStatus::Unavailable(Some(ProverProblem::FailedWillRetry))
        );
        quiet(&events);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        stop_latch.open();
        assert_eq!(status(&events), ProverStatus::Starting);
        assert_eq!(status(&events), ProverStatus::Ready);
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
        assert_eq!(status(&events), ProverStatus::Ready);
        assert_eq!(prover.next_due_ms(), None);
        assert_eq!(prover.poll(u64::MAX), None);
        let job = job();
        let deadline = job.deadline_ms;
        assert!(prover.prove(job));
        wait_until("the running check", || {
            prover.next_due_ms() == Some(deadline)
        });
        assert_eq!(prover.poll(deadline - 1), None);
        assert_eq!(prover.poll(deadline), Some(ProverStatus::Unavailable(None)));
        assert_eq!(prover.poll(deadline + 1), None);
        assert_eq!(prover.next_due_ms(), None);
        latch.open();
        assert_eq!(refusal(&done(&events)), map::RENDER);
        assert_eq!(status(&events), ProverStatus::Ready);
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
        let starter: Starter = Arc::new(move |config: &GateConfig, _: &KillHandle| {
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
            ProverStatus::Unavailable(Some(ProverProblem::FailedWillRetry)),
            ProverStatus::Starting,
            ProverStatus::Unavailable(Some(ProverProblem::Failed)),
        ] {
            assert_eq!(status(&events), expected);
        }
        assert_eq!(*seen.lock().unwrap(), [(0, 0o700), (0, 0o700)]);
        assert!(stop(prover));
        assert!(!work_dir.exists());
    }

    #[test]
    fn the_built_theory_is_the_repository_theory() {
        assert!(pin(&repository_rules()).unwrap());
        let file = repository_rules()
            .join("Fabric_Broker")
            .join("Fabric_Broker.thy");
        assert_eq!(fs::read(file).unwrap(), THEORY_TEXT.as_bytes());
        assert_eq!(built_theory_sha256().len(), 64);
    }

    #[test]
    fn a_dropped_handle_stops_the_worker() {
        let data = TempDir::new("drop");
        let gate = Double::new(io);
        let stops = Arc::clone(&gate.stops);
        let (prover, events) = spawn(options(data.path(), &[], ready(gate)));
        assert_eq!(status(&events), ProverStatus::Ready);
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
        assert_eq!(status(&events), ProverStatus::Ready);
        drop(events);
        assert!(prover.prove(job()));
        wait_until("the stop", || stops.load(Ordering::SeqCst) == 1);
        wait_until("the removal", || !data.path().join("prover").exists());
        wait_until("the worker's end", || !prover.prove(job()));
    }

    /// PW 2.3: `is_broken` is read after the check, and a broken gate is restarted although
    /// its error alone (a render error) would keep it.
    #[test]
    fn a_gate_broken_by_a_check_is_restarted_even_when_its_error_would_keep_it() {
        let data = TempDir::new("w22");
        let mut first = Double::new(render);
        first.breaks = true;
        let stops = Arc::clone(&first.stops);
        let calls = Arc::new(AtomicUsize::new(0));
        let starter = counting(&calls, move |call, _| {
            let gate = if call == 1 {
                first.clone()
            } else {
                Double::new(render)
            };
            Ok(Box::new(gate) as Box<dyn Gate>)
        });
        let (prover, events) = spawn(options(data.path(), &[Duration::from_millis(50)], starter));
        assert_eq!(status(&events), ProverStatus::Ready);
        assert!(prover.prove(job()));
        assert_eq!(refusal(&done(&events)), map::RENDER);
        for expected in [
            ProverStatus::Unavailable(Some(ProverProblem::FailedWillRetry)),
            ProverStatus::Starting,
            ProverStatus::Ready,
        ] {
            assert_eq!(status(&events), expected);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(stops.load(Ordering::SeqCst), 1);
        assert!(stop(prover));
    }

    /// PW 2.3: a check's limit is the time left before its job's deadline.
    #[test]
    fn each_check_gets_the_time_left_before_its_deadline() {
        let data = TempDir::new("w23");
        let gate = Double::new(render);
        let limits = Arc::clone(&gate.limits);
        let (prover, events) = spawn(options(data.path(), &[], ready(gate)));
        assert_eq!(status(&events), ProverStatus::Ready);
        let mut due = job();
        due.deadline_ms = now_ms() + 20_000;
        assert!(prover.prove(due));
        assert_eq!(refusal(&done(&events)), map::RENDER);
        let limits = limits.lock().unwrap().clone();
        assert_eq!(limits.len(), 1);
        assert!(
            limits[0] > Duration::from_secs(19) && limits[0] <= Duration::from_secs(20),
            "{limits:?}"
        );
        assert!(stop(prover));
    }

    /// PW 2.5: a quit gives the gate's stop what is left of `QUIT_BOUND` less `KILL_ALLOWANCE`,
    /// counted from the send.
    #[test]
    fn a_quit_gives_the_gate_what_is_left_of_the_quit_bound() {
        // An idle gate.
        let data = TempDir::new("w24-idle");
        let gate = Double::new(render);
        let stop_bounds = Arc::clone(&gate.stop_bounds);
        let (prover, events) = spawn(options(data.path(), &[], ready(gate)));
        assert_eq!(status(&events), ProverStatus::Ready);
        assert!(prover.stop(QUIT_BOUND, &mut || {}));
        let bounds = stop_bounds.lock().unwrap().clone();
        assert_eq!(bounds.len(), 1);
        assert!(bounds[0] <= GATE_STOP_BOUND, "{bounds:?}");

        // A gate in a check that ends at least 800 ms after the stop was sent.
        let data = TempDir::new("w24-check");
        let latch = Latch::default();
        let mut gate = Double::new(render);
        gate.check_latch = Some(latch.clone());
        let checks = Arc::clone(&gate.checks);
        let stop_bounds = Arc::clone(&gate.stop_bounds);
        let (prover, events) = spawn(options(data.path(), &[], ready(gate)));
        assert_eq!(status(&events), ProverStatus::Ready);
        assert!(prover.prove(job()));
        wait_until("the check", || checks.load(Ordering::SeqCst) == 1);
        let mut first_drain: Option<Instant> = None;
        assert!(prover.stop(PATIENCE, &mut || {
            let first = *first_drain.get_or_insert_with(Instant::now);
            if first.elapsed() >= Duration::from_millis(800) {
                latch.open();
            }
        }));
        let bounds = stop_bounds.lock().unwrap().clone();
        assert_eq!(bounds.len(), 1);
        assert!(
            bounds[0] <= GATE_STOP_BOUND - Duration::from_millis(800),
            "{bounds:?}"
        );

        // A stop whose bound is the kill's allowance leaves the gate's stop no time.
        let data = TempDir::new("w24-allowance");
        let gate = Double::new(render);
        let stops = Arc::clone(&gate.stops);
        let stop_bounds = Arc::clone(&gate.stop_bounds);
        let (prover, events) = spawn(options(data.path(), &[], ready(gate)));
        assert_eq!(status(&events), ProverStatus::Ready);
        let _ = prover.stop(KILL_ALLOWANCE, &mut || {});
        wait_until("the stop", || stops.load(Ordering::SeqCst) == 1);
        assert_eq!(*stop_bounds.lock().unwrap(), [Duration::ZERO]);
    }

    /// PW 2.5: jobs still queued when a stop is sent are answered without a check.
    #[test]
    fn queued_jobs_are_not_checked_once_a_stop_is_sent() {
        for dropped in [false, true] {
            let data = TempDir::new("w25");
            let latch = Latch::default();
            let mut gate = Double::new(render);
            gate.check_latch = Some(latch.clone());
            let checks = Arc::clone(&gate.checks);
            let (prover, events) = spawn(options(data.path(), &[], ready(gate)));
            assert_eq!(status(&events), ProverStatus::Ready);
            assert!(prover.prove(job()));
            wait_until("the check", || checks.load(Ordering::SeqCst) == 1);
            assert!(prover.prove(job()));
            assert!(prover.prove(job()));
            if dropped {
                drop(prover);
                latch.open();
            } else {
                assert!(prover.stop(PATIENCE, &mut || latch.open()));
            }
            let reasons = (0..3)
                .map(|_| refusal(&done(&events)).to_owned())
                .collect::<Vec<_>>();
            assert_eq!(
                reasons,
                [map::RENDER, map::NOT_RUNNING, map::NOT_RUNNING],
                "{dropped}"
            );
            assert_eq!(checks.load(Ordering::SeqCst), 1, "{dropped}");
        }
    }

    /// PW 2.2 step 3: a gate whose start returns after the worker is gone is stopped by its
    /// starter within `GATE_STOP_BOUND`.
    #[test]
    fn a_gate_started_after_the_stop_is_stopped_by_its_starter() {
        for stop_sent in [false, true] {
            let data = TempDir::new("w26");
            let latch = Latch::default();
            let gate = Double::new(render);
            let stops = Arc::clone(&gate.stops);
            let stop_bounds = Arc::clone(&gate.stop_bounds);
            let waiting = latch.clone();
            let starter: Starter = Arc::new(move |_: &GateConfig, _: &KillHandle| {
                waiting.wait();
                Ok(Box::new(gate.clone()) as Box<dyn Gate>)
            });
            let (prover, events) = spawn(options(data.path(), &[], starter));
            drop(events);
            assert!(prover.prove(job()));
            wait_until("the worker's end", || !prover.prove(job()));
            if stop_sent {
                lock(&prover.shared).stop_by = Some(Instant::now());
            }
            latch.open();
            wait_until("the starter's stop", || stops.load(Ordering::SeqCst) == 1);
            let expected = if stop_sent {
                Duration::ZERO
            } else {
                GATE_STOP_BOUND
            };
            assert_eq!(*stop_bounds.lock().unwrap(), [expected], "{stop_sent}");
            drop(prover);
        }
    }

    /// PW X9 and PW X11: the bounds fit the relay's drain and the core's proof watchdog.
    #[test]
    fn the_bounds_fit_the_core_and_the_relay() {
        assert!(QUIT_BOUND < fabric_desktop_relay::QUIT_DRAIN);
        assert!(KILL_ALLOWANCE < QUIT_BOUND);
        assert!(
            CHECK_TIMEOUT + CHECK_GRACE
                <= Duration::from_millis(fabric_broker_core::PROOF_WATCHDOG_MS)
        );
    }

    /// PW 2.5: once a stop was sent, a retry that is due does not start a gate.
    #[test]
    fn no_start_begins_once_a_stop_was_sent() {
        let data = TempDir::new("w28");
        let calls = Arc::new(AtomicUsize::new(0));
        let starter = counting(&calls, |_, _| Err(isabelle()));
        // The worker sends `Unavailable` after the first start failed, with a retry due at
        // once, and then waits in `emit` until the stop was sent.
        let held = Latch::default();
        let release = held.clone();
        let (sender, events) = mpsc::channel();
        let emit: Emit = Box::new(move |event| {
            let unavailable = matches!(event, ProverEvent::Health(ProverStatus::Unavailable(_)));
            let sent = sender.send(event).is_ok();
            if unavailable {
                release.wait();
            }
            sent
        });
        let prover = IsabelleProver::spawn(options(data.path(), &[Duration::ZERO], starter), emit);
        assert_eq!(
            status(&events),
            ProverStatus::Unavailable(Some(ProverProblem::FailedWillRetry))
        );
        assert!(prover.stop(PATIENCE, &mut || held.open()));
        // A start would have sent `Starting` before the acknowledgement.
        assert!(events.try_iter().next().is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // The retry itself: due without a stop, neither waited for nor due once one was sent.
        for (stop_by, due) in [(None, true), (Some(Instant::now()), false)] {
            let retry = State::Down {
                retry_at: Some(Instant::now()),
            };
            let (worker, _messages) =
                worker(data.path(), ready(Double::new(render)), retry, stop_by);
            assert_eq!(worker.retry_wait().is_some(), due);
            assert_eq!(worker.retry_due(), due);
        }
    }

    /// A worker built by hand, with its own channel.
    fn worker(
        data: &Path,
        starter: Starter,
        state: State,
        stop_by: Option<Instant>,
    ) -> (Worker, Receiver<Msg>) {
        let (sender, messages) = mpsc::channel();
        let worker = Worker {
            options: options(data, &[], starter),
            emit: Box::new(|_| true),
            shared: Arc::new(Mutex::new(Shared {
                health: ProverStatus::Starting,
                running: None,
                overrun: false,
                stop_by,
                kill: None,
            })),
            sender,
            state,
            failures: 0,
            gone: false,
        };
        (worker, messages)
    }

    /// PW 2.5: a stop sent before the worker has run starts no gate at all.
    #[test]
    fn a_stop_sent_before_the_first_start_starts_nothing() {
        let data = TempDir::new("w29");
        let calls = Arc::new(AtomicUsize::new(0));
        let starter = counting(&calls, |_, _| Err(isabelle()));
        let (worker, messages) =
            worker(data.path(), starter, State::Starting, Some(Instant::now()));
        let (ack, acknowledged) = mpsc::channel();
        worker.sender.send(Msg::Stop(ack)).unwrap();
        worker.run(&messages);
        // `run` dropped the worker's sender; a starter thread would hold a clone and send `Started`.
        assert!(messages.recv().is_err());
        assert_eq!(acknowledged.try_recv(), Ok(()));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(!data.path().join("prover").exists());
    }

    /// A starter that records the kill handle of each start and gives the start's gate, a clone
    /// of `gate`, that handle.
    fn handing_on(gate: Double, handles: &Arc<Mutex<Vec<KillHandle>>>) -> Starter {
        let handles = Arc::clone(handles);
        Arc::new(move |_: &GateConfig, kill: &KillHandle| {
            handles.lock().unwrap().push(kill.clone());
            let mut gate = gate.clone();
            gate.kill = Some(kill.clone());
            Ok(Box::new(gate) as Box<dyn Gate>)
        })
    }

    /// PW w30 (PW 2.5, PW K1): a stop while a starter is inside its gate's start kills that
    /// start's handle before the stop is acknowledged, on Unix; elsewhere the handle is not
    /// killed.
    #[test]
    fn a_stop_while_starting_kills_the_start() {
        let data = TempDir::new("w30");
        let latch = Latch::default();
        let waiting = latch.clone();
        let handles = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&handles);
        let starter: Starter = Arc::new(move |_: &GateConfig, kill: &KillHandle| {
            recorded.lock().unwrap().push(kill.clone());
            waiting.wait();
            Err(isabelle())
        });
        let (prover, _events) = spawn(options(data.path(), &[], starter));
        wait_until("the start", || !handles.lock().unwrap().is_empty());
        assert!(stop(prover));
        let handles = handles.lock().unwrap().clone();
        assert_eq!(handles.len(), 1);
        assert_eq!(handles[0].is_killed(), cfg!(unix));
        latch.open();
    }

    /// PW w31 (PW 2.5, PW K1): a check still running at the stop's instant has its server killed
    /// through the kill handle; the check fails, the broken gate is stopped and the stop is
    /// acknowledged soon after that instant.
    #[cfg(unix)]
    #[test]
    fn a_check_still_running_at_the_stop_instant_is_killed() {
        let data = TempDir::new("w31");
        let gate = Double::new(io);
        let checks = Arc::clone(&gate.checks);
        let stops = Arc::clone(&gate.stops);
        let handles = Arc::new(Mutex::new(Vec::new()));
        // A long wait is left, so a failure that ignored the stop would read
        // `failed_will_retry`.
        let waits = [Duration::from_secs(600)];
        let (prover, events) = spawn(options(data.path(), &waits, handing_on(gate, &handles)));
        assert_eq!(status(&events), ProverStatus::Ready);
        assert!(prover.prove(job()));
        wait_until("the check", || checks.load(Ordering::SeqCst) == 1);
        let started = Instant::now();
        assert!(prover.stop(PATIENCE, &mut || {}));
        let elapsed = started.elapsed();
        assert!(
            elapsed >= GATE_STOP_BOUND && elapsed < GATE_STOP_BOUND + Duration::from_millis(700),
            "{elapsed:?}"
        );
        assert!(handles.lock().unwrap()[0].is_killed());
        assert_eq!(stops.load(Ordering::SeqCst), 1);
        // The worker sent all it sends before it acknowledged.
        let statuses = events
            .try_iter()
            .filter_map(|event| match event {
                ProverEvent::Health(status) => Some(status),
                ProverEvent::Done(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            statuses,
            [ProverStatus::Unavailable(Some(ProverProblem::Failed))]
        );
    }

    /// PW w32 (PW 2.5): a stop whose gate is idle kills nothing through the handle, even when
    /// the gate's stop returns only after the stop's instant.
    #[test]
    fn a_stop_kills_no_server_whose_check_is_not_running() {
        let data = TempDir::new("w32");
        let mut gate = Double::new(render);
        gate.stop_overrun = Some(Duration::from_millis(200));
        let handles = Arc::new(Mutex::new(Vec::new()));
        let (prover, events) = spawn(options(data.path(), &[], handing_on(gate, &handles)));
        assert_eq!(status(&events), ProverStatus::Ready);
        let started = Instant::now();
        assert!(prover.stop(PATIENCE, &mut || {}));
        let elapsed = started.elapsed();
        assert!(elapsed >= GATE_STOP_BOUND, "{elapsed:?}");
        assert!(!handles.lock().unwrap()[0].is_killed());
    }

    /// PW w33 (PW 2.2, PW X7): every start gets a new kill handle, so a retry after a start whose
    /// handle was killed is not refused for it.
    #[test]
    fn each_start_gets_its_own_kill_handle() {
        let data = TempDir::new("w33");
        let calls = Arc::new(AtomicUsize::new(0));
        let retried = Arc::new(Mutex::new(None));
        let recorded = Arc::clone(&retried);
        let starter = counting(&calls, move |call, kill| {
            if call == 1 {
                kill.kill().unwrap();
            } else {
                *recorded.lock().unwrap() = Some(kill.is_killed());
            }
            Err(isabelle())
        });
        let (prover, events) = spawn(options(data.path(), &[Duration::from_millis(10)], starter));
        for expected in [
            ProverStatus::Unavailable(Some(ProverProblem::FailedWillRetry)),
            ProverStatus::Starting,
            ProverStatus::Unavailable(Some(ProverProblem::Failed)),
        ] {
            assert_eq!(status(&events), expected);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(*retried.lock().unwrap(), Some(false));
        assert!(stop(prover));
    }

    /// PW w34 (PW 2.5): a job that `Worker::job` let through because no stop was sent yet is not
    /// checked when the stop was sent before its check began, and no check is marked running.
    #[test]
    fn no_check_starts_once_a_stop_was_sent() {
        let data = TempDir::new("w34");
        let gate = Double::new(render);
        let checks = Arc::clone(&gate.checks);
        let (mut worker, _messages) = worker(
            data.path(),
            ready(Double::new(render)),
            State::Ready(Box::new(gate)),
            Some(Instant::now()),
        );
        let State::Ready(gate) = &mut worker.state else {
            unreachable!("the worker was built ready");
        };
        let checked = check_job(gate.as_mut(), &worker.shared, CHECK_TIMEOUT, &job());
        assert_eq!(refusal(&checked.done), map::NOT_RUNNING);
        assert_eq!(checks.load(Ordering::SeqCst), 0);
        assert!(lock(&worker.shared).running.is_none());
    }

    /// PW w35 (PW 2.3): a check held until the clock is past its job's deadline, which then fails,
    /// is late; the same failure in time is the gate's.
    #[test]
    fn a_check_that_returns_after_its_deadline_is_late() {
        let data = TempDir::new("w35-late");
        let latch = Latch::default();
        let mut gate = Double::new(isabelle);
        gate.check_latch = Some(latch.clone());
        let checks = Arc::clone(&gate.checks);
        let (prover, events) = spawn(options(data.path(), &[], ready(gate)));
        assert_eq!(status(&events), ProverStatus::Ready);
        let mut late = job();
        // Far enough for the job to reach its check before its deadline.
        late.deadline_ms = now_ms() + 1_000;
        let deadline = late.deadline_ms;
        assert!(prover.prove(late));
        wait_until("the check", || checks.load(Ordering::SeqCst) == 1);
        wait_until("the deadline", || now_ms() >= deadline);
        latch.open();
        assert_eq!(refusal(&done(&events)), map::CHECK_LATE);
        assert!(stop(prover));

        let data = TempDir::new("w35-in-time");
        let (prover, events) = spawn(options(data.path(), &[], ready(Double::new(isabelle))));
        assert_eq!(status(&events), ProverStatus::Ready);
        assert!(prover.prove(job()));
        assert_eq!(refusal(&done(&events)), map::FAILED);
        assert!(stop(prover));
    }

    /// PW w36 (PW 2.3): the adapter passes the smaller of the time left and its own check limit,
    /// and a check that fails after taking at least that limit, before the deadline, reached it.
    #[test]
    fn a_check_that_takes_its_whole_limit_reached_it() {
        let data = TempDir::new("w36");
        let limit = Duration::from_millis(50);
        let latch = Latch::default();
        let mut gate = Double::new(render);
        gate.check_latch = Some(latch.clone());
        let checks = Arc::clone(&gate.checks);
        let limits = Arc::clone(&gate.limits);
        let mut options = options(data.path(), &[], ready(gate));
        options.gate.check_timeout = limit;
        let (prover, events) = spawn(options);
        assert_eq!(status(&events), ProverStatus::Ready);
        assert!(prover.prove(job()));
        wait_until("the check", || checks.load(Ordering::SeqCst) == 1);
        // The check began before this instant, so it has taken the limit once this has passed.
        let seen = Instant::now();
        wait_until("the limit", || seen.elapsed() >= limit);
        latch.open();
        assert_eq!(refusal(&done(&events)), map::CHECK_LIMIT);
        assert_eq!(*limits.lock().unwrap(), [limit]);
        assert!(stop(prover));
    }

    /// PW w37 (PW 2.2): a pin that cannot read its files is a failed start, so it will retry while
    /// a wait is left; it is not another theory.
    #[test]
    fn a_pin_that_cannot_read_its_files_will_retry() {
        let data = TempDir::new("w37");
        let rules = data.path().join("rules");
        fs::create_dir_all(rules.join("Fabric_Broker")).unwrap();
        fs::write(rules.join("Fabric_Broker").join("ROOT"), SESSION_ROOT).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let starter = counting(&calls, |_, _| Err(isabelle()));
        let mut options = options(data.path(), &[Duration::from_secs(600)], starter);
        options.gate.rules_dir = rules;
        let (prover, events) = spawn(options);
        assert_eq!(
            status(&events),
            ProverStatus::Unavailable(Some(ProverProblem::FailedWillRetry))
        );
        quiet(&events);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(prover.prove(job()));
        assert_eq!(refusal(&done(&events)), map::NOT_RUNNING);
        assert!(stop(prover));
    }

    /// PW w38 (PW 2.3): the overrun poll says the prover is unavailable without a cause; when the
    /// check then breaks the gate, the worker says why, although the health is the same.
    #[test]
    fn overrun_then_a_broken_gate_says_why() {
        let data = TempDir::new("w38");
        let latch = Latch::default();
        let mut gate = Double::new(io);
        gate.check_latch = Some(latch.clone());
        let (prover, events) = spawn(options(
            data.path(),
            &[Duration::from_secs(600)],
            ready(gate),
        ));
        assert_eq!(status(&events), ProverStatus::Ready);
        let job = job();
        let deadline = job.deadline_ms;
        assert!(prover.prove(job));
        wait_until("the running check", || {
            prover.next_due_ms() == Some(deadline)
        });
        assert_eq!(prover.poll(deadline), Some(ProverStatus::Unavailable(None)));
        latch.open();
        assert_eq!(refusal(&done(&events)), map::FAILED);
        assert_eq!(
            status(&events),
            ProverStatus::Unavailable(Some(ProverProblem::FailedWillRetry))
        );
        assert!(stop(prover));
    }

    /// PW w39 (PW 2.2): a gate start that panics is a failed start, so the worker goes through its
    /// failure rule and keeps answering jobs.
    #[test]
    fn a_start_that_panics_is_a_failed_start() {
        let data = TempDir::new("w39");
        let starter: Starter = Arc::new(
            |_: &GateConfig, _: &KillHandle| -> Result<Box<dyn Gate>, PolicyError> {
                panic!("the test starter panics")
            },
        );
        let (prover, events) = spawn(options(data.path(), &[], starter));
        assert_eq!(
            status(&events),
            ProverStatus::Unavailable(Some(ProverProblem::Failed))
        );
        assert!(!prover.lost());
        assert!(prover.prove(job()));
        assert_eq!(refusal(&done(&events)), map::NOT_RUNNING);
        assert!(stop(prover));
    }

    /// PW w40 (PW 2.2): a worker is lost when its thread could not be spawned, or ended with no
    /// stop sent; a running worker, and one that ended at a stop, are not.
    #[test]
    fn a_worker_is_lost_only_when_it_ended_without_a_stop() {
        // Ended because the main loop takes no more events.
        let data = TempDir::new("w40-gone");
        let starter: Starter = Arc::new(|_: &GateConfig, _: &KillHandle| Err(isabelle()));
        // An emit that always fails: the main loop is gone before the worker's first event.
        let prover = IsabelleProver::spawn(options(data.path(), &[], starter), Box::new(|_| false));
        wait_until("the worker's end", || prover.lost());
        drop(prover);

        // Running, then ended at a stop.
        let data = TempDir::new("w40-stopped");
        let (prover, events) = spawn(options(data.path(), &[], ready(Double::new(render))));
        assert_eq!(status(&events), ProverStatus::Ready);
        assert!(!prover.lost());
        lock(&prover.shared).stop_by = Some(Instant::now() + GATE_STOP_BOUND);
        let (ack, acknowledged) = mpsc::channel();
        prover.sender.send(Msg::Stop(ack)).unwrap();
        assert_eq!(acknowledged.recv_timeout(PATIENCE), Ok(()));
        wait_until("the worker's end", || {
            prover.worker.as_ref().is_some_and(JoinHandle::is_finished)
        });
        assert!(!prover.lost());
        drop(prover);

        // Never spawned.
        let data = TempDir::new("w40-never");
        let (sender, _messages) = mpsc::channel();
        let never = IsabelleProver {
            sender,
            shared: Arc::new(Mutex::new(Shared {
                health: ProverStatus::Starting,
                running: None,
                overrun: false,
                stop_by: None,
                kill: None,
            })),
            work_dir: data.path().join("prover"),
            worker: None,
        };
        assert!(never.lost());
    }
}
