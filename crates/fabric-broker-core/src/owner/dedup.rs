//! Command deduplication: one transaction per command id, and the stored answer for a repeat
//! (design 5.3, D23).
//!
//! A command's id is its frame id. A repeat with the same command digest gets the stored answer
//! byte for byte and writes nothing; another command under a used id is `command_reused`, audited
//! before the answer. A new command runs in one write that also stores its answer; a refusal
//! rolls that write back and is stored by its own `owner.refused` write. `store_unavailable` and
//! `internal` are never stored, and nothing stored is ever pruned.

use std::convert::Infallible;

use fabric_desktop_contracts::error::OwnerFault;
use fabric_desktop_contracts::owner::{
    OwnerCall, OwnerMethod, OwnerNotice, OwnerRequest, OwnerResult,
};
use fabric_desktop_contracts::scalar::{Digest, Id};
use fabric_desktop_contracts::{Entry, Frame, decode};
use fabric_storage::{
    Actor, AuditEntry, CommandRow, IdempotencyScope, Mutation, OpMeta, StoreFault, TxError, WriteTx,
};
use serde_json::json;

use super::frames::{refusal_frame, refuse, result_frame};
use super::{commands, groups, preferences, profiles};
use crate::service::core::Core;
use crate::service::{Cx, OwnerRefusal, invariant};

/// The audit kind of a command id used again for another command.
pub(crate) const COMMAND_REUSED_AUDIT: &str = "command_reused";

/// The ledger operation type of a command's transaction.
fn operation_type(method: OwnerMethod) -> Option<&'static str> {
    Some(match method {
        OwnerMethod::SaveProfile => "owner.save_profile",
        OwnerMethod::SavePreferences => "owner.save_preferences",
        OwnerMethod::SaveGroup => "owner.save_group",
        OwnerMethod::DecideRequest => "owner.decide_request",
        OwnerMethod::PauseAgent => "owner.pause_agent",
        OwnerMethod::RevokeGrant => "owner.revoke_grant",
        OwnerMethod::DisconnectGoogle => "owner.disconnect_google",
        OwnerMethod::Hello
        | OwnerMethod::GetSnapshot
        | OwnerMethod::ConnectGoogle
        | OwnerMethod::GetRequestDetails
        | OwnerMethod::GetProof
        | OwnerMethod::Subscribe => return None,
    })
}

/// The envelope of a command's write or of its refusal: the owner, `owner_command`/id, the
/// command as the cause, and for a decision the request it decides.
fn meta(operation_type: &'static str, request: &OwnerRequest) -> OpMeta {
    let correlation_id = match &request.call {
        OwnerCall::DecideRequest(p) => Some(p.request_id.clone()),
        _ => None,
    };
    OpMeta {
        operation_type,
        actor: Actor::owner(),
        idempotency: Some((
            IdempotencyScope::OwnerCommand,
            request.id.as_str().to_owned(),
        )),
        causation_id: Some(request.id.as_str().to_owned()),
        correlation_id,
        // `Core::write` sets the step's `now` (D34).
        now_ms: 0,
    }
}

/// Runs the command inside the transaction.
fn execute(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    request: &OwnerRequest,
) -> Result<OwnerResult, TxError<OwnerRefusal>> {
    Ok(match &request.call {
        OwnerCall::DecideRequest(p) => {
            OwnerResult::DecideRequest(commands::decide_request(tx, cx, &request.id, p)?)
        }
        OwnerCall::PauseAgent(p) => OwnerResult::PauseAgent(commands::pause_agent(tx, cx, p)?),
        OwnerCall::RevokeGrant(p) => OwnerResult::RevokeGrant(commands::revoke_grant(tx, cx, p)?),
        OwnerCall::DisconnectGoogle(p) => {
            OwnerResult::DisconnectGoogle(commands::disconnect_google(tx, cx, p)?)
        }
        OwnerCall::SaveProfile(p) => OwnerResult::SaveProfile(profiles::save_profile(tx, cx, p)?),
        OwnerCall::SavePreferences(p) => {
            OwnerResult::SavePreferences(preferences::save_preferences(tx, cx, p)?)
        }
        OwnerCall::SaveGroup(p) => OwnerResult::SaveGroup(groups::save_group(tx, cx, p)?),
        OwnerCall::Hello(_)
        | OwnerCall::GetSnapshot(_)
        | OwnerCall::ConnectGoogle(_)
        | OwnerCall::GetRequestDetails(_)
        | OwnerCall::GetProof(_)
        | OwnerCall::Subscribe(_) => {
            return Err(TxError::Fault(invariant("only a command is executed")));
        }
    })
}

/// Stores `frame` as the command's answer in this transaction.
fn store_answer(
    tx: &mut WriteTx<'_>,
    request: &OwnerRequest,
    digest: &Digest,
    frame: &Frame,
) -> Result<(), StoreFault> {
    let mut answer = frame.to_line();
    answer.pop();
    tx.apply(Mutation::CommandAnswered {
        row: CommandRow {
            command_id: request.id.clone(),
            method: request.call.method().as_str().to_owned(),
            digest: digest.clone(),
            answer,
        },
    })
}

/// The stored answer of a command, as a frame again.
fn stored_frame(answer: &str) -> Result<Frame, StoreFault> {
    decode(Entry::OwnerResponse, answer.as_bytes()).map_err(|_| StoreFault::Decode {
        table: "commands",
        column: "answer",
    })
}

/// One command: the stored answer of a repeat, `command_reused`, or a new commit. Returns the
/// notices the command committed and its answer.
///
/// # Errors
///
/// A fault of the store: the frame is then answered `store_unavailable` and the run halts.
pub(crate) fn command(
    core: &mut Core,
    request: &OwnerRequest,
) -> Result<(Vec<OwnerNotice>, Frame), StoreFault> {
    let method = request.call.method();
    let (Some(digest), Some(operation_type)) = (request.command_digest(), operation_type(method))
    else {
        return Err(invariant("only a command is deduplicated"));
    };
    if let Some(stored) = core.read(|r| r.command(&request.id))? {
        if stored.digest == digest {
            return Ok((Vec::new(), stored_frame(&stored.answer)?));
        }
        core.write_audit(vec![AuditEntry {
            kind: COMMAND_REUSED_AUDIT,
            actor: Actor::owner(),
            request_id: None,
            detail: json!({"command_id": request.id, "method": method.as_str(),
                           "stored_method": stored.method}),
        }])?;
        return Ok((
            Vec::new(),
            refuse(
                &request.id,
                method,
                OwnerFault::CommandReused,
                "This command id was already used for a different command.",
            ),
        ));
    }
    let id: Id = request.id.clone();
    let written = core.write(meta(operation_type, request), |tx, cx| {
        let result = execute(tx, cx, request)?;
        let frame = result_frame(id.clone(), result);
        store_answer(tx, request, &digest, &frame)?;
        Ok(frame)
    });
    match written {
        Ok(committed) => Ok((committed.notices, committed.value)),
        Err(TxError::Fault(fault)) => Err(fault),
        Err(TxError::Refused(refusal)) => {
            let frame = refusal_frame(&request.id, method, refusal);
            let stored = core.write::<_, Infallible>(meta("owner.refused", request), |tx, _| {
                store_answer(tx, request, &digest, &frame)?;
                Ok(())
            });
            match stored {
                Ok(_) => Ok((Vec::new(), frame)),
                Err(TxError::Fault(fault)) => Err(fault),
                Err(TxError::Refused(never)) => match never {},
            }
        }
    }
}
