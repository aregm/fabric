// TypeScript types of the fabric-desktop/1 contract, written by hand from
// schemas/common.schema.json, owner-channel.schema.json and agent-channel.schema.json.
// The schemas are the contract; the tests check that the vocabularies below equal them.
// A nullable member is `T | null` and always present: every property is required.
// String formats (ids, timestamps, addresses) are plain strings here; the schema checks them.

// ---- Versions and limits ----

export const PROTOCOL = "fabric-desktop/1";
export type Protocol = typeof PROTOCOL;
export const SCHEMA_VERSION = 3;
export const AUTHORIZATION_PROFILE = "fabric-broker-authorization/1";

export interface Limits {
  readonly max_frame_bytes: PositiveCount;
  readonly subscriber_queue_notices: PositiveCount;
  readonly open_requests_total: PositiveCount;
  readonly open_requests_per_agent: PositiveCount;
  readonly requests_per_minute_per_agent: PositiveCount;
  readonly open_approval_cards_per_agent: PositiveCount;
  readonly session_idle_timeout_seconds: PositiveCount;
  readonly authorization_lifetime_seconds: PositiveCount;
  readonly approval_expiry_seconds: PositiveCount;
}

// The values frozen at CD-G0 (data/limits.json). The broker reports its own in hello.
export const LIMITS = {
  max_frame_bytes: 1048576,
  subscriber_queue_notices: 256,
  open_requests_total: 100,
  open_requests_per_agent: 25,
  requests_per_minute_per_agent: 15,
  open_approval_cards_per_agent: 5,
  session_idle_timeout_seconds: 1800,
  authorization_lifetime_seconds: 60,
  approval_expiry_seconds: 86400,
} as const satisfies Limits;

// ---- Scalars (common.schema.json) ----

export type Id = string;
export type Timestamp = string; // UTC with milliseconds: 2026-11-20T16:00:00.000Z
export type Digest = `sha256:${string}`;
export type Count = number; // 0 ..= 9007199254740991
export type PositiveCount = number; // 1 ..= 9007199254740991
export type Label = string;
export type Explanation = string;
export type NameToken = string;
export type Email = string;
export type Domain = string;
export type IanaZone = string;
export type CalendarDate = string;
export type LocalTime = string;
export type OffsetDateTime = string;
export type ProviderRef = string;
export type PublicKey = `ed25519:${string}`; // ed25519: and 64 lowercase hexadecimal digits
export type Nonce = string; // 64 lowercase hexadecimal digits
export type Signature = string; // 128 lowercase hexadecimal digits
export type PairingCode = string; // XXXX-XXXX-XXXX, hexadecimal digits in capitals
export type ConfigurationText = string; // 1..=8192 code points

// ---- Closed vocabularies ----

export const WEEKDAYS = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"] as const;
export type Weekday = (typeof WEEKDAYS)[number];

export const EVENT_FIELDS = ["time", "title", "location", "description", "attendees", "organizer", "own_response"] as const;
export type EventField = (typeof EVENT_FIELDS)[number];

export const SEND_UPDATES = ["all", "external_only", "none"] as const;
export type SendUpdates = (typeof SEND_UPDATES)[number];

export const CALENDAR_ACCESS = [
  "owner", "writer", "writer_without_private_access", "reader", "free_busy_reader",
] as const;
export type CalendarAccess = (typeof CALENDAR_ACCESS)[number];

export const TOOL_NAMES = [
  "fabric_calendar_list", "fabric_calendar_availability", "fabric_calendar_read", "fabric_calendar_propose_create",
  "fabric_calendar_propose_update", "fabric_calendar_propose_invitation", "fabric_calendar_propose_response",
  "fabric_calendar_propose_delete", "fabric_request_status", "fabric_request_result",
] as const;
export type ToolName = (typeof TOOL_NAMES)[number];

// The tools that create a request; their call is an action. Only the status tool is not.
export const ACTION_TOOLS = [
  "fabric_calendar_list", "fabric_calendar_availability", "fabric_calendar_read", "fabric_calendar_propose_create",
  "fabric_calendar_propose_update", "fabric_calendar_propose_invitation", "fabric_calendar_propose_response",
  "fabric_calendar_propose_delete", "fabric_request_result",
] as const;
export type ActionTool = (typeof ACTION_TOOLS)[number];

export const REQUEST_STATES = [
  "received", "validating", "proving_classification", "allowed", "pending_approval", "blocked", "undecided",
  "approved", "denied", "expired", "cancelled", "stale", "proving_fresh_permit", "queued", "dispatching", "applied",
  "reconciled", "retry_wait", "outcome_unknown", "terminal_failure", "reconciling", "absence_confirmed",
  "needs_owner_resolution", "resolved_as_applied", "resolved_as_absent", "completed",
] as const;
export type RequestState = (typeof REQUEST_STATES)[number];

// The states whose status carries nothing but the state.
export const PLAIN_REQUEST_STATES = [
  "received", "validating", "proving_classification", "allowed", "undecided", "approved", "denied", "expired",
  "cancelled", "stale", "proving_fresh_permit", "queued", "dispatching", "applied", "reconciled", "retry_wait",
  "outcome_unknown", "reconciling", "absence_confirmed", "needs_owner_resolution", "resolved_as_applied",
  "resolved_as_absent", "completed",
] as const;
export type PlainRequestState = (typeof PLAIN_REQUEST_STATES)[number];

export const FAILURE_CODES = [
  "invalid_arguments", "revision_conflict", "provider_rejected", "retry_budget_exhausted",
  "account_authentication_required",
] as const;
export type FailureCode = (typeof FAILURE_CODES)[number];

export const BUSY_LIMITS = [
  "open_requests_total", "open_requests_per_agent", "requests_per_minute_per_agent", "open_approval_cards_per_agent",
  "sessions_per_minute_per_agent",
] as const;
export type BusyLimit = (typeof BUSY_LIMITS)[number];

export const UNDECIDED_REASONS = ["prover_unavailable", "out_of_time", "failed", "provider_unreachable"] as const;
export type UndecidedReason = (typeof UNDECIDED_REASONS)[number];

export const OWNER_RESOLUTION_REASONS = [
  "applied_differs", "event_changed", "event_gone", "reads_failed", "account_disconnected",
] as const;
export type OwnerResolutionReason = (typeof OWNER_RESOLUTION_REASONS)[number];

export const ERROR_CODES = [
  "malformed_frame", "unsupported_protocol", "unsupported_schema_version", "unknown_method", "invalid_params",
  "not_found", "internal", "conflict", "command_reused", "stale_request", "expansion_not_acknowledged",
  "snapshot_required", "store_unavailable", "busy", "not_permitted", "broker_not_running",
  "prover_unavailable",
] as const;
export type ErrorCode = (typeof ERROR_CODES)[number];

export const OWNER_METHODS = [
  "hello", "get_snapshot", "connect_google", "save_profile", "save_preferences", "save_group", "decide_request",
  "pause_agent", "revoke_grant", "get_request_details", "get_proof", "subscribe", "disconnect_google",
  "select_calendars", "refresh_calendars", "resolve_outcome", "add_agent", "approve_agent", "remove_agent",
  "get_agent_configuration", "list_ended_requests",
] as const;
export type OwnerMethod = (typeof OWNER_METHODS)[number];

// Owner methods that change broker state. Their frame id is the command id.
export const OWNER_COMMANDS = [
  "connect_google", "save_profile", "save_preferences", "save_group", "decide_request", "pause_agent",
  "revoke_grant", "disconnect_google", "select_calendars", "refresh_calendars", "resolve_outcome", "add_agent",
  "approve_agent", "remove_agent",
] as const;
export type OwnerCommand = (typeof OWNER_COMMANDS)[number];

export const MODES = ["allow", "ask", "block"] as const;
export type Mode = (typeof MODES)[number];

export const POLICY_ROWS = [
  "list_and_availability", "read_event_details", "create_in_agent_drafts", "create_in_existing_calendar",
  "edit_fabric_created_event", "edit_preexisting_event", "send_or_change_invitations", "respond_to_invitation",
  "delete_event", "delete_calendar_or_change_sharing",
] as const;
export type PolicyRow = (typeof POLICY_ROWS)[number];

export const DURATION_KINDS = ["one_request", "session", "until_revoked"] as const;
export type DurationKind = (typeof DURATION_KINDS)[number];

export const GRANT_END_REASONS = [
  "consumed", "revoked_by_owner", "account_disconnected", "authority_reduced", "agent_removed",
  "session_disconnected", "session_idle_timeout", "app_quit", "broker_restart", "result_expired",
] as const;
export type GrantEndReason = (typeof GRANT_END_REASONS)[number];

export const ACCOUNT_STATES = [
  "awaiting_consent", "connected", "consent_denied", "partial_grant", "cancelled", "account_mismatch",
  "reauthentication_required", "disconnected",
] as const;
export type AccountState = (typeof ACCOUNT_STATES)[number];

export const RESPONSE_STATUSES = ["needs_action", "accepted", "declined", "tentative"] as const;
export type ResponseStatus = (typeof RESPONSE_STATUSES)[number];

// ---- Shared records (common.schema.json) ----

export interface EventTime {
  readonly date_time: OffsetDateTime;
  readonly zone: IanaZone;
}
// Half-open: start inclusive, end exclusive.
export interface Window {
  readonly start: Timestamp;
  readonly end: Timestamp;
}
export interface EventRef {
  readonly event_id: ProviderRef;
  readonly revision: ProviderRef;
}
export interface OccurrenceRef {
  readonly instance_id: ProviderRef;
  readonly original_start_time: EventTime;
  readonly revision: ProviderRef;
}
export interface CalendarRef {
  readonly account_id: Id;
  readonly calendar_id: Id;
}

// ---- Errors ----

interface ErrorOf<C extends ErrorCode, R extends boolean, V extends string, D> {
  readonly code: C;
  readonly message: Explanation;
  readonly retryable: R;
  readonly recovery: V;
  readonly correlation_id: Id | null;
  readonly detail: D;
}
export type ErrorMalformedFrame = ErrorOf<"malformed_frame", false, "report_problem", null>;
export type ErrorUnsupportedProtocol = ErrorOf<"unsupported_protocol", false, "update_app",
  { readonly received: string | null; readonly supported: readonly [Protocol] }>;
export type ErrorUnsupportedSchemaVersion = ErrorOf<"unsupported_schema_version", false, "update_app",
  { readonly received: Count; readonly supported: PositiveCount }>;
export type ErrorUnknownMethod = ErrorOf<"unknown_method", false, "report_problem", null>;
export type ErrorInvalidParams = ErrorOf<"invalid_params", false, "report_problem", { readonly pointer: string }>;
export type ErrorNotFound = ErrorOf<"not_found", false, "none", null>;
export type ErrorInternal = ErrorOf<"internal", false, "report_problem", null>;
export type ErrorConflict = ErrorOf<"conflict", false, "refresh_snapshot", { readonly current_revision: Count }>;
export type ErrorCommandReused = ErrorOf<"command_reused", false, "report_problem", null>;
export type ErrorStaleRequest = ErrorOf<"stale_request", false, "review_request_again", null>;
export type ErrorExpansionNotAcknowledged = ErrorOf<"expansion_not_acknowledged", false, "acknowledge_expansion",
  { readonly expansion: Expansion; readonly expansion_digest: Digest }>;
export type ErrorSnapshotRequired = ErrorOf<"snapshot_required", false, "refresh_snapshot", null>;
export type ErrorStoreUnavailable = ErrorOf<"store_unavailable", true, "unlock_store",
  { readonly reason: "locked" | "keychain_refused" }>;
export type ErrorBusy = ErrorOf<"busy", true, "retry_later",
  { readonly limit: BusyLimit; readonly maximum: PositiveCount; readonly retry_after_ms: Count }>;
export type ErrorNotPermitted = ErrorOf<"not_permitted", false, "none", null>;
export type ErrorBrokerNotRunning = ErrorOf<"broker_not_running", true, "start_fabric", null>;
export type ErrorProverUnavailable = ErrorOf<"prover_unavailable", true, "retry_later", null>;

export type OwnerError =
  | ErrorMalformedFrame | ErrorUnsupportedProtocol | ErrorUnsupportedSchemaVersion | ErrorUnknownMethod
  | ErrorInvalidParams | ErrorNotFound | ErrorInternal | ErrorConflict | ErrorCommandReused | ErrorStaleRequest
  | ErrorExpansionNotAcknowledged | ErrorSnapshotRequired | ErrorStoreUnavailable;
export type AgentError =
  | ErrorMalformedFrame | ErrorUnsupportedProtocol | ErrorUnsupportedSchemaVersion | ErrorUnknownMethod
  | ErrorInvalidParams | ErrorNotFound | ErrorInternal | ErrorBusy | ErrorNotPermitted | ErrorBrokerNotRunning
  | ErrorProverUnavailable;

// ---- Profiles (owner-channel.schema.json) ----

// delete_event always asks and delete_calendar_or_change_sharing is always blocked.
export interface ProfileRows {
  readonly list_and_availability: Mode;
  readonly read_event_details: Mode;
  readonly create_in_agent_drafts: Mode;
  readonly create_in_existing_calendar: Mode;
  readonly edit_fabric_created_event: Mode;
  readonly edit_preexisting_event: Mode;
  readonly send_or_change_invitations: Mode;
  readonly respond_to_invitation: Mode;
  readonly delete_event: "ask";
  readonly delete_calendar_or_change_sharing: "block";
}

export type CustomProfileId = `custom-${string}`;

interface BuiltInProfile {
  readonly kind: "built_in";
  readonly profile_id: string;
  readonly name: string;
  readonly version: 1;
  readonly rows: ProfileRows;
}

export const PROFILE_CAREFUL = {
  kind: "built_in",
  profile_id: "careful",
  name: "Careful",
  version: 1,
  rows: {
    list_and_availability: "allow",
    read_event_details: "ask",
    create_in_agent_drafts: "allow",
    create_in_existing_calendar: "ask",
    edit_fabric_created_event: "ask",
    edit_preexisting_event: "ask",
    send_or_change_invitations: "ask",
    respond_to_invitation: "ask",
    delete_event: "ask",
    delete_calendar_or_change_sharing: "block",
  },
} as const satisfies BuiltInProfile;

export const PROFILE_FLEXIBLE = {
  kind: "built_in",
  profile_id: "flexible",
  name: "Flexible",
  version: 1,
  rows: {
    list_and_availability: "allow",
    read_event_details: "allow",
    create_in_agent_drafts: "allow",
    create_in_existing_calendar: "allow",
    edit_fabric_created_event: "allow",
    edit_preexisting_event: "ask",
    send_or_change_invitations: "allow",
    respond_to_invitation: "ask",
    delete_event: "ask",
    delete_calendar_or_change_sharing: "block",
  },
} as const satisfies BuiltInProfile;

export type ProfileCareful = typeof PROFILE_CAREFUL;
export type ProfileFlexible = typeof PROFILE_FLEXIBLE;
export interface ProfileCustom {
  readonly kind: "custom";
  readonly profile_id: CustomProfileId;
  readonly name: Label;
  readonly version: PositiveCount;
  readonly rows: ProfileRows;
}
export type Profile = ProfileCareful | ProfileFlexible | ProfileCustom;

// A null id creates a profile; an id saves a new version of that one.
export interface CustomProfileDraft {
  readonly profile_id: CustomProfileId | null;
  readonly name: Label;
  readonly rows: ProfileRows;
}
export interface ExpansionItem {
  readonly row: PolicyRow;
  readonly from: Mode;
  readonly to: Mode;
}
export type Expansion = readonly ExpansionItem[]; // 1..=10 items, unique
export interface Profiles {
  readonly revision: Count;
  readonly active_profile_id: Id;
  readonly items: readonly Profile[]; // at least 2
}

// ---- Recipient sets, scopes, grants ----

export type RecipientSetSource =
  | { readonly kind: "local" }
  | { readonly kind: "google_people"; readonly account_id: Id; readonly imported_at: Timestamp };
export interface RecipientSetDefinition {
  readonly name: Label;
  readonly members: readonly Email[];
  readonly domains: readonly Domain[];
}
export interface RecipientSet {
  readonly recipient_set_id: Id;
  readonly revision: PositiveCount;
  readonly source: RecipientSetSource;
  readonly definition: RecipientSetDefinition;
}
// No rule covers everyone.
export type RecipientRule =
  | { readonly kind: "exact"; readonly addresses: readonly Email[] }
  | { readonly kind: "recipient_sets"; readonly recipient_set_ids: readonly Id[] };

// A member that does not apply to the tools is null; one that applies and is null covers nothing.
// A scope that names no calendar covers no calendar: the scope of a calendar list grant.
export interface Scope {
  readonly calendars: readonly CalendarRef[]; // 0..=200, unique
  readonly window: Window | null;
  readonly fields: readonly EventField[] | null;
  readonly recipients: RecipientRule | null;
  readonly send_updates: SendUpdates | null;
}
export interface Duration {
  readonly kind: DurationKind;
}
export type GrantTerm =
  | { readonly kind: "one_request"; readonly action_digest: Digest }
  | { readonly kind: "session"; readonly session_id: Id }
  | { readonly kind: "until_revoked"; readonly expires_at: null };
export interface GrantEnd {
  readonly at: Timestamp;
  readonly reason: GrantEndReason;
}
// What created a grant: the approval of a request, or the approval of the agent's enrollment,
// whose `enrollment_id` is the agent's id.
export type GrantOrigin =
  | { readonly request_id: Id; readonly approval_digest: Digest }
  | { readonly enrollment_id: Id };
export interface Grant {
  readonly grant_id: Id;
  readonly revision: PositiveCount;
  readonly agent_id: Id;
  readonly purpose: Task;
  readonly tools: readonly ToolName[];
  readonly scope: Scope;
  readonly term: GrantTerm;
  readonly origin: GrantOrigin;
  readonly policy: { readonly profile_id: Id; readonly version: PositiveCount };
  readonly revocation_epoch: Count;
  readonly created_at: Timestamp;
  readonly ended: GrantEnd | null;
}

// ---- Agents, accounts, health, preferences ----

export type Destination =
  | { readonly kind: "cloud"; readonly processor: Label }
  | { readonly kind: "local"; readonly endpoint: Label };
export interface Session {
  readonly session_id: Id;
  readonly started_at: Timestamp;
  readonly last_active_at: Timestamp;
}
export const AGENT_ENROLLMENTS = ["pending", "approved", "removed"] as const;
export type AgentEnrollment = (typeof AGENT_ENROLLMENTS)[number];
// An agent the owner added. `pairing_code` is the code of the key the agent proved, and
// null until it has proved one. `added_at` is null only in an answer that a broker stored
// under schema version 2 and gives again.
export interface Agent {
  readonly agent_id: Id;
  readonly revision: PositiveCount;
  readonly registered_name: Label;
  readonly destination: Destination;
  readonly client: AgentClient | null; // null only for an agent the owner did not add
  readonly enrollment: AgentEnrollment;
  readonly pairing_code: PairingCode | null;
  readonly added_at: Timestamp | null;
  readonly key_proven_at: Timestamp | null;
  readonly approved_at: Timestamp | null;
  readonly removed_at: Timestamp | null;
  readonly paused: boolean;
  readonly session: Session | null;
  readonly last_tool_call_at: Timestamp | null;
}
export interface Calendar {
  readonly calendar_id: Id;
  readonly name: Label;
  readonly access: CalendarAccess;
  readonly selected: boolean;
  readonly agent_drafts: boolean;
}
export interface Account {
  readonly account_id: Id;
  readonly revision: PositiveCount;
  readonly state: AccountState;
  readonly email: Email | null;
  readonly calendars: readonly Calendar[];
}
export const CONNECT_PROBLEMS = [
  "timed_out", "superseded", "browser_not_opened", "google_error", "client_rejected", "network",
  "identity_check_failed", "unexpected_scope", "no_refresh_token", "token_store", "already_an_account",
] as const;
export type ConnectProblem = (typeof CONNECT_PROBLEMS)[number];
export const LIST_PROBLEMS = ["too_many_calendars", "too_large", "rejected", "unreachable"] as const;
export type ListProblem = (typeof LIST_PROBLEMS)[number];
export const REVOCATIONS = ["not_requested", "pending", "revoked", "not_confirmed"] as const;
export type Revocation = (typeof REVOCATIONS)[number];
// What the broker knows about an account's connection to Google beyond its state. Owner only.
export interface ProviderAccount {
  readonly account_id: Id;
  readonly last_connect_problem: ConnectProblem | null;
  readonly last_list_problem: ListProblem | null; // null after a list that succeeded
  readonly missing_scope: string | null; // 1..=200 characters, Google's name of the scope
  readonly refresh_token_ends_at: Timestamp | null;
  readonly revocation: Revocation;
}
export const PROVER_PROBLEMS = [
  "not_configured", "not_installed", "failed_will_retry", "failed", "wrong_rules",
] as const;
export type ProverProblem = (typeof PROVER_PROBLEMS)[number];
export const AGENT_CHANNEL_PROBLEMS = [
  "path_too_long", "run_dir_unsafe", "run_dir_owner", "stale_not_socket", "socket_mode", "bind_failed",
] as const;
export type AgentChannelProblem = (typeof AGENT_CHANNEL_PROBLEMS)[number];
export const AGENT_CHANNEL_STATES = ["listening", "not_listening"] as const;
export type AgentChannelState = (typeof AGENT_CHANNEL_STATES)[number];
// `prover_problem` is null while the prover is ready or starting. `agent_channel_problem` is
// null while the agent channel is listening.
export interface Health {
  readonly prover: "ready" | "starting" | "unavailable";
  readonly prover_problem: ProverProblem | null;
  readonly store: "ready" | "locked" | "keychain_refused";
  readonly agent_channel: AgentChannelState;
  readonly agent_channel_problem: AgentChannelProblem | null;
}
export interface TimeRange {
  readonly days: readonly Weekday[]; // 1..=7, unique
  readonly start: LocalTime;
  readonly end: LocalTime;
}
// Null or an empty list means not configured.
export interface PreferencesSettings {
  readonly zone: IanaZone | null;
  readonly workday: TimeRange | null;
  readonly protected_times: readonly { readonly enabled: boolean; readonly range: TimeRange }[];
  readonly uninterrupted: {
    readonly enabled: boolean;
    readonly minimum_minutes: number; // 1..=1440
    readonly part_of_day: "any" | "morning" | "afternoon";
  } | null;
  readonly buffer: { readonly enabled: boolean; readonly minutes: number } | null; // minutes 1..=240
  readonly warning_calendars: readonly CalendarRef[];
}
export interface Preferences {
  readonly revision: Count;
  readonly settings: PreferencesSettings;
}

// ---- Requests, approvals, proofs ----

export interface Ruling {
  readonly rule: NameToken;
  readonly explanation: Explanation;
  readonly proof_id: Id;
}
export interface Failure {
  readonly code: FailureCode;
  readonly explanation: Explanation;
}
// Three states carry more than the state; a blocked request always has its ruling.
export type LifecycleStatus =
  | { readonly state: "pending_approval"; readonly approval_expires_at: Timestamp }
  | { readonly state: "blocked"; readonly denial: Ruling }
  | { readonly state: "terminal_failure"; readonly failure: Failure }
  | { readonly state: PlainRequestState };
export interface Request {
  readonly request_id: Id;
  readonly revision: PositiveCount;
  readonly agent_id: Id;
  readonly tool: ToolName;
  readonly account_id: Id | null;
  readonly received_at: Timestamp;
  readonly updated_at: Timestamp;
  readonly request_digest: Digest;
  readonly status: LifecycleStatus;
}
export interface ApprovalCard {
  readonly scope: Scope;
  readonly proposed_duration: Duration;
  readonly insufficiency: Ruling;
  readonly expires_at: Timestamp;
}
export type Approval =
  | {
      readonly decision: "approve";
      readonly command_id: Id;
      readonly decided_at: Timestamp;
      readonly approval_digest: Digest;
      readonly scope: Scope;
      readonly duration: Duration;
      readonly effective_request_digest: Digest;
      readonly grant_id: Id;
    }
  | {
      readonly decision: "deny";
      readonly command_id: Id;
      readonly decided_at: Timestamp;
      readonly approval_digest: Digest;
      readonly scope: null;
      readonly duration: null;
      readonly effective_request_digest: null;
      readonly grant_id: null;
    };
// Profile fabric-broker-authorization/1: used once, expires_at = issued_at + 60 s.
export interface Authorization {
  readonly profile: typeof AUTHORIZATION_PROFILE;
  readonly authorization_id: Id;
  readonly request_id: Id;
  readonly agent_id: Id;
  readonly action_digest: Digest;
  readonly proof_id: Id;
  readonly grant_id: Id;
  readonly issued_at: Timestamp;
  readonly expires_at: Timestamp;
  readonly lifetime_seconds: 60;
  readonly consumed_at: Timestamp | null;
}
export interface ProofReport {
  readonly proof_id: Id;
  readonly request_id: Id;
  readonly purpose: "classification" | "fresh_permit";
  readonly outcome: "permit" | "need_approval" | "block" | "undecided";
  readonly failing_rule: NameToken | null;
  readonly rule_verdicts: readonly { readonly rule: NameToken; readonly verdict: "holds" | "fails" | "undecided" }[];
  readonly isabelle_version: Label;
  readonly theory_hash: Digest;
  readonly context_digest: Digest;
  readonly action_digest: Digest;
  readonly started_at: Timestamp;
  readonly elapsed_ms: Count;
}
export const SYSTEM_ACTION_KINDS = ["refresh", "reconcile", "list_calendars"] as const;
export type SystemActionKind = (typeof SYSTEM_ACTION_KINDS)[number];
// A proof of an action the broker took itself for a request. It holds no calendar content.
export interface SystemProofReport {
  readonly proof_id: Id;
  readonly action: {
    readonly kind: SystemActionKind;
    readonly account_id: Id;
    readonly calendar_id: Id | null;
    readonly event_id: ProviderRef | null;
  };
  readonly outcome: "permit" | "need_approval" | "block" | "undecided";
  readonly prover_kind: "isabelle" | "synthetic";
  readonly recorded_at: Timestamp;
}
export const RELEASE_KINDS = ["calendars", "availability", "events", "effect"] as const;
export type ReleaseKind = (typeof RELEASE_KINDS)[number];
// One release of a result to the agent, under its own proof. `request_id` is the request that
// asked for the release.
export interface ReleaseRecord {
  readonly release_id: Id;
  readonly request_id: Id;
  readonly kind: ReleaseKind;
  readonly proof_id: Id;
  readonly grant_ids: readonly Id[]; // 0..=50, unique
  readonly released_at: Timestamp;
}
export interface Attempt {
  readonly number: PositiveCount;
  readonly proof_id: Id;
  readonly started_at: Timestamp;
  readonly finished_at: Timestamp | null;
  readonly outcome: "applied" | "transient_failure" | "terminal_failure" | "outcome_unknown" | null;
}
// For the owner only; never blocks and never reaches an agent.
export interface Warning {
  readonly kind: "protected_time" | "uninterrupted_time" | "buffer";
  readonly status: "warning" | "unavailable" | "incomplete_data";
  readonly explanation: Explanation;
  readonly observed_at: Timestamp | null;
}
export const NOTIFICATIONS = ["sent", "not_requested", "unknown"] as const;
export type Notification = (typeof NOTIFICATIONS)[number];
// `private` is null when the retrieval was redacted.
export interface RequestDetails {
  readonly request: Request;
  readonly redaction: "redacted" | "full";
  readonly private: {
    readonly action: Action;
    readonly before: EventView | null;
    readonly after: EventView | null; // as Google holds it, when the applied change differs from the approval; may be null then too
    readonly warnings: readonly Warning[];
  } | null;
  readonly card: ApprovalCard | null;
  readonly approval: Approval | null;
  readonly proof_ids: readonly Id[];
  readonly authorization: Authorization | null;
  readonly attempts: readonly Attempt[];
  readonly provider_event: EventRef | null;
  readonly undecided_reason: UndecidedReason | null; // null unless the request is undecided
  readonly owner_resolution_reason: OwnerResolutionReason | null; // why the owner is asked to resolve
  readonly notification: Notification | null;
  readonly system_proofs: readonly SystemProofReport[]; // 0..=200
  readonly releases: readonly ReleaseRecord[]; // 0..=200
}

// ---- Snapshot and changes ----

export interface Snapshot {
  readonly cursor: Count;
  readonly health: Health;
  readonly profiles: Profiles;
  readonly accounts: readonly Account[];
  readonly provider_accounts: readonly ProviderAccount[];
  readonly agents: readonly Agent[];
  readonly recipient_sets: readonly RecipientSet[];
  readonly preferences: Preferences;
  readonly requests: readonly Request[];
  readonly grants: readonly Grant[];
}
export type Change =
  | { readonly kind: "account"; readonly account: Account }
  | { readonly kind: "provider_account"; readonly provider_account: ProviderAccount }
  | { readonly kind: "agent"; readonly agent: Agent }
  | { readonly kind: "profiles"; readonly profiles: Profiles }
  | { readonly kind: "recipient_set"; readonly recipient_set: RecipientSet }
  | { readonly kind: "recipient_set_deleted"; readonly recipient_set_id: Id }
  | { readonly kind: "preferences"; readonly preferences: Preferences }
  | { readonly kind: "request"; readonly request: Request }
  | { readonly kind: "grant"; readonly grant: Grant }
  | { readonly kind: "health"; readonly health: Health }
  | { readonly kind: "snapshot_required" };

// ---- Owner methods ----

export type Empty = { readonly [key: string]: never };
export interface Party {
  readonly name: Label;
  readonly build: Label;
}
export interface ParamsHello {
  readonly client: Party;
  readonly schema_version: Count;
}
export interface ResultHello {
  readonly broker: Party;
  readonly schema_version: PositiveCount;
  readonly capabilities: readonly NameToken[];
  readonly limits: Limits;
}
export interface ParamsConnectGoogle {
  readonly account_id: Id | null;
}
export interface ResultConnectGoogle {
  readonly commit_sequence: PositiveCount;
  readonly account: Account;
}
export interface ParamsSaveProfile {
  readonly expected_revision: Count;
  readonly change:
    | { readonly kind: "activate"; readonly profile_id: Id }
    | { readonly kind: "save_custom"; readonly profile: CustomProfileDraft; readonly acknowledged_expansion: Digest | null }
    | { readonly kind: "delete_custom"; readonly profile_id: CustomProfileId };
}
export interface ResultSaveProfile {
  readonly commit_sequence: PositiveCount;
  readonly profiles: Profiles;
}
export interface ParamsSavePreferences {
  readonly expected_revision: Count;
  readonly settings: PreferencesSettings;
}
export interface ResultSavePreferences {
  readonly commit_sequence: PositiveCount;
  readonly preferences: Preferences;
}
export interface ParamsSaveGroup {
  readonly expected_revision: Count;
  readonly change:
    | { readonly kind: "save"; readonly recipient_set_id: Id | null; readonly definition: RecipientSetDefinition }
    | { readonly kind: "delete"; readonly recipient_set_id: Id };
}
export interface ResultSaveGroup {
  readonly commit_sequence: PositiveCount;
  readonly recipient_set: RecipientSet | null;
}
// A null `selected` approves the scope as asked; a scope narrows it.
export type ParamsDecideRequest =
  | {
      readonly request_id: Id;
      readonly request_digest: Digest;
      readonly expected_revision: Count;
      readonly decision: "approve";
      readonly selected: Scope | null;
      readonly duration: Duration;
    }
  | {
      readonly request_id: Id;
      readonly request_digest: Digest;
      readonly expected_revision: Count;
      readonly decision: "deny";
      readonly selected: null;
      readonly duration: null;
    };
export interface ResultDecideRequest {
  readonly commit_sequence: PositiveCount;
  readonly request: Request;
  readonly approval: Approval;
}
export interface ParamsPauseAgent {
  readonly target:
    | { readonly kind: "agent"; readonly agent_id: Id; readonly expected_revision: Count }
    | { readonly kind: "all_agents" };
  readonly paused: boolean;
}
export interface ResultPauseAgent {
  readonly commit_sequence: PositiveCount;
  readonly agents: readonly Agent[];
}
export interface ParamsRevokeGrant {
  readonly grant_id: Id;
  readonly expected_revision: Count;
}
export interface ResultRevokeGrant {
  readonly commit_sequence: PositiveCount;
  readonly grant: Grant;
}
export interface ParamsGetRequestDetails {
  readonly request_id: Id;
  readonly redaction: "redacted" | "full";
}
export interface ParamsGetProof {
  readonly proof_id: Id;
}
export interface ParamsSubscribe {
  readonly cursor: Count;
}
export interface ResultSubscribe {
  readonly cursor: Count;
}
export interface ParamsDisconnectGoogle {
  readonly account_id: Id;
  readonly expected_revision: Count;
  readonly revoke_provider_token: boolean;
}
export interface ResultDisconnectGoogle {
  readonly commit_sequence: PositiveCount;
  readonly account: Account;
  readonly ended_grant_ids: readonly Id[];
}
// `selected` is the whole selection; `agent_drafts` must be in it, or is null for none.
export interface ParamsSelectCalendars {
  readonly account_id: Id;
  readonly expected_revision: Count;
  readonly selected: readonly Id[]; // 0..=250, unique
  readonly agent_drafts: Id | null;
}
export interface ResultSelectCalendars {
  readonly commit_sequence: PositiveCount;
  readonly account: Account;
}
// Lists one account's calendars again. The list arrives later, as changes of the account.
export interface ParamsRefreshCalendars {
  readonly account_id: Id;
}
export interface ResultRefreshCalendars {
  readonly commit_sequence: PositiveCount;
}
export type Resolution = "applied" | "absent";
// The owner's own statement about a request in needs_owner_resolution. Nothing is sent to Google.
export interface ParamsResolveOutcome {
  readonly request_id: Id;
  readonly expected_revision: Count;
  readonly resolution: Resolution;
}
export interface ResultResolveOutcome {
  readonly commit_sequence: PositiveCount;
  readonly request: Request;
}
export const AGENT_CLIENTS = ["claude_desktop", "claude_code", "codex_desktop"] as const;
export type AgentClient = (typeof AGENT_CLIENTS)[number];
// The broker gives the agent an id and, from the kind of client, where its data goes.
export interface ParamsAddAgent {
  readonly registered_name: Label;
  readonly client: AgentClient;
}
export interface ResultAddAgent {
  readonly commit_sequence: PositiveCount;
  readonly agent: Agent;
}
// `pairing_code` must be the code of the key the agent proved last.
export interface ParamsApproveAgent {
  readonly agent_id: Id;
  readonly expected_revision: Count;
  readonly pairing_code: PairingCode;
}
// `grant` is the calendar list grant made in the same commit.
export interface ResultApproveAgent {
  readonly commit_sequence: PositiveCount;
  readonly agent: Agent;
  readonly grant: Grant;
}
export interface ParamsRemoveAgent {
  readonly agent_id: Id;
  readonly expected_revision: Count;
}
// `agent` is the removed agent, whose record stays.
export interface ResultRemoveAgent {
  readonly commit_sequence: PositiveCount;
  readonly agent: Agent;
  readonly ended_grant_ids: readonly Id[];
}
export interface ParamsGetAgentConfiguration {
  readonly agent_id: Id;
}
// The text that starts this agent's bridge in the client the owner named. `configuration` is
// null when the broker cannot name an installed bridge.
export interface ResultGetAgentConfiguration {
  readonly configuration: ConfigurationText | null;
}
// `before` is null for the first page, or the id of the last request of the page before.
export interface ParamsListEndedRequests {
  readonly before: Id | null;
  readonly limit: number; // 1..=100
}
// `more` is true when older ended requests remain.
export interface ResultListEndedRequests {
  readonly requests: readonly Request[]; // ..=100
  readonly more: boolean;
}

// The params and result of each owner method (data/owner-methods.json).
export interface OwnerMethods {
  readonly hello: { readonly params: ParamsHello; readonly result: ResultHello };
  readonly get_snapshot: { readonly params: Empty; readonly result: Snapshot };
  readonly connect_google: { readonly params: ParamsConnectGoogle; readonly result: ResultConnectGoogle };
  readonly save_profile: { readonly params: ParamsSaveProfile; readonly result: ResultSaveProfile };
  readonly save_preferences: { readonly params: ParamsSavePreferences; readonly result: ResultSavePreferences };
  readonly save_group: { readonly params: ParamsSaveGroup; readonly result: ResultSaveGroup };
  readonly decide_request: { readonly params: ParamsDecideRequest; readonly result: ResultDecideRequest };
  readonly pause_agent: { readonly params: ParamsPauseAgent; readonly result: ResultPauseAgent };
  readonly revoke_grant: { readonly params: ParamsRevokeGrant; readonly result: ResultRevokeGrant };
  readonly get_request_details: { readonly params: ParamsGetRequestDetails; readonly result: RequestDetails };
  readonly get_proof: { readonly params: ParamsGetProof; readonly result: ProofReport };
  readonly subscribe: { readonly params: ParamsSubscribe; readonly result: ResultSubscribe };
  readonly disconnect_google: { readonly params: ParamsDisconnectGoogle; readonly result: ResultDisconnectGoogle };
  readonly select_calendars: { readonly params: ParamsSelectCalendars; readonly result: ResultSelectCalendars };
  readonly refresh_calendars: { readonly params: ParamsRefreshCalendars; readonly result: ResultRefreshCalendars };
  readonly resolve_outcome: { readonly params: ParamsResolveOutcome; readonly result: ResultResolveOutcome };
  readonly add_agent: { readonly params: ParamsAddAgent; readonly result: ResultAddAgent };
  readonly approve_agent: { readonly params: ParamsApproveAgent; readonly result: ResultApproveAgent };
  readonly remove_agent: { readonly params: ParamsRemoveAgent; readonly result: ResultRemoveAgent };
  readonly get_agent_configuration: {
    readonly params: ParamsGetAgentConfiguration;
    readonly result: ResultGetAgentConfiguration;
  };
  readonly list_ended_requests: { readonly params: ParamsListEndedRequests; readonly result: ResultListEndedRequests };
}

// ---- Owner frames ----

// For a command the frame id is the command id.
export type OwnerRequest = {
  readonly [M in OwnerMethod]: {
    readonly protocol: Protocol;
    readonly id: Id;
    readonly method: M;
    readonly params: OwnerMethods[M]["params"];
  };
}[OwnerMethod];

// id and method are null when the broker could not read the frame that far.
export interface OwnerErrorResponse {
  readonly protocol: Protocol;
  readonly id: Id | null;
  readonly method: string | null;
  readonly error: OwnerError;
}
export type OwnerResponse =
  | {
      readonly [M in OwnerMethod]: {
        readonly protocol: Protocol;
        readonly id: Id;
        readonly method: M;
        readonly result: OwnerMethods[M]["result"];
      };
    }[OwnerMethod]
  | OwnerErrorResponse;

// Sequences rise by one; a gap means a fresh snapshot is needed.
export interface OwnerNotice {
  readonly protocol: Protocol;
  readonly sequence: PositiveCount;
  readonly change: Change;
}

// ---- Agent tools (agent-channel.schema.json) ----

export type Task = string; // the agent's own untrusted words, at most 1000 code points
export interface NewEvent {
  readonly title: string;
  readonly start: EventTime;
  readonly end: EventTime;
  readonly description: string | null;
  readonly location: string | null;
}
// A null leaves the field as it is; an empty string clears a description or a location.
export interface EventChanges {
  readonly title: string | null;
  readonly start: EventTime | null;
  readonly end: EventTime | null;
  readonly description: string | null;
  readonly location: string | null;
}
export type ResponseTarget =
  | { readonly kind: "event"; readonly event: EventRef }
  | { readonly kind: "occurrence"; readonly occurrence: OccurrenceRef };

export interface ArgumentsCalendarAvailability {
  readonly account_id: Id;
  readonly calendar_ids: readonly Id[];
  readonly window: Window;
  readonly task: Task;
}
export interface ArgumentsCalendarRead {
  readonly account_id: Id;
  readonly calendar_ids: readonly Id[];
  readonly window: Window;
  readonly fields: readonly EventField[];
  readonly task: Task;
}
export interface ArgumentsProposeCreate {
  readonly account_id: Id;
  readonly calendar_id: Id;
  readonly event: NewEvent;
  readonly task: Task;
}
export interface ArgumentsProposeUpdate {
  readonly account_id: Id;
  readonly calendar_id: Id;
  readonly event: EventRef;
  readonly changes: EventChanges;
  readonly send_updates: SendUpdates;
  readonly task: Task;
}
export interface ArgumentsProposeInvitation {
  readonly account_id: Id;
  readonly calendar_id: Id;
  readonly event: EventRef;
  readonly attendees: readonly Email[]; // the complete list after the change
  readonly send_updates: SendUpdates;
  readonly task: Task;
}
export interface ArgumentsProposeResponse {
  readonly account_id: Id;
  readonly calendar_id: Id;
  readonly target: ResponseTarget;
  readonly response: "accepted" | "declined";
  readonly send_updates: SendUpdates;
  readonly task: Task;
}
export interface ArgumentsProposeDelete {
  readonly account_id: Id;
  readonly calendar_id: Id;
  readonly event: EventRef;
  readonly send_updates: SendUpdates;
  readonly task: Task;
}
export interface ArgumentsRequestId {
  readonly request_id: Id;
}

// The arguments of each tool (data/tool-catalog.json).
export interface ToolArguments {
  readonly fabric_calendar_list: Empty;
  readonly fabric_calendar_availability: ArgumentsCalendarAvailability;
  readonly fabric_calendar_read: ArgumentsCalendarRead;
  readonly fabric_calendar_propose_create: ArgumentsProposeCreate;
  readonly fabric_calendar_propose_update: ArgumentsProposeUpdate;
  readonly fabric_calendar_propose_invitation: ArgumentsProposeInvitation;
  readonly fabric_calendar_propose_response: ArgumentsProposeResponse;
  readonly fabric_calendar_propose_delete: ArgumentsProposeDelete;
  readonly fabric_request_status: ArgumentsRequestId;
  readonly fabric_request_result: ArgumentsRequestId;
}

// The subject of the action digest.
export type Action = {
  readonly [T in ActionTool]: { readonly tool: T; readonly arguments: ToolArguments[T] };
}[ActionTool];

export interface Attendee {
  readonly email: Email;
  readonly response: ResponseStatus;
}
// All-day events are read, never created or edited, in version 1.
export type EventSpan =
  | { readonly kind: "timed"; readonly start: EventTime; readonly end: EventTime }
  | { readonly kind: "all_day"; readonly start_date: CalendarDate; readonly end_date: CalendarDate };
// A content field is null when the grant does not cover it.
export interface EventView {
  readonly event_id: ProviderRef;
  readonly revision: ProviderRef;
  readonly calendar_id: Id;
  readonly fabric_created: boolean;
  readonly occurrence: { readonly instance_id: ProviderRef; readonly original_start_time: EventTime } | null;
  readonly time: EventSpan | null;
  readonly title: string | null;
  readonly location: string | null;
  readonly description: string | null;
  readonly attendees: readonly Attendee[] | null;
  readonly organizer: Email | null;
  readonly own_response: ResponseStatus | null;
}

export interface ResultCalendars {
  readonly kind: "calendars";
  readonly accounts: readonly {
    readonly account_id: Id;
    readonly calendars: readonly {
      readonly calendar_id: Id;
      readonly name: Label;
      readonly access: CalendarAccess;
      readonly agent_drafts: boolean;
    }[];
  }[];
}
export interface ResultAvailability {
  readonly kind: "availability";
  readonly observed_at: Timestamp;
  readonly busy: readonly { readonly calendar_id: Id; readonly start: Timestamp; readonly end: Timestamp }[];
}
export interface ResultEvents {
  readonly kind: "events";
  readonly observed_at: Timestamp;
  readonly events: readonly EventView[];
}
// `event` is null after a deletion.
export interface ResultEffect {
  readonly kind: "effect";
  readonly event: EventRef | null;
}
export type ToolResult = ResultCalendars | ResultAvailability | ResultEvents | ResultEffect;

export interface AgentDenial {
  readonly rule: NameToken;
  readonly explanation: Explanation;
}
export type AgentLifecycleStatus =
  | { readonly state: "pending_approval"; readonly approval_expires_at: Timestamp }
  | { readonly state: "blocked"; readonly denial: AgentDenial }
  | { readonly state: "terminal_failure"; readonly failure_code: FailureCode }
  | { readonly state: PlainRequestState };
// Never carries calendar content.
export interface RequestStatus {
  readonly request_id: Id;
  readonly updated_at: Timestamp;
  readonly status: AgentLifecycleStatus;
}
// `result` is set only when a fresh proof released it.
export interface ToolOutcome {
  readonly request: RequestStatus;
  readonly result: ToolResult | null;
}

// ---- Agent frames ----

// The two handshake methods, which come before any tool frame on a connection and are not
// tools (README section 8).
export const HANDSHAKE_METHODS = ["agent_hello", "agent_prove"] as const;
export type HandshakeMethod = (typeof HANDSHAKE_METHODS)[number];
// Every method of the agent channel that is not a tool: the handshake and the cancel message.
export const NON_TOOL_METHODS = [...HANDSHAKE_METHODS, "agent_cancel"] as const;
export type NonToolMethod = (typeof NON_TOOL_METHODS)[number];
export interface ParamsAgentHello {
  readonly schema_version: Count;
  readonly enrollment_id: Id;
  readonly public_key: PublicKey;
}
export interface ResultAgentHello {
  readonly schema_version: PositiveCount;
  readonly broker_nonce: Nonce;
}
// The Ed25519 signature over the handshake text, made with the key of the hello.
export interface ParamsAgentProve {
  readonly signature: Signature;
}
// approved: the session has started. pending: the owner has not approved yet. `practice` is
// true exactly when the broker runs with the practice calendar.
export interface ResultAgentProve {
  readonly enrollment: "approved" | "pending";
  readonly practice: boolean;
}
// The adapter no longer waits for the answer to the call whose frame id is `call_id`, on this
// connection.
export interface ParamsAgentCancel {
  readonly call_id: Id;
}
// `held` is true when a call with that id was waiting for its answer and was dropped; false
// when no such call was waiting.
export interface ResultAgentCancel {
  readonly held: boolean;
}

export type AgentRequest =
  | { readonly protocol: Protocol; readonly id: Id; readonly method: "agent_hello"; readonly params: ParamsAgentHello }
  | { readonly protocol: Protocol; readonly id: Id; readonly method: "agent_prove"; readonly params: ParamsAgentProve }
  | { readonly protocol: Protocol; readonly id: Id; readonly method: "agent_cancel"; readonly params: ParamsAgentCancel }
  | {
      readonly [T in ToolName]: {
        readonly protocol: Protocol;
        readonly id: Id;
        readonly method: T;
        readonly params: ToolArguments[T];
      };
    }[ToolName];

export type AgentResponse =
  | { readonly protocol: Protocol; readonly id: Id; readonly method: "agent_hello"; readonly result: ResultAgentHello }
  | { readonly protocol: Protocol; readonly id: Id; readonly method: "agent_prove"; readonly result: ResultAgentProve }
  | { readonly protocol: Protocol; readonly id: Id; readonly method: "agent_cancel"; readonly result: ResultAgentCancel }
  | { readonly protocol: Protocol; readonly id: Id; readonly method: ToolName; readonly result: ToolOutcome }
  | { readonly protocol: Protocol; readonly id: Id | null; readonly method: string | null; readonly error: AgentError };
