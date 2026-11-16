//! The Fabric Calendar broker (work package CD-W04): the process that holds the owner's
//! authority.
//!
//! It speaks the owner channel of `fabric-desktop/1` (`contracts/desktop/v1/README.md`) on stdin
//! and stdout. Its state lives in the encrypted store (`fabric-storage`) in its data directory;
//! its decisions are the authority core's (`fabric-broker-core`). The executable composes them:
//! it checks its configuration before anything starts, opens the store with the key its
//! configuration names, runs the core on its main thread and the prover and the provider on
//! worker threads, and writes frames.
//!
//! The rule prover is Isabelle, started once the store is open when the three Isabelle variables
//! name it, and pinned to the rule theory the broker was built with; without it every request
//! ends undecided. No Google account can be connected (CD-W05) and no agent can connect (CD-W06).
//! A debug build started with the test key, a scratch data directory and
//! `FABRIC_BROKER_SYNTHETIC_AGENT=1` runs a synthetic agent so a person can see requests; with
//! Isabelle too, it runs the practice calendar, so a request the prover proves allowed is carried
//! out on practice data held in memory. Nothing else an agent asks is carried out.
//!
//! - [`config`]: the six variables the broker reads, and its exit codes.
//! - [`reader`]: the bounded line reader for stdin.
//! - [`prover`]: the Isabelle prover adapter, pinned to the rule theory the broker was built
//!   with.
//! - `practice` (debug builds): the in-memory practice calendar.
//! - [`shell`]: the routing that the executable and the live tests share.
//! - `synthetic` (debug builds): the synthetic agent and its classifier.
//!
//! The executable (`src/main.rs`) is the process shell around them.
//!
//! Numbers in this crate's comments, such as (3.4), (5.7) or (D17), and the test ids c01 to
//! c31, p01 to p08 and s01 to s17 cite the sections, decisions and tests of the as-built
//! design, `docs/plans/calendar-desktop-w04-design.md`. Numbers with the prefix "PW", such as
//! PW 4.3, PW c40 or PW L1, cite the design of the prover wiring,
//! `docs/plans/calendar-desktop-prover-wiring-design.md`: its sections, decisions and the tests the
//! wiring added. Its live tests (PW L1 and on, in `tests/live.rs`) are not the W04 design's
//! package decisions L1 to L7.

pub mod config;
#[cfg(debug_assertions)]
pub mod practice;
pub mod prover;
pub mod reader;
pub mod shell;
#[cfg(debug_assertions)]
pub mod synthetic;

pub use config::{Config, ConfigError, KeySource, config_from};
pub use reader::{Line, LineReader};
