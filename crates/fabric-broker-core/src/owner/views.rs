//! The read-only views: `get_snapshot`, `get_request_details` and `get_proof` (design 6.1).
//! `hello` is in `frames`, since it needs no store.
//!
//! Each view is one read of the store. A full-detail read is answered only after its audit row
//! committed (PLAN 11 para 5).

use fabric_desktop_contracts::error::OwnerFault;
use fabric_desktop_contracts::owner::{
    GetProofParams, GetRequestDetailsParams, OwnerMethod, OwnerResult, PrivateDetails, Redaction,
    RequestDetails, Snapshot,
};
use fabric_desktop_contracts::scalar::{Count, Id, List};
use fabric_desktop_contracts::{Frame, canonical};
use fabric_storage::{Actor, AuditEntry, ProverKind, ReadTx, StoreFault};
use serde_json::json;

use super::frames::{refuse, result_frame};
use crate::service::core::Core;
use crate::service::sessions::agent_view;
use crate::service::{Activity, invariant};

/// The most proof ids a request's details list.
const MAX_PROOF_IDS: usize = 200;
/// The most attempts a request's details list.
const MAX_ATTEMPTS: usize = 20;
/// The audit kind of a full-detail read.
pub(crate) const FULL_DETAILS_AUDIT: &str = "request_details_full";
/// The most bytes the owner's own records may take in a snapshot, in canonical form: the
/// recipient sets, the profiles and the live grants (README section 1, plan 9.1). One recipient
/// set of the schema's maximum (2000 addresses of 320 bytes and 50 domains of 253) fits; what is
/// left of the 1,048,576-byte frame bound holds the open requests (at most 100), the agents,
/// the accounts, the health and the preferences. The profiles notice is part of it.
pub(crate) const OWNER_STATE_BUDGET_BYTES: usize = 720 * 1024;

/// `get_snapshot`: the whole state the page shows, in one read.
pub(crate) fn snapshot(core: &Core, frame_id: Id) -> Result<Frame, StoreFault> {
    let health = core.health();
    let activity = core.activity();
    let snapshot = core.read(|r| read_snapshot(r, activity, health))?;
    Ok(result_frame(frame_id, OwnerResult::GetSnapshot(snapshot)))
}

/// The snapshot as one read sees it.
fn read_snapshot(
    r: &ReadTx<'_>,
    activity: &Activity,
    health: fabric_desktop_contracts::owner::Health,
) -> Result<Snapshot, StoreFault> {
    let mut agents = Vec::new();
    for row in r.agents()? {
        agents.push(agent_view(r, activity, &row.agent_id)?.ok_or(invariant("the agent exists"))?);
    }
    let requests = r
        .open_requests()?
        .into_iter()
        .map(|record| record.row.request)
        .collect();
    Ok(Snapshot {
        cursor: Count::new(r.meta()?.notice_sequence)
            .map_err(|_| invariant("a notice sequence is a safe integer"))?,
        health,
        profiles: r.profiles()?,
        accounts: any_list(r.accounts()?)?,
        agents: any_list(agents)?,
        enrollments: any_list(Vec::new())?,
        recipient_sets: any_list(r.recipient_sets()?)?,
        preferences: r.preferences()?,
        requests: any_list(requests)?,
        grants: any_list(r.live_grants()?)?,
    })
}

/// The bytes the owner's own records take in a snapshot (see [`OWNER_STATE_BUDGET_BYTES`]).
pub(crate) fn owner_state_bytes(r: &ReadTx<'_>) -> Result<usize, StoreFault> {
    Ok(encoded_len(&r.recipient_sets()?)?
        + encoded_len(&r.profiles()?)?
        + encoded_len(&r.live_grants()?)?)
}

/// The length of a value's canonical form.
fn encoded_len<T: serde::Serialize>(value: &T) -> Result<usize, StoreFault> {
    serde_json::to_value(value)
        .map(|value| canonical(&value).len())
        .map_err(|_| invariant("a record serializes"))
}

/// A list with no bounds.
fn any_list<T: serde::Serialize>(
    items: Vec<T>,
) -> Result<List<T, 0, { usize::MAX }, false>, StoreFault> {
    List::new(items).map_err(|_| invariant("any list"))
}

/// `get_request_details`: the request, its card, approval, Isabelle proof ids (newest first),
/// latest authorization and attempts. `full` adds the action after its audit row commits.
pub(crate) fn details(
    core: &mut Core,
    frame_id: Id,
    p: &GetRequestDetailsParams,
) -> Result<Frame, StoreFault> {
    let found = core.read(|r| read_details(r, p))?;
    let Some(mut details) = found else {
        return Ok(refuse(
            &frame_id,
            OwnerMethod::GetRequestDetails,
            OwnerFault::NotFound,
            "The broker has no request with this id.",
        ));
    };
    if p.redaction == Redaction::Full {
        core.write_audit(vec![AuditEntry {
            kind: FULL_DETAILS_AUDIT,
            actor: Actor::owner(),
            request_id: Some(p.request_id.clone()),
            detail: json!({"frame_id": frame_id}),
        }])?;
    } else {
        details.private = None;
    }
    Ok(result_frame(
        frame_id,
        OwnerResult::GetRequestDetails(details),
    ))
}

/// The details of a request as one read sees them, with its private part; `None` when unknown.
fn read_details(
    r: &ReadTx<'_>,
    p: &GetRequestDetailsParams,
) -> Result<Option<RequestDetails>, StoreFault> {
    let Some(record) = r.request(&p.request_id)? else {
        return Ok(None);
    };
    let proof_ids = r.proof_ids_of(&p.request_id, ProverKind::Isabelle, MAX_PROOF_IDS)?;
    let authorization = r
        .latest_authorization_of(&p.request_id)?
        .map(|a| a.authorization);
    let attempts = match r.effect_of(&p.request_id)? {
        Some(effect) => r
            .attempts(&effect.effect_id)?
            .into_iter()
            .take(MAX_ATTEMPTS)
            .map(|a| a.attempt)
            .collect(),
        None => Vec::new(),
    };
    Ok(Some(RequestDetails {
        request: record.row.request,
        redaction: p.redaction,
        private: Some(PrivateDetails {
            action: record.row.action,
            // W04 observes no event (CD-W05) and checks no warning (CD-W09).
            before: None,
            warnings: List::new(Vec::new()).map_err(|_| invariant("no warnings"))?,
        }),
        card: record.card,
        approval: record.approval,
        proof_ids: List::new(proof_ids).map_err(|_| invariant("distinct proof ids"))?,
        authorization,
        attempts: List::new(attempts).map_err(|_| invariant("at most 20 attempts"))?,
        provider_event: None,
        // The broker does not map its recorded reason to the contract's list yet.
        undecided_reason: None,
    }))
}

/// `get_proof`: the newest report with the id; `not_found` for none and for a synthetic report,
/// which is no live Isabelle check (CCR-10).
pub(crate) fn proof(core: &Core, frame_id: Id, p: &GetProofParams) -> Result<Frame, StoreFault> {
    let found = core.read(|r| r.proof(&p.proof_id))?;
    Ok(match found {
        Some(record) if record.prover == ProverKind::Isabelle => {
            result_frame(frame_id, OwnerResult::GetProof(record.report))
        }
        _ => refuse(
            &frame_id,
            OwnerMethod::GetProof,
            OwnerFault::NotFound,
            "The broker has no Isabelle proof report with this id.",
        ),
    })
}
