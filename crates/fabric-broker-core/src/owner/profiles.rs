//! `save_profile`: profile versions, activation, the profiles revision, and expansion (design
//! 6.1, D20).
//!
//! Every `save_profile` commit applies exactly one `ProfileStateSet` with the active profile
//! after the command and the profiles revision + 1. A saved custom profile is a new immutable
//! version; a change that makes a row wider than its baseline needs the owner's
//! acknowledgement of that expansion's digest. Activation ends no grant and stales no card.

use fabric_desktop_contracts::error::{ExpansionNotAcknowledged, OwnerFault};
use fabric_desktop_contracts::expansion_digest;
use fabric_desktop_contracts::owner::{
    BuiltInProfile, Change, CustomProfile, CustomProfileDraft, Expansion, ExpansionEntry,
    PolicyRow, ProfileChange, ProfileRows, SaveProfileParams, SaveProfileResult,
};
use fabric_desktop_contracts::scalar::{CustomProfileId, Digest, Id, PositiveCount};
use fabric_storage::{Counter, Mutation, StoreFault, TxError, WriteTx};

use super::commands::{conflict, not_found, refused, within_budget};
use super::frames::{NOT_IN_THIS_BUILD_MESSAGE, invalid_params};
use super::views::owner_state_bytes;
use crate::service::{Cx, OwnerRefusal, epochs, invariant};

/// `save_profile`.
pub(crate) fn save_profile(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    p: &SaveProfileParams,
) -> Result<SaveProfileResult, TxError<OwnerRefusal>> {
    let revision = tx.read().profiles()?.revision.get();
    if revision != p.expected_revision.get() {
        return conflict(revision, "profiles");
    }
    let (active_id, active_version) = match &p.change {
        ProfileChange::Activate { profile_id } => {
            let version = match built_in(profile_id.as_str()) {
                Some(profile) => profile.version(),
                None => match tx.read().custom_profile(profile_id)? {
                    Some(custom) => custom.version.get(),
                    None => return not_found("profile"),
                },
            };
            epochs::raise(tx, Counter::RevocationEpoch)?;
            (profile_id.clone(), version)
        }
        ProfileChange::SaveCustom {
            profile,
            acknowledged_expansion,
        } => save_custom(tx, cx, profile, acknowledged_expansion.as_ref())?,
        // not implemented in this build (contract schema version 3)
        ProfileChange::DeleteCustom { .. } => {
            return refused(OwnerFault::UnknownMethod, NOT_IN_THIS_BUILD_MESSAGE);
        }
    };
    tx.apply(Mutation::ProfileStateSet {
        active_profile_id: active_id,
        active_version,
        revision: revision + 1,
    })?;
    let profiles = tx.read().profiles()?;
    let sequence = tx.notify(Change::Profiles(profiles.clone()))?;
    Ok(SaveProfileResult {
        commit_sequence: sequence,
        profiles,
    })
}

/// The built-in profile with this id.
fn built_in(profile_id: &str) -> Option<BuiltInProfile> {
    BuiltInProfile::ALL
        .into_iter()
        .find(|b| b.profile_id() == profile_id)
}

/// True when `name` reads as a built-in profile's name (README section 14 item 28): in any
/// case, with leading, trailing or repeated whitespace, and with invisible characters (zero
/// width spaces and joiners, the word joiner, the byte order mark, the soft hyphen) anywhere.
#[must_use]
pub(crate) fn names_a_built_in(name: &str) -> bool {
    let shown = as_shown(name);
    BuiltInProfile::ALL
        .into_iter()
        .any(|b| b.name().to_lowercase() == shown)
}

/// A name as the owner sees it: invisible characters dropped, whitespace trimmed and each run
/// of it one space, in lowercase.
fn as_shown(name: &str) -> String {
    let visible: String = name
        .chars()
        .filter(|c| {
            !matches!(
                c,
                '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}' | '\u{00AD}'
            )
        })
        .collect();
    visible
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// A new custom profile id: `custom-` and 16 hex digits.
fn new_profile_id() -> Result<CustomProfileId, StoreFault> {
    let fresh = crate::ids::new_id("custom");
    let text: String = fresh.as_str().chars().take("custom-".len() + 16).collect();
    CustomProfileId::new(text).map_err(|_| invariant("a new custom profile id is valid"))
}

/// Adds a version of a custom profile; returns the active profile and version after it.
fn save_custom(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    draft: &CustomProfileDraft,
    acknowledged: Option<&Digest>,
) -> Result<(Id, u64), TxError<OwnerRefusal>> {
    if names_a_built_in(draft.name.as_str()) {
        return refused(
            invalid_params("/params/change/profile/name"),
            "Careful and Flexible are the built-in profiles, so choose another name.",
        );
    }
    let (profile_id, version, baseline) = match &draft.profile_id {
        Some(profile_id) => {
            let id = Id::new(profile_id.as_str())
                .map_err(|_| invariant("a custom profile id is an id"))?;
            let Some(previous) = tx.read().custom_profile(&id)? else {
                return not_found("profile");
            };
            (
                profile_id.clone(),
                previous.version.get() + 1,
                previous.rows,
            )
        }
        // A new profile is measured against Careful (D20).
        None => (new_profile_id()?, 1, ProfileRows::CAREFUL),
    };
    if let Some(expansion) = expansion(&baseline, &draft.rows) {
        let digest = expansion_digest(&expansion);
        if acknowledged != Some(&digest) {
            return refused(
                OwnerFault::ExpansionNotAcknowledged(ExpansionNotAcknowledged {
                    expansion,
                    expansion_digest: digest,
                }),
                "The profile permits more than before, so confirm the listed rows first.",
            );
        }
    }
    let id = Id::new(profile_id.as_str()).map_err(|_| invariant("a custom profile id is an id"))?;
    let before = owner_state_bytes(&tx.read())?;
    tx.apply(Mutation::ProfileVersionAdded {
        profile: CustomProfile {
            profile_id,
            name: draft.name.clone(),
            version: PositiveCount::new(version)
                .map_err(|_| invariant("a profile version is a safe integer"))?,
            rows: draft.rows,
        },
        at_ms: cx.now_ms,
    })?;
    within_budget(tx, before, "/params/change/profile")?;
    let (active_id, active_version, _) = tx.read().active_profile()?;
    if active_id == id {
        // A new version of the active profile is in force at once (5.6).
        epochs::raise(tx, Counter::RevocationEpoch)?;
        return Ok((active_id, version));
    }
    Ok((active_id, active_version))
}

/// The rows `next` makes wider than `baseline`, if any.
#[must_use]
pub(crate) fn expansion(baseline: &ProfileRows, next: &ProfileRows) -> Option<Expansion> {
    let entries: Vec<ExpansionEntry> = PolicyRow::ALL
        .into_iter()
        .filter(|row| next.mode(*row).is_wider_than(baseline.mode(*row)))
        .map(|row| ExpansionEntry {
            row,
            from: baseline.mode(row),
            to: next.mode(row),
        })
        .collect();
    Expansion::new(entries).ok()
}

#[cfg(test)]
mod tests {
    use fabric_desktop_contracts::owner::Mode;

    use super::*;

    #[test]
    fn the_same_rows_or_narrower_rows_are_no_expansion() {
        assert_eq!(
            expansion(&ProfileRows::CAREFUL, &ProfileRows::CAREFUL),
            None
        );
        assert_eq!(
            expansion(&ProfileRows::FLEXIBLE, &ProfileRows::CAREFUL),
            None
        );
    }

    #[test]
    fn every_wider_row_is_listed_in_matrix_order() {
        let found = expansion(&ProfileRows::CAREFUL, &ProfileRows::FLEXIBLE).unwrap();
        let rows: Vec<(PolicyRow, Mode, Mode)> =
            found.iter().map(|e| (e.row, e.from, e.to)).collect();
        assert_eq!(
            rows,
            vec![
                (PolicyRow::ReadEventDetails, Mode::Ask, Mode::Allow),
                (PolicyRow::CreateInExistingCalendar, Mode::Ask, Mode::Allow),
                (PolicyRow::EditFabricCreatedEvent, Mode::Ask, Mode::Allow),
                (PolicyRow::SendOrChangeInvitations, Mode::Ask, Mode::Allow),
            ]
        );
    }

    #[test]
    fn block_to_ask_is_an_expansion_and_allow_to_block_is_not() {
        let baseline = ProfileRows {
            list_and_availability: Mode::Block,
            ..ProfileRows::CAREFUL
        };
        let next = ProfileRows {
            list_and_availability: Mode::Ask,
            create_in_agent_drafts: Mode::Block,
            ..ProfileRows::CAREFUL
        };
        let found = expansion(&baseline, &next).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].row, PolicyRow::ListAndAvailability);
        assert_eq!((found[0].from, found[0].to), (Mode::Block, Mode::Ask));
    }

    #[test]
    fn the_expansion_digest_is_over_the_listed_rows() {
        let found = expansion(&ProfileRows::CAREFUL, &ProfileRows::FLEXIBLE).unwrap();
        let again = expansion(&ProfileRows::CAREFUL, &ProfileRows::FLEXIBLE).unwrap();
        assert_eq!(expansion_digest(&found), expansion_digest(&again));
        let fewer = expansion(
            &ProfileRows::CAREFUL,
            &ProfileRows {
                read_event_details: Mode::Allow,
                ..ProfileRows::CAREFUL
            },
        )
        .unwrap();
        assert_ne!(expansion_digest(&found), expansion_digest(&fewer));
    }

    #[test]
    fn a_built_in_name_in_any_case_is_taken() {
        for name in ["Careful", "careful", "CAREFUL", "Flexible", "fLeXiBlE"] {
            assert!(names_a_built_in(name), "{name}");
        }
        for name in ["Carefully", "Team", "Care ful", "Flexible profile"] {
            assert!(!names_a_built_in(name), "{name}");
        }
    }

    /// README section 14 item 28: a name that reads as Careful or Flexible is taken, whatever
    /// whitespace or invisible characters surround or split it.
    #[test]
    fn a_built_in_name_with_spaces_or_invisible_characters_is_taken() {
        for name in [
            "Flexible ",
            " Careful",
            "Careful\u{00A0}",
            "Careful\u{200B}",
            "\u{FEFF}Flexible",
            "Care\u{200B}ful",
            "Flexi\u{00AD}ble",
            "\u{3000}CAREFUL\t",
        ] {
            assert!(names_a_built_in(name), "{name:?}");
        }
    }

    #[test]
    fn a_new_profile_id_is_custom_and_16_hex_digits() {
        let id = new_profile_id().unwrap();
        let tail = id.as_str().strip_prefix("custom-").unwrap();
        assert_eq!(tail.len(), 16);
        assert!(tail.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}
