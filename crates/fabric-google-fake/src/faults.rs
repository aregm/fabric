//! Faults (14.2): each is armed for the next request that matches it, or the next N.
//!
//! Every armed fault whose [`Match`] fits a request fires on it and uses up one of its times;
//! when several fire on one request their actions are combined. A fault that is used up is
//! removed. [`FaultHandle::fired`] tells a test how often a fault fired.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use serde_json::Value;

use crate::keys::KeyId;
use crate::state::State;

/// Which requests a fault fires on. An unset member matches everything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Match {
    /// The method, exactly (`"PATCH"`).
    pub method: Option<String>,
    /// A prefix of the decoded path (`"/calendar/v3/calendars/a@b.example/events"`).
    pub path_prefix: Option<String>,
    /// The `grant_type` form field of a token request.
    pub grant_type: Option<String>,
}

impl Match {
    /// Matches requests with this method whose decoded path starts with `path_prefix`.
    pub fn new(method: &str, path_prefix: &str) -> Match {
        Match {
            method: Some(method.to_string()),
            path_prefix: Some(path_prefix.to_string()),
            grant_type: None,
        }
    }

    /// Matches every request.
    pub fn any() -> Match {
        Match::default()
    }

    pub(crate) fn fits(&self, method: &str, path: &str, grant_type: Option<&str>) -> bool {
        self.method.as_deref().is_none_or(|m| m == method)
            && self
                .path_prefix
                .as_deref()
                .is_none_or(|p| path.starts_with(p))
            && self
                .grant_type
                .as_deref()
                .is_none_or(|g| grant_type == Some(g))
    }
}

/// A change to the fake's state run when a fault fires, before the request is handled: an
/// event changed by someone else, a role lowered, a calendar gone (14.2).
pub type Hook = Arc<dyn Fn(&mut State) + Send + Sync>;

/// When a late change (a change Google finishes after the client stopped waiting) lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LateTrigger {
    /// After this many GET requests of the changed event's path have been answered: with
    /// `AfterReads(1)` the first read still sees the old event and the second sees the change
    /// (PD20, g43).
    AfterReads(u32),
    /// When the test calls [`crate::FakeGoogle::apply_late_changes`].
    Manual,
}

/// What a fault does.
#[derive(Clone)]
pub enum FaultAction {
    /// Answer `status` with Google's error shape (`error.errors[0].reason`) instead of handling
    /// the request: nothing is applied. `retry_after`, when set, is sent as `Retry-After`.
    Error {
        /// The status code.
        status: u16,
        /// Google's `reason`.
        reason: String,
        /// The `Retry-After` header value, as sent.
        retry_after: Option<String>,
    },
    /// Answer `status` with the OAuth error shape (`{"error", "error_description"}`) instead
    /// of handling the request (the token and revocation endpoints).
    OAuthError {
        /// The status code.
        status: u16,
        /// The `error` member.
        error: String,
        /// The `error_subtype` member, when set.
        subtype: Option<String>,
    },
    /// Handle the request (apply the change), then close the connection without an answer.
    ApplyThenClose,
    /// Handle the request (apply the change), then answer this status with Google's error
    /// shape instead of the real answer.
    ApplyThenError {
        /// The status code (503 for the common case).
        status: u16,
        /// Google's `reason`.
        reason: String,
    },
    /// Close the connection without handling the request and without an answer.
    CloseWithoutApplying,
    /// Handle the request, then wait this long before answering.
    Delay(Duration),
    /// Close without an answer now; handle the request later, when the trigger fires, against
    /// the state as it is then (its `If-Match` included). The answer of that late handling is
    /// discarded.
    ApplyLate(LateTrigger),
    /// Answer 200 with a body that is not JSON, still labelled `application/json`.
    NotJson,
    /// Pad the real answer's body with spaces (still valid JSON) to at least this many bytes.
    TooLong(usize),
    /// Send the real answer with this `Content-Type`.
    ContentType(String),
    /// Send the real answer gzip-compressed, with `Content-Encoding: gzip`.
    Gzip,
    /// Answer `302 Found` with this `Location` instead of handling the request.
    Redirect(String),
    /// Treat the request's access token as expired (it stays expired): 401.
    ExpiredAccessToken,
    /// Change a successful answer of the token endpoint.
    Token(TokenFault),
    /// Change the identity token of a successful answer of the token endpoint.
    IdToken(IdTokenFault),
    /// The event this insert creates is not found by GET (nor listed) for this many reads
    /// (PD20, g30).
    HideInsertForReads(u32),
    /// Cut the attendee list of every event in the answer: only the `self` entry and
    /// `attendeesOmitted: true`.
    CutAttendees,
    /// Run a change on the state before the request is handled.
    Before(Hook),
    /// Answer this status and JSON body instead of handling the request: nothing is applied
    /// (an insert answered with another event's id, g28).
    Json(u16, Value),
    /// Answer these bytes as they are, then close.
    Raw(Vec<u8>),
    /// Handle an insert or a PATCH (the change is applied, with a new etag), but store and
    /// answer the event with the trailing white space of its `description` removed: a fitting
    /// 200 in another form than was sent (18.7 point 2; LV18).
    RewriteDescriptionTrimmed,
    /// Handle an insert or a PATCH (the change is applied, with a new etag), but store and
    /// answer the event with the `timeZone` of `start` and `end` written `to` where it was
    /// `from`: a zone alias, such as `Asia/Calcutta` for `Asia/Kolkata` (18.7 point 2; LV18).
    RewriteZoneAlias {
        /// The zone as sent.
        from: String,
        /// The alias stored and answered.
        to: String,
    },
}

impl FaultAction {
    /// An error answer with Google's shape and no `Retry-After`.
    pub fn error(status: u16, reason: &str) -> FaultAction {
        FaultAction::Error {
            status,
            reason: reason.to_string(),
            retry_after: None,
        }
    }

    /// An error answer with Google's shape and a `Retry-After` header.
    pub fn error_retry_after(status: u16, reason: &str, retry_after: &str) -> FaultAction {
        FaultAction::Error {
            status,
            reason: reason.to_string(),
            retry_after: Some(retry_after.to_string()),
        }
    }

    /// Apply the change, then answer 503 `backendError`.
    pub fn apply_then_503() -> FaultAction {
        FaultAction::ApplyThenError {
            status: 503,
            reason: "backendError".to_string(),
        }
    }

    /// An OAuth error answer.
    pub fn oauth_error(status: u16, error: &str) -> FaultAction {
        FaultAction::OAuthError {
            status,
            error: error.to_string(),
            subtype: None,
        }
    }

    /// `400 invalid_grant`, as for a refresh token that ended (FA9).
    pub fn invalid_grant() -> FaultAction {
        FaultAction::oauth_error(400, "invalid_grant")
    }

    /// `400 admin_policy_enforced`.
    pub fn admin_policy_enforced() -> FaultAction {
        FaultAction::oauth_error(400, "admin_policy_enforced")
    }

    /// `401 invalid_client`.
    pub fn invalid_client() -> FaultAction {
        FaultAction::oauth_error(401, "invalid_client")
    }

    /// Before the request, merge `patch` into an event as someone else would (a new etag).
    pub fn change_event(calendar: &str, event: &str, patch: Value) -> FaultAction {
        let calendar = calendar.to_string();
        let event = event.to_string();
        FaultAction::Before(Arc::new(move |state: &mut State| {
            state.change_event(&calendar, &event, patch.clone());
        }))
    }

    /// Before the request, run `hook` on the state.
    pub fn before(hook: impl Fn(&mut State) + Send + Sync + 'static) -> FaultAction {
        FaultAction::Before(Arc::new(hook))
    }

    pub(crate) fn label(&self) -> String {
        match self {
            FaultAction::Error { status, reason, .. } => format!("error {status} {reason}"),
            FaultAction::OAuthError { status, error, .. } => format!("oauth {status} {error}"),
            FaultAction::ApplyThenClose => "apply then close".into(),
            FaultAction::ApplyThenError { status, .. } => format!("apply then {status}"),
            FaultAction::CloseWithoutApplying => "close without applying".into(),
            FaultAction::Delay(d) => format!("delay {} ms", d.as_millis()),
            FaultAction::ApplyLate(t) => format!("apply late {t:?}"),
            FaultAction::NotJson => "not json".into(),
            FaultAction::TooLong(n) => format!("too long {n}"),
            FaultAction::ContentType(c) => format!("content type {c}"),
            FaultAction::Gzip => "gzip".into(),
            FaultAction::Redirect(_) => "redirect".into(),
            FaultAction::ExpiredAccessToken => "expired access token".into(),
            FaultAction::Token(t) => format!("token {t:?}"),
            FaultAction::IdToken(t) => format!("id token {t:?}"),
            FaultAction::HideInsertForReads(n) => format!("hide insert for {n} reads"),
            FaultAction::CutAttendees => "cut attendees".into(),
            FaultAction::Before(_) => "before hook".into(),
            FaultAction::Json(status, _) => format!("json answer {status}"),
            FaultAction::Raw(_) => "raw answer".into(),
            FaultAction::RewriteDescriptionTrimmed => "rewrite description trimmed".into(),
            FaultAction::RewriteZoneAlias { from, to } => format!("rewrite zone {from} as {to}"),
        }
    }
}

impl fmt::Debug for FaultAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label())
    }
}

/// A change to a successful answer of the token endpoint (14.2).
#[derive(Clone, Debug, PartialEq)]
pub enum TokenFault {
    /// A refresh answer carries a new refresh token; the old one stops working.
    RotateRefreshToken,
    /// The answer has no `refresh_token`.
    NoRefreshToken,
    /// The answer has no `id_token`.
    NoIdToken,
    /// The answer's `scope` is this string, and the issued access token carries these scopes.
    Scope(String),
    /// The answer's `token_type` is this string.
    TokenType(String),
    /// The answer carries `refresh_token_expires_in` (time-based access), and the refresh token
    /// ends after this many seconds.
    RefreshTokenExpiresIn(i64),
    /// The answer's `expires_in` is this number, and the access token ends after it.
    ExpiresIn(i64),
    /// Sets a member of the answer to this value.
    Member(String, Value),
    /// Removes a member of the answer.
    DropMember(String),
}

/// A change to the identity token of a token answer (14.2, g10). Each changes one thing, so
/// that a test shows which check refuses it.
#[derive(Clone, Debug, PartialEq)]
pub enum IdTokenFault {
    /// `iss` is this string.
    Issuer(String),
    /// `aud` is this value (a string or an array).
    Audience(Value),
    /// `azp` is this string.
    Azp(String),
    /// `nonce` is this string.
    Nonce(String),
    /// No `nonce`.
    NoNonce,
    /// `exp` is two minutes ago (`iat` five minutes ago).
    Expired,
    /// `iat` is five minutes ahead.
    IssuedInFuture,
    /// `iat` is twenty minutes ago (`exp` still ahead).
    TooOld,
    /// The signature is of other bytes.
    BadSignature,
    /// Header `alg` is `none`, and the signature is empty.
    AlgNone,
    /// Header `alg` is `HS256`, signed with HMAC-SHA256 keyed by the signing key's modulus.
    AlgHs256,
    /// Header `kid` names no key of any key set.
    UnknownKid,
    /// No `kid` in the header.
    NoKid,
    /// Header `kid` is this string (the signature still from the signing key).
    Kid(String),
    /// A `crit` header member.
    Crit,
    /// A `jwk` header member holding the attacker key; the token is signed by that key and its
    /// `kid` names the ordinary signing key.
    JwkHeader,
    /// Header `alg` appears twice (`HS256`, then `RS256`).
    DoubledHeaderMember,
    /// Payload `sub` appears twice (another subject first, then the real one).
    DoubledPayloadMember,
    /// Signed by the key with exponent 3, which the key set then lists.
    WeakExponentKey,
    /// Signed by the 1024-bit key, which the key set then lists.
    ShortModulusKey,
    /// Signed by this key, with its `kid` (whether the key set lists it is the key set's
    /// business, [`State::publish_key`]).
    SignWith(KeyId),
    /// `email_verified` is `false`.
    EmailUnverified,
    /// `email` is this string.
    Email(String),
    /// `sub` is this string.
    Subject(String),
    /// Sets a claim.
    Claim(String, Value),
    /// Removes a claim.
    DropClaim(String),
}

/// A fault to arm with [`crate::FakeGoogle::arm`].
#[derive(Clone, Debug)]
pub struct Fault {
    pub(crate) matcher: Match,
    pub(crate) times: u32,
    pub(crate) actions: Vec<FaultAction>,
}

impl Fault {
    /// A fault on the next request with `method` whose decoded path starts with `path_prefix`.
    pub fn on(method: &str, path_prefix: &str, action: FaultAction) -> Fault {
        Fault::matching(Match::new(method, path_prefix), action)
    }

    /// A fault on the next request that `matcher` fits.
    pub fn matching(matcher: Match, action: FaultAction) -> Fault {
        Fault {
            matcher,
            times: 1,
            actions: vec![action],
        }
    }

    /// A fault on the next request, whatever it is.
    pub fn next_request(action: FaultAction) -> Fault {
        Fault::matching(Match::any(), action)
    }

    /// A fault on the next token request (`POST /token`).
    pub fn token(action: FaultAction) -> Fault {
        Fault::on("POST", "/token", action)
    }

    /// A change of the next successful token answer.
    pub fn token_answer(fault: TokenFault) -> Fault {
        Fault::token(FaultAction::Token(fault))
    }

    /// A change of the identity token of the next token answer.
    pub fn id_token(fault: IdTokenFault) -> Fault {
        Fault::token(FaultAction::IdToken(fault))
    }

    /// Fires on this many matching requests instead of one.
    pub fn times(mut self, times: u32) -> Fault {
        self.times = times;
        self
    }

    /// Fires on every matching request until the fake stops.
    pub fn always(self) -> Fault {
        self.times(u32::MAX)
    }

    /// Adds an action that fires together with the others.
    pub fn and(mut self, action: FaultAction) -> Fault {
        self.actions.push(action);
        self
    }

    /// Fires only on token requests with this `grant_type` (`authorization_code`,
    /// `refresh_token`).
    pub fn grant_type(mut self, grant_type: &str) -> Fault {
        self.matcher.grant_type = Some(grant_type.to_string());
        self
    }
}

/// Tells how often an armed fault fired.
#[derive(Clone, Debug)]
pub struct FaultHandle {
    pub(crate) fired: Arc<AtomicU32>,
    pub(crate) times: u32,
}

impl FaultHandle {
    /// How many requests this fault fired on.
    pub fn fired(&self) -> u32 {
        self.fired.load(Ordering::SeqCst)
    }

    /// How many more times it will fire (`u32::MAX` minus the fired for [`Fault::always`]).
    pub fn remaining(&self) -> u32 {
        self.times.saturating_sub(self.fired())
    }
}

pub(crate) struct ArmedFault {
    pub(crate) fault: Fault,
    pub(crate) fired: Arc<AtomicU32>,
}

/// What the faults that fired on one request ask for, combined.
#[derive(Default)]
pub(crate) struct Plan {
    pub(crate) labels: Vec<String>,
    pub(crate) hooks: Vec<Hook>,
    /// Answer instead of handling, without applying.
    pub(crate) instead: Option<Instead>,
    pub(crate) after_apply: Option<AfterApply>,
    pub(crate) delay: Option<Duration>,
    pub(crate) late: Option<LateTrigger>,
    pub(crate) body: Vec<BodyChange>,
    pub(crate) expired_token: bool,
    pub(crate) token: Vec<TokenFault>,
    pub(crate) id_token: Vec<IdTokenFault>,
    pub(crate) hide_insert_reads: Option<u32>,
    pub(crate) cut_attendees: bool,
    /// Rewritings of an inserted or patched event, in the order armed.
    pub(crate) rewrites: Vec<Rewrite>,
}

/// A rewriting of an inserted or patched event before it is stored and answered.
pub(crate) enum Rewrite {
    DescriptionTrimmed,
    ZoneAlias { from: String, to: String },
}

pub(crate) enum Instead {
    Error {
        status: u16,
        reason: String,
        retry_after: Option<String>,
    },
    OAuthError {
        status: u16,
        error: String,
        subtype: Option<String>,
    },
    Close,
    Redirect(String),
    Json(u16, Value),
    Raw(Vec<u8>),
}

pub(crate) enum AfterApply {
    Close,
    Error { status: u16, reason: String },
}

pub(crate) enum BodyChange {
    NotJson,
    TooLong(usize),
    ContentType(String),
    Gzip,
}

impl Plan {
    pub(crate) fn add(&mut self, action: &FaultAction) {
        self.labels.push(action.label());
        match action {
            FaultAction::Error {
                status,
                reason,
                retry_after,
            } => {
                self.instead.get_or_insert(Instead::Error {
                    status: *status,
                    reason: reason.clone(),
                    retry_after: retry_after.clone(),
                });
            }
            FaultAction::OAuthError {
                status,
                error,
                subtype,
            } => {
                self.instead.get_or_insert(Instead::OAuthError {
                    status: *status,
                    error: error.clone(),
                    subtype: subtype.clone(),
                });
            }
            FaultAction::ApplyThenClose => self.after_apply = Some(AfterApply::Close),
            FaultAction::ApplyThenError { status, reason } => {
                self.after_apply = Some(AfterApply::Error {
                    status: *status,
                    reason: reason.clone(),
                });
            }
            FaultAction::CloseWithoutApplying => {
                self.instead.get_or_insert(Instead::Close);
            }
            FaultAction::Delay(d) => self.delay = Some(self.delay.unwrap_or_default() + *d),
            FaultAction::ApplyLate(t) => self.late = Some(*t),
            FaultAction::NotJson => self.body.push(BodyChange::NotJson),
            FaultAction::TooLong(n) => self.body.push(BodyChange::TooLong(*n)),
            FaultAction::ContentType(c) => self.body.push(BodyChange::ContentType(c.clone())),
            FaultAction::Gzip => self.body.push(BodyChange::Gzip),
            FaultAction::Redirect(to) => {
                self.instead.get_or_insert(Instead::Redirect(to.clone()));
            }
            FaultAction::ExpiredAccessToken => self.expired_token = true,
            FaultAction::Token(t) => self.token.push(t.clone()),
            FaultAction::IdToken(t) => self.id_token.push(t.clone()),
            FaultAction::HideInsertForReads(n) => self.hide_insert_reads = Some(*n),
            FaultAction::CutAttendees => self.cut_attendees = true,
            FaultAction::Before(h) => self.hooks.push(h.clone()),
            FaultAction::Json(status, body) => {
                self.instead
                    .get_or_insert(Instead::Json(*status, body.clone()));
            }
            FaultAction::Raw(bytes) => {
                self.instead.get_or_insert(Instead::Raw(bytes.clone()));
            }
            FaultAction::RewriteDescriptionTrimmed => {
                self.rewrites.push(Rewrite::DescriptionTrimmed);
            }
            FaultAction::RewriteZoneAlias { from, to } => self.rewrites.push(Rewrite::ZoneAlias {
                from: from.clone(),
                to: to.clone(),
            }),
        }
    }
}
