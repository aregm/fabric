//! The Fabric Calendar broker (work package CD-W04): the process that holds the owner's
//! authority.
//!
//! It speaks the owner channel of `fabric-desktop/1` (`contracts/desktop/v1/README.md`) on stdin
//! and stdout. Its state lives in the encrypted store (`fabric-storage`) in its data directory;
//! its decisions are the authority core's (`fabric-broker-core`). The executable composes them:
//! it checks its configuration before anything starts, opens the store with the key its
//! configuration names, runs the core on its main thread and the prover on a worker thread, and
//! writes frames.
//!
//! No rule prover runs in this build (the Isabelle wiring follows CD-W03), no Google account can
//! be connected (CD-W05), and no agent can connect (CD-W06), so nothing an agent asks is ever
//! carried out. A debug build started with the test key, a scratch data directory and
//! `FABRIC_BROKER_SYNTHETIC_AGENT=1` runs a synthetic agent so a person can see requests.
//!
//! - [`config`]: the three variables the broker reads, and its exit codes.
//! - [`reader`]: the bounded line reader for stdin.
//! - `synthetic` (debug builds): the synthetic agent and its classifier.
//!
//! The executable (`src/main.rs`) is the process shell around them.
//!
//! Numbers in this crate's comments, such as (3.4), (5.7) or (D17), and test ids such as s08
//! or c18, cite the sections, decisions and tests of the as-built design,
//! `docs/plans/calendar-desktop-w04-design.md`.

pub mod config;
pub mod reader;
#[cfg(debug_assertions)]
pub mod synthetic;

pub use config::{Config, ConfigError, KeySource, config_from};
pub use reader::{Line, LineReader};
