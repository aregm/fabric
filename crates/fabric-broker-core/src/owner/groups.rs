//! `save_group`: recipient sets (design 6.1, 5.6, D19).
//!
//! A set is created, saved as a new version, or deleted. Narrowing a set applies at once;
//! adding a member or a domain, or deleting the set, ends every live grant whose scope names
//! it with `authority_reduced` (CCR-9). Every save or delete raises the revocation epoch.

use fabric_desktop_contracts::owner::{
    Change, GroupChange, RecipientSet, RecipientSetDefinition, RecipientSetSource, SaveGroupParams,
    SaveGroupResult,
};
use fabric_desktop_contracts::scalar::{Id, PositiveCount};
use fabric_storage::{Counter, Mutation, TxError, WriteTx};

use super::commands::{conflict, next_revision, not_found, within_budget};
use super::views::owner_state_bytes;
use crate::service::grants::end_grants_naming_set;
use crate::service::{Cx, OwnerRefusal, epochs};

/// `save_group`.
pub(crate) fn save_group(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    p: &SaveGroupParams,
) -> Result<SaveGroupResult, TxError<OwnerRefusal>> {
    match &p.change {
        GroupChange::Save {
            recipient_set_id: None,
            definition,
        } => {
            // A new set is named with `expected_revision` 0.
            if p.expected_revision.get() != 0 {
                return conflict(0, "recipient set");
            }
            let set = RecipientSet {
                recipient_set_id: crate::ids::new_id("set"),
                revision: PositiveCount::of(1),
                source: RecipientSetSource::Local,
                definition: definition.clone(),
            };
            epochs::raise(tx, Counter::RevocationEpoch)?;
            saved(tx, cx, set)
        }
        GroupChange::Save {
            recipient_set_id: Some(id),
            definition,
        } => {
            let current = current(tx, id, p.expected_revision.get())?;
            let ended = if widens(&current.definition, definition) {
                end_grants_naming_set(tx, cx, id)?.len()
            } else {
                0
            };
            if ended == 0 {
                epochs::raise(tx, Counter::RevocationEpoch)?;
            }
            let set = RecipientSet {
                recipient_set_id: id.clone(),
                revision: next_revision(current.revision.get())?,
                source: current.source,
                definition: definition.clone(),
            };
            saved(tx, cx, set)
        }
        GroupChange::Delete { recipient_set_id } => {
            let _ = current(tx, recipient_set_id, p.expected_revision.get())?;
            if end_grants_naming_set(tx, cx, recipient_set_id)?.is_empty() {
                epochs::raise(tx, Counter::RevocationEpoch)?;
            }
            tx.apply(Mutation::RecipientSetDeleted {
                recipient_set_id: recipient_set_id.clone(),
                at_ms: cx.now_ms,
            })?;
            let sequence = tx.notify(Change::RecipientSetDeleted(recipient_set_id.clone()))?;
            Ok(SaveGroupResult {
                commit_sequence: sequence,
                recipient_set: None,
            })
        }
    }
}

/// The set `id` at `expected_revision`: `not_found` for an unknown or deleted set, `conflict`
/// for another revision.
fn current(
    tx: &WriteTx<'_>,
    id: &Id,
    expected_revision: u64,
) -> Result<RecipientSet, TxError<OwnerRefusal>> {
    let Some(set) = tx.read().recipient_set(id)? else {
        return not_found("recipient set");
    };
    if set.revision.get() != expected_revision {
        return conflict(set.revision.get(), "recipient set");
    }
    Ok(set)
}

/// Stores the set's new version, refused when it grows the owner's records past their budget,
/// and emits its notice, the command's last.
fn saved(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    set: RecipientSet,
) -> Result<SaveGroupResult, TxError<OwnerRefusal>> {
    let before = owner_state_bytes(&tx.read())?;
    tx.apply(Mutation::RecipientSetVersionAdded {
        set: set.clone(),
        at_ms: cx.now_ms,
    })?;
    within_budget(tx, before, "/params/change/definition")?;
    let sequence = tx.notify(Change::RecipientSet(set.clone()))?;
    Ok(SaveGroupResult {
        commit_sequence: sequence,
        recipient_set: Some(set),
    })
}

/// True when `next` admits someone `before` did not: a new member or a new domain (D19).
#[must_use]
pub(crate) fn widens(before: &RecipientSetDefinition, next: &RecipientSetDefinition) -> bool {
    next.members.iter().any(|m| !before.members.contains(m))
        || next.domains.iter().any(|d| !before.domains.contains(d))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn definition(members: &[&str], domains: &[&str]) -> RecipientSetDefinition {
        fabric_desktop_contracts::decode_record(
            serde_json::to_string(&json!({"name": "Team", "members": members, "domains": domains}))
                .unwrap()
                .as_bytes(),
        )
        .unwrap()
    }

    #[test]
    fn removing_members_or_domains_narrows_a_set() {
        let before = definition(&["a@example.net", "b@example.net"], &["example.org"]);
        assert!(!widens(&before, &definition(&["a@example.net"], &[])));
        assert!(!widens(&before, &before.clone()));
    }

    #[test]
    fn a_new_member_or_domain_widens_a_set() {
        let before = definition(&["a@example.net"], &["example.org"]);
        assert!(widens(
            &before,
            &definition(&["a@example.net", "c@example.net"], &["example.org"])
        ));
        assert!(widens(
            &before,
            &definition(&["a@example.net"], &["example.org", "example.com"])
        ));
    }
}
