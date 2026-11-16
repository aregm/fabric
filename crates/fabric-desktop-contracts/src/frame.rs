//! Decoding a frame by layer, and encoding a frame as one canonical line (README sections 1 and 5).
//!
//! A receiver checks a frame in this order and stops at the first failure:
//!
//! | Layer | Check | Code |
//! |---|---|---|
//! | `frame` | at most 1,048,576 bytes, no line feed, strict JSON, an object | `malformed_frame` |
//! | `protocol` | `protocol` is the string `fabric-desktop/1` | `unsupported_protocol` |
//! | `schema` | the typed deserialization of the entry succeeds | `unknown_method` for a request whose `method` is not a string naming a method of its channel, `invalid_params` otherwise |

use std::fmt;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::agent::{AgentRequest, AgentResponse};
use crate::common::{MAX_FRAME_BYTES, PROTOCOL, ToolName};
use crate::digest::to_value;
use crate::error::ErrorCode;
use crate::json::{canonical, parse_strict};
use crate::owner::{OwnerMethod, OwnerNotice, OwnerRequest, OwnerResponse};

/// The five entry schemas a frame is read against (README section 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Entry {
    OwnerRequest,
    OwnerResponse,
    OwnerNotice,
    AgentRequest,
    AgentResponse,
}

impl Entry {
    pub const ALL: [Entry; 5] = [
        Entry::OwnerRequest,
        Entry::OwnerResponse,
        Entry::OwnerNotice,
        Entry::AgentRequest,
        Entry::AgentResponse,
    ];

    /// The schema reference of the entry, as fixtures name it.
    #[must_use]
    pub const fn reference(self) -> &'static str {
        match self {
            Entry::OwnerRequest => "owner-channel.schema.json#/$defs/owner_request",
            Entry::OwnerResponse => "owner-channel.schema.json#/$defs/owner_response",
            Entry::OwnerNotice => "owner-channel.schema.json#/$defs/owner_notice",
            Entry::AgentRequest => "agent-channel.schema.json#/$defs/agent_request",
            Entry::AgentResponse => "agent-channel.schema.json#/$defs/agent_response",
        }
    }

    #[must_use]
    pub fn from_reference(reference: &str) -> Option<Entry> {
        Entry::ALL.into_iter().find(|e| e.reference() == reference)
    }
}

/// The layer at which a frame was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Layer {
    Frame,
    Protocol,
    Schema,
}

impl Layer {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Layer::Frame => "frame",
            Layer::Protocol => "protocol",
            Layer::Schema => "schema",
        }
    }
}

/// Why a frame or record was refused: the layer, the error code the broker answers, and a
/// reason for logs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub layer: Layer,
    pub code: ErrorCode,
    pub reason: String,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at the {} layer: {}",
            self.code.as_str(),
            self.layer.as_str(),
            self.reason
        )
    }
}

impl std::error::Error for Refusal {}

/// A frame read against its entry schema.
#[allow(
    clippy::large_enum_variant,
    reason = "a transport value is built once per frame; boxing would only complicate its use"
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    OwnerRequest(OwnerRequest),
    OwnerResponse(OwnerResponse),
    OwnerNotice(OwnerNotice),
    AgentRequest(AgentRequest),
    AgentResponse(AgentResponse),
}

impl Frame {
    #[must_use]
    pub fn entry(&self) -> Entry {
        match self {
            Frame::OwnerRequest(_) => Entry::OwnerRequest,
            Frame::OwnerResponse(_) => Entry::OwnerResponse,
            Frame::OwnerNotice(_) => Entry::OwnerNotice,
            Frame::AgentRequest(_) => Entry::AgentRequest,
            Frame::AgentResponse(_) => Entry::AgentResponse,
        }
    }

    #[must_use]
    pub fn to_value(&self) -> Value {
        match self {
            Frame::OwnerRequest(f) => to_value(f),
            Frame::OwnerResponse(f) => to_value(f),
            Frame::OwnerNotice(f) => to_value(f),
            Frame::AgentRequest(f) => to_value(f),
            Frame::AgentResponse(f) => to_value(f),
        }
    }

    /// The frame as written to the channel: its canonical text and a line feed (README section 1).
    #[must_use]
    pub fn to_line(&self) -> String {
        let mut line = canonical(&self.to_value());
        line.push('\n');
        line
    }
}

fn refuse(layer: Layer, code: ErrorCode, reason: impl Into<String>) -> Refusal {
    Refusal {
        layer,
        code,
        reason: reason.into(),
    }
}

/// The frame layer for any entry: strict JSON, and the value is an object.
fn read_object(bytes: &[u8]) -> Result<Value, Refusal> {
    let value = parse_strict(bytes)
        .map_err(|e| refuse(Layer::Frame, ErrorCode::MalformedFrame, e.to_string()))?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(refuse(
            Layer::Frame,
            ErrorCode::MalformedFrame,
            "the value is not an object",
        ))
    }
}

/// Decodes the bytes of one frame, without its line feed, against `entry`.
pub fn decode(entry: Entry, bytes: &[u8]) -> Result<Frame, Refusal> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(refuse(
            Layer::Frame,
            ErrorCode::MalformedFrame,
            format!("{} bytes is more than {MAX_FRAME_BYTES}", bytes.len()),
        ));
    }
    if bytes.contains(&b'\n') {
        return Err(refuse(
            Layer::Frame,
            ErrorCode::MalformedFrame,
            "a frame holds no line feed",
        ));
    }
    let value = read_object(bytes)?;
    if value.get("protocol") != Some(&Value::String(PROTOCOL.to_owned())) {
        return Err(refuse(
            Layer::Protocol,
            ErrorCode::UnsupportedProtocol,
            "protocol is not fabric-desktop/1",
        ));
    }
    let method_known = |known: fn(&str) -> bool| match value.get("method") {
        Some(Value::String(name)) => known(name),
        _ => false,
    };
    let unknown_method = match entry {
        Entry::OwnerRequest => !method_known(|m| OwnerMethod::from_name(m).is_some()),
        Entry::AgentRequest => !method_known(|m| ToolName::from_name(m).is_some()),
        _ => false,
    };
    let schema = |e: serde_json::Error| {
        let code = if unknown_method {
            ErrorCode::UnknownMethod
        } else {
            ErrorCode::InvalidParams
        };
        refuse(Layer::Schema, code, e.to_string())
    };
    Ok(match entry {
        Entry::OwnerRequest => {
            Frame::OwnerRequest(crate::typed::from_value(&value).map_err(schema)?)
        }
        Entry::OwnerResponse => {
            Frame::OwnerResponse(crate::typed::from_value(&value).map_err(schema)?)
        }
        Entry::OwnerNotice => Frame::OwnerNotice(crate::typed::from_value(&value).map_err(schema)?),
        Entry::AgentRequest => {
            Frame::AgentRequest(crate::typed::from_value(&value).map_err(schema)?)
        }
        Entry::AgentResponse => {
            Frame::AgentResponse(crate::typed::from_value(&value).map_err(schema)?)
        }
    })
}

/// Decodes a contract record that is not a frame, such as a grant or a profile. There is no
/// size bound and no protocol layer; the value must be an object, or it is refused at the
/// frame layer, and a schema failure is `invalid_params` (README section 13).
pub fn decode_record<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Refusal> {
    let value = read_object(bytes)?;
    crate::typed::from_value(&value)
        .map_err(|e| refuse(Layer::Schema, ErrorCode::InvalidParams, e.to_string()))
}

/// The canonical text of any contract value (README section 3).
#[must_use]
pub fn encode<T: Serialize + ?Sized>(value: &T) -> String {
    canonical(&to_value(value))
}
