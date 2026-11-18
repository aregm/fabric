//! Write transactions, their metadata, and the store's failures (design 4, 5.2).

use fabric_desktop_contracts::lifecycle::RequestState;
use fabric_desktop_contracts::owner::{Change, OwnerNotice};
use fabric_desktop_contracts::scalar::{Id, PositiveCount};

use crate::mutation::Mutation;
use crate::read::ReadTx;

/// A failed store operation. Fixed labels, codes and numbers only: no rusqlite or serde error is ever formatted
/// into it, so its Display and Debug never hold key material or row content (D37).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreFault {
    /// `op` names the call site; `code` is SQLite's extended result code (-1 when rusqlite gave none).
    Sql {
        /// The call site.
        op: &'static str,
        /// SQLite's extended result code, or -1.
        code: i32,
    },
    /// A file operation failed.
    Io {
        /// The call site.
        op: &'static str,
        /// The kind of the I/O error.
        kind: std::io::ErrorKind,
    },
    /// The database was busy past the busy timeout.
    Busy,
    /// A rule of the store was broken.
    Invariant {
        /// The rule, as fixed text.
        what: &'static str,
    },
    /// The ledger chain fails at `seq`.
    Chain {
        /// The first ledger row that fails.
        seq: u64,
        /// What fails, as fixed text.
        what: &'static str,
    },
    /// A stored value does not decode.
    Decode {
        /// The table.
        table: &'static str,
        /// The column.
        column: &'static str,
    },
}

impl std::fmt::Display for StoreFault {
    /// Lowercase fixed text.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreFault::Sql { op, code } => write!(f, "store sql failure at {op} (code {code})"),
            StoreFault::Io { op, kind } => write!(f, "store i/o failure at {op} ({kind:?})"),
            StoreFault::Busy => f.write_str("the store is busy"),
            StoreFault::Invariant { what } => write!(f, "store rule broken: {what}"),
            StoreFault::Chain { seq, what } => write!(f, "ledger fails at seq {seq}: {what}"),
            StoreFault::Decode { table, column } => {
                write!(f, "stored value does not decode: {table}.{column}")
            }
        }
    }
}

impl std::error::Error for StoreFault {}

/// The fault of a failed rusqlite call. The error itself is never formatted or kept: its text
/// may hold SQL, and so key material, or row content (D37).
pub(crate) fn sql_fault(op: &'static str, error: &rusqlite::Error) -> StoreFault {
    match error {
        rusqlite::Error::SqliteFailure(failure, _) => match failure.code {
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked => {
                StoreFault::Busy
            }
            _ => StoreFault::Sql {
                op,
                code: failure.extended_code,
            },
        },
        _ => StoreFault::Sql { op, code: -1 },
    }
}

/// A broken store rule.
pub(crate) const fn invariant(what: &'static str) -> StoreFault {
    StoreFault::Invariant { what }
}

/// The failure of a write: the caller's refusal, which rolls back, or a fault of the store.
#[derive(Debug)]
pub enum TxError<E> {
    /// The caller refused; the transaction rolled back.
    Refused(E),
    /// The store failed; the transaction rolled back.
    Fault(StoreFault),
}

impl<E> From<StoreFault> for TxError<E> {
    fn from(f: StoreFault) -> Self {
        TxError::Fault(f)
    }
}

/// Who caused a ledger row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    /// The owner, through the owner channel.
    Owner,
    /// An agent the adapter authenticated.
    Agent,
    /// The broker itself (timers, recovery, the pipeline).
    Broker,
}

/// The actor of a ledger row: its kind, and the agent's id for an agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Actor {
    /// The kind of actor.
    pub kind: ActorKind,
    /// The agent's id; set only for [`ActorKind::Agent`].
    pub agent_id: Option<Id>,
}

impl Actor {
    /// The owner.
    #[must_use]
    pub fn owner() -> Actor {
        Actor {
            kind: ActorKind::Owner,
            agent_id: None,
        }
    }

    /// The broker.
    #[must_use]
    pub fn broker() -> Actor {
        Actor {
            kind: ActorKind::Broker,
            agent_id: None,
        }
    }

    /// The agent `agent_id`.
    #[must_use]
    pub fn agent(agent_id: Id) -> Actor {
        Actor {
            kind: ActorKind::Agent,
            agent_id: Some(agent_id),
        }
    }

    /// The `actor_kind` column.
    pub(crate) const fn kind_text(&self) -> &'static str {
        match self.kind {
            ActorKind::Owner => "owner",
            ActorKind::Agent => "agent",
            ActorKind::Broker => "broker",
        }
    }
}

/// The scope of a ledger row's idempotency key (section 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdempotencyScope {
    /// `owner_command`/`<command id>`.
    OwnerCommand,
    /// `agent_call`/`<agent_id>/<call_id>`.
    AgentCall,
    /// `startup`/`<run id>`.
    Startup,
}

impl IdempotencyScope {
    /// The `idempotency_scope` column.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            IdempotencyScope::OwnerCommand => "owner_command",
            IdempotencyScope::AgentCall => "agent_call",
            IdempotencyScope::Startup => "startup",
        }
    }
}

/// The envelope fields of the ledger row a write appends.
#[derive(Clone, Debug)]
pub struct OpMeta {
    /// The operation type (section 4 list).
    pub operation_type: &'static str,
    /// Who caused the operation.
    pub actor: Actor,
    /// Section 4 rule: only a command (or its refusal), a request insert, and recovery carry one.
    pub idempotency: Option<(IdempotencyScope, String)>,
    /// The cause: the owner command id, the agent call id, or the proof id of the report being
    /// settled; `None` otherwise.
    pub causation_id: Option<String>,
    /// The request the operation belongs to.
    pub correlation_id: Option<Id>,
    /// The core clock's `now` (D34).
    pub now_ms: i64,
}

/// A committed write: the caller's value, the ledger row it appended, and its owner notices.
#[derive(Debug)]
pub struct Committed<T> {
    /// What the write's closure returned.
    pub value: T,
    /// The ledger row appended; `None` when the write applied no mutation.
    pub ledger_seq: Option<u64>,
    /// The notices the write emitted, in sequence order.
    pub notices: Vec<OwnerNotice>,
}

/// One audit row.
#[derive(Clone, Debug)]
pub struct AuditEntry {
    /// The audit kind (fixed text).
    pub kind: &'static str,
    /// Who caused it.
    pub actor: Actor,
    /// The request it concerns.
    pub request_id: Option<Id>,
    /// Never a key, token or event content.
    pub detail: serde_json::Value,
}

/// One write transaction, open inside [`crate::Store::write`].
pub struct WriteTx<'a> {
    pub(crate) tx: rusqlite::Transaction<'a>,
    pub(crate) mutations: Vec<Mutation>,
    pub(crate) notices: Vec<OwnerNotice>,
    pub(crate) now_ms: i64,
    pub(crate) run_id: Option<i64>,
    /// The first fault that may have ended the transaction; once set, the write only fails.
    pub(crate) poisoned: Option<StoreFault>,
}

impl WriteTx<'_> {
    /// Fails once a fault may have ended the transaction: SQLite rolls the whole transaction back on
    /// some errors (FULL, IOERR, NOMEM, CORRUPT, `RAISE(ROLLBACK)`), and a later statement would then
    /// commit on its own in autocommit, outside the write and its ledger row.
    pub(crate) fn usable(&self) -> Result<(), StoreFault> {
        if let Some(fault) = &self.poisoned {
            return Err(fault.clone());
        }
        if self.tx.is_autocommit() {
            return Err(StoreFault::Sql {
                op: "write rolled back",
                code: -1,
            });
        }
        Ok(())
    }

    /// Records `fault` as final unless it is a guard's refusal inside a live transaction.
    fn fail(&mut self, fault: StoreFault) -> StoreFault {
        if !matches!(fault, StoreFault::Invariant { .. }) || self.tx.is_autocommit() {
            self.poisoned.get_or_insert_with(|| fault.clone());
        }
        fault
    }

    /// Sees this transaction's writes.
    #[must_use]
    pub fn read(&self) -> ReadTx<'_> {
        ReadTx::new(&self.tx)
    }

    /// The only projection write; checks the guards of section 3.5 (tested in 9.1).
    ///
    /// # Errors
    ///
    /// [`StoreFault::Invariant`] when a guard refuses, or a fault of the store.
    pub fn apply(&mut self, mutation: Mutation) -> Result<(), StoreFault> {
        self.usable()?;
        if matches!(
            mutation,
            Mutation::StoreCreated { .. } | Mutation::SchemaMigrated { .. }
        ) {
            return Err(invariant(
                "creation and migration are the migration runner's",
            ));
        }
        // Write rules only: replay accepts whatever a verified row holds, rows written before the
        // cause existed included. A move to undecided names its cause, and only that move does.
        if let Mutation::RequestMoved {
            status,
            undecided_cause,
            ..
        } = &mutation
        {
            match (status.state() == RequestState::Undecided, undecided_cause) {
                (true, None) => return Err(invariant("a move to undecided names its cause")),
                (false, Some(_)) => {
                    return Err(invariant("only a move to undecided names its cause"));
                }
                _ => {}
            }
        }
        // A savepoint makes each mutation all or nothing, even when the caller goes on after a
        // refused guard: the projection never holds half of a mutation the ledger lacks.
        let applied = (|| {
            let savepoint = self
                .tx
                .savepoint()
                .map_err(|e| sql_fault("begin mutation", &e))?;
            crate::apply::apply(&savepoint, &mutation)?;
            savepoint
                .commit()
                .map_err(|e| sql_fault("end mutation", &e))
        })();
        if let Err(fault) = applied {
            return Err(self.fail(fault));
        }
        if let Mutation::RunStarted { run_id, .. } = mutation {
            self.run_id = Some(run_id);
        }
        self.mutations.push(mutation);
        Ok(())
    }

    /// Assigns `meta.notice_sequence + 1`; returned in `Committed::notices` in order.
    ///
    /// # Errors
    ///
    /// A fault of the store.
    pub fn notify(&mut self, change: Change) -> Result<PositiveCount, StoreFault> {
        self.usable()?;
        let next = meta_count(&self.tx, "notice_sequence").map_err(|fault| self.fail(fault))? + 1;
        let sequence =
            PositiveCount::new(next).map_err(|_| invariant("notice sequence out of range"))?;
        set_meta(&self.tx, "notice_sequence", &next.to_string())
            .map_err(|fault| self.fail(fault))?;
        self.notices.push(OwnerNotice { sequence, change });
        Ok(sequence)
    }

    /// Adds one audit row to this transaction.
    ///
    /// # Errors
    ///
    /// A fault of the store.
    pub fn audit(&mut self, entry: AuditEntry) -> Result<(), StoreFault> {
        self.usable()?;
        insert_audit(&self.tx, self.now_ms, self.run_id, &entry).map_err(|fault| self.fail(fault))
    }

    /// The write's `now` (`OpMeta::now_ms`).
    #[must_use]
    pub fn now_ms(&self) -> i64 {
        self.now_ms
    }

    /// None before `RunStarted` is applied in this store's life.
    #[must_use]
    pub fn run_id(&self) -> Option<i64> {
        self.run_id
    }
}

/// Appends one audit row. The audit table is not chained and never replayed (D24).
pub(crate) fn insert_audit(
    connection: &rusqlite::Connection,
    at_ms: i64,
    run_id: Option<i64>,
    entry: &AuditEntry,
) -> Result<(), StoreFault> {
    let detail = fabric_desktop_contracts::canonical(&entry.detail);
    connection
        .execute(
            "INSERT INTO audit (at_ms, run_id, actor_kind, actor_agent_id, kind, request_id, detail) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                at_ms,
                run_id,
                entry.actor.kind_text(),
                entry.actor.agent_id.as_ref().map(Id::as_str),
                entry.kind,
                entry.request_id.as_ref().map(Id::as_str),
                detail,
            ],
        )
        .map_err(|e| sql_fault("insert audit", &e))?;
    Ok(())
}

/// One value of the `meta` table, if present.
pub(crate) fn meta_value(
    connection: &rusqlite::Connection,
    key: &str,
) -> Result<Option<String>, StoreFault> {
    use rusqlite::OptionalExtension as _;
    connection
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
            row.get::<_, String>(0)
        })
        .optional()
        .map_err(|e| sql_fault("read meta", &e))
}

/// A count in the `meta` table; 0 when absent.
pub(crate) fn meta_count(connection: &rusqlite::Connection, key: &str) -> Result<u64, StoreFault> {
    match meta_value(connection, key)? {
        None => Ok(0),
        Some(text) => text.parse().map_err(|_| StoreFault::Decode {
            table: "meta",
            column: "value",
        }),
    }
}

/// Sets one value of the `meta` table.
pub(crate) fn set_meta(
    connection: &rusqlite::Connection,
    key: &str,
    value: &str,
) -> Result<(), StoreFault> {
    connection
        .execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2) \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [key, value],
        )
        .map_err(|e| sql_fault("write meta", &e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use fabric_desktop_contracts::owner::Change;

    use super::*;
    use crate::rows::UndecidedCause;
    use crate::testing::{self, T0, TempDir};

    fn ledger_rows(store: &crate::Store) -> u64 {
        store.read(|r| r.meta()).unwrap().ledger_head_seq
    }

    #[test]
    fn a_write_appends_one_ledger_row_and_a_refused_write_appends_none() {
        let dir = TempDir::new();
        let mut store = testing::store(&dir);
        let committed = store
            .write(testing::meta("broker.started", T0), |tx| {
                tx.apply(Mutation::RunStarted {
                    run_id: 1,
                    at_ms: T0,
                })?;
                tx.apply(Mutation::AgentPut {
                    agent: testing::agent("agent-1", 1),
                })?;
                Ok::<_, TxError<()>>(7)
            })
            .unwrap();
        assert_eq!((committed.value, committed.ledger_seq), (7, Some(2)));
        let refused = store.write(testing::meta("owner.refused", T0), |tx| {
            tx.apply(Mutation::AgentPut {
                agent: testing::agent("agent-2", 1),
            })?;
            Err::<(), _>(TxError::Refused("no"))
        });
        assert!(matches!(refused, Err(TxError::Refused("no"))));
        assert_eq!(ledger_rows(&store), 2);
        assert!(
            store
                .read(|r| r.agent(&testing::id("agent-2")))
                .unwrap()
                .is_none()
        );
        store.verify_chain().unwrap();
    }

    #[test]
    fn a_write_without_a_mutation_appends_no_ledger_row() {
        let dir = TempDir::new();
        let mut store = testing::store(&dir);
        let committed = store
            .write(
                testing::meta("agent.call", T0),
                |_| Ok::<_, TxError<()>>(()),
            )
            .unwrap();
        assert_eq!(committed.ledger_seq, None);
        assert_eq!(ledger_rows(&store), 1);
    }

    #[test]
    fn an_audit_only_write_appends_no_ledger_row() {
        let dir = TempDir::new();
        let mut store = testing::store(&dir);
        let entry = AuditEntry {
            kind: "agent_busy",
            actor: Actor::agent(testing::id("agent-1")),
            request_id: None,
            detail: serde_json::json!({ "count": 3 }),
        };
        store.write_audit(T0, vec![entry.clone(), entry]).unwrap();
        assert_eq!(ledger_rows(&store), 1);
        let (audits, kind, detail): (i64, String, String) = store
            .connection()
            .query_row(
                "SELECT count(*), max(kind), max(detail) FROM audit",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (audits, kind.as_str(), detail.as_str()),
            (2, "agent_busy", r#"{"count":3}"#)
        );
    }

    #[test]
    fn notices_take_the_next_sequences_and_a_refusal_takes_none() {
        let dir = TempDir::new();
        let mut store = testing::store(&dir);
        let change = || Change::RecipientSetDeleted(testing::id("set-1"));
        let committed = store
            .write(testing::meta("owner.save_group", T0), |tx| {
                tx.notify(change())?;
                tx.notify(change())?;
                Ok::<_, TxError<()>>(())
            })
            .unwrap();
        let sequences: Vec<u64> = committed.notices.iter().map(|n| n.sequence.get()).collect();
        assert_eq!(sequences, vec![1, 2]);
        let _ = store.write(testing::meta("owner.save_group", T0), |tx| {
            tx.notify(change())?;
            Err::<(), _>(TxError::Refused(()))
        });
        let committed = store
            .write(testing::meta("owner.save_group", T0), |tx| {
                tx.notify(change()).map_err(TxError::<()>::from)
            })
            .unwrap();
        assert_eq!(committed.value.get(), 3);
        assert_eq!(store.read(|r| r.meta()).unwrap().notice_sequence, 3);
    }

    #[test]
    fn a_write_dated_before_the_last_commit_is_refused() {
        let dir = TempDir::new();
        let mut store = testing::store(&dir);
        let result = testing::apply_all(
            &mut store,
            T0 - 1,
            vec![Mutation::RunStarted {
                run_id: 1,
                at_ms: T0 - 1,
            }],
        );
        assert_eq!(testing::refused_with(result), "ledger time never goes back");
        assert_eq!(ledger_rows(&store), 1);
    }

    #[test]
    fn an_unknown_operation_type_is_refused() {
        let dir = TempDir::new();
        let mut store = testing::store(&dir);
        let result = store.write(testing::meta("owner.dance", T0), |_| {
            Ok::<_, TxError<()>>(())
        });
        assert!(matches!(
            result,
            Err(TxError::Fault(StoreFault::Invariant { .. }))
        ));
    }

    #[test]
    fn creation_and_migration_mutations_are_the_runner_s() {
        let dir = TempDir::new();
        let mut store = testing::store(&dir);
        let result = testing::apply_all(
            &mut store,
            T0,
            vec![Mutation::SchemaMigrated { from: 1, to: 2 }],
        );
        assert_eq!(
            testing::refused_with(result),
            "creation and migration are the migration runner's"
        );
    }

    /// s20. Section 4: a move to undecided names its cause and only that move does; the move is applied
    /// with it. Replay of an old move to undecided without a cause is
    /// `ledger::tests::a_store_with_causes_replays`.
    #[test]
    fn only_a_move_to_undecided_names_its_cause() {
        let dir = TempDir::new();
        let mut store = testing::world(&dir);
        let moved = |revision, status, cause| Mutation::RequestMoved {
            request_id: testing::id("request-1"),
            revision,
            status,
            at_ms: T0,
            due_at_ms: None,
            undecided_cause: cause,
        };
        testing::apply_all(
            &mut store,
            T0,
            vec![
                moved(2, testing::status("validating"), None),
                moved(3, testing::status("proving_classification"), None),
            ],
        )
        .unwrap();
        let mut write = |mutation| {
            store.write(testing::meta_of("request.advance", T0, "request-1"), |tx| {
                tx.apply(mutation)?;
                Ok::<_, TxError<()>>(())
            })
        };
        let blocked = testing::from(serde_json::json!({
            "state": "blocked",
            "denial": { "rule": "recurring_mutation_unsupported", "explanation": "No.",
                        "proof_id": "proof-1" },
        }));
        assert_eq!(
            testing::refused_with(write(moved(
                4,
                blocked,
                Some(UndecidedCause::ReportUnusable)
            ))),
            "only a move to undecided names its cause"
        );
        let undecided = testing::status("undecided");
        assert_eq!(
            testing::refused_with(write(moved(4, undecided.clone(), None))),
            "a move to undecided names its cause"
        );
        write(moved(4, undecided, Some(UndecidedCause::NoRuling))).unwrap();
        assert_eq!(
            store
                .read(|r| r.undecided_cause_of(&testing::id("request-1")))
                .unwrap(),
            Some(UndecidedCause::NoRuling)
        );
        store.verify_chain().unwrap();
        store.verify_replay().unwrap();
    }

    #[test]
    fn the_run_of_a_write_is_the_run_it_starts() {
        let dir = TempDir::new();
        let mut store = testing::store(&dir);
        let run = store
            .write(testing::meta("broker.started", T0), |tx| {
                assert_eq!(tx.run_id(), None);
                assert_eq!(tx.now_ms(), T0);
                tx.apply(Mutation::RunStarted {
                    run_id: 1,
                    at_ms: T0,
                })?;
                Ok::<_, TxError<()>>(tx.run_id())
            })
            .unwrap();
        assert_eq!(run.value, Some(1));
        let row_run: Option<i64> = store
            .connection()
            .query_row("SELECT run_id FROM ledger WHERE seq = 2", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(row_run, Some(1));
        let migration_run: Option<i64> = store
            .connection()
            .query_row("SELECT run_id FROM ledger WHERE seq = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(migration_run, None);
    }

    #[test]
    fn a_write_that_goes_on_after_a_refused_mutation_commits_only_the_rest() {
        let dir = TempDir::new();
        let mut store = testing::world(&dir);
        let committed = store
            .write(testing::meta("request.dispatch", T0), |tx| {
                tx.apply(Mutation::EffectReserved {
                    effect: testing::effect(
                        "effect-1",
                        "request-1",
                        "fabric_calendar_propose_create",
                    ),
                })?;
                let refused = tx.apply(Mutation::DispatchIntent {
                    attempt: testing::attempt("effect-1", "auth-unknown", 1, T0),
                });
                assert!(refused.is_err());
                Ok::<_, TxError<()>>(())
            })
            .unwrap();
        assert_eq!(committed.ledger_seq, Some(3));
        let effect = store
            .read(|r| r.effect_of(&testing::id("request-1")))
            .unwrap()
            .unwrap();
        assert_eq!(effect.attempts_started, 0);
        store.verify_replay().unwrap();
    }

    #[test]
    fn a_fault_that_ends_the_transaction_fails_the_whole_write_even_when_the_caller_goes_on() {
        let dir = TempDir::new();
        let mut store = testing::store(&dir);
        // RAISE(ROLLBACK) ends the whole transaction, as SQLITE_FULL, IOERR or NOMEM may.
        store
            .connection()
            .execute_batch(
                "CREATE TRIGGER inject AFTER INSERT ON agents WHEN new.agent_id = 'agent-b' \
                 BEGIN SELECT RAISE(ROLLBACK, 'injected'); END;",
            )
            .unwrap();
        let mut later = Vec::new();
        let result = store.write(testing::meta("broker.started", T0), |tx| {
            tx.apply(Mutation::AgentPut {
                agent: testing::agent("agent-a", 1),
            })?;
            let ended = tx.apply(Mutation::AgentPut {
                agent: testing::agent("agent-b", 1),
            });
            assert!(matches!(ended, Err(StoreFault::Sql { .. })));
            later.push(tx.apply(Mutation::AgentPut {
                agent: testing::agent("agent-c", 1),
            }));
            later.push(
                tx.notify(Change::RecipientSetDeleted(testing::id("set-1")))
                    .map(|_| ()),
            );
            later.push(tx.audit(AuditEntry {
                kind: "agent_busy",
                actor: Actor::broker(),
                request_id: None,
                detail: serde_json::json!({}),
            }));
            Ok::<_, TxError<()>>(())
        });
        assert!(
            later
                .iter()
                .all(|r| matches!(r, Err(StoreFault::Sql { .. })))
        );
        assert!(matches!(
            result,
            Err(TxError::Fault(StoreFault::Sql { .. }))
        ));
        let (agents, audits): (i64, i64) = store
            .connection()
            .query_row(
                "SELECT (SELECT count(*) FROM agents), (SELECT count(*) FROM audit)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((agents, audits), (0, 0));
        let meta = store.read(|r| r.meta()).unwrap();
        assert_eq!((meta.ledger_head_seq, meta.notice_sequence), (1, 0));
        store.verify_chain().unwrap();
        store.verify_replay().unwrap();
    }
}
