//! Rust mirrors of the datatypes of `isabelle/Fabric_Broker/Fabric_Broker.thy`.
//!
//! Every type here has one HOL counterpart, named in its doc comment, and [`crate::hol`]
//! renders it in that counterpart's constructor order. Times are UTC milliseconds since the
//! Unix epoch. Broker identifiers are `String`s rendered as HOL strings; text from outside
//! (addresses, domains, the provider's event ids and revisions) is a `String` rendered as the
//! list of its code points (`chars`).

use std::error::Error;
use std::fmt;

use fabric_desktop_contracts::agent::{self, ProposedResponse, ResponseTarget};
use fabric_desktop_contracts::common;
use fabric_desktop_contracts::owner::{self, ProfileRows};
use fabric_desktop_contracts::scalar::{Email, OffsetDateTime, Timestamp};
use serde::{Deserialize, Serialize};

/// `row`: the rows of the policy matrix (plan section 7, ADR-108 D2), in the theory's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Row {
    List,
    Read,
    Drafts,
    Existing,
    EditFabric,
    EditOther,
    Invite,
    Respond,
    Delete,
    Sharing,
}

impl Row {
    /// Every row, in the order of the `row` datatype.
    pub const ALL: [Row; 10] = [
        Row::List,
        Row::Read,
        Row::Drafts,
        Row::Existing,
        Row::EditFabric,
        Row::EditOther,
        Row::Invite,
        Row::Respond,
        Row::Delete,
        Row::Sharing,
    ];

    /// The HOL constructor, such as `Row_edit_fabric`.
    #[must_use]
    pub const fn hol_constructor(self) -> &'static str {
        match self {
            Row::List => "Row_list",
            Row::Read => "Row_read",
            Row::Drafts => "Row_drafts",
            Row::Existing => "Row_existing",
            Row::EditFabric => "Row_edit_fabric",
            Row::EditOther => "Row_edit_other",
            Row::Invite => "Row_invite",
            Row::Respond => "Row_respond",
            Row::Delete => "Row_delete",
            Row::Sharing => "Row_sharing",
        }
    }

    /// The profile field that holds this row's mode, such as `p_edit_fabric`.
    #[must_use]
    pub const fn profile_field(self) -> &'static str {
        match self {
            Row::List => "p_list",
            Row::Read => "p_read",
            Row::Drafts => "p_drafts",
            Row::Existing => "p_existing",
            Row::EditFabric => "p_edit_fabric",
            Row::EditOther => "p_edit_other",
            Row::Invite => "p_invite",
            Row::Respond => "p_respond",
            Row::Delete => "p_delete",
            Row::Sharing => "p_sharing",
        }
    }

    /// The contract's name for the row (`owner#/$defs/policy_row`).
    #[must_use]
    pub const fn policy_row(self) -> owner::PolicyRow {
        match self {
            Row::List => owner::PolicyRow::ListAndAvailability,
            Row::Read => owner::PolicyRow::ReadEventDetails,
            Row::Drafts => owner::PolicyRow::CreateInAgentDrafts,
            Row::Existing => owner::PolicyRow::CreateInExistingCalendar,
            Row::EditFabric => owner::PolicyRow::EditFabricCreatedEvent,
            Row::EditOther => owner::PolicyRow::EditPreexistingEvent,
            Row::Invite => owner::PolicyRow::SendOrChangeInvitations,
            Row::Respond => owner::PolicyRow::RespondToInvitation,
            Row::Delete => owner::PolicyRow::DeleteEvent,
            Row::Sharing => owner::PolicyRow::DeleteCalendarOrChangeSharing,
        }
    }
}

/// `mode`: what a profile says about one row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Blocks,
    Asks,
    Allows,
}

impl Mode {
    /// The HOL constructor: `Blocks`, `Asks` or `Allows`.
    #[must_use]
    pub const fn hol_constructor(self) -> &'static str {
        match self {
            Mode::Blocks => "Blocks",
            Mode::Asks => "Asks",
            Mode::Allows => "Allows",
        }
    }
}

impl From<owner::Mode> for Mode {
    fn from(mode: owner::Mode) -> Self {
        match mode {
            owner::Mode::Allow => Mode::Allows,
            owner::Mode::Ask => Mode::Asks,
            owner::Mode::Block => Mode::Blocks,
        }
    }
}

/// `profile`: one mode per row. The theory's `mode_of` ignores `delete` and `sharing`
/// (CD-Q05a, CD-M10); they are kept so the record has every field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Profile {
    pub list: Mode,
    pub read: Mode,
    pub drafts: Mode,
    pub existing: Mode,
    pub edit_fabric: Mode,
    pub edit_other: Mode,
    pub invite: Mode,
    pub respond: Mode,
    pub delete: Mode,
    pub sharing: Mode,
}

impl Profile {
    /// `careful`, as frozen at gate CD-G0.
    pub const CAREFUL: Profile = Profile {
        list: Mode::Allows,
        read: Mode::Asks,
        drafts: Mode::Allows,
        existing: Mode::Asks,
        edit_fabric: Mode::Asks,
        edit_other: Mode::Asks,
        invite: Mode::Asks,
        respond: Mode::Asks,
        delete: Mode::Asks,
        sharing: Mode::Blocks,
    };

    /// `flexible`, as frozen at gate CD-G0.
    pub const FLEXIBLE: Profile = Profile {
        list: Mode::Allows,
        read: Mode::Allows,
        drafts: Mode::Allows,
        existing: Mode::Allows,
        edit_fabric: Mode::Allows,
        edit_other: Mode::Asks,
        invite: Mode::Allows,
        respond: Mode::Asks,
        delete: Mode::Asks,
        sharing: Mode::Blocks,
    };

    /// The mode the record holds for a row (its field, not `mode_of`).
    #[must_use]
    pub const fn field(&self, row: Row) -> Mode {
        match row {
            Row::List => self.list,
            Row::Read => self.read,
            Row::Drafts => self.drafts,
            Row::Existing => self.existing,
            Row::EditFabric => self.edit_fabric,
            Row::EditOther => self.edit_other,
            Row::Invite => self.invite,
            Row::Respond => self.respond,
            Row::Delete => self.delete,
            Row::Sharing => self.sharing,
        }
    }
}

impl From<&ProfileRows> for Profile {
    /// The contract's rows; the two fixed rows always read Asks and Blocks (README section 10).
    fn from(rows: &ProfileRows) -> Self {
        let mode = |row: Row| Mode::from(rows.mode(row.policy_row()));
        Profile {
            list: mode(Row::List),
            read: mode(Row::Read),
            drafts: mode(Row::Drafts),
            existing: mode(Row::Existing),
            edit_fabric: mode(Row::EditFabric),
            edit_other: mode(Row::EditOther),
            invite: mode(Row::Invite),
            respond: mode(Row::Respond),
            delete: mode(Row::Delete),
            sharing: mode(Row::Sharing),
        }
    }
}

/// `tool`: the tools a grant may name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tool {
    List,
    Availability,
    Read,
    Create,
    Update,
    Invite,
    Respond,
    Delete,
}

impl Tool {
    #[must_use]
    pub const fn hol_constructor(self) -> &'static str {
        match self {
            Tool::List => "Tool_list",
            Tool::Availability => "Tool_availability",
            Tool::Read => "Tool_read",
            Tool::Create => "Tool_create",
            Tool::Update => "Tool_update",
            Tool::Invite => "Tool_invite",
            Tool::Respond => "Tool_respond",
            Tool::Delete => "Tool_delete",
        }
    }
}

/// `event_field`: a field a read may disclose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventField {
    Time,
    Title,
    Location,
    Description,
    Attendees,
    Organizer,
    OwnResponse,
}

impl EventField {
    #[must_use]
    pub const fn hol_constructor(self) -> &'static str {
        match self {
            EventField::Time => "Field_time",
            EventField::Title => "Field_title",
            EventField::Location => "Field_location",
            EventField::Description => "Field_description",
            EventField::Attendees => "Field_attendees",
            EventField::Organizer => "Field_organizer",
            EventField::OwnResponse => "Field_own_response",
        }
    }
}

impl From<common::EventField> for EventField {
    fn from(field: common::EventField) -> Self {
        match field {
            common::EventField::Time => EventField::Time,
            common::EventField::Title => EventField::Title,
            common::EventField::Location => EventField::Location,
            common::EventField::Description => EventField::Description,
            common::EventField::Attendees => EventField::Attendees,
            common::EventField::Organizer => EventField::Organizer,
            common::EventField::OwnResponse => EventField::OwnResponse,
        }
    }
}

/// `send_updates`: who the provider notifies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendUpdates {
    NotifyAll,
    NotifyExternal,
    NotifyNone,
}

impl SendUpdates {
    #[must_use]
    pub const fn hol_constructor(self) -> &'static str {
        match self {
            SendUpdates::NotifyAll => "Notify_all",
            SendUpdates::NotifyExternal => "Notify_external",
            SendUpdates::NotifyNone => "Notify_none",
        }
    }
}

impl From<common::SendUpdates> for SendUpdates {
    fn from(value: common::SendUpdates) -> Self {
        match value {
            common::SendUpdates::All => SendUpdates::NotifyAll,
            common::SendUpdates::ExternalOnly => SendUpdates::NotifyExternal,
            common::SendUpdates::None => SendUpdates::NotifyNone,
        }
    }
}

/// `response`: the answer to an invitation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Response {
    Accepted,
    Declined,
}

impl Response {
    #[must_use]
    pub const fn hol_constructor(self) -> &'static str {
        match self {
            Response::Accepted => "Accepted",
            Response::Declined => "Declined",
        }
    }
}

/// `recipient`: the address as written, and its domain in lowercase.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Recipient {
    pub address: String,
    pub domain: String,
}

impl Recipient {
    /// The recipient of a contract address: the domain is the text after the last `@`, in
    /// lowercase.
    #[must_use]
    pub fn from_email(email: &Email) -> Self {
        let address = email.as_str();
        let domain = address
            .rsplit_once('@')
            .map_or("", |(_, domain)| domain)
            .to_ascii_lowercase();
        Recipient {
            address: address.to_owned(),
            domain,
        }
    }
}

/// `recipient_set`: exact addresses, and domains matched by equality.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RecipientSet {
    pub id: String,
    pub members: Vec<String>,
    pub domains: Vec<String>,
}

/// `recipient_rule`: whom a grant covers. No rule covers everyone (G0-STRANGERS).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecipientRule {
    /// `Exact`: these addresses.
    Exact(Vec<String>),
    /// `Sets`: the members of the recipient sets with these ids.
    Sets(Vec<String>),
}

/// `duration` (CD-M-DUR).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Duration {
    /// `One_request`: bound to one action digest (64 hex characters) and used once.
    OneRequest { digest: String, used: bool },
    /// `Session`: bound to one session of the agent.
    Session { session_id: String },
    /// `Until_revoked`: no expiry.
    UntilRevoked,
}

/// A calendar named with its account, the theory's `string × string`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CalendarKey {
    pub account: String,
    pub calendar: String,
}

impl CalendarKey {
    #[must_use]
    pub fn new(account: &str, calendar: &str) -> Self {
        CalendarKey {
            account: account.to_owned(),
            calendar: calendar.to_owned(),
        }
    }
}

/// A grant's read window, the theory's `int × int`: start and end in UTC milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Window {
    pub start_ms: i64,
    pub end_ms: i64,
}

/// `grant`: a grant of the agent whose action is checked. A scope part that is absent covers
/// nothing.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Grant {
    pub id: String,
    pub tools: Vec<Tool>,
    pub duration: Duration,
    pub ended: bool,
    pub calendars: Vec<CalendarKey>,
    pub window: Option<Window>,
    pub fields: Vec<EventField>,
    pub recipients: Option<RecipientRule>,
    pub send_updates: Option<SendUpdates>,
}

/// `calendar`: a calendar the owner selected, named with its account.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Calendar {
    pub account: String,
    pub id: String,
    pub writable: bool,
    pub drafts: bool,
}

/// `event`: the broker's own observation of the event an action names.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub revision: String,
    pub fabric_created: bool,
    pub recurring: bool,
    pub default_type: bool,
    pub timed: bool,
    /// The owner is an attendee and not the organizer.
    pub invitation: bool,
    /// The other people on the event.
    pub attendees: Vec<Recipient>,
    /// The original start time, when the event is one occurrence of a recurring event.
    pub occurrence: Option<i64>,
}

/// `actor`: who asks for the action.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    Agent(String),
    System,
}

/// The agent's current session, the theory's `string × int`: its id and when it was last
/// active.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActiveSession {
    pub id: String,
    pub last_active_ms: i64,
}

/// A change the broker handed to Google and has not seen applied, the theory's
/// `string × string × chars`: account, calendar, event id.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PendingEffect {
    pub account: String,
    pub calendar: String,
    pub event_id: String,
}

/// `broker_context`: the facts the broker derived from its own records. Nothing an agent or
/// a language model said is a fact here.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Context {
    pub actor: Actor,
    pub paused: bool,
    pub now_ms: i64,
    pub session: Option<ActiveSession>,
    pub profile: Profile,
    pub connected: Vec<String>,
    pub calendars: Vec<Calendar>,
    pub warning_calendars: Vec<CalendarKey>,
    pub grants: Vec<Grant>,
    pub recipient_sets: Vec<RecipientSet>,
    pub event: Option<Event>,
    /// The action digest of the action being checked, as 64 hex characters; for a disclosure,
    /// the digest of the action whose result is disclosed.
    pub digest: String,
    /// The grant an allowed or approved request rests on, once it has one.
    pub relied: Option<String>,
    pub result_held: bool,
    pub pending_effects: Vec<PendingEffect>,
    pub pause_epoch: u64,
    pub revocation_epoch: u64,
}

/// `target`: what a response answers (CD-Q05b).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    /// `Whole_event`: an event with its revision.
    WholeEvent { event_id: String, revision: String },
    /// `Occurrence`: one occurrence with its instance id, original start time and revision.
    Occurrence {
        instance_id: String,
        original_start_ms: i64,
        revision: String,
    },
}

/// `action`: what an agent, or the broker itself, asks to do.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    ListCalendars,
    ReadAvailability {
        account: String,
        calendars: Vec<String>,
        start_ms: i64,
        end_ms: i64,
    },
    ReadEvents {
        account: String,
        calendars: Vec<String>,
        start_ms: i64,
        end_ms: i64,
        fields: Vec<EventField>,
    },
    CreateEvent {
        account: String,
        calendar: String,
    },
    UpdateEvent {
        account: String,
        calendar: String,
        event_id: String,
        revision: String,
        send_updates: SendUpdates,
    },
    ChangeInvitation {
        account: String,
        calendar: String,
        event_id: String,
        revision: String,
        /// The complete attendee list after the change.
        attendees: Vec<Recipient>,
        send_updates: SendUpdates,
    },
    Respond {
        account: String,
        calendar: String,
        target: Target,
        response: Response,
        send_updates: SendUpdates,
    },
    DeleteEvent {
        account: String,
        calendar: String,
        event_id: String,
        revision: String,
        send_updates: SendUpdates,
    },
    /// The release of this action's result to the agent.
    Disclose(Box<Action>),
    /// The release of a warning to an agent; always blocked (G0-WARNDISC).
    DiscloseWarning,
    DeleteCalendar {
        account: String,
        calendar: String,
    },
    ChangeSharing {
        account: String,
        calendar: String,
    },
    SystemRefresh {
        account: String,
        calendar: String,
    },
    SystemReconcile {
        account: String,
        calendar: String,
        event_id: String,
    },
}

/// Why a tool call has no broker action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCallError(pub String);

impl fmt::Display for ToolCallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "no broker action for this tool call: {}", self.0)
    }
}

impl Error for ToolCallError {}

fn millis(timestamp: &Timestamp) -> Result<i64, ToolCallError> {
    timestamp
        .unix_millis()
        .ok_or_else(|| ToolCallError(format!("{timestamp} is not a day of its month")))
}

/// UTC milliseconds of `YYYY-MM-DDTHH:MM:SS±HH:MM` (`common#/$defs/offset_date_time`).
fn offset_millis(value: &OffsetDateTime) -> Result<i64, ToolCallError> {
    let text = value.as_str();
    let invalid = || ToolCallError(format!("{text} is not a valid offset date-time"));
    let (local, offset) = (
        text.get(..19).ok_or_else(invalid)?,
        text.get(19..).ok_or_else(invalid)?,
    );
    let local = Timestamp::new(format!("{local}.000Z")).map_err(|_| invalid())?;
    let sign = match offset.as_bytes().first() {
        Some(b'+') => 1,
        Some(b'-') => -1,
        _ => return Err(invalid()),
    };
    let hours: i64 = offset
        .get(1..3)
        .and_then(|h| h.parse().ok())
        .ok_or_else(invalid)?;
    let minutes: i64 = offset
        .get(4..6)
        .and_then(|m| m.parse().ok())
        .ok_or_else(invalid)?;
    Ok(millis(&local)? - sign * (hours * 60 + minutes) * 60_000)
}

fn ids(values: &[fabric_desktop_contracts::scalar::Id]) -> Vec<String> {
    values.iter().map(|id| id.as_str().to_owned()).collect()
}

impl Action {
    /// The broker action of an agent tool call (README section 8). A result request is not
    /// one: it is checked as [`Action::disclosed`] of the action whose result it asks for.
    pub fn from_tool_call(call: &agent::Action) -> Result<Action, ToolCallError> {
        Ok(match call {
            agent::Action::CalendarList(_) => Action::ListCalendars,
            agent::Action::CalendarAvailability(a) => Action::ReadAvailability {
                account: a.account_id.as_str().to_owned(),
                calendars: ids(&a.calendar_ids),
                start_ms: millis(&a.window.start)?,
                end_ms: millis(&a.window.end)?,
            },
            agent::Action::CalendarRead(a) => Action::ReadEvents {
                account: a.account_id.as_str().to_owned(),
                calendars: ids(&a.calendar_ids),
                start_ms: millis(&a.window.start)?,
                end_ms: millis(&a.window.end)?,
                fields: a.fields.iter().copied().map(EventField::from).collect(),
            },
            agent::Action::ProposeCreate(a) => Action::CreateEvent {
                account: a.account_id.as_str().to_owned(),
                calendar: a.calendar_id.as_str().to_owned(),
            },
            agent::Action::ProposeUpdate(a) => Action::UpdateEvent {
                account: a.account_id.as_str().to_owned(),
                calendar: a.calendar_id.as_str().to_owned(),
                event_id: a.event.event_id.as_str().to_owned(),
                revision: a.event.revision.as_str().to_owned(),
                send_updates: a.send_updates.into(),
            },
            agent::Action::ProposeInvitation(a) => Action::ChangeInvitation {
                account: a.account_id.as_str().to_owned(),
                calendar: a.calendar_id.as_str().to_owned(),
                event_id: a.event.event_id.as_str().to_owned(),
                revision: a.event.revision.as_str().to_owned(),
                attendees: a.attendees.iter().map(Recipient::from_email).collect(),
                send_updates: a.send_updates.into(),
            },
            agent::Action::ProposeResponse(a) => Action::Respond {
                account: a.account_id.as_str().to_owned(),
                calendar: a.calendar_id.as_str().to_owned(),
                target: match &a.target {
                    ResponseTarget::Event(event) => Target::WholeEvent {
                        event_id: event.event_id.as_str().to_owned(),
                        revision: event.revision.as_str().to_owned(),
                    },
                    ResponseTarget::Occurrence(occurrence) => Target::Occurrence {
                        instance_id: occurrence.instance_id.as_str().to_owned(),
                        original_start_ms: offset_millis(
                            &occurrence.original_start_time.date_time,
                        )?,
                        revision: occurrence.revision.as_str().to_owned(),
                    },
                },
                response: match a.response {
                    ProposedResponse::Accepted => Response::Accepted,
                    ProposedResponse::Declined => Response::Declined,
                },
                send_updates: a.send_updates.into(),
            },
            agent::Action::ProposeDelete(a) => Action::DeleteEvent {
                account: a.account_id.as_str().to_owned(),
                calendar: a.calendar_id.as_str().to_owned(),
                event_id: a.event.event_id.as_str().to_owned(),
                revision: a.event.revision.as_str().to_owned(),
                send_updates: a.send_updates.into(),
            },
            agent::Action::RequestResult(_) => {
                return Err(ToolCallError(
                    "fabric_request_result releases another action's result; check that \
                     action with Action::disclosed"
                        .to_owned(),
                ));
            }
        })
    }

    /// `Disclose a`: the release of this action's result to the agent (plan section 9.2).
    #[must_use]
    pub fn disclosed(self) -> Action {
        Action::Disclose(Box::new(self))
    }
}

/// `rule`: the 18 rules, in the theory's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    Actor,
    AgentNotPaused,
    ModeNotBlocked,
    AccountConnected,
    CalendarSelected,
    CalendarWritable,
    EventKnown,
    RevisionCurrent,
    EventSupported,
    FabricCreated,
    InvitationTarget,
    ReliedGrantLive,
    ResultHeld,
    WarningsOwnerOnly,
    SystemAuthority,
    NoStrangers,
    InvitationAuthority,
    Authority,
}

impl Rule {
    /// Every rule, in the order of the `rule` datatype, which is `block_rules @
    /// approval_rules`.
    pub const ALL: [Rule; 18] = [
        Rule::Actor,
        Rule::AgentNotPaused,
        Rule::ModeNotBlocked,
        Rule::AccountConnected,
        Rule::CalendarSelected,
        Rule::CalendarWritable,
        Rule::EventKnown,
        Rule::RevisionCurrent,
        Rule::EventSupported,
        Rule::FabricCreated,
        Rule::InvitationTarget,
        Rule::ReliedGrantLive,
        Rule::ResultHeld,
        Rule::WarningsOwnerOnly,
        Rule::SystemAuthority,
        Rule::NoStrangers,
        Rule::InvitationAuthority,
        Rule::Authority,
    ];

    /// The rule's name in snake case, as the contract's `name_token`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Rule::Actor => "actor",
            Rule::AgentNotPaused => "agent_not_paused",
            Rule::ModeNotBlocked => "mode_not_blocked",
            Rule::AccountConnected => "account_connected",
            Rule::CalendarSelected => "calendar_selected",
            Rule::CalendarWritable => "calendar_writable",
            Rule::EventKnown => "event_known",
            Rule::RevisionCurrent => "revision_current",
            Rule::EventSupported => "event_supported",
            Rule::FabricCreated => "fabric_created",
            Rule::InvitationTarget => "invitation_target",
            Rule::ReliedGrantLive => "relied_grant_live",
            Rule::ResultHeld => "result_held",
            Rule::WarningsOwnerOnly => "warnings_owner_only",
            Rule::SystemAuthority => "system_authority",
            Rule::NoStrangers => "no_strangers",
            Rule::InvitationAuthority => "invitation_authority",
            Rule::Authority => "authority",
        }
    }

    /// The HOL constructor, such as `Agent_not_paused`.
    #[must_use]
    pub const fn hol_constructor(self) -> &'static str {
        match self {
            Rule::Actor => "Actor",
            Rule::AgentNotPaused => "Agent_not_paused",
            Rule::ModeNotBlocked => "Mode_not_blocked",
            Rule::AccountConnected => "Account_connected",
            Rule::CalendarSelected => "Calendar_selected",
            Rule::CalendarWritable => "Calendar_writable",
            Rule::EventKnown => "Event_known",
            Rule::RevisionCurrent => "Revision_current",
            Rule::EventSupported => "Event_supported",
            Rule::FabricCreated => "Fabric_created",
            Rule::InvitationTarget => "Invitation_target",
            Rule::ReliedGrantLive => "Relied_grant_live",
            Rule::ResultHeld => "Result_held",
            Rule::WarningsOwnerOnly => "Warnings_owner_only",
            Rule::SystemAuthority => "System_authority",
            Rule::NoStrangers => "No_strangers",
            Rule::InvitationAuthority => "Invitation_authority",
            Rule::Authority => "Authority",
        }
    }

    /// Whether the rule is in `block_rules`: when it fails, the action is blocked. The other
    /// rules are `approval_rules`: when one fails and no block rule does, the owner is asked.
    #[must_use]
    pub const fn is_block_rule(self) -> bool {
        !matches!(
            self,
            Rule::NoStrangers | Rule::InvitationAuthority | Rule::Authority
        )
    }

    /// One plain sentence for the owner on why the rule failed. It names no calendar content.
    #[must_use]
    pub const fn explanation(self) -> &'static str {
        match self {
            Rule::Actor => {
                "Only the broker may ask for its own reads, and the broker may not act for an agent."
            }
            Rule::AgentNotPaused => {
                "The agent is paused, so it can do nothing until you resume it."
            }
            Rule::ModeNotBlocked => "Your permission profile blocks this kind of action.",
            Rule::AccountConnected => "The action names a Google account that is not connected.",
            Rule::CalendarSelected => {
                "The action names a calendar you have not selected for agents."
            }
            Rule::CalendarWritable => "The action would change a calendar you can only read.",
            Rule::EventKnown => "The broker has not seen the event this action names.",
            Rule::RevisionCurrent => {
                "The event changed after the agent last read it, so the request is out of date."
            }
            Rule::EventSupported => {
                "This version can only change single, timed events of the default type, or answer one occurrence of a recurring invitation."
            }
            Rule::FabricCreated => "Only events that Fabric created may be deleted.",
            Rule::InvitationTarget => {
                "The event is not an invitation to you, or the occurrence named is not the one you were invited to."
            }
            Rule::ReliedGrantLive => "The grant this request relies on was revoked or has ended.",
            Rule::ResultHeld => "There is no result held for this request that could be released.",
            Rule::WarningsOwnerOnly => {
                "Warnings are shown only to you and are never released to an agent."
            }
            Rule::SystemAuthority => {
                "The broker has no selected calendar or change in progress that calls for this read."
            }
            Rule::NoStrangers => {
                "Someone this change reaches is not in any recipient set you approved."
            }
            Rule::InvitationAuthority => {
                "No grant you approved covers sending or changing invitations for this event."
            }
            Rule::Authority => "No grant you approved covers this action.",
        }
    }
}

/// `outcome`: what the rules decide.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Permit,
    NeedApproval,
    Block,
}

impl Outcome {
    pub const ALL: [Outcome; 3] = [Outcome::Permit, Outcome::NeedApproval, Outcome::Block];

    #[must_use]
    pub const fn hol_constructor(self) -> &'static str {
        match self {
            Outcome::Permit => "Permit",
            Outcome::NeedApproval => "Need_approval",
            Outcome::Block => "Block",
        }
    }

    /// The name in snake case: `permit`, `need_approval` or `block`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Outcome::Permit => "permit",
            Outcome::NeedApproval => "need_approval",
            Outcome::Block => "block",
        }
    }
}
