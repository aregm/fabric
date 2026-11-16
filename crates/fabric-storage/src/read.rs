//! One consistent read of the store: one `ReadTx` is one view.

use fabric_desktop_contracts::agent::{Action, ResponseTarget};
use fabric_desktop_contracts::owner::{
    Account, BuiltInProfile, CustomProfile, Grant, Preferences, PreferencesSettings, Profile,
    ProfileRows, Profiles, RecipientSet,
};
use fabric_desktop_contracts::scalar::{Count, Id, List};
use rusqlite::OptionalExtension as _;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::key::KeyKind;
use crate::mutation::SessionEndReason;
use crate::rows::{
    AgentRow, AttemptRow, AuthorizationRecord, AuthorizationState, CommandRow, Counters, EffectRow,
    Fence, Meta, OpenEffect, ProofRecord, ProverKind, Reproofs, RequestRecord, RequestRow,
    SessionRow,
};
use crate::tx::{StoreFault, invariant, meta_count, meta_value, sql_fault};

/// A read inside a transaction: [`crate::Store::read`] or [`crate::WriteTx::read`].
pub struct ReadTx<'a> {
    connection: &'a rusqlite::Connection,
}

/// The canonical JSON text of a stored value.
pub(crate) fn to_text<T: Serialize + ?Sized>(value: &T) -> String {
    fabric_desktop_contracts::encode(value)
}

/// A stored JSON value. The serde error is never formatted (D37).
pub(crate) fn from_text<T: DeserializeOwned>(
    text: &str,
    table: &'static str,
    column: &'static str,
) -> Result<T, StoreFault> {
    serde_json::from_str(text).map_err(|_| StoreFault::Decode { table, column })
}

/// The snake_case name serde gives a unit variant (a state, a reason, a kind).
pub(crate) fn name_of<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(text)) => text,
        _ => String::new(),
    }
}

/// A stored id.
fn id_of(text: String, table: &'static str, column: &'static str) -> Result<Id, StoreFault> {
    Id::new(text).map_err(|_| StoreFault::Decode { table, column })
}

/// A stored count, never negative.
fn count_of(value: i64, table: &'static str, column: &'static str) -> Result<u64, StoreFault> {
    u64::try_from(value).map_err(|_| StoreFault::Decode { table, column })
}

/// A stored small count, never negative.
fn small_count_of(
    value: i64,
    table: &'static str,
    column: &'static str,
) -> Result<u32, StoreFault> {
    u32::try_from(value).map_err(|_| StoreFault::Decode { table, column })
}

const REQUEST_COLUMNS: &str = "record, action, scope, session_id, call_id, card, \
     card_expires_at_ms, approval, relied_grant_ids, reproofs_classification, \
     reproofs_fresh_permit, due_at_ms";

/// The columns of one `requests` row, before decoding.
struct RawRequest {
    record: String,
    action: String,
    scope: Option<String>,
    session_id: Option<String>,
    call_id: String,
    card: Option<String>,
    card_expires_at_ms: Option<i64>,
    approval: Option<String>,
    relied: String,
    reproofs_classification: i64,
    reproofs_fresh_permit: i64,
    due_at_ms: Option<i64>,
}

impl RawRequest {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawRequest> {
        Ok(RawRequest {
            record: row.get(0)?,
            action: row.get(1)?,
            scope: row.get(2)?,
            session_id: row.get(3)?,
            call_id: row.get(4)?,
            card: row.get(5)?,
            card_expires_at_ms: row.get(6)?,
            approval: row.get(7)?,
            relied: row.get(8)?,
            reproofs_classification: row.get(9)?,
            reproofs_fresh_permit: row.get(10)?,
            due_at_ms: row.get(11)?,
        })
    }

    fn decode(self) -> Result<RequestRecord, StoreFault> {
        const T: &str = "requests";
        Ok(RequestRecord {
            row: RequestRow {
                request: from_text(&self.record, T, "record")?,
                action: from_text::<Action>(&self.action, T, "action")?,
                scope: self.scope.map(|s| from_text(&s, T, "scope")).transpose()?,
                session_id: self
                    .session_id
                    .map(|s| id_of(s, T, "session_id"))
                    .transpose()?,
                call_id: id_of(self.call_id, T, "call_id")?,
            },
            card: self.card.map(|s| from_text(&s, T, "card")).transpose()?,
            card_expires_at_ms: self.card_expires_at_ms,
            approval: self
                .approval
                .map(|s| from_text(&s, T, "approval"))
                .transpose()?,
            relied_grant_ids: from_text(&self.relied, T, "relied_grant_ids")?,
            reproofs: Reproofs {
                classification: small_count_of(
                    self.reproofs_classification,
                    T,
                    "reproofs_classification",
                )?,
                fresh_permit: small_count_of(
                    self.reproofs_fresh_permit,
                    T,
                    "reproofs_fresh_permit",
                )?,
            },
            due_at_ms: self.due_at_ms,
        })
    }
}

const SESSION_COLUMNS: &str =
    "session_id, agent_id, run_id, started_at_ms, ended_at_ms, end_reason";

type RawSession = (String, String, i64, i64, Option<i64>, Option<String>);

fn raw_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawSession> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
    ))
}

fn decode_session(raw: RawSession) -> Result<SessionRow, StoreFault> {
    const T: &str = "sessions";
    let (session_id, agent_id, run_id, started_at_ms, ended_at_ms, end_reason) = raw;
    let ended = match (ended_at_ms, end_reason) {
        (None, None) => None,
        (Some(at), Some(reason)) => Some((
            at,
            from_text::<SessionEndReason>(&format!("\"{reason}\""), T, "end_reason")?,
        )),
        _ => {
            return Err(StoreFault::Decode {
                table: T,
                column: "end_reason",
            });
        }
    };
    Ok(SessionRow {
        session_id: id_of(session_id, T, "session_id")?,
        agent_id: id_of(agent_id, T, "agent_id")?,
        run_id,
        started_at_ms,
        ended,
    })
}

const AUTHORIZATION_COLUMNS: &str = "record, grant_ids, attempt, run_id, fence, state";

type RawAuthorization = (String, String, i64, i64, String, String);

fn raw_authorization(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawAuthorization> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
    ))
}

fn decode_authorization(raw: RawAuthorization) -> Result<AuthorizationRecord, StoreFault> {
    const T: &str = "authorizations";
    let (record, grant_ids, attempt, run_id, fence, state) = raw;
    Ok(AuthorizationRecord {
        authorization: from_text(&record, T, "record")?,
        grant_ids: from_text(&grant_ids, T, "grant_ids")?,
        attempt: small_count_of(attempt, T, "attempt")?,
        run_id,
        fence: from_text::<Fence>(&fence, T, "fence")?,
        state: from_text::<AuthorizationState>(&format!("\"{state}\""), T, "state")?,
    })
}

const EFFECT_COLUMNS: &str = "effect_id, request_id, tool, account_id, calendar_id, \
     action_digest, provider_event_id, created_at_ms, attempts_started, first_attempt_at_ms";

/// The columns of one `effects` row, before decoding.
struct RawEffect {
    effect_id: String,
    request_id: String,
    tool: String,
    account_id: String,
    calendar_id: String,
    action_digest: String,
    provider_event_id: Option<String>,
    created_at_ms: i64,
    attempts_started: i64,
    first_attempt_at_ms: Option<i64>,
}

impl RawEffect {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawEffect> {
        Ok(RawEffect {
            effect_id: row.get(0)?,
            request_id: row.get(1)?,
            tool: row.get(2)?,
            account_id: row.get(3)?,
            calendar_id: row.get(4)?,
            action_digest: row.get(5)?,
            provider_event_id: row.get(6)?,
            created_at_ms: row.get(7)?,
            attempts_started: row.get(8)?,
            first_attempt_at_ms: row.get(9)?,
        })
    }

    fn decode(self) -> Result<EffectRow, StoreFault> {
        const T: &str = "effects";
        Ok(EffectRow {
            effect_id: id_of(self.effect_id, T, "effect_id")?,
            request_id: id_of(self.request_id, T, "request_id")?,
            tool: from_text(&format!("\"{}\"", self.tool), T, "tool")?,
            account_id: id_of(self.account_id, T, "account_id")?,
            calendar_id: id_of(self.calendar_id, T, "calendar_id")?,
            action_digest: fabric_desktop_contracts::scalar::Digest::new(self.action_digest)
                .map_err(|_| StoreFault::Decode {
                    table: T,
                    column: "action_digest",
                })?,
            provider_event_id: self.provider_event_id,
            created_at_ms: self.created_at_ms,
            attempts_started: small_count_of(self.attempts_started, T, "attempts_started")?,
            first_attempt_at_ms: self.first_attempt_at_ms,
        })
    }
}

impl<'a> ReadTx<'a> {
    /// A read on a connection that is inside a transaction.
    pub(crate) fn new(connection: &'a rusqlite::Connection) -> ReadTx<'a> {
        ReadTx { connection }
    }

    /// All rows of a query, each mapped.
    fn all<T, P: rusqlite::Params>(
        &self,
        op: &'static str,
        sql: &str,
        params: P,
        map: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Vec<T>, StoreFault> {
        let mut statement = self
            .connection
            .prepare_cached(sql)
            .map_err(|e| sql_fault(op, &e))?;
        let rows = statement
            .query_map(params, map)
            .map_err(|e| sql_fault(op, &e))?;
        rows.collect::<rusqlite::Result<Vec<T>>>()
            .map_err(|e| sql_fault(op, &e))
    }

    /// The one row of a query, if any.
    fn one<T, P: rusqlite::Params>(
        &self,
        op: &'static str,
        sql: &str,
        params: P,
        map: impl FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Option<T>, StoreFault> {
        let mut statement = self
            .connection
            .prepare_cached(sql)
            .map_err(|e| sql_fault(op, &e))?;
        statement
            .query_row(params, map)
            .optional()
            .map_err(|e| sql_fault(op, &e))
    }

    /// The `meta` table.
    pub fn meta(&self) -> Result<Meta, StoreFault> {
        let text = |key: &'static str| -> Result<String, StoreFault> {
            meta_value(self.connection, key)?.ok_or(StoreFault::Decode {
                table: "meta",
                column: key,
            })
        };
        let key_kind = match text("key_kind")?.as_str() {
            "keychain" => KeyKind::Keychain,
            "test" => KeyKind::Test,
            _ => {
                return Err(StoreFault::Decode {
                    table: "meta",
                    column: "key_kind",
                });
            }
        };
        let schema_version = u32::try_from(meta_count(self.connection, "schema_version")?)
            .map_err(|_| StoreFault::Decode {
                table: "meta",
                column: "schema_version",
            })?;
        Ok(Meta {
            store_id: text("store_id")?,
            key_kind,
            schema_version,
            notice_sequence: meta_count(self.connection, "notice_sequence")?,
            ledger_head_seq: meta_count(self.connection, "ledger_head_seq")?,
            ledger_head_hash: meta_value(self.connection, "ledger_head_hash")?
                .unwrap_or_else(|| crate::ledger::GENESIS_HASH.to_owned()),
        })
    }

    /// The two epochs.
    pub fn counters(&self) -> Result<Counters, StoreFault> {
        let value = |name: &'static str| -> Result<u64, StoreFault> {
            let value = self
                .one(
                    "read counters",
                    "SELECT value FROM counters WHERE name = ?1",
                    [name],
                    |row| row.get::<_, i64>(0),
                )?
                .ok_or(StoreFault::Decode {
                    table: "counters",
                    column: name,
                })?;
            count_of(value, "counters", "value")
        };
        Ok(Counters {
            pause_epoch: value("pause_epoch")?,
            revocation_epoch: value("revocation_epoch")?,
        })
    }

    /// The run started and not ended, if any.
    pub fn current_run(&self) -> Result<Option<i64>, StoreFault> {
        let last = self.one(
            "read runs",
            "SELECT run_id, ended_at_ms FROM runs ORDER BY run_id DESC LIMIT 1",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?)),
        )?;
        Ok(match last {
            Some((run_id, None)) => Some(run_id),
            _ => None,
        })
    }

    /// The largest run id ever started, if any.
    pub(crate) fn last_run(&self) -> Result<Option<i64>, StoreFault> {
        Ok(self
            .one("read runs", "SELECT max(run_id) FROM runs", [], |row| {
                row.get::<_, Option<i64>>(0)
            })?
            .flatten())
    }

    /// The `committed_at_ms` of the ledger's last row.
    pub fn last_committed_ms(&self) -> Result<Option<i64>, StoreFault> {
        self.one(
            "read ledger",
            "SELECT committed_at_ms FROM ledger ORDER BY seq DESC LIMIT 1",
            [],
            |row| row.get::<_, i64>(0),
        )
    }

    /// Built-ins + latest custom versions.
    pub fn profiles(&self) -> Result<Profiles, StoreFault> {
        let (active_profile_id, _, revision) = self.profile_state()?;
        let mut items: Vec<Profile> = BuiltInProfile::ALL
            .into_iter()
            .map(Profile::BuiltIn)
            .collect();
        let custom = self.all(
            "read profiles",
            "SELECT v.record FROM profile_versions v \
             WHERE v.version = (SELECT max(w.version) FROM profile_versions w \
                                WHERE w.profile_id = v.profile_id) \
             ORDER BY (SELECT min(w.created_at_ms) FROM profile_versions w \
                       WHERE w.profile_id = v.profile_id), v.profile_id",
            [],
            |row| row.get::<_, String>(0),
        )?;
        for record in custom {
            items.push(Profile::Custom(from_text(
                &record,
                "profile_versions",
                "record",
            )?));
        }
        Ok(Profiles {
            revision: Count::new(revision).map_err(|_| StoreFault::Decode {
                table: "profile_state",
                column: "revision",
            })?,
            active_profile_id,
            items: List::new(items).map_err(|_| StoreFault::Decode {
                table: "profile_versions",
                column: "record",
            })?,
        })
    }

    /// The `profile_state` row: active id, version and the profiles revision.
    fn profile_state(&self) -> Result<(Id, u64, u64), StoreFault> {
        const T: &str = "profile_state";
        let (id, version, revision) = self
            .one(
                "read profile state",
                "SELECT active_profile_id, active_version, revision FROM profile_state WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )?
            .ok_or(StoreFault::Decode {
                table: T,
                column: "id",
            })?;
        Ok((
            id_of(id, T, "active_profile_id")?,
            count_of(version, T, "active_version")?,
            count_of(revision, T, "revision")?,
        ))
    }

    /// The active profile's id, version and rows.
    pub fn active_profile(&self) -> Result<(Id, u64, ProfileRows), StoreFault> {
        let (id, version, _) = self.profile_state()?;
        if let Some(built_in) = BuiltInProfile::ALL
            .into_iter()
            .find(|b| b.profile_id() == id.as_str())
        {
            return Ok((id, version, built_in.rows()));
        }
        let record = self
            .one(
                "read profiles",
                "SELECT record FROM profile_versions WHERE profile_id = ?1 AND version = ?2",
                rusqlite::params![id.as_str(), i64::try_from(version).unwrap_or(i64::MAX)],
                |row| row.get::<_, String>(0),
            )?
            .ok_or(invariant("the active profile does not exist"))?;
        let profile: CustomProfile = from_text(&record, "profile_versions", "record")?;
        Ok((id, version, profile.rows))
    }

    /// Latest version.
    pub fn custom_profile(&self, profile_id: &Id) -> Result<Option<CustomProfile>, StoreFault> {
        self.one(
            "read profiles",
            "SELECT record FROM profile_versions WHERE profile_id = ?1 \
             ORDER BY version DESC LIMIT 1",
            [profile_id.as_str()],
            |row| row.get::<_, String>(0),
        )?
        .map(|record| from_text(&record, "profile_versions", "record"))
        .transpose()
    }

    /// The owner's preferences.
    pub fn preferences(&self) -> Result<Preferences, StoreFault> {
        const T: &str = "preferences";
        let (revision, settings) = self
            .one(
                "read preferences",
                "SELECT revision, settings FROM preferences WHERE id = 1",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )?
            .ok_or(StoreFault::Decode {
                table: T,
                column: "id",
            })?;
        Ok(Preferences {
            revision: Count::new(count_of(revision, T, "revision")?).map_err(|_| {
                StoreFault::Decode {
                    table: T,
                    column: "revision",
                }
            })?,
            settings: from_text::<PreferencesSettings>(&settings, T, "settings")?,
        })
    }

    /// Current, not deleted.
    pub fn recipient_sets(&self) -> Result<Vec<RecipientSet>, StoreFault> {
        self.all(
            "read recipient sets",
            "SELECT v.record FROM recipient_sets s JOIN recipient_set_versions v \
             ON v.recipient_set_id = s.recipient_set_id AND v.revision = s.revision \
             WHERE s.deleted_at_ms IS NULL ORDER BY s.recipient_set_id",
            [],
            |row| row.get::<_, String>(0),
        )?
        .iter()
        .map(|record| from_text(record, "recipient_set_versions", "record"))
        .collect()
    }

    /// The current version of one recipient set, unless deleted.
    pub fn recipient_set(&self, id: &Id) -> Result<Option<RecipientSet>, StoreFault> {
        self.one(
            "read recipient sets",
            "SELECT v.record FROM recipient_sets s JOIN recipient_set_versions v \
             ON v.recipient_set_id = s.recipient_set_id AND v.revision = s.revision \
             WHERE s.recipient_set_id = ?1 AND s.deleted_at_ms IS NULL",
            [id.as_str()],
            |row| row.get::<_, String>(0),
        )?
        .map(|record| from_text(&record, "recipient_set_versions", "record"))
        .transpose()
    }

    /// Every account.
    pub fn accounts(&self) -> Result<Vec<Account>, StoreFault> {
        self.all(
            "read accounts",
            "SELECT record FROM accounts ORDER BY account_id",
            [],
            |row| row.get::<_, String>(0),
        )?
        .iter()
        .map(|record| from_text(record, "accounts", "record"))
        .collect()
    }

    /// One account.
    pub fn account(&self, id: &Id) -> Result<Option<Account>, StoreFault> {
        self.one(
            "read accounts",
            "SELECT record FROM accounts WHERE account_id = ?1",
            [id.as_str()],
            |row| row.get::<_, String>(0),
        )?
        .map(|record| from_text(&record, "accounts", "record"))
        .transpose()
    }

    /// Every agent.
    pub fn agents(&self) -> Result<Vec<AgentRow>, StoreFault> {
        self.all(
            "read agents",
            "SELECT record FROM agents ORDER BY enrolled_at_ms, agent_id",
            [],
            |row| row.get::<_, String>(0),
        )?
        .iter()
        .map(|record| from_text(record, "agents", "record"))
        .collect()
    }

    /// One agent.
    pub fn agent(&self, id: &Id) -> Result<Option<AgentRow>, StoreFault> {
        self.one(
            "read agents",
            "SELECT record FROM agents WHERE agent_id = ?1",
            [id.as_str()],
            |row| row.get::<_, String>(0),
        )?
        .map(|record| from_text(&record, "agents", "record"))
        .transpose()
    }

    /// One session, live or ended.
    pub fn session(&self, id: &Id) -> Result<Option<SessionRow>, StoreFault> {
        self.one(
            "read sessions",
            &format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE session_id = ?1"),
            [id.as_str()],
            raw_session,
        )?
        .map(decode_session)
        .transpose()
    }

    /// The agent's session that has not ended, if any.
    pub fn live_session(&self, agent_id: &Id) -> Result<Option<SessionRow>, StoreFault> {
        self.one(
            "read sessions",
            &format!(
                "SELECT {SESSION_COLUMNS} FROM sessions \
                 WHERE agent_id = ?1 AND ended_at_ms IS NULL"
            ),
            [agent_id.as_str()],
            raw_session,
        )?
        .map(decode_session)
        .transpose()
    }

    /// Every session that has not ended.
    pub fn live_sessions(&self) -> Result<Vec<SessionRow>, StoreFault> {
        self.all(
            "read sessions",
            &format!(
                "SELECT {SESSION_COLUMNS} FROM sessions WHERE ended_at_ms IS NULL \
                 ORDER BY started_at_ms, session_id"
            ),
            [],
            raw_session,
        )?
        .into_iter()
        .map(decode_session)
        .collect()
    }

    /// One request.
    pub fn request(&self, id: &Id) -> Result<Option<RequestRecord>, StoreFault> {
        self.one(
            "read requests",
            &format!("SELECT {REQUEST_COLUMNS} FROM requests WHERE request_id = ?1"),
            [id.as_str()],
            RawRequest::from_row,
        )?
        .map(RawRequest::decode)
        .transpose()
    }

    /// The request an agent's call created, if any.
    pub fn request_by_call(
        &self,
        agent_id: &Id,
        call_id: &Id,
    ) -> Result<Option<RequestRecord>, StoreFault> {
        self.one(
            "read requests",
            &format!("SELECT {REQUEST_COLUMNS} FROM requests WHERE agent_id = ?1 AND call_id = ?2"),
            [agent_id.as_str(), call_id.as_str()],
            RawRequest::from_row,
        )?
        .map(RawRequest::decode)
        .transpose()
    }

    /// Every request not in a terminal state.
    pub fn open_requests(&self) -> Result<Vec<RequestRecord>, StoreFault> {
        self.all(
            "read requests",
            &format!(
                "SELECT {REQUEST_COLUMNS} FROM requests WHERE terminal = 0 \
                 ORDER BY received_at_ms, request_id"
            ),
            [],
            RawRequest::from_row,
        )?
        .into_iter()
        .map(RawRequest::decode)
        .collect()
    }

    /// Requests due at or before `until_ms`, earliest first, at most `limit`.
    pub fn due_requests(
        &self,
        until_ms: i64,
        limit: usize,
    ) -> Result<Vec<RequestRecord>, StoreFault> {
        self.all(
            "read requests",
            &format!(
                "SELECT {REQUEST_COLUMNS} FROM requests \
                 WHERE due_at_ms IS NOT NULL AND due_at_ms <= ?1 \
                 ORDER BY due_at_ms, received_at_ms, request_id LIMIT ?2"
            ),
            rusqlite::params![until_ms, i64::try_from(limit).unwrap_or(i64::MAX)],
            RawRequest::from_row,
        )?
        .into_iter()
        .map(RawRequest::decode)
        .collect()
    }

    /// Open requests, of one agent or of all.
    pub fn count_open(&self, agent_id: Option<&Id>) -> Result<u64, StoreFault> {
        let count = match agent_id {
            Some(agent_id) => self.one(
                "count requests",
                "SELECT count(*) FROM requests WHERE terminal = 0 AND agent_id = ?1",
                [agent_id.as_str()],
                |row| row.get::<_, i64>(0),
            )?,
            None => self.one(
                "count requests",
                "SELECT count(*) FROM requests WHERE terminal = 0",
                [],
                |row| row.get::<_, i64>(0),
            )?,
        };
        count_of(count.unwrap_or(0), "requests", "terminal")
    }

    /// Open approval cards of one agent.
    pub fn count_cards(&self, agent_id: &Id) -> Result<u64, StoreFault> {
        let count = self.one(
            "count requests",
            "SELECT count(*) FROM requests WHERE state = 'pending_approval' AND agent_id = ?1",
            [agent_id.as_str()],
            |row| row.get::<_, i64>(0),
        )?;
        count_of(count.unwrap_or(0), "requests", "state")
    }

    /// Ascending.
    pub fn received_since(&self, agent_id: &Id, since_ms: i64) -> Result<Vec<i64>, StoreFault> {
        self.all(
            "read requests",
            "SELECT received_at_ms FROM requests WHERE agent_id = ?1 AND received_at_ms >= ?2 \
             ORDER BY received_at_ms",
            rusqlite::params![agent_id.as_str(), since_ms],
            |row| row.get::<_, i64>(0),
        )
    }

    /// One grant.
    pub fn grant(&self, id: &Id) -> Result<Option<Grant>, StoreFault> {
        self.one(
            "read grants",
            "SELECT record FROM grants WHERE grant_id = ?1",
            [id.as_str()],
            |row| row.get::<_, String>(0),
        )?
        .map(|record| from_text(&record, "grants", "record"))
        .transpose()
    }

    /// The agent that owns a grant, if the grant exists.
    pub(crate) fn grant_agent(&self, id: &Id) -> Result<Option<String>, StoreFault> {
        self.one(
            "read grants",
            "SELECT agent_id FROM grants WHERE grant_id = ?1",
            [id.as_str()],
            |row| row.get::<_, String>(0),
        )
    }

    /// Every grant not ended.
    pub fn live_grants(&self) -> Result<Vec<Grant>, StoreFault> {
        self.all(
            "read grants",
            "SELECT record FROM grants WHERE ended_at_ms IS NULL ORDER BY rowid",
            [],
            |row| row.get::<_, String>(0),
        )?
        .iter()
        .map(|record| from_text(record, "grants", "record"))
        .collect()
    }

    /// Live and ended.
    pub fn grants_of_agent(&self, agent_id: &Id) -> Result<Vec<Grant>, StoreFault> {
        self.all(
            "read grants",
            "SELECT record FROM grants WHERE agent_id = ?1 ORDER BY rowid",
            [agent_id.as_str()],
            |row| row.get::<_, String>(0),
        )?
        .iter()
        .map(|record| from_text(record, "grants", "record"))
        .collect()
    }

    /// Newest row, any kind.
    pub fn proof(&self, proof_id: &Id) -> Result<Option<ProofRecord>, StoreFault> {
        self.one(
            "read proofs",
            "SELECT record FROM proof_reports WHERE proof_id = ?1 ORDER BY proof_row DESC LIMIT 1",
            [proof_id.as_str()],
            |row| row.get::<_, String>(0),
        )?
        .map(|record| from_text(&record, "proof_reports", "record"))
        .transpose()
    }

    /// Distinct ids of the request's reports of that kind, newest first, at most `limit`.
    pub fn proof_ids_of(
        &self,
        request_id: &Id,
        kind: ProverKind,
        limit: usize,
    ) -> Result<Vec<Id>, StoreFault> {
        self.all(
            "read proofs",
            "SELECT proof_id FROM proof_reports WHERE request_id = ?1 AND prover_kind = ?2 \
             GROUP BY proof_id ORDER BY max(proof_row) DESC LIMIT ?3",
            rusqlite::params![
                request_id.as_str(),
                name_of(&kind),
                i64::try_from(limit).unwrap_or(i64::MAX)
            ],
            |row| row.get::<_, String>(0),
        )?
        .into_iter()
        .map(|id| id_of(id, "proof_reports", "proof_id"))
        .collect()
    }

    /// One authorization.
    pub fn authorization(&self, id: &Id) -> Result<Option<AuthorizationRecord>, StoreFault> {
        self.one(
            "read authorizations",
            &format!(
                "SELECT {AUTHORIZATION_COLUMNS} FROM authorizations WHERE authorization_id = ?1"
            ),
            [id.as_str()],
            raw_authorization,
        )?
        .map(decode_authorization)
        .transpose()
    }

    /// The request's newest authorization.
    pub fn latest_authorization_of(
        &self,
        request_id: &Id,
    ) -> Result<Option<AuthorizationRecord>, StoreFault> {
        self.one(
            "read authorizations",
            &format!(
                "SELECT {AUTHORIZATION_COLUMNS} FROM authorizations WHERE request_id = ?1 \
                 ORDER BY issued_at_ms DESC, rowid DESC LIMIT 1"
            ),
            [request_id.as_str()],
            raw_authorization,
        )?
        .map(decode_authorization)
        .transpose()
    }

    /// Every authorization issued and neither consumed nor voided.
    pub fn issued_authorizations(&self) -> Result<Vec<AuthorizationRecord>, StoreFault> {
        self.all(
            "read authorizations",
            &format!(
                "SELECT {AUTHORIZATION_COLUMNS} FROM authorizations WHERE state = 'issued' \
                 ORDER BY issued_at_ms, rowid"
            ),
            [],
            raw_authorization,
        )?
        .into_iter()
        .map(decode_authorization)
        .collect()
    }

    /// The request's effect, if reserved.
    pub fn effect_of(&self, request_id: &Id) -> Result<Option<EffectRow>, StoreFault> {
        self.one(
            "read effects",
            &format!("SELECT {EFFECT_COLUMNS} FROM effects WHERE request_id = ?1"),
            [request_id.as_str()],
            RawEffect::from_row,
        )?
        .map(RawEffect::decode)
        .transpose()
    }

    /// One effect.
    pub(crate) fn effect(&self, effect_id: &Id) -> Result<Option<EffectRow>, StoreFault> {
        self.one(
            "read effects",
            &format!("SELECT {EFFECT_COLUMNS} FROM effects WHERE effect_id = ?1"),
            [effect_id.as_str()],
            RawEffect::from_row,
        )?
        .map(RawEffect::decode)
        .transpose()
    }

    /// Effects whose request is `dispatching`, `outcome_unknown` or `reconciling`, each with its target event
    /// (`TrustedFacts.pending_effects`).
    pub fn open_effects(&self) -> Result<Vec<OpenEffect>, StoreFault> {
        let columns = EFFECT_COLUMNS
            .split(", ")
            .map(|c| format!("e.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let rows = self.all(
            "read effects",
            &format!(
                "SELECT {columns}, r.action FROM effects e JOIN requests r \
                 ON r.request_id = e.request_id \
                 WHERE r.state IN ('dispatching', 'outcome_unknown', 'reconciling') \
                 ORDER BY e.created_at_ms, e.effect_id"
            ),
            [],
            |row| Ok((RawEffect::from_row(row)?, row.get::<_, String>(10)?)),
        )?;
        let mut effects = Vec::with_capacity(rows.len());
        for (raw, action) in rows {
            let effect = raw.decode()?;
            let action: Action = from_text(&action, "requests", "action")?;
            let event_id = match (&effect.provider_event_id, &action) {
                (Some(id), _) => id.clone(),
                (None, Action::ProposeUpdate(a)) => a.event.event_id.as_str().to_owned(),
                (None, Action::ProposeInvitation(a)) => a.event.event_id.as_str().to_owned(),
                (None, Action::ProposeDelete(a)) => a.event.event_id.as_str().to_owned(),
                (None, Action::ProposeResponse(a)) => match &a.target {
                    ResponseTarget::Event(event) => event.event_id.as_str().to_owned(),
                    ResponseTarget::Occurrence(occurrence) => {
                        occurrence.instance_id.as_str().to_owned()
                    }
                },
                (None, _) => return Err(invariant("an open effect names no event")),
            };
            effects.push(OpenEffect { effect, event_id });
        }
        Ok(effects)
    }

    /// The effect's attempts, in order.
    pub fn attempts(&self, effect_id: &Id) -> Result<Vec<AttemptRow>, StoreFault> {
        const T: &str = "attempts";
        self.all(
            "read attempts",
            "SELECT effect_id, authorization_id, record FROM attempts WHERE effect_id = ?1 \
             ORDER BY number",
            [effect_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )?
        .into_iter()
        .map(|(effect_id, authorization_id, record)| {
            Ok(AttemptRow {
                effect_id: id_of(effect_id, T, "effect_id")?,
                authorization_id: id_of(authorization_id, T, "authorization_id")?,
                attempt: from_text(&record, T, "record")?,
            })
        })
        .collect()
    }

    /// The stored answer of one owner command.
    pub fn command(&self, command_id: &Id) -> Result<Option<CommandRow>, StoreFault> {
        const T: &str = "commands";
        self.one(
            "read commands",
            "SELECT command_id, method, digest, answer FROM commands WHERE command_id = ?1",
            [command_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )?
        .map(|(command_id, method, digest, answer)| {
            Ok(CommandRow {
                command_id: id_of(command_id, T, "command_id")?,
                method,
                digest: fabric_desktop_contracts::scalar::Digest::new(digest).map_err(|_| {
                    StoreFault::Decode {
                        table: T,
                        column: "digest",
                    }
                })?,
                answer,
            })
        })
        .transpose()
    }

    /// Earliest `requests.due_at_ms`. Session idle deadlines and audit windows are the core's (memory).
    pub fn next_due_ms(&self) -> Result<Option<i64>, StoreFault> {
        Ok(self
            .one(
                "read requests",
                "SELECT min(due_at_ms) FROM requests",
                [],
                |row| row.get::<_, Option<i64>>(0),
            )?
            .flatten())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mutation::Mutation;
    use crate::testing::{self, T0, TempDir, id};

    #[test]
    fn next_due_is_the_earliest_due_request() {
        let dir = TempDir::new();
        let mut store = testing::world(&dir);
        assert_eq!(store.read(|r| r.next_due_ms()).unwrap(), Some(T0));
        let schedule = |request: &str, due| Mutation::RequestScheduled {
            request_id: id(request),
            due_at_ms: due,
            reproofs: Reproofs::default(),
        };
        testing::apply_all(
            &mut store,
            T0,
            vec![
                schedule("request-1", Some(T0 + 500)),
                schedule("request-2", Some(T0 + 200)),
            ],
        )
        .unwrap();
        assert_eq!(store.read(|r| r.next_due_ms()).unwrap(), Some(T0 + 200));
        let due = store.read(|r| r.due_requests(T0 + 500, 10)).unwrap();
        let ids: Vec<&str> = due
            .iter()
            .map(|d| d.row.request.request_id.as_str())
            .collect();
        assert_eq!(ids, vec!["request-2", "request-1"]);
        assert_eq!(
            store.read(|r| r.due_requests(T0 + 499, 10)).unwrap().len(),
            1
        );
        assert_eq!(
            store.read(|r| r.due_requests(T0 + 500, 1)).unwrap().len(),
            1
        );
        testing::apply_all(
            &mut store,
            T0,
            vec![schedule("request-1", None), schedule("request-2", None)],
        )
        .unwrap();
        assert_eq!(store.read(|r| r.next_due_ms()).unwrap(), None);
    }

    #[test]
    fn proof_ids_are_distinct_newest_first_and_of_one_kind() {
        let dir = TempDir::new();
        let mut store = testing::world(&dir);
        let record = |proof: &str, prover: &str| Mutation::ProofRecorded {
            proof: testing::proof(proof, "request-1", prover),
            at_ms: T0,
        };
        testing::apply_all(
            &mut store,
            T0,
            vec![
                record("proof-a", "isabelle"),
                record("proof-b", "isabelle"),
                record("proof-a", "isabelle"),
                record("proof-s", "synthetic"),
            ],
        )
        .unwrap();
        let ids = |kind, limit| {
            store
                .read(|r| r.proof_ids_of(&id("request-1"), kind, limit))
                .unwrap()
                .into_iter()
                .map(|i| i.as_str().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(ProverKind::Isabelle, 200), vec!["proof-a", "proof-b"]);
        assert_eq!(ids(ProverKind::Isabelle, 1), vec!["proof-a"]);
        assert_eq!(ids(ProverKind::Synthetic, 200), vec!["proof-s"]);
        let newest = store.read(|r| r.proof(&id("proof-s"))).unwrap().unwrap();
        assert_eq!(newest.prover, ProverKind::Synthetic);
        assert!(store.read(|r| r.proof(&id("proof-x"))).unwrap().is_none());
    }

    #[test]
    fn an_open_effect_names_the_event_it_targets() {
        let dir = TempDir::new();
        let mut store = testing::world(&dir);
        let delete = "fabric_calendar_propose_delete";
        let respond = "fabric_calendar_propose_response";
        let mut mutations = vec![
            Mutation::RequestInserted {
                row: testing::request("request-3", "agent-1", delete),
                due_at_ms: None,
            },
            Mutation::RequestInserted {
                row: testing::request("request-4", "agent-1", respond),
                due_at_ms: None,
            },
            Mutation::EffectReserved {
                effect: testing::effect("effect-1", "request-1", "fabric_calendar_propose_create"),
            },
            Mutation::EffectReserved {
                effect: testing::effect("effect-3", "request-3", delete),
            },
            Mutation::EffectReserved {
                effect: testing::effect("effect-4", "request-4", respond),
            },
        ];
        for request in ["request-1", "request-3", "request-4"] {
            for (revision, state) in [
                (2, "validating"),
                (3, "proving_classification"),
                (4, "allowed"),
                (5, "proving_fresh_permit"),
                (6, "queued"),
                (7, "dispatching"),
            ] {
                mutations.push(Mutation::RequestMoved {
                    request_id: id(request),
                    revision,
                    status: testing::status(state),
                    at_ms: T0,
                    due_at_ms: None,
                });
            }
        }
        testing::apply_all(&mut store, T0, mutations).unwrap();
        let mut targets: Vec<(String, String)> = store
            .read(|r| r.open_effects())
            .unwrap()
            .into_iter()
            .map(|o| (o.effect.effect_id.as_str().to_owned(), o.event_id))
            .collect();
        targets.sort();
        assert_eq!(
            targets,
            vec![
                ("effect-1".to_owned(), "fabriccreated0001".to_owned()),
                ("effect-3".to_owned(), "event-7".to_owned()),
                ("effect-4".to_owned(), "event-7_20261120".to_owned()),
            ]
        );
    }

    #[test]
    fn counts_and_lookups_follow_the_requests() {
        let dir = TempDir::new();
        let store = testing::world(&dir);
        store
            .read(|r| {
                assert_eq!(r.count_open(None)?, 2);
                assert_eq!(r.count_open(Some(&id("agent-1")))?, 1);
                assert_eq!(r.count_cards(&id("agent-1"))?, 0);
                assert_eq!(r.received_since(&id("agent-1"), T0)?, vec![T0]);
                assert!(r.received_since(&id("agent-1"), T0 + 1)?.is_empty());
                let by_call = r.request_by_call(&id("agent-1"), &id("call-request-1"))?;
                assert_eq!(
                    by_call.map(|c| c.row.request.request_id),
                    Some(id("request-1"))
                );
                assert_eq!(r.open_requests()?.len(), 2);
                assert_eq!(r.grants_of_agent(&id("agent-2"))?.len(), 1);
                assert_eq!(r.current_run()?, Some(1));
                assert_eq!(r.last_committed_ms()?, Some(T0));
                assert_eq!(r.counters()?.pause_epoch, 0);
                assert_eq!(r.preferences()?.revision.get(), 0);
                assert_eq!(r.accounts()?.len(), 1);
                assert!(r.issued_authorizations()?.is_empty());
                assert!(r.command(&id("command-1"))?.is_none());
                Ok(())
            })
            .unwrap();
    }
}
