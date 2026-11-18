//! The records the store keeps, as the rest of the broker reads and writes them.

use fabric_desktop_contracts::common::ToolName;
use fabric_desktop_contracts::owner::{
    Approval, ApprovalCard, Attempt, Authorization, Destination, ProofReport, Request, Ruling,
    Scope,
};
use fabric_desktop_contracts::scalar::{Digest, Id, Label};

use crate::key::KeyKind;
use crate::mutation::SessionEndReason;

/// Which prover produced a result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProverKind {
    /// The Isabelle rule prover.
    Isabelle,
    /// The synthetic classifier of debug builds; it never permits.
    Synthetic,
}

/// The broker's cause of a move to `undecided`, stored on that move (`Mutation::RequestMoved`).
///
/// Its texts are stored in the ledger, so a text, once released, is never renamed or reused. The
/// core maps each cause to the contract's `undecided_reason`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UndecidedCause {
    /// The prover was starting.
    ProverStarting,
    /// The prover was down, stopping or lost, or the job could not be handed to it.
    ProverNotRunning,
    /// The run has no prover that can decide: none attached, or a kind that is not Isabelle.
    NoProver,
    /// The proof's deadline passed before a decision.
    DeadlinePassed,
    /// The check reached the time limit the prover gave it.
    CheckLimitReached,
    /// A check ran and did not decide.
    CheckNotDecided,
    /// The prover's report could not be used: another theory, unbound, not fit or outside the facts.
    ReportUnusable,
    /// A block or a need for approval came without a ruling.
    NoRuling,
    /// A fresh permit named no authorizing grant.
    NoGrant,
    /// The request's re-proofs for classification are spent.
    ReproofsSpent,
    /// The prover refused the job before any check, for a reason that is not its state or time.
    JobRefused,
    /// The proof permitted, but no provider is attached.
    NoProvider,
    /// The proof permitted, but the agent is paused.
    AgentPaused,
    /// The proof permitted a tool that changes no calendar.
    ReadTool,
    /// The proof asked for approval, but the request has no scope.
    NoScope,
    /// The result had no report and stated no cause.
    NotStated,
}

impl UndecidedCause {
    /// Every cause, in declaration order.
    pub const ALL: [UndecidedCause; 16] = [
        UndecidedCause::ProverStarting,
        UndecidedCause::ProverNotRunning,
        UndecidedCause::NoProver,
        UndecidedCause::DeadlinePassed,
        UndecidedCause::CheckLimitReached,
        UndecidedCause::CheckNotDecided,
        UndecidedCause::ReportUnusable,
        UndecidedCause::NoRuling,
        UndecidedCause::NoGrant,
        UndecidedCause::ReproofsSpent,
        UndecidedCause::JobRefused,
        UndecidedCause::NoProvider,
        UndecidedCause::AgentPaused,
        UndecidedCause::ReadTool,
        UndecidedCause::NoScope,
        UndecidedCause::NotStated,
    ];

    /// The stored text, the same as serde's.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            UndecidedCause::ProverStarting => "prover_starting",
            UndecidedCause::ProverNotRunning => "prover_not_running",
            UndecidedCause::NoProver => "no_prover",
            UndecidedCause::DeadlinePassed => "deadline_passed",
            UndecidedCause::CheckLimitReached => "check_limit_reached",
            UndecidedCause::CheckNotDecided => "check_not_decided",
            UndecidedCause::ReportUnusable => "report_unusable",
            UndecidedCause::NoRuling => "no_ruling",
            UndecidedCause::NoGrant => "no_grant",
            UndecidedCause::ReproofsSpent => "reproofs_spent",
            UndecidedCause::JobRefused => "job_refused",
            UndecidedCause::NoProvider => "no_provider",
            UndecidedCause::AgentPaused => "agent_paused",
            UndecidedCause::ReadTool => "read_tool",
            UndecidedCause::NoScope => "no_scope",
            UndecidedCause::NotStated => "not_stated",
        }
    }

    /// The cause a stored text names; `None` for a text this build does not know.
    #[must_use]
    pub fn of_name(text: &str) -> Option<UndecidedCause> {
        UndecidedCause::ALL
            .into_iter()
            .find(|cause| cause.name() == text)
    }
}

/// Decodes a stored cause tolerantly: an unknown text, another JSON type or `null` is no cause, so
/// a row a newer build wrote still decodes and replays. Writing stays strict: only the enum is put in.
pub(crate) fn tolerant_cause<'de, D>(deserializer: D) -> Result<Option<UndecidedCause>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = <Option<serde_json::Value> as serde::Deserialize>::deserialize(deserializer)?;
    Ok(match value {
        Some(serde_json::Value::String(text)) => UndecidedCause::of_name(&text),
        _ => None,
    })
}

/// The versions a proof's facts depended on. Equality is the whole check.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Fence {
    /// `pause_epoch` when the facts were read.
    pub pause_epoch: u64,
    /// `revocation_epoch` when the facts were read.
    pub revocation_epoch: u64,
    /// The observed revision of the event the action names, if any.
    pub event_revision: Option<String>,
}

/// The `meta` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meta {
    /// The store's id.
    pub store_id: String,
    /// The kind of key the store was created with.
    pub key_kind: KeyKind,
    /// The schema version.
    pub schema_version: u32,
    /// The sequence of the last owner notice.
    pub notice_sequence: u64,
    /// The ledger's last row.
    pub ledger_head_seq: u64,
    /// The envelope hash of the ledger's last row.
    pub ledger_head_hash: String,
}

/// The two epochs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Counters {
    /// `pause_epoch`.
    pub pause_epoch: u64,
    /// `revocation_epoch`.
    pub revocation_epoch: u64,
}

/// Durable agent record. Activity (last call, last active) is not here: it lives in the core's memory (D35).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentRow {
    /// The agent.
    pub agent_id: Id,
    /// The record's revision.
    pub revision: u64,
    /// The name the agent was enrolled under.
    pub registered_name: Label,
    /// Where data approved for the agent goes.
    pub destination: Destination,
    /// True while the owner has paused the agent.
    pub paused: bool,
    /// When the agent was enrolled.
    pub enrolled_at_ms: i64,
}

/// One session of an agent.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SessionRow {
    /// The session.
    pub session_id: Id,
    /// Its agent.
    pub agent_id: Id,
    /// The run it started in.
    pub run_id: i64,
    /// When it started.
    pub started_at_ms: i64,
    /// When and why it ended; `None` while live.
    pub ended: Option<(i64, SessionEndReason)>,
}

/// The parts of a request fixed at its creation, and its current contract view.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RequestRow {
    /// The contract request.
    pub request: Request,
    /// The proposed action.
    pub action: fabric_desktop_contracts::agent::Action,
    /// The scope the agent asked for, if any.
    pub scope: Option<Scope>,
    /// The session the request came in.
    pub session_id: Option<Id>,
    /// The agent's call that created the request.
    pub call_id: Id,
}

/// Re-proofs per purpose over the request's life; never reset (D16).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Reproofs {
    /// Re-proofs for classification.
    pub classification: u32,
    /// Re-proofs for a fresh permit.
    pub fresh_permit: u32,
}

/// A request with everything the store keeps beside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestRecord {
    /// The request.
    pub row: RequestRow,
    /// Its approval card, if one was opened.
    pub card: Option<ApprovalCard>,
    /// When the card expires.
    pub card_expires_at_ms: Option<i64>,
    /// The owner's decision, if any.
    pub approval: Option<Approval>,
    /// The grants the request relies on (D36).
    pub relied_grant_ids: Vec<Id>,
    /// The re-proof counts.
    pub reproofs: Reproofs,
    /// When the pipeline next looks at it.
    pub due_at_ms: Option<i64>,
}

/// `prover` is set by the core from `Attachments`, never by the prover (D13).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProofRecord {
    /// The contract report.
    pub report: ProofReport,
    /// The prover that produced it.
    pub prover: ProverKind,
    /// The ruling, for block and need_approval.
    pub ruling: Option<Ruling>,
    /// The grants the prover proved authorize the action.
    pub authorizing_grants: Vec<Id>,
    /// Why no decision was reached; stored only in the encrypted store.
    pub undecided_reason: Option<String>,
    /// The versions the proof's facts depended on.
    pub fence: Fence,
    /// The theory text of the check; stored only in the encrypted store.
    pub check_theory_text: Option<String>,
}

/// Where an authorization is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorizationState {
    /// Issued and not yet used.
    Issued,
    /// Consumed by a dispatch intent.
    Consumed,
    /// Voided before use.
    Voided,
}

/// `authorization.grant_id` is `grant_ids[0]`; the store keeps the whole list (L7(c)).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuthorizationRecord {
    /// The contract authorization.
    pub authorization: Authorization,
    /// Every grant it relies on.
    pub grant_ids: Vec<Id>,
    /// The attempt it authorizes.
    pub attempt: u32,
    /// The run that issued it.
    pub run_id: i64,
    /// The versions its proof depended on.
    pub fence: Fence,
    /// Where it is in its life.
    pub state: AuthorizationState,
}

/// The one change a request makes in Google, reserved before its first attempt.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EffectRow {
    /// The effect.
    pub effect_id: Id,
    /// Its request.
    pub request_id: Id,
    /// The tool of the request's action.
    pub tool: ToolName,
    /// The account it changes.
    pub account_id: Id,
    /// The calendar it changes.
    pub calendar_id: Id,
    /// The digest of the action.
    pub action_digest: Digest,
    /// The stable event id of a create, persisted before the first call; `None` for every other tool.
    pub provider_event_id: Option<String>,
    /// When it was reserved.
    pub created_at_ms: i64,
    /// Attempts handed to the provider so far.
    pub attempts_started: u32,
    /// When the first attempt started.
    pub first_attempt_at_ms: Option<i64>,
}

/// An effect handed to Google and not yet seen applied, with the event it targets: `effect.provider_event_id` for
/// a create (always set for a create, an `apply` guard), otherwise the request action's target, `EventRef.event_id`
/// (update, invitation, delete, a response to one event) or `OccurrenceRef.instance_id` (a response to one occurrence).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenEffect {
    /// The effect.
    pub effect: EffectRow,
    /// The event it targets.
    pub event_id: String,
}

/// One attempt of an effect.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AttemptRow {
    /// The effect.
    pub effect_id: Id,
    /// The authorization the attempt consumed.
    pub authorization_id: Id,
    /// The contract attempt.
    pub attempt: Attempt,
}

/// An owner command's stored answer, returned again for a repeat of the same command.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CommandRow {
    /// The command.
    pub command_id: Id,
    /// The method.
    pub method: String,
    /// The command digest.
    pub digest: Digest,
    /// The response frame line, without its newline.
    pub answer: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// s19. 3.6: every cause keeps its stored text; serde writes the same text, and `ALL` holds each once.
    #[test]
    fn every_undecided_cause_has_its_stored_text() {
        let table = [
            (UndecidedCause::ProverStarting, "prover_starting"),
            (UndecidedCause::ProverNotRunning, "prover_not_running"),
            (UndecidedCause::NoProver, "no_prover"),
            (UndecidedCause::DeadlinePassed, "deadline_passed"),
            (UndecidedCause::CheckLimitReached, "check_limit_reached"),
            (UndecidedCause::CheckNotDecided, "check_not_decided"),
            (UndecidedCause::ReportUnusable, "report_unusable"),
            (UndecidedCause::NoRuling, "no_ruling"),
            (UndecidedCause::NoGrant, "no_grant"),
            (UndecidedCause::ReproofsSpent, "reproofs_spent"),
            (UndecidedCause::JobRefused, "job_refused"),
            (UndecidedCause::NoProvider, "no_provider"),
            (UndecidedCause::AgentPaused, "agent_paused"),
            (UndecidedCause::ReadTool, "read_tool"),
            (UndecidedCause::NoScope, "no_scope"),
            (UndecidedCause::NotStated, "not_stated"),
        ];
        assert_eq!(table.len(), UndecidedCause::ALL.len());
        for (cause, text) in table {
            assert_eq!(cause.name(), text);
            assert_eq!(UndecidedCause::of_name(text), Some(cause));
            assert_eq!(
                serde_json::to_value(cause).unwrap(),
                serde_json::json!(text)
            );
            assert!(UndecidedCause::ALL.contains(&cause));
        }
        let mut names: Vec<&str> = UndecidedCause::ALL.iter().map(|c| c.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), UndecidedCause::ALL.len());
        assert_eq!(UndecidedCause::of_name("a_cause_of_a_newer_build"), None);
    }
}
