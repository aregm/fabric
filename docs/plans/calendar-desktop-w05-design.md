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
contract win wherever they differ from this document. Sections 9 to 13 are requests: their owners decide the form,
and the lead writes the store design. Sections 8 and 6.9 are proposed to the owner as ADR-111 (Proposed), sections
6.6 to 7 as ADR-113 (Proposed).

Date 2026-11-17. Status: **Proposed**. Sections 2 to 8, 14 and 15 describe what phase 1 builds. Three independent
readers went through the first draft before anything was built (18.4).

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
| PD2 | No Cargo features, no test seam in release | The endpoints are Google's fixed hosts. A loopback endpoint set exists only under `cfg(debug_assertions)`, only for `127.0.0.1`, and a broker may use it only with the test key source | W04 D11: CI builds with `--all-features`; a release broker can never be pointed at another host, and a debug broker with real tokens cannot either. |
| PD3 | HTTP client | `reqwest` 0.13, blocking, no default features; TLS through the operating system (`native-tls`) on macOS and Windows only; HTTP/1.1, a connection of its own per request, no redirects, no proxy, no compression, a deadline on every call. Behind one private module (`http`), so the choice can change without touching callers | It is already locked for the demo with the same TLS split, so the lock file gains no HTTP crate. Workers are threads (W04 5.1), so the calls are blocking. Open: OQ7. |
| PD4 | The identity token is checked in full | Signature (RS256, key by `kid` from Google's key set), issuer, audience, expiry, issue time, nonce, subject. Verification only, with the RustCrypto `rsa` crate | PLAN 10.1 asks for the signature. G-DOC allows skipping it for a token read straight from Google over TLS; the plan is stricter and wins. Open: OQ6. |
| PD5 | One sign-in at a time | A new `connect_google` ends a sign-in that is still waiting (`cancelled`) | One loopback listener, one verifier in memory, nothing to mix up. Open: OQ12. |
| PD6 | The loopback listener | `127.0.0.1` on a port the system picks; redirect `http://127.0.0.1:<port>`; lives at most 300 s; only time, a supersede, or the one request with the right `state` ends it; a fixed page, nothing echoed | PLAN 10.1 (bounded lifetime, single use). G-DOC: loopback is the supported redirect for desktop apps. Open: OQ2. |
| PD7 | The broker opens the browser | `/usr/bin/open <url>`, one argument, built by the adapter. The page never sees the address | The contract's `connect_google` result carries no address, and the webview keeps no capability to open one (PLAN 4.3). Open: OQ10; RK12; LV10. |
| PD8 | A partial grant keeps no token | A missing permission: nothing is stored; state `partial_grant` | PLAN 14: no false connected state. G-DOC: installed apps cannot add scopes later, so the next attempt starts from zero anyway. Open: OQ3. |
| PD9 | More permission than asked is refused | A token whose scopes go beyond the five requested ones is not stored | PLAN 10.2: never the blanket or the ACL scope. The broker must not hold more than it shows. |
| PD10 | Identity is the subject | An account is its Google subject (`sub`); one record per subject, for good. Its verified address is required, so the owner sees what connected, but it is display only. Reconnecting an account under another subject, and adding a subject that a record already has, are `account_mismatch` | PLAN 10.1, S01; G-DOC: `sub` is "never reused", the address "could change". |
| PD11 | Only the refresh token is stored | In the token store, one item per account. Access tokens live in memory. Client secret, verifier, state, nonce and codes are never written anywhere | The least that survives a restart. A backup of the store holds no token (W04 3.6). |
| PD12 | Token store | A trait with a Keychain implementation (phase 2, macOS) and an in-memory double (phase 1). Keychain service `ai.paracels.fabric.calendar.google`, account `refresh-token-v1.<account id>`, this Mac only. With the test key source the broker uses the in-memory double | PLAN 12 (a distinct Keychain service), PLAN 10.1 (never exported to another Mac). No session may touch the Keychain. Open: OQ4. |
| PD13 | One changing request per order | A dispatch order makes at most one request that can change Google (POST, PATCH, DELETE). The adapter never sends it twice. Reads before it are allowed | PLAN 5 (a fresh proof per retry): every repeat is a new order from the core. |
| PD14 | Every change of an existing event is conditional | PATCH and DELETE always carry `If-Match` with the one revision the approval names, never a wildcard | PLAN 10.3. A change that can notify therefore cannot be applied twice. |
| PD15 | PATCH, not get-then-PUT | Field edits, attendee changes and answers are PATCH requests with only the approved members | PLAN 10.3 "preserve untouched fields": PATCH cannot drop a field the adapter does not model. LV3 confirms it, with get-then-PUT as the fallback. Open: OQ11. |
| PD16 | A create carries the reserved id and no attendees | `events.insert` with the id the core reserved (26 base32hex digits) and `sendUpdates=none` | PLAN 11; CR 14 item 21. G-DOC: ids are 5 to 1024 characters of `a-v` and `0-9`. |
| PD17 | Transient means it certainly did not happen | Transient is only: nothing was sent, or Google refused before acting (401, a rate limit, a conflict). Every other answer that is not a plain yes or a plain refusal (no answer, an unreadable 2xx, a 5xx) is an unknown outcome and is reconciled, never retried | The core proves each retry afresh on a new observation (CO6). A retry after an attempt that was in fact applied would be judged against the changed event and end as a conflict, or be sent twice. RK15 is the price. |
| PD18 | Outcome classes | Applied, Transient (with Google's wait hint), Rejected (with a contract failure code), NoAnswer; the table is 6.10 | W04 D16 decides waits from these classes. |
| PD19 | No marker in Google | Fabric writes no private property into an event. "Fabric created it" is the store's record of the create, keyed by account, calendar and event id | PLAN 7 ("ownership tracked locally"). A marker on a shared calendar can be forged by anyone who may write there. |
| PD20 | Absence needs two reads | After an unknown outcome a change is "not there" only when two reads at least 60 s apart, the first at least 10 s after the attempt ended, both show that | Google may finish a request after the broker stopped waiting, and "cannot guarantee that ID collisions will be detected at event creation time" (G-DOC). RK3. |
| PD21 | Calendar and account ids | The broker issues `cal-<32 hex>` per pair (account, Google calendar id) and keeps the pair for the life of the account record. Event ids and revisions are Google's, verbatim (the revision is the etag with its quotes) | PW 9.2 item T6, as the contract owner answered it. A Google calendar id is not a contract `id` (it holds `@`). |
| PD22 | Never `primary` | No request uses Google's keyword `primary`; every calendar is named by the id the calendar list returned | PLAN S01: never silently the primary calendar. |
| PD23 | Reads are bounded and never cut silently | A read that does not fit its bounds (2 500 events, 768 KiB of result, 2 000 attendees of one event), or whose attendee list Google cut, fails with "ask for a shorter window". A free/busy answer with an error for any calendar fails as a whole. A calendar list past 250 fails | PLAN 5 R04 bounds a read; a cut answer shown as whole would be a wrong answer. The contract's result has no "incomplete" member (V3-7). |
| PD24 | Fields are cut twice | A read asks Google only for the members the granted fields need (`fields=`), and the adapter builds the result from the granted fields only, whatever Google returned | PLAN 7 (field minimization); the second cut does not depend on the first. |
| PD25 | Texts that do not fit the contract | A title, place or description longer than the contract's bound is cut at the bound. An attendee address the contract cannot hold is left out of a read, but makes an observation fail | A read may show less; a check of who is notified may never miss a recipient. RK8; OQ15. |
| PD26 | A fifth access role | Google's `writerWithoutPrivateAccess` is shown as `reader` and is not writable, until the contract names it | The contract has four roles (V3-1). G-DOC: this role sees private events without their details, exactly as `reader` does, and may also write. `reader` is therefore a true statement of part of what the account may do; `writer` would claim a view of private events it does not have. An unknown role is never read upward (PLAN 2.3: never broaden silently). The cost: such a calendar cannot be changed through Fabric until schema version 3. |
| PD27 | No error text from Google or from a library is shown or stored | Explanations are fixed sentences of the adapter. Google's `reason` is read against a closed list; its `message` is dropped. No error of the HTTP, JSON, URL or RSA library is ever formatted | W04 D37; a server message is untrusted text, and library errors print addresses and values. |
| PD28 | Held read results | Kept in the encrypted store outside the ledger's payload, released only by a `Disclose` proof, purged on release of a one-request read, on disconnect, and after 24 h | W04 D33; CR 8; PLAN 9.2. Open: OQ5. The lead's store design decides the form (ST4, CO5). |
| PD29 | Every Calendar call has a proof | The provider worker carries out only orders that name the proof that permits them: an agent's fresh permit, or a system proof for the broker's own reads | PLAN 1 and C05. The theory lacks one system action (TH1). Open: OQ9. |
| PD30 | ADR-111 and ADR-113 | Written as Proposed with this design | `docs/architecture.md` 19: the broker settles their subjects before `CD-G5`; CD-W04 did the same with ADR-115. |
| PD31 | The account's own entry exists only on its primary calendar | An attendee entry marked `self` is the account's own only on the account's primary calendar. Elsewhere there is no invitation to answer and no own response | G-DOC: `self` marks the calendar the copy is on, not who is signed in. Without this rule Fabric would answer another person's invitation on a calendar that person shared. |
| PD32 | Ids are checked before they reach a URL or a header | An event id must be letters, digits and underscores; a revision one quoted entity tag; a calendar id comes only from the list. Anything else makes no request (5.3) | An agent writes these. `..` as an event id would be folded into the path of another Calendar method, and `If-Match: *` forces a change through. |
| PD33 | Times are read in the calendar's zone | Every events read names the calendar's zone; a view pairs Google's offset with that zone | The offset Google writes follows the zone the request names, not the event's own; pairing it with the event's zone would give times that contradict themselves (6.9). |
| PD34 | A failed sign-in revokes nothing | Tokens of a sign-in that does not end `Connected` are wiped from memory and not revoked. Revocation happens only when the owner asks for it at disconnect | A revocation is taken to end the whole grant of that Google account to the client (FA14), which would end a working account's token. |
| PD35 | One order reads every page | A read order reads all pages inside one deadline and asks the core's `still_allowed` flag before each | PLAN 8 wants authority checked at each page; the lifecycle has no way back from `dispatching` and an authorization is used once, so a page cannot be an attempt. |

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
| `http` | The one place that talks HTTP: request, response, deadline, bounds, the nothing-sent and sent-unanswered split (5.2); `ids`, the checks of 5.3, beside it |
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
`reqwest`, `base64`, `getrandom` and `zeroize` are locked already. `rsa` is new: it brings about fourteen crates
the lock file did not have (its big-integer and encoding crates) and second versions of eleven it had (`sha2`,
`digest`, `pkcs8`, `spki`, `der`, `signature` and their base crates), because `rsa` 0.10 uses the newer generation
of them. The blocking client of `reqwest` adds `futures-io` and `futures-sink`. This is the cost of PD4 (OQ6).

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
- `oauth::Flow::start(...) -> PendingFlow` and `PendingFlow::wait(...) -> ConnectOutcome` (3.2); `oauth::revoke`;
  `BrowserOpener` (a recording double in tests; `SystemBrowser` comes in phase 2).
- `tokens::{TokenStore, MemoryTokenStore, TokenStoreError, AccessCache}` and `forget`.
- `Google::new(endpoints, client, store, cache, clock)` with `list_calendars`, `free_busy`, `read_events`,
  `observe`, `change`, `reconcile_read`. `CalendarTarget` names a calendar for them (6.4).
- `outcome::{ChangeOutcome, ReadFailure, RetryPolicy, RetryDecision, next_attempt}`.
- `reconcile::{Effect, ReadBack, Verdict, decide, earliest_read_ms}`.
- No function takes an address, a path or a header from its caller. Callers pass typed ids and contract values
  (PLAN 10.2: never an arbitrary Google URL).

---------------------------------------------------------------------------------------------------------------------

## 3. Sign-in (OAuth 2.0 for a desktop app, with OpenID Connect)

### 3.1 The client

`ClientConfig { client_id, client_secret: Secret }`. `ClientConfig::from_file(path)` reads the file Google's console
gives for a Desktop app client: a JSON object whose member `installed` holds `client_id` and `client_secret`. Both
are required. Every other member, `redirect_uris` among them, is ignored and never used. Any other shape is an
error that names the path and nothing of the content.

The adapter sends the secret in the token and refresh requests. G-DOC marks it optional and calls a desktop client
public, but reports from developers say Google refuses a Desktop app client without it; whether it can be left out
is LV2. Where the broker gets the path is CO10. The secret of a desktop client is not a security boundary (PLAN
10.1); it is still never logged, never in an error, never in the store.

### 3.2 The flow

```
owner: connect_google ──> core: account awaiting_consent (commit, notice, answer)
                           └─> job Connect(account, expected subject or the subjects already connected) ──> adapter
adapter: verifier, state, nonce (32 random bytes each, base64url, 43 characters)
         listener on 127.0.0.1:<port>          (3.3)
         browser <── authorization address     (3.2.1)
                 ... the person signs in and decides, in the system browser ...
listener <── GET /?state=..&code=..   or   ?state=..&error=..
adapter: code + verifier ──> token endpoint    (3.4)
         identity token checks                 (3.5)
         granted scopes                        (3.6)
         subject against what the job named    (3.7)
         refresh token ──> token store         (section 4)
         ConnectOutcome ──> core ──> account state (3.7)
```

Nothing is written to the token store before every check has passed.

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
(PLAN 10.1). The address, with state, nonce and challenge, is an argument of a process and so visible to other
programs of the same user while `open` runs (RK12, LV10); the verifier never leaves the broker's memory.

**3.2.2 Results.** `ConnectOutcome` is one of:

| Outcome | When |
|---|---|
| `Connected { subject, email, scopes, refresh_token_ends_at }` | Everything below held and the refresh token is in the token store |
| `ConsentDenied` | The callback carried `error=access_denied` |
| `PartialGrant { missing }` | The token lacks the address scope or one of the three calendar scopes (PD8) |
| `AccountMismatch` | A reconnect signed in as another subject, or a new connect as a subject already connected (3.7) |
| `Cancelled(reason)` | `TimedOut`, `Superseded` (PD5, or the broker quits), `BrowserNotOpened`, `GoogleError` (any other `error`, or an issuer other than Google's in the callback), `ClientRejected` (Google does not accept the client: `invalid_client`, `unauthorized_client`), `Network` (the token or key request failed), `IdentityCheckFailed` (3.5), `UnexpectedScope` (PD9), `NoRefreshToken`, `TokenStore` |

A flow that ends in anything but `Connected` stores nothing, and wipes from memory whatever Google issued. It does
**not** ask Google to revoke: as far as is known a revocation ends the whole grant of that person to this client,
and that would end the working token of an account that is already connected (FA14, LV5). Tokens that were issued
and wiped are held by nobody and age out at Google.

### 3.3 The loopback listener

- Binds `127.0.0.1:0`, IPv4 only, never another interface.
- Lives at most 300 s from `Flow::start`, or until the flow is superseded (a flag it reads at least every 250 ms).
  Nothing else ends it without a result: no number of wrong requests does, so that neither another local program
  nor a web page can end a sign-in by sending requests.
- Serves up to 8 connections at once; a ninth waits. Each has 2 s to send its request head of at most 8 KiB. No
  body is read. (A browser opens connections it does not use; one slot would let any of them delay the callback.)
- Accepts only `GET` with the path `/` and a `Host` of `127.0.0.1:<port>` (a guard against a web page that rebinds
  a name to the loopback address). Any other method, path or host is answered 400 or 404 (`/favicon.ico`: 404).
- A request for `/` must carry `state` exactly once. It is compared with the flow's state in constant time. A wrong
  or missing state is answered 400 and does not end the flow.
- The first request with the right state ends the listener, whatever else it carries: `code` exactly once, or
  `error` exactly once; both or neither is `Cancelled(GoogleError)`. An `iss` parameter, when present, must be
  `https://accounts.google.com` (G-DOC: Google always sends it, RFC 9207); another value is
  `Cancelled(GoogleError)`. Every other parameter (`scope`, `authuser`, `prompt`, `hd`, `error_description`, ...) is
  ignored and dropped. The listener closes before the token request is made, so a code can be presented once.
- The answer is one fixed page: "You can close this window and return to Fabric Calendar." No parameter is echoed.
  Headers: `Cache-Control: no-store`, `Content-Type: text/html; charset=utf-8`, `Connection: close`.

### 3.4 The token request

`POST https://oauth2.googleapis.com/token`, form encoded: `grant_type=authorization_code`, `code`, `code_verifier`,
`client_id`, `client_secret`, `redirect_uri` (the same string as in 3.2.1). One attempt, 30 s. It is not repeated: a
code is single use, and a request that may have reached Google has used it.

The answer must be JSON with `access_token`, `expires_in`, `token_type` equal to `Bearer` without regard to case,
`scope`, `id_token` and `refresh_token`. `expires_in` is held to 60 to 86 400 s. `refresh_token_expires_in`, when
present (G-DOC: time-based access), gives the time the connection ends; it is passed on so the owner can be shown
it (ST1). Unknown members are ignored (G-DOC). A missing `refresh_token` is `Cancelled(NoRefreshToken)`; G-DOC says
installed apps always get one. An error answer is `Cancelled(ClientRejected)` for `invalid_client` and
`unauthorized_client`, otherwise `Cancelled(GoogleError)`; its `error_description` is dropped.

### 3.5 The identity token

The `id_token` is a JWT. Every check must hold; a failure is `Cancelled(IdentityCheckFailed)` and the reason is a
fixed label, never a part of the token.

1. Three base64url parts; the whole token is at most 16 KiB. Header and payload are JSON objects, read by a strict
   reader that refuses a member that appears twice.
2. Header `alg` is exactly `RS256`. `none`, `HS256` and every other value are refused before any key is looked at.
   A header with `crit` is refused. `jku`, `jwk`, `x5u` and `x5c` are never used: the key comes from 3 only.
3. Header `kid` is present and names a key of Google's key set (`https://www.googleapis.com/oauth2/v3/certs`), with
   `kty` `RSA`, `alg` absent or `RS256`, `use` absent or `sig`, a modulus of 2048 to 8192 bits and the exponent
   65537. The key set is fetched over TLS, kept in memory for the `max-age` of its `Cache-Control` (not under 5
   minutes, not over 24 hours), and fetched again once when a `kid` is unknown, at most once a minute.
4. The signature over `header.payload` verifies with that key: RSASSA-PKCS1-v1_5 with SHA-256 and its DigestInfo
   prefix.
5. `iss` is `https://accounts.google.com` or `accounts.google.com`.
6. `aud` is the client id (a string, or an array holding only it). `azp`, when present, is the client id.
7. `exp` is later than `iat` and later than now minus 60 s. `iat` is not later than now plus 60 s and not earlier
   than now minus 600 s.
8. `nonce` is present and equals the flow's nonce, compared in constant time (FA13).
9. `sub` is 1 to 255 printable ASCII characters.
10. `email_verified` is `true`, and `email` fits the contract's `email` after its domain is put in lower case. The
    owner must be able to see which account connected (PLAN S01), so an account without a verified address that
    the contract can hold does not connect.

`at_hash` is not checked and not needed: the identity token and the access token arrive together in one answer
from Google over TLS, not through the browser. The access token is not used to ask Google who the account is; the
identity token is the only source.

### 3.6 Granted scopes

The `scope` of the token answer is split at spaces. `email` and `https://www.googleapis.com/auth/userinfo.email`
count as the same scope. Then:

| Granted | Outcome |
|---|---|
| `openid`, the address scope and all three calendar scopes, nothing else | continue to `Connected` |
| the address scope or any of the three calendar scopes missing | `PartialGrant { missing }`: store nothing (PD8) |
| `openid` missing (then there is no usable identity token) | `Cancelled(IdentityCheckFailed)` |
| anything beyond the five | `Cancelled(UnexpectedScope)`: store nothing (PD9) |

G-DOC: a person can untick single scopes on the consent screen, and the app "must verify which scopes were actually
granted". The granted list is given to the core (ST1) and checked again by the core before it calls an account
connected. Google may list scopes this person granted to the same client earlier; `UnexpectedScope` then repeats
until the person removes Fabric Calendar's access in the Google account, and the page must say so (V3-2).

### 3.7 How the outcomes map to the contract's account states

The job names what the subject must be: for a reconnect the account's stored subject; for a new account the set of
subjects already connected, none of which it may be. The adapter compares before it writes anything, so
`AccountMismatch` never touches a stored token.

| Contract state | Set when | Stored token of the account afterwards |
|---|---|---|
| `awaiting_consent` | `connect_google` committed; the flow runs | as before |
| `connected` | `Connected` | the new one |
| `consent_denied` | `ConsentDenied` | none |
| `partial_grant` | `PartialGrant` | none |
| `cancelled` | `Cancelled(_)`; also every `awaiting_consent` account at the next start of the broker (CO3) | none |
| `account_mismatch` | `AccountMismatch` | none |
| `reauthentication_required` | Google refused the refresh token, or the token store holds none for a connected account (3.8) | none |
| `disconnected` | `disconnect_google` | none |

Only `connected` counts as connected for a proof (THEORY `Account_connected`). Every other state blocks every agent
action on that account.

A reconnect of an account that was `connected` and that ends in anything but `Connected` leaves it not connected,
and its old token is deleted with its cached access token (`Forget`): the owner asked to change the connection and
it did not succeed, so the broker stops and holds no credential for an account it does not use (fail closed;
OQ12). The next successful `connect_google` restores it. Grants of the account that have not ended stay; they are
usable again only when the account is `connected` again, which is always the owner's own act.

A new connect (`account_id: null`) whose subject is already an account ends `AccountMismatch` for the new record
and leaves the existing account and its token untouched. So one Google account is never two accounts. Unfinished
records do not pile up: CO2.

### 3.8 Refresh

Before a Calendar call the adapter takes the account's access token from `AccessCache`. When there is none, or it
ends within 60 s, it refreshes: `POST` to the token endpoint with `grant_type=refresh_token`, `refresh_token`,
`client_id`, `client_secret`. One attempt, inside the call's own deadline. A call never reaches the Calendar host
without a token, so whatever happens to the refresh, nothing was sent to the calendar.

| Answer | Result for the call |
|---|---|
| 200 with `access_token` and `expires_in` (held to 60 to 86 400 s) | the call goes on. A new `refresh_token` in the answer replaces the stored one. A `scope` member is checked as in 3.6: fewer or more than the five is `AuthLost(GrantEnded)` |
| `error` `invalid_grant` (with any `error_subtype`), or `admin_policy_enforced` | the stored refresh token and the cached access token are deleted; the call ends `AuthLost(GrantEnded)` (6.10) |
| `error` `invalid_client` or `unauthorized_client` | the same deletion; the call ends `AuthLost(ClientRejected)`: Google no longer accepts this client (a deleted client, a disabled secret), and signing in again will not help until the client is repaired |
| any other answer, or none | Transient |

G-DOC lists why a refresh token stops working: the person removed access, six months without use, the limit of 100
tokens per account and client, time-based access that ran out, and in testing mode seven days after consent. A 401
from a Calendar call drops the cached access token and is Transient; the next attempt refreshes.

### 3.9 Revocation

`oauth::revoke(account)`: `POST https://oauth2.googleapis.com/revoke`, form encoded, `token=<refresh token>`. One
attempt, 30 s. 200 is `Revoked`. 400 is `Revoked` when its `error` is `invalid_token` (Google no longer knows it;
the code is not documented, LV5), otherwise `NotConfirmed`. Anything else is `NotConfirmed`.

It is used in one place: `disconnect_google` with `revoke_provider_token: true` (CO4). The caller deletes the
stored token afterwards in every case: a token that could not be revoked is not kept around for a disconnected
account, and the owner is told to remove the access in his Google account (V3-3). The revocation is taken to end
the whole grant of that Google account to this client, on every Mac that uses the same client (FA14); the page
should say so before the owner chooses it.

### 3.10 What never happens

No embedded sign-in. No password, authorization code, verifier, token or client secret in an argument, the
environment, standard output, standard error, a log line or an error value. No incremental authorization (G-DOC:
not supported for installed apps): another scope means connecting again. No request to an address that came from a
caller, a file or Google's answer; the endpoints are constants. No token sent to a host other than the one it is
for (5.1). No revocation except the one the owner asks for at disconnect. Cross-Account Protection (Google's
security event feed) needs an HTTPS receiver and does not apply to a desktop broker.

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
  bytes. The account id is the broker's, never the address. The item stays on this Mac: it is not synchronized
  through iCloud and not readable before the first unlock (PLAN 10.1: a refresh token is never exported to another
  Mac); the exact attributes are set with phase 2 and checked in LV9. Status mapping as W04 3.2: item not found is
  `None`; `errSecUserCanceled` and `errSecAuthFailed` are `Refused`; `errSecInteractionNotAllowed` is `Locked`;
  any other status is `Failed`. Its one real test is `#[ignore]`d, uses the service
  `ai.paracels.fabric.calendar.google.test` and is the owner's manual check (LV9). It is not written in phase 1.
- `AccessCache`: account id to `(access token, end time)`, memory only, shared the same way.
- `forget(account)` removes the account from the token store and from `AccessCache` in one step. `AuthLost`, a
  failed reconnect and `disconnect_google` all go through it, so no access token outlives its refresh token.
- A token-store failure during a call happens before anything is sent: `Locked` is Transient (the Mac may be
  locked for the whole retry budget; the request then ends `retry_budget_exhausted`), `Refused` and `Failed` are
  Rejected with `account_authentication_required` and the fixed sentence "Fabric could not read this account's
  sign-in from the Keychain."
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
broker stops working with a clear error and needs an update; that is the safe direction. The loopback set exists
for tests; a broker may use it only with the test key source, never with the Keychain (CO10).

### 5.2 Rules of `http`

- HTTPS only for `Endpoints::google()`. Plain HTTP only for `Endpoints::loopback`, which a release build does not
  have.
- No redirect is followed. A 3xx is an answer like any other (6.10: Rejected).
- No proxy, whatever the environment says. TLS 1.2 or later, the system's trust store, no pinning.
- HTTP/1.1 only. No connection is kept for reuse: every request opens its own. The client repeats nothing by
  itself. (A reused connection that the server closed, and an automatic repeat, would both blur whether a request
  was sent.)
- No compression: `Accept-Encoding: identity`, and the size bounds below count the bytes as received.
- Every call has a deadline: the earlier of the order's deadline and now plus 30 s (PLAN 5). Connecting has at most
  10 s of it.
- A response body is read up to a bound and no further: 64 KiB for the token, revocation and error answers,
  256 KiB for the key set and for one event, 4 MiB for a list page. More is treated as no usable answer.
- A JSON body is read only from an answer that says `application/json`. It is read into `serde_json::Value`
  first and then into lenient structs: unknown members are ignored, missing optional members are absent, a member
  of the wrong type is an error. An answer of another content type keeps its status (Google answers some errors
  with a page), but has no readable body: for a 2xx that is no usable answer (6.10). An answer that is not a 2xx
  is held to 64 KiB whatever was asked for.
- `User-Agent: FabricCalendar/<crate version>`. No other identifying header.
- The result separates three cases, which 6.10 needs. **Nothing was sent**: name lookup, connecting or the TLS
  handshake failed, or the deadline had passed before the request was handed to the client. **A complete answer
  arrived.** **Sent and unanswered**: every other failure, a time-out of any other kind among them.
- No error of the HTTP, JSON, URL or RSA libraries, and no address, is ever formatted, logged, kept in an error
  value or passed to a panic: the HTTP library prints the address (which holds calendar ids, windows and page
  tokens) and the JSON library quotes values. Each is turned into a fixed label inside `http` or `id_token`.
  Requests and responses have hand-written `Debug`. `Secret` wipes its own memory; copies the libraries make in
  their buffers are not wiped (RK14).

### 5.3 Ids in paths and headers

An agent writes event ids and revisions (the contract allows any printable ASCII up to 1024 characters), and Google
writes calendar ids. Before any request:

- An **event id** must match `^[A-Za-z0-9_]{1,1024}$`. Google's own ids are of this form (base32hex, a leading
  underscore for imported events, and `_<time>` for an occurrence); an id of another form is Rejected
  `invalid_arguments` and no request is made. In particular `.`, `..` and their percent-encoded forms never reach
  the URL library, which would fold them into the path of another Calendar method.
- A **reserved id** for a create must match `^[0-9a-v]{5,1024}$` (G-DOC: lower-case base32hex).
- A **revision** must be one strong entity tag: a double quote, 1 to 256 characters of printable ASCII other than
  the double quote, a double quote. `*`, a `W/` prefix and a list are Rejected `invalid_arguments` with no request:
  G-DOC says `If-Match: *` forces a change through, which would undo PD14.
- A **calendar id** is taken only from a calendar list. An entry whose id is empty, longer than 1024 characters,
  not printable ASCII, holds `/`, or is `.` or `..` is left out of the list (6.2).
- The path is built from segments, never by formatting a string: each id is one segment, percent-encoded (every
  character except `A-Z a-z 0-9 - . _ ~`). Query values are encoded the same way.

---------------------------------------------------------------------------------------------------------------------

## 6. Calendar operations

### 6.1 Common rules

Every operation takes the broker's account id and Google's calendar id (the core resolves the broker's calendar id
first, section 8), a deadline, and for a change the approved contract `Action`. Every operation uses the token of
the named account only. No operation names `primary` (PD22). Every result is built from typed values; nothing
Google wrote is passed through as text except event content in a read result or a before-image.

**"The account's own entry."** Google marks one attendee entry with `self: true`. G-DOC: it "represents the calendar
on which this copy of the event appears", not the account that is signed in. On a calendar that another person
shared with the account, the marked entry is that other person. So the adapter treats a marked entry as the
account's own only when the calendar is the account's primary calendar (the order says so, from the calendar list,
PO1) and the entry's address is that calendar's id. Everywhere below, "own entry" means that. On every other
calendar there is no own entry: no invitation to answer, no own response to show.

### 6.2 Calendar list

`GET /users/me/calendarList?maxResults=250&minAccessRole=freeBusyReader&showHidden=true[&pageToken=..]`, page
after page (Google may fill a page with fewer than it was asked for). More than 250 calendars in all fails the
listing (PD23, ST2); a cut list would look like lost calendars (CO7). Deleted entries are not asked for. Hidden ones are: hiding a calendar in Google's page is a display choice and takes no access away.

Result per entry: Google's `id` (5.3); the name (`summaryOverride`, else `summary`, cut to 200 characters, "(no
name)" when empty); `primary`; `timeZone`; the access role:

| Google `accessRole` | Contract `calendar_access` | Writable |
|---|---|---|
| `owner` | `owner` | yes |
| `writer` | `writer` | yes |
| `writerWithoutPrivateAccess` | `reader` (PD26, until V3-1) | no |
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
- G-DOC states no limit on the length of the window; LV8.

### 6.4 Reading events

Per calendar: `GET /calendars/<id>/events` with `singleEvents=true`, `orderBy=startTime`, `showDeleted=false`,
`timeMin`, `timeMax` (the window; G-DOC: `timeMin` bounds an event's end, `timeMax` its start, both exclusive, so
this is "every event that overlaps the half-open window"), `timeZone=<the calendar's zone>` (6.9), `maxResults=250`,
`fields=` (below) and `pageToken` for later pages. `maxAttendees` is not sent, so Google returns every attendee.

- Recurring events arrive as their occurrences (PLAN 10.3: provider-expanded occurrences). A recurring master is
  never returned and never treated as one event.
- One read order reads every page of every asked calendar, inside the order's one deadline. Before each page the
  adapter asks the caller's `still_allowed` check, which the core turns false the moment a pause, a revocation or
  any other change of the fence commits (PO3). When it is false the read stops: `Stopped`, no further request, and
  nothing of what was gathered is returned. So the handoff of each page checks authority (PLAN 8), and the core's
  lifecycle sees one dispatch.
- A result exists only when the last page of the last calendar arrived. A failure at any page leaves nothing
  (PD23).
- Bounds (PD23), counted over all pages and calendars: at most 2 500 events and at most 768 KiB of canonical
  result. Past either, the read fails: "The window holds more events than one answer can carry. Ask for a shorter
  window." The same failure when an event has `attendeesOmitted: true` or more than 2 000 attendees and attendees
  were asked for: the contract's list cannot hold them, and a cut list is never shown as whole.
- An item with `status` `cancelled` is skipped.
- On a calendar where the account has the role `reader` (or Google's fifth role), Google returns a private event
  without its details. It appears in the result with its time and empty content; the adapter cannot tell it from
  an event that has no title.

`fields=` asks for `nextPageToken` and, per item, always `id,etag,status,recurringEventId`, and then by granted
field:

| Granted field | Members asked | In `EventView` |
|---|---|---|
| `time` | `start,end,originalStartTime` | `time`; `occurrence` (instance id and original start) for an occurrence of a timed series |
| `title` | `summary` | `title` (empty when Google has none) |
| `location` | `location` | `location` |
| `description` | `description` | `description` |
| `attendees` | `attendees(email,responseStatus,self),attendeesOmitted` | `attendees` |
| `organizer` | `organizer(email)` | `organizer` |
| `own_response` | `attendees(email,responseStatus,self),attendeesOmitted` | `own_response`: the status of the own entry (6.1); `null` on a calendar that is not the account's primary |

A field that was not granted is `null` in the view even if Google sent it (PD24). `occurrence` holds the original
start, which is a time, so it is disclosed only with `time`; without it an occurrence looks like a single event and
cannot be answered one by one. An occurrence of an all-day series has a date, not a time, as its original start;
the contract's `occurrence` cannot hold it, so it is `null` and such an occurrence cannot be answered one by one.

The read order names each calendar by both ids (the broker's and Google's), its zone and whether it is the
account's primary, and carries the ids of the events Fabric created in those calendars; the adapter puts the
broker's calendar id and `fabric_created` into each view from them (PD19, PO3).

### 6.5 Observing one event

`observe(target, event id)`: `GET /calendars/<id>/events/<event id>?timeZone=<the calendar's zone>`. `target` is
the calendar as in 6.4, with `fabric_created` for this event. It is the broker's own read of the event an action
names, made before each proof on such an action (PO2), and gives:

| Member | From Google's event |
|---|---|
| `event_id`, `revision` | `id`, `etag` (verbatim) |
| `recurring` | `recurrence` present, or `recurringEventId` present |
| `occurrence_start_ms` | `originalStartTime.dateTime` of an occurrence, as UTC milliseconds; absent for an occurrence of an all-day series |
| `default_type` | `eventType` absent or `default` |
| `timed` | `start.dateTime` present (an all-day event has `start.date`) |
| `invitation` | an own entry exists (6.1) and `organizer.self` is not `true` |
| `attendees` | the `email` of every attendee entry except the own entry, as written, domain in lower case. Rooms and the organizer's entry are among them. On a calendar that is not the account's primary, the entry marked `self` is another person and is among them |
| `before` | an `EventView` with every content field, for the owner's card only (`private.before`) |

Results: `Found`, `Absent` (404, 410, or `status` `cancelled`), or a read failure (6.10). The observation fails
with `invalid_arguments` when `attendeesOmitted` is `true` or when an attendee address does not fit the contract's
`email` (PD25): the check of who is notified must see everyone.

### 6.6 Creating an event (`fabric_calendar_propose_create`)

`POST /calendars/<id>/events?sendUpdates=none` with the body

```json
{ "id": "<reserved id>", "summary": "<title>",
  "start": { "dateTime": "<date_time>", "timeZone": "<zone>" },
  "end":   { "dateTime": "<date_time>", "timeZone": "<zone>" },
  "description": "<description, when given>", "location": "<location, when given>" }
```

and nothing else: no attendees (CR 14 item 21), no recurrence, no event type (the default type). Everything else
keeps Google's default (OQ13 names one of them).

| Answer | Outcome |
|---|---|
| 200 with an event whose `id` is the reserved one | Applied, with `(id, etag)` and `confirmed`: whether the returned event equals the approved one (6.12) |
| 200 with another `id`, or a body that is not an event | NoAnswer |
| 409 | Rejected `provider_rejected` ("an event with this id already exists"). No earlier attempt of this effect can have made it (PD17), so the event is not Fabric's, and it is never recorded as created |
| others | 6.10 |

A retry after a Transient attempt sends the same body with the same reserved id (PLAN 11). An event counts as
created by Fabric only when Google answered Fabric's own POST with it, or when reconciliation finds under the
reserved id exactly what was approved (section 7; ST6).

### 6.7 Changing an existing event

All three are `PATCH /calendars/<id>/events/<event id>?sendUpdates=<approved>` with `If-Match: <approved
revision>` (PD14, PD15). `sendUpdates` is always written: `all`, `externalOnly` or `none`.

| Tool | Reads first | Body |
|---|---|---|
| `propose_update` | no | only the members that are set: `summary`; `start` and `end` as `{"dateTime", "timeZone", "date": null}`; `description`; `location` |
| `propose_invitation` | the event (GET) | `{"attendees": [...]}`: the approved list is the complete list after the change (CR 14 item 21). Each approved address in the approved order; an address that was an attendee keeps its whole entry, a new one is `{"email": ...}`. The own entry (6.1), when the event has one, is kept as it was whether or not the approved list names it. Every other entry that is not in the approved list is removed, the organizer's and a room's among them; the owner's card must show who is removed (CO6) |
| `propose_response` | the event (GET) | `{"attendees": [...]}`: the attendee list exactly as read, with the `responseStatus` of the own entry set to `accepted` or `declined`. Nothing else differs |

- The read, where there is one, must return the approved revision and must not have `attendeesOmitted: true`;
  otherwise Rejected (`revision_conflict`, or `invalid_arguments` for a cut list) without a PATCH. For an answer to
  one occurrence the path names the instance id, and the instance's `originalStartTime` must be the approved
  instant; otherwise `revision_conflict` (CD-Q05b: instance id, original start and revision are bound together).
- An answer needs an own entry (6.1); without one it is Rejected `provider_rejected` ("this account is not invited
  to this event on this calendar"). So an invitation of another person, seen on a calendar that person shared, is
  never answered.
- G-DOC also describes a shorter body for an answer (`attendeesOmitted: true` with the one entry). It is not
  used: if Google did not honour the flag, the other attendees would be removed. The full list under `If-Match`
  cannot remove anyone. If Google refuses the full list from an attendee (LV7), the shorter body is the fallback.

| Answer | Outcome |
|---|---|
| 200 with the event | Applied, with `(id, new etag)`, `confirmed` (6.12) and `notification` (`Sent` when `sendUpdates` is not `none`, else `NotRequested`) |
| 200 with another `id`, or a body that is not an event | NoAnswer |
| 412 | Rejected `revision_conflict` |
| 404, 410 | Rejected `revision_conflict` ("the event is gone") |
| 409 | Transient (G-DOC: a conflict with another request in progress; nothing was applied) |
| others | 6.10 |

### 6.8 Deleting an event (`fabric_calendar_propose_delete`)

`DELETE /calendars/<id>/events/<event id>?sendUpdates=<approved>` with `If-Match: <approved revision>`.

| Answer | Outcome |
|---|---|
| 204 or 200 | Applied, no event, `notification` as in 6.7 |
| 412 | Rejected `revision_conflict` |
| 404, 410 | Rejected `revision_conflict` ("the event is already gone"). Fabric did not delete it and sent no notice; the agent reads again |
| others | 6.10 |

That only an event Fabric created may be deleted is the theory's rule (`Fabric_created`) on the store's record
(PD19); the adapter does not decide it. Whether the deletion is in effect is confirmed by a read afterwards
(section 7; PLAN 10.3 CD-Q05a).

### 6.9 Translation (the time model; ADR-111)

| Contract | Google | Rule |
|---|---|---|
| `event_time { date_time, zone }`, written | `{ "dateTime", "timeZone" }` | As the action gives them |
| `event_time`, read | `dateTime`, with the request's `timeZone` | Every events read names the calendar's zone as `timeZone`. Google then writes each `dateTime` with that zone's offset. The view's `zone` is the calendar's zone, so offset and zone agree. A `Z` becomes `+00:00`; a fraction of a second is dropped |
| all-day | `{ "date" }` | Read only, as `start_date` and `end_date` (the end exclusive, as Google has it). Never converted to an instant. Never written |
| `window` | `timeMin`, `timeMax` | The contract's timestamps as they are (RFC 3339, UTC) |
| busy `start`, `end` | RFC 3339 | Converted to UTC with milliseconds |
| `revision` | `etag` | Verbatim, quotes included; sent back verbatim in `If-Match` (5.3) |
| `event_id`, `instance_id` | `id` | Verbatim (5.3) |
| `send_updates` `all`, `external_only`, `none` | `sendUpdates` `all`, `externalOnly`, `none` | |
| `response` `accepted`, `declined` | `responseStatus` | A read also knows `needsAction` and `tentative` |

An event keeps its own zone at Google (`start.timeZone`), which may differ from the calendar's. A view does not
show it: pairing Google's offset with the event's own zone would give a pair that disagrees (an event made in
Berlin on a New York calendar would read `-05:00` with `Europe/Berlin`), and the broker refuses such a pair in a
proposal (CR 5). The event's own zone stays in the raw evidence (ST3). An agent that changes a start or an end
writes a zone with it, and the owner sees that zone on the card.

The adapter does no zone arithmetic. It parses RFC 3339 to an instant only to compare instants (6.12, section 7)
and to cut busy periods to the window. Whether an offset agrees with its zone, and whether a changed start is
still before the end, are the broker's checks (CR 5; CD-W09; CO6); Google refuses an end before a start with 400.

### 6.10 Outcome classes (PD17, PD18)

**Transient means it certainly did not happen**: nothing was sent, or Google refused before acting. Everything
else that is not a plain yes or a plain refusal is an unknown outcome. For a change:

| What happened | Outcome |
|---|---|
| Nothing was sent (5.2), or the deadline had passed before sending | Transient |
| Fewer than 10 s of the deadline are left when the changing request is due | Transient, and nothing is sent: with less than the connect limit left, a connection that is cut off by the deadline could not be told from a request that was sent |
| The request was sent and no complete, readable answer arrived | **NoAnswer** |
| 5xx | **NoAnswer**: Google may have made the change and failed afterwards |
| 2xx as 6.6 to 6.8 | Applied |
| 400 | Rejected `invalid_arguments` |
| 401 | Transient; the cached access token is dropped (3.8) |
| the refresh before the call ended `AuthLost` | Rejected `account_authentication_required`, and `AuthLost(cause)` for the account |
| 403 or 429 with reason `rateLimitExceeded` or `userRateLimitExceeded` | Transient, with `Retry-After` in whole seconds when present (at most 120 s) |
| 403 with reason `quotaExceeded` | Rejected `provider_rejected` ("Google's usage limit for this calendar is reached") |
| 403 with reason `forbiddenForNonOrganizer` | Rejected `provider_rejected` ("only the organizer can change this") |
| 403 with reason `insufficientPermissions` | Rejected `account_authentication_required`, and `AuthLost(GrantEnded)`: the token no longer carries the scope |
| 403 with any other reason | Rejected `provider_rejected` ("Google refused the change for this account"), and `AccessChanged` for the account, so the core lists its calendars again |
| 404, 409, 410, 412 | 6.6 to 6.8 |
| any other status, a 3xx among them | Rejected `provider_rejected` |

The reasons are read against this closed list; an unknown reason is "any other" (PD27).

For a read (6.2 to 6.5, and the reads of section 7) there is no NoAnswer: an unanswered read and a 5xx are
Transient. 404 on a calendar is Rejected `provider_rejected`; 404 on an event is `Absent`; the other rows hold.

`AuthLost` and `AccessChanged` are side results beside the outcome (PO6).

### 6.11 The retry rule (CD-Q11e)

`next_attempt(policy, retries_made, first_attempt_at_ms, now_ms, hint_ms) -> RetryDecision` with
`RetryPolicy::PLAN { max_retries: 3, backoff_ms: [2000, 4000, 8000], budget_ms: 120_000, call_ms: 30_000 }`:
wait `max(backoff_ms[retries_made], hint_ms)` when `retries_made < 3` and the wait ends inside the budget; otherwise
give up. It is the rule the core already applies to a dispatch (W04 D16), written once more as a plain function so
that the reads and jobs the core adds use the same numbers (CO9); a test holds the numbers equal to W04's, and
phase 2 keeps one copy. The adapter itself never waits and never repeats (PD13). The waits have no random part:
one broker serves one person, and CD-Q11e fixes the numbers.

Why a retry is safe: by PD17 a retry follows only attempts that certainly changed nothing. The core proves each
retry afresh on a new observation of the event (CO6); had an earlier attempt been applied, that observation would
show another revision and the retry would end as a conflict for a change that was in fact made. PD17 rules that
out by sending every doubtful answer to reconciliation instead.

"No blind resend of a notification" then holds twice over. A change whose outcome is in doubt is never sent again
(section 7). And the only requests that can notify, the PATCH and DELETE of 6.7 and 6.8, are conditional on the
approved revision, in a form that cannot be a wildcard (5.3); a send that was applied changed the revision (FA16),
so a second send could not be applied. Test g41 holds every such request to it.

### 6.12 Comparing what Google returned with what was approved

`matches(action, event) -> bool`, used in 6.6, 6.7 and section 7:

- Create: `summary` equals the title; `start` and `end` are the approved instants; `description` and `location`
  equal the approved ones (absent and empty are equal); no attendees. The zone is compared when Google returns the
  event's own `timeZone`.
- Update: each member the action sets, compared as above. Members it does not set are not compared.
- Invitation: the set of attendee addresses, without the own entry and without the account's own address if the
  approved list names it, equals the approved set (local part exact, domain without regard to case).
- Response: the own entry has the approved status.

`confirmed`, for the answer to the adapter's own PATCH, also requires for a response that every other attendee
entry is as it was read.

---------------------------------------------------------------------------------------------------------------------

## 7. Reconciliation (out of `outcome_unknown`; ADR-113)

A change whose outcome is unknown is never sent again. The broker reads and decides. The read is the broker's own
action (`System_reconcile account calendar event`, THEORY) and needs its proof (PD29).

The adapter's answer to a reconcile order says only what Google holds: `Found` (an event that is not cancelled,
with its revision and whether it equals what was approved, 6.12), `Cancelled`, or `NotFound` (404 or 410). Nothing
in it is Google's own shape. The table below is then a pure function of that answer,
`reconcile::decide(tool, approved revision, reads before, read_back) -> Verdict`. The lead's answer of 2026-11-17
is that the core owns this table (18.5); `decide` in GC is the table as plain code with its tests, for the core to
take over.

| Tool | Read back | Verdict | Lifecycle (CR 9) |
|---|---|---|---|
| create | found, and it equals the approved create (6.12) | `Applied` with `(id, revision)`, `confirmed` | `reconciling -> applied` |
| create | found, and it does not | `NeedsOwner` ("an event exists under this request's id and is not what was approved") | `needs_owner_resolution` |
| create | cancelled | `NeedsOwner` ("the event was created and then deleted") | `needs_owner_resolution` |
| create | not found | `NotYet` | below |
| update, invitation, response | found, and it equals the approved change, whatever the revision | `Applied`, `notification: Unknown` | `applied` |
| update, invitation, response | found, the revision is the approved one, and it does not equal the change | `NotYet` | below |
| update, invitation, response | found, another revision, and it does not equal the change | `NeedsOwner` ("the event changed, and Fabric cannot tell whether its change is part of it") | `needs_owner_resolution` |
| update, invitation, response | cancelled or not found | `NeedsOwner` ("the event is gone, and Fabric cannot tell whether its change was made first") | `needs_owner_resolution` |
| delete | cancelled or not found | `Applied`, no event, `notification: Unknown` | `applied` |
| delete | found, the revision is the approved one | `NotYet` | below |
| delete | found, another revision | `Absent`: it exists and changed, so the conditional delete cannot have been made and cannot be made later | `absence_confirmed` |

- **`NotYet` (PD20).** The change is not there now, but Google may still be finishing the request the broker
  stopped waiting for, and G-DOC gives no bound. On the first read `NotYet` is `ReadAgain`, no sooner than 60 s
  later; on the second it is `Absent` (`reconciling -> absence_confirmed`). This holds for every tool: a late
  PATCH or DELETE still matches its `If-Match` as long as nobody else changed the event.
- `earliest_read_ms(attempt_finished_ms)` is the attempt's end plus 10 s. After a restart the end of the attempt is
  not known; the core uses the start of its recovery plus 10 s (CO8).
- A read that fails is Transient or Rejected by 6.10; the core repeats it by 6.11 and after the budget moves the
  request to `needs_owner_resolution` (CO8).
- Notification: a change with `send_updates` other than `none` that is found applied by reading carries
  `notification: Unknown`: whether Google sent the messages is not known. Nothing is sent again (PLAN 11). The
  owner sees it (V3-4).
- `absence_confirmed` ends the request. The agent may ask again; that is a new request with a new proof and, for a
  create, a new reserved id. The two reads of PD20 are what stands between that and a change made twice (RK3).

`applied -> reconciled`: for an insert or a PATCH the answer itself is Google's statement of the event after the
change. When it is `confirmed` the core may move on to `reconciled` without another read. For a delete (no body),
or when the answer is not `confirmed`, the core runs one reconcile read and records what it finds; the request
then ends `reconciled` with the observation in its details, matching or not (CO8). Recommended; the lead may
prefer a read after every change, which costs one proof and one call more each.

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

What the adapter does (phase 1): its answer to a read order goes to the caller and nowhere else. It has no channel
to an agent and keeps nothing after it returns.

What the core must do is a request to the lead (CO5, ST4). The proposal:

1. A read request (availability, event details) runs the pipeline to a fresh permit like a change. The handoff of
   the read order is its dispatch point: no read reaches Google without a consumed authorization (PLAN 5 R04:
   "disallowed calendar/range/field requests never reach provider or leave broker"). `fabric_calendar_list` is
   answered from the broker's own records and makes no provider call (CO13).
2. The core stores the adapter's answer as the request's held result in the encrypted store. No part of it goes
   into a status, a notice or a redacted detail. The read request then ends (`dispatching -> applied ->
   reconciled`): a result that waits for its release should not hold one of the agent's open-request places.
3. `fabric_request_result` is a request of its own. Its proof is on `Disclose(a)`, where `a` is the read. THEORY
   then requires `Result_held` and, for a read, the authority of every row of `a` against the grants as they are
   now (`releasing_a_read_needs_its_authority`).
4. Only a fresh permit on that `Disclose` releases the result, in that request's tool outcome. The release is
   recorded; a one-request grant is then used, and its held result is purged.
5. A held result is purged also when its account is disconnected, and 24 h after it was stored (OQ5).

This needs four changes to CD-W04 as built, all the lead's (CO14):

- D18 says a one-request grant is offered only to its origin request and ends when that request ends. For a read
  it must stay live after the read request ends, be offered to the `fabric_request_result` request that names the
  read, and end at the release or the purge.
- `GrantFact.used` for a read must mean "its result was released", not "its request reached `applied`".
- D17 sends a fresh permit for "a tool that changes no calendar", and one with no authorizing grant, to
  `undecided`. Both must be lifted for a read order and for a release. The release of a change's outcome has no
  row and so no authorizing grant at all (W04 R21).
- D33 goes.

**Question 10** ("polling a completed read after revocation discloses nothing"). With 1 to 5: after the grant is
revoked, a status poll returns the state and nothing else, by the contract's type. A `fabric_request_result` gets a
`Disclose` proof in which `Authority` no longer holds. `Authority` is an approval rule, so the outcome is
need-approval, not block; a `fabric_request_result` has no scope to put on a card, so W04 5.7 ends it `undecided`.
Nothing is released and the held result stays where it is. (So today a lost result can be read again only under a
grant that is still valid; the plan's "new consent" for a second release would need a card for a release, V3-10.)
The test belongs to the core, against FAKE as the provider: read to completion, revoke, poll, ask for the result,
and assert that no byte of the result's marker left the broker. It can be written when ST4 and CO5 exist. Phase 1
has no stand-in for it (15.2 row 7).

---------------------------------------------------------------------------------------------------------------------

## 10. What the provider port needs (request to the lead; PORTS)

Today `Provider::dispatch(&DispatchOrder) -> AttemptDone` only. Proposed, names free:

| # | Change | Why |
|---|---|---|
| PO1 | Every order names its calendar as a target: the broker's `CalendarRef`, Google's calendar id, the calendar's zone and whether it is the account's primary, all resolved by the core in the transaction that makes the order | The adapter must not read the store (W04 5.1) and must not be given an id by an agent; 6.1 and 6.9 need the last two |
| PO2 | An observe order: a target, the event id and whether Fabric created that event; under a system proof on `System_refresh account calendar` (the calendar is selected, or the action would not have passed its checks). Answered `Found(ObservedEvent, before: EventView)`, `Absent` or a failure. The core runs it before every proof on an action that names an event and fills `TrustedFacts.event` | THEORY `Event_known`, `Revision_current`, `Event_supported`, `Invitation_target`; it replaces the practice hook (PW 4.4) |
| PO3 | A read order for availability and for event details: the targets, the window, the fields, the ids of Fabric-created events in those calendars, and a `still_allowed` flag the core turns false when the fence changes. One order reads every page (6.4). Answered `Done(ToolResult)`, `Stopped` or a failure | D33 lifted; PLAN 8 (each page's handoff checks authority) without a lifecycle edge per page: the contract has none from `dispatching` back, and an authorization is used once |
| PO4 | A reconcile order: the effect and its target, under a system proof on `System_reconcile`. Answered with the read-back in a form that names nothing of Google (found with its revision and "equals what was approved", cancelled, not found); the core owns the table of section 7 | Section 7; 18.5 |
| PO5 | A calendar-list order per account, under a system proof (TH1) | 6.2; CO7 |
| PO6 | `AttemptReport::Applied` gains `confirmed: bool` and `notification: Sent, NotRequested or Unknown`; every result may carry `AuthLost(account, cause)` or `AccessChanged(account)` | Section 7; 3.8; 6.10 |
| PO7 | Sign-in is not a provider order. It runs on its own worker: `Connect(account, the stored subject, or for a new account every subject that any account record has)` answered with `ConnectOutcome`; `Revoke(account)` answered `Revoked` or `NotConfirmed`; `Forget(account)` | A sign-in waits up to 300 s and must not hold up a dispatch; 3.7 |
| PO8 | Every order names the proof that permits it (PD29); the worker refuses one that does not | One invariant to test: no Calendar call without a proof id |

The adapter maps its `ChangeOutcome` one to one onto `AttemptReport`. Phase 2 adds that module and the dependency
on the core. The practice calendar is the other implementation of the port: it must answer PO2 and PO4 too, or a
practice run stops at `applied` (section 16 step 6).

---------------------------------------------------------------------------------------------------------------------

## 11. What the store must keep (request to the lead)

W04 3.6 reserves the next migration for this package; the lead numbers the migrations of CD-W05 and CD-W06 and has
asked that this document give no number (18.5). Additive only, as W04 3.6 and R22 require: new tables, new
nullable columns. No existing column changes. No
contract value that the store keeps changes shape (the contract session's rule); what is new about an account
lives in ST1, beside the contract's `Account` record, not inside it.

| # | What | Proposed shape |
|---|---|---|
| ST1 | The Google side of an account | `provider_accounts(account_id PK, provider 'google', subject UNIQUE, granted_scopes JSON, token_ref, connected_at_ms, refresh_token_ends_at_ms, calendars_listed_at_ms, last_connect_problem, revocation_state)`. No token. One record per subject, for good |
| ST2 | The calendar map | `provider_calendars(account_id, calendar_id, provider_calendar_id, access_role, is_primary, time_zone, listed, first_seen_ms, last_seen_ms, PRIMARY KEY(account_id, calendar_id), UNIQUE(account_id, provider_calendar_id))`. At most 250 per account (6.2) |
| ST3 | Observations | Per request and proof: the facts (`revision`, the flags of 6.5, the attendee addresses, `observed_at_ms`) as a projection row; the before-image and the event as Google returned it (PLAN 10.3 "raw encrypted evidence", at most 256 KiB) in a table outside the ledger's payload, named in the ledger by digest. Kept with the request; never purged by a disconnect (CD-Q11h) |
| ST4 | Held read results | A projection row per request (`request_id, kind, digest, bytes, observed_at_ms, stored_at_ms, released_count, purged_at_ms`) and the payload outside the projection and the ledger payload, so that it can be purged. The ledger row names the digest, not the content (W04 R13) |
| ST5 | Effects | Nullable columns: `revision_before`, `revision_after`, `confirmed`, `notification`, `reconcile_reads`, `next_reconcile_at_ms` |
| ST6 | "Fabric created this" | A create effect that reached `applied`: by Google's answer to Fabric's own POST, or by a reconcile read that found exactly what was approved. `effects_provider_event` (W04 3.5) already keys it by account, calendar and event id |
| ST7 | Open effects | `ReadTx::open_effects` must also return a change that is `applied` and not yet `reconciled`, if a read after a change is kept (section 7): THEORY authorizes `System_reconcile` only for a pending effect |
| ST8 | Mutations and operation types | For each row above, and for `owner.connect_google`, `provider.connect`, `provider.calendars`, `provider.observation`, `provider.auth_lost`, `request.read`, `request.release`, `request.reconcile` |
| ST9 | A backup still holds no token | True by ST1 |
| ST10 | Budget of the snapshot | Accounts are not in W04's D38 budget (R24). An account with 250 calendars is about 60 KiB of snapshot. How many accounts fit is the lead's arithmetic and the owner's question (OQ14) |
| ST11 | Rehearsal | The migration run on an encrypted fixture of the version before it, with a verified backup, as W04 s08 does for its rehearsal (PLAN 14 "upgrade encrypted fixture database"). W04 R22 (replay is not version-aware) must be settled first; ST1 to ST5 only add |

---------------------------------------------------------------------------------------------------------------------

## 12. What the core must do (request to the lead)

| # | What |
|---|---|
| CO1 | `connect_google`: with `account_id: null` take the placeholder (CO2) or create an account; with an id, that account. State `awaiting_consent`, revision + 1, `revocation_epoch` + 1 (W04 5.6), notice, answer; then the `Connect` job. A second `connect_google` supersedes a waiting one (PD5) |
| CO2 | At most one account that never connected exists. A new connect reuses it, so failed attempts do not pile up as accounts |
| CO3 | Recovery (`broker.started`): every `awaiting_consent` account becomes `cancelled` |
| CO4 | `disconnect_google`: as today, then `Revoke` when `revoke_provider_token` is true, then `Forget` in every case; purge the account's held results, not its observations; record whether Google confirmed (ST1 `revocation_state`; V3-3). An effect of the account that is `outcome_unknown` or `reconciling` goes to `needs_owner_resolution` ("the account was disconnected before Fabric could check"); one that is `applied` goes to `reconciled`, recorded as not read back. Or the command warns first: the owner's call (OQ16) |
| CO5 | Reads, as section 9: the read order after a fresh permit; the held result; `fabric_request_result` as `Disclose`; `result_held` and `used` in the facts; the release in the outcome only. With R08's "revoke during pagination" as a test: the flag of PO3 |
| CO6 | Observation (PO2) before each proof on an action that names an event; `fabric_created` from ST6; `private.before`, with who an invitation change removes. What follows, on the lifecycle's existing edges: the table below. `Fence.event_revision` is the revision observed for that proof and nothing more: the handoff does not read Google again, and `If-Match` is what guards the gap (RK9). Also here: a changed start must stay before the end, checked against the before-image |
| CO7 | Calendar list (PO5): at connect, at each start, on `AccessChanged`, and before the owner chooses calendars. A lost calendar or a lower role is a reduction: `revocation_epoch` + 1, the grants that name it end `authority_reduced`, its waiting cards go `stale` (PLAN 10.2: "ACL downgrade must stop writes and stale approvals"). `CalendarFact.writable` stays "owner or writer" (PD26) |
| CO8 | Reconciliation: `outcome_unknown -> reconciling` at `earliest_read_ms`; system proof; reconcile order; `reconcile::decide`; `ReadAgain` waits 60 s; a failed read repeats by 6.11, then `needs_owner_resolution`. `applied -> reconciled` as section 7. While the account is `reauthentication_required` its open effects wait; they are reconciled when it is `connected` again (THEORY blocks `System_reconcile` on an account that is not connected) |
| CO9 | System proofs: `FactActor::System`; the system actions of THEORY and TH1; their digest (W04 CCR-14); a proof job without an agent request (the calendar list at connect has none), and with no authorizing grant, which D17 sends to `undecided` today; 6.11 for the retries of reads and jobs |
| CO10 | Configuration: one more broker variable for the client file's path in development, and a debug-only one that points the broker at a loopback port for process tests against FAKE, accepted only with the test key source (W04 D27 lists the names the broker accepts). In a release the client id and secret are part of the package (CD-W11). The token store follows the key source: Keychain with `keychain`, memory with `test` |
| CO11 | `AuthLost`: the account becomes `reauthentication_required`, `revocation_epoch` + 1; nothing is dispatched for it until it is `connected` again. `AuthLost(ClientRejected)` is recorded so the page can say that signing in again will not help (V3-2) |
| CO12 | A card that waits while its event changes: PLAN S03 wants the approval disabled then. Proposal: observe again when the owner opens the card's details, and at most every 10 minutes while it waits; another revision moves the request `pending_approval -> stale`. May be deferred: a stale approval is caught after the decision in any case (CO6) |
| CO13 | `fabric_calendar_list` is answered from the store (the selected calendars of ST2, with `agent_drafts`), after its proof, with no provider call |
| CO14 | The amendments to CD-W04 that section 9 lists (D17, D18, D33, `used`), and the reducer's rows for the transitions W04 left to this package (`outcome_unknown -> reconciling -> ...`, `applied -> reconciled`, `pending_approval -> stale`) |

What the core does with an observation (CO6), using only transitions the contract has:

| Observation | Before the classification proof (request in `validating`) | Before a fresh-permit proof (request in `proving_fresh_permit`, a retry included) |
|---|---|---|
| `Found`, the revision the action names | the proof runs | the proof runs |
| `Found`, another revision | `terminal_failure` (`revision_conflict`) | `stale` |
| `Absent` | `terminal_failure` (`invalid_arguments`, "no such event"); for an event Fabric created in the last five minutes, try again by 6.11 first (G-DOC: a 404 may pass) | `stale` |
| failed: Transient | stays, tries again by 6.11; then `terminal_failure` (`retry_budget_exhausted`) | stays, tries again by 6.11; then `undecided` (V3-9 asks for a reason to show) |
| failed: Rejected | `terminal_failure` with its code | `undecided` |

---------------------------------------------------------------------------------------------------------------------

## 13. Requests to the contract session

Choosing calendars with Agent Drafts, and resolving an unknown outcome, are already asked. These are added. They
are sent once this design has been through its critique. `Account` is a value the store keeps, so V3-2 and V3-3
must not change its shape without the lead's store design; a member of the snapshot beside `accounts`, or a notice,
would not.

Schema version 3:

| # | What | Why |
|---|---|---|
| V3-1 | `calendar_access`: a fifth value for Google's `writerWithoutPrivateAccess` (may write; sees private events without details). Until the contract has it the broker shows such a calendar as `reader` and does not write to it | PD26; G-DOC |
| V3-2 | Why a connection attempt ended, owner only: a closed list (`timed_out`, `superseded`, `browser_not_opened`, `google_error`, `client_rejected`, `network`, `identity_check_failed`, `unexpected_scope`, `no_refresh_token`, `token_store`, `already_an_account`), for `partial_grant` which permission was missing, and when a connection is due to end (`refresh_token_ends_at`) | The eight states say what, not why; the page cannot tell the owner what to do next. Same pattern as `prover_problem` |
| V3-3 | Whether Google confirmed the revocation at a disconnect: `not_requested`, `pending`, `revoked`, `not_confirmed`. Not in the command's result: the command answers after its commit, and the call to Google comes after | 3.9; PLAN 9.1 "offer explicit provider token revocation"; PLAN 5 (100 ms) |
| V3-4 | In a request's details: whether the notifications of an applied change are known to be sent | PLAN 11: "display that uncertainty" |
| V3-5 | A way for the owner to ask for a fresh calendar list of one account, with or beside the command that chooses calendars | 6.2; CO7 |
| V3-6 | Say that `occurrence` in an event view is disclosed with `time`, and is absent for an occurrence of an all-day series; or give it a date form | 6.4 |
| V3-7 | Optional: a read result that can say "incomplete". Without it a read past its bounds fails (PD23), which needs no change | |
| V3-8 | Proofs of the broker's own actions: a proof purpose beyond `classification` and `fresh_permit`, and a proof report that names no agent request (the calendar list at connect has none). With W04's CCR-14 (context and digest of the broker's own actions) and CCR-1 (`connect_google` gets implemented, the code goes) | CO9 |
| V3-9 | A reason for `undecided` when Google could not be reached to observe the event | CO6 |
| V3-10 | Whether a release may ask the owner: today a `fabric_request_result` has no scope, so a release that needs approval ends `undecided` | Section 9; PLAN 9.2 "require new consent" |

Rule theory (same owner):

| # | What | Why |
|---|---|---|
| TH1 | A system action for listing an account's calendars, authorized when the account is connected | PD29: `System_refresh` needs a selected calendar, and at connect none is selected. Open: OQ9 |
| TH2 | Confirm in the theory's text what `ev_invitation` and `ev_attendees` mean on a calendar that is not the account's own (6.1, 6.5): no invitation there, and the calendar's owner counts among the attendees | PD31 |

---------------------------------------------------------------------------------------------------------------------

## 14. The fake Google server (FAKE)

### 14.1 What it is

`FakeGoogle::start() -> FakeGoogle` binds `127.0.0.1:0` and serves, on one port: the authorization endpoint, the
token and revocation endpoints, the key set, and the Calendar paths of section 6. It is a test double, written
from G-DOC. It is never evidence that Google behaves so (14.3).

- State: accounts (subject, address, verified or not), clients, per account calendars with roles, zone and the
  primary mark, events as JSON with a revision counter, issued codes (single use, bound to challenge, redirect,
  scopes, nonce), refresh and access tokens with end times on a clock the test moves, two signing keys.
- `fake.browser().visit(url, decision)` plays the person: it requests the authorization address, follows the
  redirect to the adapter's listener with a plain HTTP request, and returns the page the listener answered. The
  redirect carries `state`, `code` or `error`, and also `iss`, `scope` and `authuser`, as Google's does. Decisions:
  allow everything, deny, untick given scopes, sign in as another account, close the window.
- `fake.log()` is every request it received: method, path, query, whether `Authorization` was present and for
  which account, `If-Match`, the body, and which connection it came on. Tests assert on it ("no request was made",
  "one changing request", "never `primary`").

### 14.2 Faults, each set for the next matching request

Answer with a given status and Google's error shape (`reason`), with or without `Retry-After`; apply the change
and then close the connection without an answer, or answer 503; close without applying; delay past the deadline;
finish a change only after a delay (a late PATCH); a body that is not JSON, too long, compressed, or of another
content type; a redirect; an expired access token (401); a refresh that answers `invalid_grant`,
`admin_policy_enforced` or `invalid_client`, a new refresh token, or other scopes; an identity token with a wrong
issuer, audience, `azp`, nonce, no nonce, expiry, signature, algorithm, key id, no key id, a `crit` or `jwk`
header, a doubled member, another exponent; an unverified address; fewer or more scopes than asked; a lower role
or a vanished calendar; an event changed by someone else; an insert that becomes visible only after a number of
reads; an attendee list that Google cut (`attendeesOmitted`).

### 14.3 What FAKE assumes about Google

Each row is a place where FAKE was written from the documentation or from a guess, and the adapter depends on it.
Each has a live check in 15.3.

| # | Assumption | Basis |
|---|---|---|
| FA1 | PATCH and DELETE honour `If-Match` with one revision and answer 412 on another | G-DOC: "you must provide the current ETag value in the If-Match HTTP header" for a PATCH; 412 is stated for update and delete only |
| FA2 | An insert with an id that exists answers 409 | G-DOC (errors guide) |
| FA3 | GET of a deleted single event answers 200 with `status: cancelled`, or 404, or 410 | Not documented; the adapter treats all three as gone |
| FA4 | The token endpoint accepts a Desktop app client with its secret and PKCE | G-DOC marks the secret optional |
| FA5 | The consent screen lets a person untick single scopes, and the token then lists fewer | G-DOC |
| FA6 | An attendee may PATCH the whole attendee list with only the own answer changed | Not documented; G-DOC describes the shorter body |
| FA7 | The event Google returns after insert or PATCH holds `summary`, `start`, `end`, `description`, `location`, `attendees` as written | G-DOC |
| FA8 | `events.list` with `singleEvents=true` returns occurrences with `recurringEventId` and `originalStartTime`, and writes `dateTime` in the zone the request names | G-DOC |
| FA9 | A refresh token that ended answers `invalid_grant` | G-DOC names the causes; the answer is from community knowledge |
| FA10 | Rate limits arrive as 403 or 429 with the reasons of 6.10 | G-DOC |
| FA11 | The etag of an event is a quoted string | Not documented |
| FA12 | `freeBusy` reports a calendar it cannot read under `errors`, not as an empty list | G-DOC shows the member |
| FA13 | The identity token of a code exchange carries the `nonce` of the authorization request | G-DOC describes the claim; not stated for this flow |
| FA14 | A revocation ends the whole grant of that Google account to the client, not one token | Not documented |
| FA15 | On the account's primary calendar the entry marked `self` has the calendar's id as its address | G-DOC: `self` marks "the calendar on which this copy of the event appears" |
| FA16 | A change that Google applied always changes the event's etag | G-DOC: it "changes every time the resource changes"; unknown for a PATCH that changes nothing |
| FA17 | Google's own event ids match `^[A-Za-z0-9_]+$` | Not documented beyond the ids a client may choose |

---------------------------------------------------------------------------------------------------------------------

## 15. Tests

No test touches the Keychain, the network beyond `127.0.0.1`, or a real Google host. Every test that makes a
request makes it to FAKE. Each test name states what it proves; the ids are for citation. Every test that used
FAKE ends by checking its whole log: no path holds `primary` (PD22), and no request went out without the token
of its own account.

### 15.1 The list (GC `tests/` unless marked unit)

| Id | Proves |
|---|---|
| g01 | A full sign-in against FAKE ends `Connected`, with the subject, the verified address, the five scopes, and the refresh token in the token store |
| g02 | The authorization address has exactly the parameters of 3.2.1; the challenge is the SHA-256 of the verifier; verifier, state and nonce differ per flow and are 43 characters |
| g03 | Denied consent is `ConsentDenied`; nothing is stored; no token request is made |
| g04 | A missing calendar scope, and a missing address scope, are `PartialGrant`; nothing is stored; no revocation is asked |
| g05 | A scope beyond the five is `UnexpectedScope`; nothing stored; no revocation is asked |
| g06 | A reconnect as another subject, and a new connect as a subject some account record has, are `AccountMismatch`; the token store is never written, and the existing account's token still refreshes at FAKE |
| g07 | The listener: a wrong state, a missing state, a doubled parameter, another path, another method, another `Host` each get an error page; sixty of them do not end the flow; the right state ends it once; a second callback finds no listener; a wrong `iss` is `GoogleError`; unknown parameters are ignored |
| g08 | The listener ends after 300 s (test clock) with `TimedOut`, and at once when superseded |
| g09 | The page the listener answers is the fixed text and echoes no parameter; seven idle connections do not delay the callback |
| g10 | Each identity-token fault of 14.2 is `IdentityCheckFailed`, and nothing is stored |
| g11 | An unknown key id fetches the key set once more and then verifies; the key set is cached by its `max-age` within the bounds |
| g12 | An unverified address, and one the contract cannot hold, do not connect |
| g13 | The token answer: no refresh token, another token type, not JSON, each end `Cancelled`; `invalid_client` is `ClientRejected`; `refresh_token_expires_in` is passed on; `expires_in` is held to its bounds |
| g14 | Refresh: an access token near its end is refreshed before the call; a 401 drops it and is Transient; `invalid_grant` and `admin_policy_enforced` are `AuthLost(GrantEnded)`, `invalid_client` is `AuthLost(ClientRejected)`, each deletes the stored token and the cached one; a new refresh token is stored; other scopes are `AuthLost` |
| g15 | Revocation: 200 and `invalid_token` are `Revoked`; anything else is `NotConfirmed` |
| g16 | No secret in any `Debug`, `Display` or error text: the verifier, state, code, tokens, client secret, a calendar id and an event title of a full run with injected faults are searched for in every formatted value the tests can reach |
| g17 | Token-store failures (a scripted store): `Locked` is Transient, `Refused` and `Failed` are Rejected, and in each case FAKE saw no request |
| g18 | The calendar list: pages, the roles of 6.2, names, hidden entries listed, an unknown role and an unusable id left out, more than 250 calendars fails |
| g19 | Two accounts: each call carries its own account's token; account A's token on a calendar only B has is Rejected by FAKE as Google would. (That the broker never resolves B's calendar under A is the core's, section 8) |
| g20 | Free/busy: periods per calendar under the broker's ids, in UTC, cut to the window; an error for one calendar fails the read |
| g21 | Reading events: the window bounds, occurrences of a recurring event with their identity, an all-day event as dates, an occurrence of an all-day series without `occurrence`, a cancelled item skipped, several pages and calendars in one order |
| g22 | `still_allowed` turned false between two pages: `Stopped`, nothing returned, no further request (PLAN 5 R08, the adapter's half) |
| g23 | Fields: for each granted field, `fields=` asks only its members, and a view shows a field only when it is granted even if FAKE sends everything |
| g24 | Bounds: a read past 2 500 events or 768 KiB, with a cut attendee list, or with more than 2 000 attendees fails with the fixed sentence; nothing partial is returned |
| g25 | Times: `Z`, an offset, a fraction of a second; every events read names the calendar's zone and the view pairs Google's offset with it; a day-light-saving gap and fold as Google writes them; an all-day date is never an instant |
| g26 | The own entry: on the primary calendar `own_response` and `invitation` follow the entry marked `self`; on a shared calendar `own_response` is `null`, `invitation` is false and the marked entry is among the attendees |
| g27 | Observation: each member of 6.5 from fixtures: a plain event, an invitation, a recurring master, an occurrence, an all-day event, a focus-time event; `Absent` for 404, 410 and cancelled; a cut attendee list and an unusable attendee address fail it |
| g28 | Create: the request is exactly 6.6; Applied carries id, revision and `confirmed`; an order without a valid reserved id makes no request; 409 is Rejected; a 200 with another id is NoAnswer |
| g29 | Create, answer lost after FAKE applied it: NoAnswer; no second POST; reconciliation finds the event and says Applied; FAKE holds one event |
| g30 | Create, 503: NoAnswer, not Transient; with FAKE's late-visible insert the first reconcile read says read again and the second finds it |
| g31 | Update: PATCH with `If-Match`, only the set members, `sendUpdates` as approved; untouched members of the event are unchanged at FAKE |
| g32 | Update against a changed event: 412 is `revision_conflict` and FAKE's event is unchanged (PLAN 14, changed after preview); 404 and 410 are `revision_conflict`; 409 is Transient |
| g33 | Update, 503 after FAKE applied it: NoAnswer; reconciliation says Applied with `notification: Unknown`; one change at FAKE |
| g34 | Invitation: the attendee list after the change is exactly the approved one; kept attendees keep their answers; the own entry stays; a changed revision or a cut list at the read makes no PATCH |
| g35 | Response: the whole list is sent and only the own entry differs; an occurrence is addressed by instance id and its original start is checked; without an own entry, and on a shared calendar, Rejected and no PATCH |
| g36 | Delete: `If-Match` and `sendUpdates` as approved; 412, 404 and 410 are `revision_conflict` |
| g37 | Classification, one case per row of 6.10, for a change and for a read; `Retry-After` is passed on and capped |
| g38 | Nothing sent (FAKE closed, deadline already past) is Transient; sent and unanswered is NoAnswer for a change and Transient for a read; every request came on its own connection |
| g39 | A redirect is not followed; a body over its bound, a compressed body and a wrong content type are no usable answer |
| g40 | One changing request per order: for every tool and every fault, FAKE's log holds at most one POST, PATCH or DELETE per order |
| g41 | Every PATCH and DELETE carries `If-Match` with one strong tag, and every changing request an explicit `sendUpdates` |
| g42 | Ids: an event id of `.`, `..`, `%2e%2e`, or with `/`, `?`, `#` or a space makes no request; a calendar id with `#` and `@` reaches FAKE as one path part; a revision of `*`, `W/"x"` or a list makes no request |
| g43 | Reconciliation: every row of section 7's table through `decide`, with FAKE as the source of the read-back; the two reads for a create with a late-visible insert and for an update with a late PATCH |
| g44 | The retry rule: waits of 2, 4 and 8 s, a hint longer than the wait, the fourth failure, the 120 s budget; the numbers are W04 D16's (unit) |
| g45 | `matches`: each tool, each member, absent against empty, instants written with different offsets, the own address in an approved list (unit) |
| g46 | Forbidden dependencies and FAKE only as a dev-dependency (the scan of 2.1) |
| g47 | `Endpoints::google()` is the five constants; the loopback set accepts only `127.0.0.1` (unit) |
| g48 | The client file: the `installed` shape is read, other members ignored; a missing secret and another shape are errors that name the path and no content (unit, with a made-up file) |
| f01 | FAKE's own rules: a code is single use and bound to its challenge and redirect; `If-Match`; a duplicate id; revisions rise; the log is complete |

### 15.2 The rows of gate `CD-G2b` and what stands in for each

`CD-G2b` is "a real test-account Google spike" (PLAN 13). No test of 15.1 closes it or any part of it. The table
says which tests show the adapter's side against FAKE, what the core must add, and what only the live run shows.

| # | Row (PLAN 5, 10, 14) | Stand-in here | Still needed |
|---|---|---|---|
| 1 | Missing scope or denied consent: no false connected state (PLAN 14; R03) | g03, g04, g05, g12, g13 | core: the account states (CO1); live: LV1, LV5 |
| 2 | Account A against account B; a shared calendar's role lowered (PLAN 14; R03a) | g18, g19, g37 (403), the adapter's half only | core: resolving a calendar under its account (section 8), CO7; live: LV6 |
| 3 | Insert accepted, answer lost, broker killed: one event (PLAN 14; R12) | g29, g30, g43 | core: the kill and CO8; live: LV4 |
| 4 | Event changed after the preview: conflict, new approval (PLAN 14) | g32, g34 | core: CO6, CO12; live: LV3 |
| 5 | Invitation: only the approved recipients, notified as approved (PLAN 14; R05) | g34, g41 | live: LV7 (a real message arrives) |
| 6 | DST gap and fold, an all-day occurrence, a recurring master and exception; an unsupported change refused (PLAN 14) | g21, g25, g27 | theory and core refuse the change; live: LV8 |
| 7 | Poll a completed read after revocation: status only (PLAN 14; W04 Q10) | none in phase 1 | core: ST4, CO5 and its test (section 9) |
| 8 | Refresh, its failure, reconnect; seven days in testing mode (R03; PLAN 10.1) | g14, g06 | core: CO11; live: LV5 |
| 9 | Disconnect and revocation at Google (R03; PLAN 9.1) | g15 | core: CO4; live: LV5 |
| 10 | Read bounded events and availability from selected calendars (R04) | g20 to g24 | core: CO5; live: LV8 |
| 11 | Create and edit in an existing calendar (R05; C07, C13) | g28, g31 | live: LV3, LV6 |
| 12 | Delete a Fabric-created event with `If-Match`, confirmed by reading back (CD-Q05a) | g36, g43 | live: LV3 |
| 13 | Answer one occurrence of a recurring invitation (CD-Q05b) | g35 | live: LV7, LV13 |
| 14 | Tokens in the Keychain, never elsewhere (R13) | g16 | phase 2: `KeychainTokenStore`; live: LV9 |
| 15 | Revoke or pause during a paged read (R08) | g22 | core: the flag of PO3 and its test |
| 16 | A failed audit write: no call and no release (PLAN 14) | none | core: W04's c12 to c14 extended to read orders and releases |
| 17 | Upgrade of an encrypted fixture store (PLAN 14) | none | lead: ST11 |

### 15.3 What only a live run can show

Each is run by the owner at the Mac with the accounts of RUNBOOK, in phase 2.

| # | Check |
|---|---|
| LV1 | The real consent screen for a Desktop app client: the unverified-app warning for test users, unticking one scope, what the token then lists, and how `email` appears in it (FA5) |
| LV2 | Whether the token endpoint needs the client secret for this client type (FA4); whether the identity token carries the nonce (FA13) |
| LV3 | PATCH and DELETE with `If-Match`: applied on the right revision, 412 on another (FA1); what a GET of a deleted event answers (FA3); the form of an etag (FA11); whether a PATCH that changes nothing changes the etag (FA16) |
| LV4 | A reused event id is refused (FA2); how soon after an insert a GET sees it (PD20, RK3); whether Google returns a created event's members exactly as written (FA7) |
| LV5 | Refresh; `invalid_grant` after access is removed in the Google account and after seven days (FA9); revocation, and whether revoking one refresh token ends another of the same account and client (FA14) |
| LV6 | A calendar shared with "make changes": create and edit there; lower the role and see the write refused; each of the five roles as the list reports it (PD26) |
| LV7 | A real invitation reaches the owner's mailbox for `all`, and what `externalOnly` and `none` do; an answer with the whole attendee list is accepted from an attendee, changes only the own entry and tells the organizer (FA6) |
| LV8 | Real recurring events as occurrences, with the offset of the zone asked for; a real all-day event; free/busy over several calendars and how long a window may be (FA8, FA12) |
| LV9 | The Keychain item: created, read after a restart, deleted at disconnect; not synchronized; the prompt after a rebuild |
| LV10 | `/usr/bin/open` from the broker as the host starts it; the loopback redirect in Safari and in Chrome; what another user's `ps` shows of the address |
| LV11 | Real rate-limit answers, if they can be provoked safely (FA10) |
| LV12 | The five scopes are enough for every call of section 6 and none is unused: the calendar list with `calendar.calendarlist.readonly`, free/busy with `calendar.freebusy` (it does not accept `calendar.events`), every events call with `calendar.events`. This is the scope freeze of PLAN 10.2 |
| LV13 | The entry marked `self`: on the primary calendar it is the account (FA15); on a calendar another account shared it is that other account, and Fabric refuses to answer that person's invitation |
| LV14 | The event ids Google itself gives, for ordinary, imported and recurring events, against 5.3 (FA17) |

---------------------------------------------------------------------------------------------------------------------

## 16. Integration (phase 2), in order

1. The contract session drafts schema version 3 with V3-1 to V3-10 and the theory with TH1 and TH2; the owner's
   approval is the merge.
2. The lead writes one design for phase 2 of CD-W05 and CD-W06 (the store, the core, the port, held results, the
   order of pull requests), with an answer for every id of sections 10 to 12, as a pull request the owner merges;
   then builds the store part.
3. PORTS gains PO1 to PO8. GC gains the dependency on the core, the mapping of its outcomes, `KeychainTokenStore`
   with its ignored test, and `SystemBrowser`.
4. The core gains CO1 to CO14, each with acceptance tests that use FAKE as the provider, in process.
5. `apps/fabric-broker` gains the two workers (sign-in, provider), the configuration of CO10, and process tests
   against FAKE with the test key and the in-memory token store.
6. The practice calendar's observation hook goes (PW 4.4). If the practice calendar stays as the debug-only
   provider of the synthetic account (W04 D13; the lead's and the owner's call), it must answer the observe and
   reconcile orders too.
7. The live run of 15.3 with the owner, recorded as the evidence of `CD-G2b`, then the independent review.

Nothing of phase 2 is in this pull request.

---------------------------------------------------------------------------------------------------------------------

## 17. Decision records

ADR-111 (calendar authority and time model) and ADR-113 (projection and commit) are written as Proposed with this
design, as CD-W04 wrote ADR-115. Each covers only what the broker settles; its Rendezvous part stays parked
(`docs/architecture.md` 19). Each names where the older specifications say otherwise and keeps the plan's
decision. Merging the pull request that carries them is the owner's acceptance.

---------------------------------------------------------------------------------------------------------------------

## 18. Risks, questions, not checked

### 18.1 Risks

- RK1 Everything in phase 1 is tested against a double written from documentation. 14.3 lists what it assumes; a
  wrong assumption shows only in the live run.
- RK2 If Google does not honour `If-Match` on PATCH as documented (FA1), PD14 and PD15 fall together and the
  fallback is a read followed by a full update with `If-Match`, with attachments and conference data kept by
  their version parameters.
- RK3 A read right after an unanswered change may not see what exists a moment later, and Google may finish a
  request after the broker read. PD20 waits and reads twice; Google gives no bound, so a change made twice after
  `absence_confirmed` cannot be ruled out, only made unlikely. The owner-resolution path covers what is left.
- RK4 `sendUpdates=none` "can have significant adverse effects" and "some emails might still be sent" (G-DOC). The
  broker promises the owner which value it sends, not what Google then does (PLAN 10.3).
- RK5 A Calendar scope is coarser than any Fabric rule: with `calendar.events` the broker can change every event
  the account can. The limit on what happens is the broker's own (PLAN 10.2). A stolen refresh token has the full
  scope.
- RK6 In testing mode a connection ends after seven days. Development and the gate run must expect
  `reauthentication_required` weekly.
- RK7 Google deletes a client unused for six months, and a refresh token unused for six months.
- RK8 PD25 cuts long texts and leaves out attendee addresses the contract cannot hold; an agent may act on a
  shortened description, and the owner's before-image is cut the same way (OQ15).
- RK9 A person can change an event between the observation and the dispatch. `If-Match` catches it at Google; the
  approval then shows as a conflict, never as an overwrite.
- RK10 The `rsa` crate has an open advisory about timing in private-key operations. The adapter only verifies with
  public keys; FAKE signs with a test key that protects nothing. The crate brings about fourteen new crates and
  second versions of eleven that are locked already (2.1; OQ6).
- RK11 `reqwest` brings an async runtime thread into the broker process for the blocking client. It is confined to
  the worker; the core stays free of it (W04 section 2).
- RK12 The same-user threat of PLAN 4.3 holds here too. A program running as the user can read the Keychain item
  after the user allowed it once for a build, and can see the authorization address while `/usr/bin/open` runs.
  With the state, the nonce and the challenge from that address it could finish a sign-in of its own Google
  account and hand that code to the listener first; the broker would then be connected to the wrong account. The
  owner sees the verified address of what connected (3.5 item 10) and must choose calendars before anything works.
  Opening the address without a child process needs code outside the workspace's no-`unsafe` rule and is not done.
- RK13 Two proofs and two reads for every change of an existing event (observe, then act) add about two seconds to
  each classification and each fresh permit. W04 R8 applies. A PATCH costs three of Google's quota units; the
  limit is 600 requests a minute per person.
- RK14 A secret is wiped from the adapter's own memory, not from the buffers of the HTTP and JSON libraries.
- RK15 A 5xx from Google ends a change in reconciliation, not in a retry (PD17). After a plain failure at Google
  the request ends `absence_confirmed` some seventy seconds later, and under a row that asks the owner approves
  again. That is the price of never reporting a made change as a conflict.
- RK16 A guest of an event Fabric made can invite others (Google's default), outside every Fabric rule (OQ13).
- RK17 An event whose id is not of the form of 5.3, or whose etag is not a quoted string, cannot be changed
  through Fabric (FA11, FA17).

### 18.2 Questions only the owner can answer

The code and this design run on the default after each.

- OQ1 The scope list of RUNBOOK section 5 is the candidate. It is frozen after the live run (PLAN 10.2). Default:
  the five. Narrower ones exist and do not fit: `calendar.events.owned` cannot write to a shared calendar (C13),
  and `calendar.events.freebusy` is an alternative to `calendar.freebusy`, not to `calendar.events`.
- OQ2 How long may a sign-in wait for the browser? Default: 300 s.
- OQ3 A partial grant keeps no token and asks to connect again. Or run with what was granted? Default: no token.
- OQ4 Keychain names: service `ai.paracels.fabric.calendar.google`, account `refresh-token-v1.<account id>`.
  Default: these.
- OQ5 How long is a read result held when nobody fetches it? Default: 24 h, and never past its one release or its
  account's disconnect.
- OQ6 Checking the identity token's signature adds the `rsa` crate and what it needs (RK10). Google would allow
  leaving the check out; the plan asks for it. Default: check.
- OQ7 The HTTP client: `reqwest`, already in the lock file, or a smaller blocking client that adds crates. Default:
  `reqwest`.
- OQ9 Is the calendar list at connect a broker action that needs a proof (TH1), or part of the owner's own command?
  Default: it needs a proof, so that no Calendar call exists without one. Then an account cannot be set up while
  the rule prover is not running.
- OQ10 The broker opens the system browser itself. Default: yes.
- OQ11 PATCH with `If-Match`, pending LV3. Default: PATCH.
- OQ12 A reconnect that does not succeed, or that another `connect_google` supersedes, leaves an account that was
  working not connected, with its token deleted. Or leave it as it was? Default: not connected (fail closed).
- OQ13 Should an event Fabric creates forbid its guests to invite others (`guestsCanInviteOthers: false`)? It is a
  member the agent did not ask for and the owner did not see. Default: Google's default, unchanged.
- OQ14 How many accounts and calendars? The snapshot must fit one frame (W04 D38, R24). Default: 250 calendars per
  account. With no cap a sixth account of that size would pass the frame limit, so the lead sets a budget past
  which `connect_google` is refused, and puts it to you (18.5).
- OQ15 A before-image longer than the contract's bounds is shown cut (RK8). Or refuse to change such an event?
  Default: cut.
- OQ16 A disconnect while a change of that account is still unconfirmed ends it as "needs your look". Or warn
  before disconnecting? Default: no warning.

Answered: OQ8, the client switch "Use this client for an AI-powered agent" (RUNBOOK 6.3). The owner decided on
2026-11-17, for the development client: "Yes, leave it off". A release client is not decided.

### 18.3 Not checked

- No behaviour of the real Google: every row of 14.3 and 15.3.
- The Keychain, in any form. `/usr/bin/open`. A real browser.
- TLS to Google: the client is built with it on macOS and Windows and no test makes a TLS call.
- Linux and Windows builds, until CI runs them.
- Sections 9 to 13 are proposals. Their owners have not answered them.

### 18.4 The critique

Three independent readers went through the first draft on 2026-11-17 before anything was built: one for security,
one against the plan, the contract, the theory and the code as built, one against Google's documentation. What
changed because of them: ids are checked before they reach a URL or a header (5.3); a doubtful answer is never
retried (PD17), which removed every "read before judging" rule; the own entry exists only on the primary calendar
(PD31); a failed sign-in revokes nothing (PD34); a cut attendee list fails a read (PD23); two reads before any
change counts as absent (PD20); read times are in the calendar's zone (PD33); one order reads every page (PO3);
and sections 9 to 13 were rewritten against how the core and the contract really work.

### 18.5 The lead's first answer to sections 9 to 12 (2026-11-17)

Every item of PO, ST and CO was accepted or accepted with a change; none was refused. The answer per id comes as
the lead's own design document for phase 2 (section 16 step 2); nothing is built in the lead's areas from this
document. What it changes here:

- PD17 is accepted. Google is not attached to the core before reconciliation exists there (CO8, PO4).
- PO4: the core owns the table of section 7; the adapter reports what Google holds (done in section 7).
- PO3: the `still_allowed` flag is only an early stop. The core checks authority again in the commit that stores a
  read result, and treats a stopped read as a retry, not a failure. One order reading every page is accepted.
- PO2 and CO6: the observation becomes a broker record first, and the proof reads it from the store.
- Broker calendar ids are made by the core, never while a mutation is applied.
- How a read's result reaches the agent is open and the lead's to settle together with CD-W06, which asked for the
  same: section 9 makes every release a second call, PLAN 9.2 says an allowed read returns its result. The four
  amendments of section 9 are accepted in substance.
- ST3 and ST4 keep content outside the ledger, which touches ADR-115 D1 and D7, and ST8 adds operation types,
  which touches D2. Both need the owner. ST7 changes what the theory calls a pending effect; that goes to the
  contract session.
- OQ14: see there.

---------------------------------------------------------------------------------------------------------------------

## Appendix A. What Google's documentation states (read 2026-11-17)

| Page (`developers.google.com/` unless marked) | Relied on for |
|---|---|
| `identity/protocols/oauth2/native-app` | Loopback redirect `http://127.0.0.1:port` for desktop apps; custom schemes and copy-paste no longer supported; "Always use PKCE"; verifier 43 to 128 characters; `S256`; the authorization and token endpoints and their parameters; `client_secret` optional; refresh tokens "always returned for installed applications"; `refresh_token_expires_in`; revocation endpoint, 200 and 400; "Incremental authorization is not supported for installed apps"; a person may grant fewer scopes, check `scope`; `error=access_denied`; desktop apps "cannot keep secrets" |
| `identity/protocols/oauth2/resources/loopback-migration` | The loopback redirect "will continue to be supported on desktop apps" |
| `identity/protocols/oauth2` | Why a refresh token stops working; `admin_policy_enforced`; seven days in testing mode except for name, address and profile scopes; 100 refresh tokens per account and client |
| `identity/openid-connect/openid-connect`, `identity/openid-connect/reference` | The identity token's checks; `sub` "never reused", at most 255 ASCII characters; `email` may change; `nonce`; `prompt`; `login_hint` as an address or a `sub`; `iss` in the authorization response, "always returned" (RFC 9207); the discovery document's endpoints; `RS256`; keys change rarely and may be cached by the HTTP cache headers |
| `support.google.com/cloud/answer/15549945` | Testing mode: 100 test users, authorizations end after seven days; the user cap of an unverified app |
| `support.google.com/cloud/answer/15549257` | A desktop client needs no further information; the secret is shown once; a client unused for six months is deleted |
| `workspace/calendar/api/auth` | The scope names and what each allows |
| `workspace/calendar/api/v3/reference/calendarList/list`, `.../calendarList` | Parameters, `showHidden`, the scopes accepted, the five access roles and what each sees of private events |
| `workspace/calendar/api/v3/reference/freebusy/query` | Body, at most 50 calendars, the scopes accepted (`calendar.freebusy`, not `calendar.events`), `busy` and `errors` |
| `workspace/calendar/api/v3/reference/events/list`, `get` | `singleEvents`, `orderBy`, `timeMin` (end) and `timeMax` (start), `timeZone` of the answer, `maxAttendees`, page size 250 by default and at most 2500, the scopes accepted |
| `workspace/calendar/api/v3/reference/events/insert` | The `id` rules (lower-case `a-v`, `0-9`, 5 to 1024); `sendUpdates` values; "some emails might still be sent"; collisions may go undetected at creation |
| `workspace/calendar/api/v3/reference/events/patch`, `update`, `delete` | PATCH semantics, arrays are replaced, three quota units; update replaces the whole event; delete answers an empty body; `sendUpdates` |
| `workspace/calendar/api/v3/reference/events` | `status` `cancelled`; `eventType` values; `recurringEventId`, `originalStartTime`; `start` and `end`, the exclusive end date; attendee members; `self` marks "the calendar on which this copy of the event appears"; `attendeesOmitted`; `organizer.self`; `guestsCanInviteOthers` defaults to true |
| `workspace/calendar/api/guides/version-resources` | `etag`; `If-Match` and 412 for update and delete; no conditional insert |
| `workspace/calendar/api/guides/performance` | `fields=` syntax; for a PATCH "you must provide the current ETag value in the If-Match HTTP header"; `If-Match: *` forces it; `null` deletes a member |
| `workspace/calendar/api/guides/errors` | The status codes and reasons of 6.10 and what Google advises for each |
| `workspace/calendar/api/guides/quota` | 600 requests a minute per person, 10 000 per project, 1 000 000 a day; 403 or 429 when exceeded; standard use "at no additional cost" |
