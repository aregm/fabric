//! The Fabric Calendar broker. In work package CD-W02 it is a spike.
//!
//! The spike speaks the owner channel of `fabric-desktop/1` (`contracts/desktop/v1/README.md`)
//! on stdin and stdout and nothing else. It has no store, no policy gate, no prover, no Google
//! access and no agent channel. Its agent, account, calendar and requests are synthetic: after
//! the first `subscribe` it plays a fixed script of two requests that wait for the owner's
//! decision. An approval creates a grant, but nothing is ever dispatched, because no prover
//! can give the fresh permit; the request ends `undecided`.
//!
//! - [`spike`]: the state machine, pure and driven by the caller's clock.
//! - [`reader`]: the bounded line reader for stdin.
//! - [`time`]: contract timestamps from Unix milliseconds.
//!
//! The executable (`src/main.rs`) is the process shell around them.

pub mod reader;
pub mod spike;
pub mod time;

pub use reader::{Line, LineReader};
pub use spike::Spike;
pub use time::format_timestamp;
