//! The core: the open store, the clock, the activity of this run, and the one writer (5.1).
//!
//! Every write goes through [`Core::write`]: one store transaction whose closure gets the
//! step's context ([`Cx`]); what the commit changes in memory (sessions, proof jobs in flight)
//! is applied only after it returned. Notices, jobs, agent replies and stderr lines of the
//! service's own steps collect in an outbox the broker drains after each call.

use std::collections::HashMap;
use std::convert::Infallible;

use fabric_desktop_contracts::owner::{
    AgentChannelHealth, Change, Health, OwnerNotice, ProofPurpose, ProverProblem, StoreHealth,
};
use fabric_desktop_contracts::scalar::{Digest, Id};
use fabric_storage::{
    Actor, AuditEntry, Committed, Fence, OpMeta, ProverKind, ReadTx, Store, StoreFault, TxError,
    WriteTx,
};

use crate::broker::{Attachments, Job, Opener, Settings};
use crate::ports::{AgentReply, ConnectionId, ProverStatus};
use crate::service::clock::Clock;
use crate::service::limits::BusyWindows;
use crate::service::{Activity, AfterCommit, Cx};

/// The most steps one pump takes; each step commits, so this only bounds a fault of the pump.
const PUMP_STEPS: usize = 10_000;

/// Why `Core::open` failed.
pub(crate) enum CoreOpenError {
    Open(fabric_storage::OpenError),
    Halt(StoreFault),
}

/// What the recovery transaction and the first pump produced.
pub(crate) struct Opened {
    pub(crate) notices: Vec<OwnerNotice>,
    pub(crate) jobs: Vec<Job>,
    /// `meta.notice_sequence` before the recovery: the base of this run's notice ring (5.4).
    pub(crate) notice_sequence: u64,
}

/// A proof job handed out and not answered yet (memory only, 5.1).
#[derive(Clone, Debug)]
pub(crate) struct InFlight {
    pub(crate) request_id: Id,
    pub(crate) purpose: ProofPurpose,
    pub(crate) fence: Fence,
    pub(crate) deadline_ms: i64,
    /// The digest of the action the job proves: a report must name it (5.7, PLAN 8).
    pub(crate) action_digest: Digest,
    /// The ids of the job's `facts.grants`: the only ids a result may name (5.7, D17).
    pub(crate) grant_ids: Vec<Id>,
}

/// An agent connection of this run and the session it opened.
#[derive(Clone, Debug)]
pub(crate) struct Connection {
    pub(crate) agent_id: Id,
    pub(crate) session_id: Id,
}

/// What the service's own steps produced for the broker's output.
#[derive(Default)]
pub(crate) struct Outbox {
    pub(crate) notices: Vec<OwnerNotice>,
    pub(crate) jobs: Vec<Job>,
    pub(crate) agent: Vec<AgentReply>,
    pub(crate) close: Vec<ConnectionId>,
    pub(crate) stderr: Vec<String>,
}

/// The core over an open store.
pub(crate) struct Core {
    pub(in crate::service) store: Store,
    pub(in crate::service) settings: Settings,
    pub(in crate::service) attachments: Attachments,
    pub(in crate::service) run_id: i64,
    pub(in crate::service) clock: Clock,
    pub(in crate::service) activity: Activity,
    /// The step's `now` (D34), set by [`Core::now_ms`].
    pub(in crate::service) now: i64,
    pub(in crate::service) prover_health: ProverStatus,
    pub(in crate::service) in_flight: HashMap<u64, InFlight>,
    pub(in crate::service) next_job_id: u64,
    pub(in crate::service) connections: HashMap<ConnectionId, Connection>,
    pub(in crate::service) busy: BusyWindows,
    pub(in crate::service) outbox: Outbox,
}

impl Core {
    /// The only constructor (D9, D30): open through `opener`, start the clock, run recovery (5.8) in one
    /// transaction, run the first pump.
    pub(crate) fn open(
        opener: &mut Opener,
        settings: Settings,
        attachments: Attachments,
        raw_now_ms: u64,
    ) -> Result<(Core, Opened), CoreOpenError> {
        let raw = i64::try_from(raw_now_ms).unwrap_or(i64::MAX);
        let store = opener(raw).map_err(CoreOpenError::Open)?;
        let (last_committed, meta) = store
            .read(|r| Ok((r.last_committed_ms()?, r.meta()?)))
            .map_err(CoreOpenError::Halt)?;
        let mut clock = Clock::start(last_committed, raw_now_ms);
        let (now, _) = clock.now(raw_now_ms);
        // A run's id is the ledger sequence its recovery row will take: every earlier run's id
        // is at most the head, so run ids rise (the `RunStarted` guard).
        let run_id = i64::try_from(meta.ledger_head_seq + 1)
            .map_err(|_| CoreOpenError::Halt(super::invariant("a run id fits in 63 bits")))?;
        // A run with no prover attached has none to wait for: unavailable, not configured.
        let prover_health = attachments.prover.as_ref().map_or(
            ProverStatus::Unavailable(Some(ProverProblem::NotConfigured)),
            |p| p.health,
        );
        let mut core = Core {
            store,
            settings,
            attachments,
            run_id,
            clock,
            activity: Activity::default(),
            now,
            prover_health,
            in_flight: HashMap::new(),
            next_job_id: 0,
            connections: HashMap::new(),
            busy: BusyWindows::default(),
            outbox: Outbox::default(),
        };
        core.recover().map_err(CoreOpenError::Halt)?;
        core.pump().map_err(CoreOpenError::Halt)?;
        let outbox = core.take_outbox();
        Ok((
            core,
            Opened {
                notices: outbox.notices,
                jobs: outbox.jobs,
                notice_sequence: meta.notice_sequence,
            },
        ))
    }

    /// One consistent read of the store.
    pub(crate) fn read<T>(
        &self,
        f: impl FnOnce(&ReadTx<'_>) -> Result<T, StoreFault>,
    ) -> Result<T, StoreFault> {
        self.store.read(f)
    }

    /// One write with the step's context. `meta.now_ms` is set to the step's `now`, so every
    /// ledger row carries the core clock (D34). The commit's notices go back to the caller; its
    /// session changes and jobs take effect only after it committed.
    pub(crate) fn write<T, E>(
        &mut self,
        meta: OpMeta,
        f: impl FnOnce(&mut WriteTx<'_>, &mut Cx<'_>) -> Result<T, TxError<E>>,
    ) -> Result<Committed<T>, TxError<E>> {
        let meta = OpMeta {
            now_ms: self.now,
            ..meta
        };
        let mut after = AfterCommit::default();
        let committed = {
            let Core {
                store,
                activity,
                now,
                run_id,
                ..
            } = self;
            store.write(meta, |tx| {
                let mut cx = Cx {
                    now_ms: *now,
                    run_id: *run_id,
                    activity,
                    after: &mut after,
                };
                f(tx, &mut cx)
            })?
        };
        self.after_commit(after);
        Ok(committed)
    }

    /// One audit-only transaction.
    pub(crate) fn write_audit(&mut self, entries: Vec<AuditEntry>) -> Result<(), StoreFault> {
        self.store.write_audit(self.now, entries)
    }

    /// The broker's health for the owner, from the prover's status: the only place a `Health`
    /// is built, so `prover_problem` is null unless the prover is unavailable.
    pub(crate) fn health(&self) -> Health {
        Health {
            prover: self.prover_health.health(),
            prover_problem: self.prover_health.problem(),
            store: StoreHealth::Ready,
            // This build has no agent channel listener.
            agent_channel: AgentChannelHealth::NotListening,
            agent_channel_problem: None,
        }
    }

    /// The fixed settings.
    pub(crate) fn settings(&self) -> &Settings {
        &self.settings
    }

    /// The activity of this run.
    pub(crate) fn activity(&self) -> &Activity {
        &self.activity
    }

    /// The step's `now` (D34); call once per step.
    pub(crate) fn now_ms(&mut self, raw_now_ms: u64) -> i64 {
        let before = self.clock.offset_ms();
        let (now, grew) = self.clock.now(raw_now_ms);
        if grew {
            self.outbox.stderr.push(format!(
                "the clock went back {} ms; time continues from the last commit",
                self.clock.offset_ms() - before
            ));
        }
        self.now = now;
        now
    }

    /// What the store's due requests, the live sessions' idle deadlines and the open `busy`
    /// windows next need, on the raw wall clock; `None` when nothing waits.
    pub(crate) fn next_due_raw_ms(&self) -> Option<u64> {
        let stored = self.store.read(|r| r.next_due_ms()).ok().flatten();
        [
            stored,
            self.activity.next_idle_ms(),
            self.busy.next_end_ms(),
        ]
        .into_iter()
        .flatten()
        .min()
        .map(|due| u64::try_from(due.saturating_sub(self.clock.offset_ms())).unwrap_or(0))
    }

    /// Takes what the service's steps produced since the last call.
    pub(crate) fn take_outbox(&mut self) -> Outbox {
        std::mem::take(&mut self.outbox)
    }

    /// The prover's kind, fixed at start (D13); `None` when no prover is attached.
    pub(in crate::service) fn prover_kind(&self) -> Option<ProverKind> {
        self.attachments.prover.as_ref().map(|p| p.kind)
    }

    /// The envelope of one of the service's own writes at the step's `now`.
    pub(in crate::service) fn meta(
        &self,
        operation_type: &'static str,
        actor: Actor,
        correlation_id: Option<&Id>,
        causation_id: Option<String>,
    ) -> OpMeta {
        OpMeta {
            operation_type,
            actor,
            idempotency: None,
            causation_id,
            correlation_id: correlation_id.cloned(),
            now_ms: self.now,
        }
    }

    /// One of the service's own writes: a fault of the store halts; the notices go to the
    /// outbox.
    pub(in crate::service) fn commit<T>(
        &mut self,
        meta: OpMeta,
        f: impl FnOnce(&mut WriteTx<'_>, &mut Cx<'_>) -> Result<T, StoreFault>,
    ) -> Result<T, StoreFault> {
        match self.write::<T, Infallible>(meta, |tx, cx| f(tx, cx).map_err(TxError::Fault)) {
            Ok(committed) => {
                self.outbox.notices.extend(committed.notices);
                Ok(committed.value)
            }
            Err(TxError::Fault(fault)) => Err(fault),
            Err(TxError::Refused(never)) => match never {},
        }
    }

    /// Applies what a commit changed in memory: sessions, then the connections they leave
    /// without a session, then the jobs, each proof numbered and kept in flight.
    fn after_commit(&mut self, after: AfterCommit) {
        // A request that falls due needs no wake list: the pump, which runs after every input
        // (5.1), finds it by its due time in the store.
        let AfterCommit { jobs, sessions } = after;
        for change in sessions {
            self.activity.apply(change);
        }
        let activity = &self.activity;
        let stale: Vec<ConnectionId> = self
            .connections
            .iter()
            .filter(|(_, c)| {
                activity
                    .sessions
                    .get(&c.agent_id)
                    .is_none_or(|live| live.session_id != c.session_id)
            })
            .map(|(connection, _)| *connection)
            .collect();
        for connection in stale {
            self.connections.remove(&connection);
            self.outbox.close.push(connection);
        }
        for job in jobs {
            match job {
                Job::Prove(mut job) => {
                    self.next_job_id += 1;
                    job.job_id = self.next_job_id;
                    self.in_flight.retain(|_, f| f.request_id != job.request_id);
                    self.in_flight.insert(
                        job.job_id,
                        InFlight {
                            request_id: job.request_id.clone(),
                            purpose: job.purpose,
                            fence: job.fence.clone(),
                            deadline_ms: i64::try_from(job.deadline_ms).unwrap_or(i64::MAX),
                            action_digest: job.action_digest.clone(),
                            grant_ids: job
                                .facts
                                .grants
                                .iter()
                                .map(|g| g.grant.grant_id.clone())
                                .collect(),
                        },
                    );
                    self.outbox.jobs.push(Job::Prove(job));
                }
                Job::Dispatch(order) => self.outbox.jobs.push(Job::Dispatch(order)),
            }
        }
    }

    /// Due work (5.7): idle sessions end, ended `busy` windows are written, then every due
    /// request takes one step, one commit each.
    pub(crate) fn pump(&mut self) -> Result<(), StoreFault> {
        self.end_idle_sessions()?;
        self.flush_busy(false)?;
        for _ in 0..PUMP_STEPS {
            let now = self.now;
            let due = self.store.read(|r| r.due_requests(now, 1))?;
            let Some(record) = due.into_iter().next() else {
                return Ok(());
            };
            self.step(record)?;
        }
        Ok(())
    }

    /// Ends every live session idle at `now` (5.5): one `timer.session_idle` commit each, which
    /// ends its session grants and raises the agent revision.
    fn end_idle_sessions(&mut self) -> Result<(), StoreFault> {
        for (agent_id, session_id) in self.activity.idle(self.now) {
            let meta = self.meta("timer.session_idle", Actor::broker(), None, None);
            self.commit(meta, |tx, cx| {
                let live = tx
                    .read()
                    .live_session(&agent_id)?
                    .is_some_and(|s| s.session_id == session_id);
                if live {
                    crate::service::sessions::end_session(
                        tx,
                        cx,
                        &session_id,
                        fabric_storage::SessionEndReason::IdleTimeout,
                    )?;
                } else {
                    // The store has no such live session: memory only forgets it.
                    cx.after
                        .sessions
                        .push(crate::service::SessionChange::Ended { agent_id });
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    /// Writes the `busy` windows that ended (every open one with `all`, at quit).
    pub(in crate::service) fn flush_busy(&mut self, all: bool) -> Result<(), StoreFault> {
        let entries = self.busy.take(self.now, all);
        if entries.is_empty() {
            return Ok(());
        }
        self.write_audit(entries)
    }

    /// The prover's status changed: a health notice (no ledger row; the write applies no
    /// mutation). Only a repeat of the whole status is dropped, so a change of the problem
    /// alone is one notice.
    pub(crate) fn set_prover_health(&mut self, status: ProverStatus) -> Result<(), StoreFault> {
        if status == self.prover_health {
            return Ok(());
        }
        self.prover_health = status;
        let change = Change::Health(self.health());
        let meta = self.meta("request.advance", Actor::broker(), None, None);
        self.commit(meta, |tx, _| tx.notify(change).map(|_| ()))
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! A core on a test-key store in a temporary directory, for the unit tests of the service.

    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use fabric_storage::{OpenOptions, Store, TestKey};

    use super::{Core, CoreOpenError};
    use crate::broker::{Attachments, Opener, Settings};
    use crate::ports::{ProverIdentity, ProverStatus};
    use fabric_storage::ProverKind;

    /// The test epoch of design 9.
    pub(crate) const T0: u64 = 1_795_190_400_000;

    /// A directory under the system temporary directory, removed on drop.
    pub(crate) struct TempDir(PathBuf);

    impl TempDir {
        pub(crate) fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "fabric-broker-core-unit-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("a temporary directory");
            TempDir(path)
        }

        /// The data directory: a child that does not exist yet.
        pub(crate) fn data(&self) -> PathBuf {
            self.0.join("data")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// An opener on the directory with the public test key; never the Keychain.
    pub(crate) fn opener(dir: &TempDir) -> Opener {
        let data_dir = dir.data();
        Box::new(move |now_ms| {
            Store::open(OpenOptions {
                data_dir: data_dir.clone(),
                key: Box::new(TestKey),
                now_ms,
            })
        })
    }

    /// An Isabelle prover and a provider.
    pub(crate) fn isabelle() -> Attachments {
        Attachments {
            prover: Some(ProverIdentity {
                kind: ProverKind::Isabelle,
                health: ProverStatus::Ready,
            }),
            provider_present: true,
        }
    }

    /// A core opened at `now`.
    pub(crate) fn open(dir: &TempDir, attachments: Attachments, now: u64) -> Core {
        match Core::open(
            &mut opener(dir),
            Settings::cd_w04("test".to_owned()),
            attachments,
            now,
        ) {
            Ok((core, _)) => core,
            Err(CoreOpenError::Open(e)) => panic!("the store does not open: {e}"),
            Err(CoreOpenError::Halt(f)) => panic!("the core halted at open: {f}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use fabric_desktop_contracts::owner::{
        Change, Health, OwnerNotice, ProverHealth, ProverProblem, StoreHealth,
    };
    use fabric_storage::ProverKind;

    use super::testing::{self, T0, TempDir};
    use crate::broker::Attachments;
    use crate::ports::{ProverIdentity, ProverStatus};

    /// The health of each notice a commit sent.
    fn healths(notices: &[OwnerNotice]) -> Vec<Health> {
        notices
            .iter()
            .filter_map(|n| match &n.change {
                Change::Health(health) => Some(*health),
                _ => None,
            })
            .collect()
    }

    /// c32. 10.3: `health()` fills `prover` and `prover_problem` from the status, the problem only while
    /// unavailable with a known cause, and a run with no prover is unavailable, not configured.
    #[test]
    fn health_follows_the_status() {
        let statuses = [
            ProverStatus::Starting,
            ProverStatus::Ready,
            ProverStatus::Unavailable(None),
            ProverStatus::Unavailable(Some(ProverProblem::FailedWillRetry)),
            ProverStatus::Unavailable(Some(ProverProblem::Failed)),
            ProverStatus::Unavailable(Some(ProverProblem::WrongRules)),
        ];
        for status in statuses {
            let dir = TempDir::new();
            let attachments = Attachments {
                prover: Some(ProverIdentity {
                    kind: ProverKind::Isabelle,
                    health: status,
                }),
                provider_present: true,
            };
            let health = testing::open(&dir, attachments, T0).health();
            assert_eq!(health.prover, status.health(), "{status:?}");
            assert_eq!(health.prover_problem, status.problem(), "{status:?}");
            assert_eq!(health.store, StoreHealth::Ready);
            let text = serde_json::to_vec(&health).unwrap();
            let back: Health = fabric_desktop_contracts::decode_record(&text).unwrap();
            assert_eq!(back, health, "{status:?}");
        }
        let dir = TempDir::new();
        let none = Attachments {
            prover: None,
            provider_present: false,
        };
        let health = testing::open(&dir, none, T0).health();
        assert_eq!(health.prover, ProverHealth::Unavailable);
        assert_eq!(health.prover_problem, Some(ProverProblem::NotConfigured));
    }

    /// c32. 10.3: the filter drops only a repeat of the whole status, so a change of the problem alone
    /// is one notice.
    #[test]
    fn a_change_of_cause_alone_sends_one_notice() {
        let dir = TempDir::new();
        let mut core = testing::open(&dir, testing::isabelle(), T0);
        let _ = core.take_outbox();
        let mut sent = Vec::new();
        for status in [
            ProverStatus::Unavailable(Some(ProverProblem::FailedWillRetry)),
            ProverStatus::Unavailable(Some(ProverProblem::Failed)),
            ProverStatus::Unavailable(Some(ProverProblem::Failed)),
            ProverStatus::Starting,
        ] {
            core.set_prover_health(status).unwrap();
            sent.extend(healths(&core.take_outbox().notices));
        }
        let problems: Vec<(ProverHealth, Option<ProverProblem>)> =
            sent.iter().map(|h| (h.prover, h.prover_problem)).collect();
        assert_eq!(
            problems,
            vec![
                (
                    ProverHealth::Unavailable,
                    Some(ProverProblem::FailedWillRetry)
                ),
                (ProverHealth::Unavailable, Some(ProverProblem::Failed)),
                (ProverHealth::Starting, None),
            ]
        );
    }
}
