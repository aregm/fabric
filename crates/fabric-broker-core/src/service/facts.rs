//! The trusted facts of one proof (10.3), read in one transaction plus the activity of this run.
//!
//! Decision L6 and reading 11.3 W2: `connected` holds every connected account and
//! `calendars` every selected calendar of those accounts, not only the ones the action names.

use fabric_desktop_contracts::RequestState;
use fabric_desktop_contracts::common::{CalendarAccess, CalendarRef};
use fabric_desktop_contracts::owner::{
    AccountState, Grant, GrantTerm, ProofPurpose, RecipientRule, RecipientSet,
};
use fabric_desktop_contracts::scalar::Id;
use fabric_storage::{OpenEffect, ReadTx, RequestRecord, StoreFault};

use crate::broker::PROOF_WATCHDOG_MS;
use crate::ports::{
    CalendarFact, FactActor, GrantFact, PendingEffect, ProofAction, ProofJob, SessionFact,
    TrustedFacts,
};
use crate::service::grants::grant_is_valid;
use crate::service::{Activity, epochs};

/// The grants a proof of `request` is offered (10.3): the agent's live grants, a one-request
/// grant only for its origin request, and every relied grant even when it ended.
#[must_use]
pub(crate) fn offered_grants(
    agents_grants: Vec<Grant>,
    request_id: &Id,
    relied: &[Id],
) -> Vec<Grant> {
    agents_grants
        .into_iter()
        .filter(|g| {
            if relied.contains(&g.grant_id) {
                return true;
            }
            g.ended.is_none()
                && match g.term {
                    GrantTerm::OneRequest { .. } => &g.origin.request_id == request_id,
                    GrantTerm::Session { .. } | GrantTerm::UntilRevoked => true,
                }
        })
        .collect()
}

/// `TrustedFacts.pending_effects` from `ReadTx::open_effects`.
#[must_use]
pub(crate) fn pending_effects(open: Vec<OpenEffect>) -> Vec<PendingEffect> {
    open.into_iter()
        .map(|o| PendingEffect {
            calendar: CalendarRef {
                account_id: o.effect.account_id,
                calendar_id: o.effect.calendar_id,
            },
            provider_event_id: o.event_id,
        })
        .collect()
}

/// The grant facts of the offered grants: `ended` when ended or failing the validity predicate
/// now, `used` once a one-request grant's change was applied.
pub(crate) fn grant_facts(
    r: &ReadTx<'_>,
    activity: &Activity,
    now_ms: i64,
    grants: Vec<Grant>,
) -> Result<Vec<GrantFact>, StoreFault> {
    let mut facts = Vec::with_capacity(grants.len());
    for grant in grants {
        let origin_state = r
            .request(&grant.origin.request_id)?
            .map(|o| o.row.request.status.state());
        let origin_open = origin_state.is_some_and(|s| !s.is_terminal());
        let used = matches!(grant.term, GrantTerm::OneRequest { .. })
            && matches!(
                origin_state,
                Some(RequestState::Applied | RequestState::Reconciled)
            );
        let session = activity.sessions.get(&grant.agent_id);
        let ended = !grant_is_valid(&grant, session, origin_open, now_ms);
        facts.push(GrantFact { grant, used, ended });
    }
    Ok(facts)
}

/// The current recipient sets the grants name, each once.
fn named_sets(r: &ReadTx<'_>, grants: &[GrantFact]) -> Result<Vec<RecipientSet>, StoreFault> {
    let mut ids: Vec<Id> = Vec::new();
    for fact in grants {
        if let Some(RecipientRule::RecipientSets(named)) = &fact.grant.scope.recipients {
            for id in named.iter() {
                if !ids.contains(id) {
                    ids.push(id.clone());
                }
            }
        }
    }
    let mut sets = Vec::with_capacity(ids.len());
    for id in &ids {
        if let Some(set) = r.recipient_set(id)? {
            sets.push(set);
        }
    }
    Ok(sets)
}

/// The proof job of `request` for `purpose`, its facts read in one transaction (`r`). The job id
/// is 0; the core numbers the job when it hands it out, after commit.
pub(crate) fn proof_job(
    r: &ReadTx<'_>,
    activity: &Activity,
    now_ms: i64,
    request: &RequestRecord,
    purpose: ProofPurpose,
) -> Result<ProofJob, StoreFault> {
    let agent_id = &request.row.request.agent_id;
    let request_id = &request.row.request.request_id;
    let agent = r
        .agent(agent_id)?
        .ok_or(crate::service::invariant("the agent exists"))?;
    let relied = match purpose {
        ProofPurpose::Classification => Vec::new(),
        ProofPurpose::FreshPermit => request.relied_grant_ids.clone(),
    };
    let offered = offered_grants(r.grants_of_agent(agent_id)?, request_id, &relied);
    let grants = grant_facts(r, activity, now_ms, offered)?;
    let recipient_sets = named_sets(r, &grants)?;
    let mut connected = Vec::new();
    let mut calendars = Vec::new();
    for account in r.accounts()? {
        if account.state != AccountState::Connected {
            continue;
        }
        for calendar in account.calendars.iter().filter(|c| c.selected) {
            calendars.push(CalendarFact {
                calendar: CalendarRef {
                    account_id: account.account_id.clone(),
                    calendar_id: calendar.calendar_id.clone(),
                },
                writable: matches!(
                    calendar.access,
                    CalendarAccess::Owner | CalendarAccess::Writer
                ),
                agent_drafts: calendar.agent_drafts,
            });
        }
        connected.push(account.account_id);
    }
    let counters = r.counters()?;
    let facts = TrustedFacts {
        actor: FactActor::Agent(agent_id.clone()),
        paused: agent.paused,
        now_ms,
        session: activity.sessions.get(agent_id).map(|s| SessionFact {
            session_id: s.session_id.clone(),
            last_active_ms: s.last_active_ms,
        }),
        profile: r.active_profile()?.2,
        connected,
        calendars,
        warning_calendars: r.preferences()?.settings.warning_calendars.to_vec(),
        grants,
        recipient_sets,
        event: None,
        relied,
        result_held: false,
        pending_effects: pending_effects(r.open_effects()?),
        pause_epoch: counters.pause_epoch,
        revocation_epoch: counters.revocation_epoch,
    };
    let started_at_ms = u64::try_from(now_ms).unwrap_or(0);
    Ok(ProofJob {
        job_id: 0,
        request_id: request_id.clone(),
        purpose,
        action: ProofAction::Agent(request.row.action.clone()),
        action_digest: request.row.action.digest(),
        facts,
        fence: epochs::fence(counters),
        started_at_ms,
        deadline_ms: started_at_ms + PROOF_WATCHDOG_MS,
    })
}

#[cfg(test)]
mod tests {
    use fabric_desktop_contracts::owner::Grant;
    use fabric_storage::EffectRow;
    use serde_json::json;

    use super::*;

    fn grant(grant_id: &str, term: serde_json::Value, origin: &str, ended: bool) -> Grant {
        let ended =
            ended.then(|| json!({"at": "2026-11-20T16:00:00.000Z", "reason": "revoked_by_owner"}));
        fabric_desktop_contracts::decode_record(
            serde_json::to_string(&json!({
                "grant_id": grant_id,
                "revision": if ended.is_some() { 2 } else { 1 },
                "agent_id": "agent-a",
                "purpose": "Plan the week.",
                "tools": ["fabric_calendar_propose_create"],
                "scope": {
                    "calendars": [{"account_id": "acct-1", "calendar_id": "cal-1"}],
                    "window": null, "fields": null, "recipients": null, "send_updates": null
                },
                "term": term,
                "origin": {"request_id": origin, "approval_digest": format!("sha256:{}", "c".repeat(64))},
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

    fn id(text: &str) -> Id {
        Id::new(text).unwrap()
    }

    fn ids(grants: &[Grant]) -> Vec<&str> {
        grants.iter().map(|g| g.grant_id.as_str()).collect()
    }

    fn one_request() -> serde_json::Value {
        json!({"kind": "one_request", "action_digest": format!("sha256:{}", "d".repeat(64))})
    }

    fn until_revoked() -> serde_json::Value {
        json!({"kind": "until_revoked", "expires_at": null})
    }

    #[test]
    fn a_one_request_grant_is_offered_only_to_its_origin_request() {
        let grants = vec![
            grant("grant-own", one_request(), "request-1", false),
            grant("grant-other", one_request(), "request-2", false),
            grant("grant-kept", until_revoked(), "request-2", false),
        ];
        assert_eq!(
            ids(&offered_grants(grants.clone(), &id("request-1"), &[])),
            ["grant-own", "grant-kept"]
        );
        assert_eq!(
            ids(&offered_grants(grants, &id("request-2"), &[])),
            ["grant-other", "grant-kept"]
        );
    }

    #[test]
    fn a_classification_is_offered_no_ended_grant() {
        let grants = vec![
            grant("grant-live", until_revoked(), "request-1", false),
            grant("grant-ended", until_revoked(), "request-1", true),
        ];
        assert_eq!(
            ids(&offered_grants(grants, &id("request-1"), &[])),
            ["grant-live"]
        );
    }

    #[test]
    fn a_fresh_proof_is_offered_every_relied_grant_even_ended() {
        let grants = vec![
            grant("grant-live", until_revoked(), "request-1", false),
            grant("grant-relied", until_revoked(), "request-1", true),
            grant("grant-other-ended", until_revoked(), "request-1", true),
        ];
        assert_eq!(
            ids(&offered_grants(
                grants,
                &id("request-1"),
                &[id("grant-relied")]
            )),
            ["grant-live", "grant-relied"]
        );
    }

    fn effect(tool: &str, provider_event_id: Option<&str>) -> EffectRow {
        serde_json::from_value(json!({
            "effect_id": "effect-1",
            "request_id": "request-1",
            "tool": tool,
            "account_id": "acct-1",
            "calendar_id": "cal-1",
            "action_digest": format!("sha256:{}", "d".repeat(64)),
            "provider_event_id": provider_event_id,
            "created_at_ms": 1,
            "attempts_started": 1,
            "first_attempt_at_ms": 1
        }))
        .unwrap()
    }

    #[test]
    fn a_pending_effect_names_its_calendar_and_its_target_event() {
        // A create targets its reserved id; every other tool the action's event or occurrence,
        // which `ReadTx::open_effects` resolves into `event_id`.
        let open = vec![
            OpenEffect {
                effect: effect("fabric_calendar_propose_create", Some("reserved0id")),
                event_id: "reserved0id".to_owned(),
            },
            OpenEffect {
                effect: effect("fabric_calendar_propose_update", None),
                event_id: "event-7".to_owned(),
            },
            OpenEffect {
                effect: effect("fabric_calendar_propose_response", None),
                event_id: "instance-3".to_owned(),
            },
        ];
        let pending = pending_effects(open);
        let events: Vec<&str> = pending
            .iter()
            .map(|p| p.provider_event_id.as_str())
            .collect();
        assert_eq!(events, ["reserved0id", "event-7", "instance-3"]);
        assert!(pending.iter().all(|p| p.calendar
            == CalendarRef {
                account_id: id("acct-1"),
                calendar_id: id("cal-1")
            }));
    }
}
