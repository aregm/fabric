//! The owner channel of `schemas/owner-channel.schema.json`: profiles, grants, requests,
//! proofs, the owner methods and the owner frames (README sections 7, 10 and 11).

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value, json};

use crate::agent::{Action, EventView};
use crate::common::{
    AUTHORIZATION_LIFETIME_SECONDS, Addresses, CalendarAccess, CalendarRef, EventFields, EventRef,
    FailureCode, Limits, OwnerResolutionReason, PROTOCOL, Protocol, SendUpdates, ToolName,
    UndecidedReason, Weekdays, Window,
};
use crate::digest::to_value;
use crate::error::OwnerError;
use crate::json::canonical;
use crate::lifecycle::{PlainRequestState, RequestState};
use crate::scalar::{
    AnyList, ConfigurationText, ConstText, Count, CustomProfileId, Digest, DomainName, Email,
    Explanation, Fixed, IanaZone, Id, Integer, Label, List, LocalTime, MethodText, NameToken,
    PairingCode, PositiveCount, ProviderRef, Task, Timestamp, nullable,
};
use crate::tagged;

/// The authorization profile, `fabric-broker-authorization/1` (README section 11, `data/version.json`).
pub const AUTHORIZATION_PROFILE: &str = "fabric-broker-authorization/1";

/// `owner#/$defs/mode`: what a profile says about one row of the policy matrix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Allow,
    Ask,
    Block,
}

impl Mode {
    /// The modes from the least to the most permissive, the order an expansion is measured
    /// in (`modes` in `data/policy-matrix.json`).
    pub const BY_PERMISSIVENESS: [Mode; 3] = [Mode::Block, Mode::Ask, Mode::Allow];

    /// Whether `self` permits more than `other`, so that changing a row from `other` to
    /// `self` is an expansion (README section 7).
    #[must_use]
    pub const fn is_wider_than(self, other: Mode) -> bool {
        const fn rank(mode: Mode) -> u8 {
            match mode {
                Mode::Block => 0,
                Mode::Ask => 1,
                Mode::Allow => 2,
            }
        }
        rank(self) > rank(other)
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Mode::Allow => "allow",
            Mode::Ask => "ask",
            Mode::Block => "block",
        }
    }
}

/// `owner#/$defs/policy_row`: the rows of the policy matrix (plan section 7, ADR-108 D2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyRow {
    ListAndAvailability,
    ReadEventDetails,
    CreateInAgentDrafts,
    CreateInExistingCalendar,
    EditFabricCreatedEvent,
    EditPreexistingEvent,
    SendOrChangeInvitations,
    RespondToInvitation,
    DeleteEvent,
    DeleteCalendarOrChangeSharing,
}

impl PolicyRow {
    /// Every row, in the order of `data/policy-matrix.json`.
    pub const ALL: [PolicyRow; 10] = [
        PolicyRow::ListAndAvailability,
        PolicyRow::ReadEventDetails,
        PolicyRow::CreateInAgentDrafts,
        PolicyRow::CreateInExistingCalendar,
        PolicyRow::EditFabricCreatedEvent,
        PolicyRow::EditPreexistingEvent,
        PolicyRow::SendOrChangeInvitations,
        PolicyRow::RespondToInvitation,
        PolicyRow::DeleteEvent,
        PolicyRow::DeleteCalendarOrChangeSharing,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            PolicyRow::ListAndAvailability => "list_and_availability",
            PolicyRow::ReadEventDetails => "read_event_details",
            PolicyRow::CreateInAgentDrafts => "create_in_agent_drafts",
            PolicyRow::CreateInExistingCalendar => "create_in_existing_calendar",
            PolicyRow::EditFabricCreatedEvent => "edit_fabric_created_event",
            PolicyRow::EditPreexistingEvent => "edit_preexisting_event",
            PolicyRow::SendOrChangeInvitations => "send_or_change_invitations",
            PolicyRow::RespondToInvitation => "respond_to_invitation",
            PolicyRow::DeleteEvent => "delete_event",
            PolicyRow::DeleteCalendarOrChangeSharing => "delete_calendar_or_change_sharing",
        }
    }

    /// The single mode every profile has in this row, for the two fixed rows: deleting an
    /// event asks (CD-Q05a) and deleting a calendar or changing sharing is blocked (CD-M10).
    #[must_use]
    pub const fn fixed_mode(self) -> Option<Mode> {
        match self {
            PolicyRow::DeleteEvent => Some(Mode::Ask),
            PolicyRow::DeleteCalendarOrChangeSharing => Some(Mode::Block),
            _ => None,
        }
    }
}

/// The marker for the `"ask"` of `delete_event`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AskMode;
impl ConstText for AskMode {
    const VALUE: &'static str = "ask";
}

/// The marker for the `"block"` of `delete_calendar_or_change_sharing`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockMode;
impl ConstText for BlockMode {
    const VALUE: &'static str = "block";
}

/// `owner#/$defs/profile_rows`: one mode per row. The two fixed rows hold no data: they
/// always read and write `ask` and `block` (README section 10).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileRows {
    pub list_and_availability: Mode,
    pub read_event_details: Mode,
    pub create_in_agent_drafts: Mode,
    pub create_in_existing_calendar: Mode,
    pub edit_fabric_created_event: Mode,
    pub edit_preexisting_event: Mode,
    pub send_or_change_invitations: Mode,
    pub respond_to_invitation: Mode,
    pub delete_event: Fixed<AskMode>,
    pub delete_calendar_or_change_sharing: Fixed<BlockMode>,
}

impl ProfileRows {
    /// The rows of Careful (`data/policy-matrix.json`, README section 14).
    pub const CAREFUL: ProfileRows = ProfileRows {
        list_and_availability: Mode::Allow,
        read_event_details: Mode::Ask,
        create_in_agent_drafts: Mode::Allow,
        create_in_existing_calendar: Mode::Ask,
        edit_fabric_created_event: Mode::Ask,
        edit_preexisting_event: Mode::Ask,
        send_or_change_invitations: Mode::Ask,
        respond_to_invitation: Mode::Ask,
        delete_event: Fixed::new(),
        delete_calendar_or_change_sharing: Fixed::new(),
    };

    /// The rows of Flexible (`data/policy-matrix.json`, README section 14).
    pub const FLEXIBLE: ProfileRows = ProfileRows {
        list_and_availability: Mode::Allow,
        read_event_details: Mode::Allow,
        create_in_agent_drafts: Mode::Allow,
        create_in_existing_calendar: Mode::Allow,
        edit_fabric_created_event: Mode::Allow,
        edit_preexisting_event: Mode::Ask,
        send_or_change_invitations: Mode::Allow,
        respond_to_invitation: Mode::Ask,
        delete_event: Fixed::new(),
        delete_calendar_or_change_sharing: Fixed::new(),
    };

    /// The mode of one row.
    #[must_use]
    pub const fn mode(&self, row: PolicyRow) -> Mode {
        match row {
            PolicyRow::ListAndAvailability => self.list_and_availability,
            PolicyRow::ReadEventDetails => self.read_event_details,
            PolicyRow::CreateInAgentDrafts => self.create_in_agent_drafts,
            PolicyRow::CreateInExistingCalendar => self.create_in_existing_calendar,
            PolicyRow::EditFabricCreatedEvent => self.edit_fabric_created_event,
            PolicyRow::EditPreexistingEvent => self.edit_preexisting_event,
            PolicyRow::SendOrChangeInvitations => self.send_or_change_invitations,
            PolicyRow::RespondToInvitation => self.respond_to_invitation,
            PolicyRow::DeleteEvent => Mode::Ask,
            PolicyRow::DeleteCalendarOrChangeSharing => Mode::Block,
        }
    }
}

/// The two built-in profiles. Their names and rows are frozen (`profile_careful`, `profile_flexible`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltInProfile {
    Careful,
    Flexible,
}

impl BuiltInProfile {
    pub const ALL: [BuiltInProfile; 2] = [BuiltInProfile::Careful, BuiltInProfile::Flexible];

    #[must_use]
    pub const fn profile_id(self) -> &'static str {
        match self {
            BuiltInProfile::Careful => "careful",
            BuiltInProfile::Flexible => "flexible",
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            BuiltInProfile::Careful => "Careful",
            BuiltInProfile::Flexible => "Flexible",
        }
    }

    /// A built-in profile is always version 1.
    #[must_use]
    pub const fn version(self) -> u64 {
        1
    }

    #[must_use]
    pub const fn rows(self) -> ProfileRows {
        match self {
            BuiltInProfile::Careful => ProfileRows::CAREFUL,
            BuiltInProfile::Flexible => ProfileRows::FLEXIBLE,
        }
    }

    /// The exact JSON of the profile, as the schema's constants fix it.
    #[must_use]
    pub fn to_value(self) -> Value {
        json!({
            "kind": "built_in",
            "profile_id": self.profile_id(),
            "name": self.name(),
            "version": self.version(),
            "rows": self.rows(),
        })
    }
}

/// `owner#/$defs/profile_custom`: a named custom profile; each save is a new version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomProfile {
    pub profile_id: CustomProfileId,
    pub name: Label,
    pub version: PositiveCount,
    pub rows: ProfileRows,
}

/// `owner#/$defs/profile`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Profile {
    BuiltIn(BuiltInProfile),
    Custom(CustomProfile),
}

impl Profile {
    #[must_use]
    pub fn rows(&self) -> ProfileRows {
        match self {
            Profile::BuiltIn(b) => b.rows(),
            Profile::Custom(c) => c.rows,
        }
    }
}

impl Serialize for Profile {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Profile::BuiltIn(b) => b.to_value().serialize(s),
            Profile::Custom(c) => tagged::join(s, "kind", "custom", c),
        }
    }
}

impl<'de> Deserialize<'de> for Profile {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(d)?;
        let text = canonical(&value);
        if let Some(b) = BuiltInProfile::ALL
            .into_iter()
            .find(|b| canonical(&b.to_value()) == text)
        {
            return Ok(Profile::BuiltIn(b));
        }
        let (kind, rest) = tagged::split(value, "kind").map_err(de::Error::custom)?;
        match kind.as_str() {
            "custom" => Ok(Profile::Custom(tagged::body(rest)?)),
            "built_in" => Err(de::Error::custom("not exactly a built-in profile")),
            other => Err(tagged::unknown("kind", other)),
        }
    }
}

/// `owner#/$defs/custom_profile_draft`: what the owner saves; `None` creates a profile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomProfileDraft {
    #[serde(deserialize_with = "nullable")]
    pub profile_id: Option<CustomProfileId>,
    pub name: Label,
    pub rows: ProfileRows,
}

/// One row a save makes more permissive.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpansionEntry {
    pub row: PolicyRow,
    pub from: Mode,
    pub to: Mode,
}

/// `owner#/$defs/expansion`: 1 to 10 distinct entries, the subject of the expansion digest.
pub type Expansion = List<ExpansionEntry, 1, 10, true>;

/// `owner#/$defs/profiles`: the profiles and the one in force (README section 14).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profiles {
    pub revision: Count,
    pub active_profile_id: Id,
    pub items: List<Profile, 2, { usize::MAX }, false>,
}

/// `owner#/$defs/recipient_set_source` (CD-Q06, README section 14).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecipientSetSource {
    Local,
    GooglePeople {
        account_id: Id,
        imported_at: Timestamp,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GooglePeopleBody {
    account_id: Id,
    imported_at: Timestamp,
}

impl Serialize for RecipientSetSource {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            RecipientSetSource::Local => tagged::join(s, "kind", "local", &tagged::Empty {}),
            RecipientSetSource::GooglePeople {
                account_id,
                imported_at,
            } => tagged::join(
                s,
                "kind",
                "google_people",
                &json!({ "account_id": account_id, "imported_at": imported_at }),
            ),
        }
    }
}

impl<'de> Deserialize<'de> for RecipientSetSource {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (kind, rest) = tagged::split(d, "kind")?;
        match kind.as_str() {
            "local" => tagged::nothing(&rest).map(|()| RecipientSetSource::Local),
            "google_people" => {
                let b: GooglePeopleBody = tagged::body(rest)?;
                Ok(RecipientSetSource::GooglePeople {
                    account_id: b.account_id,
                    imported_at: b.imported_at,
                })
            }
            other => Err(tagged::unknown("kind", other)),
        }
    }
}

/// `owner#/$defs/recipient_set_definition`: a named selector, not a grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipientSetDefinition {
    pub name: Label,
    pub members: List<Email, 0, 2000, true>,
    pub domains: List<DomainName, 0, 50, true>,
}

/// `owner#/$defs/recipient_set`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipientSet {
    pub recipient_set_id: Id,
    pub revision: PositiveCount,
    pub source: RecipientSetSource,
    pub definition: RecipientSetDefinition,
}

/// `owner#/$defs/recipient_rule`: exact addresses or named recipient sets. No rule covers
/// everyone (G0-STRANGERS).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecipientRule {
    Exact(Addresses),
    RecipientSets(List<Id, 1, 50, true>),
}

impl Serialize for RecipientRule {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            RecipientRule::Exact(a) => tagged::join(s, "kind", "exact", &json!({ "addresses": a })),
            RecipientRule::RecipientSets(ids) => tagged::join(
                s,
                "kind",
                "recipient_sets",
                &json!({ "recipient_set_ids": ids }),
            ),
        }
    }
}

impl<'de> Deserialize<'de> for RecipientRule {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (kind, mut rest) = tagged::split(d, "kind")?;
        let rule = match kind.as_str() {
            "exact" => RecipientRule::Exact(tagged::read(tagged::take(&mut rest, "addresses")?)?),
            "recipient_sets" => RecipientRule::RecipientSets(tagged::read(tagged::take(
                &mut rest,
                "recipient_set_ids",
            )?)?),
            other => return Err(tagged::unknown("kind", other)),
        };
        tagged::nothing(&rest)?;
        Ok(rule)
    }
}

/// `owner#/$defs/scope`: what an approval or a grant covers. A member that does not apply to
/// the tools is `None`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub calendars: List<CalendarRef, 0, 200, true>,
    #[serde(deserialize_with = "nullable")]
    pub window: Option<Window>,
    #[serde(deserialize_with = "nullable")]
    pub fields: Option<EventFields>,
    #[serde(deserialize_with = "nullable")]
    pub recipients: Option<RecipientRule>,
    #[serde(deserialize_with = "nullable")]
    pub send_updates: Option<SendUpdates>,
}

/// The three durations every grant kind offers (CD-M-DUR).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurationKind {
    OneRequest,
    Session,
    UntilRevoked,
}

/// `owner#/$defs/duration`: `{"kind": …}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Duration {
    pub kind: DurationKind,
}

/// `owner#/$defs/grant_term`: a grant's duration and what it is bound to (README section 10).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GrantTerm {
    /// One immutable action, used once.
    OneRequest { action_digest: Digest },
    /// One agent session.
    Session { session_id: Id },
    /// Nothing but revocation; written with `"expires_at": null`.
    UntilRevoked,
}

impl Serialize for GrantTerm {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            GrantTerm::OneRequest { action_digest } => tagged::join(
                s,
                "kind",
                "one_request",
                &json!({ "action_digest": action_digest }),
            ),
            GrantTerm::Session { session_id } => {
                tagged::join(s, "kind", "session", &json!({ "session_id": session_id }))
            }
            GrantTerm::UntilRevoked => {
                tagged::join(s, "kind", "until_revoked", &json!({ "expires_at": null }))
            }
        }
    }
}

impl<'de> Deserialize<'de> for GrantTerm {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (kind, mut rest) = tagged::split(d, "kind")?;
        let term = match kind.as_str() {
            "one_request" => GrantTerm::OneRequest {
                action_digest: tagged::read(tagged::take(&mut rest, "action_digest")?)?,
            },
            "session" => GrantTerm::Session {
                session_id: tagged::read(tagged::take(&mut rest, "session_id")?)?,
            },
            "until_revoked" => {
                let crate::scalar::Null = tagged::read(tagged::take(&mut rest, "expires_at")?)?;
                GrantTerm::UntilRevoked
            }
            other => return Err(tagged::unknown("kind", other)),
        };
        tagged::nothing(&rest)?;
        Ok(term)
    }
}

/// Why a grant ended (README section 14).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantEndReason {
    Consumed,
    RevokedByOwner,
    AccountDisconnected,
    AuthorityReduced,
    AgentRemoved,
    SessionDisconnected,
    SessionIdleTimeout,
    AppQuit,
    BrokerRestart,
    ResultExpired,
}

/// `owner#/$defs/grant_end`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantEnd {
    pub at: Timestamp,
    pub reason: GrantEndReason,
}

/// `owner#/$defs/grant_origin`: what created a grant. The two branches have no tag member;
/// they differ by their members.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GrantOrigin {
    /// The owner's approval of a request.
    Approval {
        request_id: Id,
        approval_digest: Digest,
    },
    /// The owner's approval of the agent's enrollment, which gives the agent the calendar list.
    /// `enrollment_id` is the agent's id.
    Enrollment { enrollment_id: Id },
}

impl GrantOrigin {
    /// The request whose approval created the grant; `None` for a grant made by an
    /// enrollment approval.
    #[must_use]
    pub fn request_id(&self) -> Option<&Id> {
        match self {
            GrantOrigin::Approval { request_id, .. } => Some(request_id),
            GrantOrigin::Enrollment { .. } => None,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalOriginBody {
    request_id: Id,
    approval_digest: Digest,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrollmentOriginBody {
    enrollment_id: Id,
}

impl Serialize for GrantOrigin {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            GrantOrigin::Approval {
                request_id,
                approval_digest,
            } => json!({ "request_id": request_id, "approval_digest": approval_digest }),
            GrantOrigin::Enrollment { enrollment_id } => json!({ "enrollment_id": enrollment_id }),
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for GrantOrigin {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let Value::Object(members) = Value::deserialize(d)? else {
            return Err(de::Error::custom("expected an object"));
        };
        // Each branch denies unknown members, so at most one of them can hold.
        if members.contains_key("enrollment_id") {
            let b: EnrollmentOriginBody = tagged::body(members)?;
            Ok(GrantOrigin::Enrollment {
                enrollment_id: b.enrollment_id,
            })
        } else {
            let b: ApprovalOriginBody = tagged::body(members)?;
            Ok(GrantOrigin::Approval {
                request_id: b.request_id,
                approval_digest: b.approval_digest,
            })
        }
    }
}

/// The profile version in force when a grant was created.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyVersion {
    pub profile_id: Id,
    pub version: PositiveCount,
}

/// `owner#/$defs/grant`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub grant_id: Id,
    pub revision: PositiveCount,
    pub agent_id: Id,
    pub purpose: Task,
    pub tools: List<ToolName, 1, 10, true>,
    pub scope: Scope,
    pub term: GrantTerm,
    pub origin: GrantOrigin,
    pub policy: PolicyVersion,
    pub revocation_epoch: Count,
    pub created_at: Timestamp,
    #[serde(deserialize_with = "nullable")]
    pub ended: Option<GrantEnd>,
}

/// `owner#/$defs/destination`: where data approved for an agent goes (R14, G0-RISK).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destination {
    Cloud { processor: Label },
    Local { endpoint: Label },
}

impl Serialize for Destination {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Destination::Cloud { processor } => {
                tagged::join(s, "kind", "cloud", &json!({ "processor": processor }))
            }
            Destination::Local { endpoint } => {
                tagged::join(s, "kind", "local", &json!({ "endpoint": endpoint }))
            }
        }
    }
}

impl<'de> Deserialize<'de> for Destination {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (kind, mut rest) = tagged::split(d, "kind")?;
        let destination = match kind.as_str() {
            "cloud" => Destination::Cloud {
                processor: tagged::read(tagged::take(&mut rest, "processor")?)?,
            },
            "local" => Destination::Local {
                endpoint: tagged::read(tagged::take(&mut rest, "endpoint")?)?,
            },
            other => return Err(tagged::unknown("kind", other)),
        };
        tagged::nothing(&rest)?;
        Ok(destination)
    }
}

/// `owner#/$defs/session`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub session_id: Id,
    pub started_at: Timestamp,
    pub last_active_at: Timestamp,
}

/// `owner#/$defs/agent.enrollment`: whether an agent waits for the owner's approval, is
/// approved, or was removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentEnrollment {
    Pending,
    Approved,
    Removed,
}

/// `owner#/$defs/agent`: an agent the owner added. `pairing_code` is the code of the key the
/// agent proved, and `None` until it has proved one. `added_at` is `None` only in an answer
/// that a broker stored under schema version 2 and gives again ([`crate::stored`]). `client`
/// is the kind of client the owner named in `add_agent`; it is `None` only for an agent the
/// owner did not add.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    pub agent_id: Id,
    pub revision: PositiveCount,
    pub registered_name: Label,
    pub destination: Destination,
    #[serde(deserialize_with = "nullable")]
    pub client: Option<AgentClient>,
    pub enrollment: AgentEnrollment,
    #[serde(deserialize_with = "nullable")]
    pub pairing_code: Option<PairingCode>,
    #[serde(deserialize_with = "nullable")]
    pub added_at: Option<Timestamp>,
    #[serde(deserialize_with = "nullable")]
    pub key_proven_at: Option<Timestamp>,
    #[serde(deserialize_with = "nullable")]
    pub approved_at: Option<Timestamp>,
    #[serde(deserialize_with = "nullable")]
    pub removed_at: Option<Timestamp>,
    pub paused: bool,
    #[serde(deserialize_with = "nullable")]
    pub session: Option<Session>,
    #[serde(deserialize_with = "nullable")]
    pub last_tool_call_at: Option<Timestamp>,
}

/// `owner#/$defs/calendar`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Calendar {
    pub calendar_id: Id,
    pub name: Label,
    pub access: CalendarAccess,
    pub selected: bool,
    /// True for the calendar the owner selected as Agent Drafts (CD-Q05f).
    pub agent_drafts: bool,
}

/// The connection states of an account (plan section 10.1, README section 14).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountState {
    AwaitingConsent,
    Connected,
    ConsentDenied,
    PartialGrant,
    Cancelled,
    AccountMismatch,
    ReauthenticationRequired,
    Disconnected,
}

/// `owner#/$defs/account`: a connected Google account. It never carries a token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Account {
    pub account_id: Id,
    pub revision: PositiveCount,
    pub state: AccountState,
    #[serde(deserialize_with = "nullable")]
    pub email: Option<Email>,
    pub calendars: AnyList<Calendar>,
}

/// Why the last attempt to connect an account to Google ended
/// (`owner#/$defs/provider_account.last_connect_problem`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectProblem {
    TimedOut,
    Superseded,
    BrowserNotOpened,
    GoogleError,
    ClientRejected,
    Network,
    IdentityCheckFailed,
    UnexpectedScope,
    NoRefreshToken,
    TokenStore,
    AlreadyAnAccount,
}

/// Why the broker's last list of an account's calendars failed
/// (`owner#/$defs/provider_account.last_list_problem`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListProblem {
    TooManyCalendars,
    TooLarge,
    Rejected,
    Unreachable,
}

/// Whether Google confirmed a revocation of an account's connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Revocation {
    NotRequested,
    Pending,
    Revoked,
    NotConfirmed,
}

/// `owner#/$defs/provider_account`: what the broker knows about an account's connection to
/// Google beyond its state: why the last attempt to connect ended, which permission a partial
/// grant lacked (Google's name of the scope), when the connection is due to end, and whether
/// Google confirmed a revocation. Owner only. `last_list_problem` says why the broker's last
/// list of the account's calendars failed, and is `None` after a list that succeeded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderAccount {
    pub account_id: Id,
    #[serde(deserialize_with = "nullable")]
    pub last_connect_problem: Option<ConnectProblem>,
    #[serde(deserialize_with = "nullable")]
    pub last_list_problem: Option<ListProblem>,
    /// A string of 1 to 200 characters, the constraint of `label`.
    #[serde(deserialize_with = "nullable")]
    pub missing_scope: Option<Label>,
    #[serde(deserialize_with = "nullable")]
    pub refresh_token_ends_at: Option<Timestamp>,
    pub revocation: Revocation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProverHealth {
    Ready,
    Starting,
    Unavailable,
}

/// `owner#/$defs/prover_problem`: why the rule prover is unavailable (README section 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProverProblem {
    /// This run has no rule prover.
    NotConfigured,
    /// The rule checker is not installed on this machine.
    NotInstalled,
    /// The prover could not start, or stopped; the broker starts it again by itself.
    FailedWillRetry,
    /// The prover could not start, or stopped, and the broker will not try again in this run.
    Failed,
    /// The rule theory on this machine is not the one this broker was built with.
    WrongRules,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreHealth {
    Ready,
    Locked,
    KeychainRefused,
}

/// `owner#/$defs/health`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Health {
    pub prover: ProverHealth,
    /// Null while the prover is ready or starting; names the cause while it is unavailable,
    /// or is null when the broker does not know it.
    #[serde(deserialize_with = "nullable")]
    pub prover_problem: Option<ProverProblem>,
    pub store: StoreHealth,
    pub agent_channel: AgentChannelHealth,
    /// Null while the agent channel is listening; names the cause while it is not, or is null
    /// when there is none to name.
    #[serde(deserialize_with = "nullable")]
    pub agent_channel_problem: Option<AgentChannelProblem>,
}

/// Whether agents can connect (`owner#/$defs/health.agent_channel`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentChannelHealth {
    Listening,
    NotListening,
}

/// Why the agent channel is not listening (`owner#/$defs/health.agent_channel_problem`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentChannelProblem {
    PathTooLong,
    RunDirUnsafe,
    RunDirOwner,
    StaleNotSocket,
    SocketMode,
    BindFailed,
}

/// `owner#/$defs/time_range`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeRange {
    pub days: Weekdays,
    pub start: LocalTime,
    pub end: LocalTime,
}

/// One protected time in the preferences.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtectedTime {
    pub enabled: bool,
    pub range: TimeRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartOfDay {
    Any,
    Morning,
    Afternoon,
}

/// The uninterrupted-time preference: 1 to 1440 minutes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Uninterrupted {
    pub enabled: bool,
    pub minimum_minutes: Integer<1, 1440>,
    pub part_of_day: PartOfDay,
}

/// The buffer preference: 1 to 240 minutes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Buffer {
    pub enabled: bool,
    pub minutes: Integer<1, 240>,
}

/// `owner#/$defs/preferences_settings`: they ship empty; `None` or an empty list means not
/// configured (CD-Q11i).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreferencesSettings {
    #[serde(deserialize_with = "nullable")]
    pub zone: Option<IanaZone>,
    #[serde(deserialize_with = "nullable")]
    pub workday: Option<TimeRange>,
    pub protected_times: List<ProtectedTime, 0, 50, false>,
    #[serde(deserialize_with = "nullable")]
    pub uninterrupted: Option<Uninterrupted>,
    #[serde(deserialize_with = "nullable")]
    pub buffer: Option<Buffer>,
    pub warning_calendars: List<CalendarRef, 0, 200, true>,
}

/// `owner#/$defs/preferences`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub revision: Count,
    pub settings: PreferencesSettings,
}

/// `owner#/$defs/ruling`: the failing rule, a sentence and the proof to open (DEF-7).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ruling {
    pub rule: NameToken,
    pub explanation: Explanation,
    pub proof_id: Id,
}

/// `owner#/$defs/failure`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub code: FailureCode,
    pub explanation: Explanation,
}

/// `owner#/$defs/lifecycle_status`: the state and what it always carries. A blocked request
/// cannot exist without its ruling (DEF-7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LifecycleStatus {
    PendingApproval { approval_expires_at: Timestamp },
    Blocked { denial: Ruling },
    TerminalFailure { failure: Failure },
    Plain(PlainRequestState),
}

impl LifecycleStatus {
    #[must_use]
    pub fn state(&self) -> RequestState {
        match self {
            LifecycleStatus::PendingApproval { .. } => RequestState::PendingApproval,
            LifecycleStatus::Blocked { .. } => RequestState::Blocked,
            LifecycleStatus::TerminalFailure { .. } => RequestState::TerminalFailure,
            LifecycleStatus::Plain(p) => p.state(),
        }
    }
}

impl Serialize for LifecycleStatus {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let state = self.state().as_str();
        match self {
            LifecycleStatus::PendingApproval {
                approval_expires_at,
            } => tagged::join(
                s,
                "state",
                state,
                &json!({ "approval_expires_at": approval_expires_at }),
            ),
            LifecycleStatus::Blocked { denial } => {
                tagged::join(s, "state", state, &json!({ "denial": denial }))
            }
            LifecycleStatus::TerminalFailure { failure } => {
                tagged::join(s, "state", state, &json!({ "failure": failure }))
            }
            LifecycleStatus::Plain(_) => tagged::join(s, "state", state, &tagged::Empty {}),
        }
    }
}

impl<'de> Deserialize<'de> for LifecycleStatus {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (state, mut rest) = tagged::split(d, "state")?;
        let status = match state.as_str() {
            "pending_approval" => LifecycleStatus::PendingApproval {
                approval_expires_at: tagged::read(tagged::take(&mut rest, "approval_expires_at")?)?,
            },
            "blocked" => LifecycleStatus::Blocked {
                denial: tagged::read(tagged::take(&mut rest, "denial")?)?,
            },
            "terminal_failure" => LifecycleStatus::TerminalFailure {
                failure: tagged::read(tagged::take(&mut rest, "failure")?)?,
            },
            other => LifecycleStatus::Plain(tagged::read(Value::String(other.to_owned()))?),
        };
        tagged::nothing(&rest)?;
        Ok(status)
    }
}

/// `owner#/$defs/request`: one logical request, without private event details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub request_id: Id,
    pub revision: PositiveCount,
    pub agent_id: Id,
    pub tool: ToolName,
    #[serde(deserialize_with = "nullable")]
    pub account_id: Option<Id>,
    pub received_at: Timestamp,
    pub updated_at: Timestamp,
    pub request_digest: Digest,
    pub status: LifecycleStatus,
}

/// `owner#/$defs/approval_card`: what the owner is asked to approve.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalCard {
    pub scope: Scope,
    pub proposed_duration: Duration,
    pub insufficiency: Ruling,
    pub expires_at: Timestamp,
}

/// The approve branch of `owner#/$defs/approval`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approved {
    pub command_id: Id,
    pub decided_at: Timestamp,
    pub approval_digest: Digest,
    pub scope: Scope,
    pub duration: Duration,
    pub effective_request_digest: Digest,
    pub grant_id: Id,
}

/// The deny branch of `owner#/$defs/approval`; its scope, duration, effective digest and
/// grant are written as `null`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Denied {
    pub command_id: Id,
    pub decided_at: Timestamp,
    pub approval_digest: Digest,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeniedWire {
    command_id: Id,
    decided_at: Timestamp,
    approval_digest: Digest,
    scope: crate::scalar::Null,
    duration: crate::scalar::Null,
    effective_request_digest: crate::scalar::Null,
    grant_id: crate::scalar::Null,
}

/// `owner#/$defs/approval`: the exact snapshot of the owner's decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Approval {
    Approve(Approved),
    Deny(Denied),
}

impl Serialize for Approval {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Approval::Approve(a) => tagged::join(s, "decision", "approve", a),
            Approval::Deny(d) => tagged::join(
                s,
                "decision",
                "deny",
                &DeniedWire {
                    command_id: d.command_id.clone(),
                    decided_at: d.decided_at.clone(),
                    approval_digest: d.approval_digest.clone(),
                    scope: crate::scalar::Null,
                    duration: crate::scalar::Null,
                    effective_request_digest: crate::scalar::Null,
                    grant_id: crate::scalar::Null,
                },
            ),
        }
    }
}

impl<'de> Deserialize<'de> for Approval {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (decision, rest) = tagged::split(d, "decision")?;
        match decision.as_str() {
            "approve" => Ok(Approval::Approve(tagged::body(rest)?)),
            "deny" => {
                let w: DeniedWire = tagged::body(rest)?;
                Ok(Approval::Deny(Denied {
                    command_id: w.command_id,
                    decided_at: w.decided_at,
                    approval_digest: w.approval_digest,
                }))
            }
            other => Err(tagged::unknown("decision", other)),
        }
    }
}

/// The marker for the authorization profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthorizationProfileName;
impl ConstText for AuthorizationProfileName {
    const VALUE: &'static str = AUTHORIZATION_PROFILE;
}

/// `lifetime_seconds` of an authorization: always 60 (CD-Q11f).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LifetimeSeconds;

impl LifetimeSeconds {
    #[must_use]
    pub const fn get(self) -> u64 {
        AUTHORIZATION_LIFETIME_SECONDS
    }
}

impl Serialize for LifetimeSeconds {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(AUTHORIZATION_LIFETIME_SECONDS)
    }
}

impl<'de> Deserialize<'de> for LifetimeSeconds {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        if u64::deserialize(d)? == AUTHORIZATION_LIFETIME_SECONDS {
            Ok(LifetimeSeconds)
        } else {
            Err(de::Error::custom("lifetime_seconds must be 60"))
        }
    }
}

/// `owner#/$defs/authorization`: the broker's exact-action authorization, profile
/// `fabric-broker-authorization/1`, used once (README section 11).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Authorization {
    pub profile: Fixed<AuthorizationProfileName>,
    pub authorization_id: Id,
    pub request_id: Id,
    pub agent_id: Id,
    pub action_digest: Digest,
    pub proof_id: Id,
    pub grant_id: Id,
    pub issued_at: Timestamp,
    pub expires_at: Timestamp,
    pub lifetime_seconds: LifetimeSeconds,
    #[serde(deserialize_with = "nullable")]
    pub consumed_at: Option<Timestamp>,
}

impl Authorization {
    /// Whether `expires_at` is exactly `issued_at` plus `lifetime_seconds` (README section 11).
    /// A timestamp whose day does not exist in its month makes this false. This is a broker
    /// check, not a schema check (README section 5).
    #[must_use]
    pub fn expiry_matches_lifetime(&self) -> bool {
        match (self.issued_at.unix_millis(), self.expires_at.unix_millis()) {
            (Some(issued), Some(expires)) => {
                expires - issued == (self.lifetime_seconds.get() as i64) * 1000
            }
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofPurpose {
    Classification,
    FreshPermit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofOutcome {
    Permit,
    NeedApproval,
    Block,
    Undecided,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Holds,
    Fails,
    Undecided,
}

/// One rule's verdict in a proof report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleVerdict {
    pub rule: NameToken,
    pub verdict: Verdict,
}

/// `owner#/$defs/proof_report`: one live Isabelle check; digests and verdicts, never calendar content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProofReport {
    pub proof_id: Id,
    pub request_id: Id,
    pub purpose: ProofPurpose,
    pub outcome: ProofOutcome,
    #[serde(deserialize_with = "nullable")]
    pub failing_rule: Option<NameToken>,
    pub rule_verdicts: List<RuleVerdict, 0, 200, false>,
    pub isabelle_version: Label,
    pub theory_hash: Digest,
    pub context_digest: Digest,
    pub action_digest: Digest,
    pub started_at: Timestamp,
    pub elapsed_ms: Count,
}

/// The kind of an action the broker took itself (`owner#/$defs/system_proof_report.action`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemActionKind {
    Refresh,
    Reconcile,
    ListCalendars,
}

/// An action the broker took itself for a request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemAction {
    pub kind: SystemActionKind,
    pub account_id: Id,
    #[serde(deserialize_with = "nullable")]
    pub calendar_id: Option<Id>,
    #[serde(deserialize_with = "nullable")]
    pub event_id: Option<ProviderRef>,
}

/// The prover that decided a system proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProverKind {
    Isabelle,
    Synthetic,
}

/// `owner#/$defs/system_proof_report`: the record of one proof of an action the broker took
/// itself for a request: a read to look at an event, a reconciliation. It holds no calendar
/// content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemProofReport {
    pub proof_id: Id,
    pub action: SystemAction,
    pub outcome: ProofOutcome,
    pub prover_kind: ProverKind,
    pub recorded_at: Timestamp,
}

/// What a release gave the agent (`owner#/$defs/release_record.kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseKind {
    Calendars,
    Availability,
    Events,
    Effect,
}

/// `owner#/$defs/release_record`: one release of a result to the agent, under its own proof.
/// `request_id` is the request that asked for the release.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseRecord {
    pub release_id: Id,
    pub request_id: Id,
    pub kind: ReleaseKind,
    pub proof_id: Id,
    pub grant_ids: List<Id, 0, 50, true>,
    pub released_at: Timestamp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptOutcome {
    Applied,
    TransientFailure,
    TerminalFailure,
    OutcomeUnknown,
}

/// `owner#/$defs/attempt`: one dispatch to the provider, with its own fresh proof.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attempt {
    pub number: PositiveCount,
    pub proof_id: Id,
    pub started_at: Timestamp,
    #[serde(deserialize_with = "nullable")]
    pub finished_at: Option<Timestamp>,
    #[serde(deserialize_with = "nullable")]
    pub outcome: Option<AttemptOutcome>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningKind {
    ProtectedTime,
    UninterruptedTime,
    Buffer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningStatus {
    Warning,
    Unavailable,
    IncompleteData,
}

/// `owner#/$defs/warning`: an advisory notice for the owner only (G0-WARNDISC).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Warning {
    pub kind: WarningKind,
    pub status: WarningStatus,
    pub explanation: Explanation,
    #[serde(deserialize_with = "nullable")]
    pub observed_at: Option<Timestamp>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Redaction {
    Redacted,
    Full,
}

/// The event content of a request, present only when the retrieval was not redacted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateDetails {
    pub action: Action,
    #[serde(deserialize_with = "nullable")]
    pub before: Option<EventView>,
    /// The event as Google holds it, for a request whose applied change differs from what was
    /// approved: the event Google returned in its answer to the change, or the event of the
    /// broker's last read-back. Null for every other request, and null too when the broker kept
    /// no view of the event or the view passes the broker's bound.
    #[serde(deserialize_with = "nullable")]
    pub after: Option<EventView>,
    pub warnings: List<Warning, 0, 50, false>,
}

/// `owner#/$defs/request_details`: everything the owner may open about one request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestDetails {
    pub request: Request,
    pub redaction: Redaction,
    #[serde(deserialize_with = "nullable")]
    pub private: Option<PrivateDetails>,
    #[serde(deserialize_with = "nullable")]
    pub card: Option<ApprovalCard>,
    #[serde(deserialize_with = "nullable")]
    pub approval: Option<Approval>,
    pub proof_ids: List<Id, 0, 200, true>,
    #[serde(deserialize_with = "nullable")]
    pub authorization: Option<Authorization>,
    pub attempts: List<Attempt, 0, 20, false>,
    #[serde(deserialize_with = "nullable")]
    pub provider_event: Option<EventRef>,
    /// Why no proof decided the request: null unless its state is `undecided`, and null there
    /// when the broker did not record why.
    #[serde(deserialize_with = "nullable")]
    pub undecided_reason: Option<UndecidedReason>,
    /// Why the broker handed the request to the owner to resolve.
    #[serde(deserialize_with = "nullable")]
    pub owner_resolution_reason: Option<OwnerResolutionReason>,
    #[serde(deserialize_with = "nullable")]
    pub notification: Option<Notification>,
    pub system_proofs: List<SystemProofReport, 0, 200, false>,
    pub releases: List<ReleaseRecord, 0, 200, false>,
}

/// Whether the attendees were notified of a change (`owner#/$defs/request_details.notification`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Notification {
    Sent,
    NotRequested,
    Unknown,
}

/// `owner#/$defs/snapshot`: the broker's state at one cursor (README section 14).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub cursor: Count,
    pub health: Health,
    pub profiles: Profiles,
    pub accounts: AnyList<Account>,
    pub provider_accounts: AnyList<ProviderAccount>,
    pub agents: AnyList<Agent>,
    pub recipient_sets: AnyList<RecipientSet>,
    pub preferences: Preferences,
    pub requests: AnyList<Request>,
    pub grants: AnyList<Grant>,
}

/// `owner#/$defs/change`: one ordered change to the broker's state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Account(Account),
    ProviderAccount(ProviderAccount),
    Agent(Agent),
    Profiles(Profiles),
    RecipientSet(RecipientSet),
    RecipientSetDeleted(Id),
    Preferences(Preferences),
    Request(Request),
    Grant(Grant),
    Health(Health),
    /// The subscriber fell behind and must take a snapshot.
    SnapshotRequired,
}

impl Change {
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Change::Account(_) => "account",
            Change::ProviderAccount(_) => "provider_account",
            Change::Agent(_) => "agent",
            Change::Profiles(_) => "profiles",
            Change::RecipientSet(_) => "recipient_set",
            Change::RecipientSetDeleted(_) => "recipient_set_deleted",
            Change::Preferences(_) => "preferences",
            Change::Request(_) => "request",
            Change::Grant(_) => "grant",
            Change::Health(_) => "health",
            Change::SnapshotRequired => "snapshot_required",
        }
    }
}

impl Serialize for Change {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let kind = self.kind();
        let body = match self {
            Change::Account(v) => json!({ "account": v }),
            Change::ProviderAccount(v) => json!({ "provider_account": v }),
            Change::Agent(v) => json!({ "agent": v }),
            Change::Profiles(v) => json!({ "profiles": v }),
            Change::RecipientSet(v) => json!({ "recipient_set": v }),
            Change::RecipientSetDeleted(v) => json!({ "recipient_set_id": v }),
            Change::Preferences(v) => json!({ "preferences": v }),
            Change::Request(v) => json!({ "request": v }),
            Change::Grant(v) => json!({ "grant": v }),
            Change::Health(v) => json!({ "health": v }),
            Change::SnapshotRequired => json!({}),
        };
        tagged::join(s, "kind", kind, &body)
    }
}

impl<'de> Deserialize<'de> for Change {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (kind, mut rest) = tagged::split(d, "kind")?;
        let mut member = |name| tagged::take::<D::Error>(&mut rest, name);
        let change = match kind.as_str() {
            "account" => Change::Account(tagged::read(member("account")?)?),
            "provider_account" => {
                Change::ProviderAccount(tagged::read(member("provider_account")?)?)
            }
            "agent" => Change::Agent(tagged::read(member("agent")?)?),
            "profiles" => Change::Profiles(tagged::read(member("profiles")?)?),
            "recipient_set" => Change::RecipientSet(tagged::read(member("recipient_set")?)?),
            "recipient_set_deleted" => {
                Change::RecipientSetDeleted(tagged::read(member("recipient_set_id")?)?)
            }
            "preferences" => Change::Preferences(tagged::read(member("preferences")?)?),
            "request" => Change::Request(tagged::read(member("request")?)?),
            "grant" => Change::Grant(tagged::read(member("grant")?)?),
            "health" => Change::Health(tagged::read(member("health")?)?),
            "snapshot_required" => Change::SnapshotRequired,
            other => return Err(tagged::unknown("kind", other)),
        };
        tagged::nothing(&rest)?;
        Ok(change)
    }
}

/// `owner#/$defs/party`: a side of the hello exchange.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Party {
    pub name: Label,
    pub build: Label,
}

/// `owner#/$defs/params_hello`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelloParams {
    pub client: Party,
    pub schema_version: Count,
}

/// `owner#/$defs/result_hello`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelloResult {
    pub broker: Party,
    pub schema_version: PositiveCount,
    pub capabilities: List<NameToken, 0, 64, true>,
    pub limits: Limits,
}

/// `owner#/$defs/params_get_snapshot`: no parameters.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetSnapshotParams {}

/// `owner#/$defs/params_connect_google`: `None` adds an account; an id reconnects it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectGoogleParams {
    #[serde(deserialize_with = "nullable")]
    pub account_id: Option<Id>,
}

/// `owner#/$defs/result_connect_google`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectGoogleResult {
    pub commit_sequence: PositiveCount,
    pub account: Account,
}

/// The `change` of `params_save_profile`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileChange {
    /// Puts a profile in force.
    Activate { profile_id: Id },
    /// Saves a custom profile, with the digest of the expansion the owner confirmed, if any.
    SaveCustom {
        profile: CustomProfileDraft,
        acknowledged_expansion: Option<Digest>,
    },
    /// Deletes a custom profile. The profile in force cannot be deleted; a grant made under
    /// it keeps naming its id and version.
    DeleteCustom { profile_id: CustomProfileId },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveCustomBody {
    profile: CustomProfileDraft,
    #[serde(deserialize_with = "nullable")]
    acknowledged_expansion: Option<Digest>,
}

impl Serialize for ProfileChange {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            ProfileChange::Activate { profile_id } => {
                tagged::join(s, "kind", "activate", &json!({ "profile_id": profile_id }))
            }
            ProfileChange::SaveCustom {
                profile,
                acknowledged_expansion,
            } => tagged::join(
                s,
                "kind",
                "save_custom",
                &json!({ "profile": profile, "acknowledged_expansion": acknowledged_expansion }),
            ),
            ProfileChange::DeleteCustom { profile_id } => tagged::join(
                s,
                "kind",
                "delete_custom",
                &json!({ "profile_id": profile_id }),
            ),
        }
    }
}

impl<'de> Deserialize<'de> for ProfileChange {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (kind, mut rest) = tagged::split(d, "kind")?;
        match kind.as_str() {
            "activate" => {
                let profile_id = tagged::read(tagged::take(&mut rest, "profile_id")?)?;
                tagged::nothing(&rest)?;
                Ok(ProfileChange::Activate { profile_id })
            }
            "save_custom" => {
                let b: SaveCustomBody = tagged::body(rest)?;
                Ok(ProfileChange::SaveCustom {
                    profile: b.profile,
                    acknowledged_expansion: b.acknowledged_expansion,
                })
            }
            "delete_custom" => {
                let profile_id = tagged::read(tagged::take(&mut rest, "profile_id")?)?;
                tagged::nothing(&rest)?;
                Ok(ProfileChange::DeleteCustom { profile_id })
            }
            other => Err(tagged::unknown("kind", other)),
        }
    }
}

/// `owner#/$defs/params_save_profile`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveProfileParams {
    pub expected_revision: Count,
    pub change: ProfileChange,
}

/// `owner#/$defs/result_save_profile`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveProfileResult {
    pub commit_sequence: PositiveCount,
    pub profiles: Profiles,
}

/// `owner#/$defs/params_save_preferences`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavePreferencesParams {
    pub expected_revision: Count,
    pub settings: PreferencesSettings,
}

/// `owner#/$defs/result_save_preferences`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavePreferencesResult {
    pub commit_sequence: PositiveCount,
    pub preferences: Preferences,
}

/// The `change` of `params_save_group`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupChange {
    /// Saves a recipient set; `None` with expected revision 0 creates one.
    Save {
        recipient_set_id: Option<Id>,
        definition: RecipientSetDefinition,
    },
    /// Deletes the selector. Nothing changes at Google.
    Delete { recipient_set_id: Id },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveGroupBody {
    #[serde(deserialize_with = "nullable")]
    recipient_set_id: Option<Id>,
    definition: RecipientSetDefinition,
}

impl Serialize for GroupChange {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            GroupChange::Save {
                recipient_set_id,
                definition,
            } => tagged::join(
                s,
                "kind",
                "save",
                &json!({ "recipient_set_id": recipient_set_id, "definition": definition }),
            ),
            GroupChange::Delete { recipient_set_id } => tagged::join(
                s,
                "kind",
                "delete",
                &json!({ "recipient_set_id": recipient_set_id }),
            ),
        }
    }
}

impl<'de> Deserialize<'de> for GroupChange {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (kind, mut rest) = tagged::split(d, "kind")?;
        match kind.as_str() {
            "save" => {
                let b: SaveGroupBody = tagged::body(rest)?;
                Ok(GroupChange::Save {
                    recipient_set_id: b.recipient_set_id,
                    definition: b.definition,
                })
            }
            "delete" => {
                let recipient_set_id = tagged::read(tagged::take(&mut rest, "recipient_set_id")?)?;
                tagged::nothing(&rest)?;
                Ok(GroupChange::Delete { recipient_set_id })
            }
            other => Err(tagged::unknown("kind", other)),
        }
    }
}

/// `owner#/$defs/params_save_group`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveGroupParams {
    pub expected_revision: Count,
    pub change: GroupChange,
}

/// `owner#/$defs/result_save_group`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveGroupResult {
    pub commit_sequence: PositiveCount,
    #[serde(deserialize_with = "nullable")]
    pub recipient_set: Option<RecipientSet>,
}

/// The owner's decision in `params_decide_request`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestDecision {
    /// `selected` is `None` to approve the scope as asked, or a narrower scope.
    Approve {
        selected: Option<Scope>,
        duration: Duration,
    },
    /// Written with `selected` and `duration` null.
    Deny,
}

/// `owner#/$defs/params_decide_request`: the decision on one request as the owner saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecideRequestParams {
    pub request_id: Id,
    pub request_digest: Digest,
    pub expected_revision: Count,
    pub decision: RequestDecision,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecideApproveBody {
    request_id: Id,
    request_digest: Digest,
    expected_revision: Count,
    #[serde(deserialize_with = "nullable")]
    selected: Option<Scope>,
    duration: Duration,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecideDenyBody {
    request_id: Id,
    request_digest: Digest,
    expected_revision: Count,
    #[allow(dead_code)]
    selected: crate::scalar::Null,
    #[allow(dead_code)]
    duration: crate::scalar::Null,
}

impl Serialize for DecideRequestParams {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let (decision, selected, duration) = match &self.decision {
            RequestDecision::Approve { selected, duration } => {
                ("approve", to_value(selected), to_value(duration))
            }
            RequestDecision::Deny => ("deny", Value::Null, Value::Null),
        };
        json!({
            "request_id": self.request_id,
            "request_digest": self.request_digest,
            "expected_revision": self.expected_revision,
            "decision": decision,
            "selected": selected,
            "duration": duration,
        })
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for DecideRequestParams {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (decision, rest) = tagged::split(d, "decision")?;
        match decision.as_str() {
            "approve" => {
                let b: DecideApproveBody = tagged::body(rest)?;
                Ok(DecideRequestParams {
                    request_id: b.request_id,
                    request_digest: b.request_digest,
                    expected_revision: b.expected_revision,
                    decision: RequestDecision::Approve {
                        selected: b.selected,
                        duration: b.duration,
                    },
                })
            }
            "deny" => {
                let b: DecideDenyBody = tagged::body(rest)?;
                Ok(DecideRequestParams {
                    request_id: b.request_id,
                    request_digest: b.request_digest,
                    expected_revision: b.expected_revision,
                    decision: RequestDecision::Deny,
                })
            }
            other => Err(tagged::unknown("decision", other)),
        }
    }
}

/// `owner#/$defs/result_decide_request`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecideRequestResult {
    pub commit_sequence: PositiveCount,
    pub request: Request,
    pub approval: Approval,
}

/// Which agents `pause_agent` pauses or resumes (README section 14).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PauseTarget {
    Agent {
        agent_id: Id,
        expected_revision: Count,
    },
    AllAgents,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PauseAgentBody {
    agent_id: Id,
    expected_revision: Count,
}

impl Serialize for PauseTarget {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            PauseTarget::Agent {
                agent_id,
                expected_revision,
            } => tagged::join(
                s,
                "kind",
                "agent",
                &json!({ "agent_id": agent_id, "expected_revision": expected_revision }),
            ),
            PauseTarget::AllAgents => tagged::join(s, "kind", "all_agents", &tagged::Empty {}),
        }
    }
}

impl<'de> Deserialize<'de> for PauseTarget {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (kind, rest) = tagged::split(d, "kind")?;
        match kind.as_str() {
            "agent" => {
                let b: PauseAgentBody = tagged::body(rest)?;
                Ok(PauseTarget::Agent {
                    agent_id: b.agent_id,
                    expected_revision: b.expected_revision,
                })
            }
            "all_agents" => tagged::nothing(&rest).map(|()| PauseTarget::AllAgents),
            other => Err(tagged::unknown("kind", other)),
        }
    }
}

/// `owner#/$defs/params_pause_agent`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PauseAgentParams {
    pub target: PauseTarget,
    pub paused: bool,
}

/// `owner#/$defs/result_pause_agent`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PauseAgentResult {
    pub commit_sequence: PositiveCount,
    pub agents: AnyList<Agent>,
}

/// `owner#/$defs/params_revoke_grant`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeGrantParams {
    pub grant_id: Id,
    pub expected_revision: Count,
}

/// `owner#/$defs/result_revoke_grant`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeGrantResult {
    pub commit_sequence: PositiveCount,
    pub grant: Grant,
}

/// `owner#/$defs/params_get_request_details`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetRequestDetailsParams {
    pub request_id: Id,
    pub redaction: Redaction,
}

/// `owner#/$defs/params_get_proof` (README section 14).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetProofParams {
    pub proof_id: Id,
}

/// `owner#/$defs/params_subscribe`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscribeParams {
    pub cursor: Count,
}

/// `owner#/$defs/result_subscribe`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscribeResult {
    pub cursor: Count,
}

/// `owner#/$defs/params_disconnect_google`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisconnectGoogleParams {
    pub account_id: Id,
    pub expected_revision: Count,
    pub revoke_provider_token: bool,
}

/// `owner#/$defs/result_disconnect_google`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisconnectGoogleResult {
    pub commit_sequence: PositiveCount,
    pub account: Account,
    pub ended_grant_ids: List<Id, 0, { usize::MAX }, true>,
}

/// `owner#/$defs/params_select_calendars`: the whole selection of one account's calendars,
/// and its Agent Drafts calendar, which must be in `selected`, or `None` for none.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectCalendarsParams {
    pub account_id: Id,
    pub expected_revision: Count,
    pub selected: List<Id, 0, 250, true>,
    #[serde(deserialize_with = "nullable")]
    pub agent_drafts: Option<Id>,
}

/// `owner#/$defs/result_select_calendars`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectCalendarsResult {
    pub commit_sequence: PositiveCount,
    pub account: Account,
}

/// `owner#/$defs/params_refresh_calendars`: asks the broker to list one account's calendars
/// again. The list arrives later, as changes of the account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefreshCalendarsParams {
    pub account_id: Id,
}

/// `owner#/$defs/result_refresh_calendars`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefreshCalendarsResult {
    pub commit_sequence: PositiveCount,
}

/// The owner's statement in `resolve_outcome`: the change is at Google, or it is not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    Applied,
    Absent,
}

/// `owner#/$defs/params_resolve_outcome`: the owner's own statement about a request in
/// `needs_owner_resolution`. Nothing is sent to Google.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveOutcomeParams {
    pub request_id: Id,
    pub expected_revision: Count,
    pub resolution: Resolution,
}

/// `owner#/$defs/result_resolve_outcome`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveOutcomeResult {
    pub commit_sequence: PositiveCount,
    pub request: Request,
}

/// `owner#/$defs/agent_client`: the kind of client the owner named when adding an agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentClient {
    ClaudeDesktop,
    ClaudeCode,
    CodexDesktop,
}

/// `owner#/$defs/params_add_agent`: adds an agent that waits for approval. The broker gives
/// it an id and, from the kind of client, where its data goes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddAgentParams {
    pub registered_name: Label,
    pub client: AgentClient,
}

/// `owner#/$defs/result_add_agent`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddAgentResult {
    pub commit_sequence: PositiveCount,
    pub agent: Agent,
}

/// `owner#/$defs/params_approve_agent`: approves a waiting agent. `pairing_code` is the code
/// the owner saw; it must be the code of the key the agent proved last.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApproveAgentParams {
    pub agent_id: Id,
    pub expected_revision: Count,
    pub pairing_code: PairingCode,
}

/// `owner#/$defs/result_approve_agent`. `grant` is the calendar list grant made in the same
/// commit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApproveAgentResult {
    pub commit_sequence: PositiveCount,
    pub agent: Agent,
    pub grant: Grant,
}

/// `owner#/$defs/params_remove_agent`: removes an agent, waiting or approved. Ends its
/// session and its grants, and cancels its requests that were not yet handed to Google. The
/// agent's record stays, marked removed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveAgentParams {
    pub agent_id: Id,
    pub expected_revision: Count,
}

/// `owner#/$defs/result_remove_agent`: the removed agent and the grants its removal ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveAgentResult {
    pub commit_sequence: PositiveCount,
    pub agent: Agent,
    pub ended_grant_ids: List<Id, 0, { usize::MAX }, true>,
}

/// `owner#/$defs/params_get_agent_configuration`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetAgentConfigurationParams {
    pub agent_id: Id,
}

/// `owner#/$defs/result_get_agent_configuration`: the text that starts this agent's bridge in
/// the client the owner named. `configuration` is `None` when the broker cannot name an
/// installed bridge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetAgentConfigurationResult {
    #[serde(deserialize_with = "nullable")]
    pub configuration: Option<ConfigurationText>,
}

/// `owner#/$defs/params_list_ended_requests`: requests that have ended, newest first.
/// `before` is `None` for the first page, or the id of the last request of the page before.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListEndedRequestsParams {
    #[serde(deserialize_with = "nullable")]
    pub before: Option<Id>,
    pub limit: Integer<1, 100>,
}

/// `owner#/$defs/result_list_ended_requests`: `more` is true when older ended requests remain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListEndedRequestsResult {
    pub requests: List<Request, 0, 100, false>,
    pub more: bool,
}

/// The owner methods of `data/owner-methods.json`: the thirteen of plan section 9.1 and seven
/// that the plan's text needs (README section 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OwnerMethod {
    Hello,
    GetSnapshot,
    ConnectGoogle,
    SaveProfile,
    SavePreferences,
    SaveGroup,
    DecideRequest,
    PauseAgent,
    RevokeGrant,
    GetRequestDetails,
    GetProof,
    Subscribe,
    DisconnectGoogle,
    SelectCalendars,
    RefreshCalendars,
    ResolveOutcome,
    AddAgent,
    ApproveAgent,
    RemoveAgent,
    GetAgentConfiguration,
    ListEndedRequests,
}

impl OwnerMethod {
    /// Every method, in the order of `data/owner-methods.json`.
    pub const ALL: [OwnerMethod; 21] = [
        OwnerMethod::Hello,
        OwnerMethod::GetSnapshot,
        OwnerMethod::ConnectGoogle,
        OwnerMethod::SaveProfile,
        OwnerMethod::SavePreferences,
        OwnerMethod::SaveGroup,
        OwnerMethod::DecideRequest,
        OwnerMethod::PauseAgent,
        OwnerMethod::RevokeGrant,
        OwnerMethod::GetRequestDetails,
        OwnerMethod::GetProof,
        OwnerMethod::Subscribe,
        OwnerMethod::DisconnectGoogle,
        OwnerMethod::SelectCalendars,
        OwnerMethod::RefreshCalendars,
        OwnerMethod::ResolveOutcome,
        OwnerMethod::AddAgent,
        OwnerMethod::ApproveAgent,
        OwnerMethod::RemoveAgent,
        OwnerMethod::GetAgentConfiguration,
        OwnerMethod::ListEndedRequests,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            OwnerMethod::Hello => "hello",
            OwnerMethod::GetSnapshot => "get_snapshot",
            OwnerMethod::ConnectGoogle => "connect_google",
            OwnerMethod::SaveProfile => "save_profile",
            OwnerMethod::SavePreferences => "save_preferences",
            OwnerMethod::SaveGroup => "save_group",
            OwnerMethod::DecideRequest => "decide_request",
            OwnerMethod::PauseAgent => "pause_agent",
            OwnerMethod::RevokeGrant => "revoke_grant",
            OwnerMethod::GetRequestDetails => "get_request_details",
            OwnerMethod::GetProof => "get_proof",
            OwnerMethod::Subscribe => "subscribe",
            OwnerMethod::DisconnectGoogle => "disconnect_google",
            OwnerMethod::SelectCalendars => "select_calendars",
            OwnerMethod::RefreshCalendars => "refresh_calendars",
            OwnerMethod::ResolveOutcome => "resolve_outcome",
            OwnerMethod::AddAgent => "add_agent",
            OwnerMethod::ApproveAgent => "approve_agent",
            OwnerMethod::RemoveAgent => "remove_agent",
            OwnerMethod::GetAgentConfiguration => "get_agent_configuration",
            OwnerMethod::ListEndedRequests => "list_ended_requests",
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<OwnerMethod> {
        OwnerMethod::ALL.into_iter().find(|m| m.as_str() == name)
    }

    /// Whether the method changes broker state: its frame id is the command id and it is
    /// deduplicated by command digest (README section 4).
    #[must_use]
    pub const fn is_command(self) -> bool {
        matches!(
            self,
            OwnerMethod::ConnectGoogle
                | OwnerMethod::SaveProfile
                | OwnerMethod::SavePreferences
                | OwnerMethod::SaveGroup
                | OwnerMethod::DecideRequest
                | OwnerMethod::PauseAgent
                | OwnerMethod::RevokeGrant
                | OwnerMethod::DisconnectGoogle
                | OwnerMethod::SelectCalendars
                | OwnerMethod::RefreshCalendars
                | OwnerMethod::ResolveOutcome
                | OwnerMethod::AddAgent
                | OwnerMethod::ApproveAgent
                | OwnerMethod::RemoveAgent
        )
    }

    /// The `$defs` name of the params schema in `owner-channel.schema.json`.
    #[must_use]
    pub const fn params_definition(self) -> &'static str {
        match self {
            OwnerMethod::Hello => "params_hello",
            OwnerMethod::GetSnapshot => "params_get_snapshot",
            OwnerMethod::ConnectGoogle => "params_connect_google",
            OwnerMethod::SaveProfile => "params_save_profile",
            OwnerMethod::SavePreferences => "params_save_preferences",
            OwnerMethod::SaveGroup => "params_save_group",
            OwnerMethod::DecideRequest => "params_decide_request",
            OwnerMethod::PauseAgent => "params_pause_agent",
            OwnerMethod::RevokeGrant => "params_revoke_grant",
            OwnerMethod::GetRequestDetails => "params_get_request_details",
            OwnerMethod::GetProof => "params_get_proof",
            OwnerMethod::Subscribe => "params_subscribe",
            OwnerMethod::DisconnectGoogle => "params_disconnect_google",
            OwnerMethod::SelectCalendars => "params_select_calendars",
            OwnerMethod::RefreshCalendars => "params_refresh_calendars",
            OwnerMethod::ResolveOutcome => "params_resolve_outcome",
            OwnerMethod::AddAgent => "params_add_agent",
            OwnerMethod::ApproveAgent => "params_approve_agent",
            OwnerMethod::RemoveAgent => "params_remove_agent",
            OwnerMethod::GetAgentConfiguration => "params_get_agent_configuration",
            OwnerMethod::ListEndedRequests => "params_list_ended_requests",
        }
    }

    /// The `$defs` name of the result schema in `owner-channel.schema.json`.
    #[must_use]
    pub const fn result_definition(self) -> &'static str {
        match self {
            OwnerMethod::Hello => "result_hello",
            OwnerMethod::GetSnapshot => "snapshot",
            OwnerMethod::ConnectGoogle => "result_connect_google",
            OwnerMethod::SaveProfile => "result_save_profile",
            OwnerMethod::SavePreferences => "result_save_preferences",
            OwnerMethod::SaveGroup => "result_save_group",
            OwnerMethod::DecideRequest => "result_decide_request",
            OwnerMethod::PauseAgent => "result_pause_agent",
            OwnerMethod::RevokeGrant => "result_revoke_grant",
            OwnerMethod::GetRequestDetails => "request_details",
            OwnerMethod::GetProof => "proof_report",
            OwnerMethod::Subscribe => "result_subscribe",
            OwnerMethod::DisconnectGoogle => "result_disconnect_google",
            OwnerMethod::SelectCalendars => "result_select_calendars",
            OwnerMethod::RefreshCalendars => "result_refresh_calendars",
            OwnerMethod::ResolveOutcome => "result_resolve_outcome",
            OwnerMethod::AddAgent => "result_add_agent",
            OwnerMethod::ApproveAgent => "result_approve_agent",
            OwnerMethod::RemoveAgent => "result_remove_agent",
            OwnerMethod::GetAgentConfiguration => "result_get_agent_configuration",
            OwnerMethod::ListEndedRequests => "result_list_ended_requests",
        }
    }
}

/// The method and params of an owner request frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OwnerCall {
    Hello(HelloParams),
    GetSnapshot(GetSnapshotParams),
    ConnectGoogle(ConnectGoogleParams),
    SaveProfile(SaveProfileParams),
    SavePreferences(SavePreferencesParams),
    SaveGroup(SaveGroupParams),
    DecideRequest(DecideRequestParams),
    PauseAgent(PauseAgentParams),
    RevokeGrant(RevokeGrantParams),
    GetRequestDetails(GetRequestDetailsParams),
    GetProof(GetProofParams),
    Subscribe(SubscribeParams),
    DisconnectGoogle(DisconnectGoogleParams),
    SelectCalendars(SelectCalendarsParams),
    RefreshCalendars(RefreshCalendarsParams),
    ResolveOutcome(ResolveOutcomeParams),
    AddAgent(AddAgentParams),
    ApproveAgent(ApproveAgentParams),
    RemoveAgent(RemoveAgentParams),
    GetAgentConfiguration(GetAgentConfigurationParams),
    ListEndedRequests(ListEndedRequestsParams),
}

impl OwnerCall {
    #[must_use]
    pub fn method(&self) -> OwnerMethod {
        match self {
            OwnerCall::Hello(_) => OwnerMethod::Hello,
            OwnerCall::GetSnapshot(_) => OwnerMethod::GetSnapshot,
            OwnerCall::ConnectGoogle(_) => OwnerMethod::ConnectGoogle,
            OwnerCall::SaveProfile(_) => OwnerMethod::SaveProfile,
            OwnerCall::SavePreferences(_) => OwnerMethod::SavePreferences,
            OwnerCall::SaveGroup(_) => OwnerMethod::SaveGroup,
            OwnerCall::DecideRequest(_) => OwnerMethod::DecideRequest,
            OwnerCall::PauseAgent(_) => OwnerMethod::PauseAgent,
            OwnerCall::RevokeGrant(_) => OwnerMethod::RevokeGrant,
            OwnerCall::GetRequestDetails(_) => OwnerMethod::GetRequestDetails,
            OwnerCall::GetProof(_) => OwnerMethod::GetProof,
            OwnerCall::Subscribe(_) => OwnerMethod::Subscribe,
            OwnerCall::DisconnectGoogle(_) => OwnerMethod::DisconnectGoogle,
            OwnerCall::SelectCalendars(_) => OwnerMethod::SelectCalendars,
            OwnerCall::RefreshCalendars(_) => OwnerMethod::RefreshCalendars,
            OwnerCall::ResolveOutcome(_) => OwnerMethod::ResolveOutcome,
            OwnerCall::AddAgent(_) => OwnerMethod::AddAgent,
            OwnerCall::ApproveAgent(_) => OwnerMethod::ApproveAgent,
            OwnerCall::RemoveAgent(_) => OwnerMethod::RemoveAgent,
            OwnerCall::GetAgentConfiguration(_) => OwnerMethod::GetAgentConfiguration,
            OwnerCall::ListEndedRequests(_) => OwnerMethod::ListEndedRequests,
        }
    }

    #[must_use]
    pub fn params(&self) -> Value {
        match self {
            OwnerCall::Hello(p) => to_value(p),
            OwnerCall::GetSnapshot(p) => to_value(p),
            OwnerCall::ConnectGoogle(p) => to_value(p),
            OwnerCall::SaveProfile(p) => to_value(p),
            OwnerCall::SavePreferences(p) => to_value(p),
            OwnerCall::SaveGroup(p) => to_value(p),
            OwnerCall::DecideRequest(p) => to_value(p),
            OwnerCall::PauseAgent(p) => to_value(p),
            OwnerCall::RevokeGrant(p) => to_value(p),
            OwnerCall::GetRequestDetails(p) => to_value(p),
            OwnerCall::GetProof(p) => to_value(p),
            OwnerCall::Subscribe(p) => to_value(p),
            OwnerCall::DisconnectGoogle(p) => to_value(p),
            OwnerCall::SelectCalendars(p) => to_value(p),
            OwnerCall::RefreshCalendars(p) => to_value(p),
            OwnerCall::ResolveOutcome(p) => to_value(p),
            OwnerCall::AddAgent(p) => to_value(p),
            OwnerCall::ApproveAgent(p) => to_value(p),
            OwnerCall::RemoveAgent(p) => to_value(p),
            OwnerCall::GetAgentConfiguration(p) => to_value(p),
            OwnerCall::ListEndedRequests(p) => to_value(p),
        }
    }

    fn from_parts<E: de::Error>(method: OwnerMethod, params: Value) -> Result<OwnerCall, E> {
        use tagged::read;
        Ok(match method {
            OwnerMethod::Hello => OwnerCall::Hello(read(params)?),
            OwnerMethod::GetSnapshot => OwnerCall::GetSnapshot(read(params)?),
            OwnerMethod::ConnectGoogle => OwnerCall::ConnectGoogle(read(params)?),
            OwnerMethod::SaveProfile => OwnerCall::SaveProfile(read(params)?),
            OwnerMethod::SavePreferences => OwnerCall::SavePreferences(read(params)?),
            OwnerMethod::SaveGroup => OwnerCall::SaveGroup(read(params)?),
            OwnerMethod::DecideRequest => OwnerCall::DecideRequest(read(params)?),
            OwnerMethod::PauseAgent => OwnerCall::PauseAgent(read(params)?),
            OwnerMethod::RevokeGrant => OwnerCall::RevokeGrant(read(params)?),
            OwnerMethod::GetRequestDetails => OwnerCall::GetRequestDetails(read(params)?),
            OwnerMethod::GetProof => OwnerCall::GetProof(read(params)?),
            OwnerMethod::Subscribe => OwnerCall::Subscribe(read(params)?),
            OwnerMethod::DisconnectGoogle => OwnerCall::DisconnectGoogle(read(params)?),
            OwnerMethod::SelectCalendars => OwnerCall::SelectCalendars(read(params)?),
            OwnerMethod::RefreshCalendars => OwnerCall::RefreshCalendars(read(params)?),
            OwnerMethod::ResolveOutcome => OwnerCall::ResolveOutcome(read(params)?),
            OwnerMethod::AddAgent => OwnerCall::AddAgent(read(params)?),
            OwnerMethod::ApproveAgent => OwnerCall::ApproveAgent(read(params)?),
            OwnerMethod::RemoveAgent => OwnerCall::RemoveAgent(read(params)?),
            OwnerMethod::GetAgentConfiguration => OwnerCall::GetAgentConfiguration(read(params)?),
            OwnerMethod::ListEndedRequests => OwnerCall::ListEndedRequests(read(params)?),
        })
    }
}

/// `owner#/$defs/owner_request`: a frame from the Tauri host. For a command the frame id is
/// the command id (README section 14).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerRequest {
    pub id: Id,
    pub call: OwnerCall,
}

impl OwnerRequest {
    /// The command digest of `{"method", "params"}`, or `None` when the method is not a command.
    #[must_use]
    pub fn command_digest(&self) -> Option<Digest> {
        let method = self.call.method();
        method
            .is_command()
            .then(|| crate::digest::command_digest(method.as_str(), &self.call.params()))
    }
}

impl Serialize for OwnerRequest {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        json!({
            "protocol": PROTOCOL,
            "id": self.id,
            "method": self.call.method().as_str(),
            "params": self.call.params(),
        })
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for OwnerRequest {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (method, mut rest) = tagged::split(d, "method")?;
        let method =
            OwnerMethod::from_name(&method).ok_or_else(|| tagged::unknown("method", &method))?;
        let _: Protocol = tagged::read(tagged::take(&mut rest, "protocol")?)?;
        let id = tagged::read(tagged::take(&mut rest, "id")?)?;
        let params = tagged::take(&mut rest, "params")?;
        tagged::nothing(&rest)?;
        Ok(OwnerRequest {
            id,
            call: OwnerCall::from_parts(method, params)?,
        })
    }
}

/// The result of one owner method.
#[allow(
    clippy::large_enum_variant,
    reason = "a transport value is built once per frame; boxing would only complicate its use"
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OwnerResult {
    Hello(HelloResult),
    GetSnapshot(Snapshot),
    ConnectGoogle(ConnectGoogleResult),
    SaveProfile(SaveProfileResult),
    SavePreferences(SavePreferencesResult),
    SaveGroup(SaveGroupResult),
    DecideRequest(DecideRequestResult),
    PauseAgent(PauseAgentResult),
    RevokeGrant(RevokeGrantResult),
    GetRequestDetails(RequestDetails),
    GetProof(ProofReport),
    Subscribe(SubscribeResult),
    DisconnectGoogle(DisconnectGoogleResult),
    SelectCalendars(SelectCalendarsResult),
    RefreshCalendars(RefreshCalendarsResult),
    ResolveOutcome(ResolveOutcomeResult),
    AddAgent(AddAgentResult),
    ApproveAgent(ApproveAgentResult),
    RemoveAgent(RemoveAgentResult),
    GetAgentConfiguration(GetAgentConfigurationResult),
    ListEndedRequests(ListEndedRequestsResult),
}

impl OwnerResult {
    #[must_use]
    pub fn method(&self) -> OwnerMethod {
        match self {
            OwnerResult::Hello(_) => OwnerMethod::Hello,
            OwnerResult::GetSnapshot(_) => OwnerMethod::GetSnapshot,
            OwnerResult::ConnectGoogle(_) => OwnerMethod::ConnectGoogle,
            OwnerResult::SaveProfile(_) => OwnerMethod::SaveProfile,
            OwnerResult::SavePreferences(_) => OwnerMethod::SavePreferences,
            OwnerResult::SaveGroup(_) => OwnerMethod::SaveGroup,
            OwnerResult::DecideRequest(_) => OwnerMethod::DecideRequest,
            OwnerResult::PauseAgent(_) => OwnerMethod::PauseAgent,
            OwnerResult::RevokeGrant(_) => OwnerMethod::RevokeGrant,
            OwnerResult::GetRequestDetails(_) => OwnerMethod::GetRequestDetails,
            OwnerResult::GetProof(_) => OwnerMethod::GetProof,
            OwnerResult::Subscribe(_) => OwnerMethod::Subscribe,
            OwnerResult::DisconnectGoogle(_) => OwnerMethod::DisconnectGoogle,
            OwnerResult::SelectCalendars(_) => OwnerMethod::SelectCalendars,
            OwnerResult::RefreshCalendars(_) => OwnerMethod::RefreshCalendars,
            OwnerResult::ResolveOutcome(_) => OwnerMethod::ResolveOutcome,
            OwnerResult::AddAgent(_) => OwnerMethod::AddAgent,
            OwnerResult::ApproveAgent(_) => OwnerMethod::ApproveAgent,
            OwnerResult::RemoveAgent(_) => OwnerMethod::RemoveAgent,
            OwnerResult::GetAgentConfiguration(_) => OwnerMethod::GetAgentConfiguration,
            OwnerResult::ListEndedRequests(_) => OwnerMethod::ListEndedRequests,
        }
    }

    #[must_use]
    pub fn to_value(&self) -> Value {
        match self {
            OwnerResult::Hello(r) => to_value(r),
            OwnerResult::GetSnapshot(r) => to_value(r),
            OwnerResult::ConnectGoogle(r) => to_value(r),
            OwnerResult::SaveProfile(r) => to_value(r),
            OwnerResult::SavePreferences(r) => to_value(r),
            OwnerResult::SaveGroup(r) => to_value(r),
            OwnerResult::DecideRequest(r) => to_value(r),
            OwnerResult::PauseAgent(r) => to_value(r),
            OwnerResult::RevokeGrant(r) => to_value(r),
            OwnerResult::GetRequestDetails(r) => to_value(r),
            OwnerResult::GetProof(r) => to_value(r),
            OwnerResult::Subscribe(r) => to_value(r),
            OwnerResult::DisconnectGoogle(r) => to_value(r),
            OwnerResult::SelectCalendars(r) => to_value(r),
            OwnerResult::RefreshCalendars(r) => to_value(r),
            OwnerResult::ResolveOutcome(r) => to_value(r),
            OwnerResult::AddAgent(r) => to_value(r),
            OwnerResult::ApproveAgent(r) => to_value(r),
            OwnerResult::RemoveAgent(r) => to_value(r),
            OwnerResult::GetAgentConfiguration(r) => to_value(r),
            OwnerResult::ListEndedRequests(r) => to_value(r),
        }
    }

    fn from_parts<E: de::Error>(method: OwnerMethod, result: Value) -> Result<OwnerResult, E> {
        use tagged::read;
        Ok(match method {
            OwnerMethod::Hello => OwnerResult::Hello(read(result)?),
            OwnerMethod::GetSnapshot => OwnerResult::GetSnapshot(read(result)?),
            OwnerMethod::ConnectGoogle => OwnerResult::ConnectGoogle(read(result)?),
            OwnerMethod::SaveProfile => OwnerResult::SaveProfile(read(result)?),
            OwnerMethod::SavePreferences => OwnerResult::SavePreferences(read(result)?),
            OwnerMethod::SaveGroup => OwnerResult::SaveGroup(read(result)?),
            OwnerMethod::DecideRequest => OwnerResult::DecideRequest(read(result)?),
            OwnerMethod::PauseAgent => OwnerResult::PauseAgent(read(result)?),
            OwnerMethod::RevokeGrant => OwnerResult::RevokeGrant(read(result)?),
            OwnerMethod::GetRequestDetails => OwnerResult::GetRequestDetails(read(result)?),
            OwnerMethod::GetProof => OwnerResult::GetProof(read(result)?),
            OwnerMethod::Subscribe => OwnerResult::Subscribe(read(result)?),
            OwnerMethod::DisconnectGoogle => OwnerResult::DisconnectGoogle(read(result)?),
            OwnerMethod::SelectCalendars => OwnerResult::SelectCalendars(read(result)?),
            OwnerMethod::RefreshCalendars => OwnerResult::RefreshCalendars(read(result)?),
            OwnerMethod::ResolveOutcome => OwnerResult::ResolveOutcome(read(result)?),
            OwnerMethod::AddAgent => OwnerResult::AddAgent(read(result)?),
            OwnerMethod::ApproveAgent => OwnerResult::ApproveAgent(read(result)?),
            OwnerMethod::RemoveAgent => OwnerResult::RemoveAgent(read(result)?),
            OwnerMethod::GetAgentConfiguration => OwnerResult::GetAgentConfiguration(read(result)?),
            OwnerMethod::ListEndedRequests => OwnerResult::ListEndedRequests(read(result)?),
        })
    }
}

/// `owner#/$defs/owner_response`: the broker's answer, with the request's id and method
/// (README section 14), and either a result or an error.
#[allow(
    clippy::large_enum_variant,
    reason = "a transport value is built once per frame; boxing would only complicate its use"
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OwnerResponse {
    Result {
        id: Id,
        result: OwnerResult,
    },
    /// `id` and `method` are `None` when the frame could not be read that far.
    Error {
        id: Option<Id>,
        method: Option<MethodText>,
        error: OwnerError,
    },
}

/// Reads a response frame's members and its `protocol`, leaving the rest.
pub(crate) fn response_members<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<((), Map<String, Value>), D::Error> {
    let Value::Object(mut rest) = Value::deserialize(d)? else {
        return Err(de::Error::custom("expected an object"));
    };
    let _: Protocol = tagged::read(tagged::take(&mut rest, "protocol")?)?;
    Ok(((), rest))
}

impl Serialize for OwnerResponse {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            OwnerResponse::Result { id, result } => json!({
                "protocol": PROTOCOL,
                "id": id,
                "method": result.method().as_str(),
                "result": result.to_value(),
            }),
            OwnerResponse::Error { id, method, error } => json!({
                "protocol": PROTOCOL,
                "id": id,
                "method": method,
                "error": error,
            }),
        }
        .serialize(s)
    }
}

impl<'de> Deserialize<'de> for OwnerResponse {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let ((), mut rest) = response_members(d)?;
        let id = tagged::take(&mut rest, "id")?;
        let method = tagged::take(&mut rest, "method")?;
        let response = if let Some(error) = rest.remove("error") {
            OwnerResponse::Error {
                id: tagged::read(id)?,
                method: tagged::read(method)?,
                error: tagged::read(error)?,
            }
        } else {
            let Value::String(name) = method else {
                return Err(de::Error::custom("method is not a string"));
            };
            let method =
                OwnerMethod::from_name(&name).ok_or_else(|| tagged::unknown("method", &name))?;
            OwnerResponse::Result {
                id: tagged::read(id)?,
                result: OwnerResult::from_parts(method, tagged::take(&mut rest, "result")?)?,
            }
        };
        tagged::nothing(&rest)?;
        Ok(response)
    }
}

/// `owner#/$defs/owner_notice`: a change pushed to a subscriber; sequences rise by one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerNotice {
    pub sequence: PositiveCount,
    pub change: Change,
}

impl Serialize for OwnerNotice {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        json!({ "protocol": PROTOCOL, "sequence": self.sequence, "change": self.change })
            .serialize(s)
    }
}

impl<'de> Deserialize<'de> for OwnerNotice {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let ((), mut rest) = response_members(d)?;
        let sequence = tagged::read(tagged::take(&mut rest, "sequence")?)?;
        let change = tagged::read(tagged::take(&mut rest, "change")?)?;
        tagged::nothing(&rest)?;
        Ok(OwnerNotice { sequence, change })
    }
}
