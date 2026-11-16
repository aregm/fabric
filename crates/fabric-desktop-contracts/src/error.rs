//! The error grammar of `schemas/common.schema.json` (README sections 7 and 14).
//!
//! Every error is `{code, message, retryable, recovery, correlation_id, detail}`. The code
//! fixes `retryable`, `recovery` and the shape of `detail`, so the Rust types hold only the
//! code with its typed detail, the message and the correlation id. The owner channel and the
//! agent channel have different sets of codes: [`OwnerFault`] and [`AgentFault`].

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value, json};

use crate::common::{BusyLimit, PROTOCOL};
use crate::owner::Expansion;
use crate::scalar::{Count, Digest, Explanation, Id, PositiveCount, Text, nullable};
use crate::tagged;

/// Every error code of either channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    MalformedFrame,
    UnsupportedProtocol,
    UnsupportedSchemaVersion,
    UnknownMethod,
    InvalidParams,
    NotFound,
    Internal,
    Conflict,
    CommandReused,
    StaleRequest,
    ExpansionNotAcknowledged,
    SnapshotRequired,
    StoreUnavailable,
    Busy,
    NotPermitted,
    BrokerNotRunning,
}

/// What the interface or the agent should do about an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Recovery {
    ReportProblem,
    UpdateApp,
    None,
    RefreshSnapshot,
    ReviewRequestAgain,
    AcknowledgeExpansion,
    UnlockStore,
    RetryLater,
    StartFabric,
}

impl Recovery {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Recovery::ReportProblem => "report_problem",
            Recovery::UpdateApp => "update_app",
            Recovery::None => "none",
            Recovery::RefreshSnapshot => "refresh_snapshot",
            Recovery::ReviewRequestAgain => "review_request_again",
            Recovery::AcknowledgeExpansion => "acknowledge_expansion",
            Recovery::UnlockStore => "unlock_store",
            Recovery::RetryLater => "retry_later",
            Recovery::StartFabric => "start_fabric",
        }
    }
}

impl ErrorCode {
    pub const ALL: [ErrorCode; 16] = [
        ErrorCode::MalformedFrame,
        ErrorCode::UnsupportedProtocol,
        ErrorCode::UnsupportedSchemaVersion,
        ErrorCode::UnknownMethod,
        ErrorCode::InvalidParams,
        ErrorCode::NotFound,
        ErrorCode::Internal,
        ErrorCode::Conflict,
        ErrorCode::CommandReused,
        ErrorCode::StaleRequest,
        ErrorCode::ExpansionNotAcknowledged,
        ErrorCode::SnapshotRequired,
        ErrorCode::StoreUnavailable,
        ErrorCode::Busy,
        ErrorCode::NotPermitted,
        ErrorCode::BrokerNotRunning,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ErrorCode::MalformedFrame => "malformed_frame",
            ErrorCode::UnsupportedProtocol => "unsupported_protocol",
            ErrorCode::UnsupportedSchemaVersion => "unsupported_schema_version",
            ErrorCode::UnknownMethod => "unknown_method",
            ErrorCode::InvalidParams => "invalid_params",
            ErrorCode::NotFound => "not_found",
            ErrorCode::Internal => "internal",
            ErrorCode::Conflict => "conflict",
            ErrorCode::CommandReused => "command_reused",
            ErrorCode::StaleRequest => "stale_request",
            ErrorCode::ExpansionNotAcknowledged => "expansion_not_acknowledged",
            ErrorCode::SnapshotRequired => "snapshot_required",
            ErrorCode::StoreUnavailable => "store_unavailable",
            ErrorCode::Busy => "busy",
            ErrorCode::NotPermitted => "not_permitted",
            ErrorCode::BrokerNotRunning => "broker_not_running",
        }
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<ErrorCode> {
        ErrorCode::ALL.into_iter().find(|c| c.as_str() == name)
    }

    /// The fixed `retryable` of the code: only `store_unavailable`, `busy` and
    /// `broker_not_running` may be retried.
    #[must_use]
    pub const fn retryable(self) -> bool {
        matches!(
            self,
            ErrorCode::StoreUnavailable | ErrorCode::Busy | ErrorCode::BrokerNotRunning
        )
    }

    /// The fixed recovery action of the code.
    #[must_use]
    pub const fn recovery(self) -> Recovery {
        match self {
            ErrorCode::MalformedFrame
            | ErrorCode::UnknownMethod
            | ErrorCode::InvalidParams
            | ErrorCode::Internal
            | ErrorCode::CommandReused => Recovery::ReportProblem,
            ErrorCode::UnsupportedProtocol | ErrorCode::UnsupportedSchemaVersion => {
                Recovery::UpdateApp
            }
            ErrorCode::NotFound | ErrorCode::NotPermitted => Recovery::None,
            ErrorCode::Conflict | ErrorCode::SnapshotRequired => Recovery::RefreshSnapshot,
            ErrorCode::StaleRequest => Recovery::ReviewRequestAgain,
            ErrorCode::ExpansionNotAcknowledged => Recovery::AcknowledgeExpansion,
            ErrorCode::StoreUnavailable => Recovery::UnlockStore,
            ErrorCode::Busy => Recovery::RetryLater,
            ErrorCode::BrokerNotRunning => Recovery::StartFabric,
        }
    }
}

/// The detail of `unsupported_protocol`: what was received, at most 200 code points or
/// `None`. `supported` is always `["fabric-desktop/1"]` and is written by the type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsupportedProtocol {
    pub received: Option<Text<0, 200>>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UnsupportedProtocolWire {
    #[serde(deserialize_with = "nullable")]
    received: Option<Text<0, 200>>,
    supported: crate::scalar::List<crate::common::Protocol, 1, 1, false>,
}

impl Serialize for UnsupportedProtocol {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        json!({ "received": self.received, "supported": [PROTOCOL] }).serialize(s)
    }
}

impl<'de> Deserialize<'de> for UnsupportedProtocol {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let wire = UnsupportedProtocolWire::deserialize(d)?;
        Ok(UnsupportedProtocol {
            received: wire.received,
        })
    }
}

/// The detail of `unsupported_schema_version`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnsupportedSchemaVersion {
    pub received: Count,
    pub supported: PositiveCount,
}

/// The detail of `invalid_params`: a JSON Pointer of at most 500 code points to the first
/// value that was refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvalidParams {
    pub pointer: Text<0, 500>,
}

/// The detail of `conflict`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conflict {
    pub current_revision: Count,
}

/// The detail of `expansion_not_acknowledged`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpansionNotAcknowledged {
    pub expansion: Expansion,
    pub expansion_digest: Digest,
}

/// Why the store is unavailable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreReason {
    Locked,
    KeychainRefused,
}

/// The detail of `store_unavailable`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoreUnavailable {
    pub reason: StoreReason,
}

/// The detail of `busy`: the limit, its maximum and a wait in milliseconds (README section 8).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Busy {
    pub limit: BusyLimit,
    pub maximum: PositiveCount,
    pub retry_after_ms: Count,
}

/// A set of error codes with their typed details.
pub trait Fault: Sized {
    /// The codes of the set, in the order of its `oneOf`.
    const CODES: &'static [ErrorCode];
    fn code(&self) -> ErrorCode;
    /// The `detail` member: `null` or the typed detail.
    fn detail(&self) -> Value;
    /// Reads the detail of `code`, or fails when the code is not in this set.
    fn from_parts(code: ErrorCode, detail: Value) -> Result<Self, String>;
}

fn read<T: serde::de::DeserializeOwned>(detail: Value) -> Result<T, String> {
    crate::typed::from_value(&detail).map_err(|e| e.to_string())
}

fn null(detail: &Value) -> Result<(), String> {
    if detail.is_null() {
        Ok(())
    } else {
        Err("detail must be null".to_owned())
    }
}

/// `common#/$defs/owner_error`: the codes the broker answers on the owner channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OwnerFault {
    MalformedFrame,
    UnsupportedProtocol(UnsupportedProtocol),
    UnsupportedSchemaVersion(UnsupportedSchemaVersion),
    UnknownMethod,
    InvalidParams(InvalidParams),
    NotFound,
    Internal,
    Conflict(Conflict),
    CommandReused,
    StaleRequest,
    ExpansionNotAcknowledged(ExpansionNotAcknowledged),
    SnapshotRequired,
    StoreUnavailable(StoreUnavailable),
}

impl Fault for OwnerFault {
    const CODES: &'static [ErrorCode] = &[
        ErrorCode::MalformedFrame,
        ErrorCode::UnsupportedProtocol,
        ErrorCode::UnsupportedSchemaVersion,
        ErrorCode::UnknownMethod,
        ErrorCode::InvalidParams,
        ErrorCode::NotFound,
        ErrorCode::Internal,
        ErrorCode::Conflict,
        ErrorCode::CommandReused,
        ErrorCode::StaleRequest,
        ErrorCode::ExpansionNotAcknowledged,
        ErrorCode::SnapshotRequired,
        ErrorCode::StoreUnavailable,
    ];

    fn code(&self) -> ErrorCode {
        match self {
            OwnerFault::MalformedFrame => ErrorCode::MalformedFrame,
            OwnerFault::UnsupportedProtocol(_) => ErrorCode::UnsupportedProtocol,
            OwnerFault::UnsupportedSchemaVersion(_) => ErrorCode::UnsupportedSchemaVersion,
            OwnerFault::UnknownMethod => ErrorCode::UnknownMethod,
            OwnerFault::InvalidParams(_) => ErrorCode::InvalidParams,
            OwnerFault::NotFound => ErrorCode::NotFound,
            OwnerFault::Internal => ErrorCode::Internal,
            OwnerFault::Conflict(_) => ErrorCode::Conflict,
            OwnerFault::CommandReused => ErrorCode::CommandReused,
            OwnerFault::StaleRequest => ErrorCode::StaleRequest,
            OwnerFault::ExpansionNotAcknowledged(_) => ErrorCode::ExpansionNotAcknowledged,
            OwnerFault::SnapshotRequired => ErrorCode::SnapshotRequired,
            OwnerFault::StoreUnavailable(_) => ErrorCode::StoreUnavailable,
        }
    }

    fn detail(&self) -> Value {
        use crate::digest::to_value;
        match self {
            OwnerFault::UnsupportedProtocol(d) => to_value(d),
            OwnerFault::UnsupportedSchemaVersion(d) => to_value(d),
            OwnerFault::InvalidParams(d) => to_value(d),
            OwnerFault::Conflict(d) => to_value(d),
            OwnerFault::ExpansionNotAcknowledged(d) => to_value(d),
            OwnerFault::StoreUnavailable(d) => to_value(d),
            _ => Value::Null,
        }
    }

    fn from_parts(code: ErrorCode, detail: Value) -> Result<Self, String> {
        Ok(match code {
            ErrorCode::MalformedFrame => null(&detail).map(|()| OwnerFault::MalformedFrame)?,
            ErrorCode::UnsupportedProtocol => OwnerFault::UnsupportedProtocol(read(detail)?),
            ErrorCode::UnsupportedSchemaVersion => {
                OwnerFault::UnsupportedSchemaVersion(read(detail)?)
            }
            ErrorCode::UnknownMethod => null(&detail).map(|()| OwnerFault::UnknownMethod)?,
            ErrorCode::InvalidParams => OwnerFault::InvalidParams(read(detail)?),
            ErrorCode::NotFound => null(&detail).map(|()| OwnerFault::NotFound)?,
            ErrorCode::Internal => null(&detail).map(|()| OwnerFault::Internal)?,
            ErrorCode::Conflict => OwnerFault::Conflict(read(detail)?),
            ErrorCode::CommandReused => null(&detail).map(|()| OwnerFault::CommandReused)?,
            ErrorCode::StaleRequest => null(&detail).map(|()| OwnerFault::StaleRequest)?,
            ErrorCode::ExpansionNotAcknowledged => {
                OwnerFault::ExpansionNotAcknowledged(read(detail)?)
            }
            ErrorCode::SnapshotRequired => null(&detail).map(|()| OwnerFault::SnapshotRequired)?,
            ErrorCode::StoreUnavailable => OwnerFault::StoreUnavailable(read(detail)?),
            other => return Err(format!("{} is not an owner error", other.as_str())),
        })
    }
}

/// `common#/$defs/agent_error`: the codes an agent may receive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentFault {
    MalformedFrame,
    UnsupportedProtocol(UnsupportedProtocol),
    UnsupportedSchemaVersion(UnsupportedSchemaVersion),
    UnknownMethod,
    InvalidParams(InvalidParams),
    NotFound,
    Internal,
    Busy(Busy),
    NotPermitted,
    BrokerNotRunning,
}

impl Fault for AgentFault {
    const CODES: &'static [ErrorCode] = &[
        ErrorCode::MalformedFrame,
        ErrorCode::UnsupportedProtocol,
        ErrorCode::UnsupportedSchemaVersion,
        ErrorCode::UnknownMethod,
        ErrorCode::InvalidParams,
        ErrorCode::NotFound,
        ErrorCode::Internal,
        ErrorCode::Busy,
        ErrorCode::NotPermitted,
        ErrorCode::BrokerNotRunning,
    ];

    fn code(&self) -> ErrorCode {
        match self {
            AgentFault::MalformedFrame => ErrorCode::MalformedFrame,
            AgentFault::UnsupportedProtocol(_) => ErrorCode::UnsupportedProtocol,
            AgentFault::UnsupportedSchemaVersion(_) => ErrorCode::UnsupportedSchemaVersion,
            AgentFault::UnknownMethod => ErrorCode::UnknownMethod,
            AgentFault::InvalidParams(_) => ErrorCode::InvalidParams,
            AgentFault::NotFound => ErrorCode::NotFound,
            AgentFault::Internal => ErrorCode::Internal,
            AgentFault::Busy(_) => ErrorCode::Busy,
            AgentFault::NotPermitted => ErrorCode::NotPermitted,
            AgentFault::BrokerNotRunning => ErrorCode::BrokerNotRunning,
        }
    }

    fn detail(&self) -> Value {
        use crate::digest::to_value;
        match self {
            AgentFault::UnsupportedProtocol(d) => to_value(d),
            AgentFault::UnsupportedSchemaVersion(d) => to_value(d),
            AgentFault::InvalidParams(d) => to_value(d),
            AgentFault::Busy(d) => to_value(d),
            _ => Value::Null,
        }
    }

    fn from_parts(code: ErrorCode, detail: Value) -> Result<Self, String> {
        Ok(match code {
            ErrorCode::MalformedFrame => null(&detail).map(|()| AgentFault::MalformedFrame)?,
            ErrorCode::UnsupportedProtocol => AgentFault::UnsupportedProtocol(read(detail)?),
            ErrorCode::UnsupportedSchemaVersion => {
                AgentFault::UnsupportedSchemaVersion(read(detail)?)
            }
            ErrorCode::UnknownMethod => null(&detail).map(|()| AgentFault::UnknownMethod)?,
            ErrorCode::InvalidParams => AgentFault::InvalidParams(read(detail)?),
            ErrorCode::NotFound => null(&detail).map(|()| AgentFault::NotFound)?,
            ErrorCode::Internal => null(&detail).map(|()| AgentFault::Internal)?,
            ErrorCode::Busy => AgentFault::Busy(read(detail)?),
            ErrorCode::NotPermitted => null(&detail).map(|()| AgentFault::NotPermitted)?,
            ErrorCode::BrokerNotRunning => null(&detail).map(|()| AgentFault::BrokerNotRunning)?,
            other => return Err(format!("{} is not an agent error", other.as_str())),
        })
    }
}

/// One error: the code with its detail, a message for the person and a correlation id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractError<F> {
    pub fault: F,
    pub message: Explanation,
    pub correlation_id: Option<Id>,
}

/// An error on the owner channel.
pub type OwnerError = ContractError<OwnerFault>;
/// An error on the agent channel.
pub type AgentError = ContractError<AgentFault>;

impl<F: Fault> ContractError<F> {
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        self.fault.code()
    }
}

impl<F: Fault> Serialize for ContractError<F> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let code = self.fault.code();
        json!({
            "code": code.as_str(),
            "message": self.message,
            "retryable": code.retryable(),
            "recovery": code.recovery().as_str(),
            "correlation_id": self.correlation_id,
            "detail": self.fault.detail(),
        })
        .serialize(s)
    }
}

impl<'de, F: Fault> Deserialize<'de> for ContractError<F> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let (code, mut rest): (String, Map<String, Value>) = tagged::split(d, "code")?;
        let code = ErrorCode::from_name(&code).ok_or_else(|| tagged::unknown("code", &code))?;
        let message = tagged::read(tagged::take(&mut rest, "message")?)?;
        if tagged::take(&mut rest, "retryable")? != Value::Bool(code.retryable()) {
            return Err(de::Error::custom("retryable does not match the code"));
        }
        if tagged::take(&mut rest, "recovery")? != Value::String(code.recovery().as_str().into()) {
            return Err(de::Error::custom("recovery does not match the code"));
        }
        let correlation_id = tagged::read(tagged::take(&mut rest, "correlation_id")?)?;
        let detail = tagged::take(&mut rest, "detail")?;
        tagged::nothing(&rest)?;
        let fault = F::from_parts(code, detail).map_err(de::Error::custom)?;
        Ok(ContractError {
            fault,
            message,
            correlation_id,
        })
    }
}
