//! The shape of the real lasting grant, as the rule gate sees it (p09 of the CD-W04 design,
//! `docs/plans/calendar-desktop-w04-design.md`; D40).
//!
//! The real core is driven through a proposal, its card and an approval until revoked, in a run
//! that can carry out a change (an Isabelle prover and a provider attached, D40). The
//! test takes the fresh-permit job the core hands out and maps it with the adapter's
//! `context_of`, the same function the prover worker calls. It checks the facts the table's
//! `until_revoked` cells rest on: the grant names the create tool only, the request's calendar
//! only, no window, fields, recipients or notice choice, and the request rests on it. Both cells
//! that propose `until_revoked` are covered: a create in Agent Drafts under Careful, and a create
//! in a plain calendar under Flexible. No Isabelle runs here; the live test
//! `the_proposed_duration_agrees_with_the_rule_theory` in `tests/live.rs` checks the same grants
//! against the theory.

#![cfg(debug_assertions)]

mod support;

use fabric_broker::prover::map::context_of;
use fabric_broker_core::ports::ProofJob;
use fabric_broker_policy::{CalendarKey, Duration, Grant, Profile, Tool};
use fabric_desktop_contracts::owner::ProfileRows;
use support::{ACCOUNT, CALENDAR, Core, DRAFTS, create_params};

const CREATE: &str = "fabric_calendar_propose_create";

/// The mapped grant `grant_id` of `job`, after checking that the context rests on it alone.
fn mapped(job: &ProofJob, grant_id: &str) -> (Grant, fabric_broker_policy::Context) {
    let (context, _) = context_of(job).expect("the job maps");
    assert_eq!(context.relied, [grant_id.to_owned()]);
    let grant = context
        .grants
        .iter()
        .find(|g| g.id == grant_id)
        .cloned()
        .unwrap_or_else(|| panic!("the context offers no grant {grant_id}"));
    (grant, context)
}

/// Asserts the shape `create_grant` gives a create's grant on `calendar`, whatever its term.
fn assert_create_shape(grant: &Grant, calendar: &str) {
    assert_eq!(grant.tools, [Tool::Create]);
    assert!(!grant.ended);
    assert_eq!(grant.calendars, [CalendarKey::new(ACCOUNT, calendar)]);
    assert_eq!(grant.window, None);
    assert!(grant.fields.is_empty());
    assert_eq!(grant.recipients, None);
    assert_eq!(grant.send_updates, None);
}

/// p09, the Careful cell: a create in Agent Drafts proposes until_revoked; the grant of an
/// approval until revoked, and of one for the session, maps to the create tool on that calendar
/// alone, and the context marks the calendar Agent Drafts under Careful's rows.
#[test]
fn a_lasting_grant_for_a_create_in_agent_drafts_maps_to_the_create_on_that_calendar() {
    let mut core = Core::start("a1-careful");
    let request_id = core.pending(CREATE, create_params(DRAFTS));
    assert_eq!(core.proposed(&request_id), "until_revoked");
    let (grant_id, job) = core.approve(&request_id, "until_revoked");
    let (grant, context) = mapped(&job, &grant_id);
    assert_create_shape(&grant, DRAFTS);
    assert_eq!(grant.duration, Duration::UntilRevoked);
    assert_eq!(context.profile, Profile::from(&ProfileRows::CAREFUL));
    let drafts = context
        .calendars
        .iter()
        .find(|c| c.account == ACCOUNT && c.id == DRAFTS)
        .expect("the context names the Agent Drafts calendar");
    assert!(drafts.drafts && drafts.writable);

    let request_id = core.pending(CREATE, create_params(DRAFTS));
    assert_eq!(core.proposed(&request_id), "until_revoked");
    let (grant_id, job) = core.approve(&request_id, "session");
    let session = job
        .facts
        .session
        .as_ref()
        .expect("the agent's live session is a fact")
        .session_id
        .as_str()
        .to_owned();
    let (grant, _) = mapped(&job, &grant_id);
    assert_create_shape(&grant, DRAFTS);
    assert_eq!(
        grant.duration,
        Duration::Session {
            session_id: session
        }
    );
}

/// p09, the Flexible cell: a create in a plain calendar proposes until_revoked under Flexible;
/// the grant maps to the create tool on that calendar alone, under Flexible's rows.
#[test]
fn a_lasting_grant_for_a_create_in_a_plain_calendar_maps_to_the_create_on_that_calendar() {
    let mut core = Core::start("a1-flexible");
    core.activate("flexible");
    let request_id = core.pending(CREATE, create_params(CALENDAR));
    assert_eq!(core.proposed(&request_id), "until_revoked");
    let (grant_id, job) = core.approve(&request_id, "until_revoked");
    let (grant, context) = mapped(&job, &grant_id);
    assert_create_shape(&grant, CALENDAR);
    assert_eq!(grant.duration, Duration::UntilRevoked);
    assert_eq!(context.profile, Profile::from(&ProfileRows::FLEXIBLE));
    let calendar = context
        .calendars
        .iter()
        .find(|c| c.account == ACCOUNT && c.id == CALENDAR)
        .expect("the context names the calendar");
    assert!(!calendar.drafts && calendar.writable);
}
