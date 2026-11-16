//! The agent inputs: connects, calls (proposals and `fabric_request_status`) and disconnects.
//!
//! The adapter authenticates an agent and hands its frames over as [`AgentInput`]s (CD-W06).
//! A connect of an enrolled agent opens a session (5.5); a call on a connection whose session
//! is the agent's live one is activity (D35); a proposal inserts a request in the only
//! transaction with `agent_call` idempotency, after the limits (5.9); a repeated call returns
//! its request and writes nothing (D25).

use fabric_desktop_contracts::Frame;
use fabric_desktop_contracts::RequestState;
use fabric_desktop_contracts::agent::{
    Action, AgentCall, AgentDenial, AgentLifecycleStatus, AgentRequest, AgentResponse,
    RequestStatus, ToolOutcome,
};
use fabric_desktop_contracts::common::ToolName;
use fabric_desktop_contracts::error::{AgentError, AgentFault, InvalidParams};
use fabric_desktop_contracts::owner::{Change, LifecycleStatus, Request};
use fabric_desktop_contracts::scalar::{Id, MethodText, PositiveCount, Text};
use fabric_storage::{
    Actor, AuditEntry, IdempotencyScope, Mutation, RequestRow, StoreFault, TxError,
};

use crate::ports::{AgentInput, AgentReply, ConnectionId};
use crate::service::core::{Connection, Core};
use crate::service::limits::{Load, busy};
use crate::service::requests::{record_of, request_digest_of, scope_of};
use crate::service::sessions::{end_session, start_session};
use crate::service::{invariant, plain, timestamp};

/// The agent's view of a request's status: the card's expiry, the bounded denial, the failure
/// code, or the state alone (CR 8).
#[must_use]
pub(crate) fn agent_status(status: &LifecycleStatus) -> AgentLifecycleStatus {
    match status {
        LifecycleStatus::PendingApproval {
            approval_expires_at,
        } => AgentLifecycleStatus::PendingApproval {
            approval_expires_at: approval_expires_at.clone(),
        },
        LifecycleStatus::Blocked { denial } => AgentLifecycleStatus::Blocked {
            denial: AgentDenial {
                rule: denial.rule.clone(),
                explanation: denial.explanation.clone(),
            },
        },
        LifecycleStatus::TerminalFailure { failure } => AgentLifecycleStatus::TerminalFailure {
            failure_code: failure.code,
        },
        LifecycleStatus::Plain(state) => AgentLifecycleStatus::Plain(*state),
    }
}

/// The answer to a call: the request's status and no result (W04 holds no results, D33).
#[must_use]
pub(crate) fn status_reply(
    connection: ConnectionId,
    call_id: Id,
    tool: ToolName,
    request: &Request,
) -> AgentReply {
    AgentReply {
        connection,
        frame: Frame::AgentResponse(AgentResponse::Result {
            id: call_id,
            method: tool,
            result: ToolOutcome {
                request: RequestStatus {
                    request_id: request.request_id.clone(),
                    updated_at: request.updated_at.clone(),
                    status: agent_status(&request.status),
                },
                result: None,
            },
        }),
    }
}

/// A refusal of a call.
#[must_use]
pub(crate) fn error_reply(
    connection: ConnectionId,
    call_id: Id,
    tool: ToolName,
    fault: AgentFault,
    message: &str,
) -> AgentReply {
    AgentReply {
        connection,
        frame: Frame::AgentResponse(AgentResponse::Error {
            id: Some(call_id),
            method: MethodText::new(tool.as_str()).ok(),
            error: AgentError {
                fault,
                message: Text::new(message)
                    .unwrap_or_else(|_| Text::new("The call was refused.").expect("a message")),
                correlation_id: None,
            },
        }),
    }
}

impl Core {
    /// One input of the agent adapter.
    pub(crate) fn agent(&mut self, input: AgentInput) -> Result<(), StoreFault> {
        match input {
            AgentInput::Connect {
                connection,
                agent_id,
            } => self.connect(connection, agent_id),
            AgentInput::Call {
                connection,
                request,
            } => self.call(connection, request),
            AgentInput::Disconnect { connection } => self.disconnect(connection),
        }
    }

    /// A connect: a session for an enrolled agent (one commit, `agent.connect`); for an unknown
    /// one the connection is closed and one audit row written, nothing else (5.5).
    fn connect(&mut self, connection: ConnectionId, agent_id: Id) -> Result<(), StoreFault> {
        if self.store.read(|r| r.agent(&agent_id))?.is_none() {
            self.outbox.close.push(connection);
            return self.write_audit(vec![AuditEntry {
                kind: "unknown_agent",
                actor: Actor::broker(),
                request_id: None,
                detail: serde_json::json!({"agent_id": agent_id, "connection": connection.0}),
            }]);
        }
        let meta = self.meta("agent.connect", Actor::agent(agent_id.clone()), None, None);
        let session = self.commit(meta, |tx, cx| start_session(tx, cx, &agent_id))?;
        self.connections.insert(
            connection,
            Connection {
                agent_id,
                session_id: session.session_id,
            },
        );
        Ok(())
    }

    /// A disconnect ends the connection's session when it is the agent's live one
    /// (`agent.disconnect`).
    fn disconnect(&mut self, connection: ConnectionId) -> Result<(), StoreFault> {
        let Some(bound) = self.connections.remove(&connection) else {
            return Ok(());
        };
        let live = self
            .activity
            .sessions
            .get(&bound.agent_id)
            .is_some_and(|s| s.session_id == bound.session_id);
        if !live {
            return Ok(());
        }
        let meta = self.meta(
            "agent.disconnect",
            Actor::agent(bound.agent_id.clone()),
            None,
            None,
        );
        self.commit(meta, |tx, cx| {
            end_session(
                tx,
                cx,
                &bound.session_id,
                fabric_storage::SessionEndReason::Disconnected,
            )
            .map(|_| ())
        })
    }

    /// One call. A connection with no live session of its agent is `not_permitted` (audited);
    /// every other call is activity, then a status read or a proposal.
    fn call(&mut self, connection: ConnectionId, request: AgentRequest) -> Result<(), StoreFault> {
        let tool = request.call.tool();
        let bound = self.connections.get(&connection).cloned().filter(|c| {
            self.activity
                .sessions
                .get(&c.agent_id)
                .is_some_and(|s| s.session_id == c.session_id)
        });
        let Some(bound) = bound else {
            self.outbox.agent.push(error_reply(
                connection,
                request.id.clone(),
                tool,
                AgentFault::NotPermitted,
                "This connection has no live session of an enrolled agent.",
            ));
            return self.write_audit(vec![AuditEntry {
                kind: "not_permitted",
                actor: Actor::broker(),
                request_id: None,
                detail: serde_json::json!({
                    "connection": connection.0,
                    "call_id": request.id,
                    "tool": tool,
                }),
            }]);
        };
        self.activity.record_call(&bound.agent_id, self.now);
        match request.call {
            AgentCall::RequestStatus(arguments) => {
                let found = self.store.read(|r| r.request(&arguments.request_id))?;
                let reply = match found {
                    Some(record) if record.row.request.agent_id == bound.agent_id => {
                        status_reply(connection, request.id, tool, &record.row.request)
                    }
                    _ => error_reply(
                        connection,
                        request.id,
                        tool,
                        AgentFault::NotFound,
                        "No request of this agent has that id.",
                    ),
                };
                self.outbox.agent.push(reply);
                Ok(())
            }
            AgentCall::Action(action) => self.propose(connection, &bound, request.id, action),
        }
    }

    /// A proposal (5.7 `(new) -> received`, 5.9, D25).
    fn propose(
        &mut self,
        connection: ConnectionId,
        bound: &Connection,
        call_id: Id,
        action: Action,
    ) -> Result<(), StoreFault> {
        let tool = action.tool();
        let agent_id = bound.agent_id.clone();
        let existing = self
            .store
            .read(|r| r.request_by_call(&agent_id, &call_id))?;
        if let Some(existing) = existing {
            let reply = if existing.row.action == action {
                status_reply(connection, call_id, tool, &existing.row.request)
            } else {
                error_reply(
                    connection,
                    call_id,
                    tool,
                    AgentFault::InvalidParams(InvalidParams {
                        pointer: Text::new("/params").map_err(|_| invariant("a pointer"))?,
                    }),
                    "This call id was used for another action.",
                )
            };
            self.outbox.agent.push(reply);
            return Ok(());
        }
        let limits = self.settings.limits;
        let now = self.now;
        let scope = scope_of(&action);
        let request = Request {
            request_id: crate::ids::new_id("request"),
            revision: PositiveCount::of(1),
            agent_id: agent_id.clone(),
            tool,
            account_id: crate::service::requests::calendars_of(&action).map(|(a, _)| a),
            received_at: timestamp(now),
            updated_at: timestamp(now),
            request_digest: request_digest_of(&agent_id, &action, scope.as_ref()),
            status: plain(RequestState::Received)?,
        };
        let row = RequestRow {
            request: request.clone(),
            action,
            scope,
            session_id: Some(bound.session_id.clone()),
            call_id: call_id.clone(),
        };
        let request_id = request.request_id.clone();
        let mut meta = self.meta(
            "agent.call",
            Actor::agent(agent_id.clone()),
            Some(&request_id),
            Some(call_id.as_str().to_owned()),
        );
        meta.idempotency = Some((
            IdempotencyScope::AgentCall,
            format!("{}/{}", agent_id.as_str(), call_id.as_str()),
        ));
        let written = self.write(meta, |tx, _| {
            let load = Load::read(&tx.read(), &agent_id, now)?;
            if let Some(refusal) = busy(&limits, &load, now) {
                return Err(TxError::Refused(refusal));
            }
            tx.apply(Mutation::RequestInserted {
                row,
                due_at_ms: Some(now),
            })?;
            let inserted = record_of(tx, &request_id)?.row.request;
            tx.notify(Change::Request(inserted.clone()))?;
            Ok(inserted)
        });
        match written {
            Ok(committed) => {
                self.outbox.notices.extend(committed.notices);
                self.outbox
                    .agent
                    .push(status_reply(connection, call_id, tool, &committed.value));
                Ok(())
            }
            Err(TxError::Refused(refusal)) => {
                self.busy.count(&agent_id, refusal.limit, now);
                self.outbox.agent.push(error_reply(
                    connection,
                    call_id,
                    tool,
                    AgentFault::Busy(refusal),
                    "Too many requests: wait and try again.",
                ));
                Ok(())
            }
            Err(TxError::Fault(fault)) => {
                self.outbox.agent.push(error_reply(
                    connection,
                    call_id,
                    tool,
                    AgentFault::Internal,
                    "The broker could not record the request and stopped.",
                ));
                Err(fault)
            }
        }
    }

    /// Creates the seed's agents and accounts that do not exist yet, in one `broker.seed`
    /// commit; a seed with nothing new writes nothing (debug builds; 5.10).
    #[cfg(debug_assertions)]
    pub(crate) fn seed(&mut self, seed: &crate::broker::Seed) -> Result<(), StoreFault> {
        use fabric_storage::{AgentRow, Counter};

        use crate::service::epochs;
        use crate::service::sessions::agent_view;

        let meta = self.meta("broker.seed", Actor::broker(), None, None);
        self.commit(meta, |tx, cx| {
            for agent in &seed.agents {
                if tx.read().agent(&agent.agent_id)?.is_some() {
                    continue;
                }
                tx.apply(Mutation::AgentPut {
                    agent: AgentRow {
                        agent_id: agent.agent_id.clone(),
                        revision: 1,
                        registered_name: agent.registered_name.clone(),
                        destination: agent.destination.clone(),
                        paused: false,
                        enrolled_at_ms: cx.now_ms,
                    },
                })?;
                let view = agent_view(&tx.read(), cx.activity, &agent.agent_id)?
                    .ok_or(invariant("a seeded agent exists"))?;
                tx.notify(Change::Agent(view))?;
            }
            let mut added = false;
            for account in &seed.accounts {
                if tx.read().account(&account.account_id)?.is_some() {
                    continue;
                }
                let account = fabric_desktop_contracts::owner::Account {
                    revision: PositiveCount::of(1),
                    ..account.clone()
                };
                tx.apply(Mutation::AccountPut {
                    account: account.clone(),
                })?;
                tx.notify(Change::Account(account))?;
                added = true;
            }
            if added {
                epochs::raise(tx, Counter::RevocationEpoch)?;
            }
            Ok(())
        })
    }
}
