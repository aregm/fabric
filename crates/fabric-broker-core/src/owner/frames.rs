//! Owner frames: decoding a line, refusals, and the response and notice frames.
//!
//! `refusal_answer`, `error_frame`, `result_frame` and the oversized and `hello` answers come
//! from the CD-W02 stand-in broker (design 6.1); they never panic on what a line holds.

use fabric_desktop_contracts::error::{
    ErrorCode, InvalidParams, OwnerError, OwnerFault, StoreReason, StoreUnavailable,
    UnsupportedProtocol, UnsupportedSchemaVersion,
};
use fabric_desktop_contracts::owner::{
    HelloParams, HelloResult, OwnerMethod, OwnerResponse, OwnerResult, Party,
};
use fabric_desktop_contracts::scalar::{
    Explanation, Id, Label, List, MethodText, PositiveCount, Text,
};
use fabric_desktop_contracts::{Frame, Layer, Limits, Refusal, SCHEMA_VERSION, parse_strict};
use serde_json::Value;

use crate::broker::BROKER_NAME;
use crate::service::OwnerRefusal;

/// The answer to a line that is not one strict JSON object.
pub(crate) const MALFORMED_MESSAGE: &str =
    "The frame is not one strict JSON object of at most 1,048,576 bytes.";
/// The answer to a line longer than the frame limit.
pub(crate) const OVERSIZED_MESSAGE: &str = "The frame is longer than 1,048,576 bytes.";
/// The answer whose frame would be longer than the frame limit (README section 1).
pub(crate) const TOO_LONG_MESSAGE: &str = "The answer would be longer than 1,048,576 bytes, so \
     the broker cannot send it; remove some recipient sets, profiles or grants.";
/// The answer to `connect_google` in this build (design 5.3, CCR-1).
pub(crate) const CONNECT_GOOGLE_MESSAGE: &str =
    "This build cannot connect a Google account yet; CD-W05 adds it.";
/// The answer to a frame whose write failed: the run stops (design 5.2, D7).
pub(crate) const HALT_MESSAGE: &str = "The broker could not use its store, so nothing was \
     changed by this frame; the broker stops and checks the store when it starts again.";
/// The fallback for a message that does not fit an explanation.
const FALLBACK_MESSAGE: &str = "The broker refused the frame.";

/// An explanation from broker text: at most 1000 code points, never empty.
pub(crate) fn explanation(message: &str) -> Explanation {
    let text: String = message.chars().take(1000).collect();
    Explanation::new(text)
        .unwrap_or_else(|_| Explanation::new(FALLBACK_MESSAGE).expect("a fixed explanation"))
}

/// The method's name as response text.
pub(crate) fn method_text(method: OwnerMethod) -> Option<MethodText> {
    MethodText::new(method.as_str()).ok()
}

/// An error response.
pub(crate) fn error_frame(
    id: Option<Id>,
    method: Option<MethodText>,
    fault: OwnerFault,
    message: &str,
) -> Frame {
    Frame::OwnerResponse(OwnerResponse::Error {
        id,
        method,
        error: OwnerError {
            fault,
            message: explanation(message),
            correlation_id: None,
        },
    })
}

/// An error response to a request with `id` and `method`.
pub(crate) fn refuse(id: &Id, method: OwnerMethod, fault: OwnerFault, message: &str) -> Frame {
    error_frame(Some(id.clone()), method_text(method), fault, message)
}

/// The response to a refused command.
pub(crate) fn refusal_frame(id: &Id, method: OwnerMethod, refusal: OwnerRefusal) -> Frame {
    refuse(id, method, refusal.fault, &refusal.message)
}

/// A result response.
pub(crate) fn result_frame(id: Id, result: OwnerResult) -> Frame {
    Frame::OwnerResponse(OwnerResponse::Result { id, result })
}

/// `store_unavailable` with `reason` and `message`.
pub(crate) fn store_unavailable(
    id: Option<Id>,
    method: Option<MethodText>,
    reason: StoreReason,
    message: &str,
) -> Frame {
    error_frame(
        id,
        method,
        OwnerFault::StoreUnavailable(StoreUnavailable { reason }),
        message,
    )
}

/// The answer to a frame in hand when a write or read of the store failed: `store_unavailable`
/// with the reason `locked` (design 5.2).
pub(crate) fn halted(id: &Id, method: OwnerMethod) -> Frame {
    store_unavailable(
        Some(id.clone()),
        method_text(method),
        StoreReason::Locked,
        HALT_MESSAGE,
    )
}

/// The answer to a line longer than the frame limit.
pub(crate) fn oversized() -> Frame {
    error_frame(None, None, OwnerFault::MalformedFrame, OVERSIZED_MESSAGE)
}

/// An `invalid_params` refusal at `pointer`.
pub(crate) fn invalid_params(pointer: &str) -> OwnerFault {
    OwnerFault::InvalidParams(InvalidParams {
        pointer: Text::new(pointer)
            .unwrap_or_else(|_| Text::new("/params").expect("a fixed pointer")),
    })
}

/// `hello`: the broker's name, the build, the schema version and the limits; another schema
/// version is refused.
pub(crate) fn hello(frame_id: Id, params: &HelloParams, build: &str, limits: Limits) -> Frame {
    if params.schema_version.get() != SCHEMA_VERSION {
        return refuse(
            &frame_id,
            OwnerMethod::Hello,
            OwnerFault::UnsupportedSchemaVersion(UnsupportedSchemaVersion {
                received: params.schema_version,
                supported: PositiveCount::of(SCHEMA_VERSION),
            }),
            "The broker speaks schema version 1 only.",
        );
    }
    // A build that is not a label (empty or too long) is named by the package version.
    let build = Label::new(build)
        .unwrap_or_else(|_| Label::new(env!("CARGO_PKG_VERSION")).expect("a version label"));
    result_frame(
        frame_id,
        OwnerResult::Hello(HelloResult {
            broker: Party {
                name: Label::new(BROKER_NAME).expect("the broker's name is a label"),
                build,
            },
            schema_version: PositiveCount::of(SCHEMA_VERSION),
            capabilities: List::new(Vec::new()).expect("no capabilities"),
            limits,
        }),
    )
}

/// The answer to a frame that `decode` refused, by the layer it was refused at (README
/// section 5). Never stored.
pub(crate) fn refusal_answer(line: &[u8], refusal: &Refusal) -> Frame {
    if refusal.layer == Layer::Frame {
        return error_frame(None, None, OwnerFault::MalformedFrame, MALFORMED_MESSAGE);
    }
    // The frame layer passed, so the line is one strict JSON object.
    let value = parse_strict(line).ok();
    let member = |name: &str| {
        value
            .as_ref()
            .and_then(|v| v.get(name))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let frame_id = member("id").and_then(|s| Id::new(s).ok());
    let method = member("method").and_then(|s| MethodText::new(s).ok());
    let (fault, message) = match refusal.layer {
        Layer::Protocol => (
            OwnerFault::UnsupportedProtocol(UnsupportedProtocol {
                received: member("protocol").and_then(|s| Text::new(s).ok()),
            }),
            "The frame does not name the protocol fabric-desktop/1.",
        ),
        _ if refusal.code == ErrorCode::UnknownMethod => (
            OwnerFault::UnknownMethod,
            "The method is not an owner method of fabric-desktop/1.",
        ),
        // The contract crate does not say which value failed, so the pointer names the
        // whole of `params`.
        _ => (
            invalid_params("/params"),
            "The frame does not match the schema of its method.",
        ),
    };
    error_frame(frame_id, method, fault, message)
}
