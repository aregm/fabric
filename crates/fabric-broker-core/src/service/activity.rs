//! Agent activity of this run (D35): connects, calls and idle deadlines, kept in memory only.
//!
//! Every agent call, a status poll included, is activity: it moves the session's
//! `last_active_ms` and the agent's `last_tool_call_ms`. Nothing is written and no revision
//! changes; the durable event is the session's end.

use fabric_desktop_contracts::scalar::Id;

use crate::broker::SESSION_IDLE_MS;
use crate::service::{Activity, SessionChange};

/// True while a session whose last activity was at `last_active_ms` is live at `now_ms`: before
/// the last activity plus the idle limit, as the theory's `session_live` reads it
/// (`ctx_now c < active + session_idle_timeout_ms`, isabelle/Fabric_Broker/Fabric_Broker.thy).
/// The validity predicate, the idle end and its deadline all use it, so a session ends at the
/// instant the theory stops calling it live (5.5, c39).
#[must_use]
pub(crate) const fn session_live_at(last_active_ms: i64, now_ms: i64) -> bool {
    now_ms.saturating_sub(last_active_ms) < SESSION_IDLE_MS
}

impl Activity {
    /// Applies what a committed step changed.
    pub(crate) fn apply(&mut self, change: SessionChange) {
        match change {
            SessionChange::Started { agent_id, session } => {
                self.sessions.insert(agent_id, session);
            }
            SessionChange::Ended { agent_id } => {
                self.sessions.remove(&agent_id);
            }
        }
    }

    /// One call of the agent at `now_ms`.
    pub(crate) fn record_call(&mut self, agent_id: &Id, now_ms: i64) {
        if let Some(session) = self.sessions.get_mut(agent_id) {
            session.last_active_ms = session.last_active_ms.max(now_ms);
        }
        self.last_tool_call_ms.insert(agent_id.clone(), now_ms);
    }

    /// The live sessions idle at `now_ms`: `SESSION_IDLE_MS` or more since their last activity
    /// (5.5, [`session_live_at`]), as (agent, session).
    #[must_use]
    pub(crate) fn idle(&self, now_ms: i64) -> Vec<(Id, Id)> {
        let mut idle: Vec<(Id, Id)> = self
            .sessions
            .iter()
            .filter(|(_, s)| !session_live_at(s.last_active_ms, now_ms))
            .map(|(agent_id, s)| (agent_id.clone(), s.session_id.clone()))
            .collect();
        idle.sort();
        idle
    }

    /// The first instant a live session is idle: its last activity plus the idle limit.
    #[must_use]
    pub(crate) fn next_idle_ms(&self) -> Option<i64> {
        self.sessions
            .values()
            .map(|s| s.last_active_ms + SESSION_IDLE_MS)
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::LiveSession;

    const T0: i64 = 1_795_190_400_000;

    fn id(text: &str) -> Id {
        Id::new(text).unwrap()
    }

    fn started(activity: &mut Activity, agent: &str, session: &str, at: i64) {
        activity.apply(SessionChange::Started {
            agent_id: id(agent),
            session: LiveSession {
                session_id: id(session),
                started_at_ms: at,
                last_active_ms: at,
            },
        });
    }

    #[test]
    fn a_session_with_no_call_idles_1800000_ms_after_its_connect() {
        let mut activity = Activity::default();
        started(&mut activity, "agent-a", "session-1", T0);
        assert!(activity.idle(T0 + SESSION_IDLE_MS - 1).is_empty());
        assert_eq!(
            activity.idle(T0 + SESSION_IDLE_MS),
            vec![(id("agent-a"), id("session-1"))]
        );
        assert_eq!(activity.next_idle_ms(), Some(T0 + SESSION_IDLE_MS));
        assert!(!activity.last_tool_call_ms.contains_key(&id("agent-a")));
    }

    #[test]
    fn a_call_moves_the_idle_deadline_and_records_the_last_tool_call() {
        let mut activity = Activity::default();
        started(&mut activity, "agent-a", "session-1", T0);
        activity.record_call(&id("agent-a"), T0 + 1_000);
        assert_eq!(activity.last_tool_call_ms[&id("agent-a")], T0 + 1_000);
        assert_eq!(activity.sessions[&id("agent-a")].last_active_ms, T0 + 1_000);
        assert!(activity.idle(T0 + SESSION_IDLE_MS + 1).is_empty());
        assert_eq!(activity.next_idle_ms(), Some(T0 + 1_000 + SESSION_IDLE_MS));
    }

    #[test]
    fn a_session_is_idle_at_exactly_the_idle_limit() {
        // c39: the theory finds a session live only while now < active + the limit, so the idle
        // end and its deadline fall on the limit itself, never 1 ms after it.
        let last = T0 + 7_000;
        assert!(session_live_at(last, last + SESSION_IDLE_MS - 1));
        assert!(!session_live_at(last, last + SESSION_IDLE_MS));
        let mut activity = Activity::default();
        started(&mut activity, "agent-a", "session-1", last);
        assert!(activity.idle(last + SESSION_IDLE_MS - 1).is_empty());
        assert_eq!(
            activity.idle(last + SESSION_IDLE_MS),
            vec![(id("agent-a"), id("session-1"))]
        );
        assert_eq!(activity.next_idle_ms(), Some(last + SESSION_IDLE_MS));
    }

    #[test]
    fn an_ended_session_has_no_deadline() {
        let mut activity = Activity::default();
        started(&mut activity, "agent-a", "session-1", T0);
        activity.apply(SessionChange::Ended {
            agent_id: id("agent-a"),
        });
        assert_eq!(activity.next_idle_ms(), None);
        // A call without a session is still the agent's last tool call.
        activity.record_call(&id("agent-a"), T0 + 5);
        assert_eq!(activity.last_tool_call_ms[&id("agent-a")], T0 + 5);
    }
}
