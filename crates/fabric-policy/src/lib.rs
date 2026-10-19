//! The Isabelle policy gate for the calendar demo (docs/isabelle-calendar-demo.md, section 8).
//!
//! The rules live in `isabelle/Fabric_Policy/Fabric_Policy.thy`. This crate renders a trusted
//! [`Context`] and an agent's [`Action`]s into a check theory, has a running Isabelle server
//! prove each rule or its negation, and turns the result into [`Verdict`]s. It never permits
//! an action it could not check.

pub mod check;
pub mod error;
pub mod explain;
pub mod gate;
pub mod hol;
pub mod model;
pub mod server;
pub mod time;

pub use check::{ActionVerdict, Denial, RuleOutcome, RuleResult, Verdict};
pub use error::PolicyError;
pub use gate::{CheckKind, CheckReport, Gate, GateConfig};
pub use model::{
    Action, Answer, Candidate, CloseReason, Consent, Context, Decision, Negotiation, Parameters,
    Request, Role, Rule,
};
