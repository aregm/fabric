//! The duration a card proposes (contract schema version 2, README section 10): the longest
//! duration the owner may choose for it. `decide_request` refuses a longer one.
//!
//! The ceiling is `until_revoked` only where a lasting grant made by `create_grant` would serve
//! the request in the fresh proof that follows the approval; everywhere else it is
//! `one_request`, which always serves through `exact` (isabelle/Fabric_Broker/Fabric_Broker.thy,
//! `exact` and `authorizes`). It is never `session`: a session is offered below `until_revoked`.
//! The function reads stored facts and two facts of this run, never a prover (D40).
//!
//! The table, by tool and row mode:
//! - `fabric_calendar_propose_create` whose row is allow: `until_revoked`, when the four
//!   conditions below hold. The row is `create_in_agent_drafts` when any entry of the calendar
//!   has Agent Drafts, as the theory's `drafts` reads it, else `create_in_existing_calendar`.
//! - A create whose row asks or blocks: `one_request`. Under asks the theory serves no lasting
//!   grant of a create (`standing_serves`).
//! - Availability and reads: `one_request`, a deliberate under-proposal while no read finishes
//!   in this build (a fresh Permit of a read is `undecided`, `read_tool`). Lifted with CD-W05
//!   and CD-W06, when a read's result is held and released.
//! - Updates, invitations and responses: `one_request`, a deliberate under-proposal while the
//!   core holds no observed event and cannot tell the rows or the attendees apart. Lifted with
//!   CD-W05 and a theory statement of when a lasting grant serves (PW K14).
//! - Deletes: `one_request`, always; no lasting grant covers a delete (`scope_covers`).
//! - The list tool and a result request: no card ever opens; `one_request`, defensively.
//!
//! The `until_revoked` cell rests on these block rules of the theory, each implied by an input:
//! - `Actor`: the request is an agent's; every card is.
//! - `Mode_not_blocked`: the row's mode is allow.
//! - `Account_connected` and `Calendar_selected`: the calendar is among `calendars`, which holds
//!   only selected calendars of connected accounts (`facts::selected_calendars`).
//! - `Calendar_writable`: an entry of the calendar is writable, as the theory's `writable`.
//! - `Relied_grant_live`: an `until_revoked` grant is live until it ends; a `session` grant
//!   while its session is (`create_grant` binds only a live one).
//! - `Agent_not_paused` is no input: a paused agent's fresh proof blocks, and the lasting grant
//!   stays (D20, c38).
//! - `Event_known`, `Revision_current`, `Event_supported`, `Fabric_created`,
//!   `Invitation_target`, `Result_held`, `Warnings_owner_only` and `System_authority` do not
//!   apply to a create. Neither do `No_strangers` and `Invitation_authority`: a create has no
//!   `Row_invite`. `Authority` holds by `standing_serves` under allow and `scope_covers` with
//!   the grant's one tool and the request's calendar.
//!
//! "A row in block never gets a card" holds with the Isabelle kind only: the synthetic
//! classifier may ask where the theory blocks. Such a card proposes `one_request`, because a
//! run without the Isabelle kind cannot carry out a change (D40, `can_change`).

use fabric_desktop_contracts::agent::Action;
use fabric_desktop_contracts::owner::{DurationKind, Mode, PolicyRow, ProfileRows};

use crate::ports::CalendarFact;

/// The duration a card proposes for `action` (D40): `until_revoked` only when
/// 1. the action is a create whose row in `profile` is allow;
/// 2. its calendar is among `calendars` (selected, of a connected account) and writable;
/// 3. `can_change`: this run can carry out a change, the prover's kind is Isabelle and a
///    provider is attached, the two facts `pipeline::accepted` checks before it authorizes;
/// 4. not `lasting_origin`: no lasting grant of the agent already has this request as its
///    origin, so a request asked again after a lasting approval never adds another one.
///
/// `one_request` in every other case, including every fact it cannot read. Never `session`.
#[must_use]
pub(crate) fn proposed_duration(
    action: &Action,
    profile: &ProfileRows,
    calendars: &[CalendarFact],
    can_change: bool,
    lasting_origin: bool,
) -> DurationKind {
    if !can_change || lasting_origin {
        return DurationKind::OneRequest;
    }
    let Action::ProposeCreate(create) = action else {
        return DurationKind::OneRequest;
    };
    let entries: Vec<&CalendarFact> = calendars
        .iter()
        .filter(|c| {
            c.calendar.account_id == create.account_id
                && c.calendar.calendar_id == create.calendar_id
        })
        .collect();
    if !entries.iter().any(|c| c.writable) {
        // Missing (deselected, disconnected, unknown) or read-only: fail closed.
        return DurationKind::OneRequest;
    }
    let row = if entries.iter().any(|c| c.agent_drafts) {
        PolicyRow::CreateInAgentDrafts
    } else {
        PolicyRow::CreateInExistingCalendar
    };
    if profile.mode(row) == Mode::Allow {
        DurationKind::UntilRevoked
    } else {
        DurationKind::OneRequest
    }
}

/// The order of durations, from the shortest (README section 10): `one_request`, `session`,
/// `until_revoked`. `DurationKind` derives no order.
#[must_use]
pub(crate) const fn rank(kind: DurationKind) -> u8 {
    match kind {
        DurationKind::OneRequest => 0,
        DurationKind::Session => 1,
        DurationKind::UntilRevoked => 2,
    }
}

/// True when `chosen` is longer than the card's `ceiling`, so `decide_request` refuses it.
#[must_use]
pub(crate) const fn longer(chosen: DurationKind, ceiling: DurationKind) -> bool {
    rank(chosen) > rank(ceiling)
}

#[cfg(test)]
mod tests {
    use fabric_desktop_contracts::common::{CalendarRef, ToolName};
    use fabric_desktop_contracts::scalar::{Fixed, Id};
    use serde_json::{Value, json};

    use super::*;

    const ACCOUNT: &str = "acct-1";
    const CALENDAR: &str = "cal-primary";

    fn id(text: &str) -> Id {
        Id::new(text).unwrap()
    }

    fn action(tool: ToolName, params: Value) -> Action {
        Action::from_parts::<serde_json::Error>(tool, params)
            .unwrap()
            .unwrap()
    }

    fn create_on(account: &str, calendar: &str) -> Action {
        action(
            ToolName::ProposeCreate,
            json!({
                "account_id": account,
                "calendar_id": calendar,
                "event": {
                    "title": "Planning block",
                    "start": {"date_time": "2026-11-20T10:00:00-06:00", "zone": "America/Chicago"},
                    "end": {"date_time": "2026-11-20T11:00:00-06:00", "zone": "America/Chicago"},
                    "description": null,
                    "location": null
                },
                "task": "Block time to plan next week."
            }),
        )
    }

    /// One action of every tool but the create, on `CALENDAR`.
    fn other_actions() -> Vec<Action> {
        let event = json!({"event_id": "evt-fabric-0042", "revision": "3181161799000000"});
        let window =
            json!({"start": "2026-11-20T06:00:00.000Z", "end": "2026-11-21T06:00:00.000Z"});
        vec![
            action(ToolName::CalendarList, json!({})),
            action(
                ToolName::CalendarAvailability,
                json!({"account_id": ACCOUNT, "calendar_ids": [CALENDAR], "window": window,
                       "task": "Find a free hour."}),
            ),
            action(
                ToolName::CalendarRead,
                json!({"account_id": ACCOUNT, "calendar_ids": [CALENDAR], "window": window,
                       "fields": ["time", "title"], "task": "See what is on Friday."}),
            ),
            action(
                ToolName::ProposeUpdate,
                json!({"account_id": ACCOUNT, "calendar_id": CALENDAR, "event": event,
                       "changes": {"title": "Design review", "start": null, "end": null,
                                   "description": null, "location": null},
                       "send_updates": "none", "task": "Rename the event."}),
            ),
            action(
                ToolName::ProposeInvitation,
                json!({"account_id": ACCOUNT, "calendar_id": CALENDAR, "event": event,
                       "attendees": ["ally@example.net"], "send_updates": "all",
                       "task": "Invite the team."}),
            ),
            action(
                ToolName::ProposeResponse,
                json!({"account_id": ACCOUNT, "calendar_id": CALENDAR,
                       "target": {"kind": "event", "event": event}, "response": "accepted",
                       "send_updates": "all", "task": "Answer the invitation."}),
            ),
            action(
                ToolName::ProposeDelete,
                json!({"account_id": ACCOUNT, "calendar_id": CALENDAR, "event": event,
                       "send_updates": "none", "task": "Remove the draft I made."}),
            ),
            action(ToolName::RequestResult, json!({"request_id": "request-25"})),
        ]
    }

    fn fact(account: &str, calendar: &str, writable: bool, agent_drafts: bool) -> CalendarFact {
        CalendarFact {
            calendar: CalendarRef {
                account_id: id(account),
                calendar_id: id(calendar),
            },
            writable,
            agent_drafts,
        }
    }

    /// A profile with every changeable row in `mode`.
    fn all(mode: Mode) -> ProfileRows {
        ProfileRows {
            list_and_availability: mode,
            read_event_details: mode,
            create_in_agent_drafts: mode,
            create_in_existing_calendar: mode,
            edit_fabric_created_event: mode,
            edit_preexisting_event: mode,
            send_or_change_invitations: mode,
            respond_to_invitation: mode,
            delete_event: Fixed::new(),
            delete_calendar_or_change_sharing: Fixed::new(),
        }
    }

    fn profiles() -> Vec<ProfileRows> {
        vec![
            ProfileRows::CAREFUL,
            ProfileRows::FLEXIBLE,
            all(Mode::Allow),
            all(Mode::Ask),
            all(Mode::Block),
        ]
    }

    /// The calendar kinds a create can name: Agent Drafts, plain, read-only, absent.
    fn calendar_kinds() -> Vec<(&'static str, Vec<CalendarFact>)> {
        vec![
            ("drafts", vec![fact(ACCOUNT, CALENDAR, true, true)]),
            ("plain", vec![fact(ACCOUNT, CALENDAR, true, false)]),
            ("read-only", vec![fact(ACCOUNT, CALENDAR, false, false)]),
            (
                "read-only drafts",
                vec![fact(ACCOUNT, CALENDAR, false, true)],
            ),
            ("absent", vec![fact(ACCOUNT, "cal-other", true, true)]),
        ]
    }

    /// The table's answer, written out by hand from the theory.
    fn expected(profile: &ProfileRows, kind: &str) -> DurationKind {
        let row = match kind {
            "drafts" => PolicyRow::CreateInAgentDrafts,
            "plain" => PolicyRow::CreateInExistingCalendar,
            _ => return DurationKind::OneRequest,
        };
        if profile.mode(row) == Mode::Allow {
            DurationKind::UntilRevoked
        } else {
            DurationKind::OneRequest
        }
    }

    #[test]
    fn the_table_proposes_until_revoked_only_for_an_allowed_create_in_a_writable_calendar() {
        // c37: every tool, every mode, every calendar kind, in a run that can carry out a change.
        for profile in profiles() {
            for (kind, calendars) in calendar_kinds() {
                assert_eq!(
                    proposed_duration(
                        &create_on(ACCOUNT, CALENDAR),
                        &profile,
                        &calendars,
                        true,
                        false
                    ),
                    expected(&profile, kind),
                    "a create on a {kind} calendar under {profile:?}"
                );
                for other in other_actions() {
                    assert_eq!(
                        proposed_duration(&other, &profile, &calendars, true, false),
                        DurationKind::OneRequest,
                        "{:?} on a {kind} calendar under {profile:?}",
                        other.tool()
                    );
                }
            }
        }
        // The two built-in cells that propose until_revoked.
        let drafts = [fact(ACCOUNT, CALENDAR, true, true)];
        let plain = [fact(ACCOUNT, CALENDAR, true, false)];
        let create = create_on(ACCOUNT, CALENDAR);
        let careful = ProfileRows::CAREFUL;
        let flexible = ProfileRows::FLEXIBLE;
        assert_eq!(
            proposed_duration(&create, &careful, &drafts, true, false),
            DurationKind::UntilRevoked
        );
        assert_eq!(
            proposed_duration(&create, &careful, &plain, true, false),
            DurationKind::OneRequest
        );
        assert_eq!(
            proposed_duration(&create, &flexible, &plain, true, false),
            DurationKind::UntilRevoked
        );
    }

    #[test]
    fn a_custom_profile_chooses_the_row_by_the_calendar() {
        // The drafts row and the existing-calendar row are never swapped.
        let create = create_on(ACCOUNT, CALENDAR);
        let drafts = [fact(ACCOUNT, CALENDAR, true, true)];
        let plain = [fact(ACCOUNT, CALENDAR, true, false)];
        let existing_only = ProfileRows {
            create_in_agent_drafts: Mode::Ask,
            create_in_existing_calendar: Mode::Allow,
            ..ProfileRows::CAREFUL
        };
        assert_eq!(
            proposed_duration(&create, &existing_only, &plain, true, false),
            DurationKind::UntilRevoked
        );
        assert_eq!(
            proposed_duration(&create, &existing_only, &drafts, true, false),
            DurationKind::OneRequest
        );
        let drafts_blocked = ProfileRows {
            create_in_agent_drafts: Mode::Block,
            ..ProfileRows::FLEXIBLE
        };
        assert_eq!(
            proposed_duration(&create, &drafts_blocked, &drafts, true, false),
            DurationKind::OneRequest
        );
    }

    #[test]
    fn a_duplicated_calendar_entry_reads_as_the_theory_reads_it() {
        // c37: `drafts` and `writable` hold when any entry with the account and calendar has the
        // mark. A duplicated entry with Agent Drafts makes the row create_in_agent_drafts.
        let create = create_on(ACCOUNT, CALENDAR);
        let duplicated = [
            fact(ACCOUNT, CALENDAR, true, false),
            fact(ACCOUNT, CALENDAR, false, true),
        ];
        let drafts_only = ProfileRows {
            create_in_agent_drafts: Mode::Allow,
            create_in_existing_calendar: Mode::Ask,
            ..ProfileRows::CAREFUL
        };
        assert_eq!(
            proposed_duration(&create, &drafts_only, &duplicated, true, false),
            DurationKind::UntilRevoked
        );
        let existing_only = ProfileRows {
            create_in_agent_drafts: Mode::Ask,
            create_in_existing_calendar: Mode::Allow,
            ..ProfileRows::CAREFUL
        };
        assert_eq!(
            proposed_duration(&create, &existing_only, &duplicated, true, false),
            DurationKind::OneRequest
        );
        // Two read-only entries are still read-only.
        let read_only = [
            fact(ACCOUNT, CALENDAR, false, true),
            fact(ACCOUNT, CALENDAR, false, false),
        ];
        assert_eq!(
            proposed_duration(&create, &ProfileRows::FLEXIBLE, &read_only, true, false),
            DurationKind::OneRequest
        );
    }

    #[test]
    fn a_calendar_missing_from_the_facts_fails_closed() {
        // c37: deselected, disconnected or unknown calendars are absent from the facts, and the
        // same calendar id under another account is another calendar.
        let create = create_on(ACCOUNT, CALENDAR);
        for calendars in [
            Vec::new(),
            vec![fact("acct-2", CALENDAR, true, true)],
            vec![fact(ACCOUNT, "cal-other", true, true)],
        ] {
            assert_eq!(
                proposed_duration(&create, &ProfileRows::FLEXIBLE, &calendars, true, false),
                DurationKind::OneRequest,
                "{calendars:?}"
            );
        }
    }

    #[test]
    fn a_run_that_cannot_carry_out_a_change_proposes_one_request() {
        // c37: the synthetic classifier, or no provider.
        let drafts = [fact(ACCOUNT, CALENDAR, true, true)];
        for profile in profiles() {
            assert_eq!(
                proposed_duration(
                    &create_on(ACCOUNT, CALENDAR),
                    &profile,
                    &drafts,
                    false,
                    false
                ),
                DurationKind::OneRequest
            );
        }
    }

    #[test]
    fn a_request_that_is_the_origin_of_a_lasting_grant_proposes_one_request() {
        // c37: asked again after a lasting approval, the request never gets another
        // lasting grant, whatever the latest approval was.
        let drafts = [fact(ACCOUNT, CALENDAR, true, true)];
        let create = create_on(ACCOUNT, CALENDAR);
        assert_eq!(
            proposed_duration(&create, &ProfileRows::CAREFUL, &drafts, true, true),
            DurationKind::OneRequest
        );
        assert_eq!(
            proposed_duration(&create, &ProfileRows::CAREFUL, &drafts, true, false),
            DurationKind::UntilRevoked
        );
    }

    #[test]
    fn the_function_never_proposes_a_session() {
        let mut calendars = Vec::new();
        for (_, kind) in calendar_kinds() {
            calendars.extend(kind);
        }
        let mut actions = other_actions();
        actions.push(create_on(ACCOUNT, CALENDAR));
        for profile in profiles() {
            for action in &actions {
                for can_change in [true, false] {
                    for lasting_origin in [true, false] {
                        assert_ne!(
                            proposed_duration(
                                action,
                                &profile,
                                &calendars,
                                can_change,
                                lasting_origin
                            ),
                            DurationKind::Session
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_duration_is_longer_only_above_the_ceiling() {
        // c37: all nine pairs.
        use DurationKind::{OneRequest, Session, UntilRevoked};
        let cases = [
            (OneRequest, OneRequest, false),
            (OneRequest, Session, false),
            (OneRequest, UntilRevoked, false),
            (Session, OneRequest, true),
            (Session, Session, false),
            (Session, UntilRevoked, false),
            (UntilRevoked, OneRequest, true),
            (UntilRevoked, Session, true),
            (UntilRevoked, UntilRevoked, false),
        ];
        for (chosen, ceiling, refused) in cases {
            assert_eq!(
                longer(chosen, ceiling),
                refused,
                "{chosen:?} on {ceiling:?}"
            );
        }
        assert!(rank(OneRequest) < rank(Session) && rank(Session) < rank(UntilRevoked));
    }
}
