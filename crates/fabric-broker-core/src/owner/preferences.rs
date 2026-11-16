//! `save_preferences` (design 6.1): the owner's settings, stored now and used by CD-W09.

use fabric_desktop_contracts::owner::{
    Change, Preferences, SavePreferencesParams, SavePreferencesResult,
};
use fabric_desktop_contracts::scalar::Count;
use fabric_storage::{Mutation, TxError, WriteTx};

use super::commands::{conflict, refused};
use super::frames::invalid_params;
use crate::service::{Cx, OwnerRefusal, invariant};

/// `save_preferences`: a revision check, then the whole settings replaced. A warning calendar
/// must name a calendar the store holds. Zones are checked by their pattern only (CD-W09).
pub(crate) fn save_preferences(
    tx: &mut WriteTx<'_>,
    cx: &mut Cx<'_>,
    p: &SavePreferencesParams,
) -> Result<SavePreferencesResult, TxError<OwnerRefusal>> {
    // Preferences end no grant and move no request, so the step's context is not needed.
    let _ = cx;
    let revision = tx.read().preferences()?.revision.get();
    if revision != p.expected_revision.get() {
        return conflict(revision, "preferences");
    }
    let accounts = tx.read().accounts()?;
    for (index, calendar) in p.settings.warning_calendars.iter().enumerate() {
        let stored = accounts.iter().any(|a| {
            a.account_id == calendar.account_id
                && a.calendars
                    .iter()
                    .any(|c| c.calendar_id == calendar.calendar_id)
        });
        if !stored {
            return refused(
                invalid_params(&format!("/params/settings/warning_calendars/{index}")),
                "A warning calendar names no calendar the broker knows.",
            );
        }
    }
    let preferences = Preferences {
        revision: Count::new(revision + 1)
            .map_err(|_| invariant("a revision is a safe integer"))?,
        settings: p.settings.clone(),
    };
    tx.apply(Mutation::PreferencesPut {
        preferences: preferences.clone(),
    })?;
    let sequence = tx.notify(Change::Preferences(preferences.clone()))?;
    Ok(SavePreferencesResult {
        commit_sequence: sequence,
        preferences,
    })
}
