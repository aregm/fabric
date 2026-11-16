//! The shared types of `schemas/common.schema.json`: event times and references, the small
//! enumerations, the tool names and the limits.

use serde::{Deserialize, Serialize};

use crate::scalar::{
    ConstText, Email, Fixed, IanaZone, Id, List, OffsetDateTime, PositiveCount, ProviderRef,
    Timestamp,
};

/// The protocol name and major version, `fabric-desktop/1` (README section 1).
pub const PROTOCOL: &str = "fabric-desktop/1";

/// The schema version of this contract (`data/version.json`, README section 12).
pub const SCHEMA_VERSION: u64 = 2;

/// The marker for `common#/$defs/protocol`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtocolName;

impl ConstText for ProtocolName {
    const VALUE: &'static str = PROTOCOL;
}

/// `common#/$defs/protocol`: the member every frame carries.
pub type Protocol = Fixed<ProtocolName>;

/// `common#/$defs/event_time`: an instant with its offset and the explicit IANA zone it is
/// shown and stored in (CD-Q05c).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventTime {
    pub date_time: OffsetDateTime,
    pub zone: IanaZone,
}

/// `common#/$defs/window`: a half-open UTC interval. The broker checks that start is before end.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub start: Timestamp,
    pub end: Timestamp,
}

/// `common#/$defs/event_ref`: one event and the revision a request was written against.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventRef {
    pub event_id: ProviderRef,
    pub revision: ProviderRef,
}

/// `common#/$defs/occurrence_ref`: one occurrence of a recurring invitation (CD-Q05b).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OccurrenceRef {
    pub instance_id: ProviderRef,
    pub original_start_time: EventTime,
    pub revision: ProviderRef,
}

/// `common#/$defs/calendar_ref`: one calendar named with its account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalendarRef {
    pub account_id: Id,
    pub calendar_id: Id,
}

/// `common#/$defs/weekday`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

/// `common#/$defs/weekdays`: 1 to 7 distinct days.
pub type Weekdays = List<Weekday, 1, 7, true>;

/// `common#/$defs/event_field`: a field a read may disclose (README section 14).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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

/// `common#/$defs/event_fields`: 1 to 7 distinct fields.
pub type EventFields = List<EventField, 1, 7, true>;

/// `common#/$defs/send_updates`: who the provider notifies (README section 14).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendUpdates {
    All,
    ExternalOnly,
    None,
}

/// `common#/$defs/calendar_access`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalendarAccess {
    Owner,
    Writer,
    Reader,
    FreeBusyReader,
}

/// `common#/$defs/calendar_ids`: 1 to 50 distinct ids.
pub type CalendarIds = List<Id, 1, 50, true>;

/// `common#/$defs/addresses`: 1 to 200 distinct addresses.
pub type Addresses = List<Email, 1, 200, true>;

/// `common#/$defs/tool_name`: the agent tools, which are the methods of the agent channel
/// (`data/tool-catalog.json`, README section 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ToolName {
    #[serde(rename = "fabric_calendar_list")]
    CalendarList,
    #[serde(rename = "fabric_calendar_availability")]
    CalendarAvailability,
    #[serde(rename = "fabric_calendar_read")]
    CalendarRead,
    #[serde(rename = "fabric_calendar_propose_create")]
    ProposeCreate,
    #[serde(rename = "fabric_calendar_propose_update")]
    ProposeUpdate,
    #[serde(rename = "fabric_calendar_propose_invitation")]
    ProposeInvitation,
    #[serde(rename = "fabric_calendar_propose_response")]
    ProposeResponse,
    #[serde(rename = "fabric_calendar_propose_delete")]
    ProposeDelete,
    #[serde(rename = "fabric_request_status")]
    RequestStatus,
    #[serde(rename = "fabric_request_result")]
    RequestResult,
}

impl ToolName {
    /// Every tool, in the order of `data/tool-catalog.json`.
    pub const ALL: [ToolName; 10] = [
        ToolName::CalendarList,
        ToolName::CalendarAvailability,
        ToolName::CalendarRead,
        ToolName::ProposeCreate,
        ToolName::ProposeUpdate,
        ToolName::ProposeInvitation,
        ToolName::ProposeResponse,
        ToolName::ProposeDelete,
        ToolName::RequestStatus,
        ToolName::RequestResult,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ToolName::CalendarList => "fabric_calendar_list",
            ToolName::CalendarAvailability => "fabric_calendar_availability",
            ToolName::CalendarRead => "fabric_calendar_read",
            ToolName::ProposeCreate => "fabric_calendar_propose_create",
            ToolName::ProposeUpdate => "fabric_calendar_propose_update",
            ToolName::ProposeInvitation => "fabric_calendar_propose_invitation",
            ToolName::ProposeResponse => "fabric_calendar_propose_response",
            ToolName::ProposeDelete => "fabric_calendar_propose_delete",
            ToolName::RequestStatus => "fabric_request_status",
            ToolName::RequestResult => "fabric_request_result",
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<ToolName> {
        ToolName::ALL.into_iter().find(|t| t.as_str() == name)
    }

    /// Whether a call creates a request and is an action. Only the status tool does not
    /// (README section 8).
    #[must_use]
    pub const fn creates_request(self) -> bool {
        !matches!(self, ToolName::RequestStatus)
    }

    /// The `$defs` name of the tool's argument schema in `agent-channel.schema.json`.
    #[must_use]
    pub const fn arguments_definition(self) -> &'static str {
        match self {
            ToolName::CalendarList => "arguments_calendar_list",
            ToolName::CalendarAvailability => "arguments_calendar_availability",
            ToolName::CalendarRead => "arguments_calendar_read",
            ToolName::ProposeCreate => "arguments_propose_create",
            ToolName::ProposeUpdate => "arguments_propose_update",
            ToolName::ProposeInvitation => "arguments_propose_invitation",
            ToolName::ProposeResponse => "arguments_propose_response",
            ToolName::ProposeDelete => "arguments_propose_delete",
            ToolName::RequestStatus => "arguments_request_status",
            ToolName::RequestResult => "arguments_request_result",
        }
    }
}

/// `common#/$defs/failure_code`: why a request ended in `terminal_failure` (README section 14).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCode {
    InvalidArguments,
    RevisionConflict,
    ProviderRejected,
    RetryBudgetExhausted,
    AccountAuthenticationRequired,
}

/// `common#/$defs/busy_limit`: the limit a busy answer names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BusyLimit {
    OpenRequestsTotal,
    OpenRequestsPerAgent,
    RequestsPerMinutePerAgent,
    OpenApprovalCardsPerAgent,
}

/// `common#/$defs/undecided_reason`: why no proof decided a request (README section 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UndecidedReason {
    /// The rule prover was starting or not running.
    ProverUnavailable,
    /// The proof did not finish within its time limit.
    OutOfTime,
    /// The check ran and did not decide the request, or its report could not be used.
    Failed,
}

/// `common#/$defs/limits`: the numbers the broker enforces and reports in `hello`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub max_frame_bytes: PositiveCount,
    pub subscriber_queue_notices: PositiveCount,
    pub open_requests_total: PositiveCount,
    pub open_requests_per_agent: PositiveCount,
    pub requests_per_minute_per_agent: PositiveCount,
    pub open_approval_cards_per_agent: PositiveCount,
    pub session_idle_timeout_seconds: PositiveCount,
    pub authorization_lifetime_seconds: PositiveCount,
    pub approval_expiry_seconds: PositiveCount,
}

/// The largest frame in bytes, without its line feed (README section 1, plan 9.1).
pub const MAX_FRAME_BYTES: usize = 1_048_576;

/// The authorization's lifetime in seconds (CD-Q11f, README section 11).
pub const AUTHORIZATION_LIFETIME_SECONDS: u64 = 60;

impl Limits {
    /// The limits frozen at gate CD-G0 (`data/limits.json`): plan 9.1 for the frame bound and
    /// the subscriber queue, CD-Q11d and CD-Q11g for the request limits, CD-M-DUR for the idle
    /// timeout, CD-Q11f for the authorization lifetime and the approval expiry.
    pub const CD_G0: Limits = Limits {
        max_frame_bytes: PositiveCount::of(MAX_FRAME_BYTES as u64),
        subscriber_queue_notices: PositiveCount::of(256),
        open_requests_total: PositiveCount::of(100),
        open_requests_per_agent: PositiveCount::of(25),
        requests_per_minute_per_agent: PositiveCount::of(15),
        open_approval_cards_per_agent: PositiveCount::of(5),
        session_idle_timeout_seconds: PositiveCount::of(1800),
        authorization_lifetime_seconds: PositiveCount::of(AUTHORIZATION_LIFETIME_SECONDS),
        approval_expiry_seconds: PositiveCount::of(86_400),
    };
}
