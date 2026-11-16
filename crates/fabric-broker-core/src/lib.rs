//! The Fabric Calendar broker's authority core (work package CD-W04).
//!
//! A pure state machine over the encrypted store (`fabric-storage`), driven by the caller's
//! clock: it takes owner lines, agent calls and worker results as [`Input`]s and answers with
//! an [`Output`] of frames to write, jobs to hand to workers, and a halt after a failed write.
//! It runs no thread and does no I/O besides the store.
//!
//! - [`broker`]: the facade the process shell calls, its unavailable mode, reopen and halt.
//! - [`ports`]: what the prover, the provider and the agent adapter exchange with the core.
//! - [`ids`]: new ids.
//!
//! Numbers in this crate's comments, such as (3.4), (5.7) or (D17), and test ids such as s08
//! or c18, cite the sections, decisions and tests of the as-built design,
//! `docs/plans/calendar-desktop-w04-design.md`.

pub mod broker;
pub mod ids;
pub mod ports;

mod owner;
mod service;

pub use broker::*;
pub use fabric_storage::time::{MAX_TIMESTAMP_MS, format_timestamp};
