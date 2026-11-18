//! Sign-in to Google for a desktop app, refresh and revocation (design section 3).
//!
//! - [`pkce`]: the verifier, the state and the nonce of one sign-in, and the S256 challenge
//!   (3.2).
//! - [`url`]: the authorization address the browser is given (3.2.1).
//! - [`listener`]: the loopback listener that hears Google's redirect (3.3).
//! - [`exchange`]: the token request and its answer (3.4).
//! - [`id_token`]: the ten checks of the identity token and Google's key set (3.5).
//! - [`scopes`]: the granted scopes against the five asked for (3.6).
//! - [`flow`]: [`Flow`], [`PendingFlow`], [`ConnectOutcome`]: one sign-in from start to the
//!   token store (3.2, 3.2.2, 3.7).
//! - [`access`]: [`GoogleAccess`], the access tokens of Calendar calls, refreshed when needed
//!   (3.8).
//! - [`revoke`](mod@revoke): revocation at disconnect (3.9).
//!
//! What a caller does:
//!
//! - Sign-in: build one [`Flow`] with [`Flow::new`] (endpoints, client, clock), then for each
//!   `connect_google` job call [`Flow::start`] with a [`ConnectJob`]; it prepares and gives a
//!   [`PendingFlow`], and opens no browser. Take a [`Supersede`] handle from it when another
//!   thread may end it (PD5), then call [`PendingFlow::wait`] with a [`BrowserOpener`], the
//!   token store and the access cache; it opens the browser unless the flow was superseded
//!   before. It gives the [`ConnectOutcome`].
//! - Access tokens: build one [`GoogleAccess`] over the same transport, client, token store,
//!   cache and clock; it implements [`crate::tokens::AccessTokens`].
//! - Revocation: [`revoke()`] with the transport, the token store and the account. The caller
//!   deletes the stored token afterwards, whatever the result (3.9).
//!
//! Nothing here formats a token, a code, the verifier, the state, the nonce, the client secret
//! or an address into an error, a `Debug` or a `Display` (3.10, PD27).

pub mod access;
pub mod exchange;
pub mod flow;
pub mod id_token;
pub mod listener;
pub mod pkce;
pub mod revoke;
pub mod scopes;
pub mod url;

pub use access::GoogleAccess;
#[cfg(debug_assertions)]
pub use flow::RecordingBrowser;
pub use flow::{
    BrowserNotOpened, BrowserOpener, CancelReason, ConnectJob, ConnectOutcome, Flow, PendingFlow,
    Supersede,
};
pub use id_token::IdentityProblem;
pub use revoke::{Revocation, revoke};
pub use scopes::Scope;
