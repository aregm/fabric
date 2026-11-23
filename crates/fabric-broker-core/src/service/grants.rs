//! Grants: creation, ending, and the validity predicate (5.5).

use fabric_desktop_contracts::error::{InvalidParams, OwnerFault};
use fabric_desktop_contracts::owner::{
    Change, Duration, DurationKind, Grant, GrantEnd, GrantEndReason, GrantOrigin, GrantTerm,
    PolicyVersion, RecipientRule, Scope,
};
use fabric_desktop_contracts::scalar::{Count, Digest, Id, List, PositiveCount, Text};
use fabric_storage::{Counter, Mutation, RequestRecord, StoreFault, TxError, WriteTx};

use crate::broker::SESSION_IDLE_MS;
use crate::service::requests::task_of;
use crate::service::{Cx, LiveSession, OwnerRefusal, epochs, invariant, timestamp};

/// Creates the grant an approval asks for; returns it with its notice's sequence.
///
/// A `session` duration binds the agent's current live session, which may be later than the
/// request's (5.5); with none in this run the duration is refused at `/params/duration` (D22).
pub(crate) fn create_grant(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    request: &RequestRecord,
    scope: &Scope,
    duration: Duration,
    approval_digest: &Digest,
) -> Result<(Grant, PositiveCount), TxError<OwnerRefusal>> {
    let agent_id = &request.row.request.agent_id;
    let term = match duration.kind {
        DurationKind::OneRequest => GrantTerm::OneRequest {
            action_digest: request.row.action.digest(),
        },
        DurationKind::Session => match cx.activity.sessions.get(agent_id) {
            Some(session) => GrantTerm::Session {
                session_id: session.session_id.clone(),
            },
            None => {
                return Err(TxError::Refused(OwnerRefusal {
                    fault: OwnerFault::InvalidParams(InvalidParams {
                        pointer: Text::new("/params/duration")
                            .map_err(|_| invariant("a pointer is short text"))?,
                    }),
                    message: "The agent has no live session, so a session grant cannot be bound \
                              to one. Approve it for one request or until revoked."
                        .to_owned(),
                }));
            }
        },
        DurationKind::UntilRevoked => GrantTerm::UntilRevoked,
    };
    let purpose =
        task_of(&request.row.action).ok_or(invariant("an approved request has a task"))?;
    let (profile_id, version, _) = tx.read().active_profile()?;
    let revocation_epoch = tx.read().counters()?.revocation_epoch;
    let grant = Grant {
        grant_id: crate::ids::new_id("grant"),
        revision: PositiveCount::of(1),
        agent_id: agent_id.clone(),
        purpose,
        tools: List::new(vec![request.row.action.tool()])
            .map_err(|_| invariant("a grant names one tool"))?,
        scope: scope.clone(),
        term,
        origin: GrantOrigin::Approval {
            request_id: request.row.request.request_id.clone(),
            approval_digest: approval_digest.clone(),
        },
        policy: PolicyVersion {
            profile_id,
            version: PositiveCount::new(version)
                .map_err(|_| invariant("a profile version is positive"))?,
        },
        revocation_epoch: Count::new(revocation_epoch)
            .map_err(|_| invariant("an epoch is a safe integer"))?,
        created_at: timestamp(cx.now_ms),
        ended: None,
    };
    tx.apply(Mutation::GrantInserted {
        grant: grant.clone(),
    })?;
    let sequence = tx.notify(Change::Grant(grant.clone()))?;
    Ok((grant, sequence))
}

/// Ends one grant, raises revocation_epoch, emits the grant notice. None when already ended.
pub(crate) fn end_grant(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    grant_id: &Id,
    reason: GrantEndReason,
) -> Result<Option<(Grant, PositiveCount)>, StoreFault> {
    let grant = tx
        .read()
        .grant(grant_id)?
        .ok_or(invariant("only a known grant ends"))?;
    if grant.ended.is_some() {
        return Ok(None);
    }
    tx.apply(Mutation::GrantEnded {
        grant_id: grant_id.clone(),
        revision: grant.revision.get() + 1,
        end: GrantEnd {
            at: timestamp(cx.now_ms),
            reason,
        },
    })?;
    epochs::raise(tx, Counter::RevocationEpoch)?;
    let ended = tx
        .read()
        .grant(grant_id)?
        .ok_or(invariant("an ended grant stays"))?;
    let sequence = tx.notify(Change::Grant(ended.clone()))?;
    Ok(Some((ended, sequence)))
}

/// True when the grant's scope names the recipient set.
fn names_set(grant: &Grant, recipient_set_id: &Id) -> bool {
    matches!(&grant.scope.recipients, Some(RecipientRule::RecipientSets(ids)) if ids.contains(recipient_set_id))
}

/// True when the grant's scope names a calendar of the account.
fn names_account(grant: &Grant, account_id: &Id) -> bool {
    grant
        .scope
        .calendars
        .iter()
        .any(|c| &c.account_id == account_id)
}

/// Ends every live grant that names the recipient set.
pub(crate) fn end_grants_naming_set(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    recipient_set_id: &Id,
) -> Result<Vec<PositiveCount>, StoreFault> {
    let named: Vec<Id> = tx
        .read()
        .live_grants()?
        .into_iter()
        .filter(|g| names_set(g, recipient_set_id))
        .map(|g| g.grant_id)
        .collect();
    let mut sequences = Vec::with_capacity(named.len());
    for grant_id in named {
        if let Some((_, sequence)) = end_grant(tx, cx, &grant_id, GrantEndReason::AuthorityReduced)?
        {
            sequences.push(sequence);
        }
    }
    Ok(sequences)
}

/// Ends every live grant on the account.
pub(crate) fn end_grants_of_account(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    account_id: &Id,
) -> Result<Vec<(Id, PositiveCount)>, StoreFault> {
    let named: Vec<Id> = tx
        .read()
        .live_grants()?
        .into_iter()
        .filter(|g| names_account(g, account_id))
        .map(|g| g.grant_id)
        .collect();
    let mut ended = Vec::with_capacity(named.len());
    for grant_id in named {
        if let Some((_, sequence)) =
            end_grant(tx, cx, &grant_id, GrantEndReason::AccountDisconnected)?
        {
            ended.push((grant_id, sequence));
        }
    }
    Ok(ended)
}

/// 5.5. `session`: the agent's live session from `Activity` (None when the agent has none in this run).
///
/// `origin_open` is true while the grant's origin request is not terminal; the caller offers a
/// one-request grant only to its origin request.
#[must_use]
pub(crate) fn grant_is_valid(
    grant: &Grant,
    session: Option<&LiveSession>,
    origin_open: bool,
    now_ms: i64,
) -> bool {
    if grant.ended.is_some() {
        return false;
    }
    match &grant.term {
        GrantTerm::OneRequest { .. } => origin_open,
        GrantTerm::Session { session_id } => session.is_some_and(|live| {
            &live.session_id == session_id
                && now_ms.saturating_sub(live.last_active_ms) <= SESSION_IDLE_MS
        }),
        GrantTerm::UntilRevoked => true,
    }
}

#[cfg(test)]
mod tests {
    use fabric_desktop_contracts::owner::Grant;
    use serde_json::json;

    use super::*;

    const T0: i64 = 1_795_190_400_000;

    fn grant(term: serde_json::Value, ended: Option<&str>) -> Grant {
        let ended = ended.map(|reason| json!({"at": "2026-11-20T16:00:00.000Z", "reason": reason}));
        fabric_desktop_contracts::decode_record(
            serde_json::to_string(&json!({
                "grant_id": "grant-1",
                "revision": if ended.is_some() { 2 } else { 1 },
                "agent_id": "agent-a",
                "purpose": "Plan the week.",
                "tools": ["fabric_calendar_propose_create"],
                "scope": {
                    "calendars": [{"account_id": "acct-1", "calendar_id": "cal-1"}],
                    "window": null, "fields": null,
                    "recipients": {"kind": "recipient_sets", "recipient_set_ids": ["set-1"]},
                    "send_updates": null
                },
                "term": term,
                "origin": {"request_id": "request-1", "approval_digest": format!("sha256:{}", "c".repeat(64))},
                "policy": {"profile_id": "careful", "version": 1},
                "revocation_epoch": 0,
                "created_at": "2026-11-20T16:00:00.000Z",
                "ended": ended
            }))
            .unwrap()
            .as_bytes(),
        )
        .unwrap()
    }

    fn one_request() -> serde_json::Value {
        json!({"kind": "one_request", "action_digest": format!("sha256:{}", "d".repeat(64))})
    }

    fn session_term(id: &str) -> serde_json::Value {
        json!({"kind": "session", "session_id": id})
    }

    fn until_revoked() -> serde_json::Value {
        json!({"kind": "until_revoked", "expires_at": null})
    }

    fn live(session_id: &str, last_active_ms: i64) -> LiveSession {
        LiveSession {
            session_id: Id::new(session_id).unwrap(),
            started_at_ms: T0,
            last_active_ms,
        }
    }

    const END_REASONS: [&str; 9] = [
        "consumed",
        "revoked_by_owner",
        "account_disconnected",
        "authority_reduced",
        "agent_removed",
        "session_disconnected",
        "session_idle_timeout",
        "app_quit",
        "broker_restart",
    ];

    #[test]
    fn an_ended_grant_of_any_term_is_never_valid() {
        let session = live("session-1", T0);
        for term in [one_request(), session_term("session-1"), until_revoked()] {
            for reason in END_REASONS {
                let ended = grant(term.clone(), Some(reason));
                assert!(
                    !grant_is_valid(&ended, Some(&session), true, T0),
                    "{term} ended {reason}"
                );
            }
        }
    }

    #[test]
    fn a_one_request_grant_is_valid_only_while_its_origin_is_open() {
        let g = grant(one_request(), None);
        assert!(grant_is_valid(&g, None, true, T0));
        assert!(!grant_is_valid(&g, None, false, T0));
    }

    #[test]
    fn an_until_revoked_grant_is_valid_until_it_ends() {
        let g = grant(until_revoked(), None);
        assert!(grant_is_valid(&g, None, false, T0 + 10 * SESSION_IDLE_MS));
    }

    #[test]
    fn a_session_grant_needs_its_session_live_in_this_run() {
        let g = grant(session_term("session-1"), None);
        // An earlier run's session is never in memory.
        assert!(!grant_is_valid(&g, None, true, T0));
        // Another session of the agent does not carry it.
        assert!(!grant_is_valid(&g, Some(&live("session-2", T0)), true, T0));
        assert!(grant_is_valid(&g, Some(&live("session-1", T0)), true, T0));
    }

    #[test]
    fn a_session_grant_idles_after_1800000_ms_of_activity() {
        let g = grant(session_term("session-1"), None);
        let last_call = T0 + 5_000;
        let session = live("session-1", last_call);
        assert!(grant_is_valid(
            &g,
            Some(&session),
            true,
            last_call + 1_800_000
        ));
        assert!(!grant_is_valid(
            &g,
            Some(&session),
            true,
            last_call + 1_800_001
        ));
    }

    #[test]
    fn a_session_with_no_call_idles_1800000_ms_after_its_connect() {
        let g = grant(session_term("session-1"), None);
        // `last_active_ms` starts at the connect.
        let session = live("session-1", T0);
        assert!(grant_is_valid(&g, Some(&session), true, T0 + 1_800_000));
        assert!(!grant_is_valid(&g, Some(&session), true, T0 + 1_800_001));
    }

    #[test]
    fn a_grant_names_its_sets_and_its_accounts() {
        let g = grant(until_revoked(), None);
        assert!(names_set(&g, &Id::new("set-1").unwrap()));
        assert!(!names_set(&g, &Id::new("set-2").unwrap()));
        assert!(names_account(&g, &Id::new("acct-1").unwrap()));
        assert!(!names_account(&g, &Id::new("acct-2").unwrap()));
    }
}
