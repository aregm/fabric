//! The process shell's routing, shared by the executable and the live tests: it writes the
//! core's frames to stdout, hands proof jobs to the prover and dispatch orders to the provider,
//! and feeds their results back to the core as inputs.
//!
//! `main.rs` keeps the argument and configuration refusals, `Broker::start`, the channel, the
//! stdin reader thread, the loop over [`next_event`] and the exit codes.
//!
//! The workers:
//! - the Isabelle prover ([`crate::prover::IsabelleProver`]), spawned only once the store is
//!   open and its lock is held, so a second broker on the same data
//!   directory never touches the first one's prover directory or server;
//! - in a debug build, the synthetic classifier of a synthetic run without Isabelle, on a worker
//!   thread of its own;
//! - in a practice run (debug builds), the practice calendar's provider, on a worker thread of its
//!   own.
//!
//! Every worker sends its results on the main loop's bounded channel. The main thread never sends
//! on that channel itself (it is its only receiver, so a full channel would never drain): the
//! status and the results that the shell itself learns wait in the shell until the next
//! [`Shell::turn`].
//!
//! "PW" numbers, such as PW 2.2 or the test ids PW sh01 to PW sh11, refer to
//! `docs/plans/calendar-desktop-prover-wiring-design.md`; bare numbers, such as (D34),
//! cite the CD-W04 design (`docs/plans/calendar-desktop-w04-design.md`).

use std::io::Write;
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fabric_broker_core::ports::{
    AttemptDone, DispatchOrder, ProofDone, ProofJob, ProverStatus, Provider,
};
#[cfg(debug_assertions)]
use fabric_broker_core::ports::{ProofAction, Prover};
use fabric_broker_core::{Attachments, Broker, Input, Job, Opener, Output};
use fabric_desktop_contracts::owner::{ProofPurpose, ProverHealth, ProverProblem};
use fabric_desktop_contracts::scalar::Id;
// A release build off macOS has no key source, so `opener` uses none of these.
#[cfg(any(debug_assertions, target_os = "macos"))]
use fabric_storage::{OpenOptions, Store};

#[cfg(any(debug_assertions, target_os = "macos"))]
use crate::config::KeySource;
use crate::config::{Config, EXIT_HALT, EXIT_IO};
use crate::prover::{IsabelleProver, ProverEvent, ProverOptions, QUIT_BOUND, map};

/// How many events may wait for the main loop. The reader and the workers block beyond this, so
/// a fast host cannot make the broker hold more than a few frames in memory.
pub const QUEUED_EVENTS: usize = 4;

/// How long the main loop sleeps at most while a job is out with the Isabelle worker, so that a
/// worker that ended without a word (a panic sends no event) is noticed within that time and its
/// jobs are answered then (PW 2.2).
pub const LOSS_LOOK_MS: u64 = 250;

/// The raw wall clock in Unix milliseconds. The core keeps its own time from it, which never
/// goes back (design 5.1, D34).
#[must_use]
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// What the reader thread and the workers pass to the main loop.
pub enum Event {
    /// One owner line, without its newline.
    Line(Vec<u8>),
    /// An owner line longer than the frame limit.
    Oversized,
    /// The input ended; the main loop stops.
    End,
    /// Reading failed; the main loop stops with an error.
    Failed(std::io::Error),
    /// The prover's result for one job, boxed so lines do not carry its size.
    Proof(Box<ProofDone>),
    /// The prover's status changed.
    Health(ProverStatus),
    /// The provider's result for one attempt.
    Attempt(Box<AttemptDone>),
}

/// The prover the shell attaches (the table of `Plan::of`).
pub enum ProverChoice {
    /// No prover: every request ends `undecided` at once.
    None,
    /// The synthetic classifier of a synthetic run without Isabelle.
    #[cfg(debug_assertions)]
    Classifier,
    /// The Isabelle prover, started once the store is open. Boxed: its paths make it far larger
    /// than the other variants, and on Windows large enough for clippy to refuse the enum.
    Isabelle(Box<crate::prover::ProverOptions>),
}

/// Everything the shell attaches.
pub struct Plan {
    /// The prover.
    pub prover: ProverChoice,
    /// `<data_dir>/prover`. Once the store is open and its lock held, the shell removes it in
    /// every configuration, so a check theory a killed broker left goes even when this run
    /// has no prover.
    pub work_dir: std::path::PathBuf,
    /// The practice calendar of a practice run.
    #[cfg(debug_assertions)]
    pub practice: Option<crate::practice::PracticeCalendar>,
    /// The synthetic agent of a synthetic run.
    #[cfg(debug_assertions)]
    pub agent: Option<crate::synthetic::SyntheticAgent>,
}

impl Plan {
    /// The plan of a configuration (the table below); the practice calendar with
    /// `Faults::default()`. Main's only way to make one.
    ///
    /// | Configuration | Prover | Provider |
    /// |---|---|---|
    /// | no Isabelle, no synthetic agent | none | none |
    /// | no Isabelle, synthetic agent (debug) | the synthetic classifier | none |
    /// | Isabelle, no synthetic agent | Isabelle | none |
    /// | Isabelle and the synthetic agent (debug): practice | Isabelle | the practice calendar |
    #[must_use]
    pub fn of(config: &Config, now_ms: u64) -> Plan {
        let prover = match &config.isabelle {
            Some(isabelle) => ProverChoice::Isabelle(Box::new(ProverOptions::with_installation(
                isabelle,
                &config.installation,
                &config.data_dir,
            ))),
            // An installed Isabelle that is missing or another version leaves `isabelle` empty:
            // a run without a rule prover, so the owner sees `not_configured`. Once contract
            // schema version 3 is merged, a follow-up sends `not_installed` for those two cases of
            // `config.installation` here.
            #[cfg(debug_assertions)]
            None if config.synthetic_agent => ProverChoice::Classifier,
            None => ProverChoice::None,
        };
        #[cfg(debug_assertions)]
        {
            use crate::practice::{Faults, PracticeCalendar};
            use crate::synthetic::SyntheticAgent;
            let practice = config
                .practice()
                .then(|| PracticeCalendar::new(now_ms, Faults::default()));
            let agent = config.synthetic_agent.then(|| match &practice {
                Some(calendar) => SyntheticAgent::practice(calendar.clone()),
                None => SyntheticAgent::new(),
            });
            Plan {
                prover,
                work_dir: config.data_dir.join("prover"),
                practice,
                agent,
            }
        }
        #[cfg(not(debug_assertions))]
        {
            let _ = now_ms;
            Plan {
                prover,
                work_dir: config.data_dir.join("prover"),
            }
        }
    }

    /// The prover's identity and whether a provider is attached (practice only).
    #[must_use]
    pub fn attachments(&self) -> Attachments {
        let prover = match &self.prover {
            ProverChoice::None => None,
            #[cfg(debug_assertions)]
            ProverChoice::Classifier => Some(crate::synthetic::SyntheticClassifier.identity()),
            ProverChoice::Isabelle(_) => Some(IsabelleProver::identity()),
        };
        #[cfg(debug_assertions)]
        let provider_present = self.practice.is_some();
        #[cfg(not(debug_assertions))]
        let provider_present = false;
        Attachments {
            prover,
            provider_present,
        }
    }
}

/// Opens the store of `config` with its key; called at start and for every reopen attempt.
/// Moved from main.
#[must_use]
pub fn opener(config: &Config) -> Opener {
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

/// The build named in `hello`: ` synthetic` or ` synthetic practice` after `CD-W04`.
#[must_use]
pub fn build_label(config: &Config) -> String {
    let suffix = if config.practice() {
        " synthetic practice"
    } else if config.synthetic_agent {
        " synthetic"
    } else {
        ""
    };
    format!("{} CD-W04{suffix}", env!("CARGO_PKG_VERSION"))
}

/// Waits for the next event, or until `due`; `None` when the wait ended at `due`. With no due
/// time it waits for the next event; a channel with no sender left reads as the end of input.
pub fn next_event(events: &Receiver<Event>, due: Option<u64>) -> Option<Event> {
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

/// The input of a reader or worker event. Main matches `End` and `Failed` first (exit 0 after
/// `quit`, exit 74 with the error on stderr); for those two this returns `None`. A free
/// function, because `Shell<W>` would leave `W` uninferred at a call that names no shell.
#[must_use]
pub fn input_of(event: Event) -> Option<Input> {
    match event {
        Event::Line(bytes) => Some(Input::OwnerLine(bytes)),
        Event::Oversized => Some(Input::OwnerOversized),
        Event::Proof(done) => Some(Input::ProofDone(*done)),
        Event::Health(status) => Some(Input::ProverHealth(status)),
        Event::Attempt(done) => Some(Input::Attempt(*done)),
        Event::End | Event::Failed(_) => None,
    }
}

/// Where the shell's proof jobs go.
enum ProverSlot {
    /// No prover is attached; the core never makes a proof job then.
    None,
    /// The synthetic classifier's worker.
    #[cfg(debug_assertions)]
    Classifier(Sender<ProofJob>),
    /// The Isabelle prover, waiting for the store to open; boxed as in `ProverChoice`.
    Waiting(Box<ProverOptions>),
    /// The Isabelle worker.
    Isabelle(IsabelleProver),
}

/// Frames to stdout, jobs to the workers, results back as inputs.
pub struct Shell<W: Write> {
    /// Where frames go.
    stdout: W,
    /// The main loop's channel, for the workers' results.
    events: SyncSender<Event>,
    /// The prover, or the options to start it with.
    prover: ProverSlot,
    /// The prover's work directory, until the first `route` with the store open removes it.
    stale_work_dir: Option<std::path::PathBuf>,
    /// The provider worker's orders; `None` when no provider is attached.
    provider: Option<Sender<DispatchOrder>>,
    /// The practice calendar, for the observation of an update or delete.
    #[cfg(debug_assertions)]
    practice: Option<crate::practice::PracticeCalendar>,
    /// The synthetic agent.
    #[cfg(debug_assertions)]
    agent: Option<crate::synthetic::SyntheticAgent>,
    /// The status the shell itself must feed: a lost worker or an overrun; drained by `turn`.
    pending_health: Option<ProverStatus>,
    /// The results the shell itself must feed, in order: the answer to each job it could not
    /// hand over, and to each job a lost worker held; drained by `turn`.
    pending_results: Vec<ProofDone>,
    /// The id, request and purpose of each job handed to the Isabelle worker whose result has
    /// not passed through `turn` yet; answered at once if the worker is lost (PW 2.2).
    handed: Vec<(u64, Id, ProofPurpose)>,
    /// Set at the first sign that the Isabelle worker is lost, so `failed` is fed once.
    prover_lost: bool,
    /// The raw wall clock time of the last turn's start, from which the next look for a lost
    /// worker is due while a job is out (PW 2.2).
    last_turn_ms: u64,
}

impl<W: Write> Shell<W> {
    /// Starts the classifier and provider workers now; the Isabelle worker waits for the open
    /// store.
    #[must_use]
    pub fn new(stdout: W, plan: Plan, events: SyncSender<Event>) -> Shell<W> {
        let prover = match plan.prover {
            ProverChoice::None => ProverSlot::None,
            #[cfg(debug_assertions)]
            ProverChoice::Classifier => ProverSlot::Classifier(spawn_classifier(
                Box::new(crate::synthetic::SyntheticClassifier),
                events.clone(),
            )),
            ProverChoice::Isabelle(options) => ProverSlot::Waiting(options),
        };
        #[cfg(debug_assertions)]
        let provider = plan
            .practice
            .as_ref()
            .map(|calendar| spawn_provider(Box::new(calendar.provider()), events.clone()));
        #[cfg(not(debug_assertions))]
        let provider = None;
        Shell {
            stdout,
            events,
            prover,
            stale_work_dir: Some(plan.work_dir),
            provider,
            #[cfg(debug_assertions)]
            practice: plan.practice,
            #[cfg(debug_assertions)]
            agent: plan.agent,
            pending_health: None,
            pending_results: Vec::new(),
            handed: Vec::new(),
            prover_lost: false,
            last_turn_ms: 0,
        }
    }

    /// Writes stderr lines and frames, then routes the jobs. `Err` is the exit code (74 or 75).
    pub fn emit(&mut self, broker: &Broker, output: Output) -> Result<(), ExitCode> {
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
        self.route(broker, output.jobs);
        Ok(())
    }

    /// Once `broker.store_ready()`: removes what a killed broker left in the prover's work
    /// directory, then starts the Isabelle worker. Hands `Prove` jobs to the prover (with the
    /// practice observation) and `Dispatch` orders to the provider.
    pub fn route(&mut self, broker: &Broker, jobs: Vec<Job>) {
        if broker.store_ready()
            && let Some(work_dir) = self.stale_work_dir.take()
            && crate::prover::remove_work_dir(&work_dir).is_err()
        {
            eprintln!(
                "fabric-broker: the prover's work directory left by an earlier run could not be removed"
            );
        }
        if broker.store_ready()
            && matches!(self.prover, ProverSlot::Waiting(_))
            && let ProverSlot::Waiting(options) =
                std::mem::replace(&mut self.prover, ProverSlot::None)
        {
            self.prover = ProverSlot::Isabelle(spawn_isabelle(*options, self.events.clone()));
        }
        for job in jobs {
            match job {
                Job::Prove(job) => self.prove(job),
                Job::Dispatch(order) => match &self.provider {
                    // The worker lives as long as the shell; a send fails only after it
                    // panicked, and the core's provider deadline then ends the attempt.
                    Some(provider) => {
                        let _ = provider.send(order);
                    }
                    None => eprintln!(
                        "fabric-broker: a dispatch came with no provider attached; it is not carried out"
                    ),
                },
            }
        }
    }

    /// Hands one proof job to the prover. A job it cannot hand over is answered at once with
    /// `NOT_RUNNING`, so the request ends `undecided` in the next turn, not by its watchdog: when
    /// the Isabelle worker is lost, the classifier's channel is closed, or no prover runs
    /// (PW 2.2). Only the Isabelle worker's loss changes the status.
    #[cfg_attr(
        not(debug_assertions),
        allow(unused_mut, reason = "the observation is debug only")
    )]
    fn prove(&mut self, mut job: ProofJob) {
        #[cfg(debug_assertions)]
        if let Some(calendar) = &self.practice
            && job.facts.event.is_none()
            && let ProofAction::Agent(action) = &job.action
        {
            job.facts.event = calendar.observation(action);
        }
        let (job_id, request_id, purpose) = (job.job_id, job.request_id.clone(), job.purpose);
        let mut lost = false;
        let handed = match &self.prover {
            // The worker lives as long as the shell; a send fails only after it panicked.
            #[cfg(debug_assertions)]
            ProverSlot::Classifier(classifier) => classifier.send(job).is_ok(),
            ProverSlot::Isabelle(_) if self.prover_lost => false,
            ProverSlot::Isabelle(prover) => {
                if prover.prove(job) {
                    self.handed.push((job_id, request_id.clone(), purpose));
                    true
                } else {
                    lost = true;
                    false
                }
            }
            ProverSlot::None | ProverSlot::Waiting(_) => {
                eprintln!(
                    "fabric-broker: a proof job came with no prover running; it ends undecided"
                );
                false
            }
        };
        if lost {
            self.lose();
        }
        if !handed {
            self.pending_results
                .push(map::answer(job_id, &request_id, purpose, map::NOT_RUNNING));
        }
    }

    /// Asks the Isabelle worker whether it is lost, then runs the overrun poll. Once the worker is
    /// lost the poll still runs, so its latch is set and the prover's due time clears, but its
    /// status is dropped and cannot replace `failed` (PW 2.2).
    fn watch_prover(&mut self, now_ms: u64) {
        let (lost, polled) = match &self.prover {
            ProverSlot::Isabelle(prover) => (prover.lost(), prover.poll(now_ms)),
            _ => (false, None),
        };
        if lost {
            self.lose();
        }
        if let Some(status) = polled
            && !self.prover_lost
        {
            self.pending_health = Some(status);
        }
    }

    /// At the first sign that the Isabelle worker is lost (a failed send, or
    /// [`IsabelleProver::lost`]), once: the status to feed becomes `failed`, and every job handed
    /// to the worker without a result is answered with `NOT_RUNNING` (PW 2.2).
    fn lose(&mut self) {
        if self.prover_lost {
            return;
        }
        self.prover_lost = true;
        self.pending_health = Some(ProverStatus::Unavailable(Some(ProverProblem::Failed)));
        for (job_id, request_id, purpose) in self.handed.drain(..) {
            self.pending_results
                .push(map::answer(job_id, &request_id, purpose, map::NOT_RUNNING));
        }
    }

    /// The start of a turn. A worker's result in hand leaves `handed` first, so a loss noticed in
    /// this turn never supersedes it with `NOT_RUNNING`; then `watch_prover` (PW 2.2).
    fn notice(&mut self, input: &Input, now_ms: u64) {
        self.last_turn_ms = now_ms;
        if let Input::ProofDone(done) = input {
            self.handed.retain(|(job_id, request_id, purpose)| {
                (*job_id, request_id, *purpose) != (done.job_id, &done.request_id, done.purpose)
            });
        }
        self.watch_prover(now_ms);
    }

    /// One turn: `notice`; `pending_health` if any (to the broker
    /// and the agent, emit); each pending result (emit); the input (health also to the agent),
    /// the agent's replies, emit; the agent's tick, emit.
    ///
    /// Once the worker is lost, a status still queued from it is dropped and fed as a tick, so it
    /// cannot replace `failed` (PW 2.2).
    pub fn turn(&mut self, broker: &mut Broker, input: Input, now_ms: u64) -> Result<(), ExitCode> {
        self.notice(&input, now_ms);
        if let Some(status) = self.pending_health.take() {
            self.agent_health(status.health());
            let mut output = broker.handle(Input::ProverHealth(status), now_ms);
            self.agent_replies(&mut output);
            self.emit(broker, output)?;
        }
        for done in std::mem::take(&mut self.pending_results) {
            let mut output = broker.handle(Input::ProofDone(done), now_ms);
            self.agent_replies(&mut output);
            self.emit(broker, output)?;
        }
        let input = match input {
            Input::ProverHealth(_) if self.prover_lost => Input::Tick,
            input => input,
        };
        if let Input::ProverHealth(status) = &input {
            self.agent_health(status.health());
        }
        let mut output = broker.handle(input, now_ms);
        self.agent_replies(&mut output);
        self.emit(broker, output)?;
        let output = self.agent_tick(broker, now_ms);
        self.emit(broker, output)
    }

    /// `broker.quit`, the agent's last replies, emit.
    pub fn quit(&mut self, broker: &mut Broker, now_ms: u64) -> Result<(), ExitCode> {
        let mut output = broker.quit(now_ms);
        self.agent_replies(&mut output);
        self.emit(broker, output)
    }

    /// `Some(0)` while a status or a result is pending; else the earliest of the broker's, the
    /// agent's and the prover's due times, and, while a job is out with an Isabelle worker not
    /// known to be lost, the last turn's time plus [`LOSS_LOOK_MS`] (PW 2.2). All are raw wall
    /// clock times, as [`next_event`] reads them.
    #[must_use]
    pub fn next_due_ms(&self, broker: &Broker) -> Option<u64> {
        if self.pending_health.is_some() || !self.pending_results.is_empty() {
            return Some(0);
        }
        let (prover, look) = match &self.prover {
            ProverSlot::Isabelle(prover) => (
                prover.next_due_ms(),
                (!self.prover_lost && !self.handed.is_empty())
                    .then(|| self.last_turn_ms.saturating_add(LOSS_LOOK_MS)),
            ),
            _ => (None, None),
        };
        [broker.next_due_ms(), self.agent_due_ms(), prover, look]
            .into_iter()
            .flatten()
            .min()
    }

    /// The frames written so far, for tests that write to a `Vec<u8>`.
    pub fn stdout_mut(&mut self) -> &mut W {
        &mut self.stdout
    }

    /// Stops the Isabelle worker within `QUIT_BOUND`, draining `events` meanwhile, so a
    /// worker blocked on the full channel can read its stop. False when the worker did not
    /// acknowledge in time; its work directory is then removed by this thread. True when no
    /// Isabelle worker runs. The other workers end when their channels close, with the shell.
    pub fn stop(self, events: &Receiver<Event>) -> bool {
        let Shell {
            prover,
            events: sender,
            provider,
            ..
        } = self;
        drop(provider);
        drop(sender);
        match prover {
            ProverSlot::Isabelle(prover) => {
                let mut drain = || while events.try_recv().is_ok() {};
                prover.stop(QUIT_BOUND, &mut drain)
            }
            _ => true,
        }
    }

    /// Tells the synthetic agent the prover's health.
    fn agent_health(&mut self, health: ProverHealth) {
        #[cfg(debug_assertions)]
        if let Some(agent) = &mut self.agent {
            agent.set_prover_health(health);
        }
        #[cfg(not(debug_assertions))]
        let _ = health;
    }

    /// Hands the synthetic agent its replies.
    fn agent_replies(&mut self, output: &mut Output) {
        #[cfg(debug_assertions)]
        if let Some(agent) = &mut self.agent {
            agent.take_replies(output);
        }
        #[cfg(not(debug_assertions))]
        let _ = output;
    }

    /// The synthetic agent's due work.
    fn agent_tick(&mut self, broker: &mut Broker, now_ms: u64) -> Output {
        #[cfg(debug_assertions)]
        if let Some(agent) = &mut self.agent {
            return agent.tick(broker, now_ms);
        }
        let _ = (broker, now_ms);
        Output::default()
    }

    /// When the synthetic agent has something to do next.
    fn agent_due_ms(&self) -> Option<u64> {
        #[cfg(debug_assertions)]
        if let Some(agent) = &self.agent {
            return agent.next_due_ms();
        }
        None
    }
}

/// Spawns the Isabelle worker, whose events go to the main loop's channel.
fn spawn_isabelle(options: ProverOptions, events: SyncSender<Event>) -> IsabelleProver {
    IsabelleProver::spawn(
        options,
        Box::new(move |event| {
            let event = match event {
                ProverEvent::Done(done) => Event::Proof(done),
                ProverEvent::Health(status) => Event::Health(status),
            };
            events.send(event).is_ok()
        }),
    )
}

/// Runs the synthetic classifier on its own thread. Jobs go to it over an unbounded channel, so
/// the main loop never waits for the worker; the worker may wait on the bounded result channel
/// until the main loop drains it, which cannot deadlock.
#[cfg(debug_assertions)]
fn spawn_classifier(mut prover: Box<dyn Prover>, events: SyncSender<Event>) -> Sender<ProofJob> {
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

/// Runs `provider` on its own thread, the same way: orders over an unbounded channel, results
/// on the main loop's bounded one.
#[cfg_attr(
    not(debug_assertions),
    allow(dead_code, reason = "only practice runs have a provider")
)]
fn spawn_provider(
    mut provider: Box<dyn Provider>,
    events: SyncSender<Event>,
) -> Sender<DispatchOrder> {
    let (orders, received) = mpsc::channel::<DispatchOrder>();
    thread::spawn(move || {
        for order in received {
            if events
                .send(Event::Attempt(Box::new(provider.dispatch(&order))))
                .is_err()
            {
                return;
            }
        }
    });
    orders
}

#[cfg(test)]
mod tests {
    //! The shell without Isabelle: no test here starts a gate that could prove. The stores these
    //! tests open use the test key in a temporary directory; none touches the Keychain.

    use super::*;
    use crate::config::IsabelleSettings;
    #[cfg(debug_assertions)]
    use fabric_broker_core::ports::ProverKind;
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    /// A fresh directory under the system's temporary directory, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> TempDir {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "fabric-broker-shell-{label}-{}-{n}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }

        fn data(&self) -> PathBuf {
            self.0.join("data")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Isabelle paths under `root` that hold nothing: a worker started with them fails at once.
    fn nowhere(root: &Path) -> IsabelleSettings {
        IsabelleSettings {
            isabelle_home: root.join("no-isabelle"),
            user_home: root.join("no-user-home"),
            rules_dir: root.join("no-rules"),
        }
    }

    /// The key source of a configuration this build can hold.
    #[cfg(any(debug_assertions, target_os = "macos"))]
    fn key() -> KeySource {
        #[cfg(debug_assertions)]
        return KeySource::Test;
        #[cfg(not(debug_assertions))]
        return KeySource::Keychain;
    }

    #[cfg(any(debug_assertions, target_os = "macos"))]
    fn config(
        data_dir: &Path,
        synthetic_agent: bool,
        isabelle: Option<IsabelleSettings>,
    ) -> Config {
        Config {
            data_dir: data_dir.to_path_buf(),
            key_source: key(),
            synthetic_agent,
            isabelle,
            installation: crate::config::Installation::NotAsked,
        }
    }

    /// PW sh01: each configuration attaches what the table of `Plan::of` says. Only plans are built
    /// here; no store is opened and no worker starts.
    #[cfg(any(debug_assertions, target_os = "macos"))]
    #[test]
    fn the_plan_of_each_configuration_is_the_table_of_plan_of() {
        let dir = TempDir::new("plan");
        let isabelle = Some(nowhere(&dir.0));
        let version = env!("CARGO_PKG_VERSION");

        let bare = config(&dir.data(), false, None);
        let plan = Plan::of(&bare, 0);
        assert!(matches!(plan.prover, ProverChoice::None));
        let attachments = plan.attachments();
        assert_eq!(attachments.prover, None);
        assert!(!attachments.provider_present);
        assert_eq!(build_label(&bare), format!("{version} CD-W04"));

        let proved = config(&dir.data(), false, isabelle.clone());
        let plan = Plan::of(&proved, 0);
        match &plan.prover {
            ProverChoice::Isabelle(options) => {
                assert_eq!(options.gate.work_dir, dir.data().join("prover"));
                assert_eq!(options.restart_waits, crate::prover::RESTART_WAITS.to_vec());
            }
            _ => panic!("not Isabelle"),
        }
        let attachments = plan.attachments();
        assert_eq!(attachments.prover, Some(IsabelleProver::identity()));
        assert_eq!(IsabelleProver::identity().health, ProverStatus::Starting);
        assert!(!attachments.provider_present);
        assert_eq!(build_label(&proved), format!("{version} CD-W04"));

        #[cfg(debug_assertions)]
        {
            let synthetic = config(&dir.data(), true, None);
            let plan = Plan::of(&synthetic, 0);
            assert!(matches!(plan.prover, ProverChoice::Classifier));
            assert!(plan.practice.is_none());
            assert!(plan.agent.is_some());
            let attachments = plan.attachments();
            let prover = attachments.prover.expect("the classifier");
            assert_eq!(
                (prover.kind, prover.health),
                (
                    ProverKind::Synthetic,
                    ProverStatus::Unavailable(Some(ProverProblem::NotConfigured))
                )
            );
            assert!(!attachments.provider_present);
            assert_eq!(
                build_label(&synthetic),
                format!("{version} CD-W04 synthetic")
            );

            let practice = config(&dir.data(), true, isabelle);
            let plan = Plan::of(&practice, 0);
            assert!(matches!(plan.prover, ProverChoice::Isabelle(_)));
            assert!(plan.practice.is_some());
            assert!(plan.agent.is_some());
            let attachments = plan.attachments();
            assert_eq!(attachments.prover, Some(IsabelleProver::identity()));
            assert!(attachments.provider_present);
            assert_eq!(
                build_label(&practice),
                format!("{version} CD-W04 synthetic practice")
            );
        }
        assert!(!dir.data().exists(), "a plan made the data directory");
    }

    /// A broker on `config`'s store, started now.
    #[cfg(debug_assertions)]
    fn start(config: &Config, plan: &Plan) -> (Broker, Output) {
        Broker::start(
            opener(config),
            fabric_broker_core::Settings::cd_w04(build_label(config)),
            plan.attachments(),
            now_ms(),
        )
        .unwrap_or_else(|e| panic!("the broker starts: {e}"))
    }

    /// PW sh02: while another holder keeps the store, the broker runs in the unavailable mode and
    /// the Isabelle worker never starts, so it never touches `<data_dir>/prover`.
    #[cfg(debug_assertions)]
    #[test]
    fn the_isabelle_worker_does_not_start_while_the_store_is_in_use() {
        let dir = TempDir::new("in-use");
        let config = config(&dir.data(), false, Some(nowhere(&dir.0)));
        let held = (opener(&config))(0).expect("the test holds the store");
        let plan = Plan::of(&config, now_ms());
        let (mut broker, started) = start(&config, &plan);
        assert!(!broker.store_ready());
        let (sender, events) = mpsc::sync_channel(QUEUED_EVENTS);
        let mut shell = Shell::new(Vec::new(), plan, sender);
        shell.emit(&broker, started).unwrap();
        shell.turn(&mut broker, Input::Tick, now_ms()).unwrap();
        thread::sleep(Duration::from_millis(200));
        assert!(matches!(shell.prover, ProverSlot::Waiting(_)));
        assert!(!dir.data().join("prover").exists());
        assert!(shell.stop(&events));
        assert!(!dir.data().join("prover").exists());
        drop(held);
    }

    /// PW sh05: a check theory a killed broker left in `<data_dir>/prover` is
    /// removed once the store is open, also in a run with no prover; while another holder keeps
    /// the store, it is left alone.
    #[cfg(debug_assertions)]
    #[test]
    fn a_left_check_theory_goes_once_the_store_is_open_in_every_configuration() {
        let dir = TempDir::new("left-check");
        let config = config(&dir.data(), false, None);
        let plant = || {
            let work_dir = dir.data().join("prover");
            std::fs::create_dir_all(&work_dir).unwrap();
            let left = work_dir.join("Check_000001.thy");
            std::fs::write(&left, "left by a killed broker").unwrap();
            left
        };
        drop((opener(&config))(0).expect("the store is created"));
        let left = plant();

        let held = (opener(&config))(0).expect("the test holds the store");
        let plan = Plan::of(&config, now_ms());
        assert!(matches!(plan.prover, ProverChoice::None));
        let (broker, started) = start(&config, &plan);
        assert!(!broker.store_ready());
        let (sender, events) = mpsc::sync_channel(QUEUED_EVENTS);
        let mut shell = Shell::new(Vec::new(), plan, sender);
        shell.emit(&broker, started).unwrap();
        assert!(
            left.exists(),
            "removed while another holder keeps the store"
        );
        assert!(shell.stop(&events));
        drop(broker);
        drop(held);

        let plan = Plan::of(&config, now_ms());
        let (broker, started) = start(&config, &plan);
        assert!(broker.store_ready());
        let (sender, events) = mpsc::sync_channel(QUEUED_EVENTS);
        let mut shell = Shell::new(Vec::new(), plan, sender);
        shell.emit(&broker, started).unwrap();
        assert!(
            !dir.data().join("prover").exists(),
            "the left check theory stays"
        );
        assert!(shell.stop(&events));
    }

    /// One owner frame through `turn`; its response's result.
    #[cfg(debug_assertions)]
    fn owner(
        shell: &mut Shell<Vec<u8>>,
        broker: &mut Broker,
        id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> serde_json::Value {
        let line = fabric_desktop_contracts::canonical(&serde_json::json!({
            "protocol": "fabric-desktop/1", "id": id, "method": method, "params": params
        }));
        shell
            .turn(broker, Input::OwnerLine(line.into_bytes()), now_ms())
            .unwrap();
        let response = shell
            .stdout_mut()
            .split(|b| *b == b'\n')
            .filter_map(|l| serde_json::from_slice::<serde_json::Value>(l).ok())
            .find(|v| v["id"] == serde_json::json!(id))
            .unwrap_or_else(|| panic!("no response to {method}"));
        assert!(response.get("error").is_none(), "{response}");
        response["result"].clone()
    }

    /// PW sh06: a request made while the Isabelle prover is down ends
    /// `undecided` through the shell. The worker answers its proof job at once with no report;
    /// no proof is recorded, nothing is authorized and nothing is dispatched. The agent here is
    /// the synthetic agent without the practice script, which proposes whatever the prover's
    /// health; the practice script waits for `ready`, so a process test cannot show this.
    #[cfg(debug_assertions)]
    #[test]
    fn a_request_while_the_isabelle_prover_is_down_ends_undecided() {
        let dir = TempDir::new("down");
        // The repository's rules, so the pin passes and the start fails (PW I8: missing rules would
        // be `wrong_rules`).
        let config = config(&dir.data(), true, Some(pinned(&dir.0)));
        let mut options = ProverOptions::new(&pinned(&dir.0), &config.data_dir);
        // No wait after the first failure: down for the rest of the run.
        options.restart_waits = Vec::new();
        let plan = Plan {
            prover: ProverChoice::Isabelle(Box::new(options)),
            work_dir: config.data_dir.join("prover"),
            practice: None,
            agent: Some(crate::synthetic::SyntheticAgent::new()),
        };
        let (mut broker, started) = start(&config, &plan);
        assert!(broker.store_ready());
        let (sender, events) = mpsc::sync_channel(QUEUED_EVENTS);
        let mut shell = Shell::new(Vec::new(), plan, sender);
        shell.emit(&broker, started).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut n = 0;
        let mut ask = |shell: &mut Shell<Vec<u8>>, broker: &mut Broker, method, params| {
            n += 1;
            owner(shell, broker, &format!("sh06-{n}"), method, params)
        };
        let pump = |shell: &mut Shell<Vec<u8>>, broker: &mut Broker| {
            assert!(
                Instant::now() < deadline,
                "the run did not get there in time"
            );
            let cap = now_ms() + 50;
            let due = shell.next_due_ms(broker).map_or(cap, |due| due.min(cap));
            let input = match next_event(&events, Some(due)) {
                None => Input::Tick,
                Some(event) => input_of(event).expect("no end of input here"),
            };
            shell.turn(broker, input, now_ms()).unwrap();
        };
        loop {
            let snapshot = ask(
                &mut shell,
                &mut broker,
                "get_snapshot",
                serde_json::json!({}),
            );
            if snapshot["health"]["prover"] == "unavailable" {
                let cursor = snapshot["cursor"].clone();
                ask(
                    &mut shell,
                    &mut broker,
                    "subscribe",
                    serde_json::json!({"cursor": cursor}),
                );
                break;
            }
            pump(&mut shell, &mut broker);
        }
        // The snapshot lists open requests only; an ended one is seen in its notices.
        let request = loop {
            let ended = shell
                .stdout_mut()
                .split(|b| *b == b'\n')
                .filter_map(|l| serde_json::from_slice::<serde_json::Value>(l).ok())
                .filter(|v| v["change"]["kind"] == "request")
                .map(|v| v["change"]["request"].clone())
                .find(|r| r["status"]["state"] == "undecided");
            if let Some(request) = ended {
                break request;
            }
            pump(&mut shell, &mut broker);
        };
        let snapshot = ask(
            &mut shell,
            &mut broker,
            "get_snapshot",
            serde_json::json!({}),
        );
        assert_eq!(snapshot["health"]["prover"], "unavailable", "{snapshot}");
        // No wait is left, so the worker will not try again.
        assert_eq!(snapshot["health"]["prover_problem"], "failed", "{snapshot}");
        let details = ask(
            &mut shell,
            &mut broker,
            "get_request_details",
            serde_json::json!({"request_id": request["request_id"], "redaction": "redacted"}),
        );
        assert_eq!(details["proof_ids"], serde_json::json!([]), "{details}");
        assert!(details["authorization"].is_null(), "{details}");
        assert_eq!(details["attempts"], serde_json::json!([]), "{details}");
        // The worker's `NOT_RUNNING` reaches the store as its cause.
        assert_eq!(
            details["undecided_reason"], "prover_unavailable",
            "{details}"
        );
        assert!(matches!(shell.prover, ProverSlot::Isabelle(_)));
        assert!(shell.stop(&events));
    }

    /// PW sh03: a shell with no Isabelle worker, or one still waiting for the store, stops at once.
    #[test]
    fn stop_without_an_isabelle_worker_returns_at_once() {
        let dir = TempDir::new("stop");
        let isabelle = nowhere(&dir.0);
        let plans = [
            ProverChoice::None,
            ProverChoice::Isabelle(Box::new(ProverOptions::new(&isabelle, &dir.data()))),
        ];
        for prover in plans {
            let (sender, events) = mpsc::sync_channel(QUEUED_EVENTS);
            let plan = Plan {
                prover,
                work_dir: dir.data().join("prover"),
                #[cfg(debug_assertions)]
                practice: None,
                #[cfg(debug_assertions)]
                agent: None,
            };
            let shell = Shell::new(Vec::new(), plan, sender);
            let began = Instant::now();
            assert!(shell.stop(&events));
            assert!(began.elapsed() < Duration::from_millis(100));
        }
        assert!(!dir.data().exists());
    }

    /// The Isabelle paths of [`nowhere`], but with the repository's rules, so the pin passes and
    /// a worker reaches its starter.
    #[cfg(debug_assertions)]
    fn pinned(root: &Path) -> IsabelleSettings {
        IsabelleSettings {
            rules_dir: Path::new(env!("CARGO_MANIFEST_DIR")).join("../../isabelle"),
            ..nowhere(root)
        }
    }

    /// A gate whose check waits until the clock is past `deadline` and then panics, so the
    /// worker thread ends with its check still marked running and no result sent. It never
    /// proves. The test sets `deadline` once its worker is ready, so no setup time counts.
    #[cfg(debug_assertions)]
    struct PanickingGate {
        deadline: std::sync::Arc<std::sync::atomic::AtomicU64>,
    }

    #[cfg(debug_assertions)]
    impl crate::prover::Gate for PanickingGate {
        fn check_within(
            &mut self,
            _: &fabric_broker_policy::Context,
            _: &fabric_broker_policy::Action,
            _: Duration,
        ) -> Result<fabric_broker_policy::Check, fabric_broker_policy::PolicyError> {
            while now_ms() <= self.deadline.load(std::sync::atomic::Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(5));
            }
            panic!("the test gate's check panics");
        }

        fn is_broken(&self) -> bool {
            false
        }

        fn stop_within(&mut self, _: Duration) -> Result<(), fabric_broker_policy::PolicyError> {
            Ok(())
        }

        fn theory_sha256(&self) -> &str {
            crate::prover::built_theory_sha256()
        }
    }

    /// A proof job of the map tests with its own id and deadline; the core knows no such job, so
    /// it ignores its result.
    #[cfg(debug_assertions)]
    fn job_with(job_id: u64, deadline_ms: u64) -> ProofJob {
        let mut job = crate::prover::map::tests::job_of(crate::prover::map::tests::create());
        job.job_id = job_id;
        job.deadline_ms = deadline_ms;
        job
    }

    /// PW sh04 (PW 2.2): an Isabelle worker lost inside a check, past that check's deadline. Its
    /// loss is noticed once, at the start of a turn: the status to feed becomes `failed`, the job
    /// whose result is that turn's input leaves the handed jobs first, and the other jobs handed to
    /// it without a result are answered with `NOT_RUNNING`. One turn feeds them, answers the owner line it was
    /// given, and leaves no due time in the past, because the overrun poll still runs; its status,
    /// and a `ready` the worker queued before its loss, do not replace `failed`. Later jobs are
    /// answered at once, and a second loss sets nothing.
    #[cfg(debug_assertions)]
    #[test]
    fn a_lost_worker_feeds_unavailable_once() {
        use fabric_broker_core::ports::ProverCause;
        use std::sync::atomic::{AtomicU64, Ordering};

        let dir = TempDir::new("lost");
        let config = config(&dir.data(), false, Some(pinned(&dir.0)));
        let shared_deadline = std::sync::Arc::new(AtomicU64::new(0));
        let gate_deadline = std::sync::Arc::clone(&shared_deadline);
        let mut options = ProverOptions::new(&pinned(&dir.0), &config.data_dir);
        options.restart_waits = Vec::new();
        options.starter = std::sync::Arc::new(
            move |_: &fabric_broker_policy::GateConfig, _: &fabric_broker_policy::KillHandle| {
                Ok(Box::new(PanickingGate {
                    deadline: std::sync::Arc::clone(&gate_deadline),
                }) as Box<dyn crate::prover::Gate>)
            },
        );
        let plan = Plan {
            prover: ProverChoice::Isabelle(Box::new(options)),
            work_dir: config.data_dir.join("prover"),
            practice: None,
            agent: None,
        };
        let (mut broker, started) = start(&config, &plan);
        assert!(broker.store_ready());
        let (sender, events) = mpsc::sync_channel(QUEUED_EVENTS);
        let mut shell = Shell::new(Vec::new(), plan, sender);
        shell.emit(&broker, started).unwrap();
        // The worker's `ready`, held back until after its loss.
        let ready = events
            .recv_timeout(Duration::from_secs(10))
            .expect("the worker's status");
        assert!(matches!(ready, Event::Health(ProverStatus::Ready)));
        // Far enough for the first job to reach its check before its deadline.
        let deadline = now_ms() + 1_000;
        shared_deadline.store(deadline, Ordering::SeqCst);
        shell.route(
            &broker,
            vec![
                Job::Prove(job_with(6, deadline)),
                Job::Prove(job_with(7, deadline)),
                Job::Prove(job_with(8, deadline)),
            ],
        );
        assert_eq!(shell.handed.len(), 3);
        assert!(shell.pending_results.is_empty());
        let wait = Instant::now() + Duration::from_secs(10);
        while !matches!(&shell.prover, ProverSlot::Isabelle(prover) if prover.lost()) {
            assert!(Instant::now() < wait, "the worker was not lost");
            thread::sleep(Duration::from_millis(20));
        }
        let ProverSlot::Isabelle(prover) = &shell.prover else {
            unreachable!("the Isabelle worker runs");
        };
        assert_eq!(prover.next_due_ms(), Some(deadline), "still marked running");

        // The turn's input is job 7's result, as if it had come before the loss.
        let seven = map::no_report(&job_with(7, deadline), map::FAILED);
        shell.notice(&Input::ProofDone(seven), now_ms());
        assert_eq!(
            shell.pending_health,
            Some(ProverStatus::Unavailable(Some(ProverProblem::Failed)))
        );
        let answered = shell
            .pending_results
            .iter()
            .map(|done| (done.job_id, done.cause, done.undecided_reason.as_deref()))
            .collect::<Vec<_>>();
        let not_running = (Some(ProverCause::NotRunning), Some(map::NOT_RUNNING.text));
        assert_eq!(
            answered,
            [
                (6, not_running.0, not_running.1),
                (8, not_running.0, not_running.1)
            ]
        );
        assert!(shell.handed.is_empty());
        assert_eq!(shell.next_due_ms(&broker), Some(0));

        let snapshot = owner(
            &mut shell,
            &mut broker,
            "sh04-1",
            "get_snapshot",
            serde_json::json!({}),
        );
        assert_eq!(snapshot["health"]["prover"], "unavailable", "{snapshot}");
        assert_eq!(snapshot["health"]["prover_problem"], "failed", "{snapshot}");
        assert!(shell.pending_health.is_none() && shell.pending_results.is_empty());
        let now = now_ms();
        assert!(
            shell.next_due_ms(&broker).is_none_or(|due| due >= now),
            "a due time in the past"
        );

        shell
            .turn(&mut broker, input_of(ready).unwrap(), now_ms())
            .unwrap();
        let snapshot = owner(
            &mut shell,
            &mut broker,
            "sh04-2",
            "get_snapshot",
            serde_json::json!({}),
        );
        assert_eq!(snapshot["health"]["prover_problem"], "failed", "{snapshot}");

        shell.route(&broker, vec![Job::Prove(job_with(9, deadline))]);
        shell.lose();
        assert_eq!(shell.pending_health, None);
        assert_eq!(
            shell.pending_results,
            [map::no_report(&job_with(9, deadline), map::NOT_RUNNING)]
        );
        assert!(shell.handed.is_empty());
        assert_eq!(shell.next_due_ms(&broker), Some(0));
        shell.turn(&mut broker, Input::Tick, now_ms()).unwrap();
        assert!(shell.pending_health.is_none() && shell.pending_results.is_empty());
        assert!(shell.prover_lost);
        // The worker is gone: nothing acknowledges the stop.
        assert!(!shell.stop(&events));
    }

    /// PW sh07 (PW 2.2), end to end: the synthetic agent proposes with an Isabelle plan whose
    /// worker is lost. Within a few turns the request is `undecided`, its redacted details read
    /// `prover_unavailable`, and the health reads `unavailable` with `failed`. The worker here has
    /// a main loop that takes no event, so its first status ends it with no stop sent.
    #[cfg(debug_assertions)]
    #[test]
    fn a_request_while_the_isabelle_worker_is_lost_ends_undecided() {
        let dir = TempDir::new("lost-run");
        let config = config(&dir.data(), true, Some(pinned(&dir.0)));
        let options = || {
            let mut options = ProverOptions::new(&pinned(&dir.0), &config.data_dir);
            options.restart_waits = Vec::new();
            options.starter = std::sync::Arc::new(
                |_: &fabric_broker_policy::GateConfig, _: &fabric_broker_policy::KillHandle| {
                    Err(fabric_broker_policy::PolicyError::Isabelle(
                        "no heap".to_owned(),
                    ))
                },
            );
            options
        };
        let plan = Plan {
            prover: ProverChoice::Isabelle(Box::new(options())),
            work_dir: config.data_dir.join("prover"),
            practice: None,
            agent: Some(crate::synthetic::SyntheticAgent::new()),
        };
        let (mut broker, started) = start(&config, &plan);
        assert!(broker.store_ready());
        let (sender, events) = mpsc::sync_channel(QUEUED_EVENTS);
        let mut shell = Shell::new(Vec::new(), plan, sender);
        shell.prover = ProverSlot::Isabelle(IsabelleProver::spawn(options(), Box::new(|_| false)));
        shell.emit(&broker, started).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut n = 0;
        let mut ask = |shell: &mut Shell<Vec<u8>>, broker: &mut Broker, method, params| {
            n += 1;
            owner(shell, broker, &format!("lost-{n}"), method, params)
        };
        let snapshot = ask(
            &mut shell,
            &mut broker,
            "get_snapshot",
            serde_json::json!({}),
        );
        ask(
            &mut shell,
            &mut broker,
            "subscribe",
            serde_json::json!({"cursor": snapshot["cursor"].clone()}),
        );
        let request = loop {
            let ended = shell
                .stdout_mut()
                .split(|b| *b == b'\n')
                .filter_map(|l| serde_json::from_slice::<serde_json::Value>(l).ok())
                .filter(|v| v["change"]["kind"] == "request")
                .map(|v| v["change"]["request"].clone())
                .find(|r| r["status"]["state"] == "undecided");
            if let Some(request) = ended {
                break request;
            }
            assert!(
                Instant::now() < deadline,
                "the run did not get there in time"
            );
            let cap = now_ms() + 50;
            let due = shell.next_due_ms(&broker).map_or(cap, |due| due.min(cap));
            let input = match next_event(&events, Some(due)) {
                None => Input::Tick,
                Some(event) => input_of(event).expect("no end of input here"),
            };
            shell.turn(&mut broker, input, now_ms()).unwrap();
        };
        assert!(shell.prover_lost);
        let snapshot = ask(
            &mut shell,
            &mut broker,
            "get_snapshot",
            serde_json::json!({}),
        );
        assert_eq!(snapshot["health"]["prover"], "unavailable", "{snapshot}");
        assert_eq!(snapshot["health"]["prover_problem"], "failed", "{snapshot}");
        let details = ask(
            &mut shell,
            &mut broker,
            "get_request_details",
            serde_json::json!({"request_id": request["request_id"], "redaction": "redacted"}),
        );
        assert_eq!(
            details["undecided_reason"], "prover_unavailable",
            "{details}"
        );
        assert_eq!(details["proof_ids"], serde_json::json!([]), "{details}");
        let _ = shell.stop(&events);
    }

    /// PW sh09 (PW 2.2): a failed `prove` send is a sign of loss on its own. With the worker
    /// ended and its loss not yet noticed, a job the shell cannot hand over sets `failed` once and
    /// answers the jobs handed earlier, then itself, with `NOT_RUNNING`.
    #[cfg(debug_assertions)]
    #[test]
    fn a_failed_prove_send_is_a_sign_of_loss() {
        use std::sync::atomic::AtomicU64;

        let dir = TempDir::new("send-lost");
        let mut options = ProverOptions::new(&pinned(&dir.0), &dir.data());
        options.restart_waits = Vec::new();
        options.starter = std::sync::Arc::new(
            |_: &fabric_broker_policy::GateConfig, _: &fabric_broker_policy::KillHandle| {
                Ok(Box::new(PanickingGate {
                    deadline: std::sync::Arc::new(AtomicU64::new(0)),
                }) as Box<dyn crate::prover::Gate>)
            },
        );
        // The main loop is gone before the worker's first event, so the worker ends at it.
        let prover = IsabelleProver::spawn(options, Box::new(|_| false));
        let wait = Instant::now() + Duration::from_secs(10);
        while !prover.lost() {
            assert!(Instant::now() < wait, "the worker did not end");
            thread::sleep(Duration::from_millis(20));
        }
        let (sender, _events) = mpsc::sync_channel(QUEUED_EVENTS);
        let plan = Plan {
            prover: ProverChoice::None,
            work_dir: dir.data().join("prover"),
            practice: None,
            agent: None,
        };
        let mut shell = Shell::new(Vec::new(), plan, sender);
        shell.prover = ProverSlot::Isabelle(prover);
        let earlier = [job_with(5, u64::MAX), job_with(6, u64::MAX)];
        for job in &earlier {
            shell
                .handed
                .push((job.job_id, job.request_id.clone(), job.purpose));
        }
        let job = job_with(9, u64::MAX);
        shell.prove(job.clone());
        assert_eq!(
            shell.pending_health,
            Some(ProverStatus::Unavailable(Some(ProverProblem::Failed)))
        );
        assert_eq!(
            shell.pending_results,
            [
                map::no_report(&earlier[0], map::NOT_RUNNING),
                map::no_report(&earlier[1], map::NOT_RUNNING),
                map::no_report(&job, map::NOT_RUNNING),
            ]
        );
        assert!(shell.prover_lost && shell.handed.is_empty());
    }

    /// PW sh10 (PW 2.2): while a job is out with an Isabelle worker not known to be lost, the next
    /// due time is no later than the last turn's time plus `LOSS_LOOK_MS`. With no job out, or once
    /// the loss is known, it is the earliest of the other due times.
    #[cfg(debug_assertions)]
    #[test]
    fn a_job_out_bounds_the_next_due_time() {
        use std::sync::atomic::AtomicU64;

        let dir = TempDir::new("look");
        let config = config(&dir.data(), false, Some(pinned(&dir.0)));
        let mut options = ProverOptions::new(&pinned(&dir.0), &config.data_dir);
        options.restart_waits = Vec::new();
        options.starter = std::sync::Arc::new(
            |_: &fabric_broker_policy::GateConfig, _: &fabric_broker_policy::KillHandle| {
                Ok(Box::new(PanickingGate {
                    deadline: std::sync::Arc::new(AtomicU64::new(0)),
                }) as Box<dyn crate::prover::Gate>)
            },
        );
        let plan = Plan {
            prover: ProverChoice::None,
            work_dir: config.data_dir.join("prover"),
            practice: None,
            agent: None,
        };
        let (broker, _) = start(&config, &plan);
        let (sender, _events) = mpsc::sync_channel(QUEUED_EVENTS);
        let mut shell = Shell::new(Vec::new(), plan, sender);
        // The main loop is gone before the worker's first event, so the worker ends at it.
        shell.prover = ProverSlot::Isabelle(IsabelleProver::spawn(options, Box::new(|_| false)));
        let ProverSlot::Isabelle(prover) = &shell.prover else {
            unreachable!("the Isabelle worker runs");
        };
        let others = [broker.next_due_ms(), prover.next_due_ms()]
            .into_iter()
            .flatten()
            .min();
        let last = now_ms();
        shell.last_turn_ms = last;
        assert_eq!(shell.next_due_ms(&broker), others, "no job out");

        let job = job_with(6, u64::MAX);
        shell
            .handed
            .push((job.job_id, job.request_id.clone(), job.purpose));
        let look = last + LOSS_LOOK_MS;
        let due = shell.next_due_ms(&broker);
        assert!(due.is_some_and(|due| due <= look), "{due:?}");
        assert_eq!(due, Some(others.map_or(look, |other| other.min(look))));

        shell.prover_lost = true;
        assert_eq!(shell.next_due_ms(&broker), others, "the loss is known");
    }

    /// PW sh11 (PW 2.2), end to end: a worker that panics inside a check whose deadline is 30 s
    /// ahead, with no other event sent. The loop of `main.rs` wakes within `LOSS_LOOK_MS` of each
    /// turn, so within a few looks, well before the deadline, the job is answered `NOT_RUNNING`
    /// and the status is `failed`.
    #[cfg(debug_assertions)]
    #[test]
    fn a_worker_lost_with_a_job_out_is_noticed_within_a_look() {
        use fabric_broker_core::ports::ProverCause;
        use std::sync::atomic::AtomicU64;

        let dir = TempDir::new("look-run");
        let config = config(&dir.data(), false, Some(pinned(&dir.0)));
        let mut options = ProverOptions::new(&pinned(&dir.0), &config.data_dir);
        options.restart_waits = Vec::new();
        // The check panics at once.
        options.starter = std::sync::Arc::new(
            |_: &fabric_broker_policy::GateConfig, _: &fabric_broker_policy::KillHandle| {
                Ok(Box::new(PanickingGate {
                    deadline: std::sync::Arc::new(AtomicU64::new(0)),
                }) as Box<dyn crate::prover::Gate>)
            },
        );
        let plan = Plan {
            prover: ProverChoice::Isabelle(Box::new(options)),
            work_dir: config.data_dir.join("prover"),
            practice: None,
            agent: None,
        };
        let (mut broker, started) = start(&config, &plan);
        assert!(broker.store_ready());
        let (sender, events) = mpsc::sync_channel(QUEUED_EVENTS);
        let mut shell = Shell::new(Vec::new(), plan, sender);
        shell.emit(&broker, started).unwrap();
        let ready = events
            .recv_timeout(Duration::from_secs(10))
            .expect("the worker's status");
        assert!(matches!(ready, Event::Health(ProverStatus::Ready)));
        shell
            .turn(&mut broker, input_of(ready).unwrap(), now_ms())
            .unwrap();

        let job = job_with(6, now_ms() + 30_000);
        shell.route(&broker, vec![Job::Prove(job.clone())]);
        assert_eq!(shell.handed.len(), 1);
        let began = Instant::now();
        // The test's own bound, far past a look, so a loop that would sleep longer (until the
        // check's deadline, or with no due time at all until an event that never comes) fails
        // here instead of hanging.
        let bound = now_ms() + 5_000;
        let mut looks = 0;
        let mut answered = Vec::new();
        while !shell.prover_lost {
            assert!(
                began.elapsed() < Duration::from_secs(5),
                "the loss was not noticed in time"
            );
            // As in `main.rs`, but no later than the test's bound.
            let due = shell
                .next_due_ms(&broker)
                .map_or(bound, |due| due.min(bound));
            let input = match next_event(&events, Some(due)) {
                None => Input::Tick,
                Some(_) => panic!("no event is sent here"),
            };
            looks += 1;
            let now = now_ms();
            // The results this turn feeds, seen before `turn` drains them; `turn` notices again,
            // which sets nothing a second time.
            shell.notice(&input, now);
            answered.extend(
                shell
                    .pending_results
                    .iter()
                    .map(|done| (done.job_id, done.cause)),
            );
            shell.turn(&mut broker, input, now).unwrap();
        }
        let took = began.elapsed();
        assert!(took < Duration::from_secs(5), "took {took:?}");
        assert_eq!(answered, [(job.job_id, Some(ProverCause::NotRunning))]);
        assert!(shell.handed.is_empty() && shell.pending_results.is_empty());
        let snapshot = owner(
            &mut shell,
            &mut broker,
            "sh11-1",
            "get_snapshot",
            serde_json::json!({}),
        );
        assert_eq!(snapshot["health"]["prover"], "unavailable", "{snapshot}");
        assert_eq!(snapshot["health"]["prover_problem"], "failed", "{snapshot}");
        eprintln!("sh11: noticed after {looks} looks in {took:?}");
        // The worker is gone: nothing acknowledges the stop.
        assert!(!shell.stop(&events));
    }

    /// PW sh08 (PW 2.2): a closed classifier channel, no prover, and an Isabelle prover still
    /// waiting for the store each answer a job at once with `NOT_RUNNING`, and leave the status as
    /// it is.
    #[test]
    fn a_job_no_prover_can_take_is_answered_not_running() {
        let dir = TempDir::new("no-taker");
        #[cfg_attr(
            not(debug_assertions),
            allow(unused_mut, reason = "the classifier is debug only")
        )]
        let mut slots = vec![
            ProverSlot::None,
            ProverSlot::Waiting(Box::new(ProverOptions::new(&nowhere(&dir.0), &dir.data()))),
        ];
        #[cfg(debug_assertions)]
        {
            let (jobs, received) = mpsc::channel();
            drop(received);
            slots.push(ProverSlot::Classifier(jobs));
        }
        for slot in slots {
            let (sender, events) = mpsc::sync_channel(QUEUED_EVENTS);
            let plan = Plan {
                prover: ProverChoice::None,
                work_dir: dir.data().join("prover"),
                #[cfg(debug_assertions)]
                practice: None,
                #[cfg(debug_assertions)]
                agent: None,
            };
            let mut shell = Shell::new(Vec::new(), plan, sender);
            shell.prover = slot;
            let job = crate::prover::map::tests::job_of(crate::prover::map::tests::create());
            shell.prove(job.clone());
            assert_eq!(
                shell.pending_results,
                [map::no_report(&job, map::NOT_RUNNING)]
            );
            assert_eq!(shell.pending_health, None);
            assert!(!shell.prover_lost && shell.handed.is_empty());
            assert!(shell.stop(&events));
        }
        assert!(!dir.data().exists());
    }
}
