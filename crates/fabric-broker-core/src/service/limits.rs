//! Request limits (5.9; `Limits::CD_G0`) and the `busy` audit windows.
//!
//! The limits are checked only in the `agent.call` transaction that would insert a request
//! (L2). A refusal writes no ledger row: it is counted into the agent's window in
//! memory, and one `agent_busy` audit row per agent per window is written when the window ends
//! or at quit (D24; section 4).

use std::collections::{BTreeMap, BTreeSet};

use fabric_desktop_contracts::Limits;
use fabric_desktop_contracts::common::BusyLimit;
use fabric_desktop_contracts::error::Busy;
use fabric_desktop_contracts::scalar::{Count, Id};
use fabric_storage::{Actor, AuditEntry, ReadTx, StoreFault};

use crate::broker::BUSY_AUDIT_WINDOW_MS;

/// The window the per-agent rate counts over.
pub(crate) const RATE_WINDOW_MS: i64 = 60_000;
/// The wait a `busy` answer names when no request leaves a window.
pub(crate) const DEFAULT_RETRY_AFTER_MS: u64 = 5_000;

/// What the limits are checked against, for one agent at one instant.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Load {
    /// Open requests of every agent.
    pub(crate) open_total: u64,
    /// Open requests of the agent.
    pub(crate) open_agent: u64,
    /// Open approval cards of the agent.
    pub(crate) cards_agent: u64,
    /// When the agent's requests of the rate window were received, ascending.
    pub(crate) recent_ms: Vec<i64>,
}

impl Load {
    /// The load of `agent_id` at `now_ms`, read in the transaction that would insert.
    pub(crate) fn read(r: &ReadTx<'_>, agent_id: &Id, now_ms: i64) -> Result<Load, StoreFault> {
        Ok(Load {
            open_total: r.count_open(None)?,
            open_agent: r.count_open(Some(agent_id))?,
            cards_agent: r.count_cards(agent_id)?,
            recent_ms: r.received_since(agent_id, now_ms - RATE_WINDOW_MS + 1)?,
        })
    }
}

/// The first limit a new request would pass, as the `busy` detail, or `None` when it may be
/// created. The order is: open requests in total, open requests of the agent, its open cards,
/// its requests in the last 60 s.
#[must_use]
pub(crate) fn busy(limits: &Limits, load: &Load, now_ms: i64) -> Option<Busy> {
    let refuse = |limit, maximum: fabric_desktop_contracts::scalar::PositiveCount, wait| Busy {
        limit,
        maximum,
        retry_after_ms: Count::of(wait),
    };
    if load.open_total >= limits.open_requests_total.get() {
        return Some(refuse(
            BusyLimit::OpenRequestsTotal,
            limits.open_requests_total,
            DEFAULT_RETRY_AFTER_MS,
        ));
    }
    if load.open_agent >= limits.open_requests_per_agent.get() {
        return Some(refuse(
            BusyLimit::OpenRequestsPerAgent,
            limits.open_requests_per_agent,
            DEFAULT_RETRY_AFTER_MS,
        ));
    }
    if load.cards_agent >= limits.open_approval_cards_per_agent.get() {
        return Some(refuse(
            BusyLimit::OpenApprovalCardsPerAgent,
            limits.open_approval_cards_per_agent,
            DEFAULT_RETRY_AFTER_MS,
        ));
    }
    let recent = load.recent_ms.len() as u64;
    if recent >= limits.requests_per_minute_per_agent.get() {
        let wait = load
            .recent_ms
            .first()
            .map_or(DEFAULT_RETRY_AFTER_MS, |oldest| {
                u64::try_from(oldest + RATE_WINDOW_MS - now_ms).unwrap_or(0)
            });
        return Some(refuse(
            BusyLimit::RequestsPerMinutePerAgent,
            limits.requests_per_minute_per_agent,
            wait,
        ));
    }
    None
}

/// The contract name of a limit.
fn limit_name(limit: BusyLimit) -> &'static str {
    match limit {
        BusyLimit::OpenRequestsTotal => "open_requests_total",
        BusyLimit::OpenRequestsPerAgent => "open_requests_per_agent",
        BusyLimit::RequestsPerMinutePerAgent => "requests_per_minute_per_agent",
        BusyLimit::OpenApprovalCardsPerAgent => "open_approval_cards_per_agent",
    }
}

/// One agent's open window of `busy` refusals.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Window {
    start_ms: i64,
    count: u64,
    limits: BTreeSet<&'static str>,
}

/// The open `busy` windows, by agent; memory only, so a crash loses at most their counts (R16).
#[derive(Default)]
pub(crate) struct BusyWindows {
    windows: BTreeMap<Id, Window>,
}

impl BusyWindows {
    /// Counts one refusal of `agent_id`; the first opens the agent's window.
    pub(crate) fn count(&mut self, agent_id: &Id, limit: BusyLimit, now_ms: i64) {
        let window = self.windows.entry(agent_id.clone()).or_insert(Window {
            start_ms: now_ms,
            count: 0,
            limits: BTreeSet::new(),
        });
        window.count += 1;
        window.limits.insert(limit_name(limit));
    }

    /// When the earliest window ends.
    #[must_use]
    pub(crate) fn next_end_ms(&self) -> Option<i64> {
        self.windows
            .values()
            .map(|w| w.start_ms + BUSY_AUDIT_WINDOW_MS)
            .min()
    }

    /// The audit rows of the windows that ended by `now_ms` (all of them with `all`), removed
    /// from memory. The caller writes them; a failed write halts the broker.
    pub(crate) fn take(&mut self, now_ms: i64, all: bool) -> Vec<AuditEntry> {
        let ended: Vec<Id> = self
            .windows
            .iter()
            .filter(|(_, w)| all || now_ms >= w.start_ms + BUSY_AUDIT_WINDOW_MS)
            .map(|(agent_id, _)| agent_id.clone())
            .collect();
        ended
            .into_iter()
            .filter_map(|agent_id| {
                let window = self.windows.remove(&agent_id)?;
                let end_ms = if all {
                    now_ms.min(window.start_ms + BUSY_AUDIT_WINDOW_MS)
                } else {
                    window.start_ms + BUSY_AUDIT_WINDOW_MS
                };
                Some(AuditEntry {
                    kind: "agent_busy",
                    actor: Actor::agent(agent_id.clone()),
                    request_id: None,
                    detail: serde_json::json!({
                        "agent_id": agent_id,
                        "window_start_ms": window.start_ms,
                        "window_end_ms": end_ms,
                        "count": window.count,
                        "limits": window.limits.iter().collect::<Vec<_>>(),
                    }),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_795_190_400_000;

    fn load(open_total: u64, open_agent: u64, cards_agent: u64, recent: usize) -> Load {
        Load {
            open_total,
            open_agent,
            cards_agent,
            recent_ms: (0..recent).map(|n| T0 + n as i64).collect(),
        }
    }

    fn limit_of(load: &Load) -> Option<BusyLimit> {
        busy(&Limits::CD_G0, load, T0 + 1_000).map(|b| b.limit)
    }

    #[test]
    fn each_limit_refuses_at_its_maximum_and_not_below() {
        assert_eq!(limit_of(&load(99, 24, 4, 14)), None);
        assert_eq!(
            limit_of(&load(100, 0, 0, 0)),
            Some(BusyLimit::OpenRequestsTotal)
        );
        assert_eq!(
            limit_of(&load(25, 25, 0, 0)),
            Some(BusyLimit::OpenRequestsPerAgent)
        );
        assert_eq!(
            limit_of(&load(5, 5, 5, 0)),
            Some(BusyLimit::OpenApprovalCardsPerAgent)
        );
        assert_eq!(
            limit_of(&load(0, 0, 0, 15)),
            Some(BusyLimit::RequestsPerMinutePerAgent)
        );
    }

    #[test]
    fn the_total_is_checked_first() {
        assert_eq!(
            limit_of(&load(100, 25, 5, 15)),
            Some(BusyLimit::OpenRequestsTotal)
        );
        assert_eq!(
            limit_of(&load(10, 25, 5, 15)),
            Some(BusyLimit::OpenRequestsPerAgent)
        );
        assert_eq!(
            limit_of(&load(10, 10, 5, 15)),
            Some(BusyLimit::OpenApprovalCardsPerAgent)
        );
    }

    #[test]
    fn the_rate_waits_until_the_oldest_call_leaves_the_window() {
        let refused = busy(&Limits::CD_G0, &load(0, 0, 0, 15), T0 + 1_000).unwrap();
        assert_eq!(refused.maximum.get(), 15);
        assert_eq!(refused.retry_after_ms.get(), 59_000);
        let other = busy(&Limits::CD_G0, &load(100, 0, 0, 0), T0).unwrap();
        assert_eq!(other.retry_after_ms.get(), DEFAULT_RETRY_AFTER_MS);
    }

    fn agent(text: &str) -> Id {
        Id::new(text).unwrap()
    }

    #[test]
    fn a_busy_window_is_one_row_per_agent_with_its_count() {
        let mut windows = BusyWindows::default();
        windows.count(&agent("agent-a"), BusyLimit::RequestsPerMinutePerAgent, T0);
        windows.count(
            &agent("agent-a"),
            BusyLimit::RequestsPerMinutePerAgent,
            T0 + 1_000,
        );
        windows.count(
            &agent("agent-a"),
            BusyLimit::OpenApprovalCardsPerAgent,
            T0 + 2_000,
        );
        windows.count(&agent("agent-b"), BusyLimit::OpenRequestsTotal, T0 + 30_000);
        assert_eq!(windows.next_end_ms(), Some(T0 + 60_000));

        assert!(windows.take(T0 + 59_999, false).is_empty());
        let rows = windows.take(T0 + 60_000, false);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, "agent_busy");
        assert_eq!(rows[0].detail["agent_id"], "agent-a");
        assert_eq!(rows[0].detail["count"], 3);
        assert_eq!(rows[0].detail["window_start_ms"], T0);
        assert_eq!(rows[0].detail["window_end_ms"], T0 + 60_000);
        assert_eq!(
            rows[0].detail["limits"],
            serde_json::json!([
                "open_approval_cards_per_agent",
                "requests_per_minute_per_agent"
            ])
        );
        assert_eq!(windows.next_end_ms(), Some(T0 + 90_000));
    }

    #[test]
    fn quit_flushes_every_open_window() {
        let mut windows = BusyWindows::default();
        windows.count(&agent("agent-a"), BusyLimit::RequestsPerMinutePerAgent, T0);
        windows.count(
            &agent("agent-b"),
            BusyLimit::RequestsPerMinutePerAgent,
            T0 + 10,
        );
        let rows = windows.take(T0 + 20, true);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].detail["window_end_ms"], T0 + 20);
        assert_eq!(windows.next_end_ms(), None);
        // A refusal after the flush opens a new window.
        windows.count(
            &agent("agent-a"),
            BusyLimit::RequestsPerMinutePerAgent,
            T0 + 30,
        );
        assert_eq!(windows.take(T0 + 40, true)[0].detail["count"], 1);
    }
}
