//! Recovery at open (5.8): the one transaction that closes what the last run left open.
//!
//! It runs in every `Core::open`, at start and at every reopen, as one `broker.started`
//! transaction (idempotency `startup`/run id): a new run; every live session ends with its
//! session grants (`broker_restart`); every issued authorization is voided; `queued` goes back
//! to a fresh proof; `dispatching` becomes `outcome_unknown` with its open attempt finished so;
//! every request the pipeline drives is due now. Quit is the matching transaction at the end of
//! input.

use fabric_desktop_contracts::RequestState;
use fabric_desktop_contracts::owner::AttemptOutcome;
use fabric_storage::{
    Actor, IdempotencyScope, Mutation, SessionEndReason, StoreFault, VoidReason, WriteTx,
};

use crate::service::core::Core;
use crate::service::requests::move_request;
use crate::service::sessions::end_session;
use crate::service::{Cx, plain};

/// The recovery's changes, inside its transaction.
fn recover(tx: &mut WriteTx<'_>, cx: &mut Cx<'_>) -> Result<(), StoreFault> {
    let now = cx.now_ms;
    tx.apply(Mutation::RunStarted {
        run_id: cx.run_id,
        at_ms: now,
    })?;
    for session in tx.read().live_sessions()? {
        end_session(tx, cx, &session.session_id, SessionEndReason::BrokerRestart)?;
    }
    for authorization in tx.read().issued_authorizations()? {
        tx.apply(Mutation::AuthorizationVoided {
            authorization_id: authorization.authorization.authorization_id,
            at_ms: now,
            reason: VoidReason::BrokerRestart,
        })?;
    }
    for record in tx.read().open_requests()? {
        let request_id = record.row.request.request_id.clone();
        match record.row.request.status.state() {
            RequestState::Queued => {
                move_request(
                    tx,
                    cx,
                    &request_id,
                    plain(RequestState::ProvingFreshPermit)?,
                    Some(now),
                )?;
            }
            RequestState::Dispatching => {
                if let Some(effect) = tx.read().effect_of(&request_id)? {
                    let open = tx
                        .read()
                        .attempts(&effect.effect_id)?
                        .last()
                        .is_some_and(|a| a.attempt.outcome.is_none());
                    if open {
                        tx.apply(Mutation::AttemptFinished {
                            effect_id: effect.effect_id,
                            number: effect.attempts_started,
                            finished_at_ms: now,
                            outcome: AttemptOutcome::OutcomeUnknown,
                        })?;
                    }
                }
                move_request(
                    tx,
                    cx,
                    &request_id,
                    plain(RequestState::OutcomeUnknown)?,
                    None,
                )?;
            }
            RequestState::Received
            | RequestState::Validating
            | RequestState::ProvingClassification
            | RequestState::ProvingFreshPermit
            | RequestState::Allowed
            | RequestState::Approved => {
                tx.apply(Mutation::RequestScheduled {
                    request_id,
                    due_at_ms: Some(now),
                    reproofs: record.reproofs,
                })?;
            }
            _ => {}
        }
    }
    Ok(())
}

impl Core {
    /// The recovery transaction of 5.8.
    pub(in crate::service) fn recover(&mut self) -> Result<(), StoreFault> {
        let mut meta = self.meta("broker.started", Actor::broker(), None, None);
        meta.idempotency = Some((IdempotencyScope::Startup, self.run_id.to_string()));
        self.commit(meta, recover)
    }

    /// End of input (5.8): one `broker.quit` transaction ends the live sessions and their
    /// session grants (`app_quit`), writes the open `busy` windows and ends the run cleanly. No
    /// job begins after it; `dispatching` effects are left for the next start.
    pub(crate) fn quit(&mut self) -> Result<(), StoreFault> {
        let windows = self.busy.take(self.now, true);
        let run_id = self.run_id;
        let meta = self.meta("broker.quit", Actor::broker(), None, None);
        self.commit(meta, |tx, cx| {
            for session in tx.read().live_sessions()? {
                end_session(tx, cx, &session.session_id, SessionEndReason::AppQuit)?;
            }
            for entry in windows {
                tx.audit(entry)?;
            }
            tx.apply(Mutation::RunEnded {
                run_id,
                at_ms: cx.now_ms,
                clean: true,
            })
        })?;
        let open: Vec<_> = self.connections.drain().map(|(c, _)| c).collect();
        self.outbox.close.extend(open);
        self.in_flight.clear();
        Ok(())
    }
}
