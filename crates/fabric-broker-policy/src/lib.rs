//! The live-proof gate of the Fabric Calendar permission broker (work package CD-W03;
//! `docs/plans/calendar-desktop-plan.md` sections 7, 8 and 11; ADR-108).
//!
//! The rules live in `isabelle/Fabric_Broker/Fabric_Broker.thy`. This crate mirrors its
//! datatypes ([`model`]), renders a trusted [`Context`] and one [`Action`] into a check theory
//! ([`hol`], [`check`]), has a running Isabelle server prove every rule or its negation
//! ([`gate`]), and turns the result into the contract's proof report and ruling ([`report`]).
//! The outcome follows from the eighteen rule results by lemmas the theory proves for every
//! context. Anything Isabelle did not prove completely is undecided, and the broker treats
//! undecided as a refusal.

pub mod check;
pub mod gate;
pub mod hol;
pub mod model;
pub mod report;
pub mod theory;

pub use check::{GrantResult, Reading, RuleOutcome, RuleResult, Verdict};
pub use fabric_policy::server::KillHandle;
pub use fabric_policy::{PolicyError, ServerAddress};
pub use gate::{BrokerGate, Check, Fault, GateConfig, IsabelleInstallation};
pub use model::{
    Action, ActiveSession, Actor, Calendar, CalendarKey, Context, Duration, Event, EventField,
    Grant, Mode, Outcome, PendingEffect, Profile, Recipient, RecipientRule, RecipientSet, Response,
    Row, Rule, SendUpdates, Target, Tool, ToolCallError, Window,
};
pub use report::{context_digest, proof_id, proof_report, ruling};
pub use theory::built_theory_sha256;
