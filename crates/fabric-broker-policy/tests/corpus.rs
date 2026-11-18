//! The broker's worked cases decided by a live Isabelle session. It needs Isabelle from
//! `scripts/setup-demo.sh`, so it is ignored by default:
//!
//!   sh scripts/run.sh cargo test -p fabric-broker-policy --test corpus -- --ignored --nocapture
//!
//! `every_case_is_decided_as_expected_by_isabelle` rebuilds every statement of
//! `isabelle/Fabric_Broker_Examples/Broker_Examples.thy` that states an `outcome` or a
//! `first_failing`, under the lemma's name, with the expectation the examples file states.
//! The examples' synthetic digests `''d-this''` and `''d-other''` become two fixed 64-hex
//! digests, since the gate accepts only real digests; only their equality matters.

mod common;

use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration as StdDuration, Instant};

use fabric_broker_policy::{
    Action, ActiveSession, Actor, BrokerGate, Calendar, CalendarKey, Context, Duration, Event,
    EventField, GateConfig, Grant, Mode, Outcome, PendingEffect, Profile, Recipient, RecipientRule,
    RecipientSet, Response, Rule, RuleOutcome, SendUpdates, Target, Tool, Verdict, Window,
};

/// The live tests run one at a time, so the timings are not disturbed by another session.
static LIVE: Mutex<()> = Mutex::new(());

const D_THIS: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const D_OTHER: &str = "2222222222222222222222222222222222222222222222222222222222222222";

fn repository() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives two levels below the repository root")
}

/// The server's name ends in this process's id: `isabelle server` reuses a running server of
/// the same name, and every checkout on a machine shares one Isabelle user home, so two runs
/// with one name would share a server and the first to finish would stop it under the other.
fn config(name: &str) -> GateConfig {
    let mut config = GateConfig::for_repository(repository());
    config.work_dir = repository().join(".local").join(name);
    config.server_name = format!("{name}-{}", std::process::id());
    config
}

/// A character-code list of the examples, such as `[100]`, as text.
fn c(codes: &[u32]) -> String {
    codes
        .iter()
        .map(|&code| char::from_u32(code).unwrap())
        .collect()
}

fn s(text: &str) -> String {
    text.to_owned()
}

fn key(calendar: &str) -> CalendarKey {
    CalendarKey::new("acct-1", calendar)
}

fn calendar(id: &str, writable: bool, drafts: bool) -> Calendar {
    Calendar {
        account: s("acct-1"),
        id: s(id),
        writable,
        drafts,
    }
}

/// `base`.
fn base() -> Context {
    Context {
        actor: Actor::Agent(s("agent-1")),
        paused: false,
        now_ms: 1000,
        session: Some(ActiveSession {
            id: s("session-1"),
            last_active_ms: 0,
        }),
        profile: Profile::CAREFUL,
        connected: vec![s("acct-1")],
        calendars: vec![
            calendar("drafts", true, true),
            calendar("primary", true, false),
            calendar("team", true, false),
            calendar("readonly", false, false),
        ],
        warning_calendars: Vec::new(),
        grants: Vec::new(),
        recipient_sets: Vec::new(),
        event: None,
        digest: s(D_THIS),
        relied: Vec::new(),
        result_held: false,
        pending_effects: Vec::new(),
        pause_epoch: 0,
        revocation_epoch: 0,
    }
}

/// `system`.
fn system() -> Context {
    Context {
        actor: Actor::System,
        paused: true,
        session: None,
        ..base()
    }
}

fn with(context: Context, edit: impl FnOnce(&mut Context)) -> Context {
    let mut context = context;
    edit(&mut context);
    context
}

fn grants(grants: Vec<Grant>) -> Context {
    with(base(), |c| c.grants = grants)
}

/// `approval`.
fn approval() -> Grant {
    Grant {
        id: s("g-approval"),
        tools: Vec::new(),
        duration: Duration::OneRequest {
            digest: s(D_THIS),
            used: false,
        },
        ended: false,
        calendars: Vec::new(),
        window: None,
        fields: Vec::new(),
        recipients: None,
        send_updates: None,
    }
}

/// `standing_grant`.
fn standing_grant() -> Grant {
    Grant {
        id: s("g-standing"),
        duration: Duration::UntilRevoked,
        ..approval()
    }
}

fn availability_grant() -> Grant {
    Grant {
        id: s("g-availability"),
        tools: vec![Tool::List, Tool::Availability],
        calendars: vec![key("primary")],
        window: Some(Window {
            start_ms: 0,
            end_ms: 1000,
        }),
        ..standing_grant()
    }
}

fn read_grant() -> Grant {
    Grant {
        id: s("g-read"),
        tools: vec![Tool::Read],
        duration: Duration::Session {
            session_id: s("session-1"),
        },
        calendars: vec![key("primary")],
        window: Some(Window {
            start_ms: 0,
            end_ms: 1000,
        }),
        fields: vec![EventField::Time, EventField::Title],
        ..standing_grant()
    }
}

fn create_grant() -> Grant {
    Grant {
        id: s("g-create"),
        tools: vec![Tool::Create],
        calendars: vec![key("drafts"), key("team")],
        ..standing_grant()
    }
}

fn edit_grant() -> Grant {
    Grant {
        id: s("g-edit"),
        tools: vec![Tool::Update],
        calendars: vec![key("drafts"), key("team")],
        ..standing_grant()
    }
}

fn invite_grant() -> Grant {
    Grant {
        id: s("g-invite"),
        tools: vec![Tool::Invite],
        calendars: vec![key("team")],
        recipients: Some(RecipientRule::Sets(vec![s("set-team")])),
        send_updates: Some(SendUpdates::NotifyAll),
        ..standing_grant()
    }
}

fn one_read() -> Grant {
    Grant {
        id: s("g-read-once"),
        ..approval()
    }
}

fn one_request(digest: &str, used: bool) -> Duration {
    Duration::OneRequest {
        digest: s(digest),
        used,
    }
}

fn recipient(address: u32, domain: u32) -> Recipient {
    Recipient {
        address: c(&[address]),
        domain: c(&[domain]),
    }
}

fn bob() -> Recipient {
    recipient(1, 10)
}

fn carol() -> Recipient {
    recipient(2, 10)
}

fn dan() -> Recipient {
    recipient(3, 11)
}

fn team() -> RecipientSet {
    RecipientSet {
        id: s("set-team"),
        members: vec![c(&[1]), c(&[2])],
        domains: Vec::new(),
    }
}

fn own_event() -> Event {
    Event {
        id: c(&[100]),
        revision: c(&[7]),
        fabric_created: true,
        recurring: false,
        default_type: true,
        timed: true,
        invitation: false,
        attendees: Vec::new(),
        occurrence: None,
    }
}

fn invitation() -> Event {
    Event {
        id: c(&[200]),
        fabric_created: false,
        invitation: true,
        ..own_event()
    }
}

fn occurrence() -> Event {
    Event {
        id: c(&[201]),
        recurring: true,
        occurrence: Some(5000),
        ..invitation()
    }
}

fn widest() -> Profile {
    Profile {
        list: Mode::Allows,
        read: Mode::Allows,
        drafts: Mode::Allows,
        existing: Mode::Allows,
        edit_fabric: Mode::Allows,
        edit_other: Mode::Allows,
        invite: Mode::Allows,
        respond: Mode::Allows,
        delete: Mode::Allows,
        sharing: Mode::Allows,
    }
}

fn availability(calendars: &[&str], start_ms: i64, end_ms: i64) -> Action {
    Action::ReadAvailability {
        account: s("acct-1"),
        calendars: calendars.iter().map(|id| s(id)).collect(),
        start_ms,
        end_ms,
    }
}

fn read(start_ms: i64, end_ms: i64, fields: &[EventField]) -> Action {
    Action::ReadEvents {
        account: s("acct-1"),
        calendars: vec![s("primary")],
        start_ms,
        end_ms,
        fields: fields.to_vec(),
    }
}

/// `the_read`.
fn the_read() -> Action {
    read(10, 20, &[EventField::Title])
}

fn create(account: &str, calendar: &str) -> Action {
    Action::CreateEvent {
        account: s(account),
        calendar: s(calendar),
    }
}

fn update(calendar: &str, send_updates: SendUpdates) -> Action {
    Action::UpdateEvent {
        account: s("acct-1"),
        calendar: s(calendar),
        event_id: c(&[100]),
        revision: c(&[7]),
        send_updates,
    }
}

/// `edit_own`.
fn edit_own() -> Action {
    update("team", SendUpdates::NotifyAll)
}

/// `invite rs`.
fn invite(attendees: Vec<Recipient>) -> Action {
    Action::ChangeInvitation {
        account: s("acct-1"),
        calendar: s("team"),
        event_id: c(&[100]),
        revision: c(&[7]),
        attendees,
        send_updates: SendUpdates::NotifyAll,
    }
}

fn respond(target: Target, response: Response) -> Action {
    Action::Respond {
        account: s("acct-1"),
        calendar: s("primary"),
        target,
        response,
        send_updates: SendUpdates::NotifyAll,
    }
}

fn whole(event: u32, revision: u32) -> Target {
    Target::WholeEvent {
        event_id: c(&[event]),
        revision: c(&[revision]),
    }
}

fn occurrence_target(instance: u32, start: i64, revision: u32) -> Target {
    Target::Occurrence {
        instance_id: c(&[instance]),
        original_start_ms: start,
        revision: c(&[revision]),
    }
}

/// `delete_own`.
fn delete_own() -> Action {
    Action::DeleteEvent {
        account: s("acct-1"),
        calendar: s("team"),
        event_id: c(&[100]),
        revision: c(&[7]),
        send_updates: SendUpdates::NotifyAll,
    }
}

fn system_refresh(calendar: &str) -> Action {
    Action::SystemRefresh {
        account: s("acct-1"),
        calendar: s(calendar),
    }
}

/// One lemma of the section on lasting approvals: its name, the profile, the grant the
/// approval made, the action, the event the action names, and the rule that fails first when
/// the grant does not serve.
type LastingCell = (
    &'static str,
    Profile,
    Grant,
    Action,
    Option<Event>,
    Option<Rule>,
);

struct Case {
    name: String,
    context: Context,
    action: Action,
    /// The outcome the examples state, when they state one.
    outcome: Option<Outcome>,
    /// The first failing rule the examples state, when they state one.
    failing: Option<Rule>,
    /// For each grant of the context in order, whether it authorizes the action, when the
    /// examples state it (or it follows from a stated permit with a single grant, by the
    /// theory's lemma `permit_rests_on_an_authorizing_grant`).
    authorizes: Option<Vec<bool>>,
}

#[allow(
    clippy::too_many_lines,
    reason = "one entry per statement of the examples file"
)]
fn cases() -> Vec<Case> {
    use Outcome::{Block, NeedApproval as Ask, Permit};
    let mut cases = Vec::new();
    let mut add = |name: &str, context: Context, action: Action, outcome, failing| {
        cases.push(Case {
            name: name.to_owned(),
            context,
            action,
            outcome,
            failing,
            authorizes: None,
        });
    };

    // Reading (CD-M01, CD-M02).
    add(
        "availability_without_a_grant_asks",
        base(),
        availability(&["primary"], 0, 100),
        Some(Ask),
        Some(Rule::Authority),
    );
    let available = || grants(vec![availability_grant()]);
    add(
        "availability_inside_a_grant_is_permitted (1)",
        available(),
        availability(&["primary"], 0, 100),
        Some(Permit),
        None,
    );
    add(
        "availability_inside_a_grant_is_permitted (2)",
        available(),
        Action::ListCalendars,
        Some(Permit),
        None,
    );
    add(
        "availability_outside_the_grants_window_or_calendars_asks (1)",
        available(),
        availability(&["primary"], 0, 1001),
        Some(Ask),
        None,
    );
    add(
        "availability_outside_the_grants_window_or_calendars_asks (2)",
        available(),
        availability(&["primary", "team"], 0, 100),
        Some(Ask),
        None,
    );
    add(
        "an_availability_grant_does_not_cover_event_details",
        available(),
        read(0, 100, &[EventField::Time]),
        Some(Ask),
        None,
    );
    let reading = || grants(vec![read_grant()]);
    add(
        "careful_reads_run_inside_the_approved_grant",
        reading(),
        the_read(),
        Some(Permit),
        None,
    );
    add(
        "a_field_or_a_time_outside_the_grant_asks (1)",
        reading(),
        read(10, 20, &[EventField::Title, EventField::Attendees]),
        Some(Ask),
        None,
    );
    add(
        "a_field_or_a_time_outside_the_grant_asks (2)",
        reading(),
        read(10, 2000, &[EventField::Title]),
        Some(Ask),
        None,
    );
    add(
        "a_session_grant_stops_at_the_idle_timeout (1)",
        with(reading(), |c| c.now_ms = 1_799_999),
        the_read(),
        Some(Permit),
        None,
    );
    add(
        "a_session_grant_stops_at_the_idle_timeout (2)",
        with(reading(), |c| c.now_ms = 1_800_000),
        the_read(),
        Some(Ask),
        None,
    );
    add(
        "a_session_grant_stops_at_the_idle_timeout (3)",
        with(reading(), |c| {
            c.session = Some(ActiveSession {
                id: s("session-2"),
                last_active_ms: 0,
            });
        }),
        the_read(),
        Some(Ask),
        None,
    );
    add(
        "a_session_grant_stops_at_the_idle_timeout (4)",
        with(reading(), |c| c.session = None),
        the_read(),
        Some(Ask),
        None,
    );
    add(
        "a_request_is_blocked_when_its_grant_is_revoked_or_expires (1, 2)",
        with(base(), |c| {
            c.grants = vec![Grant {
                ended: true,
                ..read_grant()
            }];
            c.relied = vec![s("g-read")];
        }),
        the_read(),
        Some(Block),
        Some(Rule::ReliedGrantLive),
    );
    add(
        "a_request_is_blocked_when_its_grant_is_revoked_or_expires (3)",
        with(reading(), |c| {
            c.relied = vec![s("g-read")];
            c.now_ms = 1_800_000;
        }),
        the_read(),
        Some(Block),
        None,
    );
    add(
        "a_blocked_row_is_blocked_inside_a_grant (1, 2)",
        with(reading(), |c| {
            c.profile = Profile {
                read: Mode::Blocks,
                ..Profile::CAREFUL
            };
        }),
        the_read(),
        Some(Block),
        Some(Rule::ModeNotBlocked),
    );

    // Creating (CD-M03, CD-M04).
    let creating = |profile: Profile| with(grants(vec![create_grant()]), |c| c.profile = profile);
    add(
        "creating_in_agent_drafts_is_allowed_in_a_grant (1)",
        creating(Profile::CAREFUL),
        create("acct-1", "drafts"),
        Some(Permit),
        None,
    );
    add(
        "creating_in_agent_drafts_is_allowed_in_a_grant (2)",
        base(),
        create("acct-1", "drafts"),
        Some(Ask),
        None,
    );
    add(
        "careful_asks_for_each_create_in_an_existing_calendar (1)",
        creating(Profile::CAREFUL),
        create("acct-1", "team"),
        Some(Ask),
        None,
    );
    add(
        "careful_asks_for_each_create_in_an_existing_calendar (2)",
        grants(vec![approval()]),
        create("acct-1", "team"),
        Some(Permit),
        None,
    );
    add(
        "flexible_allows_creates_in_the_calendars_a_grant_names (1)",
        creating(Profile::FLEXIBLE),
        create("acct-1", "team"),
        Some(Permit),
        None,
    );
    add(
        "flexible_allows_creates_in_the_calendars_a_grant_names (2)",
        creating(Profile::FLEXIBLE),
        create("acct-1", "primary"),
        Some(Ask),
        None,
    );
    add(
        "switching_to_careful_stops_a_standing_grant (1)",
        creating(Profile::FLEXIBLE),
        create("acct-1", "team"),
        Some(Permit),
        None,
    );
    add(
        "switching_to_careful_stops_a_standing_grant (2)",
        creating(Profile::CAREFUL),
        create("acct-1", "team"),
        Some(Ask),
        None,
    );
    let approved = || grants(vec![approval()]);
    add(
        "creates_that_are_blocked (1, 2)",
        approved(),
        create("acct-1", "readonly"),
        Some(Block),
        Some(Rule::CalendarWritable),
    );
    add(
        "creates_that_are_blocked (3)",
        approved(),
        create("acct-1", "unselected"),
        None,
        Some(Rule::CalendarSelected),
    );
    add(
        "creates_that_are_blocked (4)",
        approved(),
        create("acct-2", "primary"),
        None,
        Some(Rule::AccountConnected),
    );
    add(
        "creates_that_are_blocked (5)",
        with(approved(), |c| c.paused = true),
        create("acct-1", "team"),
        None,
        Some(Rule::AgentNotPaused),
    );

    // Editing (CD-M05, CD-M06, CD-M07).
    let editing = |profile: Profile, grants: Vec<Grant>, sets: Vec<RecipientSet>, event: Event| {
        with(base(), |c| {
            c.profile = profile;
            c.grants = grants;
            c.recipient_sets = sets;
            c.event = Some(event);
        })
    };
    let attended = |attendees: Vec<Recipient>| Event {
        attendees,
        ..own_event()
    };
    add(
        "flexible_edits_a_fabric_created_event_inside_a_grant",
        editing(Profile::FLEXIBLE, vec![edit_grant()], vec![], own_event()),
        edit_own(),
        Some(Permit),
        None,
    );
    add(
        "an_edit_that_reaches_attendees_asks_without_invitation_authority (1, 2)",
        editing(
            Profile::FLEXIBLE,
            vec![edit_grant()],
            vec![],
            attended(vec![bob()]),
        ),
        edit_own(),
        Some(Ask),
        Some(Rule::NoStrangers),
    );
    add(
        "an_edit_that_reaches_approved_attendees_is_permitted",
        editing(
            Profile::FLEXIBLE,
            vec![edit_grant(), invite_grant()],
            vec![team()],
            attended(vec![bob(), carol()]),
        ),
        edit_own(),
        Some(Permit),
        None,
    );
    add(
        "an_edit_that_reaches_a_stranger_asks (1, 2)",
        editing(
            Profile::FLEXIBLE,
            vec![edit_grant(), invite_grant()],
            vec![team()],
            attended(vec![bob(), dan()]),
        ),
        edit_own(),
        Some(Ask),
        Some(Rule::NoStrangers),
    );
    add(
        "an_edit_that_reaches_a_stranger_asks (3)",
        editing(
            widest(),
            vec![edit_grant(), invite_grant()],
            vec![team()],
            attended(vec![bob(), dan()]),
        ),
        edit_own(),
        Some(Ask),
        None,
    );
    add(
        "an_edit_that_reaches_attendees_needs_matching_notification_and_profile (1, 2)",
        editing(
            Profile::FLEXIBLE,
            vec![
                edit_grant(),
                Grant {
                    send_updates: Some(SendUpdates::NotifyNone),
                    ..invite_grant()
                },
            ],
            vec![team()],
            attended(vec![bob()]),
        ),
        edit_own(),
        Some(Ask),
        Some(Rule::InvitationAuthority),
    );
    add(
        "an_edit_that_reaches_attendees_needs_matching_notification_and_profile (3)",
        editing(
            Profile::CAREFUL,
            vec![edit_grant(), invite_grant()],
            vec![team()],
            attended(vec![bob()]),
        ),
        edit_own(),
        Some(Ask),
        None,
    );
    add(
        "the_owners_approval_of_the_edit_covers_both_rows",
        editing(
            Profile::CAREFUL,
            vec![Grant {
                recipients: Some(RecipientRule::Exact(vec![c(&[1])])),
                ..approval()
            }],
            vec![],
            attended(vec![bob()]),
        ),
        edit_own(),
        Some(Permit),
        None,
    );
    add(
        "careful_edits_fabric_created_events_in_drafts_only (1)",
        editing(Profile::CAREFUL, vec![edit_grant()], vec![], own_event()),
        update("drafts", SendUpdates::NotifyNone),
        Some(Permit),
        None,
    );
    add(
        "careful_edits_fabric_created_events_in_drafts_only (2)",
        editing(Profile::CAREFUL, vec![edit_grant()], vec![], own_event()),
        edit_own(),
        Some(Ask),
        None,
    );
    let pre_existing = Event {
        fabric_created: false,
        ..own_event()
    };
    add(
        "a_pre_existing_event_asks_unless_a_custom_profile_allows (1)",
        editing(
            Profile::FLEXIBLE,
            vec![edit_grant()],
            vec![],
            pre_existing.clone(),
        ),
        edit_own(),
        Some(Ask),
        None,
    );
    add(
        "a_pre_existing_event_asks_unless_a_custom_profile_allows (2)",
        editing(
            Profile::FLEXIBLE,
            vec![approval()],
            vec![],
            pre_existing.clone(),
        ),
        edit_own(),
        Some(Permit),
        None,
    );
    add(
        "a_pre_existing_event_asks_unless_a_custom_profile_allows (3)",
        editing(widest(), vec![edit_grant()], vec![], pre_existing),
        edit_own(),
        Some(Permit),
        None,
    );
    let approved_edit = |event: Option<Event>| with(approved(), |c| c.event = event);
    add(
        "edits_that_are_blocked (1, 6)",
        approved_edit(Some(Event {
            revision: c(&[8]),
            ..own_event()
        })),
        edit_own(),
        Some(Block),
        Some(Rule::RevisionCurrent),
    );
    add(
        "edits_that_are_blocked (2)",
        approved_edit(None),
        edit_own(),
        None,
        Some(Rule::EventKnown),
    );
    add(
        "edits_that_are_blocked (3)",
        approved_edit(Some(Event {
            recurring: true,
            ..own_event()
        })),
        edit_own(),
        None,
        Some(Rule::EventSupported),
    );
    add(
        "edits_that_are_blocked (4)",
        approved_edit(Some(Event {
            timed: false,
            ..own_event()
        })),
        edit_own(),
        None,
        Some(Rule::EventSupported),
    );
    add(
        "edits_that_are_blocked (5)",
        approved_edit(Some(Event {
            default_type: false,
            ..own_event()
        })),
        edit_own(),
        None,
        Some(Rule::EventSupported),
    );

    // Invitations (CD-M07, G0-STRANGERS).
    let inviting = |profile: Profile, sets: Vec<RecipientSet>, event: Event| {
        editing(profile, vec![invite_grant()], sets, event)
    };
    add(
        "flexible_allows_invitations_to_an_approved_recipient_set",
        inviting(Profile::FLEXIBLE, vec![team()], own_event()),
        invite(vec![bob(), carol()]),
        Some(Permit),
        None,
    );
    add(
        "careful_asks_for_every_invitation (1, 2)",
        inviting(Profile::CAREFUL, vec![team()], own_event()),
        invite(vec![bob(), carol()]),
        Some(Ask),
        Some(Rule::InvitationAuthority),
    );
    add(
        "a_stranger_asks_in_every_profile (1, 2)",
        inviting(Profile::FLEXIBLE, vec![team()], own_event()),
        invite(vec![bob(), dan()]),
        Some(Ask),
        Some(Rule::NoStrangers),
    );
    add(
        "a_stranger_asks_in_every_profile (3)",
        inviting(widest(), vec![team()], own_event()),
        invite(vec![bob(), dan()]),
        Some(Ask),
        None,
    );
    add(
        "domains_and_removed_attendees (1)",
        inviting(
            Profile::FLEXIBLE,
            vec![RecipientSet {
                domains: vec![c(&[11])],
                ..team()
            }],
            own_event(),
        ),
        invite(vec![bob(), dan()]),
        Some(Permit),
        None,
    );
    add(
        "domains_and_removed_attendees (2)",
        inviting(Profile::FLEXIBLE, vec![team()], attended(vec![dan()])),
        invite(vec![bob()]),
        Some(Ask),
        None,
    );
    let exact_approval = |addresses: &[u32]| Grant {
        recipients: Some(RecipientRule::Exact(
            addresses.iter().map(|&a| c(&[a])).collect(),
        )),
        ..approval()
    };
    add(
        "the_owners_approval_of_an_invitation_to_a_stranger (1)",
        editing(
            Profile::CAREFUL,
            vec![exact_approval(&[1, 3])],
            vec![],
            own_event(),
        ),
        invite(vec![bob(), dan()]),
        Some(Permit),
        None,
    );
    add(
        "the_owners_approval_of_an_invitation_to_a_stranger (2)",
        editing(
            Profile::CAREFUL,
            vec![exact_approval(&[1])],
            vec![],
            own_event(),
        ),
        invite(vec![bob(), dan()]),
        Some(Ask),
        None,
    );

    // Responding (CD-M08, CD-Q05b).
    let answering = |profile: Profile, grants: Vec<Grant>, event: Event| {
        editing(profile, grants, vec![], event)
    };
    add(
        "a_response_asks_in_both_built_in_profiles (1)",
        answering(Profile::CAREFUL, vec![], invitation()),
        respond(whole(200, 7), Response::Accepted),
        Some(Ask),
        None,
    );
    add(
        "a_response_asks_in_both_built_in_profiles (2)",
        answering(Profile::FLEXIBLE, vec![], invitation()),
        respond(whole(200, 7), Response::Accepted),
        Some(Ask),
        None,
    );
    add(
        "a_response_asks_in_both_built_in_profiles (3)",
        answering(Profile::CAREFUL, vec![approval()], invitation()),
        respond(whole(200, 7), Response::Accepted),
        Some(Permit),
        None,
    );
    let to_occurrence = || answering(Profile::CAREFUL, vec![approval()], occurrence());
    add(
        "a_response_to_one_occurrence (1)",
        to_occurrence(),
        respond(occurrence_target(201, 5000, 7), Response::Declined),
        Some(Permit),
        None,
    );
    add(
        "a_response_to_one_occurrence (2)",
        to_occurrence(),
        respond(occurrence_target(201, 5001, 7), Response::Declined),
        None,
        Some(Rule::InvitationTarget),
    );
    add(
        "a_response_to_one_occurrence (3)",
        to_occurrence(),
        respond(occurrence_target(201, 5000, 8), Response::Declined),
        None,
        Some(Rule::RevisionCurrent),
    );
    add(
        "a_response_to_one_occurrence (4)",
        to_occurrence(),
        respond(occurrence_target(202, 5000, 7), Response::Declined),
        None,
        Some(Rule::EventKnown),
    );
    add(
        "a_response_to_one_occurrence (5)",
        answering(
            Profile::CAREFUL,
            vec![approval()],
            Event {
                recurring: true,
                ..invitation()
            },
        ),
        respond(whole(200, 7), Response::Declined),
        None,
        Some(Rule::EventSupported),
    );
    add(
        "a_response_to_one_occurrence (6)",
        answering(Profile::CAREFUL, vec![approval()], own_event()),
        respond(whole(100, 7), Response::Accepted),
        None,
        Some(Rule::InvitationTarget),
    );

    // Deleting (CD-Q05a) and what is always blocked (CD-M10, G0-WARNDISC).
    add(
        "deleting_needs_the_owners_approval_each_time (1)",
        answering(Profile::CAREFUL, vec![approval()], own_event()),
        delete_own(),
        Some(Permit),
        None,
    );
    add(
        "deleting_needs_the_owners_approval_each_time (2)",
        answering(
            widest(),
            vec![Grant {
                tools: vec![Tool::Delete],
                calendars: vec![key("team")],
                send_updates: Some(SendUpdates::NotifyAll),
                ..standing_grant()
            }],
            own_event(),
        ),
        delete_own(),
        Some(Ask),
        None,
    );
    add(
        "deleting_needs_the_owners_approval_each_time (3)",
        answering(
            Profile::CAREFUL,
            vec![Grant {
                duration: one_request(D_THIS, true),
                ..approval()
            }],
            own_event(),
        ),
        delete_own(),
        Some(Ask),
        None,
    );
    add(
        "deleting_needs_the_owners_approval_each_time (4)",
        answering(
            Profile::CAREFUL,
            vec![Grant {
                duration: one_request(D_OTHER, false),
                ..approval()
            }],
            own_event(),
        ),
        delete_own(),
        Some(Ask),
        None,
    );
    add(
        "only_events_fabric_created_are_deleted (1, 2)",
        answering(
            Profile::CAREFUL,
            vec![approval()],
            Event {
                fabric_created: false,
                ..own_event()
            },
        ),
        delete_own(),
        Some(Block),
        Some(Rule::FabricCreated),
    );
    let untouchable = || with(approved(), |c| c.profile = widest());
    add(
        "calendars_and_sharing_are_never_touched (1)",
        untouchable(),
        Action::DeleteCalendar {
            account: s("acct-1"),
            calendar: s("team"),
        },
        Some(Block),
        None,
    );
    add(
        "calendars_and_sharing_are_never_touched (2)",
        untouchable(),
        Action::ChangeSharing {
            account: s("acct-1"),
            calendar: s("team"),
        },
        None,
        Some(Rule::ModeNotBlocked),
    );
    add(
        "calendars_and_sharing_are_never_touched (3)",
        untouchable(),
        Action::DiscloseWarning,
        Some(Block),
        None,
    );

    // Releasing results (plan section 9.2).
    let held = |grants: Vec<Grant>| {
        with(base(), |c| {
            c.grants = grants;
            c.result_held = true;
        })
    };
    add(
        "a_read_result_is_released_once (1)",
        held(vec![one_read()]),
        the_read().disclosed(),
        Some(Permit),
        None,
    );
    add(
        "a_read_result_is_released_once (2)",
        held(vec![Grant {
            duration: one_request(D_THIS, true),
            ..one_read()
        }]),
        the_read().disclosed(),
        Some(Ask),
        None,
    );
    add(
        "a_read_result_is_released_once (3)",
        held(vec![Grant {
            ended: true,
            ..read_grant()
        }]),
        the_read().disclosed(),
        Some(Ask),
        None,
    );
    add(
        "a_read_result_is_released_once (4)",
        held(vec![read_grant()]),
        the_read().disclosed(),
        Some(Permit),
        None,
    );
    add(
        "releases_that_are_blocked (1)",
        grants(vec![one_read()]),
        the_read().disclosed(),
        None,
        Some(Rule::ResultHeld),
    );
    add(
        "releases_that_are_blocked (2)",
        with(held(vec![one_read()]), |c| c.paused = true),
        the_read().disclosed(),
        None,
        Some(Rule::AgentNotPaused),
    );
    add(
        "releases_that_are_blocked (3)",
        held(vec![one_read()]),
        the_read().disclosed().disclosed(),
        None,
        Some(Rule::ResultHeld),
    );
    add(
        "releases_that_are_blocked (4)",
        with(held(vec![one_read()]), |c| c.connected = Vec::new()),
        the_read().disclosed(),
        None,
        Some(Rule::AccountConnected),
    );
    add(
        "the_outcome_of_a_change_is_released_to_its_agent (1)",
        held(vec![]),
        create("acct-1", "team").disclosed(),
        Some(Permit),
        None,
    );
    add(
        "the_outcome_of_a_change_is_released_to_its_agent (2)",
        with(held(vec![]), |c| c.paused = true),
        create("acct-1", "team").disclosed(),
        Some(Block),
        None,
    );

    // The broker's own reads (plan sections 8 and 11).
    add(
        "the_brokers_own_reads (1)",
        system(),
        system_refresh("primary"),
        Some(Permit),
        None,
    );
    add(
        "the_brokers_own_reads (2)",
        with(system(), |c| c.warning_calendars = vec![key("private")]),
        system_refresh("private"),
        Some(Permit),
        None,
    );
    add(
        "the_brokers_own_reads (3)",
        system(),
        system_refresh("private"),
        None,
        Some(Rule::SystemAuthority),
    );
    let reconcile = Action::SystemReconcile {
        account: s("acct-1"),
        calendar: s("team"),
        event_id: c(&[100]),
    };
    add(
        "the_brokers_own_reads (4)",
        with(system(), |context| {
            context.pending_effects = vec![PendingEffect {
                account: s("acct-1"),
                calendar: s("team"),
                event_id: c(&[100]),
            }];
        }),
        reconcile.clone(),
        Some(Permit),
        None,
    );
    add(
        "the_brokers_own_reads (5)",
        system(),
        reconcile,
        None,
        Some(Rule::SystemAuthority),
    );
    add(
        "the_brokers_own_reads (6)",
        base(),
        system_refresh("primary"),
        None,
        Some(Rule::Actor),
    );
    add(
        "the_brokers_own_reads (7)",
        system(),
        create("acct-1", "team"),
        None,
        Some(Rule::Actor),
    );

    // What a lasting approval is worth (contract README, section 10).
    let asking = || Profile {
        list: Mode::Asks,
        read: Mode::Asks,
        drafts: Mode::Asks,
        existing: Mode::Asks,
        edit_fabric: Mode::Asks,
        edit_other: Mode::Asks,
        invite: Mode::Asks,
        respond: Mode::Asks,
        delete: Mode::Asks,
        sharing: Mode::Blocks,
    };
    // `lasting t cal`.
    let lasting = |tool: Tool, calendar: &str| Grant {
        id: s("g-lasting"),
        tools: vec![tool],
        calendars: vec![key(calendar)],
        ..standing_grant()
    };
    let for_session = |grant: Grant| Grant {
        duration: Duration::Session {
            session_id: s("session-1"),
        },
        ..grant
    };
    // `after_approval p g`, with the event the case names.
    let after_approval = |profile: Profile, grant: Grant, event: Option<Event>| {
        with(base(), |c| {
            c.profile = profile;
            c.relied = vec![grant.id.clone()];
            c.grants = vec![grant];
            c.event = event;
        })
    };
    let lasting_availability = || Grant {
        window: Some(Window {
            start_ms: 0,
            end_ms: 100,
        }),
        ..lasting(Tool::Availability, "primary")
    };
    let lasting_read = || Grant {
        window: Some(Window {
            start_ms: 0,
            end_ms: 100,
        }),
        fields: vec![EventField::Time, EventField::Title],
        ..lasting(Tool::Read, "primary")
    };
    let lasting_edit = |calendar: &str, send_updates: SendUpdates| Grant {
        send_updates: Some(send_updates),
        ..lasting(Tool::Update, calendar)
    };
    let lasting_invitation = || Grant {
        recipients: Some(RecipientRule::Exact(vec![c(&[1]), c(&[3])])),
        send_updates: Some(SendUpdates::NotifyAll),
        ..lasting(Tool::Invite, "team")
    };
    let lasting_response = || Grant {
        send_updates: Some(SendUpdates::NotifyAll),
        ..lasting(Tool::Respond, "primary")
    };
    let lasting_delete = || Grant {
        send_updates: Some(SendUpdates::NotifyAll),
        ..lasting(Tool::Delete, "team")
    };
    let read_asked = || read(0, 100, &[EventField::Time, EventField::Title]);
    let edit_in_drafts = || update("drafts", SendUpdates::NotifyNone);
    let accept = || respond(whole(200, 7), Response::Accepted);
    let pre_existing = || Event {
        fabric_created: false,
        ..own_event()
    };
    let with_bob = || Event {
        attendees: vec![bob()],
        ..own_event()
    };
    // One row per lemma of the section that has the common shape. With no rule, the lemma
    // states Permit until revoked (1) and for the session (2). With a rule, it states
    // Need_approval and that rule until revoked (1, 2), and Need_approval for the session (3).
    let cells: Vec<LastingCell> = vec![
        (
            "a_lasting_approval_of_availability_serves_under_allow",
            widest(),
            lasting_availability(),
            availability(&["primary"], 0, 100),
            None,
            None,
        ),
        (
            "a_lasting_approval_of_availability_does_not_serve_under_ask",
            asking(),
            lasting_availability(),
            availability(&["primary"], 0, 100),
            None,
            Some(Rule::Authority),
        ),
        (
            "a_lasting_approval_of_a_read_serves_under_allow",
            widest(),
            lasting_read(),
            read_asked(),
            None,
            None,
        ),
        (
            "a_lasting_approval_of_a_read_serves_under_ask",
            asking(),
            lasting_read(),
            read_asked(),
            None,
            None,
        ),
        (
            "a_lasting_approval_of_a_create_in_agent_drafts_serves_under_allow",
            widest(),
            lasting(Tool::Create, "drafts"),
            create("acct-1", "drafts"),
            None,
            None,
        ),
        (
            "a_lasting_approval_of_a_create_in_agent_drafts_does_not_serve_under_ask",
            asking(),
            lasting(Tool::Create, "drafts"),
            create("acct-1", "drafts"),
            None,
            Some(Rule::Authority),
        ),
        (
            "a_lasting_approval_of_a_create_in_another_calendar_serves_under_allow",
            widest(),
            lasting(Tool::Create, "team"),
            create("acct-1", "team"),
            None,
            None,
        ),
        (
            "a_lasting_approval_of_a_create_in_another_calendar_does_not_serve_under_ask",
            asking(),
            lasting(Tool::Create, "team"),
            create("acct-1", "team"),
            None,
            Some(Rule::Authority),
        ),
        (
            "a_lasting_approval_of_an_edit_in_agent_drafts_serves_under_allow",
            widest(),
            lasting_edit("drafts", SendUpdates::NotifyNone),
            edit_in_drafts(),
            Some(own_event()),
            None,
        ),
        (
            "a_lasting_approval_of_an_edit_in_agent_drafts_serves_under_ask",
            asking(),
            lasting_edit("drafts", SendUpdates::NotifyNone),
            edit_in_drafts(),
            Some(own_event()),
            None,
        ),
        (
            "a_lasting_approval_of_an_edit_outside_agent_drafts_serves_under_allow",
            widest(),
            lasting_edit("team", SendUpdates::NotifyAll),
            edit_own(),
            Some(own_event()),
            None,
        ),
        (
            "a_lasting_approval_of_an_edit_outside_agent_drafts_does_not_serve_under_ask",
            asking(),
            lasting_edit("team", SendUpdates::NotifyAll),
            edit_own(),
            Some(own_event()),
            Some(Rule::Authority),
        ),
        (
            "a_lasting_approval_of_an_edit_of_a_pre_existing_event_serves_under_allow",
            widest(),
            lasting_edit("team", SendUpdates::NotifyAll),
            edit_own(),
            Some(pre_existing()),
            None,
        ),
        (
            "a_lasting_approval_of_an_edit_of_a_pre_existing_event_does_not_serve_under_ask",
            asking(),
            lasting_edit("team", SendUpdates::NotifyAll),
            edit_own(),
            Some(pre_existing()),
            Some(Rule::Authority),
        ),
        (
            "a_lasting_approval_of_an_invitation_serves_under_allow",
            widest(),
            lasting_invitation(),
            invite(vec![bob(), dan()]),
            Some(own_event()),
            None,
        ),
        (
            "a_lasting_approval_of_an_invitation_does_not_serve_under_ask",
            asking(),
            lasting_invitation(),
            invite(vec![bob(), dan()]),
            Some(own_event()),
            Some(Rule::InvitationAuthority),
        ),
        (
            "a_lasting_approval_of_a_response_serves_under_allow",
            widest(),
            lasting_response(),
            accept(),
            Some(invitation()),
            None,
        ),
        (
            "a_lasting_approval_of_a_response_does_not_serve_under_ask",
            asking(),
            lasting_response(),
            accept(),
            Some(invitation()),
            Some(Rule::Authority),
        ),
    ];
    for (lemma, profile, grant, action, event, rule) in cells {
        let session = after_approval(profile, for_session(grant.clone()), event.clone());
        let revoked = after_approval(profile, grant, event);
        if rule.is_some() {
            add(
                &format!("{lemma} (1, 2)"),
                revoked,
                action.clone(),
                Some(Ask),
                rule,
            );
            add(&format!("{lemma} (3)"), session, action, Some(Ask), None);
        } else {
            add(
                &format!("{lemma} (1)"),
                revoked,
                action.clone(),
                Some(Permit),
                None,
            );
            add(&format!("{lemma} (2)"), session, action, Some(Permit), None);
        }
    }
    let attended_edit = || lasting_edit("team", SendUpdates::NotifyAll);
    add(
        "a_lasting_approval_of_an_edit_that_reaches_attendees_never_serves (1, 2)",
        after_approval(widest(), attended_edit(), Some(with_bob())),
        edit_own(),
        Some(Ask),
        Some(Rule::NoStrangers),
    );
    add(
        "a_lasting_approval_of_an_edit_that_reaches_attendees_never_serves (3)",
        after_approval(widest(), for_session(attended_edit()), Some(with_bob())),
        edit_own(),
        Some(Ask),
        None,
    );
    add(
        "a_lasting_approval_of_an_edit_that_reaches_attendees_never_serves (4)",
        after_approval(
            Profile {
                invite: Mode::Asks,
                ..widest()
            },
            attended_edit(),
            Some(with_bob()),
        ),
        edit_own(),
        Some(Ask),
        None,
    );
    add(
        "a_lasting_approval_of_an_edit_that_reaches_attendees_never_serves (5)",
        after_approval(
            Profile {
                invite: Mode::Allows,
                ..asking()
            },
            attended_edit(),
            Some(with_bob()),
        ),
        edit_own(),
        Some(Ask),
        None,
    );
    add(
        "a_lasting_approval_of_an_edit_that_reaches_attendees_never_serves (6)",
        after_approval(asking(), attended_edit(), Some(with_bob())),
        edit_own(),
        Some(Ask),
        None,
    );
    add(
        "a_lasting_approval_of_a_delete_never_serves (1, 2)",
        after_approval(asking(), lasting_delete(), Some(own_event())),
        delete_own(),
        Some(Ask),
        Some(Rule::Authority),
    );
    add(
        "a_lasting_approval_of_a_delete_never_serves (3)",
        after_approval(asking(), for_session(lasting_delete()), Some(own_event())),
        delete_own(),
        Some(Ask),
        None,
    );
    add(
        "a_lasting_approval_of_a_delete_never_serves (4)",
        after_approval(widest(), lasting_delete(), Some(own_event())),
        delete_own(),
        Some(Ask),
        None,
    );

    // Which grants a permitted action rests on.
    let both_edit_grants = || {
        editing(
            Profile::FLEXIBLE,
            vec![edit_grant(), invite_grant()],
            vec![team()],
            attended(vec![bob()]),
        )
    };
    let relying_on_both =
        |context: Context| with(context, |c| c.relied = vec![s("g-edit"), s("g-invite")]);
    let mut new = vec![
        (
            "an_edit_that_reaches_attendees_rests_on_two_grants (1, 2, 3)",
            both_edit_grants(),
            edit_own(),
            Some(Permit),
            None,
            Some(vec![true, true]),
        ),
        (
            "only_the_grants_that_cover_the_action_authorize_it (1, 2)",
            grants(vec![availability_grant(), read_grant()]),
            the_read(),
            None,
            None,
            Some(vec![false, true]),
        ),
        (
            "only_the_grants_that_cover_the_action_authorize_it (3)",
            grants(vec![create_grant()]),
            create("acct-1", "team"),
            None,
            None,
            Some(vec![false]),
        ),
        (
            "only_the_grants_that_cover_the_action_authorize_it (4)",
            creating(Profile::FLEXIBLE),
            create("acct-1", "team"),
            None,
            None,
            Some(vec![true]),
        ),
        (
            "the_owners_approval_authorizes_until_it_is_used (1)",
            answering(Profile::CAREFUL, vec![approval()], own_event()),
            delete_own(),
            None,
            None,
            Some(vec![true]),
        ),
        // The statement names a grant outside `ctx_grants`; `authorizes` does not read
        // `ctx_grants`, so the grant is checked as the context's only grant.
        (
            "the_owners_approval_authorizes_until_it_is_used (2)",
            answering(
                Profile::CAREFUL,
                vec![Grant {
                    duration: one_request(D_THIS, true),
                    ..approval()
                }],
                own_event(),
            ),
            delete_own(),
            None,
            None,
            Some(vec![false]),
        ),
        (
            "the_owners_approval_authorizes_until_it_is_used (3)",
            answering(
                Profile::CAREFUL,
                vec![Grant {
                    duration: one_request(D_OTHER, false),
                    ..approval()
                }],
                own_event(),
            ),
            delete_own(),
            None,
            None,
            Some(vec![false]),
        ),
        (
            "releasing_the_outcome_of_a_change_rests_on_no_grant",
            held(vec![approval()]),
            create("acct-1", "team").disclosed(),
            None,
            None,
            Some(vec![false]),
        ),
        (
            "a_request_resting_on_two_grants_is_blocked_when_one_ends (1)",
            relying_on_both(both_edit_grants()),
            edit_own(),
            Some(Permit),
            None,
            None,
        ),
        (
            "a_request_resting_on_two_grants_is_blocked_when_one_ends (2, 3)",
            relying_on_both(with(both_edit_grants(), |c| {
                c.grants = vec![
                    edit_grant(),
                    Grant {
                        ended: true,
                        ..invite_grant()
                    },
                ];
            })),
            edit_own(),
            Some(Block),
            Some(Rule::ReliedGrantLive),
            None,
        ),
    ];
    for (name, context, action, outcome, failing, authorizes) in new.drain(..) {
        cases.push(Case {
            name: name.to_owned(),
            context,
            action,
            outcome,
            failing,
            authorizes,
        });
    }

    // Earlier cases whose authorizing grants the examples make plain.
    for (name, authorizes) in [
        ("availability_without_a_grant_asks", vec![]),
        ("availability_inside_a_grant_is_permitted (1)", vec![true]),
        ("availability_inside_a_grant_is_permitted (2)", vec![true]),
        (
            "an_availability_grant_does_not_cover_event_details",
            vec![false],
        ),
        ("careful_reads_run_inside_the_approved_grant", vec![true]),
        (
            "careful_asks_for_each_create_in_an_existing_calendar (1)",
            vec![false],
        ),
        (
            "careful_asks_for_each_create_in_an_existing_calendar (2)",
            vec![true],
        ),
        (
            "flexible_allows_creates_in_the_calendars_a_grant_names (1)",
            vec![true],
        ),
        (
            "deleting_needs_the_owners_approval_each_time (1)",
            vec![true],
        ),
        (
            "deleting_needs_the_owners_approval_each_time (3)",
            vec![false],
        ),
        (
            "deleting_needs_the_owners_approval_each_time (4)",
            vec![false],
        ),
        (
            "the_outcome_of_a_change_is_released_to_its_agent (1)",
            vec![],
        ),
        ("the_brokers_own_reads (1)", vec![]),
    ] {
        let case = cases
            .iter_mut()
            .find(|case| case.name == name)
            .unwrap_or_else(|| panic!("{name} is a case"));
        case.authorizes = Some(authorizes);
    }
    cases
}

/// The check theory files left in a work directory.
fn theory_files(dir: &Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("Check_") && name.ends_with(".thy"))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Starts a gate on `config` after removing check theories an older build left behind.
fn start(config: GateConfig) -> BrokerGate {
    for path in theory_files(&config.work_dir) {
        std::fs::remove_file(path).expect("an old check theory is removed");
    }
    BrokerGate::start(config).expect("the gate starts")
}

/// The outcome a first failing rule implies under the theory's `outcome`.
fn outcome_of_failing(rule: Rule) -> Outcome {
    if rule.is_block_rule() {
        Outcome::Block
    } else {
        Outcome::NeedApproval
    }
}

#[test]
#[ignore = "needs Isabelle from scripts/setup-demo.sh; run with --ignored"]
fn every_case_is_decided_as_expected_by_isabelle() {
    let _live = LIVE.lock().unwrap_or_else(PoisonError::into_inner);
    let started = Instant::now();
    let config = config("fabric-broker-corpus");
    let work_dir = config.work_dir.clone();
    let mut gate = start(config);
    eprintln!(
        "gate started in {:.1}s ({}, theory {})",
        started.elapsed().as_secs_f64(),
        gate.isabelle(),
        &gate.theory_sha256()[..12]
    );

    let cases = cases();
    let mut failures = Vec::new();
    for case in &cases {
        let check = gate
            .check(&case.context, &case.action)
            .expect("the check runs");
        let expected = case
            .outcome
            .or_else(|| case.failing.map(outcome_of_failing));
        let outcome = check.verdict.outcome();
        let mut as_expected = outcome.is_some() && (expected.is_none() || outcome == expected);
        if let Some(authorizes) = &case.authorizes {
            let proven: Vec<Option<bool>> = check
                .grants
                .iter()
                .map(|grant| match grant.outcome {
                    RuleOutcome::Holds => Some(true),
                    RuleOutcome::Fails => Some(false),
                    RuleOutcome::Undecided => None,
                })
                .collect();
            let expected_proven: Vec<Option<bool>> = authorizes.iter().map(|&a| Some(a)).collect();
            let expected_ids: Vec<String> = case
                .context
                .grants
                .iter()
                .zip(authorizes)
                .filter(|(_, authorizes)| **authorizes)
                .map(|(grant, _)| grant.id.clone())
                .collect();
            as_expected &= proven == expected_proven && check.authorizing_grants == expected_ids;
        }
        if let Some(rule) = case.failing {
            as_expected &= check.failing_rule == Some(rule);
        }
        if outcome == Some(Outcome::Permit) {
            as_expected &= check.failing_rule.is_none();
        }
        let shown = match &check.verdict {
            Verdict::Decided { outcome } => format!(
                "{}{}",
                outcome.name(),
                check
                    .failing_rule
                    .map(|rule| format!(" ({})", rule.name()))
                    .unwrap_or_default()
            ),
            Verdict::Undecided { reason } => format!("undecided: {reason}"),
        };
        let shown = format!("{shown} {:?}", check.authorizing_grants);
        eprintln!(
            "  {} {:<80} {shown} {:.2}s",
            if as_expected { "ok " } else { "BAD" },
            case.name,
            check.seconds
        );
        if !as_expected {
            failures.push(format!(
                "{}: expected {:?} / {:?} / {:?}, got {shown}",
                case.name, case.outcome, case.failing, case.authorizes
            ));
        }
    }

    // The golden fixture's context and action must be well-formed HOL that Isabelle decides.
    let golden = gate
        .check(&common::context(), &common::action())
        .expect("the check runs");
    eprintln!(
        "  golden fixture: {:?} {:?}",
        golden.verdict, golden.authorizing_grants
    );
    gate.stop().expect("the gate stops");
    let left = theory_files(&work_dir);
    assert!(left.is_empty(), "check theories left behind: {left:?}");

    assert!(
        failures.is_empty(),
        "{} of {} cases failed:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
    assert!(
        golden.verdict.is_permit(),
        "the golden fixture's case is permitted: {:?}",
        golden.verdict
    );
    assert!(golden.rules.iter().all(|r| r.outcome == RuleOutcome::Holds));
    // `g-invite` covers the invitation; `g-once` is an unused approval of exactly this
    // digest; `g-standing` has ended.
    assert_eq!(golden.authorizing_grants, ["g-invite", "g-once"]);
    eprintln!("\nall {} cases decided as expected", cases.len());
}

#[test]
#[ignore = "needs Isabelle from scripts/setup-demo.sh; run with --ignored"]
fn a_check_that_runs_out_of_time_is_undecided() {
    let _live = LIVE.lock().unwrap_or_else(PoisonError::into_inner);
    let mut config = config("fabric-broker-timeout");
    config.check_timeout = StdDuration::from_millis(1);
    let work_dir = config.work_dir.clone();
    let mut gate = start(config);
    for case in cases().iter().take(3) {
        let check = gate
            .check(&case.context, &case.action)
            .expect("the check returns");
        eprintln!(
            "  {:<60} {:?} after {:.3}s",
            case.name, check.verdict, check.seconds
        );
        assert!(
            matches!(check.verdict, Verdict::Undecided { .. }),
            "{}: {:?}",
            case.name,
            check.verdict
        );
        assert_eq!(check.failing_rule, None);
        assert!(check.authorizing_grants.is_empty());
    }
    gate.stop().expect("the gate stops");
    let left = theory_files(&work_dir);
    assert!(left.is_empty(), "check theories left behind: {left:?}");
}

#[test]
#[ignore = "needs Isabelle from scripts/setup-demo.sh; run with --ignored"]
fn checks_are_timed() {
    let _live = LIVE.lock().unwrap_or_else(PoisonError::into_inner);
    let started = Instant::now();
    let mut gate = start(config("fabric-broker-timing"));
    let session_start = started.elapsed().as_secs_f64();

    // Every case of the corpus, of every kind: reads, creates, edits, invitations,
    // responses, deletions, releases and the broker's own reads.
    let cases = cases();
    assert!(cases.len() >= 30);
    let mut seconds = Vec::new();
    for case in &cases {
        let check = gate
            .check(&case.context, &case.action)
            .expect("the check runs");
        assert!(check.verdict.outcome().is_some(), "{}", case.name);
        seconds.push(check.seconds);
    }
    gate.stop().expect("the gate stops");

    seconds.sort_by(f64::total_cmp);
    let count = seconds.len();
    #[allow(clippy::cast_precision_loss, reason = "a few dozen checks")]
    let mean = seconds.iter().sum::<f64>() / count as f64;
    let median = if count % 2 == 0 {
        f64::midpoint(seconds[count / 2 - 1], seconds[count / 2])
    } else {
        seconds[count / 2]
    };
    let max = seconds[count - 1];
    println!(
        "session start {session_start:.2}s; {count} checks: mean {mean:.3}s, median \
         {median:.3}s, max {max:.3}s, min {:.3}s",
        seconds[0]
    );
}
