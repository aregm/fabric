//! The agent channel of `schemas/agent-channel.schema.json`: tool arguments, actions, results,
//! request status and the agent frames (README section 8).

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Value, json};

use crate::common::{
    CalendarAccess, CalendarIds, EventFields, EventRef, EventTime, FailureCode, OccurrenceRef,
    SendUpdates, ToolName, Window,
};
use crate::error::AgentError;
use crate::handshake::{AgentHelloParams, AgentHelloResult, AgentProveParams, AgentProveResult};
use crate::scalar::{
    AnyList, Date, Email, EventDescription, EventLocation, EventTitle, Explanation, Id, Label,
    List, NameToken, ProviderRef, ReadTitle, Task, Timestamp, nullable,
};
use crate::tagged;

/// `agent#/$defs/new_event`: a new timed event with no attendees (README section 14).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewEvent {
    pub title: EventTitle,
    pub start: EventTime,
    pub end: EventTime,
    #[serde(deserialize_with = "nullable")]
    pub description: Option<EventDescription>,
    #[serde(deserialize_with = "nullable")]
    pub location: Option<EventLocation>,
}

/// `agent#/$defs/event_changes`: the editable fields; `None` leaves a field as it is
/// (README section 14).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventChanges {
    #[serde(deserialize_with = "nullable")]
    pub title: Option<EventTitle>,
    #[serde(deserialize_with = "nullable")]
    pub start: Option<EventTime>,
    #[serde(deserialize_with = "nullable")]
    pub end: Option<EventTime>,
    #[serde(deserialize_with = "nullable")]
    pub description: Option<EventDescription>,
    #[serde(deserialize_with = "nullable")]
    pub location: Option<EventLocation>,
}

/// `agent#/$defs/response_target`: a single event, or one occurrence of a recurring invitation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResponseTarget {
    Event(EventRef),
    Occurrence(OccurrenceRef),
}

impl Serialize for ResponseTarget {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            ResponseTarget::Event(e) => tagged::join(s, "kind", "event", &json!({ "event": e })),
            ResponseTarget::Occurrence(o) => {
                tagged::join(s, "kind", "occurrence", &json!({ "occurrence": o }))
            }
        }
    }
}

impl<'de> Deserialize<'de> for ResponseTarget {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (kind, mut rest) = tagged::split(d, "kind")?;
        let target = match kind.as_str() {
            "event" => ResponseTarget::Event(tagged::read(tagged::take(&mut rest, "event")?)?),
            "occurrence" => {
                ResponseTarget::Occurrence(tagged::read(tagged::take(&mut rest, "occurrence")?)?)
            }
            other => return Err(tagged::unknown("kind", other)),
        };
        tagged::nothing(&rest)?;
        Ok(target)
    }
}

/// `agent#/$defs/arguments_calendar_list`: no arguments.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalendarListArguments {}

/// `agent#/$defs/arguments_calendar_availability`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalendarAvailabilityArguments {
    pub account_id: Id,
    pub calendar_ids: CalendarIds,
    pub window: Window,
    pub task: Task,
}

/// `agent#/$defs/arguments_calendar_read`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalendarReadArguments {
    pub account_id: Id,
    pub calendar_ids: CalendarIds,
    pub window: Window,
    pub fields: EventFields,
    pub task: Task,
}

/// `agent#/$defs/arguments_propose_create`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposeCreateArguments {
    pub account_id: Id,
    pub calendar_id: Id,
    pub event: NewEvent,
    pub task: Task,
}

/// `agent#/$defs/arguments_propose_update`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposeUpdateArguments {
    pub account_id: Id,
    pub calendar_id: Id,
    pub event: EventRef,
    pub changes: EventChanges,
    pub send_updates: SendUpdates,
    pub task: Task,
}

/// The complete attendee list an invitation proposes: at most 200 distinct addresses.
pub type Attendees = List<Email, 0, 200, true>;

/// `agent#/$defs/arguments_propose_invitation`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposeInvitationArguments {
    pub account_id: Id,
    pub calendar_id: Id,
    pub event: EventRef,
    pub attendees: Attendees,
    pub send_updates: SendUpdates,
    pub task: Task,
}

/// The response an agent may propose (README section 14).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposedResponse {
    Accepted,
    Declined,
}

/// `agent#/$defs/arguments_propose_response`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposeResponseArguments {
    pub account_id: Id,
    pub calendar_id: Id,
    pub target: ResponseTarget,
    pub response: ProposedResponse,
    pub send_updates: SendUpdates,
    pub task: Task,
}

/// `agent#/$defs/arguments_propose_delete` (CD-Q05a).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposeDeleteArguments {
    pub account_id: Id,
    pub calendar_id: Id,
    pub event: EventRef,
    pub send_updates: SendUpdates,
    pub task: Task,
}

/// `agent#/$defs/arguments_request_status` and `arguments_request_result`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestIdArguments {
    pub request_id: Id,
}

/// `agent#/$defs/action`: a tool and its exact arguments, the subject of the action digest.
/// `fabric_request_status` is not an action (README section 8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    CalendarList(CalendarListArguments),
    CalendarAvailability(CalendarAvailabilityArguments),
    CalendarRead(CalendarReadArguments),
    ProposeCreate(ProposeCreateArguments),
    ProposeUpdate(ProposeUpdateArguments),
    ProposeInvitation(ProposeInvitationArguments),
    ProposeResponse(ProposeResponseArguments),
    ProposeDelete(ProposeDeleteArguments),
    RequestResult(RequestIdArguments),
}

impl Action {
    #[must_use]
    pub fn tool(&self) -> ToolName {
        match self {
            Action::CalendarList(_) => ToolName::CalendarList,
            Action::CalendarAvailability(_) => ToolName::CalendarAvailability,
            Action::CalendarRead(_) => ToolName::CalendarRead,
            Action::ProposeCreate(_) => ToolName::ProposeCreate,
            Action::ProposeUpdate(_) => ToolName::ProposeUpdate,
            Action::ProposeInvitation(_) => ToolName::ProposeInvitation,
            Action::ProposeResponse(_) => ToolName::ProposeResponse,
            Action::ProposeDelete(_) => ToolName::ProposeDelete,
            Action::RequestResult(_) => ToolName::RequestResult,
        }
    }

    /// The arguments as JSON.
    #[must_use]
    pub fn arguments(&self) -> Value {
        use crate::digest::to_value;
        match self {
            Action::CalendarList(a) => to_value(a),
            Action::CalendarAvailability(a) => to_value(a),
            Action::CalendarRead(a) => to_value(a),
            Action::ProposeCreate(a) => to_value(a),
            Action::ProposeUpdate(a) => to_value(a),
            Action::ProposeInvitation(a) => to_value(a),
            Action::ProposeResponse(a) => to_value(a),
            Action::ProposeDelete(a) => to_value(a),
            Action::RequestResult(a) => to_value(a),
        }
    }

    /// Reads the arguments of `tool`, or `None` for the status tool, which is not an action.
    pub fn from_parts<E: de::Error>(tool: ToolName, arguments: Value) -> Result<Option<Action>, E> {
        Ok(Some(match tool {
            ToolName::CalendarList => Action::CalendarList(tagged::read(arguments)?),
            ToolName::CalendarAvailability => {
                Action::CalendarAvailability(tagged::read(arguments)?)
            }
            ToolName::CalendarRead => Action::CalendarRead(tagged::read(arguments)?),
            ToolName::ProposeCreate => Action::ProposeCreate(tagged::read(arguments)?),
            ToolName::ProposeUpdate => Action::ProposeUpdate(tagged::read(arguments)?),
            ToolName::ProposeInvitation => Action::ProposeInvitation(tagged::read(arguments)?),
            ToolName::ProposeResponse => Action::ProposeResponse(tagged::read(arguments)?),
            ToolName::ProposeDelete => Action::ProposeDelete(tagged::read(arguments)?),
            ToolName::RequestResult => Action::RequestResult(tagged::read(arguments)?),
            ToolName::RequestStatus => return Ok(None),
        }))
    }

    /// The action digest (README section 4).
    #[must_use]
    pub fn digest(&self) -> crate::scalar::Digest {
        crate::digest::action_digest(self)
    }
}

fn tool_named<E: de::Error>(name: &str) -> Result<ToolName, E> {
    ToolName::from_name(name).ok_or_else(|| tagged::unknown("tool", name))
}

impl Serialize for Action {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        json!({ "tool": self.tool().as_str(), "arguments": self.arguments() }).serialize(s)
    }
}

impl<'de> Deserialize<'de> for Action {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (tool, mut rest) = tagged::split(d, "tool")?;
        let tool = tool_named(&tool)?;
        let arguments = tagged::take(&mut rest, "arguments")?;
        tagged::nothing(&rest)?;
        Action::from_parts(tool, arguments)?
            .ok_or_else(|| de::Error::custom("fabric_request_status is not an action"))
    }
}

/// `agent#/$defs/response_status`: an attendee's answer as the provider reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStatus {
    NeedsAction,
    Accepted,
    Declined,
    Tentative,
}

/// `agent#/$defs/attendee`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attendee {
    pub email: Email,
    pub response: ResponseStatus,
}

/// `agent#/$defs/event_span`: a timed event, or an all-day event read with date semantics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventSpan {
    Timed { start: EventTime, end: EventTime },
    AllDay { start_date: Date, end_date: Date },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TimedBody {
    start: EventTime,
    end: EventTime,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AllDayBody {
    start_date: Date,
    end_date: Date,
}

impl Serialize for EventSpan {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            EventSpan::Timed { start, end } => tagged::join(
                s,
                "kind",
                "timed",
                &TimedBody {
                    start: start.clone(),
                    end: end.clone(),
                },
            ),
            EventSpan::AllDay {
                start_date,
                end_date,
            } => tagged::join(
                s,
                "kind",
                "all_day",
                &AllDayBody {
                    start_date: start_date.clone(),
                    end_date: end_date.clone(),
                },
            ),
        }
    }
}

impl<'de> Deserialize<'de> for EventSpan {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (kind, rest) = tagged::split(d, "kind")?;
        match kind.as_str() {
            "timed" => {
                let b: TimedBody = tagged::body(rest)?;
                Ok(EventSpan::Timed {
                    start: b.start,
                    end: b.end,
                })
            }
            "all_day" => {
                let b: AllDayBody = tagged::body(rest)?;
                Ok(EventSpan::AllDay {
                    start_date: b.start_date,
                    end_date: b.end_date,
                })
            }
            other => Err(tagged::unknown("kind", other)),
        }
    }
}

/// The occurrence identity in an `event_view`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Occurrence {
    pub instance_id: ProviderRef,
    pub original_start_time: EventTime,
}

/// At most 2000 attendees as a read discloses them.
pub type AttendeeList = List<Attendee, 0, 2000, false>;

/// `agent#/$defs/event_view`: one event as a read discloses it. A content field is `None`
/// when the proven grant does not cover it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventView {
    pub event_id: ProviderRef,
    pub revision: ProviderRef,
    pub calendar_id: Id,
    pub fabric_created: bool,
    #[serde(deserialize_with = "nullable")]
    pub occurrence: Option<Occurrence>,
    #[serde(deserialize_with = "nullable")]
    pub time: Option<EventSpan>,
    #[serde(deserialize_with = "nullable")]
    pub title: Option<ReadTitle>,
    #[serde(deserialize_with = "nullable")]
    pub location: Option<EventLocation>,
    #[serde(deserialize_with = "nullable")]
    pub description: Option<EventDescription>,
    #[serde(deserialize_with = "nullable")]
    pub attendees: Option<AttendeeList>,
    #[serde(deserialize_with = "nullable")]
    pub organizer: Option<Email>,
    #[serde(deserialize_with = "nullable")]
    pub own_response: Option<ResponseStatus>,
}

/// One calendar in a `calendars` result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListedCalendar {
    pub calendar_id: Id,
    pub name: Label,
    pub access: CalendarAccess,
    pub agent_drafts: bool,
}

/// One account in a `calendars` result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListedAccount {
    pub account_id: Id,
    pub calendars: AnyList<ListedCalendar>,
}

/// One busy interval in an `availability` result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BusyInterval {
    pub calendar_id: Id,
    pub start: Timestamp,
    pub end: Timestamp,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservedBusy {
    observed_at: Timestamp,
    busy: AnyList<BusyInterval>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservedEvents {
    observed_at: Timestamp,
    events: AnyList<EventView>,
}

/// `agent#/$defs/tool_result`: what a fresh proof released.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolResult {
    /// `result_calendars`.
    Calendars { accounts: AnyList<ListedAccount> },
    /// `result_availability`.
    Availability {
        observed_at: Timestamp,
        busy: AnyList<BusyInterval>,
    },
    /// `result_events`.
    Events {
        observed_at: Timestamp,
        events: AnyList<EventView>,
    },
    /// `result_effect`: the event as the provider now has it, or `None` after a deletion.
    Effect { event: Option<EventRef> },
}

impl Serialize for ToolResult {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            ToolResult::Calendars { accounts } => {
                tagged::join(s, "kind", "calendars", &json!({ "accounts": accounts }))
            }
            ToolResult::Availability { observed_at, busy } => tagged::join(
                s,
                "kind",
                "availability",
                &json!({ "observed_at": observed_at, "busy": busy }),
            ),
            ToolResult::Events {
                observed_at,
                events,
            } => tagged::join(
                s,
                "kind",
                "events",
                &json!({ "observed_at": observed_at, "events": events }),
            ),
            ToolResult::Effect { event } => {
                tagged::join(s, "kind", "effect", &json!({ "event": event }))
            }
        }
    }
}

impl<'de> Deserialize<'de> for ToolResult {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (kind, mut rest) = tagged::split(d, "kind")?;
        match kind.as_str() {
            "calendars" => {
                let accounts = tagged::read(tagged::take(&mut rest, "accounts")?)?;
                tagged::nothing(&rest)?;
                Ok(ToolResult::Calendars { accounts })
            }
            "availability" => {
                let b: ObservedBusy = tagged::body(rest)?;
                Ok(ToolResult::Availability {
                    observed_at: b.observed_at,
                    busy: b.busy,
                })
            }
            "events" => {
                let b: ObservedEvents = tagged::body(rest)?;
                Ok(ToolResult::Events {
                    observed_at: b.observed_at,
                    events: b.events,
                })
            }
            "effect" => {
                let event = tagged::read(tagged::take(&mut rest, "event")?)?;
                tagged::nothing(&rest)?;
                Ok(ToolResult::Effect { event })
            }
            other => Err(tagged::unknown("kind", other)),
        }
    }
}

/// `agent#/$defs/agent_denial`: the failing rule and a sentence; the proof stays with the
/// owner (README section 14).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentDenial {
    pub rule: NameToken,
    pub explanation: Explanation,
}

/// `agent#/$defs/agent_lifecycle_status`: the state and, for the three states that carry
/// more, the card's expiry, the bounded denial, or the failure code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentLifecycleStatus {
    PendingApproval { approval_expires_at: Timestamp },
    Blocked { denial: AgentDenial },
    TerminalFailure { failure_code: FailureCode },
    Plain(crate::lifecycle::PlainRequestState),
}

impl AgentLifecycleStatus {
    #[must_use]
    pub fn state(&self) -> crate::lifecycle::RequestState {
        use crate::lifecycle::RequestState;
        match self {
            AgentLifecycleStatus::PendingApproval { .. } => RequestState::PendingApproval,
            AgentLifecycleStatus::Blocked { .. } => RequestState::Blocked,
            AgentLifecycleStatus::TerminalFailure { .. } => RequestState::TerminalFailure,
            AgentLifecycleStatus::Plain(p) => p.state(),
        }
    }
}

impl Serialize for AgentLifecycleStatus {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let state = self.state().as_str();
        match self {
            AgentLifecycleStatus::PendingApproval {
                approval_expires_at,
            } => tagged::join(
                s,
                "state",
                state,
                &json!({ "approval_expires_at": approval_expires_at }),
            ),
            AgentLifecycleStatus::Blocked { denial } => {
                tagged::join(s, "state", state, &json!({ "denial": denial }))
            }
            AgentLifecycleStatus::TerminalFailure { failure_code } => {
                tagged::join(s, "state", state, &json!({ "failure_code": failure_code }))
            }
            AgentLifecycleStatus::Plain(_) => tagged::join(s, "state", state, &tagged::Empty {}),
        }
    }
}

impl<'de> Deserialize<'de> for AgentLifecycleStatus {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (state, mut rest) = tagged::split(d, "state")?;
        let status = match state.as_str() {
            "pending_approval" => AgentLifecycleStatus::PendingApproval {
                approval_expires_at: tagged::read(tagged::take(&mut rest, "approval_expires_at")?)?,
            },
            "blocked" => AgentLifecycleStatus::Blocked {
                denial: tagged::read(tagged::take(&mut rest, "denial")?)?,
            },
            "terminal_failure" => AgentLifecycleStatus::TerminalFailure {
                failure_code: tagged::read(tagged::take(&mut rest, "failure_code")?)?,
            },
            other => AgentLifecycleStatus::Plain(tagged::read(Value::String(other.to_owned()))?),
        };
        tagged::nothing(&rest)?;
        Ok(status)
    }
}

/// `agent#/$defs/request_status`: safe lifecycle metadata, never calendar content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestStatus {
    pub request_id: Id,
    pub updated_at: Timestamp,
    pub status: AgentLifecycleStatus,
}

/// `agent#/$defs/tool_outcome`: what every tool answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolOutcome {
    pub request: RequestStatus,
    #[serde(deserialize_with = "nullable")]
    pub result: Option<ToolResult>,
}

/// Every method of the agent channel that is not a tool, in the order of the schema: the two
/// handshake methods and `agent_cancel`.
pub const NON_TOOL_METHODS: [&str; 3] = ["agent_hello", "agent_prove", "agent_cancel"];

/// `agent#/$defs/params_agent_cancel`: the adapter no longer waits for the answer to the call
/// whose frame id is `call_id`, on this connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCancelParams {
    pub call_id: Id,
}

/// `agent#/$defs/result_agent_cancel`: `held` is true when a call with that id was waiting for
/// its answer and was dropped; false when no such call was waiting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCancelResult {
    pub held: bool,
}

/// The method and params of an agent request frame: one of the two handshake methods,
/// `agent_cancel`, an action, or the status tool.
#[allow(
    clippy::large_enum_variant,
    reason = "a transport value is built once per frame; boxing would only complicate its use"
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentCall {
    /// `agent_hello`, the first frame of a connection (README section 8).
    Hello(AgentHelloParams),
    /// `agent_prove`, the signature over the handshake text.
    Prove(AgentProveParams),
    /// `agent_cancel`: the adapter no longer waits for the answer to a call.
    Cancel(AgentCancelParams),
    Action(Action),
    RequestStatus(RequestIdArguments),
}

impl AgentCall {
    /// The tool, or `None` for a handshake method or `agent_cancel`, which are not tools.
    #[must_use]
    pub fn tool(&self) -> Option<ToolName> {
        match self {
            AgentCall::Hello(_) | AgentCall::Prove(_) | AgentCall::Cancel(_) => None,
            AgentCall::Action(a) => Some(a.tool()),
            AgentCall::RequestStatus(_) => Some(ToolName::RequestStatus),
        }
    }

    /// The frame's method: a handshake method or the tool's name.
    #[must_use]
    pub fn method(&self) -> &'static str {
        match self {
            AgentCall::Hello(_) => "agent_hello",
            AgentCall::Prove(_) => "agent_prove",
            AgentCall::Cancel(_) => "agent_cancel",
            AgentCall::Action(a) => a.tool().as_str(),
            AgentCall::RequestStatus(_) => ToolName::RequestStatus.as_str(),
        }
    }

    /// Whether `name` is a method of the agent channel: a tool, a handshake method or
    /// `agent_cancel`.
    #[must_use]
    pub fn is_method(name: &str) -> bool {
        ToolName::from_name(name).is_some() || NON_TOOL_METHODS.contains(&name)
    }

    #[must_use]
    pub fn params(&self) -> Value {
        match self {
            AgentCall::Hello(p) => crate::digest::to_value(p),
            AgentCall::Prove(p) => crate::digest::to_value(p),
            AgentCall::Cancel(p) => crate::digest::to_value(p),
            AgentCall::Action(a) => a.arguments(),
            AgentCall::RequestStatus(a) => crate::digest::to_value(a),
        }
    }
}

/// `agent#/$defs/agent_request`: a frame from an agent adapter. The method is a tool, one of
/// the two handshake methods, or `agent_cancel`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentRequest {
    pub id: Id,
    pub call: AgentCall,
}

impl AgentRequest {
    /// The action digest of `{"tool": method, "arguments": params}`, or `None` for the
    /// status tool and the handshake, which are not actions (README sections 4 and 13).
    #[must_use]
    pub fn action_digest(&self) -> Option<crate::scalar::Digest> {
        match &self.call {
            AgentCall::Action(a) => Some(a.digest()),
            AgentCall::Hello(_)
            | AgentCall::Prove(_)
            | AgentCall::Cancel(_)
            | AgentCall::RequestStatus(_) => None,
        }
    }
}

impl Serialize for AgentRequest {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        json!({
            "protocol": crate::common::PROTOCOL,
            "id": self.id,
            "method": self.call.method(),
            "params": self.call.params(),
        })
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for AgentRequest {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (method, mut rest) = tagged::split(d, "method")?;
        let tool = match method.as_str() {
            "agent_hello" | "agent_prove" | "agent_cancel" => None,
            other => {
                Some(ToolName::from_name(other).ok_or_else(|| tagged::unknown("method", other))?)
            }
        };
        let _: crate::common::Protocol = tagged::read(tagged::take(&mut rest, "protocol")?)?;
        let id = tagged::read(tagged::take(&mut rest, "id")?)?;
        let params = tagged::take(&mut rest, "params")?;
        tagged::nothing(&rest)?;
        let call = match tool {
            None if method == "agent_hello" => AgentCall::Hello(tagged::read(params)?),
            None if method == "agent_cancel" => AgentCall::Cancel(tagged::read(params)?),
            None => AgentCall::Prove(tagged::read(params)?),
            Some(tool) => match Action::from_parts(tool, params.clone())? {
                Some(action) => AgentCall::Action(action),
                None => AgentCall::RequestStatus(tagged::read(params)?),
            },
        };
        Ok(AgentRequest { id, call })
    }
}

/// `agent#/$defs/agent_response`: the broker's answer to an agent frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentResponse {
    /// The answer to `agent_hello`.
    Hello { id: Id, result: AgentHelloResult },
    /// The answer to `agent_prove`.
    Prove { id: Id, result: AgentProveResult },
    /// The answer to `agent_cancel`.
    Cancel { id: Id, result: AgentCancelResult },
    /// The tool's outcome, under the request's id and method.
    Result {
        id: Id,
        method: ToolName,
        result: ToolOutcome,
    },
    /// An error; `id` and `method` are `None` when the frame could not be read that far.
    Error {
        id: Option<Id>,
        method: Option<crate::scalar::MethodText>,
        error: AgentError,
    },
}

impl Serialize for AgentResponse {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let p = crate::common::PROTOCOL;
        match self {
            AgentResponse::Hello { id, result } => {
                json!({ "protocol": p, "id": id, "method": "agent_hello", "result": result })
            }
            AgentResponse::Prove { id, result } => {
                json!({ "protocol": p, "id": id, "method": "agent_prove", "result": result })
            }
            AgentResponse::Cancel { id, result } => {
                json!({ "protocol": p, "id": id, "method": "agent_cancel", "result": result })
            }
            AgentResponse::Result { id, method, result } => {
                json!({ "protocol": p, "id": id, "method": method, "result": result })
            }
            AgentResponse::Error { id, method, error } => {
                json!({ "protocol": p, "id": id, "method": method, "error": error })
            }
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for AgentResponse {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (_protocol, mut rest) = crate::owner::response_members(d)?;
        let id = tagged::take(&mut rest, "id")?;
        let method = tagged::take(&mut rest, "method")?;
        let response = if let Some(error) = rest.remove("error") {
            AgentResponse::Error {
                id: tagged::read(id)?,
                method: tagged::read(method)?,
                error: tagged::read(error)?,
            }
        } else {
            let result = tagged::take(&mut rest, "result")?;
            match method.as_str() {
                Some("agent_hello") => AgentResponse::Hello {
                    id: tagged::read(id)?,
                    result: tagged::read(result)?,
                },
                Some("agent_prove") => AgentResponse::Prove {
                    id: tagged::read(id)?,
                    result: tagged::read(result)?,
                },
                Some("agent_cancel") => AgentResponse::Cancel {
                    id: tagged::read(id)?,
                    result: tagged::read(result)?,
                },
                _ => AgentResponse::Result {
                    id: tagged::read(id)?,
                    method: tagged::read(method)?,
                    result: tagged::read(result)?,
                },
            }
        };
        tagged::nothing(&rest)?;
        Ok(response)
    }
}
