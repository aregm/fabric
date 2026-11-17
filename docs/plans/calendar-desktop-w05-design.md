# CD-W05 design: the Google Calendar provider

**What this is.** The design of work package `CD-W05` of the Calendar Desktop plan: sign-in to Google, token
storage, the Calendar calls, recovery after an unknown outcome, and what the store, the core and the contract must
gain so that the adapter can be attached. Phase 1 builds the adapter as a crate of its own
(`crates/fabric-google-calendar`) and a fake Google server for its tests (`crates/fabric-google-fake`). Phase 1
makes no real Google call, touches no Keychain and edits no other crate.

**How code comments cite it.** In those two crates a number such as `(6.7)` or `section 3.5` names a section of this
document, `PDn` a decision of section 0, `FAn` an assumption of the fake (14.3), `LVn` a check only a live run can
make (15.3), and `g01`, `f01` the tests of section 15. Requests to other owners are `POn` (the provider port), `STn`
(the store), `COn` (the core), `V3-n` (the contract) and `THn` (the rule theory). `RKn` are risks and `OQn` the
owner's questions (section 18).

**Who decides.** Every `OQn` is the owner's; the code runs on the default written after it. The plan and the
contract win wherever they differ from this document. Sections 10 to 13 are requests: their owners decide the form,
and the lead writes the store design. Sections 8 and 6.9 are proposed to the owner as ADR-111 (Proposed), sections
6.6 to 7 as ADR-113 (Proposed).

Date 2026-11-17. Status: **Proposed**. Sections 2 to 9 and 14 to 15 describe what phase 1 builds; they become "as
built" when its pull request is opened.

Abbreviations (paths relative to the repository root):

| Short | Means |
|---|---|
| PLAN | `docs/plans/calendar-desktop-plan.md` |
| W04 | `docs/plans/calendar-desktop-w04-design.md` (the store and the core, as built) |
| PW | `docs/plans/calendar-desktop-prover-wiring-design.md` |
| RUNBOOK | `docs/plans/calendar-desktop-google-runbook.md` (the owner's Google setup; pull request 33) |
| CR | `contracts/desktop/v1/README.md` (the contract, schema version 2) |
| THEORY | `isabelle/Fabric_Broker/Fabric_Broker.thy` |
| PORTS | `crates/fabric-broker-core/src/ports.rs` |
| GC | `crates/fabric-google-calendar/` |
| FAKE | `crates/fabric-google-fake/` |
| G-DOC | Google's documentation as read on 2026-11-17; appendix A lists the pages and what each states |

---------------------------------------------------------------------------------------------------------------------

## 0. Decisions

| # | Topic | Chosen | Why |
|---|---|---|---|
| PD1 | Crates | `crates/fabric-google-calendar` (the adapter) and `crates/fabric-google-fake` (a test double, only ever a dev-dependency). In phase 1 the adapter depends on the contract crate only, not on the core | PLAN 13 names the adapter crate. The port it must implement does not exist yet (section 10); a half implementation of today's `Provider` would be thrown away. |
| PD2 | No Cargo features, no test seam in release | The endpoints are Google's fixed hosts. A loopback endpoint set exists only under `cfg(debug_assertions)` and only for `127.0.0.1` | W04 D11: CI builds with `--all-features`; a release broker can never be pointed at another host. |
| PD3 | HTTP client | `reqwest` 0.13, blocking, no default features; TLS through the operating system (`native-tls`) on macOS and Windows only; no redirects, no proxy, a deadline on every call. Behind one private module (`http`), so the choice can change without touching callers | It is already locked for the demo with the same TLS split, so the lock file gains no HTTP crate. Workers are threads (W04 5.1), so the calls are blocking. Open: OQ7. |
| PD4 | The identity token is checked in full | Signature (RS256, key by `kid` from Google's key set), issuer, audience, expiry, issue time, nonce, subject. Verification only, with the RustCrypto `rsa` crate | PLAN 10.1 asks for the signature. G-DOC allows skipping it for a token read straight from Google over TLS; the plan is stricter and wins. Open: OQ6. |
| PD5 | One sign-in at a time | A new `connect_google` ends a sign-in that is still waiting (`cancelled`) | One loopback listener, one verifier in memory, nothing to mix up. |
| PD6 | The loopback listener | `127.0.0.1` on a port the system picks; redirect `http://127.0.0.1:<port>`; lives at most 300 s; the first request with the right `state` ends it; a fixed page, nothing echoed | PLAN 10.1 (bounded lifetime, single use). G-DOC: loopback is the supported redirect for desktop apps. Open: OQ2. |
| PD7 | The broker opens the browser | `/usr/bin/open <url>`, one argument, built by the adapter. The page never sees the address | The contract's `connect_google` result carries no address, and the webview keeps no capability to open one (PLAN 4.3). Unverified from a child with an empty environment: LV10. |
| PD8 | A partial grant keeps no token | Missing calendar permission: the tokens just issued are revoked at Google and nothing is stored; state `partial_grant` | PLAN 14: no false connected state. G-DOC: installed apps cannot add scopes later, so the next attempt starts from zero anyway. Open: OQ3. |
| PD9 | More permission than asked is refused | A token whose scopes go beyond the five requested ones is revoked and not stored | PLAN 10.2: never the blanket or the ACL scope. The broker must not hold more than it shows. |
| PD10 | Identity is the subject | An account is its Google subject (`sub`). The address is display only and may be absent. Reconnecting an account under another subject is `account_mismatch` | PLAN 10.1; G-DOC: `sub` is "never reused", the address "could change". |
| PD11 | Only the refresh token is stored | In the token store, one item per account. Access tokens live in memory. Client secret, verifier, state, nonce and codes are never written anywhere | The least that survives a restart. A backup of the store holds no token (W04 3.6). |
| PD12 | Token store | A trait with a Keychain implementation (phase 2, macOS) and an in-memory double (phase 1). Keychain service `ai.paracels.fabric.calendar.google`, account `refresh-token-v1.<account id>`. With the test key source the broker uses the in-memory double | PLAN 12 (a distinct Keychain service). No session may touch the Keychain. Open: OQ4. |
| PD13 | One changing request per order | A dispatch order makes at most one request that can change Google (POST, PATCH, DELETE). The adapter never sends it twice. Reads before it are allowed | PLAN 5 (a fresh proof per retry): every repeat is a new order from the core. |
| PD14 | Every change of an existing event is conditional | PATCH and DELETE always carry `If-Match` with the revision the approval names. A change that can notify therefore cannot be applied twice | PLAN 10.3; PLAN 5 "no blind resend of a notification": the second send meets a changed revision. |
| PD15 | PATCH, not get-then-PUT | Field edits, attendee changes and answers are PATCH requests with only the approved members | PLAN 10.3 "preserve untouched fields": PATCH cannot drop a field the adapter does not model. G-DOC does not name PATCH for `If-Match`: LV3, with get-then-PUT as the fallback. Open: OQ11. |
| PD16 | A create carries the reserved id and no attendees | `events.insert` with the id the core reserved (26 base32hex digits) and `sendUpdates=none` | PLAN 11; CR 14 item 21. G-DOC: ids are 5 to 1024 characters of `a-v` and `0-9`. |
| PD17 | After "it no longer holds" in a retry, read before judging | On 409 for a create, or 409, 410, 412 for attempt 2 and later, the adapter reads the event and compares it with the approved change before it reports | An earlier attempt may have been applied and answered with a failure; without the read a done change would be reported as a conflict. |
| PD18 | Outcome classes | Applied, Transient (with Google's wait hint), Rejected (with a contract failure code), NoAnswer. Nothing sent is Transient; sent and unanswered is NoAnswer; the table is 6.10 | W04 D16 decides waits from these classes. Only "sent and unanswered" is an unknown outcome. |
| PD19 | No marker in Google | Fabric writes no private property into an event. "Fabric created it" is the store's record of the create, keyed by account, calendar and event id | PLAN 7 ("ownership tracked locally"). A marker on a shared calendar can be forged by anyone who may write there. |
| PD20 | Absence of a create needs two reads | A create is "not there" only when two reads at least 60 s apart, the first at least 10 s after the attempt ended, both find nothing | G-DOC: Google "cannot guarantee that ID collisions will be detected at event creation time", so a read right after a lost answer may be too early (RK3). |
| PD21 | Calendar and account ids | The broker issues `cal-<32 hex>` per pair (account, Google calendar id) and keeps the pair for the life of the account record. Event ids and revisions are Google's, verbatim (the revision is the etag with its quotes) | Contract owner's ruling T6. A Google calendar id is not a contract `id` (it holds `@`). |
| PD22 | Never `primary` | No request uses Google's keyword `primary`; every calendar is named by the id the calendar list returned | PLAN S01: never silently the primary calendar. |
| PD23 | Reads are bounded and never cut silently | A read that does not fit its bounds (2 500 events, 768 KiB of result) fails with "ask for a shorter window". A free/busy answer with an error for any calendar fails as a whole | PLAN S06: incomplete data is said, not hidden. The contract's result has no "incomplete" member (V3-7). |
| PD24 | Fields are cut twice | A read asks Google only for the members the granted fields need (`fields=`), and the adapter builds the result from the granted fields only, whatever Google returned | PLAN 7 (field minimization); the second cut does not depend on the first. |
| PD25 | Texts that do not fit the contract | A title, place or description longer than the contract's bound is cut at the bound. An attendee address the contract cannot hold is left out of a read, but makes an observation fail | A read may show less; a check of who is notified may never miss a recipient. Loss recorded in RK8. |
| PD26 | A fifth access role | Google's `writerWithoutPrivateAccess` is treated as `writer` until the contract names it | It may write events; the contract has four roles (V3-1). |
| PD27 | No error text from Google is shown or stored | Explanations are fixed sentences of the adapter. Google's `reason` is read against a closed list; its `message` is dropped | W04 D37; a server message is untrusted text and may hold content. |
| PD28 | Held read results | Kept in the encrypted store outside the ledger's payload, released only by a `Disclose` proof, purged on release of a one-request read, on disconnect, and after 24 h | W04 D33; CR 8; PLAN 9.2. Open: OQ5. The lead's store design decides the form (ST4). |
| PD29 | Every Calendar call has a proof | The provider worker carries out only orders that name the proof that permits them: an agent's fresh permit, or a system proof for the broker's own reads | PLAN 1 and C05. The theory lacks one system action (TH1). Open: OQ9. |
| PD30 | ADR-111 and ADR-113 | Written as Proposed with this design | `docs/architecture.md` 19: the broker settles their subjects before `CD-G5`; CD-W04 did the same with ADR-115. |

---------------------------------------------------------------------------------------------------------------------

## 1. Scope

### 1.1 Phase 1 delivers

1. RUNBOOK (pull request 33).
2. This design, ADR-111 and ADR-113 (Proposed).
3. GC: the OAuth code, the token store trait with its in-memory double, the Calendar calls (calendar list,
   free/busy, events get, list, insert, patch, delete), translation between the contract's types and Google's,
   outcome classification, the retry rule and the reconciliation rule as plain functions.
4. FAKE: a Google double that speaks HTTP on `127.0.0.1`, with scripted faults and a request log.
5. Tests of 3 against 4 (section 15). Nothing else is evidence for anything.

### 1.2 Phase 2 (after the broker part of schema version 2 is merged; the lead schedules it)

The port, the store migration, the core's commands and jobs, the Keychain token store, the broker's configuration
and workers (sections 10 to 12, 16). Then the live run with the owner at the Mac, which alone can close `CD-G2b`.

### 1.3 Not in CD-W05

Google People import (its own later package, CD-Q06). Warnings and zone arithmetic (CD-W09). The MCP bridge
(CD-W06). Publishing the Google app and its verification (CD-W11, Q08). A persisted replica of calendars and
incremental sync (PLAN 13: "sync deferred").

---------------------------------------------------------------------------------------------------------------------

## 2. Crates and modules

```
fabric-desktop-contracts  <-  fabric-google-calendar          (phase 2 adds: <- fabric-broker-core, for the port)
                              fabric-google-fake  (dev-dependency of fabric-google-calendar; later of apps/fabric-broker)
```

GC modules:

| Module | Holds |
|---|---|
| `lib` | The public surface (2.2) |
| `endpoints` | Google's fixed addresses; the loopback set of debug builds (5.1) |
| `http` | The one place that talks HTTP: request, response, deadline, bounds, the nothing-sent and sent-unanswered split (5.2) |
| `secret` | `Secret`: a string that is wiped on drop and prints as `[redacted]` |
| `client` | `ClientConfig` and reading Google's client file (3.1) |
| `oauth::{pkce, url, listener, exchange, id_token, scopes, flow}` | Section 3 |
| `tokens` | `TokenStore`, `MemoryTokenStore`, `AccessCache` (section 4) |
| `calendar::{list, freebusy, events, change}` | Section 6 |
| `translate` | Contract types to and from Google's resources; `time` beside it (6.9) |
| `outcome` | Classification of an HTTP result (6.10) and the retry rule (6.11) |
| `reconcile` | The decision after an unknown outcome (section 7) |

FAKE modules: `server` (a small HTTP/1.1 server on `std::net`), `state`, `oauth`, `calendar`, `faults`, `log`,
`keys` (a fixed RSA test key, written as numbers, not as a PEM file).

### 2.1 Dependencies

GC: the contract crate, `serde`, `serde_json`, `sha2`, `base64`, `getrandom`, `zeroize`, `reqwest` (PD3), `rsa`
(PD4, verification only). Dev: FAKE. FAKE: `serde_json`, `sha2`, `base64`, `rsa` (signing with the test key).
`reqwest`, `base64`, `sha2`, `getrandom` and `zeroize` are locked already; `rsa` and what it needs are new.

Forbidden, checked by `crates/fabric-google-calendar/tests/fitness.rs` (a manifest scan, as W04 section 2):

| Crate | Must not depend on |
|---|---|
| GC | `fabric-storage`, `fabric-policy`, `fabric-broker-policy`, `fabric-token`, `fabric-demo`, `fabric-core`, Tauri, FAKE outside `[dev-dependencies]` |
| every workspace member | FAKE outside `[dev-dependencies]` |
| FAKE | every `fabric-*` crate except the contract crate |

Platforms: both crates build and test on macOS, Linux and Windows. TLS is linked on macOS and Windows only; on
Linux GC can reach only the loopback double, which is all a test needs (the broker is a macOS product).

### 2.2 Public surface of GC (phase 1)

- `ClientConfig`, `Endpoints`, `Clock` (`SystemClock`), `Secret`.
- `oauth::Flow::start(...) -> PendingFlow` and `PendingFlow::wait(...) -> ConnectOutcome` (3.2); `oauth::refresh`,
  `oauth::revoke`; `BrowserOpener` (`SystemBrowser` on macOS, a recording double in tests).
- `tokens::{TokenStore, MemoryTokenStore, TokenStoreError, AccessCache}`.
- `Google::new(endpoints, client, store, clock)` with `list_calendars`, `free_busy`, `read_events`, `observe`,
  `change`, `reconcile_read`.
- `outcome::{ChangeOutcome, ReadFailure, RetryPolicy, RetryDecision, next_attempt}`.
- `reconcile::{Effect, ReadBack, Verdict, decide, earliest_read_ms}`.
- No function takes an address, a path or a header from its caller. Callers pass typed ids and contract values
  (PLAN 10.2: never an arbitrary Google URL).

---------------------------------------------------------------------------------------------------------------------

## 3. Sign-in (OAuth 2.0 for a desktop app, with OpenID Connect)

### 3.1 The client

`ClientConfig { client_id, client_secret: Option<Secret> }`. `ClientConfig::from_file(path)` reads the file Google's
console gives for a Desktop app client: a JSON object whose member `installed` holds `client_id` and
`client_secret`. Any other shape is an error that names the path and nothing of the content. The adapter sends the
secret in the token and refresh requests when it has one. G-DOC marks it optional and calls a desktop client public;
whether Google accepts a Desktop app client without it is LV2.

Where the broker gets the path is CO10. The secret of a desktop client is not a security boundary (PLAN 10.1); it
is still never logged, never in an error, never in the store.

### 3.2 The flow

```
owner: connect_google ──> core: account awaiting_consent (commit, notice, answer)
                           └─> job Connect ──> adapter
adapter: verifier, state, nonce (32 random bytes each, base64url, 43 characters)
         listener on 127.0.0.1:<port>          (3.3)
         browser <── authorization address     (3.2.1)
                 ... the person signs in and decides, in the system browser ...
listener <── GET /?state=..&code=..   or   ?state=..&error=..
adapter: code + verifier ──> token endpoint    (3.4)
         identity token checks                 (3.5)
         granted scopes                        (3.6)
         refresh token ──> token store         (section 4)
         ConnectOutcome ──> core ──> account state (3.7)
```

**3.2.1 The authorization address.** `https://accounts.google.com/o/oauth2/v2/auth` with exactly these parameters:

| Parameter | Value |
|---|---|
| `client_id` | from the client file |
| `redirect_uri` | `http://127.0.0.1:<port>` (no path) |
| `response_type` | `code` |
| `scope` | `openid email https://www.googleapis.com/auth/calendar.calendarlist.readonly https://www.googleapis.com/auth/calendar.freebusy https://www.googleapis.com/auth/calendar.events` |
| `code_challenge` | base64url of SHA-256 of the verifier |
| `code_challenge_method` | `S256` |
| `state` | the state |
| `nonce` | the nonce |
| `prompt` | `select_account`, only for a new account |
| `login_hint` | the stored subject, only when reconnecting a known account (G-DOC: an address or a `sub`; the subject keeps the address out of the URL) |

The address is opened with `BrowserOpener::open`. `SystemBrowser` (macOS) runs `/usr/bin/open` with that one
argument and refuses any address that does not start with the authorization endpoint of its `Endpoints`. If the
browser cannot be started the flow ends `Cancelled(BrowserNotOpened)`. Sign-in never happens in the app's webview
(PLAN 10.1).

**3.2.2 Results.** `ConnectOutcome` is one of:

| Outcome | When |
|---|---|
| `Connected { subject, email, scopes }` | Everything below held and the refresh token is in the token store |
| `ConsentDenied` | The callback carried `error=access_denied` |
| `PartialGrant { missing }` | The token lacks one of the three calendar scopes (PD8) |
| `AccountMismatch` | A reconnect signed in as another subject, or a new connect as a subject already connected (3.7) |
| `Cancelled(reason)` | `TimedOut`, `Superseded` (PD5, or the broker quits), `BrowserNotOpened`, `GoogleError` (any other `error`), `Network` (the token or key request failed), `IdentityCheckFailed` (3.5), `UnexpectedScope` (PD9), `NoRefreshToken`, `TokenStore` |

A flow that ends in anything but `Connected` stores nothing. If Google had already issued tokens, the adapter asks
Google to revoke them (one attempt, result ignored) and wipes them.

### 3.3 The loopback listener

- Binds `127.0.0.1:0`, IPv4 only, never another interface. One connection at a time.
- Lives at most 300 s from `Flow::start`, or until the flow is superseded (a flag it reads at least every 250 ms).
- Reads at most 8 KiB of request head, with a 5 s read limit per connection. No body is read.
- Accepts only `GET` with the path `/` and a `Host` of `127.0.0.1:<port>` (a guard against a web page that rebinds
  a name to the loopback address). Anything else is answered 400 or 404 and does not end the flow. After 50 such
  requests the listener gives up and the flow ends `Cancelled(TimedOut)`.
- A request for `/` must carry `state` exactly once. It is compared with the flow's state in constant time. A wrong
  or missing state is answered 400 and does not end the flow: another local program must not be able to end or
  steer a sign-in by guessing the port.
- The first request with the right state ends the listener, whatever else it carries: `code` exactly once, or
  `error` exactly once; both or neither is `Cancelled(GoogleError)`. The listener closes before the token request
  is made, so a code can be presented once.
- The answer is one fixed page: "You can close this window and return to Fabric Calendar." No parameter is echoed.
  Headers: `Cache-Control: no-store`, `Content-Type: text/html; charset=utf-8`, `Connection: close`.

### 3.4 The token request

`POST https://oauth2.googleapis.com/token`, form encoded: `grant_type=authorization_code`, `code`, `code_verifier`,
`client_id`, `client_secret` (when the client file has one), `redirect_uri` (the same string as in 3.2.1). One
attempt, 30 s. It is not repeated: a code is single use, and a request that may have reached Google has used it.

The answer must be JSON with `access_token`, `expires_in`, `token_type` equal to `Bearer` without regard to case,
`scope`, `id_token` and `refresh_token`. Unknown members are ignored (G-DOC). A missing `refresh_token` is
`Cancelled(NoRefreshToken)`; G-DOC says installed apps always get one. An error answer is `Cancelled(GoogleError)`.

### 3.5 The identity token

The `id_token` is a JWT. Every check must hold; a failure is `Cancelled(IdentityCheckFailed)` and the reason is a
fixed label, never a part of the token.

1. Three base64url parts; header and payload are JSON objects; the whole token is at most 16 KiB.
2. Header `alg` is exactly `RS256`. `none`, `HS256` and every other value are refused before any key is looked at.
3. Header `kid` names a key of Google's key set (`https://www.googleapis.com/oauth2/v3/certs`), with `kty` `RSA`,
   `alg` absent or `RS256`, `use` absent or `sig`, and a modulus of 2048 to 8192 bits. The key set is fetched over
   TLS, kept in memory for the `max-age` of its `Cache-Control` (not under 5 minutes, not over 24 hours), and
   fetched again once when a `kid` is unknown, at most once a minute.
4. The signature over `header.payload` verifies with that key (RSASSA-PKCS1-v1_5, SHA-256).
5. `iss` is `https://accounts.google.com` or `accounts.google.com`.
6. `aud` is the client id (a string, or an array holding only it). `azp`, when present, is the client id.
7. `exp` is later than now minus 60 s. `iat` is not later than now plus 60 s and not earlier than now minus 600 s.
8. `nonce` equals the flow's nonce, compared in constant time.
9. `sub` is 1 to 255 ASCII characters.
10. The address is taken only when `email_verified` is `true` and `email` fits the contract's `email` after its
    domain is put in lower case. Otherwise the account has no address to show (`email: null`), and still connects.

The access token is not used to ask Google who the account is; the identity token is the only source.

### 3.6 Granted scopes

The `scope` of the token answer is split at spaces. `email` and `https://www.googleapis.com/auth/userinfo.email`
count as the same scope. Then:

| Granted | Outcome |
|---|---|
| `openid`, the address scope and all three calendar scopes, nothing else | continue to `Connected` |
| any of the three calendar scopes missing | `PartialGrant { missing }`: revoke, store nothing (PD8) |
| `openid` missing (then there is no usable identity token) | `Cancelled(IdentityCheckFailed)` |
| the address scope missing | continue; the account has no address |
| anything beyond the five | `Cancelled(UnexpectedScope)`: revoke, store nothing (PD9) |

G-DOC: a person can untick single scopes on the consent screen, and the app "must verify which scopes were actually
granted". The granted list is given to the core (ST1) and checked again by the core before it calls an account
connected.

### 3.7 How the outcomes map to the contract's account states

| Contract state | Set when | Tokens held |
|---|---|---|
| `awaiting_consent` | `connect_google` committed; the flow runs | the old one, if the account had one |
| `connected` | `Connected`, and for a reconnect the subject is the stored one | yes |
| `consent_denied` | `ConsentDenied` | no change |
| `partial_grant` | `PartialGrant` | none of this attempt |
| `cancelled` | `Cancelled(_)`; also every `awaiting_consent` account at the next start of the broker (CO3) | no change |
| `account_mismatch` | `AccountMismatch` | none of this attempt |
| `reauthentication_required` | Google refused the refresh token (`invalid_grant`), or the token store holds none for a connected account (3.8) | deleted |
| `disconnected` | `disconnect_google` | deleted |

Only `connected` counts as connected for a proof (THEORY `Account_connected`). Every other state blocks every agent
action on that account. An account that was `connected` and whose reconnect ends in `consent_denied`, `cancelled`,
`partial_grant` or `account_mismatch` is not connected afterwards, although its old token may still work: the owner
asked to change the connection and it did not succeed, so the broker stops (fail closed). The next successful
`connect_google` restores it. Its grants that have not ended stay; they are usable again only when the account is
`connected` again, which is always the owner's own act.

A new connect (`account_id: null`) whose subject is already an account ends `AccountMismatch` for the new record
and leaves the existing account untouched. So one Google account is never two accounts. Unfinished records do not
pile up: CO2.

### 3.8 Refresh

Before a Calendar call the adapter takes the account's access token from `AccessCache`. When there is none, or it
ends within 60 s, it refreshes: `POST` to the token endpoint with `grant_type=refresh_token`, `refresh_token`,
`client_id`, `client_secret` (when present). One attempt, inside the call's own deadline.

| Answer | Result for the call |
|---|---|
| 200 with `access_token` and `expires_in` | the call goes on. A `scope` member is checked as in 3.6; fewer calendar scopes than before is treated as `invalid_grant` |
| 400 or 401 with `error` `invalid_grant` | the stored refresh token is deleted; the call ends `AuthLost` (6.10) |
| any other answer, or none | Transient |

G-DOC lists why a refresh token stops working: the person removed access, six months without use, the limit of 100
tokens per account and client, and in testing mode seven days after consent. All of them arrive as `invalid_grant`
and all of them mean `reauthentication_required`. A 401 from a Calendar call drops the cached access token and is
Transient; the next attempt refreshes.

### 3.9 Revocation

`oauth::revoke(account)`: `POST https://oauth2.googleapis.com/revoke`, form encoded, `token=<refresh token>`. One
attempt, 30 s. 200 is `Revoked`. 400 with `invalid_token` is `Revoked` too (Google no longer knows it). Anything
else is `NotConfirmed`. The caller deletes the stored token afterwards in every case (CO4): a token that could not
be revoked is not kept around for a disconnected account; the owner is told to remove the access in his Google
account (V3-3).

### 3.10 What never happens

No embedded sign-in. No password, code or token in an argument, the environment, standard output, standard error,
a log line or an error value. No incremental authorization (G-DOC: not supported for installed apps): another scope
means connecting again. No request to an address that came from a caller, a file or Google's answer; the endpoints
are constants. No token sent to a host other than the one it is for (5.1).

---------------------------------------------------------------------------------------------------------------------

## 4. Token storage

```rust
pub trait TokenStore: Send {
    fn read(&self, account: &Id) -> Result<Option<Secret>, TokenStoreError>;   // the refresh token
    fn write(&mut self, account: &Id, refresh_token: &Secret) -> Result<(), TokenStoreError>;
    fn delete(&mut self, account: &Id) -> Result<(), TokenStoreError>;         // deleting what is not there is Ok
}
pub enum TokenStoreError { Refused, Locked, Failed { status: i32 } }           // fixed labels only
```

- `MemoryTokenStore` (phase 1, and every run with `FABRIC_BROKER_KEY_SOURCE=test`): a map behind a mutex, shared by
  the sign-in worker and the provider worker. Its tokens end with the process, so after a restart every account of
  a test run is `reauthentication_required`. That is what a test of that state needs.
- `KeychainTokenStore` (phase 2, macOS only): one generic password per account. Service
  `ai.paracels.fabric.calendar.google`, account `refresh-token-v1.<account id>`; the value is the refresh token's
  bytes. The account id is the broker's, never the address. Status mapping as W04 3.2: item not found is `None`;
  `errSecUserCanceled` and `errSecAuthFailed` are `Refused`; `errSecInteractionNotAllowed` is `Locked`; any other
  status is `Failed`. Its one real test is `#[ignore]`d, uses the service `ai.paracels.fabric.calendar.google.test`
  and is the owner's manual check (15.3 LV9). It is not written in phase 1.
- `AccessCache`: account id to `(access token, end time)`, memory only, shared the same way.
- A token-store failure during a call: `Locked` is Transient, `Refused` and `Failed` are Rejected with
  `account_authentication_required` and the fixed sentence "Fabric could not read this account's sign-in from the
  Keychain."
- The store's database keeps a token reference (the Keychain account name), never a token (ST1). A database whose
  account says `connected` while the token store has no item (a restored backup, a reset Keychain) becomes
  `reauthentication_required` at the first call.

---------------------------------------------------------------------------------------------------------------------

## 5. Transport

### 5.1 Endpoints

```rust
pub struct Endpoints { authorization, token, revocation, keys, calendar_api }   // private fields
impl Endpoints {
    pub const fn google() -> Endpoints;                      // the five constants below
    #[cfg(debug_assertions)]
    pub fn loopback(port: u16) -> Endpoints;                 // http://127.0.0.1:<port>/..., nothing else
}
```

| Use | Address | What is sent there |
|---|---|---|
| authorization | `https://accounts.google.com/o/oauth2/v2/auth` | nothing by the broker; the browser goes there |
| token | `https://oauth2.googleapis.com/token` | code, verifier, refresh token, client secret |
| revocation | `https://oauth2.googleapis.com/revoke` | refresh token |
| keys | `https://www.googleapis.com/oauth2/v3/certs` | nothing |
| Calendar | `https://www.googleapis.com/calendar/v3` | the access token, as `Authorization: Bearer` |

The addresses are the ones G-DOC's discovery document shows. They are constants: the discovery document is not
read at run time, so no answer of a server can move a token to another host. If Google changes an address the
broker stops working with a clear error and needs an update; that is the safe direction.

### 5.2 Rules of `http`

- HTTPS only for `Endpoints::google()`. Plain HTTP only for `Endpoints::loopback`, which a release build does not
  have.
- No redirect is followed. A 3xx is an answer like any other (6.10: Rejected).
- No proxy, whatever the environment says. TLS 1.2 or later, the system's trust store, no pinning.
- Every call has a deadline: the earlier of the order's deadline and now plus 30 s (PLAN 5). Connecting has at most
  10 s of it.
- A response body is read up to a bound and no further: 64 KiB for the token, revocation and error answers,
  256 KiB for the key set and for one event, 4 MiB for a list page. More is treated as no usable answer.
- JSON answers must say `application/json`. They are read into `serde_json::Value` first and then into lenient
  structs: unknown members are ignored, missing optional members are absent, a member of the wrong type is an error.
- Path parts and query values are percent-encoded: everything except `A-Z a-z 0-9 - . _ ~`. A calendar id such as
  `en.usa#holiday@group.v.calendar.google.com` and an event id an agent wrote cannot change the path.
- `User-Agent: FabricCalendar/<crate version>`. No other identifying header.
- The result separates three cases, which 6.10 needs: nothing was sent (name lookup, connect or TLS failed, or the
  deadline had passed); a complete answer arrived; something was sent and no complete answer arrived.

---------------------------------------------------------------------------------------------------------------------

## 6. Calendar operations

### 6.1 Common rules

Every operation takes the broker's account id and Google's calendar id (the core resolves the broker's calendar id
first, section 8), a deadline, and for a change the approved contract `Action`. Every operation uses the token of
the named account only. No operation names `primary` (PD22). Every result is built from typed values; nothing
Google wrote is passed through as text except event content in a read result or a before-image.

### 6.2 Calendar list

`GET /users/me/calendarList?maxResults=250&minAccessRole=freeBusyReader[&pageToken=..]`, every page, at most 20
pages. Deleted and hidden entries are not asked for.

Result per entry: Google's `id`; the name (`summaryOverride`, else `summary`, cut to 200 characters, "(no name)"
when empty); `primary`; `timeZone`; the access role:

| Google `accessRole` | Contract `calendar_access` | Writable |
|---|---|---|
| `owner` | `owner` | yes |
| `writer` | `writer` | yes |
| `writerWithoutPrivateAccess` | `writer` (PD26, V3-1) | yes |
| `reader` | `reader` | no |
| `freeBusyReader` | `free_busy_reader` | no |
| anything else | the entry is left out | |

The core turns the list into the account's calendars, issues ids (section 8), keeps `selected` and `agent_drafts`
as the owner set them, and treats a lost entry or a lower role as a reduction of authority (CO7).

### 6.3 Free/busy

`POST /freeBusy` with `{"timeMin", "timeMax", "timeZone": "UTC", "items": [{"id": <Google calendar id>}, ...]}`, at
most 50 calendars (the contract's bound and Google's). The answer has `calendars.<id>.busy[]` and
`calendars.<id>.errors[]`.

- An `errors` entry for any asked calendar, or a missing calendar, fails the whole read (PD23) with the fixed
  sentence "Google could not report busy times for one of the calendars."
- Each busy period becomes `BusyInterval { calendar_id (the broker's), start, end }` in UTC with milliseconds, cut
  to the asked window. Periods are ordered by calendar (the order asked) and then by start.
- `observed_at` is the adapter's clock when the answer arrived.

### 6.4 Reading events

Per calendar: `GET /calendars/<id>/events` with `singleEvents=true`, `orderBy=startTime`, `showDeleted=false`,
`timeMin`, `timeMax` (the window; G-DOC: `timeMin` bounds an event's end, `timeMax` its start, both exclusive, so
this is "every event that overlaps the half-open window"), `maxResults=250`, `maxAttendees=2000`, `fields=` (below)
and `pageToken` for later pages.

- Recurring events arrive as their occurrences (PLAN 10.3: provider-expanded occurrences). A recurring master is
  never returned and never treated as one event.
- Bounds (PD23): at most 2 500 events over all asked calendars and at most 768 KiB of canonical result. Past
  either, the read fails: "The window holds more events than one answer can carry. Ask for a shorter window."
- One page is one call. `read_events` returns either the finished result or `More { next }`, so that the core can
  check authority again before each further page (PLAN 8: "every later handoff, including ... paginated reads,
  rechecks authority"; PO3).
- An item with `status` `cancelled` is skipped.

`fields=` asks for `nextPageToken`, `timeZone` and, per item, always `id,etag,status,recurringEventId`, and then by
granted field:

| Granted field | Members asked | In `EventView` |
|---|---|---|
| `time` | `start,end,originalStartTime` | `time`; `occurrence` (instance id and original start) for an occurrence |
| `title` | `summary` | `title` (empty when Google has none) |
| `location` | `location` | `location` |
| `description` | `description` | `description` |
| `attendees` | `attendees(email,responseStatus,self)` | `attendees` |
| `organizer` | `organizer(email)` | `organizer` |
| `own_response` | `attendees(self,responseStatus)` | `own_response`: the status of the entry marked `self` |

A field that was not granted is `null` in the view even if Google sent it (PD24). `occurrence` holds the original
start, which is a time, so it is disclosed only with `time`; without it an occurrence looks like a single event and
cannot be answered one by one. `fabric_created` is not the adapter's to know: the caller passes it per event id
(PD19, CO6).

### 6.5 Observing one event

`observe(account, calendar, event id)`: `GET /calendars/<id>/events/<event id>`. It is the broker's own read of
the event an action names, made before each proof on such an action (PO2), and gives:

| Member | From Google's event |
|---|---|
| `event_id`, `revision` | `id`, `etag` (verbatim) |
| `recurring` | `recurrence` present, or `recurringEventId` present |
| `occurrence_start_ms` | `originalStartTime.dateTime` of an occurrence, as UTC milliseconds |
| `default_type` | `eventType` absent or `default` |
| `timed` | `start.dateTime` present (an all-day event has `start.date`) |
| `invitation` | an attendee entry with `self: true` exists and `organizer.self` is not `true` |
| `attendees` | the `email` of every attendee entry without `self: true`, as written, domain in lower case |
| `before` | an `EventView` with every content field, for the owner's card only (`private.before`) |

Results: `Found`, `Absent` (404, 410, or `status` `cancelled`), or a read failure (6.10). An attendee address that
does not fit the contract's `email` makes the observation fail with `invalid_arguments` (PD25): the check of who is
notified must see everyone.

### 6.6 Creating an event (`fabric_calendar_propose_create`)

`POST /calendars/<id>/events?sendUpdates=none` with the body

```json
{ "id": "<reserved id>", "summary": "<title>",
  "start": { "dateTime": "<date_time>", "timeZone": "<zone>" },
  "end":   { "dateTime": "<date_time>", "timeZone": "<zone>" },
  "description": "<description, when given>", "location": "<location, when given>" }
```

and nothing else: no attendees (CR 14 item 21), no recurrence, no event type (the default type). The order must
carry a reserved id of 5 to 1024 characters of `a-v` and `0-9`; otherwise it is Rejected before any request.

| Answer | Outcome |
|---|---|
| 200 with an event whose `id` is the reserved one | Applied, with `(id, etag)` and whether the returned event equals the approved one (6.12) |
| 409 (`duplicate`) | read `GET .../events/<reserved id>` (PD17): found and not cancelled is Applied, with the comparison; cancelled is Rejected `provider_rejected` ("an event with this id existed and was deleted"); not found is NoAnswer |
| others | 6.10 |

### 6.7 Changing an existing event

All three are `PATCH /calendars/<id>/events/<event id>?sendUpdates=<approved>` with `If-Match: <approved
revision>` (PD14, PD15). `sendUpdates` is always written: `all`, `externalOnly` or `none`.

| Tool | Reads first | Body |
|---|---|---|
| `propose_update` | no | only the members that are set: `summary`; `start` and `end` as `{"dateTime", "timeZone", "date": null}`; `description`; `location` |
| `propose_invitation` | the event (GET) | `{"attendees": [...]}`: the approved addresses in the approved order; an address that was an attendee keeps its whole entry, a new one is `{"email": ...}`; the entry marked `self` is kept as it was |
| `propose_response` | the event (GET) | `{"attendeesOmitted": true, "attendees": [{"email": <the entry marked self>, "responseStatus": "accepted" or "declined"}]}` |

- The read, where there is one, must return the approved revision; another revision is Rejected
  `revision_conflict` without a PATCH. For an answer to one occurrence the path names the instance id, and the
  instance's `originalStartTime` must be the approved instant; otherwise `revision_conflict` (CD-Q05b: instance
  id, original start and revision are bound together).
- An answer needs an entry marked `self`; without one it is Rejected `provider_rejected` ("this account is not
  invited to the event").
- 200 with the event is Applied, with `(id, new etag)` and the comparison (6.12).
- 412 is Rejected `revision_conflict` on attempt 1. On a later attempt the adapter reads the event first (PD17):
  if it now equals the approved change, the earlier attempt was applied, and the outcome is Applied.

### 6.8 Deleting an event (`fabric_calendar_propose_delete`)

`DELETE /calendars/<id>/events/<event id>?sendUpdates=<approved>` with `If-Match: <approved revision>`.

| Answer | Outcome |
|---|---|
| 204 or 200 | Applied, no event |
| 410 | Applied, no event: it is deleted (G-DOC: "no further action is necessary") |
| 404 on attempt 2 or later | Applied, no event (PD17: the earlier attempt may have done it) |
| 404 on attempt 1 | Rejected `invalid_arguments` ("no such event") |
| 412 | Rejected `revision_conflict` |

That only an event Fabric created may be deleted is the theory's rule (`Fabric_created`) on the store's record
(PD19); the adapter does not decide it. Whether the deletion is in effect is confirmed by a read afterwards
(section 7; PLAN 10.3 CD-Q05a).

### 6.9 Translation (the time model; ADR-111)

| Contract | Google | Rule |
|---|---|---|
| `event_time { date_time, zone }` | `{ "dateTime", "timeZone" }` | Written as given. Read: a `Z` becomes `+00:00`, a fraction of a second is dropped, a missing `timeZone` is the calendar's (`timeZone` of the list answer) |
| all-day | `{ "date" }` | Read only, as `start_date` and `end_date` (the end exclusive, as Google has it). Never converted to an instant. Never written |
| `window` | `timeMin`, `timeMax` | The contract's timestamps as they are (RFC 3339, UTC) |
| busy `start`, `end` | RFC 3339 | Converted to UTC with milliseconds |
| `revision` | `etag` | Verbatim, quotes included; sent back verbatim in `If-Match` |
| `event_id`, `instance_id` | `id` | Verbatim |
| `send_updates` `all`, `external_only`, `none` | `sendUpdates` `all`, `externalOnly`, `none` | |
| `response` `accepted`, `declined` | `responseStatus` | A read also knows `needsAction` and `tentative` |

The adapter does no zone arithmetic. It parses RFC 3339 to an instant only to compare instants (6.12, section 7)
and to cut busy periods to the window. Whether an offset agrees with its zone is the broker's check (CR 5; CD-W09).

### 6.10 Outcome classes (PD18)

For a change:

| What happened | Outcome |
|---|---|
| Nothing was sent (5.2), or the deadline had passed before sending | Transient |
| The request was sent and no complete, readable answer arrived | **NoAnswer** |
| 2xx whose body cannot be read as an event (for insert and PATCH) | NoAnswer |
| 2xx as 6.6 to 6.8 | Applied |
| 400 | Rejected `invalid_arguments` |
| 401 | Transient; the cached access token is dropped (3.8) |
| the refresh before the call ended `invalid_grant` | Rejected `account_authentication_required`, and `AuthLost` for the account |
| 403 with reason `rateLimitExceeded` or `userRateLimitExceeded`; 429 | Transient, with `Retry-After` in whole seconds when present (at most 120 s) |
| 403 with any other reason | Rejected `provider_rejected` ("Google refused the change for this account"), and `AccessChanged` for the account, so the core lists its calendars again |
| 404 | Rejected `invalid_arguments` (6.8 for a delete) |
| 409, 410, 412 | 6.6 to 6.8 |
| 5xx | Transient |
| any other status, a 3xx among them | Rejected `provider_rejected` |

For a read (6.2 to 6.5, and the reads of section 7) there is no NoAnswer: an unanswered read is Transient. 404 on a
calendar is Rejected `provider_rejected`; the other rows hold.

`AuthLost` and `AccessChanged` are side results beside the outcome (PO6).

### 6.11 The retry rule (CD-Q11e)

`next_attempt(policy, retries_made, first_attempt_at_ms, now_ms, hint_ms) -> RetryDecision` with
`RetryPolicy::PLAN { max_retries: 3, backoff_ms: [2000, 4000, 8000], budget_ms: 120_000, call_ms: 30_000 }`:
wait `max(backoff_ms[retries_made], hint_ms)` when `retries_made < 3` and the wait ends inside the budget; otherwise
give up. It is the rule the core already applies to a dispatch (W04 D16), written once more as a plain function so
that the reads and jobs the core adds use the same numbers (CO9). The adapter itself never waits and never repeats
(PD13).

"No blind resend of a notification" is structural (PD14): the only requests that can notify are the PATCH and
DELETE of 6.7 and 6.8; each is conditional on the approved revision; a first send that was applied changed the
revision; so a second send cannot be applied and cannot notify again. Test g38 holds every such request to it.

### 6.12 Comparing what Google returned with what was approved

`matches(action, event) -> bool`, used in 6.6, 6.7 and section 7:

- Create: `summary` equals the title; `start` and `end` are the approved instants with the approved zones;
  `description` and `location` equal the approved ones (absent and empty are equal); no attendees.
- Update: each member the action sets, compared as above. Members it does not set are not compared.
- Invitation: the set of attendee addresses, without the entry marked `self`, equals the approved set (local part
  exact, domain without regard to case).
- Response: the entry marked `self` has the approved status.

---------------------------------------------------------------------------------------------------------------------

## 7. Reconciliation (out of `outcome_unknown`; ADR-113)

A change whose outcome is unknown is never sent again. The broker reads and decides. The read is the broker's own
action (`System_reconcile account calendar event`, THEORY) and needs its proof (PD29).

`reconcile::decide(effect, read_back) -> Verdict`, a pure function. `effect` is the tool, the approved action, the
reserved id of a create, the approved revision of a change, and how many reads came before. `read_back` is `Found`
(the event, its revision), `Absent`, or the first-read time.

| Tool | Read back | Verdict | Lifecycle (CR 9) |
|---|---|---|---|
| create | found, not cancelled | `Applied` with `(id, revision)` and whether it matches (6.12) | `reconciling -> applied` |
| create | absent, first read | `ReadAgain` no sooner than 60 s later (PD20) | stays `reconciling` |
| create | absent, second read | `Absent` | `reconciling -> absence_confirmed` |
| update, invitation, response | found, revision is the approved one | `Absent`: the change did not happen | `absence_confirmed` |
| update, invitation, response | found, another revision, and it matches (6.12) | `Applied` | `applied` |
| update, invitation, response | found, another revision, and it does not match | `NeedsOwner` ("the event changed, and Fabric cannot tell whether its change is part of it") | `needs_owner_resolution` |
| update, invitation, response | absent | `Absent` (there is no event that carries the change) | `absence_confirmed` |
| delete | absent | `Applied`, no event | `applied` |
| delete | found | `Absent`: it was not deleted | `absence_confirmed` |

- `earliest_read_ms(attempt_finished_ms)` is the attempt's end plus 10 s: Google may finish a request the broker
  stopped waiting for.
- A read that fails is Transient or Rejected by 6.10; the core repeats it by 6.11 and after the budget moves the
  request to `needs_owner_resolution` (CO8).
- Notification: for a change with `send_updates` other than `none` that ends `Applied` through this path, whether
  Google sent the messages is not known. The verdict says `notification: Unknown`; nothing is sent again (PLAN 11).
  The owner sees it (V3-4).
- `absence_confirmed` ends the request. The agent may ask again; that is a new request with a new proof and, for a
  create, a new reserved id. The two reads of PD20 are what stands between that and a duplicate.

`applied -> reconciled`: for an insert or a PATCH the answer itself is Google's statement of the event after the
change. When it matches (6.12) the adapter reports `confirmed: true` and the core may move on to `reconciled`
without another read. For a delete (no body), or when the answer does not match, the core runs one reconcile read
and records what it finds; the request then ends `reconciled` with the observation in its details, matching or not
(CO8). Recommended; the lead may prefer a read after every change, which costs one proof and one call more each.

---------------------------------------------------------------------------------------------------------------------

## 8. Ids (ADR-111)

| Thing | Id | Issued by | Kept |
|---|---|---|---|
| Account | the contract's `account_id` (`account-<32 hex>`) | the broker, at `connect_google` | with the Google subject (ST1) |
| Calendar | `cal-<32 hex>` | the broker, when a calendar list first shows the pair (account, Google calendar id) | the pair, for the life of the account record, also while Google does not list the calendar (ST2) |
| Event | Google's `id` | Google, or the broker for a create (the reserved id) | in requests, effects, observations |
| Occurrence | Google's instance `id` with its `originalStartTime` | Google | as an event |
| Revision | Google's `etag` | Google | with the observation |

- The same Google calendar reached through two accounts is two calendars with two ids (PLAN 10.1: everything is
  namespaced per account).
- An agent and the page never see a Google calendar id. An order to the adapter carries it; the core resolves it in
  the transaction that makes the order, under the account the action names. A calendar id of another account does
  not resolve (PLAN 14: account A against account B).
- Google is the authority for what an event is. Fabric keeps no copy of a calendar: only observations tied to
  requests, held read results, and its own records of what it created. Fabric is the authority for grants,
  authorizations, effects and "Fabric created this".

---------------------------------------------------------------------------------------------------------------------

## 9. Held read results (W04 D33; question 10 of W04 11.2)

What the plan and the contract fix: a status never carries calendar content (CR 8); reading a result is itself an
action with its own proof (PLAN 9.2); a one-request read grant permits one release.

Design (the form is the lead's, ST4, CO5):

1. A read request runs the pipeline to a fresh permit like a change. The handoff of the read order is its dispatch
   point: no read reaches Google without a consumed authorization (PLAN 14, "a disallowed calendar ... never
   reaches provider").
2. The adapter's answer goes to the core and nowhere else. The core stores it as the request's held result in the
   encrypted store and moves the request on. No part of it goes into a status, a notice or a redacted detail.
3. `fabric_request_result` is a request of its own. Its proof is on `Disclose(a)`, where `a` is the read. THEORY
   then requires `Result_held` and, for a read, the authority of every row of `a` against the grants as they are
   now (`releasing_a_read_needs_its_authority`).
4. Only a fresh permit on that `Disclose` releases the result, in that request's tool outcome. The release is
   recorded; a one-request grant is then used (`GrantFact.used`), and its held result is purged.
5. A held result is purged also when its account is disconnected, and 24 h after it was stored (OQ5).

**Question 10** ("polling a completed read after revocation discloses nothing"). With 1 to 5: after the grant is
revoked, a status poll returns the state and nothing else, by the contract's type; a `fabric_request_result` gets a
`Disclose` proof that does not permit, because `Authority` no longer holds, so nothing is released and the held
result stays where it is. The test is c-level, in the core, against FAKE as the provider (15.2 row 7): read to
completion, revoke, poll, ask for the result, and assert that no byte of the result's marker left the broker. It
can be written when ST4 and CO5 exist. The adapter's part is 2, and test g31 holds it: the adapter has no channel
to an agent.

---------------------------------------------------------------------------------------------------------------------

## 10. What the provider port needs (request to the lead; PORTS)

Today `Provider::dispatch(&DispatchOrder) -> AttemptDone` only. Proposed, names free:

| # | Change | Why |
|---|---|---|
| PO1 | `DispatchOrder` gains `provider_calendar_id: String` (Google's id, resolved by the core) | The adapter must not read the store (W04 5.1) and must not be given an id by an agent |
| PO2 | An observe order: `(account, provider calendar id, event id)` under a system proof, answered `Found(ObservedEvent without fabric_created, before: EventView)`, `Absent` or a failure. The core runs it before every proof on an action that names an event, fills `TrustedFacts.event` (adding `fabric_created` from its own records) and `Fence.event_revision` | THEORY `Event_known`, `Revision_current`, `Event_supported`, `Invitation_target`; it replaces the practice hook (PW 4.4) |
| PO3 | A read order for the three read tools, one page per order, answered `Done(ToolResult)`, `More(next)` or a failure; the core checks authority again before each page | D33 lifted; PLAN 8 |
| PO4 | A reconcile order: the effect and its target, under a system proof, answered with the read-back; the core applies `reconcile::decide` | Section 7 |
| PO5 | A calendar-list order per account, under a system proof (TH1) | 6.2; CO7 |
| PO6 | `AttemptReport::Applied` gains `confirmed: bool` and `notification: Sent, NotRequested or Unknown`; every result may carry `AuthLost(account)` or `AccessChanged(account)` | Section 7; 3.8; 6.10 |
| PO7 | Sign-in is not a provider order. It runs on its own worker: `Connect(account, expected subject)` answered with `ConnectOutcome`; `Revoke(account)` answered `Revoked` or `NotConfirmed`; `Forget(account)` | A sign-in waits up to 300 s and must not hold up a dispatch |
| PO8 | Every order names the proof that permits it (PD29); the worker refuses one that does not | One invariant to test: no Calendar call without a proof id |

The adapter maps its `ChangeOutcome` one to one onto `AttemptReport`. Phase 2 adds that module and the dependency
on the core.

---------------------------------------------------------------------------------------------------------------------

## 11. What the store must keep (request to the lead; W04 3.6 reserves migration 2)

Additive only, as W04 3.6 and R22 require: new tables, new nullable columns. No existing column changes.

| # | What | Proposed shape |
|---|---|---|
| ST1 | The Google side of an account | `provider_accounts(account_id PK, provider 'google', subject UNIQUE, granted_scopes JSON, token_ref, connected_at_ms, calendars_listed_at_ms)`. No token |
| ST2 | The calendar map | `provider_calendars(account_id, calendar_id, provider_calendar_id, access_role, is_primary, time_zone, listed, first_seen_ms, last_seen_ms, PRIMARY KEY(account_id, calendar_id), UNIQUE(account_id, provider_calendar_id))` |
| ST3 | Observations | Per request and proof: `(request_id, proof purpose, account_id, calendar_id, event_id, revision, facts JSON, before JSON, observed_at_ms)`. PLAN 11 "provider observations"; PLAN 10.3 "raw encrypted evidence": the event as Google returned it, at most 256 KiB |
| ST4 | Held read results | A projection row per request (`request_id, kind, digest, bytes, observed_at_ms, stored_at_ms, released_count, purged_at_ms`) and the payload in a table outside the projection and the ledger payload, so that it can be purged. The ledger row names the digest, not the content (W04 R13) |
| ST5 | Effects | Nullable columns: `revision_before`, `revision_after`, `confirmed`, `notification`, `reconcile_reads`, `next_reconcile_at_ms`, `first_reconcile_read_at_ms` |
| ST6 | "Fabric created this" | Readable from `effects`: a create whose request reached `applied`. `effects_provider_event` (W04 3.5) already keys it by account, calendar and event id |
| ST7 | Mutations and operation types | For each row above, and for `owner.connect_google`, `provider.connect`, `provider.calendars`, `provider.observation`, `provider.auth_lost`, `request.read`, `request.release`, `request.reconcile` |
| ST8 | A backup still holds no token | True by ST1 |
| ST9 | Budget of the snapshot | Accounts are not in W04's D38 budget (R24). An account with 250 calendars is about 60 KiB of snapshot; bound calendars per account (250) and accounts (owner's call; CD-W00b says no cap in code) |

The replay question of W04 R22 stays the lead's: ST1 to ST5 only add.

---------------------------------------------------------------------------------------------------------------------

## 12. What the core must do (request to the lead)

| # | What |
|---|---|
| CO1 | `connect_google`: with `account_id: null` take the placeholder (CO2) or create an account; with an id, that account. State `awaiting_consent`, revision + 1, `revocation_epoch` + 1 (W04 5.6), notice, answer; then the `Connect` job. A second `connect_google` supersedes a waiting one (PD5) |
| CO2 | At most one account that never connected exists. A new connect reuses it, so failed attempts do not pile up as accounts |
| CO3 | Recovery (`broker.started`): every `awaiting_consent` account becomes `cancelled` |
| CO4 | `disconnect_google`: as today, then `Revoke` when `revoke_provider_token` is true, then `Forget` in every case; purge the account's held results and observations; report whether Google confirmed (V3-3) |
| CO5 | Reads: lift D33. The read order after a fresh permit; the held result; `fabric_request_result` as `Disclose`; `result_held` and `used` in the facts; the release in the outcome only (section 9) |
| CO6 | Observation before each proof on an action that names an event; `fabric_created` from ST6; `private.before`; `Fence.event_revision`; an `Absent` observation ends the request `terminal_failure` (`invalid_arguments`); a revision other than the approved one after approval is `stale` (PLAN 14 "provider event changed after preview") |
| CO7 | Calendar list: at connect, at each start, on `AccessChanged`, and before the owner chooses calendars. A lost calendar or a lower role is a reduction: `revocation_epoch` + 1, the grants that name it end `authority_reduced`, its waiting cards go `stale` (PLAN 10.2: "ACL downgrade must stop writes and stale approvals") |
| CO8 | Reconciliation: `outcome_unknown -> reconciling` at `earliest_read_ms`; system proof; reconcile order; `reconcile::decide`; `ReadAgain` waits; a failed read repeats by 6.11, then `needs_owner_resolution`. `applied -> reconciled` as section 7 |
| CO9 | System proofs: `FactActor::System`, the three system actions (THEORY, TH1), their digest (W04 CCR-14), and 6.11 for the retries of reads and jobs |
| CO10 | Configuration: one more broker variable for the client file's path in development (W04 D27 lists the names the broker accepts); in a release the client id is part of the package (CD-W11). The token store follows the key source: Keychain with `keychain`, memory with `test` |
| CO11 | `AuthLost`: the account becomes `reauthentication_required`, `revocation_epoch` + 1; nothing is dispatched for it until it is `connected` again |

---------------------------------------------------------------------------------------------------------------------

## 13. Requests to the contract session

Choosing calendars with Agent Drafts, and resolving an unknown outcome, are already asked. These are added. They
are sent once this design has been through its critique.

Schema version 3:

| # | What | Why |
|---|---|---|
| V3-1 | `calendar_access`: Google's fifth role, or the statement that it is shown as `writer` | PD26; G-DOC |
| V3-2 | Why a connection attempt ended, owner only: a closed list (`timed_out`, `superseded`, `browser_not_opened`, `google_error`, `network`, `identity_check_failed`, `unexpected_scope`, `no_refresh_token`, `token_store`), and for `partial_grant` which permission was missing | The eight states say what, not why; the page cannot tell the owner what to do next. Same pattern as `prover_problem` |
| V3-3 | The result of `disconnect_google`: whether Google confirmed the revocation (`revoked`, `not_requested`, `not_confirmed`) | 3.9; PLAN 9.1 "offer explicit provider token revocation" |
| V3-4 | In a request's details: whether the notifications of an applied change are known to be sent | PLAN 11: "display that uncertainty" |
| V3-5 | A way for the owner to ask for a fresh calendar list of one account, with or beside the command that chooses calendars | 6.2; CO7 |
| V3-6 | Say that `occurrence` in an event view is disclosed with `time` | 6.4 |
| V3-7 | Optional: a read result that can say "incomplete". Without it a read past its bounds fails (PD23), which needs no change | PLAN S06 |
| V3-8 | Unchanged asks of W04 that this package now needs: CCR-1 (`connect_google` gets implemented, the code goes), CCR-14 (context and digest of the broker's own actions) | |

Rule theory (same owner):

| # | What | Why |
|---|---|---|
| TH1 | A system action for listing an account's calendars, authorized when the account is connected | PD29: `System_refresh` needs a selected calendar, and at connect none is selected |
| TH2 | `ctx_pending_effects` should also hold a change that is `applied` and not yet `reconciled`, if the read after a change is kept | Section 7: `System_reconcile` is authorized only for a pending effect |
| TH3 | Confirm: `writerWithoutPrivateAccess` gives `cal_writable` | PD26 |

No stored shape changes in any of these (the contract session's rule: a stored shape needs a store design first).

---------------------------------------------------------------------------------------------------------------------

## 14. The fake Google server (FAKE)

### 14.1 What it is

`FakeGoogle::start() -> FakeGoogle` binds `127.0.0.1:0` and serves, on one port: the authorization endpoint, the
token and revocation endpoints, the key set, and the Calendar paths of section 6. It is a test double, written
from G-DOC. It is never evidence that Google behaves so (14.3).

- State: accounts (subject, address, verified or not), clients, per account calendars with roles, events as JSON
  with a revision counter, issued codes (single use, bound to challenge, redirect, scopes, nonce), refresh and
  access tokens with end times on a clock the test moves, two signing keys.
- `fake.browser().visit(url, decision)` plays the person: it requests the authorization address, follows the
  redirect to the adapter's listener with a plain HTTP request, and returns the page the listener answered.
  Decisions: allow everything, deny, untick given scopes, sign in as another account, close the window.
- `fake.log()` is every request it received: method, path, query, whether `Authorization` was present and for
  which account, `If-Match`, the body. Tests assert on it ("no request was made", "one changing request").

### 14.2 Faults, each set for the next matching request

Answer with a given status and Google's error shape (`reason`), with or without `Retry-After`; apply the change
and then close the connection without an answer; close without applying; delay past the deadline; a body that is
not JSON, too long, or of another content type; a redirect; an expired access token (401); a refresh token that is
`invalid_grant`; an identity token with a wrong issuer, audience, nonce, expiry, signature, algorithm or key id;
an unverified address; fewer or more scopes than asked; a lower role or a vanished calendar; an event changed by
someone else; an insert that becomes visible only after a number of reads.

### 14.3 What FAKE assumes about Google

Each row is a place where FAKE was written from the documentation or from a guess, and the adapter depends on it.
Each has a live check in 15.3.

| # | Assumption | Basis |
|---|---|---|
| FA1 | PATCH honours `If-Match` and answers 412 on another revision | G-DOC names only update and delete |
| FA2 | An insert with an id that exists answers 409 `duplicate` | G-DOC |
| FA3 | GET of a deleted single event answers 200 with `status: cancelled`; DELETE of it answers 410 | G-DOC says cancelled events are deleted events; the status codes are from the error guide |
| FA4 | The token endpoint accepts a Desktop app client with its secret and PKCE | G-DOC marks the secret optional |
| FA5 | The consent screen lets a person untick single scopes, and the token then lists fewer | G-DOC |
| FA6 | A PATCH with `attendeesOmitted: true` and one attendee changes only that attendee's answer | G-DOC, one sentence |
| FA7 | The event Google returns after insert or PATCH holds `summary`, `start`, `end`, `description`, `location`, `attendees` as written | G-DOC |
| FA8 | `events.list` with `singleEvents=true` returns occurrences with `recurringEventId` and `originalStartTime` | G-DOC |
| FA9 | A refresh token that ended answers 400 `invalid_grant` | G-DOC names the causes, not the answer |
| FA10 | Rate limits arrive as 403 or 429 with the reasons of 6.10 | G-DOC |
| FA11 | The etag of an event is a quoted string that changes with every change | G-DOC ("changes every time the resource changes") |
| FA12 | `freeBusy` reports a calendar it cannot read under `errors`, not as an empty list | G-DOC shows the member |

---------------------------------------------------------------------------------------------------------------------

## 15. Tests

No test touches the Keychain, the network beyond `127.0.0.1`, or a real Google host. Every test that makes a
request makes it to FAKE. Each test name states what it proves; the ids are for citation.

### 15.1 The list (GC `tests/` unless marked unit)

| Id | Proves |
|---|---|
| g01 | A full sign-in against FAKE ends `Connected`, with the subject, the verified address, the five scopes, and the refresh token in the token store |
| g02 | The authorization address has exactly the parameters of 3.2.1; the challenge is the SHA-256 of the verifier; verifier, state and nonce differ per flow and are 43 characters |
| g03 | Denied consent is `ConsentDenied`; nothing is stored; no token request is made |
| g04 | A missing calendar scope is `PartialGrant`; the tokens are revoked at FAKE; nothing is stored |
| g05 | A scope beyond the five is `UnexpectedScope`; revoked; nothing stored |
| g06 | A reconnect as another subject, and a new connect as a known subject, are `AccountMismatch`; the old token is untouched |
| g07 | The listener: a wrong state, a missing state, a doubled parameter, another path, another method, another `Host` each get an error page and do not end the flow; the right state ends it once; a second callback finds no listener |
| g08 | The listener ends after 300 s (test clock) with `TimedOut`, and at once when superseded |
| g09 | The page the listener answers is the fixed text and echoes no parameter |
| g10 | Each identity-token fault of 14.2 is `IdentityCheckFailed`, and nothing is stored: wrong issuer, audience, `azp`, nonce, expired, issued in the future, too old, bad signature, `alg` `none`, `alg` `HS256`, unknown key id, a short key |
| g11 | An unknown key id fetches the key set once more and then verifies; the key set is cached by its `max-age` |
| g12 | An unverified or unusable address connects with no address |
| g13 | A token answer without a refresh token, of another token type, or that is not JSON, ends `Cancelled` |
| g14 | An access token near its end is refreshed before the call; a 401 drops it and is Transient; `invalid_grant` deletes the stored token and is `AuthLost` |
| g15 | Revocation: 200 and `invalid_token` are `Revoked`; anything else is `NotConfirmed` |
| g16 | No secret in any `Debug`, `Display` or error text: the verifier, state, code, tokens and client secret of a full run are searched for in every formatted value the tests can reach |
| g17 | The calendar list: pages, the five roles of 6.2, names, an unknown role left out; never `primary` |
| g18 | Two accounts: each call carries its own account's token; account A's token on a calendar only B has is Rejected (PLAN 14, account A against B) |
| g19 | Free/busy: periods per calendar under the broker's ids, in UTC, cut to the window; an error for one calendar fails the read |
| g20 | Reading events: the window bounds, occurrences of a recurring event with their identity, an all-day event as dates, a cancelled item skipped, paging through `More` |
| g21 | Fields: for each granted field, `fields=` asks only its members, and a view shows a field only when it is granted even if FAKE sends everything |
| g22 | Bounds: a read past 2 500 events or 768 KiB fails with the fixed sentence; nothing partial is returned |
| g23 | Times: `Z`, an offset, a fraction of a second, a missing zone, a day-light-saving gap and fold given by Google are read as 6.9; an all-day date is never an instant |
| g24 | Observation: each member of 6.5 from fixtures: a plain event, an invitation, a recurring master, an occurrence, an all-day event, a focus-time event; `Absent` for 404, 410 and cancelled; an unusable attendee address fails it |
| g25 | Create: the request is exactly 6.6 (reserved id, `sendUpdates=none`, no attendees); Applied carries id and revision; an order without a valid reserved id makes no request |
| g26 | Create, answer lost after FAKE applied it: NoAnswer; a later order with the same id gets 409, reads, and reports Applied; FAKE holds one event |
| g27 | Update: PATCH with `If-Match`, only the set members, `sendUpdates` as approved; untouched members of the event are unchanged at FAKE |
| g28 | Update against a changed event: 412 is `revision_conflict` and FAKE's event is unchanged (PLAN 14, changed after preview) |
| g29 | Update, attempt 2 after an applied attempt 1 that answered 503: reads, compares, reports Applied; one change at FAKE |
| g30 | Invitation: the attendee list after the change is exactly the approved one; kept attendees keep their answers; a changed revision at the read makes no PATCH |
| g31 | The adapter has no path to an agent: a source scan finds no agent type and no frame type in GC (section 9) |
| g32 | Response: only the own entry changes; an occurrence is addressed by instance id and its original start is checked; no own entry is Rejected |
| g33 | Delete: `If-Match` and `sendUpdates` as approved; 412; 410 is Applied; 404 on attempt 1 and on attempt 2 |
| g34 | Classification, one case per row of 6.10, for a change and for a read; `Retry-After` is passed on and capped |
| g35 | Nothing sent (FAKE closed, deadline already past) is Transient; sent and unanswered is NoAnswer; a slow answer past the deadline is NoAnswer for a change and Transient for a read |
| g36 | A redirect is not followed; a body over its bound and a wrong content type are no usable answer |
| g37 | One changing request per order: for every tool and every fault, FAKE's log holds at most one POST, PATCH or DELETE per order |
| g38 | Every request that can notify carries `If-Match`, and every changing request carries an explicit `sendUpdates` |
| g39 | Path building: calendar and event ids with `#`, `/`, `?`, `%`, `..` and spaces reach FAKE as one path part |
| g40 | Reconciliation: every row of section 7's table through `decide`, with FAKE as the source of the read-back; the two-read rule for a create with FAKE's late-visible insert |
| g41 | The retry rule: waits of 2, 4 and 8 s, a hint longer than the wait, the fourth failure, the 120 s budget (unit) |
| g42 | `matches`: each tool, each member, absent against empty, instants written with different offsets (unit) |
| g43 | Forbidden dependencies and FAKE only as a dev-dependency (the scan of 2.1) |
| g44 | `Endpoints::google()` is the five constants; the loopback set accepts only `127.0.0.1` (unit) |
| g45 | The client file: the `installed` shape is read; another shape is an error that names the path and no content (unit, with a made-up file) |
| f01 | FAKE's own rules: a code is single use and bound to its challenge and redirect; `If-Match`; a duplicate id; revisions rise; the log is complete |

### 15.2 The rows of gate `CD-G2b` and what stands in for each

`CD-G2b` is "a real test-account Google spike" (PLAN 13). No test of 15.1 closes it or any part of it. The table
says which tests show the adapter's side against FAKE, what the core must add, and what only the live run shows.

| # | Row (PLAN 5, 10, 14) | Stand-in here | Still needed |
|---|---|---|---|
| 1 | Missing scope or denied consent: no false connected state (PLAN 14; R03) | g03, g04, g05, g13 | core: the account states (CO1); live: LV1, LV5 |
| 2 | Account A against account B; a shared calendar's role lowered (PLAN 14; R03a) | g17, g18, g34 (403) | core: CO7; live: LV6 |
| 3 | Insert accepted, answer lost, broker killed: one event (PLAN 14; R12) | g26, g40 | core: the kill and CO8; live: LV4 |
| 4 | Event changed after the preview: conflict, new approval (PLAN 14) | g28, g30 | core: CO6; live: LV3 |
| 5 | Invitation: only the approved recipients, notified as approved (PLAN 14; R05) | g30, g38 | live: LV7 (a real message arrives) |
| 6 | DST gap and fold, an all-day occurrence, a recurring master and exception; an unsupported change refused (PLAN 14) | g20, g23, g24 | theory and core refuse the change; live: LV8 |
| 7 | Poll a completed read after revocation: status only (PLAN 14; W04 Q10) | g31 | core: ST4, CO5 and its test (section 9) |
| 8 | Refresh, its failure, reconnect; seven days in testing mode (R03; PLAN 10.1) | g14, g06 | core: CO11; live: LV5 |
| 9 | Disconnect and revocation at Google (R03; PLAN 9.1) | g15 | core: CO4; live: LV5 |
| 10 | Read bounded events and availability from selected calendars (R04) | g19 to g22 | core: CO5; live: LV8 |
| 11 | Create and edit in an existing calendar (R05; C07, C13) | g25, g27 | live: LV3, LV6 |
| 12 | Delete a Fabric-created event with `If-Match`, confirmed by reading back (CD-Q05a) | g33, g40 | live: LV3 |
| 13 | Answer one occurrence of a recurring invitation (CD-Q05b) | g32 | live: LV7 |
| 14 | Tokens in the Keychain, never elsewhere (R13) | g16 | phase 2: `KeychainTokenStore`; live: LV9 |

### 15.3 What only a live run can show

Each is run by the owner at the Mac with the accounts of RUNBOOK, in phase 2.

| # | Check |
|---|---|
| LV1 | The real consent screen for a Desktop app client: the warning for test users, unticking one scope, what the token then lists (FA5) |
| LV2 | Whether the token endpoint needs the client secret for this client type (FA4) |
| LV3 | PATCH and DELETE with `If-Match`: applied on the right revision, 412 on another (FA1); what a GET of a deleted event answers (FA3); the form of an etag (FA11) |
| LV4 | A reused event id is refused (FA2); how soon after an insert a GET sees it (PD20, RK3) |
| LV5 | Refresh; `invalid_grant` after access is removed in the Google account and after seven days (FA9); revocation |
| LV6 | A calendar shared with "make changes": create and edit there; lower the role and see the write refused; the fifth role (PD26) |
| LV7 | A real invitation reaches the owner's mailbox for `all`, and what `externalOnly` and `none` do; an answer changes only the own entry and tells the organizer (FA6) |
| LV8 | Real recurring events as occurrences; a real all-day event; free/busy over several calendars and its limits (FA8, FA12) |
| LV9 | The Keychain item: created, read after a restart, deleted at disconnect; the prompt after a rebuild |
| LV10 | `/usr/bin/open` from the broker as the host starts it; the loopback redirect in Safari and in Chrome |
| LV11 | Real rate-limit answers, if they can be provoked safely (FA10) |

---------------------------------------------------------------------------------------------------------------------

## 16. Integration (phase 2), in order

1. The contract session drafts schema version 3 with V3-1 to V3-6 and the theory with TH1 to TH3; the owner's
   approval is the merge.
2. The lead writes the store design for migration 2 (ST1 to ST9) with this session, then builds it.
3. PORTS gains PO1 to PO8. GC gains the dependency on the core, the mapping of its outcomes, `KeychainTokenStore`
   with its ignored test, and `SystemBrowser`.
4. The core gains CO1 to CO11, each with acceptance tests that use FAKE as the provider, in process.
5. `apps/fabric-broker` gains the two workers (sign-in, provider), the configuration of CO10, and process tests
   against FAKE with the test key and the in-memory token store.
6. The practice calendar's observation hook goes (PW 4.4). The practice calendar itself may stay as the debug-only
   provider of the synthetic account; that is the lead's and the owner's call (W04 D13).
7. The live run of 15.3 with the owner, recorded as the evidence of `CD-G2b`, then the independent review.

Nothing of phase 2 is in this pull request.

---------------------------------------------------------------------------------------------------------------------

## 17. Decision records

ADR-111 (calendar authority and time model) and ADR-113 (projection and commit) are written as Proposed with this
design, as CD-W04 wrote ADR-115. Each covers only what the broker settles; its Rendezvous part stays parked
(`docs/architecture.md` 19). Merging the pull request that carries them is the owner's acceptance.

---------------------------------------------------------------------------------------------------------------------

## 18. Risks, questions, not checked

### 18.1 Risks

- RK1 Everything in phase 1 is tested against a double written from documentation. 14.3 lists what it assumes; a
  wrong assumption shows only in the live run.
- RK2 Google does not document `If-Match` for PATCH (FA1). If it is not honoured, PD14 and PD15 fall together and
  the fallback is a read followed by a full update with `If-Match`, with attachments and conference data kept by
  their version parameters.
- RK3 A read right after an unanswered insert may not see an event that exists a moment later. PD20 waits and
  reads twice; Google gives no bound, so a duplicate after `absence_confirmed` cannot be ruled out, only made
  unlikely. The owner-resolution path covers what is left.
- RK4 `sendUpdates=none` "can have significant adverse effects" and "some emails might still be sent" (G-DOC). The
  broker promises the owner which value it sends, not what Google then does (PLAN 10.3).
- RK5 A Calendar scope is coarser than any Fabric rule: with `calendar.events` the broker can change every event
  the account can. The limit on what happens is the broker's own (PLAN 10.2). A stolen refresh token has the full
  scope.
- RK6 In testing mode a connection ends after seven days. Development and the gate run must expect
  `reauthentication_required` weekly.
- RK7 Google deletes a client unused for six months, and a refresh token unused for six months.
- RK8 PD25 cuts long texts and leaves out attendee addresses the contract cannot hold; an agent may act on a
  shortened description. The owner's before-image is cut the same way.
- RK9 A person can change an event between the observation and the dispatch. `If-Match` catches it at Google; the
  approval then shows as a conflict, never as an overwrite.
- RK10 The `rsa` crate has an open advisory about timing in private-key operations. The adapter only verifies with
  public keys; FAKE signs with a test key that protects nothing.
- RK11 `reqwest` brings an async runtime thread into the broker process for the blocking client. It is confined to
  the worker; the core stays free of it (W04 section 2).
- RK12 The same-user threat of PLAN 4.3 holds here too: a program running as the user can read the Keychain item
  after the user allowed it once for a build, and can race for the loopback port. The state check and PKCE keep a
  stolen code useless; they do not stop a program that already controls the account's browser.
- RK13 Two proofs and two reads for every change of an existing event (observe, then act) add about two seconds to
  each classification and each fresh permit. W04 R8 applies.

### 18.2 Questions only the owner can answer

The code and this design run on the default after each.

- OQ1 The scope list of RUNBOOK section 5 is the candidate. It is frozen after the live run (PLAN 10.2). Default:
  the five.
- OQ2 How long may a sign-in wait for the browser? Default: 300 s.
- OQ3 A partial grant keeps no token and asks to connect again. Or run with what was granted? Default: no token.
- OQ4 Keychain names: service `ai.paracels.fabric.calendar.google`, account `refresh-token-v1.<account id>`.
  Default: these.
- OQ5 How long is a read result held when nobody fetches it? Default: 24 h, and never past its one release or its
  account's disconnect.
- OQ6 Checking the identity token's signature adds the `rsa` crate. Google would allow leaving the check out; the
  plan asks for it. Default: check.
- OQ7 The HTTP client: `reqwest`, already in the lock file, or a smaller blocking client that adds crates. Default:
  `reqwest`.
- OQ8 The client switch "Use this client for an AI-powered agent" (RUNBOOK 6.3). Default: off.
- OQ9 Is the calendar list at connect a broker action that needs a proof (TH1), or part of the owner's own command?
  Default: it needs a proof, so that no Calendar call exists without one.
- OQ10 The broker opens the system browser itself. Default: yes.
- OQ11 PATCH with `If-Match`, pending LV3. Default: PATCH.

### 18.3 Not checked

- No behaviour of the real Google: every row of 14.3 and 15.3.
- The Keychain, in any form. `/usr/bin/open`. A real browser.
- TLS to Google: the client is built with it on macOS and Windows and no test makes a TLS call.
- Linux and Windows builds, until CI runs them.
- Sections 10 to 13 are proposals. Their owners have not answered them.

---------------------------------------------------------------------------------------------------------------------

## Appendix A. What Google's documentation states (read 2026-11-17)

| Page (`developers.google.com/` unless marked) | Relied on for |
|---|---|
| `identity/protocols/oauth2/native-app` | Loopback redirect `http://127.0.0.1:port` for desktop apps; custom schemes and copy-paste no longer supported; "Always use PKCE"; verifier 43 to 128 characters; `S256`; the authorization and token endpoints and their parameters; `client_secret` optional; refresh tokens "always returned for installed applications"; revocation endpoint, 200 and 400; "Incremental authorization is not supported for installed apps"; a person may grant fewer scopes, check `scope`; `error=access_denied`; desktop apps "cannot keep secrets" |
| `identity/protocols/oauth2` | Why a refresh token stops working; seven days in testing mode except for name, address and profile scopes; 100 refresh tokens per account and client |
| `identity/openid-connect/openid-connect` | The identity token's checks; `sub` "never reused", at most 255 ASCII characters; `email` may change; `nonce`; the discovery document's endpoints; `RS256`; keys change rarely and may be cached by the HTTP cache headers |
| `support.google.com/cloud/answer/15549945` | Testing mode: 100 test users, authorizations end after seven days; the user cap of an unverified app |
| `support.google.com/cloud/answer/15549257` | A desktop client needs no further information; the secret is shown once; a client unused for six months is deleted |
| `workspace/calendar/api/auth` | The scope names and what each allows |
| `workspace/calendar/api/v3/reference/calendarList/list`, `.../calendarList` | Parameters, the scopes accepted, the five access roles |
| `workspace/calendar/api/v3/reference/freebusy/query` | Body, at most 50 calendars, the scopes accepted (`calendar.freebusy`, not `calendar.events`), `busy` and `errors` |
| `workspace/calendar/api/v3/reference/events/list`, `get` | `singleEvents`, `orderBy`, `timeMin` (end) and `timeMax` (start), page size 250 by default and at most 2500, the scopes accepted |
| `workspace/calendar/api/v3/reference/events/insert` | The `id` rules; `sendUpdates` values; "some emails might still be sent"; collisions may go undetected at creation |
| `workspace/calendar/api/v3/reference/events/patch`, `update`, `delete` | PATCH semantics, arrays are replaced; update replaces the whole event; delete answers an empty body; `sendUpdates` |
| `workspace/calendar/api/v3/reference/events` | `status` `cancelled`; `eventType` values; `recurringEventId`, `originalStartTime`; `start` and `end`; attendee members, `self`, `attendeesOmitted`; `organizer.self` |
| `workspace/calendar/api/guides/version-resources` | `etag`; `If-Match` and 412 for update and delete; no conditional insert |
| `workspace/calendar/api/guides/errors` | The status codes and reasons of 6.10 and what Google advises for each |
| `workspace/calendar/api/guides/quota` | 600 requests a minute per user and 10 000 per project; 403 or 429 when exceeded |
