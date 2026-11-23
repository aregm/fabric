//! The Tauri host's relay to the broker child, without Tauri (work package CD-W02).
//!
//! The host starts one broker process and talks to it over the owner channel of
//! `fabric-desktop/1`: one JSON frame per line on the child's stdin and stdout. This crate holds
//! everything the host does that does not need Tauri:
//!
//! - [`BrokerConfig`] checks the broker's path before any process starts and builds the
//!   command, with an environment that holds only the names of an allowlist.
//! - [`Session`] runs the child, validates every frame in both directions with the contract
//!   crate, forwards the valid ones, notices the child's death and stops it with a bounded drain.
//! - [`InstanceLock`] keeps a second host from starting a second broker.
//!
//! The relay validates and forwards. It makes no policy decision, holds no state about
//! requests, never builds a frame of its own and never answers for the broker.

mod config;
mod instance;
mod line;
mod session;

pub use config::{
    BROKER_APP_RESOURCES_DIR, BROKER_APP_SUPPORT_DIR, BROKER_DATA_DIR, BROKER_ISABELLE_HOME,
    BROKER_ISABELLE_USER_HOME, BROKER_KEY_SOURCE, BROKER_RULES_DIR, BROKER_SYNTHETIC_AGENT,
    BROKER_VARIABLE_PREFIX, BrokerConfig, ConfigError, ENVIRONMENT_ALLOWLIST, broker_beside,
    is_broker_variable, same_name,
};
pub use instance::{InstanceError, InstanceLock};
pub use session::{Ending, Event, QUIT_DRAIN, SendError, Session};
