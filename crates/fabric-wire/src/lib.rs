//! The wire protocol between two people's gateways (docs/isabelle-calendar-demo.md, sections 6,
//! 11 and D16–D17): typed messages signed with identity keys, inside Noise IK sessions, in
//! frames that a relay carries without being able to read them.

pub mod error;
pub mod frame;
pub mod message;
pub mod noise;

pub use error::WireError;
pub use frame::{Frame, MAX_FRAME_BYTES, SessionId};
pub use message::{Body, MAX_MESSAGE_BYTES, MESSAGE_AAD, Message, SignedRecord};
pub use noise::{Call, IncomingCall, PATTERN, Session};
