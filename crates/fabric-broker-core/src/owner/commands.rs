//! Owner commands, each inside the transaction `dedup` opens: decisions, pauses, revocations
//! and disconnecting Google (design 6.1).
//!
//! A command returns its result, or a refusal that rolls its transaction back. Its notices come
//! from the transaction; `commit_sequence` is the sequence of the command's last notice (5.4).

use fabric_desktop_contracts::error::{Conflict, OwnerFault};
use fabric_desktop_contracts::owner::{
    AccountState, Approval, Approved, Change, DecideRequestParams, DecideRequestResult, Denied,
    DisconnectGoogleParams, DisconnectGoogleResult, GrantEndReason, PauseAgentParams,
    PauseAgentResult, PauseTarget, RequestDecision, RevokeGrantParams, RevokeGrantResult,
};
use fabric_desktop_contracts::scalar::{Count, Id, List, PositiveCount};
use fabric_desktop_contracts::{Decision, RequestState, approval_digest};
use fabric_storage::{Counter, Mutation, StoreFault, TxError, WriteTx};

use super::frames::invalid_params;
use super::views::{OWNER_STATE_BUDGET_BYTES, owner_state_bytes};
use crate::service::grants::{create_grant, end_grant, end_grants_of_account};
use crate::service::relied::set_relied;
use crate::service::requests::{decided, move_request, record_of};
use crate::service::sessions::{agent_view, stale_pending_of_account};
use crate::service::{Cx, OwnerRefusal, epochs, invariant, timestamp};

/// A refusal of the command; its transaction rolls back.
pub(crate) fn refused<T>(fault: OwnerFault, message: &str) -> Result<T, TxError<OwnerRefusal>> {
    Err(TxError::Refused(OwnerRefusal {
        fault,
        message: message.to_owned(),
    }))
}

/// `not_found` for the record a command names.
pub(crate) fn not_found<T>(what: &str) -> Result<T, TxError<OwnerRefusal>> {
    refused(
        OwnerFault::NotFound,
        &format!("The broker has no {what} with this id."),
    )
}

/// `conflict` with the record's current revision (5.3 step 6).
pub(crate) fn conflict<T>(current_revision: u64, what: &str) -> Result<T, TxError<OwnerRefusal>> {
    let current_revision =
        Count::new(current_revision).map_err(|_| invariant("a revision is a safe integer"))?;
    refused(
        OwnerFault::Conflict(Conflict { current_revision }),
        &format!("The {what} changed since it was shown, so look at it again before changing it."),
    )
}

/// The answer to a command that would grow the owner's records past their budget.
pub(crate) const BUDGET_MESSAGE: &str = "The recipient sets, profiles and grants would no longer \
     fit in one snapshot frame of 1,048,576 bytes, so remove or shorten some first.";

/// Refuses a command whose changes so far grew the owner's records (`before` bytes) past
/// their budget, so that a snapshot stays within the frame bound (README section 1).
pub(crate) fn within_budget(
    tx: &WriteTx<'_>,
    before: usize,
    pointer: &str,
) -> Result<(), TxError<OwnerRefusal>> {
    let after = owner_state_bytes(&tx.read())?;
    if after > before && after > OWNER_STATE_BUDGET_BYTES {
        return refused(invalid_params(pointer), BUDGET_MESSAGE);
    }
    Ok(())
}

/// The revision a record has after one more change.
pub(crate) fn next_revision(revision: u64) -> Result<PositiveCount, StoreFault> {
    PositiveCount::new(revision + 1).map_err(|_| invariant("a revision is a safe integer"))
}

/// `decide_request` (6.1; D22): approve with a grant the pump then proves afresh, or deny.
pub(crate) fn decide_request(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    command_id: &Id,
    p: &DecideRequestParams,
) -> Result<DecideRequestResult, TxError<OwnerRefusal>> {
    let Some(record) = tx.read().request(&p.request_id)? else {
        return not_found("request");
    };
    let request = &record.row.request;
    let state = request.status.state();
    if state != RequestState::PendingApproval
        || request.revision.get() != p.expected_revision.get()
        || request.request_digest != p.request_digest
    {
        return refused(
            OwnerFault::StaleRequest,
            "The request is not the one that was shown, so review it again before deciding.",
        );
    }
    let request_digest = request.request_digest.clone();
    let decided_at = timestamp(cx.now_ms);
    let (approval, sequence) = match &p.decision {
        RequestDecision::Approve {
            selected: Some(_), ..
        } => {
            return refused(
                invalid_params("/params/selected"),
                "This build does not narrow a scope, so selected must be null.",
            );
        }
        RequestDecision::Approve {
            selected: None,
            duration,
        } => {
            let scope = record
                .card
                .as_ref()
                .ok_or(invariant("a pending request has a card"))?
                .scope
                .clone();
            let approval_digest = approval_digest(
                &p.request_id,
                &request_digest,
                &Decision::Approve {
                    scope: scope.clone(),
                    duration: *duration,
                },
            );
            let before = owner_state_bytes(&tx.read())?;
            let (grant, _) = create_grant(tx, cx, &record, &scope, *duration, &approval_digest)?;
            within_budget(tx, before, "/params/decision")?;
            let approval = Approval::Approve(Approved {
                command_id: command_id.clone(),
                decided_at,
                approval_digest,
                scope,
                duration: *duration,
                // The scope is approved as asked, so the effective digest is the request's.
                effective_request_digest: request_digest,
                grant_id: grant.grant_id.clone(),
            });
            tx.apply(Mutation::ApprovalRecorded {
                request_id: p.request_id.clone(),
                approval: approval.clone(),
            })?;
            set_relied(tx, &p.request_id, vec![grant.grant_id])?;
            let to = decided(state, true).ok_or(invariant("a pending request is approved"))?;
            // Due now: the pump of this step starts the fresh proof.
            let due = cx.now_ms;
            let sequence = move_request(tx, cx, &p.request_id, to, Some(due))?;
            (approval, sequence)
        }
        RequestDecision::Deny => {
            let approval = Approval::Deny(Denied {
                command_id: command_id.clone(),
                decided_at,
                approval_digest: approval_digest(&p.request_id, &request_digest, &Decision::Deny),
            });
            tx.apply(Mutation::ApprovalRecorded {
                request_id: p.request_id.clone(),
                approval: approval.clone(),
            })?;
            let to = decided(state, false).ok_or(invariant("a pending request is denied"))?;
            let sequence = move_request(tx, cx, &p.request_id, to, None)?;
            (approval, sequence)
        }
    };
    Ok(DecideRequestResult {
        commit_sequence: sequence,
        request: record_of(tx, &p.request_id)?.row.request,
        approval,
    })
}

/// `pause_agent` (6.1, 5.6): every targeted agent's revision rises by one with a notice, even
/// when its pause does not change; a pause raises `pause_epoch` once.
pub(crate) fn pause_agent(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    p: &PauseAgentParams,
) -> Result<PauseAgentResult, TxError<OwnerRefusal>> {
    let targets = match &p.target {
        PauseTarget::Agent {
            agent_id,
            expected_revision,
        } => {
            let Some(agent) = tx.read().agent(agent_id)? else {
                return not_found("agent");
            };
            if agent.revision != expected_revision.get() {
                return conflict(agent.revision, "agent");
            }
            vec![agent]
        }
        PauseTarget::AllAgents => {
            let agents = tx.read().agents()?;
            if agents.is_empty() {
                // CCR-3: a command that changes nothing has no commit sequence.
                return refused(OwnerFault::NotFound, "The broker has no agent to pause.");
            }
            agents
        }
    };
    if p.paused {
        epochs::raise(tx, Counter::PauseEpoch)?;
    }
    let mut views = Vec::with_capacity(targets.len());
    let mut last = None;
    for mut agent in targets {
        let agent_id = agent.agent_id.clone();
        agent.paused = p.paused;
        agent.revision += 1;
        tx.apply(Mutation::AgentPut { agent })?;
        let view =
            agent_view(&tx.read(), cx.activity, &agent_id)?.ok_or(invariant("the agent exists"))?;
        last = Some(tx.notify(Change::Agent(view.clone()))?);
        views.push(view);
    }
    Ok(PauseAgentResult {
        commit_sequence: last.ok_or(invariant("a pause names an agent"))?,
        agents: List::new(views).map_err(|_| invariant("any list of agents"))?,
    })
}

/// `revoke_grant` (6.1): the grant ends `revoked_by_owner` in this commit; an ended grant or
/// another revision is `conflict` (CCR-6).
pub(crate) fn revoke_grant(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    p: &RevokeGrantParams,
) -> Result<RevokeGrantResult, TxError<OwnerRefusal>> {
    let Some(grant) = tx.read().grant(&p.grant_id)? else {
        return not_found("grant");
    };
    if grant.ended.is_some() || grant.revision.get() != p.expected_revision.get() {
        return conflict(grant.revision.get(), "grant");
    }
    let (grant, sequence) = end_grant(tx, cx, &p.grant_id, GrantEndReason::RevokedByOwner)?
        .ok_or(invariant("a live grant ends"))?;
    Ok(RevokeGrantResult {
        commit_sequence: sequence,
        grant,
    })
}

/// `disconnect_google` (6.1): the account's live grants end `account_disconnected`, its pending
/// requests go stale, then the account is disconnected; the account notice is the last.
/// `revoke_provider_token` revokes nothing: no token exists before CD-W05.
pub(crate) fn disconnect_google(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    p: &DisconnectGoogleParams,
) -> Result<DisconnectGoogleResult, TxError<OwnerRefusal>> {
    let Some(mut account) = tx.read().account(&p.account_id)? else {
        return not_found("account");
    };
    if account.revision.get() != p.expected_revision.get() {
        return conflict(account.revision.get(), "account");
    }
    let ended = end_grants_of_account(tx, cx, &p.account_id)?;
    let _ = stale_pending_of_account(tx, cx, &p.account_id)?;
    if ended.is_empty() {
        // The account's state changes, so the revocation epoch rises even with no grant ended.
        epochs::raise(tx, Counter::RevocationEpoch)?;
    }
    account.state = AccountState::Disconnected;
    account.revision = next_revision(account.revision.get())?;
    tx.apply(Mutation::AccountPut {
        account: account.clone(),
    })?;
    let sequence = tx.notify(Change::Account(account.clone()))?;
    Ok(DisconnectGoogleResult {
        commit_sequence: sequence,
        account,
        ended_grant_ids: List::new(ended.into_iter().map(|(id, _)| id).collect())
            .map_err(|_| invariant("ended grant ids are distinct"))?,
    })
}
