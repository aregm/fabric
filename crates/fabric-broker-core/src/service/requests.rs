//! Request moves: one lifecycle step, its revision and its owner notice.

use fabric_desktop_contracts::agent::Action;
use fabric_desktop_contracts::common::{Addresses, CalendarRef, ToolName};
use fabric_desktop_contracts::owner::{
    Change, GrantEndReason, GrantTerm, LifecycleStatus, RecipientRule, Scope,
};
use fabric_desktop_contracts::scalar::{Digest, Id, List, PositiveCount, Task};
use fabric_desktop_contracts::{Domain, RequestState};
use fabric_storage::{Mutation, RequestRecord, StoreFault, WriteTx};

use crate::service::reducer::{Event, next};
use crate::service::{Cx, grants, invariant};

/// The request `request_id` as this transaction sees it.
pub(crate) fn record_of(tx: &WriteTx<'_>, request_id: &Id) -> Result<RequestRecord, StoreFault> {
    tx.read()
        .request(request_id)?
        .ok_or(invariant("the request exists"))
}

/// Moves the request to `to`; returns the request notice's sequence.
///
/// A move to a terminal state clears the request's due time and, in the same commit, ends every
/// live one-request grant whose origin is the request with `consumed` (D18); the grant notices
/// come before the request notice.
pub(crate) fn move_request(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    request_id: &Id,
    to: LifecycleStatus,
    due_at_ms: Option<i64>,
) -> Result<PositiveCount, StoreFault> {
    let record = record_of(tx, request_id)?;
    let from = record.row.request.status.state();
    let target = to.state();
    if !from.can_transition_to(target) {
        return Err(invariant("a request moves only along the lifecycle"));
    }
    let terminal = target.is_terminal();
    if terminal {
        let origin_grants: Vec<Id> = tx
            .read()
            .grants_of_agent(&record.row.request.agent_id)?
            .into_iter()
            .filter(|g| {
                g.ended.is_none()
                    && matches!(g.term, GrantTerm::OneRequest { .. })
                    && &g.origin.request_id == request_id
            })
            .map(|g| g.grant_id)
            .collect();
        for grant_id in origin_grants {
            let _ = grants::end_grant(tx, cx, &grant_id, GrantEndReason::Consumed)?;
        }
    }
    tx.apply(Mutation::RequestMoved {
        request_id: request_id.clone(),
        revision: record.row.request.revision.get() + 1,
        status: to,
        at_ms: cx.now_ms,
        due_at_ms: if terminal { None } else { due_at_ms },
    })?;
    let request = record_of(tx, request_id)?.row.request;
    tx.notify(Change::Request(request))
}

/// The status an owner's decision moves a request in `from` to: `approved` or `denied` from
/// `pending_approval` (5.7), or `None` from any other state. For `decide_request`.
#[must_use]
pub(crate) fn decided(from: RequestState, approve: bool) -> Option<LifecycleStatus> {
    next(from, Event::Decide { approve }).and_then(|to| to.plain().map(LifecycleStatus::Plain))
}

/// The account and calendars an action names; `None` for the list tool and a result request,
/// which name none.
#[must_use]
pub(crate) fn calendars_of(action: &Action) -> Option<(Id, Vec<Id>)> {
    match action {
        Action::CalendarList(_) | Action::RequestResult(_) => None,
        Action::CalendarAvailability(a) => Some((a.account_id.clone(), a.calendar_ids.to_vec())),
        Action::CalendarRead(a) => Some((a.account_id.clone(), a.calendar_ids.to_vec())),
        Action::ProposeCreate(a) => Some((a.account_id.clone(), vec![a.calendar_id.clone()])),
        Action::ProposeUpdate(a) => Some((a.account_id.clone(), vec![a.calendar_id.clone()])),
        Action::ProposeInvitation(a) => Some((a.account_id.clone(), vec![a.calendar_id.clone()])),
        Action::ProposeResponse(a) => Some((a.account_id.clone(), vec![a.calendar_id.clone()])),
        Action::ProposeDelete(a) => Some((a.account_id.clone(), vec![a.calendar_id.clone()])),
    }
}

/// The agent's words for the action; `None` for the list tool and a result request.
#[must_use]
pub(crate) fn task_of(action: &Action) -> Option<Task> {
    match action {
        Action::CalendarList(_) | Action::RequestResult(_) => None,
        Action::CalendarAvailability(a) => Some(a.task.clone()),
        Action::CalendarRead(a) => Some(a.task.clone()),
        Action::ProposeCreate(a) => Some(a.task.clone()),
        Action::ProposeUpdate(a) => Some(a.task.clone()),
        Action::ProposeInvitation(a) => Some(a.task.clone()),
        Action::ProposeResponse(a) => Some(a.task.clone()),
        Action::ProposeDelete(a) => Some(a.task.clone()),
    }
}

/// True for a tool that changes a calendar: only these are dispatched to the provider.
#[must_use]
pub(crate) fn is_change(tool: ToolName) -> bool {
    matches!(
        tool,
        ToolName::ProposeCreate
            | ToolName::ProposeUpdate
            | ToolName::ProposeInvitation
            | ToolName::ProposeResponse
            | ToolName::ProposeDelete
    )
}

/// The scope an action needs: its calendars and, where they apply, its window, fields,
/// recipients and who is notified. `None` for an action that names no calendar, which can never
/// be put to the owner on a card.
#[must_use]
pub(crate) fn scope_of(action: &Action) -> Option<Scope> {
    let (account_id, calendar_ids) = calendars_of(action)?;
    let calendars = List::new(
        calendar_ids
            .into_iter()
            .map(|calendar_id| CalendarRef {
                account_id: account_id.clone(),
                calendar_id,
            })
            .collect(),
    )
    .ok()?;
    let mut scope = Scope {
        calendars,
        window: None,
        fields: None,
        recipients: None,
        send_updates: None,
    };
    match action {
        Action::CalendarAvailability(a) => scope.window = Some(a.window.clone()),
        Action::CalendarRead(a) => {
            scope.window = Some(a.window.clone());
            scope.fields = Some(a.fields.clone());
        }
        Action::ProposeUpdate(a) => scope.send_updates = Some(a.send_updates),
        Action::ProposeInvitation(a) => {
            scope.recipients = Addresses::new(a.attendees.to_vec())
                .ok()
                .map(RecipientRule::Exact);
            scope.send_updates = Some(a.send_updates);
        }
        Action::ProposeResponse(a) => scope.send_updates = Some(a.send_updates),
        Action::ProposeDelete(a) => scope.send_updates = Some(a.send_updates),
        Action::CalendarList(_) | Action::RequestResult(_) | Action::ProposeCreate(_) => {}
    }
    Some(scope)
}

/// The request digest over the agent, the action and the scope it needs; a `null` scope for an
/// action that names no calendar.
#[must_use]
pub(crate) fn request_digest_of(agent_id: &Id, action: &Action, scope: Option<&Scope>) -> Digest {
    match scope {
        Some(scope) => fabric_desktop_contracts::request_digest(agent_id, action, scope),
        None => fabric_desktop_contracts::digest(
            Domain::Request,
            &serde_json::json!({"agent_id": agent_id, "action": action, "scope": null}),
        ),
    }
}
