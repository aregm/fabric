//! Agent sessions (5.5) and the contract view of an agent.

use fabric_desktop_contracts::RequestState;
use fabric_desktop_contracts::owner::{Agent, AgentEnrollment, Change, GrantTerm, Session};
use fabric_desktop_contracts::scalar::{Id, PositiveCount};
use fabric_storage::{Mutation, ReadTx, SessionEndReason, SessionRow, StoreFault, WriteTx};

use crate::service::reducer::{Event, next};
use crate::service::requests::move_request;
use crate::service::{
    Activity, Cx, LiveSession, SessionChange, grants, invariant, plain, timestamp,
};

/// Raises the agent's revision by one and emits its notice (5.4); returns the notice's sequence.
pub(crate) fn raise_agent(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    agent_id: &Id,
) -> Result<PositiveCount, StoreFault> {
    let mut agent = tx
        .read()
        .agent(agent_id)?
        .ok_or(invariant("the agent exists"))?;
    agent.revision += 1;
    tx.apply(Mutation::AgentPut { agent })?;
    let view =
        agent_view(&tx.read(), cx.activity, agent_id)?.ok_or(invariant("the agent exists"))?;
    tx.notify(Change::Agent(view))
}

/// Starts a session of the agent in this run, ending its live session first (`Replaced`), and
/// raises the agent revision once with one agent notice (5.4, 5.5). Returns the new session.
pub(crate) fn start_session(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    agent_id: &Id,
) -> Result<LiveSession, StoreFault> {
    if let Some(old) = tx.read().live_session(agent_id)? {
        end_session(tx, cx, &old.session_id, SessionEndReason::Replaced)?;
    }
    let session = LiveSession {
        session_id: crate::ids::new_id("session"),
        started_at_ms: cx.now_ms,
        last_active_ms: cx.now_ms,
    };
    tx.apply(Mutation::SessionStarted {
        session: SessionRow {
            session_id: session.session_id.clone(),
            agent_id: agent_id.clone(),
            run_id: cx.run_id,
            started_at_ms: session.started_at_ms,
            ended: None,
        },
    })?;
    raise_agent(tx, cx, agent_id)?;
    cx.after.sessions.push(SessionChange::Started {
        agent_id: agent_id.clone(),
        session: session.clone(),
    });
    Ok(session)
}

/// Ends the session and its grants (grant notices) and queues `SessionChange::Ended`. For every reason except
/// `Replaced` it also raises the agent revision and emits the agent notice; for `Replaced` the connect's
/// `SessionStarted` in the same commit does both, once (5.4).
pub(crate) fn end_session(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    session_id: &Id,
    reason: SessionEndReason,
) -> Result<Vec<PositiveCount>, StoreFault> {
    let session = tx
        .read()
        .session(session_id)?
        .ok_or(invariant("only a known session ends"))?;
    if session.ended.is_some() {
        return Err(invariant("only a live session ends"));
    }
    tx.apply(Mutation::SessionEnded {
        session_id: session_id.clone(),
        at_ms: cx.now_ms,
        reason,
    })?;
    let bound: Vec<Id> = tx
        .read()
        .grants_of_agent(&session.agent_id)?
        .into_iter()
        .filter(|g| {
            g.ended.is_none()
                && matches!(&g.term, GrantTerm::Session { session_id: bound } if bound == session_id)
        })
        .map(|g| g.grant_id)
        .collect();
    let mut sequences = Vec::with_capacity(bound.len() + 1);
    for grant_id in bound {
        if let Some((_, sequence)) =
            grants::end_grant(tx, cx, &grant_id, reason.grant_end_reason())?
        {
            sequences.push(sequence);
        }
    }
    if reason != SessionEndReason::Replaced {
        sequences.push(raise_agent(tx, cx, &session.agent_id)?);
    }
    cx.after.sessions.push(SessionChange::Ended {
        agent_id: session.agent_id,
    });
    Ok(sequences)
}

/// The contract `Agent`: the durable row joined with the live session and the activity of `cx.activity`.
///
/// The session is the store's live session of the agent, so a view built inside a transaction
/// shows the session that transaction started or ended. Its `last_active_at` comes from memory
/// when the activity knows that session, and is its start otherwise (D35).
///
/// `session.last_active_at` and `last_tool_call_at` are live fields outside the cursor: a call
/// or a poll changes them in memory only, with no commit and no notice (D35), so two
/// snapshots at one cursor may differ in them and a subscriber sees them only in its next
/// snapshot. This departs from README section 7 ("the state at a cursor") and is recorded as
/// CCR-17 for the contract owner.
pub(crate) fn agent_view(
    r: &ReadTx<'_>,
    activity: &Activity,
    agent_id: &Id,
) -> Result<Option<Agent>, StoreFault> {
    let Some(row) = r.agent(agent_id)? else {
        return Ok(None);
    };
    let session = r.live_session(agent_id)?.map(|live| {
        let last_active_ms = activity
            .sessions
            .get(agent_id)
            .filter(|known| known.session_id == live.session_id)
            .map_or(live.started_at_ms, |known| known.last_active_ms);
        Session {
            session_id: live.session_id,
            started_at: timestamp(live.started_at_ms),
            last_active_at: timestamp(last_active_ms),
        }
    });
    // Contract schema version 3: an agent the store holds is approved; the store keeps no key,
    // so it has no pairing code, and its enrollment time stands for when it was added and
    // approved.
    let enrolled_at = timestamp(row.enrolled_at_ms);
    Ok(Some(Agent {
        agent_id: row.agent_id,
        revision: PositiveCount::new(row.revision)
            .map_err(|_| invariant("an agent revision is positive"))?,
        registered_name: row.registered_name,
        destination: row.destination,
        enrollment: AgentEnrollment::Approved,
        pairing_code: None,
        added_at: enrolled_at.clone(),
        key_proven_at: None,
        approved_at: Some(enrolled_at),
        removed_at: None,
        paused: row.paused,
        session,
        last_tool_call_at: activity
            .last_tool_call_ms
            .get(agent_id)
            .map(|ms| timestamp(*ms)),
    }))
}

/// Makes the account's pending requests stale.
pub(crate) fn stale_pending_of_account(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    account_id: &Id,
) -> Result<Vec<PositiveCount>, StoreFault> {
    let pending: Vec<Id> = tx
        .read()
        .open_requests()?
        .into_iter()
        .filter(|r| {
            r.row.request.status.state() == RequestState::PendingApproval
                && r.row.request.account_id.as_ref() == Some(account_id)
        })
        .map(|r| r.row.request.request_id)
        .collect();
    let mut sequences = Vec::with_capacity(pending.len());
    let stale = next(RequestState::PendingApproval, Event::AccountGone)
        .ok_or(invariant("a pending request goes stale"))?;
    for request_id in pending {
        sequences.push(move_request(tx, cx, &request_id, plain(stale)?, None)?);
    }
    Ok(sequences)
}
