//! `subscribe` and the notice ring (at most 256 notices; design 5.4, D5, D6).
//!
//! The ring holds the last notices of this run. Its base is the store's notice sequence when
//! the run opened, so a page that took its snapshot right after a restart may subscribe at that
//! cursor. Notices are written to the page only while it is subscribed; one step producing more
//! than the ring holds sends one `snapshot_required` change instead and ends the subscription.

use fabric_desktop_contracts::Frame;
use fabric_desktop_contracts::owner::{Change, OwnerNotice};

use super::OwnerChannel;

/// The most notices the ring keeps, and the most one step may send to a subscriber.
pub(crate) const RING_CAPACITY: usize = 256;

/// Why a cursor cannot be served from the ring.
pub(crate) const AHEAD_MESSAGE: &str =
    "The cursor is ahead of the last notice, so take a snapshot first.";
/// A cursor from before this run of the broker.
pub(crate) const BEFORE_RUN_MESSAGE: &str =
    "The cursor is older than this run of the broker, so take a snapshot first.";
/// A cursor whose next notices left the ring.
pub(crate) const NOT_KEPT_MESSAGE: &str =
    "The notices after the cursor are no longer kept, so take a snapshot first.";

impl OwnerChannel {
    /// Empties the ring, sets its base and ends the subscription (after every `Core::open`).
    pub(super) fn reset_ring(&mut self, base: u64) {
        self.ring.clear();
        self.base = base;
        self.subscribed = false;
    }

    /// Keeps `notices` in the ring, dropping the oldest past its capacity.
    pub(super) fn ring_notices(&mut self, notices: &[OwnerNotice]) {
        for notice in notices {
            self.ring.push_back(notice.clone());
        }
        while self.ring.len() > RING_CAPACITY {
            self.ring.pop_front();
        }
    }

    /// The frames that write `notices` to a subscriber: all of them, or one `snapshot_required`
    /// change with the first notice's sequence when there are more than the ring holds, which
    /// also ends the subscription (CCR-4). Nothing while not subscribed.
    pub(super) fn notice_frames(&mut self, notices: Vec<OwnerNotice>) -> Vec<Frame> {
        if !self.subscribed {
            return Vec::new();
        }
        if notices.len() > RING_CAPACITY {
            self.subscribed = false;
            return notices
                .into_iter()
                .next()
                .map(|first| {
                    Frame::OwnerNotice(OwnerNotice {
                        sequence: first.sequence,
                        change: Change::SnapshotRequired,
                    })
                })
                .into_iter()
                .collect();
        }
        notices.into_iter().map(Frame::OwnerNotice).collect()
    }

    /// Starts a subscription at `cursor` when the ring holds every notice after it; returns
    /// those notices. `notice_sequence` is the store's last notice. The refusal's message
    /// otherwise; a refusal leaves the channel as it was.
    pub(super) fn subscribe_at(
        &mut self,
        cursor: u64,
        notice_sequence: u64,
    ) -> Result<Vec<OwnerNotice>, &'static str> {
        if cursor > notice_sequence {
            return Err(AHEAD_MESSAGE);
        }
        if cursor < self.base {
            return Err(BEFORE_RUN_MESSAGE);
        }
        if cursor < notice_sequence {
            let first = self.ring.front().map(|n| n.sequence.get());
            let last = self.ring.back().map(|n| n.sequence.get());
            let kept = matches!((first, last), (Some(first), Some(last))
                if first <= cursor + 1 && last == notice_sequence);
            if !kept {
                return Err(NOT_KEPT_MESSAGE);
            }
        }
        self.subscribed = true;
        self.ever_subscribed = true;
        Ok(self
            .ring
            .iter()
            .filter(|n| n.sequence.get() > cursor)
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use fabric_desktop_contracts::owner::{Change, OwnerNotice};
    use fabric_desktop_contracts::scalar::{Id, PositiveCount};

    use super::*;

    fn notice(sequence: u64) -> OwnerNotice {
        OwnerNotice {
            sequence: PositiveCount::new(sequence).unwrap(),
            change: Change::RecipientSetDeleted(Id::new(format!("set-{sequence}")).unwrap()),
        }
    }

    fn notices(range: std::ops::RangeInclusive<u64>) -> Vec<OwnerNotice> {
        range.map(notice).collect()
    }

    fn sequences(frames: &[Frame]) -> Vec<u64> {
        frames
            .iter()
            .map(|f| match f {
                Frame::OwnerNotice(n) => n.sequence.get(),
                other => panic!("{other:?}"),
            })
            .collect()
    }

    fn channel(base: u64) -> OwnerChannel {
        let mut channel = OwnerChannel::new("test".to_owned());
        channel.begin_run(base, Vec::new());
        channel
    }

    #[test]
    fn a_cursor_ahead_of_the_store_needs_a_snapshot() {
        let mut channel = channel(4);
        assert_eq!(channel.subscribe_at(5, 4), Err(AHEAD_MESSAGE));
        assert!(!channel.ever_subscribed());
    }

    #[test]
    fn a_cursor_below_the_base_needs_a_snapshot() {
        let mut channel = channel(4);
        assert_eq!(channel.subscribe_at(3, 4), Err(BEFORE_RUN_MESSAGE));
        assert!(!channel.subscribed);
    }

    #[test]
    fn a_cursor_equal_to_the_base_after_a_restart_is_accepted() {
        let mut channel = channel(4);
        assert_eq!(channel.subscribe_at(4, 4), Ok(Vec::new()));
        assert!(channel.subscribed);
        assert!(channel.ever_subscribed());
    }

    #[test]
    fn a_cursor_older_than_the_ring_needs_a_snapshot() {
        let mut channel = channel(0);
        let _ = channel.publish(notices(1..=300));
        assert_eq!(channel.ring.len(), RING_CAPACITY);
        assert_eq!(channel.subscribe_at(43, 300), Err(NOT_KEPT_MESSAGE));
        // The ring keeps 45..=300: a cursor of 44 is served whole.
        let sent = channel.subscribe_at(44, 300).unwrap();
        assert_eq!(sent.first().map(|n| n.sequence.get()), Some(45));
        assert_eq!(sent.len(), RING_CAPACITY);
    }

    #[test]
    fn a_subscription_sends_the_ringed_notices_after_its_cursor() {
        let mut channel = channel(2);
        channel.begin_run(2, notices(3..=5));
        let sent = channel.subscribe_at(3, 5).unwrap();
        assert_eq!(sent, notices(4..=5));
        assert_eq!(sequences(&channel.publish(notices(6..=6))), vec![6]);
    }

    #[test]
    fn nothing_is_written_before_a_subscription() {
        let mut channel = channel(0);
        assert!(channel.publish(notices(1..=3)).is_empty());
        assert_eq!(channel.ring.len(), 3);
    }

    #[test]
    fn a_step_of_256_notices_is_sent_whole_and_257_needs_a_snapshot() {
        let mut channel = channel(0);
        channel.subscribe_at(0, 0).unwrap();
        let sent = channel.publish(notices(1..=256));
        assert_eq!(sequences(&sent), (1..=256).collect::<Vec<_>>());
        assert!(channel.subscribed);

        let sent = channel.publish(notices(257..=513));
        assert_eq!(
            sent,
            vec![Frame::OwnerNotice(OwnerNotice {
                sequence: PositiveCount::new(257).unwrap(),
                change: Change::SnapshotRequired,
            })]
        );
        assert!(!channel.subscribed);
        assert!(channel.publish(notices(514..=514)).is_empty());
        // The ring still holds the last notices, so a later subscription can resume.
        assert_eq!(channel.subscribe_at(513, 514).unwrap(), notices(514..=514));
    }

    /// An account notice of `calendars` calendars with long ids.
    fn account_notice(sequence: u64, calendars: usize) -> OwnerNotice {
        let calendars: Vec<serde_json::Value> = (0..calendars)
            .map(|n| {
                serde_json::json!({"calendar_id": format!("cal-{n:0>120}"), "name": "Calendar",
                    "access": "owner", "selected": true, "agent_drafts": false})
            })
            .collect();
        let account = serde_json::json!({"account_id": "acct-1", "revision": 1,
            "state": "connected", "email": "owner@example.com", "calendars": calendars});
        OwnerNotice {
            sequence: PositiveCount::new(sequence).unwrap(),
            change: Change::Account(
                fabric_desktop_contracts::decode_record(account.to_string().as_bytes()).unwrap(),
            ),
        }
    }

    /// README section 1: a notice over the frame bound is never written; one
    /// `snapshot_required` change with its sequence takes its place, the subscription ends and
    /// the notices after it in the step are not written. (A response over the bound: the
    /// integration test `an_answer_over_the_frame_bound_is_refused_and_the_broker_runs_on`.)
    #[test]
    fn a_frame_over_the_bound_is_never_written() {
        let mut subscribed = channel(0);
        subscribed.subscribe_at(0, 0).unwrap();
        let small = Frame::OwnerNotice(notice(1));
        let frames = subscribed.fit_frames(vec![
            small.clone(),
            Frame::OwnerNotice(account_notice(2, 6_000)),
            Frame::OwnerNotice(notice(3)),
        ]);
        assert_eq!(
            frames,
            vec![
                small,
                Frame::OwnerNotice(OwnerNotice {
                    sequence: PositiveCount::new(2).unwrap(),
                    change: Change::SnapshotRequired,
                }),
            ]
        );
        assert!(!subscribed.subscribed);

        // A notice within the bound is written as it is.
        let mut fresh = channel(0);
        fresh.subscribe_at(0, 0).unwrap();
        let within = Frame::OwnerNotice(account_notice(1, 10));
        assert_eq!(fresh.fit_frames(vec![within.clone()]), vec![within]);
        assert!(fresh.subscribed);
    }

    #[test]
    fn a_new_run_empties_the_ring_and_ends_the_subscription() {
        let mut channel = channel(0);
        channel.subscribe_at(0, 0).unwrap();
        let _ = channel.publish(notices(1..=3));
        channel.begin_run(3, Vec::new());
        assert!(channel.ring.is_empty());
        assert!(!channel.subscribed);
        assert!(channel.ever_subscribed());
        assert_eq!(channel.subscribe_at(2, 3), Err(BEFORE_RUN_MESSAGE));
    }
}
