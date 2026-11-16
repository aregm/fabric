//! Shared helpers of the unit tests: temporary directories, test stores and contract records.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use fabric_desktop_contracts::owner::{Account, Grant, LifecycleStatus};
use fabric_desktop_contracts::scalar::{Digest, Id, Timestamp};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::key::TestKey;
use crate::mutation::Mutation;
use crate::rows::{
    AgentRow, AttemptRow, AuthorizationRecord, AuthorizationState, EffectRow, Fence, ProofRecord,
    RequestRow,
};
use crate::store::{OpenOptions, Store};
use crate::tx::{Actor, Committed, OpMeta, TxError};

/// The test epoch of design 9.
pub(crate) const T0: i64 = 1_795_190_400_000;

/// A directory under the system temporary directory, removed on drop.
pub(crate) struct TempDir(PathBuf);

impl TempDir {
    pub(crate) fn new() -> TempDir {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "fabric-storage-unit-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a temporary directory");
        TempDir(path)
    }

    /// The data directory a test passes: a child that does not exist yet.
    pub(crate) fn data(&self) -> PathBuf {
        self.0.join("data")
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Options for a test-key store in `dir` at `now_ms`.
pub(crate) fn options(dir: &TempDir, now_ms: i64) -> OpenOptions {
    OpenOptions {
        data_dir: dir.data(),
        key: Box::new(TestKey),
        now_ms,
    }
}

/// A new test-key store.
pub(crate) fn store(dir: &TempDir) -> Store {
    Store::open(options(dir, T0)).expect("a test store opens")
}

/// A contract value from JSON.
pub(crate) fn from<T: DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).expect("a valid contract value")
}

pub(crate) fn id(text: &str) -> Id {
    Id::new(text).expect("an id")
}

pub(crate) fn ts(ms: i64) -> Timestamp {
    crate::time::format_timestamp(u64::try_from(ms).expect("after 1970"))
}

pub(crate) fn digest(fill: char) -> Digest {
    Digest::new(format!("sha256:{}", fill.to_string().repeat(64))).expect("a digest")
}

pub(crate) fn status(state: &str) -> LifecycleStatus {
    from(json!({ "state": state }))
}

pub(crate) fn agent(agent_id: &str, revision: u64) -> AgentRow {
    from(json!({
        "agent_id": agent_id,
        "revision": revision,
        "registered_name": "Test agent",
        "destination": { "kind": "local", "endpoint": "in-process" },
        "paused": false,
        "enrolled_at_ms": T0,
    }))
}

pub(crate) fn account(account_id: &str, revision: u64) -> Account {
    from(json!({
        "account_id": account_id,
        "revision": revision,
        "state": "connected",
        "email": null,
        "calendars": [],
    }))
}

/// The action of a create, an update or a response to an occurrence.
pub(crate) fn action(tool: &str) -> Value {
    let time = |hour: &str| json!({ "date_time": format!("2026-11-20T{hour}:00:00+00:00"), "zone": "Europe/Berlin" });
    let event_ref = json!({ "event_id": "event-7", "revision": "rev-1" });
    let arguments = match tool {
        "fabric_calendar_propose_create" => json!({
            "account_id": "account-1",
            "calendar_id": "calendar-1",
            "event": { "title": "Review", "start": time("16"), "end": time("17"),
                       "description": null, "location": null },
            "task": "plan the review",
        }),
        "fabric_calendar_propose_delete" => json!({
            "account_id": "account-1",
            "calendar_id": "calendar-1",
            "event": event_ref,
            "send_updates": "none",
            "task": "clear the slot",
        }),
        "fabric_calendar_propose_response" => json!({
            "account_id": "account-1",
            "calendar_id": "calendar-1",
            "target": { "kind": "occurrence", "occurrence": {
                "instance_id": "event-7_20261120", "original_start_time": time("16"),
                "revision": "rev-1" } },
            "response": "accepted",
            "send_updates": "none",
            "task": "accept",
        }),
        other => panic!("no test action for {other}"),
    };
    json!({ "tool": tool, "arguments": arguments })
}

/// A new request of `agent_id`, received at `T0`.
pub(crate) fn request(request_id: &str, agent_id: &str, tool: &str) -> RequestRow {
    from(json!({
        "request": {
            "request_id": request_id,
            "revision": 1,
            "agent_id": agent_id,
            "tool": tool,
            "account_id": "account-1",
            "received_at": ts(T0),
            "updated_at": ts(T0),
            "request_digest": digest('a'),
            "status": { "state": "received" },
        },
        "action": action(tool),
        "scope": null,
        "session_id": null,
        "call_id": format!("call-{request_id}"),
    }))
}

/// A live until-revoked grant of `agent_id`, created by `origin`.
pub(crate) fn grant(grant_id: &str, agent_id: &str, origin: &str) -> Grant {
    from(json!({
        "grant_id": grant_id,
        "revision": 1,
        "agent_id": agent_id,
        "purpose": "plan the review",
        "tools": ["fabric_calendar_propose_create"],
        "scope": {
            "calendars": [{ "account_id": "account-1", "calendar_id": "calendar-1" }],
            "window": null, "fields": null, "recipients": null, "send_updates": null,
        },
        "term": { "kind": "until_revoked", "expires_at": null },
        "origin": { "request_id": origin, "approval_digest": digest('b') },
        "policy": { "profile_id": "careful", "version": 1 },
        "revocation_epoch": 0,
        "created_at": ts(T0),
        "ended": null,
    }))
}

pub(crate) fn fence() -> Fence {
    Fence {
        pause_epoch: 0,
        revocation_epoch: 0,
        event_revision: None,
    }
}

/// An issued authorization of `request_id` in `run_id`, resting on `grant_id`.
pub(crate) fn authorization(
    authorization_id: &str,
    request_id: &str,
    agent_id: &str,
    grant_id: &str,
    run_id: i64,
    issued_at_ms: i64,
) -> AuthorizationRecord {
    AuthorizationRecord {
        authorization: from(json!({
            "profile": "fabric-broker-authorization/1",
            "authorization_id": authorization_id,
            "request_id": request_id,
            "agent_id": agent_id,
            "action_digest": digest('c'),
            "proof_id": "proof-1",
            "grant_id": grant_id,
            "issued_at": ts(issued_at_ms),
            "expires_at": ts(issued_at_ms + 60_000),
            "lifetime_seconds": 60,
            "consumed_at": null,
        })),
        grant_ids: vec![id(grant_id)],
        attempt: 1,
        run_id,
        fence: fence(),
        state: AuthorizationState::Issued,
    }
}

/// An effect of `request_id`; a create reserves its event id.
pub(crate) fn effect(effect_id: &str, request_id: &str, tool: &str) -> EffectRow {
    let create = tool == "fabric_calendar_propose_create";
    EffectRow {
        effect_id: id(effect_id),
        request_id: id(request_id),
        tool: from(json!(tool)),
        account_id: id("account-1"),
        calendar_id: id("calendar-1"),
        action_digest: digest('c'),
        provider_event_id: create.then(|| "fabriccreated0001".to_owned()),
        created_at_ms: T0,
        attempts_started: 0,
        first_attempt_at_ms: None,
    }
}

pub(crate) fn attempt(
    effect_id: &str,
    authorization_id: &str,
    number: u64,
    at_ms: i64,
) -> AttemptRow {
    AttemptRow {
        effect_id: id(effect_id),
        authorization_id: id(authorization_id),
        attempt: from(json!({
            "number": number,
            "proof_id": "proof-1",
            "started_at": ts(at_ms),
            "finished_at": null,
            "outcome": null,
        })),
    }
}

pub(crate) fn proof(proof_id: &str, request_id: &str, prover: &str) -> ProofRecord {
    from(json!({
        "report": {
            "proof_id": proof_id,
            "request_id": request_id,
            "purpose": "classification",
            "outcome": "permit",
            "failing_rule": null,
            "rule_verdicts": [],
            "isabelle_version": "Isabelle2025",
            "theory_hash": digest('d'),
            "context_digest": digest('e'),
            "action_digest": digest('c'),
            "started_at": ts(T0),
            "elapsed_ms": 5,
        },
        "prover": prover,
        "ruling": null,
        "authorizing_grants": [],
        "undecided_reason": null,
        "fence": { "pause_epoch": 0, "revocation_epoch": 0, "event_revision": null },
        "check_theory_text": null,
    }))
}

/// The metadata of a broker write at `now_ms`.
pub(crate) fn meta(operation_type: &'static str, now_ms: i64) -> OpMeta {
    OpMeta {
        operation_type,
        actor: Actor::broker(),
        idempotency: None,
        causation_id: None,
        correlation_id: None,
        now_ms,
    }
}

/// Applies the mutations in one write.
pub(crate) fn apply_all(
    store: &mut Store,
    now_ms: i64,
    mutations: Vec<Mutation>,
) -> Result<Committed<()>, TxError<()>> {
    store.write(meta("request.advance", now_ms), |tx| {
        for mutation in mutations {
            tx.apply(mutation)?;
        }
        Ok(())
    })
}

/// The fault of a write that must fail, as fixed text.
pub(crate) fn refused_with(result: Result<Committed<()>, TxError<()>>) -> &'static str {
    match result {
        Err(TxError::Fault(crate::tx::StoreFault::Invariant { what })) => what,
        Err(TxError::Fault(other)) => panic!("expected a broken rule, got {other}"),
        Err(TxError::Refused(())) => panic!("expected a broken rule, got a refusal"),
        Ok(_) => panic!("expected a broken rule, the write committed"),
    }
}

/// A store with run 1, agents `agent-1` and `agent-2`, account `account-1`, a create request
/// of each agent (`request-1`, `request-2`) and an until-revoked grant of each (`grant-1`,
/// `grant-2`).
pub(crate) fn world(dir: &TempDir) -> Store {
    let mut store = store(dir);
    let create = "fabric_calendar_propose_create";
    apply_all(
        &mut store,
        T0,
        vec![
            Mutation::RunStarted {
                run_id: 1,
                at_ms: T0,
            },
            Mutation::AgentPut {
                agent: agent("agent-1", 1),
            },
            Mutation::AgentPut {
                agent: agent("agent-2", 1),
            },
            Mutation::AccountPut {
                account: account("account-1", 1),
            },
            Mutation::RequestInserted {
                row: request("request-1", "agent-1", create),
                due_at_ms: Some(T0),
            },
            Mutation::RequestInserted {
                row: request("request-2", "agent-2", create),
                due_at_ms: None,
            },
            Mutation::GrantInserted {
                grant: grant("grant-1", "agent-1", "request-1"),
            },
            Mutation::GrantInserted {
                grant: grant("grant-2", "agent-2", "request-2"),
            },
        ],
    )
    .expect("the test world commits");
    store
}
