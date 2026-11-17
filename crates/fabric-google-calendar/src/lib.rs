//! The Google Calendar adapter of Fabric Calendar (work package CD-W05).
//!
//! The adapter signs in to Google for a desktop app, keeps the refresh token in a token store,
//! makes the Calendar calls the broker orders, and classifies every answer so that a change
//! whose outcome is unknown is reconciled and never sent twice. It talks only to Google's fixed
//! endpoints, or in a debug build to a loopback double; no function takes an address, a path
//! or a header from its caller (2.2).
//!
//! Numbers in this crate's comments, such as (6.7) or (PD14), and test ids such as g26, cite
//! the sections, decisions and tests of `docs/plans/calendar-desktop-w05-design.md`.
//!
//! No error value, `Debug` or `Display` of this crate carries a secret (a token, the client
//! secret, a verifier, a state, a nonce, a code), an address, a calendar id or event content.
//! Errors are fixed labels and numbers (PD27, 3.10). Secrets are held as [`Secret`].
//!
//! The foundation modules:
//!
//! - [`secret`] holds [`Secret`].
//! - [`clock`] holds [`Clock`], [`SystemClock`], the test double [`ManualClock`].
//! - [`endpoints`] holds [`Endpoints`], Google's five addresses and the loopback set (5.1).
//! - [`http`] holds [`http::Transport`], the only place that talks HTTP (5.2), and [`http::ids`],
//!   the typed ids that alone may reach a path or a header (5.3).
//! - [`client`] holds [`ClientConfig`] and Google's client file (3.1).
//! - [`tokens`] holds [`TokenStore`], [`MemoryTokenStore`], [`AccessCache`], [`forget`]
//!   (section 4).
//! - [`time`] turns RFC 3339 to instants and to the contract's time types (6.9).

pub mod calendar;
pub mod client;
pub mod clock;
pub mod endpoints;
pub mod http;
pub mod oauth;
pub mod outcome;
pub mod reconcile;
pub mod secret;
pub mod time;
pub mod tokens;
pub mod translate;

pub use client::{ClientConfig, ClientFileError, ClientFileProblem};
pub use clock::{Clock, ManualClock, SharedClock, SystemClock};
pub use endpoints::{Endpoint, Endpoints, Scheme, Service};
pub use http::ids::{CalendarId, EventId, IdError, ReservedEventId, Revision};
pub use secret::Secret;
pub use tokens::{
    AccessCache, AccessFailure, AccessTokens, AuthLostCause, MemoryTokenStore, TokenStore,
    TokenStoreError, forget,
};
