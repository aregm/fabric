//! The application service (design 5): one writer over the store, the reducer, the pipeline,
//! sessions, grants and recovery. Crate-private; the owner channel (`crate::owner`) runs its
//! commands inside the transactions this module opens.

use std::collections::HashMap;

use fabric_desktop_contracts::RequestState;
use fabric_desktop_contracts::error::OwnerFault;
use fabric_desktop_contracts::owner::LifecycleStatus;
use fabric_desktop_contracts::scalar::{Id, Timestamp};
use fabric_storage::StoreFault;

pub(crate) mod activity;
pub(crate) mod agent;
pub(crate) mod clock;
pub(crate) mod core;
pub(crate) mod durations;
pub(crate) mod epochs;
pub(crate) mod facts;
pub(crate) mod grants;
pub(crate) mod limits;
pub(crate) mod pipeline;
pub(crate) mod recovery;
pub(crate) mod reducer;
pub(crate) mod relied;
pub(crate) mod requests;
pub(crate) mod sessions;
#[cfg(all(test, debug_assertions))]
mod tests;

/// An owner command's refusal: the contract fault and the owner message.
pub(crate) struct OwnerRefusal {
    pub(crate) fault: OwnerFault,
    pub(crate) message: String,
}

/// A live session of this run, in memory (D35). `last_active_ms` starts at `started_at_ms` (connect).
#[derive(Clone, Debug)]
pub(crate) struct LiveSession {
    pub(crate) session_id: Id,
    pub(crate) started_at_ms: i64,
    pub(crate) last_active_ms: i64,
}

/// Agent activity of this run, in memory only.
#[derive(Default)]
pub(crate) struct Activity {
    /// By agent id.
    pub(crate) sessions: HashMap<Id, LiveSession>,
    /// By agent id; absent until the agent's first call.
    pub(crate) last_tool_call_ms: HashMap<Id, i64>,
}

/// What a committed step changes in memory; applied only after commit.
pub(crate) enum SessionChange {
    Started { agent_id: Id, session: LiveSession },
    Ended { agent_id: Id },
}

/// What a step hands on once its transaction commits.
#[derive(Default)]
pub(crate) struct AfterCommit {
    pub(crate) jobs: Vec<crate::broker::Job>,
    pub(crate) sessions: Vec<SessionChange>,
}

/// Per-step context.
pub(crate) struct Cx<'a> {
    pub(crate) now_ms: i64,
    pub(crate) run_id: i64,
    pub(crate) activity: &'a Activity,
    pub(crate) after: &'a mut AfterCommit,
}

/// A broken rule of the service: a fault of the store, never a refusal.
pub(crate) const fn invariant(what: &'static str) -> StoreFault {
    StoreFault::Invariant { what }
}

/// The contract timestamp of a core-clock instant; an instant before 1970 is written as 1970.
pub(crate) fn timestamp(ms: i64) -> Timestamp {
    crate::format_timestamp(u64::try_from(ms).unwrap_or(0))
}

/// The status of a state that carries nothing but the state.
///
/// # Errors
///
/// [`StoreFault::Invariant`] for `pending_approval`, `blocked` and `terminal_failure`, which carry more.
pub(crate) fn plain(state: RequestState) -> Result<LifecycleStatus, StoreFault> {
    state
        .plain()
        .map(LifecycleStatus::Plain)
        .ok_or(invariant("a plain status names a plain state"))
}
