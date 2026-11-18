//! The broker facade the process shell calls (design 10.4): start, one input at a time, due
//! work, quit; the unavailable mode while the store cannot be opened, its reopen, and the halt
//! after a failed write.

#[cfg(debug_assertions)]
use fabric_desktop_contracts::owner::{Account, Destination};
#[cfg(debug_assertions)]
use fabric_desktop_contracts::scalar::{Id, Label};
use fabric_desktop_contracts::{Frame, Limits};
use fabric_storage::{OpenError, Store, StoreFault};

use crate::owner::OwnerChannel;
use crate::ports::*;
use crate::service::core::{Core, CoreOpenError};

/// The broker's name in `hello`.
pub const BROKER_NAME: &str = "fabric-broker";
/// A proof without a result after this long is undecided.
pub const PROOF_WATCHDOG_MS: u64 = 35_000;
/// Re-proofs per purpose over a request's life.
pub const MAX_REPROOFS: u32 = 3;
/// A provider call without an answer after this long has no answer.
pub const PROVIDER_CALL_MS: u64 = 30_000;
/// How often the unavailable mode may try to open the store again.
pub const REOPEN_INTERVAL_MS: u64 = 5_000;
/// A session with no call for this long ends.
pub const SESSION_IDLE_MS: i64 = 1_800_000;
/// The window of one `busy` audit row per agent.
pub const BUSY_AUDIT_WINDOW_MS: i64 = 60_000;

/// How a transient provider failure is retried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Retries after the first attempt.
    pub max_retries: u32,
    /// The wait before each retry.
    pub backoff_ms: [u64; 3],
    /// The whole time a request may spend retrying.
    pub budget_ms: u64,
}

impl RetryPolicy {
    /// The plan's policy: three retries after 2, 4 and 8 seconds, within two minutes.
    pub const PLAN: RetryPolicy = RetryPolicy {
        max_retries: 3,
        backoff_ms: [2_000, 4_000, 8_000],
        budget_ms: 120_000,
    };
}

/// The broker's fixed settings.
#[derive(Clone, Debug)]
pub struct Settings {
    /// The build named in `hello`.
    pub build: String,
    /// The request limits.
    pub limits: Limits,
    /// The retry policy.
    pub retry: RetryPolicy,
}

impl Settings {
    /// `Limits::CD_G0`, `RetryPolicy::PLAN`.
    #[must_use]
    pub fn cd_w04(build: String) -> Settings {
        Settings {
            build,
            limits: Limits::CD_G0,
            retry: RetryPolicy::PLAN,
        }
    }
}

/// Fixed at start (D13).
#[derive(Clone, Debug)]
pub struct Attachments {
    /// The prover, if one runs.
    pub prover: Option<ProverIdentity>,
    /// True when a provider worker runs.
    pub provider_present: bool,
}

/// Opens the store; called by `Core::open` at start and for every reopen attempt.
pub type Opener = Box<dyn FnMut(i64) -> Result<Store, OpenError> + Send>;

/// An agent the seed enrolls.
#[cfg(debug_assertions)]
#[derive(Clone, Debug)]
pub struct SeedAgent {
    /// The agent.
    pub agent_id: Id,
    /// Its enrolled name.
    pub registered_name: Label,
    /// Where data approved for it goes.
    pub destination: Destination,
}

/// Records created if absent (synthetic agent and tests now; CD-W06 enrollment later). Debug builds only (D11, D26).
#[cfg(debug_assertions)]
#[derive(Clone, Debug, Default)]
pub struct Seed {
    /// Agents to enroll.
    pub agents: Vec<SeedAgent>,
    /// Accounts to add.
    pub accounts: Vec<Account>,
}

/// One input to the broker.
#[allow(
    clippy::large_enum_variant,
    reason = "one input at a time on the main thread"
)]
pub enum Input {
    /// One owner line, without its newline.
    OwnerLine(Vec<u8>),
    /// An owner line longer than the frame limit.
    OwnerOversized,
    /// Something from the agent adapter.
    Agent(AgentInput),
    /// A prover worker's result.
    ProofDone(ProofDone),
    /// A provider worker's result.
    Attempt(AttemptDone),
    /// The prover's status changed: its health, or only the problem while unavailable.
    ProverHealth(ProverStatus),
    /// Due work only.
    Tick,
}

/// Work for a worker thread.
#[allow(clippy::large_enum_variant, reason = "jobs are few and moved once")]
pub enum Job {
    /// A proof for the prover worker.
    Prove(ProofJob),
    /// An attempt for the provider worker.
    Dispatch(DispatchOrder),
}

/// What one call to the broker produced.
#[derive(Default)]
pub struct Output {
    /// Frames for the owner channel, in order.
    pub owner: Vec<Frame>,
    /// Frames for agent connections.
    pub agent: Vec<AgentReply>,
    /// Agent connections to close.
    pub close: Vec<ConnectionId>,
    /// Empty when `halt` is set: a halted step's jobs are dropped (D7).
    pub jobs: Vec<Job>,
    /// Fixed-text lines for stderr (the clock line of 5.1).
    pub stderr: Vec<String>,
    /// Set after a failed write: main writes `owner`, prints the fault, exits 75.
    pub halt: Option<StoreFault>,
}

/// Why the broker could not start.
#[derive(Debug)]
pub enum StartError {
    /// Exit 78.
    Config(OpenError),
    /// Exit 75.
    Halt(StoreFault),
}

impl std::fmt::Display for StartError {
    /// Fixed text.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartError::Config(error) => write!(f, "the broker cannot start: {error}"),
            StartError::Halt(fault) => {
                write!(f, "the broker stopped while recovering its store: {fault}")
            }
        }
    }
}

/// The core when the store is open, or the reason it is not.
#[allow(
    clippy::large_enum_variant,
    reason = "one state per broker, replaced only on reopen"
)]
enum State {
    Ready(Core),
    Unavailable {
        reason: fabric_storage::Unavailable,
        last_attempt_ms: u64,
    },
}

/// The broker.
pub struct Broker {
    state: State,
    owner: OwnerChannel,
    opener: Opener,
    settings: Settings,
    attachments: Attachments,
    /// The fault of a failed write: the run is over (D7); every later call answers it again.
    halted: Option<StoreFault>,
    /// True after `quit`: no input is taken and no job begins.
    stopped: bool,
}

/// Moves what the core's steps produced into `out`; the notices go through the owner channel,
/// which rings them and writes them while subscribed.
fn drain(core: &mut Core, owner: &mut OwnerChannel, out: &mut Output) {
    let outbox = core.take_outbox();
    out.owner.extend(owner.publish(outbox.notices));
    out.agent.extend(outbox.agent);
    out.close.extend(outbox.close);
    out.jobs.extend(outbox.jobs);
    out.stderr.extend(outbox.stderr);
}

/// True when the line is an owner request that needs the store: every method but `hello`. A
/// line that does not decode is refused without the store.
fn needs_store(line: &[u8]) -> bool {
    matches!(
        fabric_desktop_contracts::decode(fabric_desktop_contracts::Entry::OwnerRequest, line),
        Ok(Frame::OwnerRequest(request))
            if request.call.method() != fabric_desktop_contracts::owner::OwnerMethod::Hello
    )
}

/// One input against a ready core: due work, the input, due work again (5.1).
fn step_ready(
    core: &mut Core,
    owner: &mut OwnerChannel,
    input: Input,
    raw_now_ms: u64,
    out: &mut Output,
) -> Result<(), StoreFault> {
    let now = core.now_ms(raw_now_ms);
    core.pump()?;
    drain(core, owner, out);
    match input {
        Input::OwnerLine(line) => {
            let (frames, fault) = owner.answer(core, &line, now);
            drain(core, owner, out);
            out.owner.extend(frames);
            if let Some(fault) = fault {
                return Err(fault);
            }
        }
        Input::OwnerOversized => out.owner.push(owner.oversized()),
        Input::Agent(input) => core.agent(input)?,
        Input::ProofDone(done) => core.proof_done(done)?,
        Input::Attempt(done) => core.attempt_done(done)?,
        Input::ProverHealth(status) => core.set_prover_health(status)?,
        Input::Tick => {}
    }
    drain(core, owner, out);
    core.pump()?;
    drain(core, owner, out);
    Ok(())
}

impl Broker {
    /// `Core::open` (store, clock, recovery 5.8, first pump); the unavailable mode on `OpenError::Unavailable`.
    ///
    /// # Errors
    ///
    /// [`StartError::Config`] for a configuration failure of the store; [`StartError::Halt`]
    /// when the recovery transaction fails.
    pub fn start(
        mut opener: Opener,
        settings: Settings,
        attachments: Attachments,
        now_ms: u64,
    ) -> Result<(Broker, Output), StartError> {
        let mut owner = OwnerChannel::new(settings.build.clone());
        let mut out = Output::default();
        let state = match Core::open(&mut opener, settings.clone(), attachments.clone(), now_ms) {
            Ok((core, opened)) => {
                owner.begin_run(opened.notice_sequence, opened.notices);
                out.jobs = opened.jobs;
                State::Ready(core)
            }
            Err(CoreOpenError::Open(OpenError::Unavailable(reason))) => State::Unavailable {
                reason,
                last_attempt_ms: now_ms,
            },
            Err(CoreOpenError::Open(error)) => return Err(StartError::Config(error)),
            Err(CoreOpenError::Halt(fault)) => return Err(StartError::Halt(fault)),
        };
        Ok((
            Broker {
                state,
                owner,
                opener,
                settings,
                attachments,
                halted: None,
                stopped: false,
            },
            out,
        ))
    }

    /// Due work, the input, due work again. In the unavailable mode an owner frame that needs the store may first
    /// try `Core::open` again (3.4 table).
    pub fn handle(&mut self, input: Input, now_ms: u64) -> Output {
        let mut out = Output::default();
        if let Some(fault) = &self.halted {
            out.halt = Some(fault.clone());
            return out;
        }
        if self.stopped {
            return out;
        }
        let result = match self.state {
            State::Ready(_) => self.handle_ready(input, now_ms, &mut out),
            State::Unavailable { .. } => self.handle_unavailable(input, now_ms, &mut out),
        };
        if let Err(fault) = result {
            self.halt(fault, &mut out);
        }
        out.owner = self.owner.fit_frames(std::mem::take(&mut out.owner));
        out
    }

    /// One input against the ready core.
    fn handle_ready(
        &mut self,
        input: Input,
        now_ms: u64,
        out: &mut Output,
    ) -> Result<(), StoreFault> {
        let Broker { state, owner, .. } = self;
        let State::Ready(core) = state else {
            return Ok(());
        };
        let result = step_ready(core, owner, input, now_ms, out);
        if result.is_err() {
            drain(core, owner, out);
        }
        result
    }

    /// One input while the store cannot be used (3.4, D8, D9): `hello` answers, every other
    /// method `store_unavailable`; an owner frame that needs the store first tries to reopen it
    /// when the reason may pass and 5 s went by since the last attempt. A reopen runs the whole
    /// `Core::open`, recovery and first pump included, before the frame is answered.
    fn handle_unavailable(
        &mut self,
        input: Input,
        now_ms: u64,
        out: &mut Output,
    ) -> Result<(), StoreFault> {
        let State::Unavailable {
            reason,
            last_attempt_ms,
        } = &mut self.state
        else {
            return Ok(());
        };
        match input {
            Input::OwnerLine(line) => {
                let due = now_ms.saturating_sub(*last_attempt_ms) >= REOPEN_INTERVAL_MS;
                if needs_store(&line) && reason.retry_may_help() && due {
                    *last_attempt_ms = now_ms;
                    match Core::open(
                        &mut self.opener,
                        self.settings.clone(),
                        self.attachments.clone(),
                        now_ms,
                    ) {
                        Ok((core, opened)) => {
                            self.owner.begin_run(opened.notice_sequence, opened.notices);
                            out.jobs.extend(opened.jobs);
                            self.state = State::Ready(core);
                            return self.handle_ready(Input::OwnerLine(line), now_ms, out);
                        }
                        Err(CoreOpenError::Open(OpenError::Unavailable(next))) => *reason = next,
                        // The configuration was accepted at start; keep the reason in hand.
                        Err(CoreOpenError::Open(OpenError::Config { .. })) => {}
                        Err(CoreOpenError::Halt(fault)) => return Err(fault),
                    }
                }
                let State::Unavailable { reason, .. } = &self.state else {
                    return Ok(());
                };
                out.owner
                    .extend(self.owner.answer_unavailable(reason, &line));
            }
            Input::OwnerOversized => out.owner.push(self.owner.oversized()),
            Input::Agent(AgentInput::Connect { connection, .. }) => out.close.push(connection),
            Input::Agent(AgentInput::Call {
                connection,
                request,
            }) => out.agent.push(crate::service::agent::error_reply(
                connection,
                request.id,
                request.call.method(),
                fabric_desktop_contracts::error::AgentFault::Internal,
                "The broker's store is unavailable; nothing was recorded.",
            )),
            Input::Agent(AgentInput::Disconnect { .. })
            | Input::ProofDone(_)
            | Input::Attempt(_)
            | Input::ProverHealth(_)
            | Input::Tick => {}
        }
        Ok(())
    }

    /// A failed write (5.2, D7): the step's jobs are dropped and the run is over.
    fn halt(&mut self, fault: StoreFault, out: &mut Output) {
        out.jobs.clear();
        out.halt = Some(fault.clone());
        self.halted = Some(fault);
    }

    /// Earliest of the store's due requests, live sessions' idle deadlines, proof watchdogs and `busy` windows.
    #[must_use]
    pub fn next_due_ms(&self) -> Option<u64> {
        if self.halted.is_some() || self.stopped {
            return None;
        }
        match &self.state {
            State::Ready(core) => core.next_due_raw_ms(),
            State::Unavailable { .. } => None,
        }
    }

    /// End of stdin (5.8).
    pub fn quit(&mut self, now_ms: u64) -> Output {
        let mut out = Output::default();
        if let Some(fault) = &self.halted {
            out.halt = Some(fault.clone());
            return out;
        }
        if self.stopped {
            return out;
        }
        self.stopped = true;
        let Broker { state, owner, .. } = self;
        let State::Ready(core) = state else {
            return out;
        };
        let _ = core.now_ms(now_ms);
        let result = core.quit();
        drain(core, owner, &mut out);
        if let Err(fault) = result {
            self.halt(fault, &mut out);
        }
        out.owner = self.owner.fit_frames(std::mem::take(&mut out.owner));
        out
    }

    /// Creates the seed's records if absent. Debug builds only.
    #[cfg(debug_assertions)]
    pub fn seed(&mut self, seed: &Seed, now_ms: u64) -> Output {
        let mut out = Output::default();
        if let Some(fault) = &self.halted {
            out.halt = Some(fault.clone());
            return out;
        }
        if self.stopped {
            return out;
        }
        let Broker { state, owner, .. } = self;
        let State::Ready(core) = state else {
            return out;
        };
        let _ = core.now_ms(now_ms);
        let result = core.seed(seed).and_then(|()| {
            drain(core, owner, &mut out);
            core.pump()
        });
        drain(core, owner, &mut out);
        if let Err(fault) = result {
            self.halt(fault, &mut out);
        }
        out.owner = self.owner.fit_frames(std::mem::take(&mut out.owner));
        out
    }

    /// True while the store is open.
    #[must_use]
    pub fn store_ready(&self) -> bool {
        matches!(self.state, State::Ready(_))
    }

    /// True from the first successful `subscribe` answered in this process on (5.10).
    #[must_use]
    pub fn subscribed(&self) -> bool {
        self.owner.ever_subscribed()
    }
}
