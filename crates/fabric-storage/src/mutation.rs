//! The changes a write may apply to the projection (serde: tag "op", snake_case; payload_version 1).
//!
//! `apply` formats contract timestamps from the `*_ms` fields with `crate::time::format_timestamp` (L1).

use fabric_desktop_contracts::owner::{
    Account, Approval, ApprovalCard, AttemptOutcome, CustomProfile, Grant, GrantEnd,
    LifecycleStatus, Preferences, RecipientSet,
};
use fabric_desktop_contracts::scalar::Id;

use crate::key::KeyKind;
use crate::rows::{
    AgentRow, AttemptRow, AuthorizationRecord, CommandRow, EffectRow, ProofRecord, Reproofs,
    RequestRow, SessionRow, UndecidedCause,
};

/// One projection change. A ledger row's payload is the list of a transaction's mutations.
#[allow(
    clippy::large_enum_variant,
    reason = "a mutation is built once, applied once and written once into the ledger"
)]
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Mutation {
    /// Genesis: the store's id and key kind.
    StoreCreated {
        /// The store's id.
        store_id: String,
        /// The kind of key the store was created with.
        key_kind: KeyKind,
        /// When the store was created.
        created_at_ms: i64,
    },
    /// The schema moved from one version to the next.
    SchemaMigrated {
        /// The version before.
        from: u32,
        /// The version after.
        to: u32,
    },
    /// A broker run started.
    RunStarted {
        /// The run.
        run_id: i64,
        /// When it started.
        at_ms: i64,
    },
    /// A broker run ended.
    RunEnded {
        /// The run.
        run_id: i64,
        /// When it ended.
        at_ms: i64,
        /// True for a quit at end of input.
        clean: bool,
    },
    /// An epoch rose.
    CounterRaised {
        /// The epoch.
        counter: Counter,
        /// Its new value; never lower than the old.
        to: u64,
    },
    /// An agent record was created or changed.
    AgentPut {
        /// The whole record.
        agent: AgentRow,
    },
    /// A session started.
    SessionStarted {
        /// The new session.
        session: SessionRow,
    },
    /// A session ended.
    SessionEnded {
        /// The session.
        session_id: Id,
        /// When it ended.
        at_ms: i64,
        /// Why it ended.
        reason: SessionEndReason,
    },
    /// Never touches the profiles revision.
    ProfileVersionAdded {
        /// The new version of the custom profile.
        profile: CustomProfile,
        /// When it was saved.
        at_ms: i64,
    },
    /// Once per save_profile (D20).
    ProfileStateSet {
        /// The active profile.
        active_profile_id: Id,
        /// Its version.
        active_version: u64,
        /// The profiles revision.
        revision: u64,
    },
    /// The owner's preferences were replaced.
    PreferencesPut {
        /// The whole preferences.
        preferences: Preferences,
    },
    /// A recipient set was created or changed.
    RecipientSetVersionAdded {
        /// The new version.
        set: RecipientSet,
        /// When it was saved.
        at_ms: i64,
    },
    /// A recipient set was deleted.
    RecipientSetDeleted {
        /// The set.
        recipient_set_id: Id,
        /// When it was deleted.
        at_ms: i64,
    },
    /// An account record was created or changed.
    AccountPut {
        /// The whole record.
        account: Account,
    },
    /// A request was created.
    RequestInserted {
        /// The request.
        row: RequestRow,
        /// When the pipeline next looks at it.
        due_at_ms: Option<i64>,
    },
    /// A request moved to another lifecycle status.
    RequestMoved {
        /// The request.
        request_id: Id,
        /// Its new revision.
        revision: u64,
        /// Its new status.
        status: LifecycleStatus,
        /// When it moved.
        at_ms: i64,
        /// When the pipeline next looks at it.
        due_at_ms: Option<i64>,
        /// The broker's cause of a move to `undecided`; absent on every other move and on rows
        /// written before it existed. Left out when `None`, so those rows keep their bytes, and
        /// decoded tolerantly, so an unknown text is `None`.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "crate::rows::tolerant_cause"
        )]
        undecided_cause: Option<UndecidedCause>,
    },
    /// A request's next look and re-proof counts changed without a move.
    RequestScheduled {
        /// The request.
        request_id: Id,
        /// When the pipeline next looks at it.
        due_at_ms: Option<i64>,
        /// The re-proof counts.
        reproofs: Reproofs,
    },
    /// An approval card was opened for a request.
    CardOpened {
        /// The request.
        request_id: Id,
        /// The card.
        card: ApprovalCard,
        /// When it opened.
        opened_at_ms: i64,
    },
    /// The owner's decision on a request.
    ApprovalRecorded {
        /// The request.
        request_id: Id,
        /// The decision.
        approval: Approval,
    },
    /// Replaces the request's relied grant list (D36). Each id names a grant of the request's agent.
    ReliedGrantSet {
        /// The request.
        request_id: Id,
        /// The relied grants.
        grant_ids: Vec<Id>,
    },
    /// A prover's result.
    ProofRecorded {
        /// The result.
        proof: ProofRecord,
        /// When it was recorded.
        at_ms: i64,
    },
    /// A grant was created.
    GrantInserted {
        /// The grant.
        grant: Grant,
    },
    /// A grant ended.
    GrantEnded {
        /// The grant.
        grant_id: Id,
        /// Its new revision.
        revision: u64,
        /// How it ended.
        end: GrantEnd,
    },
    /// An authorization was issued.
    AuthorizationIssued {
        /// The authorization.
        record: AuthorizationRecord,
    },
    /// An authorization was consumed by a dispatch intent.
    AuthorizationConsumed {
        /// The authorization.
        authorization_id: Id,
        /// When it was consumed.
        at_ms: i64,
    },
    /// An authorization was voided.
    AuthorizationVoided {
        /// The authorization.
        authorization_id: Id,
        /// When it was voided.
        at_ms: i64,
        /// Why.
        reason: VoidReason,
    },
    /// An effect was reserved for a request.
    EffectReserved {
        /// The effect.
        effect: EffectRow,
    },
    /// An attempt is about to be handed to the provider.
    DispatchIntent {
        /// The attempt.
        attempt: AttemptRow,
    },
    /// An attempt finished.
    AttemptFinished {
        /// The effect.
        effect_id: Id,
        /// The attempt's number.
        number: u32,
        /// When it finished.
        finished_at_ms: i64,
        /// Its outcome.
        outcome: AttemptOutcome,
    },
    /// An owner command was answered; its answer is kept for repeats.
    CommandAnswered {
        /// The command and its answer.
        row: CommandRow,
    },
}

/// The two epochs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Counter {
    /// `pause_epoch`.
    PauseEpoch,
    /// `revocation_epoch`.
    RevocationEpoch,
}

/// Why a session ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEndReason {
    /// The agent's connection closed.
    Disconnected,
    /// A new connection of the same agent replaced it.
    Replaced,
    /// No call within the idle time.
    IdleTimeout,
    /// The app quit.
    AppQuit,
    /// The broker restarted while the session was live.
    BrokerRestart,
}

impl SessionEndReason {
    /// Disconnected | Replaced -> SessionDisconnected; IdleTimeout -> SessionIdleTimeout; AppQuit; BrokerRestart.
    #[must_use]
    pub fn grant_end_reason(self) -> fabric_desktop_contracts::owner::GrantEndReason {
        use fabric_desktop_contracts::owner::GrantEndReason;
        match self {
            SessionEndReason::Disconnected | SessionEndReason::Replaced => {
                GrantEndReason::SessionDisconnected
            }
            SessionEndReason::IdleTimeout => GrantEndReason::SessionIdleTimeout,
            SessionEndReason::AppQuit => GrantEndReason::AppQuit,
            SessionEndReason::BrokerRestart => GrantEndReason::BrokerRestart,
        }
    }
}

/// Why an authorization was voided.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoidReason {
    /// Its lifetime passed.
    Expired,
    /// The broker restarted before it was consumed.
    BrokerRestart,
    /// An epoch or the event changed since its proof.
    FenceChanged,
    /// A grant it relied on is no longer valid.
    GrantInvalid,
    /// Its agent was paused.
    AgentPaused,
}
