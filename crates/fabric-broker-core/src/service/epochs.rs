//! The pause and revocation epochs (5.6).

use fabric_storage::{Counter, Counters, Fence, Mutation, StoreFault, WriteTx};

/// Raises the counter by one; returns its new value.
pub(crate) fn raise(tx: &mut WriteTx<'_>, counter: Counter) -> Result<u64, StoreFault> {
    let counters = tx.read().counters()?;
    let to = match counter {
        Counter::PauseEpoch => counters.pause_epoch,
        Counter::RevocationEpoch => counters.revocation_epoch,
    } + 1;
    tx.apply(Mutation::CounterRaised { counter, to })?;
    Ok(to)
}

/// The fence of the store's state now. W04 observes no event, so `event_revision` is `None`
/// until CD-W05.
#[must_use]
pub(crate) fn fence(counters: Counters) -> Fence {
    Fence {
        pause_epoch: counters.pause_epoch,
        revocation_epoch: counters.revocation_epoch,
        event_revision: None,
    }
}

/// True when a proof's fence still holds: every version it depended on is unchanged (5.6).
#[must_use]
pub(crate) fn fence_holds(proved: &Fence, now: &Fence) -> bool {
    proved == now
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counters(pause_epoch: u64, revocation_epoch: u64) -> Counters {
        Counters {
            pause_epoch,
            revocation_epoch,
        }
    }

    #[test]
    fn a_fence_holds_only_while_every_version_is_unchanged() {
        let proved = fence(counters(2, 7));
        assert!(fence_holds(&proved, &fence(counters(2, 7))));
        assert!(!fence_holds(&proved, &fence(counters(3, 7))));
        assert!(!fence_holds(&proved, &fence(counters(2, 8))));
        let observed = Fence {
            event_revision: Some("rev-2".to_owned()),
            ..fence(counters(2, 7))
        };
        assert!(!fence_holds(&proved, &observed));
        assert!(!fence_holds(&observed, &proved));
    }
}
