//! The relied grant list of a request (D36).

use fabric_desktop_contracts::scalar::Id;
use fabric_storage::{Mutation, StoreFault, WriteTx};

/// `ReliedGrantSet`. The core does not check the ids again: the grant check of 5.7
/// (`pipeline::judge`, `Judgement::OutsideFacts`) is the one place that turns a grant outside the
/// job's facts into `undecided`, before this is reached; the store's apply guard alone refuses an
/// id that is not a grant of the request's agent, as an invariant fault (a broken rule, a halt).
pub(crate) fn set_relied(
    tx: &mut WriteTx<'_>,
    request_id: &Id,
    grant_ids: Vec<Id>,
) -> Result<(), StoreFault> {
    tx.apply(Mutation::ReliedGrantSet {
        request_id: request_id.clone(),
        grant_ids,
    })
}
