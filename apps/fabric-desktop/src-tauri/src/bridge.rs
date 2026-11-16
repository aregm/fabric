//! The shapes that cross between the Tauri host and the webview.
//!
//! This is the Tauri side of the application, not part of the `fabric-desktop/1` contract. The
//! webview's copy of these shapes is `apps/fabric-desktop/ui/src/host.ts`; the tests below fix
//! the JSON so the two cannot drift apart unnoticed.

use fabric_desktop_contracts::Refusal;
use fabric_desktop_relay::{Ending, SendError};
use serde::Serialize;
use serde_json::Value;

/// What the host sends to the webview on the channel given to `owner_attach`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostMessage {
    /// The connection changed. `run` counts broker starts: 0 before any, 1 for the first.
    State { run: u64, state: HostState },
    /// One frame the broker wrote, already validated: an owner response or an owner notice.
    Frame { run: u64, frame: Value },
}

/// The host's connection to the broker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "name", rename_all = "snake_case")]
pub enum HostState {
    /// A broker child is running.
    Connected,
    /// No broker is running. Only the owner starts one again (`broker_restart`).
    Disconnected {
        reason: DisconnectReason,
        detail: String,
    },
    /// Another instance of the application holds the lock. This one started no broker.
    Conflict { detail: String },
    /// The configuration was refused before any process was started.
    ConfigurationError { detail: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DisconnectReason {
    /// The broker ended by itself or was killed from outside.
    Exited,
    /// The broker wrote something that is not a valid frame, and the host killed it.
    ProtocolViolation,
    /// The host stopped the broker because the application is quitting.
    Stopped,
    /// The operating system could not start the broker.
    StartFailed,
}

impl DisconnectReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            DisconnectReason::Exited => "exited",
            DisconnectReason::ProtocolViolation => "protocol_violation",
            DisconnectReason::Stopped => "stopped",
            DisconnectReason::StartFailed => "start_failed",
        }
    }
}

impl HostState {
    /// The state after a session ended.
    #[must_use]
    pub fn ended(ending: &Ending) -> HostState {
        let (reason, detail) = match ending {
            Ending::Exited { description, .. } => (DisconnectReason::Exited, description.clone()),
            Ending::ProtocolViolation { reason } => {
                (DisconnectReason::ProtocolViolation, reason.clone())
            }
            Ending::Stopped { killed: false } => (
                DisconnectReason::Stopped,
                "the broker stopped when asked".to_owned(),
            ),
            Ending::Stopped { killed: true } => (
                DisconnectReason::Stopped,
                "the broker did not stop in time and was killed".to_owned(),
            ),
        };
        HostState::Disconnected { reason, detail }
    }
}

/// Why `owner_send` did not pass a frame to the broker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SendRefusal {
    /// The frame is not a valid owner request; the layer and code are the contract's.
    Refused {
        layer: &'static str,
        code: &'static str,
        reason: String,
    },
    /// No broker is running.
    NotConnected,
    /// Writing to the broker failed.
    Io { detail: String },
}

impl SendRefusal {
    #[must_use]
    pub fn refused(refusal: &Refusal) -> SendRefusal {
        SendRefusal::Refused {
            layer: refusal.layer.as_str(),
            code: refusal.code.as_str(),
            reason: refusal.reason.clone(),
        }
    }
}

impl From<SendError> for SendRefusal {
    fn from(error: SendError) -> SendRefusal {
        match error {
            SendError::Refused(refusal) => SendRefusal::refused(&refusal),
            SendError::NotRunning => SendRefusal::NotConnected,
            SendError::Io(detail) => SendRefusal::Io { detail },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric_desktop_contracts::{Entry, decode};
    use serde_json::json;

    fn value<T: Serialize>(message: &T) -> Value {
        serde_json::to_value(message).expect("the bridge types serialize")
    }

    #[test]
    fn state_messages_have_the_shape_the_webview_reads() {
        assert_eq!(
            value(&HostMessage::State {
                run: 1,
                state: HostState::Connected
            }),
            json!({"kind": "state", "run": 1, "state": {"name": "connected"}})
        );
        assert_eq!(
            value(&HostMessage::State {
                run: 1,
                state: HostState::Disconnected {
                    reason: DisconnectReason::Exited,
                    detail: "killed by signal 9".to_owned()
                }
            }),
            json!({"kind": "state", "run": 1, "state":
                {"name": "disconnected", "reason": "exited", "detail": "killed by signal 9"}})
        );
        assert_eq!(
            value(&HostState::Conflict {
                detail: "x".to_owned()
            }),
            json!({"name": "conflict", "detail": "x"})
        );
        assert_eq!(
            value(&HostState::ConfigurationError {
                detail: "x".to_owned()
            }),
            json!({"name": "configuration_error", "detail": "x"})
        );
    }

    #[test]
    fn every_disconnect_reason_is_written_as_its_name() {
        for reason in [
            DisconnectReason::Exited,
            DisconnectReason::ProtocolViolation,
            DisconnectReason::Stopped,
            DisconnectReason::StartFailed,
        ] {
            assert_eq!(value(&reason), json!(reason.as_str()));
        }
    }

    #[test]
    fn frame_messages_carry_the_frame_as_an_object() {
        let frame = json!({"protocol": "fabric-desktop/1", "sequence": 1, "change": {"kind": "snapshot_required"}});
        assert_eq!(
            value(&HostMessage::Frame {
                run: 2,
                frame: frame.clone()
            }),
            json!({"kind": "frame", "run": 2, "frame": frame})
        );
    }

    #[test]
    fn endings_become_disconnected_states() {
        assert_eq!(
            HostState::ended(&Ending::Exited {
                code: Some(3),
                description: "exit code 3".to_owned()
            }),
            HostState::Disconnected {
                reason: DisconnectReason::Exited,
                detail: "exit code 3".to_owned()
            }
        );
        assert!(matches!(
            HostState::ended(&Ending::ProtocolViolation {
                reason: "bad".to_owned()
            }),
            HostState::Disconnected {
                reason: DisconnectReason::ProtocolViolation,
                ..
            }
        ));
        assert!(matches!(
            HostState::ended(&Ending::Stopped { killed: true }),
            HostState::Disconnected {
                reason: DisconnectReason::Stopped,
                ..
            }
        ));
    }

    #[test]
    fn refusals_name_the_contract_layer_and_code() {
        let refusal = decode(Entry::OwnerRequest, b"not json").unwrap_err();
        let sent = value(&SendRefusal::from(SendError::Refused(refusal)));
        assert_eq!(sent["kind"], "refused");
        assert_eq!(sent["layer"], "frame");
        assert_eq!(sent["code"], "malformed_frame");
        assert!(sent["reason"].is_string());
        assert_eq!(
            value(&SendRefusal::from(SendError::NotRunning)),
            json!({"kind": "not_connected"})
        );
        assert_eq!(
            value(&SendRefusal::from(SendError::Io("x".to_owned()))),
            json!({"kind": "io", "detail": "x"})
        );
    }
}
