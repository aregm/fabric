# CD-W04 design: encrypted store, authority core, owner channel

**What this is.** The design of work package `CD-W04` of the Calendar Desktop plan, as built: the encrypted
store (`crates/fabric-storage`), the authority core (`crates/fabric-broker-core`), the broker executable
(`apps/fabric-broker`) and the shell changes that run them. It describes the code as it is, not a plan for it.

**How code comments cite it.** A number in a code comment of those three crates, such as `(5.7)`, `section 3.4`
or `(D17)`, names a section or a decision of this document. Test ids such as `c18`, `s08` or `p04` name the tests
of section 9. `L1` to `L7` are the package decisions of section 0.2, `CCR-n` the contract change requests of
section 6.2, `Rn` the risks of section 11.1 (two-digit ids such as `R08` are the plan's requirements, see 7.3),
`Qn` the owner's questions of section 11.2, and `W1`, `W2` the readings of section 11.3.

**Who decides.** Every decision marked open is the owner's: section 11.2 lists them with the default the code
implements today. The Calendar Desktop plan and the contract (`contracts/desktop/v1`) win wherever they differ
from this document. Section 4 is proposed to the owner as ADR-115 (Proposed).

Date 2026-11-16. Status: as built, describing the worktree of `CD-W04` before merge.

Abbreviations (paths relative to the repository root):

| Short | Means |
|---|---|
| PLAN | `docs/plans/calendar-desktop-plan.md` |
| RM | `docs/roadmap.md` |
| ADR107, ADR108 | `docs/adr/ADR-107-operation-log-and-sync-evolution.md`, `docs/adr/ADR-108-agent-and-mcp-permission-model.md` |
| ADR-115 | `docs/adr/ADR-115-broker-ledger-envelope.md` (Proposed; section 4 of this document) |
| CR | `contracts/desktop/v1/README.md` (the contract; "README section n" in code comments) |
| CC | `crates/fabric-desktop-contracts/src/` |
| BR | `apps/fabric-broker/` |
| HOST, RELAY | `apps/fabric-desktop/src-tauri/src/`, `apps/fabric-desktop/relay/src/` |
| W03 | work package CD-W03 (the broker's rule theory and `BrokerGate`, pull request 22, not merged) |
| SPIKE | the SQLCipher storage spike run before this package; not in the repository, summarized in appendix A.1 |
| THEORY | a reading of W03's theory interface made for this package; not in the repository, summarized in appendix A.2 |
| DEF-n | the demo's recorded defects (RM, `RD-11`); DEF-1, DEF-2, DEF-4 and DEF-7 are broker acceptance rows of 7.3 |

---------------------------------------------------------------------------------------------------------------------

## 0. Decisions

### 0.1 Design decisions

| # | Topic | Chosen | Why |
|---|---|---|---|
| D1 | Crate layout | Directories and packages `crates/fabric-storage`, `crates/fabric-broker-core`; `apps/fabric-broker` composes. This deviates from PLAN 13's proposed `crates/fabric-broker` (L2) | PLAN 13 puts the Google adapter in its own crate; ports inside the executable's lib would force a dependency cycle. The package name `fabric-broker` is taken by the executable. |
| D2 | Schema shape | Relational STRICT tables; contract objects as canonical JSON columns beside key columns | Database constraints carry authority invariants (one live session, one live authorization per request, `expires = issued + 60000`, attempt and authorization unique). |
| D3 | Write path | Typed `Mutation` applied only through `WriteTx::apply`; one ledger row per transaction that applied a mutation; `verify_replay` rebuilds the projection. The one other writer of projection rows is a migration's SQL (3.6) | ADR107 D1 "no other write path" becomes structural; replay is checkable (PLAN 11 para 8). |
| D4 | Tables for CD-W05/W06 data | None now; they come by their migrations | No speculative schema; W04 owns and rehearses the migration runner. |
| D5 | Notice durability | Sequence counter durable in `meta`; bodies in a 256-entry in-memory ring whose base is `meta.notice_sequence` at run start; no `notices` table | Gap-free durable sequences; `snapshot_required` for history not kept; a cursor equal to the run's base is accepted. |
| D6 | Subscriber bound | Synchronous stdout writes; one step producing more than 256 notices sends one `snapshot_required` change and ends the subscription | No memory growth (PLAN 9.1), no writer thread. |
| D7 | Write failure mid-run | Answer the frame in hand (`store_unavailable` owner, `internal` agent), one stderr line, exit 75. The jobs of the halted step are dropped, also those an earlier commit of the same step queued; a committed `dispatching` becomes `outcome_unknown` at the next start. After a halt every call answers the halt again | No in-memory state the store lacks; deterministic. |
| D8 | Store unavailable at start | Stay up; `hello` answers; every other owner method `store_unavailable` with a message naming cause and recovery; an agent connect is closed and an agent call answered `internal` | The page ignores `health` and shows error text (`model.ts` 323-341). |
| D9 | Retrying the open | Only `InUse`, `KeychainLocked` (which includes `KeyError::Failed`), `KeyGeneration` (the system random source failed, `KeyError::Random`) and `Io` (a passing I/O condition: SQLite `CANTOPEN`, `IOERR`, `FULL` or `NOMEM`, or an unreadable store file), at most once per 5 s, when an owner frame needing the store arrives. A successful reopen goes through `Core::open`, the only constructor, which runs the 5.8 recovery and the first pump before the broker leaves the unavailable mode | A retry after `KeychainRefused` re-prompts. A reopen without recovery would let a dead run's authority live on. A passing I/O failure is not an integrity failure, whose recovery is to move the store aside. |
| D10 | Key sources | `FABRIC_BROKER_KEY_SOURCE` required: `keychain` (macOS) or `test` (debug builds only; public constant key; SQLCipher on macOS, plaintext elsewhere). No key file. `key_kind` in `meta`; a mismatch refuses | Explicit (DEF-2), no silent plaintext (PLAN 11), a test-key store never becomes a Keychain store. |
| D11 | Developer seams | `cfg(debug_assertions)`, no Cargo features: key source `test`, the synthetic agent, `Broker::seed`, `FABRIC_DESKTOP_DATA_DIR` | `--all-features` CI needs nothing; release builds drop them; `verify-desktop.sh` compiles the release broker and runs its configuration tests in release. |
| D12 | Fault injection | No product hooks: SQL triggers and a held `BEGIN IMMEDIATE` from a second test connection; a crash child that re-executes its test binary; in-process drop of `Broker` for "killed after commit" | Nothing test-only ships. |
| D13 | Prover kind | Fixed at start from `Attachments.prover.kind`, never taken from a result. Only an `Isabelle` permit can lead to an authorization. No prover or provider double in product code; tests read `Output.jobs` and feed `Input::ProofDone` / `Input::Attempt`. Amendment, proposed with the pull request of the prover wiring and the owner's to accept: one debug-only exception, the practice calendar (`apps/fabric-broker/src/practice.rs`), an in-memory provider on the synthetic account, compiled only in debug builds and attached only with the synthetic agent (so the test key) and a configured Isabelle; it carries out only dispatch orders the core makes after a consumed authorization resting on an Isabelle permit, and never counts as evidence for a Google gate (`docs/plans/calendar-desktop-prover-wiring-design.md`, sections 4.1 and 9.3 O1) | A port result could claim any kind; a synthetic proof never authorizes (PLAN 1 C05, PLAN 14). |
| D14 | No prover attached | Request goes `undecided` with no proof report | Inventing a report would be a fake proof. |
| D15 | Fence | `{pause_epoch, revocation_epoch, event_revision}`; broad rule for raising `revocation_epoch` (5.6) | Coarse is fail-closed and cheap. |
| D16 | Re-proofs, watchdog, retries | `MAX_REPROOFS = 3` per purpose (classification, fresh permit), counted over the request's life and never reset by a retry; then `stale` (fresh permit) or `undecided` (classification). Watchdog 35 s. Retries 3, backoff 2/4/8 s or the provider's hint if longer, budget 120 s, decided at `dispatching` | PLAN 5 values; the contract has no `retry_wait -> terminal_failure` edge. |
| D17 | Permit that cannot authorize | `proving_fresh_permit -> undecided`, no authorization, when there is no provider, the prover is not Isabelle, the agent is paused, the tool changes no calendar, or `authorizing_grants` is empty (L7). A result is first checked for its binding to its job (D39); then a result whose `authorizing_grants` names an id outside the job's `facts.grants` is refused before the fence or the verdict is looked at, whatever its purpose and verdict: recorded, relied list unchanged, request `undecided` (5.7); it is never a store fault | Fail-closed; no authorization exists that nothing can use or that rests on no grant. A wrong adapter cannot halt the broker, which would recur after every restart. |
| D18 | One-request grant | Offered only to its origin request; ends `consumed` in the commit that makes the origin request terminal; stays live until then, also while `applied` or `outcome_unknown`; the desktop README says so (L2) | Retries of the same effect need it; a second request cannot reuse it. |
| D19 | Recipient-set edits | Narrowing applies at once; adding a member or domain, or a delete, ends live grants naming the set (`authority_reduced`) | PLAN 7: expansion needs owner acceptance at grant level. Open: Q6. |
| D20 | Profile changes | Activation ends no grant and stales no card; expansion baseline: previous version, Careful for a new profile. Every `save_profile` commit applies exactly one `ProfileStateSet` with revision + 1; `ProfileVersionAdded` never touches the revision | One rule for the profiles revision. Open: Q5, Q7. |
| D21 | Methods implemented | All except `connect_google` (`unknown_method`, CCR-1); `save_preferences`, `disconnect_google` store halves implemented | `connect_google` needs OAuth (CD-W05). |
| D22 | Edge answers | `selected` non-null `invalid_params` (acceptable only with CCR-7 and before CD-W08); `revoke_grant` on an ended grant `conflict`; `pause_agent all_agents` with no agents `not_found`; `session` duration without a live session `invalid_params` at `/params/duration`; `decide_request` revision mismatch `stale_request` | No false success; the page relies on `stale_request`. |
| D23 | Command dedup | Success row in the command's transaction; a refusal in its own transaction (`owner.refused`); `store_unavailable`/`internal` never stored; never pruned | Commit-before-answer and durable dedup. |
| D24 | Access audit | `audit` table: full-detail reads (`request_details_full`), `command_reused`, unknown agents (`unknown_agent`), `not_permitted`; `busy` refusals aggregated to one `agent_busy` row per agent per one-minute window with a count. Audit-only writes go through `Store::write_audit` and append no ledger row | ADR107 D1 keeps audit apart from the ledger; a flooding agent cannot turn the limit into a write amplifier (L2). |
| D25 | Agent entry | `AgentInput::{Connect, Call(AgentRequest), Disconnect}`; `UNIQUE(agent_id, call_id)`: a repeated call returns its request and writes nothing (another action under the same call id: `invalid_params` at `/params`). Only the transaction that inserts a request carries `agent_call` idempotency. `fabric_request_status` is implemented and writes nothing | PLAN 7 "a retry returns the existing lifecycle"; a repeat must not hit the ledger's UNIQUE key and halt. |
| D26 | Synthetic agent | `apps/fabric-broker/src/synthetic.rs`, debug builds, opt-in `FABRIC_BROKER_SYNTHETIC_AGENT=1`, refused unless `FABRIC_BROKER_KEY_SOURCE=test`; the host passes it only with the test key and an explicit `FABRIC_DESKTOP_DATA_DIR`. It starts after the run's first successful `subscribe`; its classifier answers only `need_approval` or `undecided`. Amendment, proposed with the pull request of the prover wiring and the owner's to accept: with Isabelle configured as well, the classifier is not attached; the agent runs a practice script against the practice calendar, the debug-only exception to D13 (`docs/plans/calendar-desktop-prover-wiring-design.md`, section 5) | No synthetic record in a product store; the packaged check sees its notices. Open: Q4. |
| D27 | Broker variables | The broker reads exactly three names and refuses any other `FABRIC_BROKER_*` name with exit 78. Test helpers and the host remove inherited `FABRIC_BROKER_*` names before setting their own | A misspelt name is an error, not a silent default (DEF-2; L2). |
| D28 | Locations and names | `<app_data_dir>/broker/broker.db`; Keychain service `ai.paracels.fabric.calendar.store`, account `database-key-v1`; `<data_dir>/prover` reserved for the prover's work directory (L7) | Keeps the host's `instance.lock` apart. Open: Q2. |
| D29 | `locking_mode = EXCLUSIVE` | Not used | Untested in SPIKE; blocks the second test connection. |
| D30 | Recovery | One transaction (`broker.started`) at every `Core::open`: at start and at every successful reopen | Atomic; a reopen is a restart. |
| D31 | Rate window | Counted from `requests.received_at_ms` | Durable, no extra state. |
| D32 | Envelope | The field set of section 4, proposed in ADR-115 | Keeps every ADR107 D2 field that means something on one device; adds the actor. |
| D33 | Read results and disclosure | Not in W04: no provider can produce a result. A fresh Permit for a tool that changes no calendar goes `undecided`; a `request_result` call ends `terminal_failure` (`invalid_arguments`, "no result is held"). The release of a change's outcome and the broker's own reads, which a later package may permit with no grant, are not dispatches and need no authorization record | CD-W05/W06 add the result table (L7(d)). |
| D34 | Clock | A per-step monotonic clock: `now = raw + offset`, and when that falls below the last `now` of the run the offset grows by the difference. `committed_at_ms` never decreases and `verify_chain` checks it; `Store::write` refuses a row dated before the last one. A `schema.migrated` row, written inside `Store::open` before the clock exists, takes `max(OpenOptions.now_ms, last committed_at_ms)` (3.6) | A backward wall-clock step neither halts the broker nor writes a decreasing ledger; lifetimes keep running at wall rate after the step. |
| D35 | Agent activity | Every agent call, a status poll included, is activity. Activity is kept in memory only: a session's `last_active` (set to the session's start at connect, so never null) and the agent's `last_tool_call_at` (null until its first call in the run); it raises no revision, writes no row and sends no notice. The durable event is the session ending | No fsync'd row per call; pause cannot be starved (L2). The two fields are live, outside the cursor (CCR-17, R23). |
| D36 | Relied grants | The store keeps each request's relied grant list: set in the commit that makes it `allowed` (the classification's `authorizing_grants`) or `approved` (the approval's grant), replaced by each accepted fresh proof's `authorizing_grants`. Every result not accepted, for any reason, leaves it unchanged (L7's "previous check" read as the previous accepted check; 11.3 W1). A fresh proof's `TrustedFacts.relied` is that list; the authorization names its first id and keeps the whole list | L7; the theory blocks a request when any relied grant is no longer live. |
| D37 | Error texts | `StoreFault`, `KeyError`, `OpenError` carry only fixed labels, codes, numbers and the data-directory path; no rusqlite or serde error is ever formatted; key statements live in `Zeroizing<String>` | rusqlite's `SqlInputError` prints the SQL, which holds the key; serde quotes values. |
| D38 | Frame bound | No owner frame is written over 1,048,576 bytes (CR section 1). The owner's own records (recipient sets, profiles, live grants) are kept within a budget of 720 KiB of canonical JSON: a command that would grow them past it is refused `invalid_params` and stores nothing; a command that shrinks them is never refused. An answer that still does not fit becomes `internal` with its id and method; a notice that does not fit becomes one `snapshot_required` change and ends the subscription. The broker runs on | The contract bounds frames; a snapshot must fit in one. The contract has no owner limit code (CCR-18). |
| D39 | Proof binding | A result's report must name its job's request, purpose and action digest (PLAN 8 step 5). One that does not proves nothing: the request goes `undecided`, nothing is recorded, the relied list is unchanged. This check runs before the grant check of D17 | A report filed under the request it names, not the job's, could fault the store or authorize another request. |

### 0.2 Package decisions (L1 to L7)

These were settled while the package was prepared. The labels stay because the code cites them.

| # | Decision | Where it lands |
|---|---|---|
| L1 | Recovery runs at every open, including a reopen (D9, D30, 5.8); the clock never goes back (D34); a key left without a store is reused and a store left without tables is created again (3.2, 3.4); no error formats key material or content (D37); a fresh Permit with no authorizing grant authorizes nothing (D17, D36). The packaged script has no cursor check: store survival is proven by the process tests its step 1 runs against the bundled broker (p04). `time.rs` moved from the broker into `fabric-storage`, and `fabric-broker-core` re-exports `format_timestamp` (section 2). A repeated agent call writes nothing (D25) | D9, D17, D25, D26, D30, D34, D36, D37; 2, 3.2, 3.4, 5.8, 5.10, 10.6 |
| L2 | Activity never raises the agent's revision; `get_proof` answers `not_found` for synthetic rows; open-card and open-request limits are checked only at creation, so a re-ask may exceed five open cards; the re-proof counter is per purpose and not reset by a retry; every agent call is activity, kept in memory, and the session ending is the durable event; `busy` is audited as one row per agent per one-minute window; D27 as written; D18 kept and stated in the desktop README; no owner-uid check, mode checks kept on Unix; `verify-desktop.sh` compiles the release broker; the crate is `crates/fabric-broker-core` (D1) | D5, D7, D13, D16, D20, D24, D25, D27, D35; 3.1, 3.3, 3.4, 3.6, 5.4, 5.5, 5.7, 5.9, 6.1, 10.6; section 9 |
| L3 | The owner's Q1 to Q10 stay open; the code runs on the defaults of 11.2. Q3: one Keychain prompt per ad-hoc rebuild is accepted. Q4: the synthetic agent stays opt-in, only with the test key and an explicit scratch data directory. Q8: proceed without the missing owner commands. Q9: the Isabelle prover is wired in a follow-up branch after CD-W03 merges. Q10: accepted for this package | 11.2, 1.2, 1.3, 7.3 |
| L4 | Section 4's envelope is a proposal, written up as ADR-115 (Proposed), and implemented as written | 4 |
| L5 | No change to `contracts/desktop/v1` or `crates/fabric-desktop-contracts`; contract change requests stay a list | 6.2 |
| L6 | W03 interface: `BrokerGate::check(&mut self, &Context, &Action) -> Result<Check, PolicyError>`; `Check` holds the verdict (Decided Permit/NeedApproval/Block, or Undecided with a reason), `failing_rule`, all 18 rule results, `proof_id`, `theory_sha256`, `isabelle`, `context_digest`, `action_digest`, `seconds`. The broker builds every `Context` field from its own records. A one-request grant is spent by the change it permits (for a read, by the release of its result); a request whose relied grant is no longer live is blocked even if another grant covers it | 10.3 `TrustedFacts` and its field table; 5.5 `used`; 5.6 dispatch checks |
| L7 | Every check result carries `authorizing_grants`. `Context.relied` is a list: empty for classification; for a fresh proof, the previous accepted check's `authorizing_grants`, or the grant a just-made approval created. (a) `ProofDone.authorizing_grants: Vec<Id>`, `TrustedFacts.relied: Vec<Id>`; (b) the store keeps the list (`ReliedGrantSet` carries a list); (c) `Authorization.grant_id` records the first id of the fresh proof's list, the store keeps all (CCR-15); (d) a fresh-proof Permit with an empty list issues no authorization and goes `undecided`; (e) `<data_dir>/prover` is reserved for the prover's work directory | D17, D28, D36; 3.3, 3.5, 5.7, 6.2, 10.2, 10.3 |

---------------------------------------------------------------------------------------------------------------------

## 1. Scope

### 1.1 CD-W04 delivers (PLAN 13 row W04; RM 242)

1. `crates/fabric-storage`: SQLCipher store (macOS) / plain SQLite (Linux, Windows, tests only), key providers,
   single-writer lock, migrations with a verified pre-migration backup, start-up checks, refusal of a newer schema,
   hash-chained ledger, access audit, typed mutations and reads, and the contract timestamp formatter (`time.rs`,
   moved from BR).
2. `crates/fabric-broker-core`: the single-writer service. Request reducer over the 23 states; grants with three
   terms; sessions; pause and revocation epochs; relied grant lists; exact-action authorizations used once, durably
   (DEF-4); effect reservation, dispatch intent, attempts, `outcome_unknown` at restart; request limits; restart
   recovery (at start and at every reopen) and quit; the owner channel (13 methods, dedup, revisions, notices,
   subscribe, the frame bound); the agent entry including `fabric_request_status`; the prover and provider ports.
3. `apps/fabric-broker`: composition (configuration checked first, store, service, workers), the synthetic agent;
   `src/spike.rs` and `src/time.rs` deleted.
4. Shell changes to run it: relay allowlist and shared variable names, host store settings, harness, packaged and
   release checks, banner, desktop README.
5. Gate G2a evidence (7.3).

### 1.2 Left to other packages, and their port

| Package | Owns | Plugs into |
|---|---|---|
| CD-W03 (PR 22) | theory, `BrokerGate`, `Check.authorizing_grants` | `ports::Prover` (10.3). An `IsabelleProver` adapter (`apps/fabric-broker/src/prover.rs`: `TrustedFacts` -> `Context`, `Check` -> `ProofDone`, work directory `<data_dir>/prover`) comes in a follow-up branch after CD-W03 merges (L3, Q9) |
| CD-W05 | OAuth, token refs, provider calls, observations, reconciliation (`outcome_unknown -> reconciling -> ...`, `applied -> reconciled`), held results, `connect_google`, provider token revocation | `ports::Provider` (extends it with reconcile), migration 2+ (3.6, R22), `TrustedFacts.event`, `Fence.event_revision`, `GrantFact.used` for reads |
| CD-W06/W07 | socket, enrollment, key-possession handshake, runtime | `Input::Agent(AgentInput)`, `Output.agent`, `Output.close`; enrollment replaces `Broker::seed` |
| CD-W08 | screens, `selected` narrowing, pause/revoke UI | the owner channel |
| CD-W09 | warnings, zone checks | `PrivateDetails.warnings` stays `[]`; preferences stored now |

### 1.3 Not in W04

Owner-facing backup/restore and redacted export (no contract method); audit deletion; multi-device envelope fields;
the missing owner commands of CR 15 (L3, Q8: proceed without them); read-result storage and disclosure (D33);
Isabelle wiring (L3, Q9); SQLCipher third-party notices in the bundle (CD-W11; recorded in
`apps/fabric-desktop/README.md` now).

### 1.4 What the owner sees

`sh scripts/build-desktop.sh --configuration debug`, launch: store `ready` (Keychain prompt on first run and after
each ad-hoc rebuild, L3, Q3), prover `unavailable`, empty lists. A synthetic demo never touches that store: from a
terminal,

```sh
FABRIC_DESKTOP_DATA_DIR="$(mktemp -d)" FABRIC_BROKER_KEY_SOURCE=test FABRIC_BROKER_SYNTHETIC_AGENT=1 \
  "<bundle>/Contents/MacOS/fabric-desktop"
```

Once the window has subscribed, a synthetic agent and account appear, a request reaches `pending_approval`, approval
creates a grant, the request ends `undecided`, and everything survives a restart with the same directory. The host
refuses the test key or the synthetic agent without `FABRIC_DESKTOP_DATA_DIR` (configuration error, no broker).
Nothing can be authorized or dispatched until CD-W03 is wired and CD-W05 lands.

---------------------------------------------------------------------------------------------------------------------

## 2. Crates and modules

```
fabric-desktop-contracts  <-  fabric-storage  <-  fabric-broker-core  <-  apps/fabric-broker
                                                                          (later: the IsabelleProver adapter over
                                                                           fabric-broker-policy; fabric-google-calendar, W05)
```

Modules:
- `fabric-storage`: `lib`, `key`, `keychain` (macOS), `store` (open, probe, pragmas, lock, checks), `migrate`,
  `migrations/0001_initial.sql`, `tx` (`WriteTx`, `OpMeta`, faults), `mutation`, `apply` (the only Rust code that
  writes projection tables), `rows`, `read` (`ReadTx`), `ledger` (envelope, hashes, operation ids, verify, replay),
  `backup`, `time` (moved unchanged from BR `src/time.rs`, with its tests), `testing` (unit-test helpers).
- `fabric-broker-core`: `lib` (re-exports `fabric_storage::time::{format_timestamp, MAX_TIMESTAMP_MS}`), `broker`
  (facade, unavailable mode, reopen, halt), `ports`, `ids`, `service/{mod, core, clock, activity, reducer, requests,
  grants, relied, sessions, epochs, facts, pipeline, agent, limits, recovery}`, `owner/{mod, frames, dedup, commands,
  profiles, groups, preferences, views, subscribe}`.
- `apps/fabric-broker`: `main`, `config`, `reader` (unchanged), `synthetic` (`cfg(debug_assertions)`), `lib`.

Dependencies. `fabric-storage`: the contracts crate, `getrandom`, `serde`, `serde_json`, `zeroize`; on macOS
`rusqlite =0.43.0` with `bundled-sqlcipher` and `security-framework`; elsewhere `rusqlite =0.43.0` with `bundled`.
Its `crash` test runs with `harness = false`. `fabric-broker-core`: the contracts crate, `fabric-storage`,
`getrandom`, `serde`, `serde_json`, and rusqlite as a dev-dependency (the second connection of 7.2).
`apps/fabric-broker` adds `fabric-broker-core` and `fabric-storage`. New locked crates: fallible-iterator,
fallible-streaming-iterator, hashlink, libsqlite3-sys, rsqlite-vfs, rusqlite, sqlcipher-src, sqlite-wasm-rs,
thiserror and thiserror-impl (the last four reached only through the wasm target but locked and fetched). No crate the
frozen demo uses moved.

`.cargo/config.toml` sets `[env] LIBSQLITE3_FLAGS = "-USQLITE_TEMP_STORE -DSQLITE_TEMP_STORE=3"` (SPIKE 2: file
temp storage leaks plaintext; `libsqlite3-sys`'s build script passes the tokens through). `.gitattributes` keeps
`*.sql` at LF, so the frozen schema compares the same on a Windows checkout.

Forbidden dependencies (PLAN 4.3, 4.4; RM RD-9, RD-11), checked by `crates/fabric-broker-core/tests/fitness.rs`
(manifest scan):

| Crate | Must not depend on |
|---|---|
| `fabric-storage` | `fabric-broker-core`, `fabric-policy`, `fabric-token`, `fabric-broker-policy`, `fabric-demo`, `fabric-core`, any HTTP/async/model crate |
| `fabric-broker-core` | `fabric-policy`, `fabric-token`, `fabric-broker-policy`, `fabric-demo`, `fabric-core`, any HTTP/async/model crate, Tauri |
| `apps/fabric-broker` | `fabric-demo`, `fabric-token`, `fabric-policy` (the Isabelle wiring adds `fabric-broker-policy` only for the adapter) |
| relay, host | `fabric-storage`, `fabric-broker-core` |

The demo's `AuditLog` and `fabric-token`'s `ReplayCache` are not used (PLAN 11 para 5; DEF-1, DEF-4).

---------------------------------------------------------------------------------------------------------------------

## 3. Storage

### 3.1 Engine and connection

SQLCipher 4.20.0 community / SQLite 3.54.1 via `rusqlite =0.43.0` `bundled-sqlcipher` on macOS (CommonCrypto, no
OpenSSL); `bundled` plain SQLite elsewhere (SPIKE option (c)). One read-write connection, owned by the service
thread; every write is `BEGIN IMMEDIATE ... COMMIT`. Start-up also uses one short read-only probe connection (3.4).

Key statement (macOS): `PRAGMA key = "x'<64 hex>'"` built in a `Zeroizing<String>`, executed, dropped. An error of
that statement or of the keyed `ATTACH` (3.6) maps to a fixed result (`WrongKey` or `StoreFault::Sql { op, code }`);
the rusqlite error is never formatted, logged or kept (its `SqlInputError` prints the SQL; D37).

On every keyed connection (macOS), right after the key statement and before any other statement, the build
assertions run: `cipher_version` returns a row; `cipher_provider` = `commoncrypto`; `compile_options` contains
`HAS_CODEC` and `TEMP_STORE=3`; then `cipher_log_level = NONE` (no SQLCipher log line on stdout or stderr; a level
other than `NONE` is refused). A failure is `OpenError::Config` (exit 78), never plaintext. `SQLITE_NOTADB` there is
`WrongKey`; a passing I/O code is `Io`.

Pragmas on the read-write connection, in order, each asserted: `cipher_memory_security = ON` (macOS),
`temp_store = MEMORY`, `secure_delete = ON`, `journal_mode = WAL` (assert `wal`), `synchronous = FULL`,
`fullfsync = ON`, `checkpoint_fullfsync = ON`, `foreign_keys = ON`, `busy_timeout = 250`.

File assertion, macOS only (elsewhere the test store is plaintext by design, D10): an existing `broker.db` of 16
bytes or more whose first 16 bytes are `SQLite format 3\0` is `Unavailable::NotEncrypted`. A file that cannot be
opened or read for this check is `Unavailable::Io`.

### 3.2 Keys

- Sources: `keychain` (macOS only; generic password, service `ai.paracels.fabric.calendar.store`, account
  `database-key-v1`; code copied from `crates/fabric-token/src/keys.rs`, not a dependency) and `test`
  (`TEST_KEY = [0x46; 32]`; accepted by the broker only in debug builds).
- No `broker.db`: call `read()` first. A key that exists (left by a crash between creating the key and creating the
  file) is reused. Only when `read()` answers `None`: generate 32 bytes, `create`, then create the file. A file never
  exists without its key, and a crash in that window never locks the store.
- An existing file with no key: `KeyMissing`; never a new key over an existing file.
- `key_kind` is written in `meta` at creation; a provider of another kind: `KeyKindMismatch` (3.4 step 8). Because a
  Keychain key never equals the public test key, step 8 cannot see a test-key store opened by a Keychain-kind
  provider: that provider meets `KeyMissing` or `WrongKey` first. So when a non-test provider meets either at step 3
  or 4, the store tries the public test key read-only, and if it opens the file and `meta.key_kind` is `test`, the
  answer is `KeyKindMismatch { stored: Test }`. The reverse case, a Keychain store opened with the test key, stays
  `WrongKey` (the stored kind cannot be read without the Keychain key; R26).
- Keychain status mapping: `errSecItemNotFound` (-25300) -> no key; `errSecUserCanceled` (-128), `errSecAuthFailed`
  (-25293) -> `KeyError::Refused` -> `Unavailable::KeychainRefused`; `errSecInteractionNotAllowed` (-25308) ->
  `KeyError::Locked` -> `Unavailable::KeychainLocked`; any other status -> `KeyError::Failed { status }` ->
  `Unavailable::KeychainLocked` (retried, reported `locked`). An item that does not hold 32 bytes is
  `Failed { status: errSecDecode }`.
- A failure of the system random source in `StoreKey::generate`, or when drawing the new store's id, is not a
  Keychain status and never maps to one: `KeyError::Random` -> `Unavailable::KeyGeneration` (retried like
  `KeychainLocked`, reported `locked`, with its own owner message: "The broker could not get random bytes from the
  system to create the store key. Nothing was written; it tries again when the page next asks.").
- The key lives in `Zeroizing`; never in arguments, environment, stdout, stderr, logs, error values or the ledger
  (PLAN 12 para 3). A key read from the Keychain is copied straight into zeroed-on-drop memory
  (`StoreKey::from_slice`); the framework's own buffer cannot be wiped under `forbid(unsafe_code)` (R25).
- Tests never touch the Keychain: library tests use `TestKey` or a scripted `KeyProvider`; every binary test sets
  `FABRIC_BROKER_KEY_SOURCE=test`. The one Keychain test is `#[ignore]`d, uses service
  `ai.paracels.fabric.calendar.store.test`, and is the owner's manual check M1 (7.3).

### 3.3 Configuration and files (DEF-2)

The relay starts the broker with `env_clear()` and no arguments (RELAY `config.rs`). The broker checks its
environment before reading stdin or starting any thread. Refusal: one stderr line naming the variable and the
reason (never the value of a variable other than a path), **exit 78**.

| Variable | Values | Refused when |
|---|---|---|
| `FABRIC_BROKER_DATA_DIR` | absolute directory | unset, relative, an existing non-directory; a directory that cannot be created, or whose mode is open to others, is refused by the store at open (`OpenError::Config`, also exit 78) |
| `FABRIC_BROKER_KEY_SOURCE` | `keychain`, `test` | unset; another value; `keychain` off macOS; `test` in a release build |
| `FABRIC_BROKER_SYNTHETIC_AGENT` | absent or `1` | another value; set in a release build; `1` with a key source other than `test` |
| any other name starting `FABRIC_BROKER_` | none | always (D27) |

The broker's configuration never creates the data directory (a directory it made with the default mode would then
fail the store's mode check). The variable names are spelt once in the relay (`BROKER_DATA_DIR`,
`BROKER_KEY_SOURCE`, `BROKER_SYNTHETIC_AGENT`, `BROKER_VARIABLE_PREFIX`, `is_broker_variable`, `same_name`); the
host uses the relay's, and the broker's own copy is pinned by a test to the same literals.

Data directory and files: the store creates the directory if it is missing (mode 0700 on Unix) and its files with
mode 0600. On Unix an existing data directory whose mode grants group or other any permission is
`OpenError::Config { check: "data directory mode" }`. There is no owner-uid check (std has no safe `getuid`; L2).
Contents: `broker.db` (+ `-wal`, `-shm`), `broker.lock` (`File::try_lock` for the life of the store; released by the
OS on death), `backups/`, and `prover/`, reserved for the prover's work directory, which the Isabelle wiring sets
(L7(e)); W04 neither creates nor reads `prover/`. Exit codes: 64 usage and 74 stdin/stdout failure keep their
meaning; 78 configuration; 75 halt after a failed write (5.2).

### 3.4 Start-up, in order (PLAN 11 para 8)

1. Data directory: create if missing; Unix mode check (3.3).
2. Lock `broker.lock`; held -> `Unavailable::InUse`.
3. Key (3.2). Remember whether `broker.db` existed before this step.
4. Probe, only for an existing file: the macOS plaintext-header check (3.1); a read-only connection
   (`SQLITE_OPEN_READ_ONLY`), the key and the build assertions (macOS), then `PRAGMA user_version` and
   `SELECT count(*) FROM sqlite_schema`; `SQLITE_NOTADB` or a key failure -> `WrongKey` (or `KeyKindMismatch`, 3.2),
   `SQLITE_BUSY` -> `InUse`, a passing I/O code -> `Io`. Close the probe. No pragma that writes runs before this step.
5. `user_version` > the newest migration -> `NewerSchema`; nothing further is read or written. `user_version = 0`
   with a non-empty `sqlite_schema` is `Integrity` (`check: "unversioned schema"`).
6. Create the file privately if it did not exist; open the read-write connection with the pragmas and assertions of
   3.1.
7. Not created yet (no file before step 3, or `user_version = 0` and an empty `sqlite_schema`, which a crash between
   `Connection::open` and the first migration's commit leaves): draw a store id and run every migration (3.6) with no
   backup, since there is nothing to save; continue at step 10.
8. A created store: `meta.key_kind` differs from the provider's kind -> `KeyKindMismatch`; `PRAGMA integrity_check` =
   `ok`; macOS `PRAGMA cipher_integrity_check` no rows; `PRAGMA foreign_key_check` no rows; ledger: every
   `payload_hash`/`envelope_hash` recomputed and the chain verified from seq 1, gap-free, known types and versions,
   `committed_at_ms` non-decreasing, head equal to `meta.ledger_head_*`. Any failure -> `Integrity` naming the first
   failing check.
9. `user_version` < `SCHEMA_VERSION`: backup (3.6), then migrate.
10. Invariant queries, written for `SCHEMA_VERSION` and therefore run after any migration; each must return no row:
    a request state not among the 23 or `terminal` disagreeing with it; `pending_approval` without a card; more than
    one live session per agent; a session grant whose session is unknown; an `issued` authorization of a run earlier
    than the newest run; a relied grant id that names no grant of the request's agent; the active profile not
    existing. Failure -> `Integrity` naming the query.
11. Return `Store`.

Steps 1-5 write nothing to `broker.db`, so a refusal there leaves the file byte-identical (step 1 may create the
directory, step 3 may create a Keychain item when no file exists). Refusals at steps 8 and 10 run no transaction, so
the logical content is unchanged; closing the connection may checkpoint the WAL into the main file.

A failed statement at any step is `Io` when SQLite answers a passing I/O code (`CANTOPEN`, `IOERR`, `FULL`, `NOMEM`)
and `Integrity` otherwise. A bad result, a broken chain, a decode failure or `CORRUPT` stay `Integrity`; a file error
of the store's own (a file where `backups/` belongs) stays `Integrity { check: "backup" }`.

The service then builds `Core` only through `Core::open`, which runs recovery as one transaction (5.8) and the first
pump, at start and at every reopen. Store outcomes:

| Outcome | Broker |
|---|---|
| `OpenError::Config` (build assertions, unusable data directory) | exit 78 before any frame. At a reopen it keeps the earlier reason |
| `Unavailable::{InUse, KeychainLocked, KeyGeneration, Io}` | stay up; `store_unavailable{locked}`; on an owner frame that needs the store, at most every 5 s, try `Core::open` again; on success recovery and the first pump run, then the frame is answered |
| `Unavailable::KeychainRefused` | stay up; `store_unavailable{keychain_refused}`; no reopen in this run |
| `Unavailable::{KeyMissing, WrongKey, KeyKindMismatch, NotEncrypted, NewerSchema, Integrity}` | stay up; `store_unavailable{locked}`; final for the run |

In the unavailable mode `hello` answers normally (with `Limits::CD_G0`), every other owner method, `connect_google`
included, answers `store_unavailable` with a message built from the reason alone (example: "The store was written by
a newer Fabric Calendar (schema 2); this build reads schema 1. Nothing was changed."), an agent connect is closed
and an agent call answered `internal`. The `Io` message never tells the owner to move the store aside.

### 3.5 Schema version 1 (`crates/fabric-storage/migrations/0001_initial.sql`)

Timestamps are integer UTC ms. `record`, `action`, `scope`, `card`, `approval`, `report`, `fence`, `grant_ids`,
`relied_grant_ids` columns hold canonical JSON (`fabric_desktop_contracts::canonical`) of the named contract or row
type. The file is the source; this is its content.

```sql
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;
  -- store_id, key_kind, schema_version, created_at_ms, notice_sequence, ledger_head_seq, ledger_head_hash
CREATE TABLE runs (run_id INTEGER PRIMARY KEY, started_at_ms INTEGER NOT NULL, ended_at_ms INTEGER,
  clean INTEGER NOT NULL DEFAULT 0 CHECK (clean IN (0,1))) STRICT;
CREATE TABLE counters (name TEXT PRIMARY KEY CHECK (name IN ('pause_epoch','revocation_epoch')),
  value INTEGER NOT NULL CHECK (value >= 0)) STRICT;

CREATE TABLE ledger (
  seq INTEGER PRIMARY KEY,
  operation_id TEXT NOT NULL UNIQUE,
  operation_type TEXT NOT NULL,
  payload_version INTEGER NOT NULL,
  committed_at_ms INTEGER NOT NULL,
  run_id INTEGER,
  actor_kind TEXT NOT NULL CHECK (actor_kind IN ('owner','agent','broker')),
  actor_agent_id TEXT,
  idempotency_scope TEXT, idempotency_key TEXT,
  causation_id TEXT, correlation_id TEXT,
  payload TEXT NOT NULL, payload_hash TEXT NOT NULL,
  previous_hash TEXT NOT NULL, envelope_hash TEXT NOT NULL UNIQUE,
  CHECK ((actor_kind = 'agent') = (actor_agent_id IS NOT NULL)),
  CHECK ((idempotency_scope IS NULL) = (idempotency_key IS NULL)),
  UNIQUE (idempotency_scope, idempotency_key)) STRICT;
CREATE INDEX ledger_correlation ON ledger(correlation_id, seq) WHERE correlation_id IS NOT NULL;
CREATE TRIGGER ledger_no_update BEFORE UPDATE ON ledger BEGIN SELECT RAISE(ABORT, 'ledger is append-only'); END;
CREATE TRIGGER ledger_no_delete BEFORE DELETE ON ledger BEGIN SELECT RAISE(ABORT, 'ledger is append-only'); END;

CREATE TABLE audit (audit_id INTEGER PRIMARY KEY, at_ms INTEGER NOT NULL, run_id INTEGER,
  actor_kind TEXT NOT NULL CHECK (actor_kind IN ('owner','agent','broker')), actor_agent_id TEXT,
  kind TEXT NOT NULL, request_id TEXT, detail TEXT NOT NULL) STRICT;
CREATE TRIGGER audit_no_update BEFORE UPDATE ON audit BEGIN SELECT RAISE(ABORT, 'audit is append-only'); END;
CREATE TRIGGER audit_no_delete BEFORE DELETE ON audit BEGIN SELECT RAISE(ABORT, 'audit is append-only'); END;

CREATE TABLE profile_versions (profile_id TEXT NOT NULL, version INTEGER NOT NULL CHECK (version >= 1),
  record TEXT NOT NULL, created_at_ms INTEGER NOT NULL, PRIMARY KEY (profile_id, version)) STRICT;
CREATE TABLE profile_state (id INTEGER PRIMARY KEY CHECK (id = 1), active_profile_id TEXT NOT NULL,
  active_version INTEGER NOT NULL, revision INTEGER NOT NULL) STRICT;          -- seeded: careful, revision 0
CREATE TABLE preferences (id INTEGER PRIMARY KEY CHECK (id = 1), revision INTEGER NOT NULL,
  settings TEXT NOT NULL) STRICT;                                              -- seeded: revision 0, empty (CD-Q11i)
CREATE TABLE recipient_sets (recipient_set_id TEXT PRIMARY KEY, revision INTEGER NOT NULL,
  deleted_at_ms INTEGER) STRICT;
CREATE TABLE recipient_set_versions (recipient_set_id TEXT NOT NULL REFERENCES recipient_sets,
  revision INTEGER NOT NULL, record TEXT NOT NULL, created_at_ms INTEGER NOT NULL,
  PRIMARY KEY (recipient_set_id, revision)) STRICT;

CREATE TABLE accounts (account_id TEXT PRIMARY KEY, revision INTEGER NOT NULL, state TEXT NOT NULL,
  record TEXT NOT NULL) STRICT;                                                -- contract Account (calendars inside)
CREATE TABLE agents (agent_id TEXT PRIMARY KEY, revision INTEGER NOT NULL,
  paused INTEGER NOT NULL CHECK (paused IN (0,1)), record TEXT NOT NULL,      -- AgentRow (no activity fields)
  enrolled_at_ms INTEGER NOT NULL) STRICT;
CREATE TABLE sessions (session_id TEXT PRIMARY KEY, agent_id TEXT NOT NULL REFERENCES agents,
  run_id INTEGER NOT NULL REFERENCES runs, started_at_ms INTEGER NOT NULL,     -- last activity: memory only (D35)
  ended_at_ms INTEGER, end_reason TEXT, CHECK ((ended_at_ms IS NULL) = (end_reason IS NULL))) STRICT;
CREATE UNIQUE INDEX sessions_one_live ON sessions(agent_id) WHERE ended_at_ms IS NULL;

CREATE TABLE requests (request_id TEXT PRIMARY KEY, agent_id TEXT NOT NULL REFERENCES agents,
  call_id TEXT NOT NULL, session_id TEXT REFERENCES sessions, account_id TEXT,
  state TEXT NOT NULL, terminal INTEGER NOT NULL CHECK (terminal IN (0,1)), revision INTEGER NOT NULL,
  received_at_ms INTEGER NOT NULL, record TEXT NOT NULL, action TEXT NOT NULL, scope TEXT,
  card TEXT, card_expires_at_ms INTEGER, approval TEXT,
  relied_grant_ids TEXT NOT NULL DEFAULT '[]',                                 -- JSON list of grant ids (D36)
  reproofs_classification INTEGER NOT NULL DEFAULT 0,
  reproofs_fresh_permit INTEGER NOT NULL DEFAULT 0,
  due_at_ms INTEGER,
  CHECK ((state = 'pending_approval') <= (card IS NOT NULL)),
  UNIQUE (agent_id, call_id)) STRICT;
CREATE INDEX requests_open_by_agent ON requests(agent_id) WHERE terminal = 0;
CREATE INDEX requests_received ON requests(agent_id, received_at_ms);
CREATE INDEX requests_due ON requests(due_at_ms) WHERE due_at_ms IS NOT NULL;

CREATE TABLE grants (grant_id TEXT PRIMARY KEY, agent_id TEXT NOT NULL REFERENCES agents,
  revision INTEGER NOT NULL,
  term_kind TEXT NOT NULL CHECK (term_kind IN ('one_request','session','until_revoked')),
  action_digest TEXT, session_id TEXT REFERENCES sessions,
  origin_request_id TEXT NOT NULL REFERENCES requests, record TEXT NOT NULL,
  ended_at_ms INTEGER, end_reason TEXT,
  CHECK ((term_kind = 'one_request') = (action_digest IS NOT NULL)),
  CHECK ((term_kind = 'session') = (session_id IS NOT NULL)),
  CHECK ((ended_at_ms IS NULL) = (end_reason IS NULL))) STRICT;
CREATE INDEX grants_live_by_agent ON grants(agent_id) WHERE ended_at_ms IS NULL;
CREATE INDEX grants_live_by_session ON grants(session_id) WHERE ended_at_ms IS NULL;

CREATE TABLE proof_reports (proof_row INTEGER PRIMARY KEY, proof_id TEXT NOT NULL,
  request_id TEXT NOT NULL REFERENCES requests, purpose TEXT NOT NULL,
  prover_kind TEXT NOT NULL CHECK (prover_kind IN ('isabelle','synthetic')),
  record TEXT NOT NULL, recorded_at_ms INTEGER NOT NULL) STRICT;              -- ProofRecord
CREATE INDEX proof_reports_id ON proof_reports(proof_id);                     -- not unique (THEORY 4 item 4)
CREATE INDEX proof_reports_request ON proof_reports(request_id, proof_row);
CREATE TRIGGER proof_reports_no_update BEFORE UPDATE ON proof_reports BEGIN SELECT RAISE(ABORT, 'append-only'); END;
CREATE TRIGGER proof_reports_no_delete BEFORE DELETE ON proof_reports BEGIN SELECT RAISE(ABORT, 'append-only'); END;

CREATE TABLE authorizations (authorization_id TEXT PRIMARY KEY, request_id TEXT NOT NULL REFERENCES requests,
  attempt INTEGER NOT NULL, run_id INTEGER NOT NULL REFERENCES runs, record TEXT NOT NULL, fence TEXT NOT NULL,
  grant_ids TEXT NOT NULL,                                                     -- the fresh proof's authorizing_grants
  issued_at_ms INTEGER NOT NULL, expires_at_ms INTEGER NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('issued','consumed','voided')),
  consumed_at_ms INTEGER, voided_at_ms INTEGER, void_reason TEXT,
  CHECK (expires_at_ms = issued_at_ms + 60000),
  CHECK ((state = 'consumed') = (consumed_at_ms IS NOT NULL)),
  CHECK ((state = 'voided') = (voided_at_ms IS NOT NULL)),
  UNIQUE (request_id, attempt)) STRICT;
CREATE UNIQUE INDEX authorizations_one_live ON authorizations(request_id) WHERE state = 'issued';
CREATE TABLE effects (effect_id TEXT PRIMARY KEY, request_id TEXT NOT NULL UNIQUE REFERENCES requests,
  tool TEXT NOT NULL, account_id TEXT NOT NULL, calendar_id TEXT NOT NULL, action_digest TEXT NOT NULL,
  provider_event_id TEXT, created_at_ms INTEGER NOT NULL,
  attempts_started INTEGER NOT NULL DEFAULT 0, first_attempt_at_ms INTEGER) STRICT;
CREATE UNIQUE INDEX effects_provider_event ON effects(account_id, calendar_id, provider_event_id)
  WHERE provider_event_id IS NOT NULL;
CREATE TABLE attempts (effect_id TEXT NOT NULL REFERENCES effects, number INTEGER NOT NULL,
  authorization_id TEXT NOT NULL UNIQUE REFERENCES authorizations, record TEXT NOT NULL,   -- contract Attempt
  PRIMARY KEY (effect_id, number)) STRICT;

CREATE TABLE commands (command_id TEXT PRIMARY KEY, method TEXT NOT NULL, digest TEXT NOT NULL,
  answer TEXT NOT NULL) STRICT;
```

The file ends with the seeds of the projection: both epochs at 0, the Careful profile (version 1) active at
revision 0, and empty preferences at revision 0 (CD-Q11i).

Projection tables: every table except `meta`, `ledger`, `audit`. Only `apply.rs` writes them from Rust (s16); the
migration SQL writes the seeds (D3, 3.6).

PLAN 11 para 2 families: accounts/selected calendars -> `accounts`; agents/enrollments -> `agents`; sessions ->
`sessions`; immutable profile versions -> `profile_versions`, `profile_state`; recipient groups/versions ->
`recipient_sets`, `recipient_set_versions`; global preferences -> `preferences`; grants/revocation epochs -> `grants`,
`counters`; requests/approvals/relied grants -> `requests`; proof reports -> `proof_reports`; effects/attempts ->
`effects`, `attempts`; consumed authorizations -> `authorizations`; command deduplication -> `commands`,
`requests(agent_id, call_id)`; audit lifecycle -> `ledger`, `audit`; schema migrations -> `user_version`,
`meta.schema_version`, ledger `schema.migrated`; token refs, provider observations, results -> CD-W05 migration 2+.
Per-account namespacing: every scope, calendar and effect names its account; a calendar is always named with its
account, so the same calendar id in two accounts is two calendars. `TrustedFacts` carries every connected account
(11.3 W2).

What `apply` guards (each mutation runs in a savepoint, so a refused guard leaves no part of it; a refusal is
`StoreFault::Invariant` and halts):
- `StoreCreated` and `SchemaMigrated` are the migration runner's; `WriteTx::apply` refuses them.
- Requests: `RequestInserted` has status `received`, revision 1 and `request.tool == action.tool()`; a move follows
  `RequestState::can_transition_to`; `CardOpened` comes before the move to `pending_approval` (the CHECK above);
  `ApprovalRecorded` only while the request is `pending_approval`.
- Revisions: `AgentPut`, `AccountPut`, `RecipientSetVersionAdded`, `ProfileVersionAdded` (version) start at 1 and
  rise by exactly 1; `RequestMoved`, `GrantEnded`, `ProfileStateSet`, `PreferencesPut` rise by 1;
  `ProfileVersionAdded` never touches the profiles revision. `CounterRaised` refuses only a decrease. Re-proof counts
  never decrease.
- Grants: `GrantInserted` has revision 1, is not ended, and its origin request is of the same agent; an ended grant
  never changes. `GrantInserted` comes before a `ReliedGrantSet` naming it, and every relied id is a grant of the
  request's agent.
- Authorizations: `AuthorizationIssued` is of the current run, `grant_ids[0] == authorization.grant_id`, every id a
  grant of the authorization's agent, and each issue of one request takes a new `attempt` number
  (`UNIQUE(request_id, attempt)`). `AuthorizationConsumed` needs `issued`, `at_ms < expires`, the current run.
  A voided authorization is never consumed or voided again.
- Effects: `EffectReserved` has the request's tool, a `provider_event_id` if and only if the tool is a create, no
  attempt yet. `DispatchIntent` needs `attempt.number == effects.attempts_started + 1`, a `consumed` authorization of
  the same request.
- Sessions: `SessionStarted` is of the current run and a known agent; a session ends once.

### 3.6 Migrations and backup

- `SCHEMA_VERSION = 1`; `MIGRATIONS: &[Migration { to, sql }]`, SQL via `include_str!`; a list with a gap is
  `OpenError::Config { check: "migration list" }`. Each migration runs in one transaction that sets `user_version`
  and `meta.schema_version` and appends one `schema.migrated` ledger row with `run_id` null. Migration 1 of a new
  store writes one row whose payload is `[SchemaMigrated {0, 1}, StoreCreated {..}]`: no separate `store.created`
  row is written, though the type stays in the list of section 4. The migration SQL may seed projection rows (version
  1 seeds `counters`, `profile_state`, `preferences`); `verify_replay` runs the same SQL at each `SchemaMigrated`.
- A migration row's `committed_at_ms` is `max(OpenOptions.now_ms, the last committed_at_ms)` (no earlier row for
  migration 1 of a new store): the core clock does not exist yet, and a wall clock that went back between two runs
  must not write a decreasing ledger time, which the next start's `verify_chain` would refuse as `Integrity` for
  good (D34; s08). The backup name's `at_ms` is the same value. A failure inside migration 1 rolls back to a file
  with no tables, which the next open treats as not created (3.4 step 7).
- Released migration SQL is frozen: a test compares migration 1 on an empty database, and a store made by
  `Store::open`, with `crates/fabric-storage/tests/fixtures/schema-v1.sql` (normalized `sqlite_schema` dump; CRLF read
  as LF).
- Before migrating a created store: `wal_checkpoint(TRUNCATE)` (busy -> `StoreFault::Busy`), then
  `backups/broker-v{n}-{store_id}-h{ledger_head_seq}-{at_ms}.db` (the name binds schema and ledger watermark) via
  `ATTACH ... KEY "x'..'"` (statement in `Zeroizing<String>`, errors mapped to fixed text);
  `SELECT sqlcipher_export('backup'); DETACH` (macOS) or `VACUUM INTO` (elsewhere). Then reopen the backup read-only
  with the same key, run `verify_chain` and `verify_replay` on it, and check its head and version equal the name's
  (PLAN 11 para 8 "verifies replay"). Failure removes the backup file, leaves the store unavailable (`Integrity`,
  `check: "backup"`, or `Io` for a passing I/O code) and the store file unchanged.
- No reverse migration; downgrade = restore the backup by hand (PLAN 11 para 8). Backups contain no token.
- Replay applies this build's `apply` code to every earlier ledger row. A later migration that changes a table
  `apply` writes (a renamed or retyped column, a changed constraint) would make the rows before it replay wrongly. So
  until replay is version-aware (the apply and decode rules chosen by the replay database's `user_version`,
  `verify_chain` accepting every known `payload_version`, and a rehearsal migration that alters a projection table),
  a migration may only add: new tables, new nullable or defaulted columns, new indexes. This must be settled before
  CD-W05's migration 2 (R22).

---------------------------------------------------------------------------------------------------------------------

## 4. Ledger and envelope (RD-14; ADR107 D1-D3; ADR-115)

Rule: every projection change is a `Mutation` applied inside `Store::write`; at commit exactly one ledger row is
appended for the transaction (if it applied at least one mutation) and the head in `meta` is updated in the same
transaction. The projection is the fold of the mutations: `verify_replay` rebuilds an in-memory database from the
ledger (running migration SQL at each `schema.migrated`) and compares every projection table. `Store::write` refuses
an operation type outside the list below and a row dated before the last one (D34). A fault that ends the
transaction poisons the `WriteTx`: every later `apply`, `notify` or `audit` of the closure fails, and the write
returns the fault even when the closure went on, so no part of a failed write commits.

The envelope is proposed to the owner as ADR-115 (Q1) and implemented as written (L4). If the owner changes it,
every hash vector changes.

| Field | Rule | ADR107 D2 |
|---|---|---|
| `seq` | gap-free from 1 | `device_sequence` (`meta.store_id` stands for `device_id`) |
| `operation_id` | `op-` + 32 hex (128 random bits), generated by `fabric-storage` | `operation_id` (no UUIDv7 in tree) |
| `operation_type`, `payload_version` | list below; version 1; unknown type or version fails verification | same |
| `committed_at_ms` | the step's `now` from the core's clock (D34); never below the previous row; `verify_chain` checks it | replaces HLC |
| `run_id` | the current run after the write's mutations (so the recovery row carries the new run and the quit row the ending run); null only on migration rows | new |
| `actor_kind`, `actor_agent_id` | `owner`, `broker`, or `agent` + the agent id the adapter authenticated | **the agent field** |
| `idempotency_scope`, `idempotency_key` | rule below; UNIQUE | same |
| `causation_id`, `correlation_id` | cause: the owner command id, the agent call id, or the proof id of the report being settled; null otherwise. Correlation: the request id; null on rows not tied to a request (connect, disconnect, seed, timers, start, quit, the health write) | same |
| `payload`, `payload_hash` | canonical JSON `{"mutations":[...]}`; `sha256_hex(payload)` | same |
| `previous_hash`, `envelope_hash` | see below | `previous_device_hash` (SHA-256, no BLAKE3 in tree) |
| absent | `space_id`, `key_epoch`, HLC, COSE signature | deferred to ADR107 S1 (PLAN 11 para 3) |

Idempotency: `owner_command`/`<command id>` on a command's transaction or on its `owner.refused` transaction (only
one of the two ever commits for one id); `agent_call`/`<agent_id>/<call_id>` only on the transaction that inserts the
request; `startup`/`<run id>` on the recovery transaction. Every other transaction carries none: a repeated agent
call and `fabric_request_status` write nothing at all, and session starts and ends, timers, the pipeline and quit
carry `None`. A UNIQUE violation is therefore a fault (5.2), never a normal repeat.

```text
envelope_hash = sha256_hex("fabric-broker-ledger-v1\n" + canonical({seq, operation_id, operation_type,
                payload_version, committed_at_ms, run_id, actor: {kind, agent_id}, idempotency: {scope, key} | null,
                causation_id, correlation_id, payload_hash, previous_hash}))
previous_hash = envelope_hash of seq - 1; "0" x 64 for seq 1
```

No signature: the signing key would sit beside the store key, readable by the same process; it adds nothing against
anyone who can rewrite the store (same-user malware is an accepted residual, PLAN 4.3 para 2). The chain detects
edits, reordering and truncation inside the file; SQLCipher's page HMAC detects page tampering. Not detected: the
whole file replaced by an older copy (R14). Canonical JSON, not CBOR (CBOR via `fabric-token` links `fabric-policy`,
RD-9).

Operation types (fixed; tests target them by name): `store.created`, `schema.migrated`, `broker.started`,
`broker.quit`, `broker.seed`, `owner.decide_request`, `owner.pause_agent`, `owner.revoke_grant`, `owner.save_profile`,
`owner.save_preferences`, `owner.save_group`, `owner.disconnect_google`, `owner.refused`, `agent.connect`,
`agent.disconnect`, `agent.call`, `timer.session_idle`, `request.advance` (pump steps, and the prover-health write,
which only notifies and appends no row), `request.proof` (results and the watchdog), `request.authorize` (only an
accepted fresh Permit that authorizes), `request.dispatch` (the handoff, or its void), `request.attempt`.

Access audit (`audit` table; not chained, never replayed, same writer, fail-closed for the first four kinds):
`request_details_full` (owner `get_request_details` with `redaction: full`, detail `{frame_id}`, committed before the
answer); `command_reused` (detail `command_id, method, stored_method`); `unknown_agent` (actor broker, detail
`agent_id, connection`; one row per connection attempt); `not_permitted` (actor broker, detail `connection, call_id,
tool`); and `agent_busy`: the first `busy` of an agent opens a window of 60 s in memory that counts its `busy`
refusals; when the window ends (a pump step) or at quit one row `{agent_id, window_start_ms, window_end_ms, count,
limits}` is written; a crash loses at most the open window's count (R16). Audit-only writes use `Store::write_audit`
and append no ledger row. One logical request = one `requests` row + many ledger rows with `correlation_id` = request
id (PLAN 11 para 3, PLAN 5 R09).

---------------------------------------------------------------------------------------------------------------------

## 5. Application service

### 5.1 One writer

`Broker` is a pure state machine on the main thread; the caller passes the raw wall-clock `now_ms` each call (the
executable reads `SystemTime`, not a start time plus `Instant`, because `Instant` on macOS does not advance while the
Mac sleeps). It owns the one `Store`. Workers (stdin reader, prover, provider, later agent adapters) never touch the
store; their results come back as `Input`s. Revoke and pause are ordinary inputs, so they never wait behind a proof
(PLAN 5, PLAN 8), and all accounts' authority transitions are serialized. `handle` reads the step's `now` once, runs
due work, then the input, then due work again, so a step the input made due (an approval's fresh proof) starts
without waiting for the next wake-up; notices of the first pump precede the answer. A pump takes due requests one at
a time, at most 10 000 steps per pump (a bound against a fault of the pump, not a limit of normal work).

`Core` is built only by `Core::open` (10.5): open the store through the `Opener`, start the clock, run the recovery
transaction (5.8), run the first pump. `Broker::start` and the reopen path both use it.

In memory only (safe to lose; every restart ends every session anyway): in-flight proof jobs (job id -> request id,
purpose, action digest, fence, deadline, the grant ids of the job's `facts.grants`); agent connections (connection
-> agent id, session id); agent activity (D35: per live session `last_active_ms`, per agent `last_tool_call_ms`);
open `busy` audit windows; the subscription flag, the 256-notice ring and its base; the last reopen attempt; the
clock. After a halt (D7) every call answers the halt again; after `quit` nothing more is taken.

Clock (D34). At `Core::open`: `last = max(raw, last committed_at_ms)` (the last row may be a `schema.migrated` row
of this open, already floored, 3.6), `offset = last - raw` (0 when the wall clock is ahead). Each step:
`t = raw + offset`; if `t < last` then `offset += last - t` and `t = last`; `last = t`. Every `now` of the step
(decisions, lifetimes, `committed_at_ms`) is that `t`; `Core::write` sets `OpMeta.now_ms` to it. A backward
wall-clock step mid-run neither halts nor lowers time; the elapsed time across the step counts as zero, and after it
lifetimes (authorization 60 s, idle 30 min, card 24 h) run at wall rate. A forward step shortens them (fail-closed).
When the offset changes the step adds one stderr line to `Output.stderr` ("the clock went back <n> ms; time
continues from the last commit").

### 5.2 Steps, atomicity, halt

Each step is one `Store::write(OpMeta, |tx| ...)`: reads and decisions inside the transaction, mutations, notices,
audit rows, one ledger row, `COMMIT`. Frames, jobs and changes to the in-memory activity are built from the closure's
returned value only after commit. A closure returning `TxError::Refused` rolls back and appends nothing. A
`TxError::Fault` (I/O, busy, constraint, `apply` invariant) halts: the frame in hand gets `store_unavailable{locked}`
(owner, with a fixed message) or `internal` (agent); `Output.halt` holds the fault (its Display is fixed text, D37);
the jobs of the halted step are dropped, including those an earlier commit of the same step queued (`Output.jobs` is
empty in a halted output; a `dispatching` committed earlier becomes `outcome_unknown` at the next start); main writes
the frames, prints one stderr line, exits 75. The next start runs every check (3.4).

### 5.3 Commands (CR 4, 7; PLAN 9.1)

1. Frame/protocol/schema refusals: `refusal_answer` (moved from BR `spike.rs`); never stored.
2. `connect_google`: `unknown_method`, "This build cannot connect a Google account yet; CD-W05 adds it."; not stored
   (CR 5 reserves that code for a method the channel does not have; CCR-1).
3. Look up `commands` by frame id: same `OwnerRequest::command_digest()` -> the stored answer, byte for byte, nothing
   written, no notices; different digest -> `command_reused` (+ audit row through `write_audit`, committed before the
   answer).
4. Run the command in one write (idempotency `owner_command`/id, causation the command id; `decide_request` also
   carries the request id as correlation): mutations, notices, `CommandAnswered` with the answer line (including
   `commit_sequence`). Commit, then answer: the command's notices (written while subscribed) come first, then its
   response.
5. A decided refusal (`not_found`, `conflict`, `stale_request`, `invalid_params`, `expansion_not_acknowledged`):
   rollback, then a write of type `owner.refused` with only `CommandAnswered`; then answer.
6. `expected_revision` mismatch -> `conflict{current_revision}`, except `decide_request` (`stale_request`, D22).
7. A failed read or write answers the frame in hand `store_unavailable{locked}` with fixed text and halts (5.2).

### 5.4 Revisions, notices, subscribe

- Each record revision rises by exactly 1 per change (`apply` enforces); a request's revision rises on every move.
- Agent revision: rises by 1 in each commit that enrolls (or seeds) the agent, that a `pause_agent` naming it makes,
  or that starts or ends its session (durable changes of the `Agent` record), never by more than 1 in one commit: a
  connect that replaces a live session ends the old one with `SessionEndReason::Replaced` (and closes the old
  connection), for which `end_session` neither raises the revision nor emits the agent notice, and the
  `SessionStarted` of the same commit raises it once and emits one agent notice (5.5; c30). Activity never raises it
  (D35): the `Agent` view joins `session.last_active_at` and `last_tool_call_at` from memory, and a call sends no
  notice. So an active agent cannot starve `pause_agent{expected_revision}`; a `conflict` carries
  `current_revision`, which the page may use at once. The two activity fields are live, outside the cursor: two
  snapshots at one cursor may differ in them (CCR-17, R23).
- Profiles revision (D20): every `save_profile` commit applies exactly one `ProfileStateSet` with the active profile
  id and version after the command and `revision + 1`; `ProfileVersionAdded` never touches it.
- `WriteTx::notify(change)` assigns `meta.notice_sequence + 1` inside the transaction; sequences never repeat across
  restarts; a refused write takes none. `get_snapshot.cursor = meta.notice_sequence`.
- `commit_sequence` is the last notice of the command: `decide_request` the request notice (grant notice first);
  `revoke_grant` the grant notice; `pause_agent` the last agent notice; `save_profile` the profiles notice;
  `save_preferences` the preferences notice; `save_group` the set or set-deleted notice (grant notices first);
  `disconnect_google` the account notice, emitted after its grant notices and its `request` notices
  (`pending_approval -> stale`).
- After commit, notices enter the ring (last 256 of this run); they are written to stdout only while subscribed. The
  service's own writes (pump, agent, proofs, attempts, seed, quit) are published by the facade after each phase; a
  command's notices go back to the owner channel, which writes them before its response.
- Ring base: at every `Core::open` the ring empties, the subscription ends, and the base becomes
  `meta.notice_sequence` read before the recovery transaction; the recovery's and the first pump's notices are rung.
  `subscribe(cursor)`: `cursor > notice_sequence` -> `snapshot_required` error; `cursor < base` or the ring no longer
  holding every notice after `cursor` -> `snapshot_required` error; otherwise answer `{cursor}` (the requested one)
  and write the ring's notices after it. A page that took a snapshot right after a restart therefore subscribes
  without a loop. A second `subscribe` replaces the first; a refused one leaves an existing subscription as it was.
  The first successful `subscribe` of the process sets `Broker::subscribed()` (5.10).
- One step producing more than 256 notices while subscribed: write one `snapshot_required` change whose `sequence`
  is the first unsent notice's, end the subscription (CCR-4).
- A notice over the frame bound is never written: it becomes one `snapshot_required` change with its sequence, ends
  the subscription, and the later notices of the step are dropped (D38).

### 5.5 Sessions, activity and grant terms (PLAN 7; CR 10; ADR108 D4)

| Term | Bound to | Valid at use iff | Ends with |
|---|---|---|---|
| `one_request` | action digest; origin request | not ended; used only by its origin request, which is not terminal | `consumed` in the commit that makes the origin request terminal; `revoked_by_owner`, `account_disconnected`, `authority_reduced` |
| `session` | session id | not ended; the session is live in memory for the current run; `now - last_active <= 1_800_000` | `session_disconnected`, `session_idle_timeout`, `app_quit`, `broker_restart`, and the reasons above |
| `until_revoked` | nothing (`expires_at: null`) | not ended | `revoked_by_owner`, `account_disconnected`, `authority_reduced`, `agent_removed` |

- Validity is a predicate checked wherever a grant is used (facts, proof acceptance, dispatch); sweeps only make
  endings durable. A session grant from an earlier run is invalid before the recovery commits, because no session of
  an earlier run is in memory.
- A one-request grant stays live until its origin request is terminal, also while `applied` or `outcome_unknown`
  (D18; the desktop README says so, R19). Its `used` fact (L6) is true once the change it permits was applied (the
  origin request reached `applied` or `reconciled`); for a read it will be true once the result is released
  (CD-W05/W06).
- `AgentInput::Connect` for a registered agent starts a session in the current run (`SessionStarted`, agent revision
  + 1, agent notice; idempotency none) and sets its in-memory `last_active_ms` to `started_at_ms`. An agent's live
  session is ended first in the same commit (`SessionEnded{Replaced}`; its session grants end `session_disconnected`
  with their grant notices); `Replaced` raises no agent revision and emits no agent notice, so the commit raises the
  revision once (5.4). Unknown agent: connection closed, `unknown_agent` audit row, nothing else stored.
  `AgentInput::Disconnect` ends the connection's session only when it is still the agent's live one.
- A call on a connection with no live session of its agent is answered `not_permitted` and audited (`not_permitted`).
- Activity (D35): every other `AgentInput::Call`, `fabric_request_status` included, sets the session's
  `last_active_ms` and the agent's `last_tool_call_ms` in memory. Nothing is written and no revision changes. A status
  poll therefore keeps a session alive (R20); a poll of a request of another agent is `not_found`. A session's
  `last_active_ms` starts at its `started_at_ms` (connect), so the contract's required `Session.last_active_at`
  always has a value and a session with no call idles 1 800 000 ms after its connect. Only the agent's
  `last_tool_call_ms` (contract `Agent.last_tool_call_at`, nullable) is absent until the agent's first call in the
  run; no session survives a restart (5.8). The idle deadline of each live session (`last_active_ms + 1_800_000`) is
  part of `Broker::next_due_ms`; at the deadline one commit (`timer.session_idle`) ends the session
  (`SessionEnded{IdleTimeout}`), its grants, and raises the agent revision. 1 800 000 ms =
  `Limits::CD_G0.session_idle_timeout_seconds`, kept equal by hand to the theory's `session_idle_timeout_ms`
  (THEORY note B).
- Grant fields from the approval: `purpose` = the action's `task`; `tools` = `[action.tool()]`; `scope` = the card
  scope; `term`; `origin` = `{request_id, approval_digest}`; `policy` = active profile id and version;
  `revocation_epoch` = counter at creation (CCR-5); `revision` 1; `ended` null. A `session` duration binds the agent's
  current live session, which may be a later session than the one the request came from (PLAN 7 "bind to the
  registered agent session and current broker run").

### 5.6 Epochs and the fence

- `pause_epoch` +1 in every `pause_agent` commit that sets `paused = true` on at least one agent. Resume changes no
  epoch and revives nothing.
- `revocation_epoch` +1 for each grant a transaction ends (any reason, including session ends and consumption), and
  once in every transaction that activates a profile, saves a version of the active custom profile, saves or deletes
  a recipient set or disconnects an account when it ends no grant, or changes an account's state or calendars.
- `Fence { pause_epoch, revocation_epoch, event_revision }` is read with the facts of each proof (one read
  transaction) and stored with the proof and the authorization. A proof result is accepted only when the current
  fence equals it (PLAN 8 step 5); otherwise it is recorded, discarded and the proof re-run (5.7, D16).
- At dispatch (PLAN 8 step 7) the authorization must be `issued`, of the current run and unexpired at the current
  clock (PLAN 8 para 1), the current fence must equal the authorization's, every grant in the authorization's
  `grant_ids` must be valid (L6: a request whose relied grant is no longer live never dispatches, even if another
  grant covers it), and the agent must not be paused. The checks run in that order; the first that fails names the
  void reason (`broker_restart`, `expired`, `fence_changed`, `grant_invalid`, `agent_paused`). Because the owner
  paths that end a grant or pause an agent also raise an epoch, `fence_changed` fires first for them;
  `grant_invalid` and `agent_paused` are reached only without an epoch raise (unit tests in 9.1).
- Recipient sets: narrowing applies at once; adding a member or domain, or deleting the set, ends live grants whose
  scope names it (`authority_reduced`; CCR-9).

### 5.7 Reducer and pipeline

One function moves a request: checks `RequestState::can_transition_to`, builds the `LifecycleStatus` payload, raises
the revision, writes `RequestMoved`, emits the request notice, sets `due_at_ms`. A move to a terminal state first
ends the request's own live one-request grants `consumed` (grant notices before the request notice) and clears its
due time. A pump runs before and after every input and on every `Tick`: it takes due requests in order (`due_at_ms`,
then `received_at_ms`, then id) and performs one step per commit (timers, the pipeline after a command, recovery).

Request scope and digest: `RequestRow.scope` is derived from the action (the calendars of its account; the window for
reads, the fields for `calendar_read`, `Exact(attendees)` for an invitation with attendees, `send_updates` where the
tool has it; `None` for `calendar_list` and `request_result`). `request_digest` is the contract's
`request_digest(agent, action, scope)`, or the `request` domain digest of `{"agent_id", "action", "scope": null}` when
the scope is `None`.

Relied grants (D36; L7). Each request keeps a list `relied_grant_ids`:
- set by `ReliedGrantSet` in the commit that moves it to `allowed` (the classification's `authorizing_grants`, which
  may be empty) or `approved` (exactly `[the grant the approval created]`);
- replaced by `ReliedGrantSet` with the `authorizing_grants` of every accepted fresh proof, whatever its outcome, in
  the commit that handles it;
- unchanged by every result that is not accepted, whatever the reason (a report bound to another job, a changed
  fence, an authorizing grant invalid at acceptance, an id outside the job's facts, a late or unknown job), and by a
  void at dispatch. L7's "the request's previous check" is read as the previous accepted check (11.3 W1).
A classification job's `TrustedFacts.relied` is empty. A fresh-permit job's `relied` is the stored list, and
`TrustedFacts.grants` holds each relied grant even if it has ended (marked `ended`), so the theory can block on it.
`set_relied` itself does not re-check the ids: the store's `apply` guard (every id a grant of the request's agent)
decides, and callers pass only ids that passed the grant check below, or the grant an approval just created.

Re-proof counters (D16; L2): `reproofs_classification` and `reproofs_fresh_permit` count discarded results (fence
changed, or for a fresh permit an authorizing grant invalid at acceptance) and voids at dispatch. They are never
reset, by a retry or otherwise. When a counter is at `MAX_REPROOFS` and another re-proof would be needed:
classification -> `undecided`, fresh permit -> `stale` (the pump moves `proving_fresh_permit -> stale` without
starting a proof).

Prover kind: always `Attachments.prover.kind` fixed at start (D13); a `ProofDone` carries no kind.

Handling a result, in order:
1. A result for an unknown or expired job id, one whose `request_id` or `purpose` differs from its job's, or one for
   a request no longer in the job's proving state is ignored and not recorded.
2. Binding (D39): a report whose `request_id`, `purpose` or `action_digest` is not its job's proves nothing. The
   request goes `undecided` in one commit (`request.proof`); nothing is recorded and the relied list is unchanged, so
   no proof is filed under the request the report names.
3. Grant check (D17): each id in `ProofDone.authorizing_grants` must be among the grant ids of the job's
   `facts.grants`, kept with the in-flight job (5.1). If one is not (a wrong adapter, a grant of another agent, an
   unknown id), the result is refused in one commit (`request.proof`): `ProofRecorded` (the report and list as
   received, `undecided_reason` "authorizing grant outside the facts"), no `ReliedGrantSet`, and the request goes
   `undecided`, for every purpose and verdict. It is not a re-proof. So a prover result never becomes a
   `StoreFault`, and a restart cannot replay a halt (c18). Grant ids are unique within one context: the offered
   grants come from one list of the agent's grants.
4. No report: `undecided`, nothing recorded.
5. Fence changed, or a fresh Permit naming a grant no longer valid now (5.5): recorded with the job's fence and
   discarded; proved again while the purpose's counter allows.
6. Otherwise accepted: recorded, and the verdict decides (table below).

| From -> to | Driver in W04 |
|---|---|
| (new) -> received | `agent.call` after limits pass (5.9); the only transaction with `agent_call` idempotency. A repeated `(agent_id, call_id)` returns the existing request and writes nothing (different action: `invalid_params`) |
| received -> validating -> proving_classification | pump. World checks: account known and connected, every calendar known and selected, a window starts before it ends. Zones: CD-W09 |
| validating -> terminal_failure (`invalid_arguments`) | pump, when a world check fails or the call is `request_result` (no result is held, D33) |
| proving_classification -> allowed | accepted Permit, prover kind `Isabelle`: `ProofRecorded`, `ReliedGrantSet(authorizing_grants)`, move, one commit (`request.proof`) |
| proving_classification -> pending_approval / blocked | accepted NeedApproval / Block with a `Ruling`. `pending_approval` opens a 24 h card showing the request's scope with proposed duration `one_request`; a request with no scope cannot show a card and goes `undecided` |
| proving_classification -> undecided | no prover attached (at once, no report); watchdog 35 s (late result ignored by job id); an Undecided verdict; a missing report; Block or NeedApproval without a `Ruling`; NeedApproval for a request with no scope; a Permit from a non-Isabelle prover; a report bound to another job; an `authorizing_grants` id outside the job's facts (any verdict); the classification counter exhausted |
| pending_approval -> approved / denied | `decide_request`; approve: `create_grant`, `ApprovalRecorded` (while still pending), `ReliedGrantSet([grant])`, move, one commit; the pump of the same step then starts the fresh proof |
| pending_approval -> expired | pump at `card_expires_at_ms` (opened + 86 400 000) |
| pending_approval -> stale | `disconnect_google` of its account; W05 for provider changes |
| approved / allowed -> proving_fresh_permit | pump; the job's `relied` is the stored list |
| proving_fresh_permit -> queued | accepted Permit, every id in `authorizing_grants` valid now (5.5), prover kind `Isabelle`, provider present, `authorizing_grants` non-empty, agent not paused, the tool changes a calendar: one commit (`request.authorize`) with `ProofRecorded`, `ReliedGrantSet(authorizing_grants)`, `AuthorizationIssued` (60 s; `grant_id` = `authorizing_grants[0]`, `grant_ids` = the whole list; a new attempt number; L7(c)), `EffectReserved` (first time; `provider_event_id` set for creates and only for creates, an `apply` guard), move |
| proving_fresh_permit -> undecided | Permit with an empty `authorizing_grants` (L7(d)), no provider, a non-Isabelle prover, a paused agent, or a tool that changes no calendar (D17, D33); a report bound to another job; an `authorizing_grants` id outside the job's facts (any verdict); an Undecided verdict; watchdog; a missing report; Block or NeedApproval without a `Ruling` |
| proving_fresh_permit -> blocked | Block with a `Ruling` (a relied grant that is no longer live ends here: the theory blocks, L6) |
| proving_fresh_permit -> pending_approval | NeedApproval with a `Ruling`: a new 24 h card. The open-card limit is not checked here; a re-ask may exceed five open cards (5.9, R18) |
| proving_fresh_permit -> stale | the fresh-permit counter exhausted |
| queued -> dispatching | pump: one commit (`request.dispatch`) when every dispatch check of 5.6 holds: `AuthorizationConsumed`, `DispatchIntent` (attempt row, number `attempts_started + 1`), move. Only after commit: `Job::Dispatch` (provider deadline 30 s). The handoff is the linearization point (PLAN 8) |
| queued -> proving_fresh_permit | a dispatch check failed: `AuthorizationVoided{BrokerRestart, Expired, FenceChanged, GrantInvalid or AgentPaused}` + move, one commit; the fresh-permit counter + 1. The fresh proof then sees the change (a paused agent or an ended grant blocks). A `queued` request with no issued authorization also goes back to a fresh proof |
| dispatching -> applied | `Input::Attempt` Applied (`request.attempt`) |
| dispatching -> retry_wait | `Input::Attempt` Transient, decided here: `backoff = max(RetryPolicy.backoff_ms[retries], retry_after_ms or 0)`; `retries` counts the retries already made for this effect, `effects.attempts_started - 1` (0 when the first attempt reports Transient); `retry_wait` with `due = now + backoff` only if `retries < RetryPolicy.max_retries` (3) and `now + backoff <= first_attempt_at_ms + 120_000`. So at most four attempts: Transient on attempts 1, 2 and 3 waits 2, 4 and 8 s; Transient on attempt 4 ends `terminal_failure` |
| dispatching -> terminal_failure | `Input::Attempt` Rejected (its code), or Transient past the rule above (`retry_budget_exhausted`) |
| dispatching -> outcome_unknown | `Input::Attempt` NoAnswer; startup recovery |
| retry_wait -> proving_fresh_permit | pump at `due`; fresh proof (relied list kept) and a new authorization |
| outcome_unknown -> reconciling -> ...; applied -> reconciled | CD-W05; in W04 these stay open and visible |
| any -> cancelled | not driven (no cancel message in v1) |

The reducer drives 25 of the contract's 39 transitions; the rest belong to later packages. An attempt result for
another attempt than the open one is ignored.

Audit before effect: no `Job::Dispatch` leaves unless the commit holding `AuthorizationConsumed` + `DispatchIntent`
returned; no command answer before its commit; no full-detail read answer before its audit row (PLAN 11 para 5;
PLAN 14 "block release if local audit can fail open").

### 5.8 Recovery, reopen and quit

Recovery, in every `Core::open` (start and reopen; one transaction `broker.started`, idempotency `startup`/run id):
`RunStarted` with run id `meta.ledger_head_seq + 1` read before recovery, so run ids rise (a previous run without
`RunEnded` ended uncleanly); end every live session and its session grants (`broker_restart`); void every `issued`
authorization (`broker_restart`); `queued -> proving_fresh_permit`; `dispatching -> outcome_unknown` (open attempt
finished `outcome_unknown`); `due_at_ms = now` for `received`, `validating`, `proving_*`, `allowed`, `approved`;
`retry_wait` and `pending_approval` keep their due times (expired cards expire in the first pump). Untouched:
until-revoked grants, ended grants, epochs, consumed authorizations, approvals, proofs, relied lists, re-proof
counters, commands (RM 242). A restart with nothing open sends no notice. Then the first pump. Only then does a
reopen leave the unavailable mode and answer the frame that triggered it (c27). The in-memory activity, jobs,
connections and ring start empty.

Quit (end of stdin): one transaction `broker.quit`: end live sessions and session grants (`app_quit`), write the open
`busy` windows' audit rows, `RunEnded{clean: true}`. No new job after quit begins; `dispatching` effects are left for
the next start. One fsync'd commit (about 5 ms, SPIKE 2) fits the 1 s stop bound (`STOP_BOUND`,
`quit_stops_the_broker_without_a_kill`).

### 5.9 Request limits (CR 8; ADR108 D9; `Limits::CD_G0`)

Checked only in the `agent.call` transaction that would insert a request, before inserting (L2): open requests 100
total (checked first) and 25 per agent (`terminal = 0`); 5 `pending_approval` per agent
(`open_approval_cards_per_agent`); 15 received per agent in the last 60 s (D31). Past one: `busy{limit, maximum,
retry_after_ms}` (for the rate, the time until the oldest call leaves the window, else 5000), counted into the
agent's `busy` audit window (section 4), no request, no ledger row. The contract says no request is created past a
limit; a later transition of an existing request is not refused, so `proving_fresh_permit -> pending_approval` may
make a sixth open card (R18).

### 5.10 Synthetic agent (debug builds, test key, scratch directory)

`apps/fabric-broker/src/synthetic.rs`, compiled only with `debug_assertions`, active only with
`FABRIC_BROKER_SYNTHETIC_AGENT=1` and `FABRIC_BROKER_KEY_SOURCE=test` (3.3). It does nothing until
`Broker::subscribed()` is true (the run's first successful `subscribe`). Then: `Broker::seed` (debug builds; one
`broker.seed` commit, idempotent) creates agent `synthetic-agent` (`Local{endpoint: "in-process"}`, name "Synthetic
agent (test build)") and account `synthetic-account` (`connected`, one selected calendar `synthetic-calendar`); it
connects through `AgentInput::Connect` on `SyntheticAgent::CONNECTION`; 1 s later it proposes a `propose_create` on
that calendar (one hour, on the hour, seven days ahead, zone `Etc/UTC`, title `synthetic::TITLE`, a marker the tests
look for). Call and poll ids are fresh ids, so a later run's proposals never meet `UNIQUE(agent_id, call_id)`. It
learns the request id from the reply (main hands it every `AgentReply` on its connection, `take_replies`), polls
`fabric_request_status` once a second while that request is open, and proposes again 1 s after it is terminal, at
most 2 proposals per run. If the broker closes its connection it stops for the run.

A `SyntheticClassifier` is the attached prover (`Attachments.prover = Some({kind: Synthetic, health:
Unavailable})`), run on a worker thread: classification -> `need_approval` with rule `synthetic_no_rule_theory`,
`isabelle_version` "synthetic (no prover)", `authorizing_grants` empty; fresh permit -> `undecided` with an
`undecided_reason`. Its result type cannot express permit, and the core refuses any non-Isabelle permit. Health stays
`prover: unavailable`. `hello`'s build label is `"<version> CD-W04 synthetic"`. Its reports are stored but
`get_proof` answers `not_found` for them (6.1).

---------------------------------------------------------------------------------------------------------------------

## 6. Owner channel

### 6.1 The 13 methods

| Method | CD-W04 |
|---|---|
| `hello` | name `fabric-broker`, build `Settings.build` (`"<version> CD-W04"`, + ` synthetic` in a synthetic run), limits `Limits::CD_G0`; answers in the unavailable mode |
| `get_snapshot` | One read: cursor, health `{prover: attached prover's health or unavailable, store: ready}`, profiles (built-ins + latest custom versions, active, revision), accounts, agents (live session joined, its `last_active_at` from memory, which starts at the session's start; `last_tool_call_at` from memory, null until the agent's first call in this run), recipient sets (not deleted), preferences, open requests, live grants (a one-request grant of an `applied` or `outcome_unknown` request is among them, D18). An answer over the frame bound is `internal` (D38) |
| `connect_google` | `unknown_method` (5.3) |
| `save_profile` | `activate`: unknown id `not_found`; `ProfileStateSet` revision + 1; `revocation_epoch` + 1; no grant ended. `save_custom`: a name that reads as Careful or Flexible `invalid_params` `/params/change/profile/name` (compared after dropping invisible characters, trimming and collapsing whitespace, in any case); unknown id `not_found`; expansion = rows wider (`Mode::is_wider_than`) than the previous version, or Careful for a new profile; non-empty and `acknowledged_expansion != expansion_digest` -> `expansion_not_acknowledged{expansion, expansion_digest}`; past the budget (D38) `invalid_params` `/params/change/profile`; else new immutable version (new id `custom-` + 16 hex), `ProfileVersionAdded`, then `ProfileStateSet` (the active id and version after the command, revision + 1), `revocation_epoch` + 1 if active |
| `save_preferences` | revision check; store; `invalid_params` at `/params/settings/warning_calendars/<index>` when an entry names no stored calendar; zones by pattern only (CD-W09) |
| `save_group` | create: id null and `expected_revision` 0, new id, source local; update: `not_found`/`conflict`, new version; delete: `not_found`/`conflict`, `recipient_set_deleted`, result `null`; past the budget `invalid_params` `/params/change/definition`; grants per 5.6; `revocation_epoch` + 1 |
| `decide_request` | `not_found`; state not `pending_approval`, revision or `request_digest` mismatch -> `stale_request`; `selected` non-null -> `invalid_params` `/params/selected` (acceptable for W04 only with CCR-7 and before CD-W08); `session` duration with no live session of the request's agent -> `invalid_params` `/params/duration`; an approval past the budget `invalid_params` `/params/decision`. Approve: `Approval::Approve` (`approval_digest(request_id, request_digest, Decision::Approve{scope, duration})`, `effective_request_digest` = request digest), grant (5.5; a `session` grant binds the agent's current live session), `ReliedGrantSet([grant])`, grant notice, request -> `approved`. Deny: `Approval::Deny`, -> `denied` |
| `pause_agent` | one agent: `not_found`/`conflict`; all agents: every agent, none -> `not_found`; each targeted agent gets its own `AgentPut` (paused, revision + 1) and a notice even when unchanged; pause raises `pause_epoch`. Activity never raises an agent revision (5.4) |
| `revoke_grant` | `not_found`; revision mismatch or already ended -> `conflict{current_revision}`; else ended `revoked_by_owner`, revision + 1, `revocation_epoch` + 1; committed before the answer; never waits for a proof. A call already handed off stays `dispatching` with its open attempt visible (W08 says it may still reach Google) |
| `get_request_details` | request, card, approval, `proof_ids` (distinct ids of the request's Isabelle reports, newest first, at most 200), latest authorization, attempts (at most 20), provider event (null); `full` adds `private{action, before: null, warnings: []}` after its audit row commits; `not_found` |
| `get_proof` | the newest `proof_reports` row with that id; `not_found` when there is none or when its `prover_kind` is `synthetic` (CC `owner.rs`: a proof report is one live Isabelle check; L2; CCR-10) |
| `subscribe` | 5.4 |
| `disconnect_google` | `not_found`/`conflict`; its live grants end `account_disconnected` (grant notices); its `pending_approval` requests -> `stale` (request notices); account -> `disconnected`, revision + 1, account notice last (`commit_sequence`); `revocation_epoch` + 1; `ended_grant_ids`. An already disconnected account still succeeds (revision + 1). `revoke_provider_token: true` revokes nothing (no token exists before CD-W05) |

The budget of D38 is 720 KiB of canonical JSON over the recipient sets, the custom profiles and the live grants;
one recipient set at the schema maximum (about 660 KB) fits. Accounts (CD-W05) and agents are not budgeted, and the
open requests (at most 100) are assumed to fit in the rest of the frame (R24).

`refusal_answer`, `error_frame`, `result_frame`, `handle_oversized` and the `hello` checks moved from BR `spike.rs`
to `owner/frames.rs` with their tests.

### 6.2 Contract change requests (not made here, L5; for the contract owner)

- CCR-1 An error code for "method not implemented in this build". Today `connect_google` answers `unknown_method`,
  which CR 5 reserves for a method the channel does not have.
- CCR-2 `health.store` / `StoreReason` values for in use, newer schema, integrity failed, key missing, wrong key,
  key-kind mismatch, not encrypted, Keychain failure, random-source failure at key generation, and a passing I/O
  failure (all `locked` today); what a locked snapshot means.
- CCR-3 `commit_sequence` for a command that changes nothing (`pause_agent all_agents` with no agents answers
  `not_found` today).
- CCR-4 Whether a `snapshot_required` notice consumes a sequence (today: the first unsent sequence is reused).
- CCR-5 Meaning of `grant.revocation_epoch` (proposal: the counter at creation).
- CCR-6 `revoke_grant` on an ended grant (today `conflict`).
- CCR-7 `decide_request`: `conflict` vs `stale_request` on revision mismatch; code when no longer pending; code for a
  `session` duration with no live session (today `invalid_params`); `selected` narrowing, which CR 7 allows and W04
  refuses with `invalid_params` until CD-W08.
- CCR-8 End reason of a one-request grant whose request ended without use (today `consumed`).
- CCR-9 End reason for "a recipient set the grant names was widened or deleted" (today `authority_reduced`).
- CCR-10 A prover-kind marker on `proof_report`, so a synthetic report can never pass for Isabelle (today
  `get_proof` hides synthetic reports).
- CCR-11 The missing commands of CR 15 / RM 254: cancel, select calendars and Agent Drafts, approve/remove enrollment,
  resolve unknown outcome, People import, list ended requests.
- CCR-12 `subscribe` result cursor (requested vs current); behaviour of a second `subscribe`; the accepted cursor
  range after a restart (`base <= cursor <= notice_sequence`).
- CCR-13 A JSON Pointer in `Refusal`; the bounded line reader in the contract crate.
- CCR-14 The formal context and digest for broker-initiated actions (CR 15; THEORY 4 item 9).
- CCR-15 `Authorization.grant_ids` (a list) in place of the single `grant_id`: a Permit can rest on several grants
  (L7(c)). Today the first id is recorded and the store keeps the list.
- CCR-16 Whether `Agent.last_tool_call_at` may reset to null at a restart (D35; today it is null until the agent's
  first call in the run).
- CCR-17 `session.last_active_at` and `agent.last_tool_call_at` are live fields outside the cursor: they change in
  memory with no commit and no notice (D35), so two snapshots at one cursor may differ in them and a subscriber sees
  them only in its next snapshot. This departs from CR section 7 ("the state at a cursor"). The contract should say
  so, or give them a notice.
- CCR-18 An owner-side limit code. Today a command that would grow the owner's records past their budget is refused
  `invalid_params` (D38), and an answer too long for one frame is refused `internal` with its id and method. Both
  are limits of the owner channel, not wrong parameters or broker faults.

---------------------------------------------------------------------------------------------------------------------

## 7. Audit before effect and gate G2a

### 7.1 Rule

Nothing reaches a provider, an agent result, or an owner success unless the transaction recording it committed
(5.2, 5.7). Any failed write halts the run and drops the step's jobs. There is no path that sends first and records
after.

### 7.2 Fault injection (no product hooks)

- Failing write: the test opens a second connection with the test key and creates
  `CREATE TRIGGER fault BEFORE INSERT ON ledger WHEN NEW.operation_type = '<type>' BEGIN SELECT RAISE(ABORT,
  'injected'); END;` The broker's next commit of that type fails and rolls back; dropping the trigger ends the fault.
  A `RAISE(ROLLBACK)` trigger on a projection table ends the transaction under the writer (the poisoned `WriteTx`,
  section 4).
- Locked database: the second connection holds `BEGIN IMMEDIATE`; the broker's write gets `SQLITE_BUSY` after 250 ms.
  At start: hold `broker.lock` or start a second store on the directory.
- Kill inside a transaction: `crates/fabric-storage/tests/crash.rs` (`harness = false`) re-executes itself; the child
  starts a write, prints `STALLED`, sleeps (or calls `std::process::abort()`); the parent SIGKILLs / waits and
  reopens: no trace of the write, chain verifies. After a kill the reopen retries `InUse` for up to 2 s, because the
  operating system may release the dead process's lock a moment later (Windows).
- A store left without tables (a crash between `Connection::open` and the first migration's commit): built directly
  (a raw keyed connection that creates `broker.db` and closes; a first open whose migration list fails inside
  migration 1), then a clean open.
- Kill after a commit: drop `Broker` without `quit` and start a new one on the same directory (equivalent, 5.1);
  process tests add real SIGKILLs and wait up to 2 s for the lock before the next start.
- Passing I/O at open: the store file made unreadable (`chmod 000`, Unix) and readable again.
- Clock steps: tests pass `now_ms` values that go backward mid-run.

### 7.3 G2a evidence (PLAN 13; RM 242; PLAN 14 store-level rows; ADR108 D10)

| Requirement | Tests (section 9) |
|---|---|
| Fail audit persistence or lock the database | c12, c13, c14 (both tests), s12, s06, p02; the poisoned write (9.1 `tx.rs`) |
| Expire grants; restart (also a reopen) | c01-c06, c27, c27b, c29 (the authorization half in 9.1 `service/tests.rs`), p04 |
| Revoke/pause during proof, queued, before retry | c07-c10 (a void while `queued` is reachable only in-process: the three void reasons are unit tests in 9.1 `service/tests.rs`), c28 |
| Approve, then alter | c11 |
| Accepted insert, response lost, broker killed (store half) | c15, c30 |
| Double-click, dropped notice, cursor gap | c16, c17, p05 |
| Upgrade encrypted fixture database | s07, s08, s09 |
| Kill Isabelle / budget (store half) | c18 (including the report bound to another job), c26, c31 |
| Account A vs B (store half) | c21 |
| Unregistered agent (store half) | c22 |
| DEF-1 (one writer) | s06, p02 (no second writer), c24 (sequential load, gap-free sequences), s11, s15, s16 |
| DEF-2 (configuration first) | p01; the configuration unit tests of 9.1 |
| DEF-4 (used authorizations survive) | c05, c27 |
| DEF-7 (the failing rule stays shown) | c19, c19b |
| PLAN 5 R08 (pause and revoke before the next dispatch) | c07-c10 |
| PLAN 5 R09 (one correlated lifecycle) | c24 (ledger correlation) |
| PLAN 5 R12 (no blind duplicates after restarts) | s12, s15, c12-c15 |
| PLAN 5 R13 (secrets and content protected locally) | s02, s03, s17, p08 |
| Frame bound (CR section 1) | the two frame tests of c20's file (`limits.rs`), 9.1 `owner/subscribe.rs` |
| Not provable in W04 | poll a completed read after revocation (no result storage, D33): moved to CD-W05/W06; accepted for this package (L3) and open for the owner (Q10) |

Results as they stand (macOS, Apple silicon, 2026-11-16, debug profile):
- `sh scripts/run.sh cargo test -p fabric-storage -p fabric-broker-core -p fabric-broker --locked`: 329 passed,
  0 failed; 1 ignored (the Keychain test, M1).
- `sh scripts/verify.sh`: passed.
- `sh scripts/verify-desktop.sh`: passed (host tests; release check of the broker and its configuration tests in
  release: 17 passed).
- `sh scripts/verify-desktop.sh --packaged`: passed on a packaged debug build of an earlier state of this
  package; not run against the final code.
- Not run: Linux and Windows builds and tests (CI runners); the release-profile run of the acceptance tests (they are
  `cfg(debug_assertions)`); the Keychain test.

Manual checks by the owner: M1 `cargo test -p fabric-storage --test keychain -- --ignored`; M2 a packaged debug build
launched twice across a rebuild (record the Keychain prompt); M3 Keychain denial shows the `keychain_refused` message
and a relaunch after allowing works. Also open for the owner: ADR-115 (RD-14) and acceptance by the
other-model-family reviewer (PLAN 15.1 CD-W00d).

---------------------------------------------------------------------------------------------------------------------

## 8. Cross-platform and CI

- CI: `cargo check/clippy/test --workspace --all-targets --all-features --locked` on macos-26, ubuntu-24.04,
  windows-2025-vs2026. No Cargo features are added.
- `cfg(target_os = "macos")`: SQLCipher, key pragma, `cipher_log_level`, cipher assertions, the plaintext-header
  check, `keychain` module and `KeychainKey`, `security-framework`, encryption tests s02/s03, the Keychain test.
- `cfg(debug_assertions)` (in `apps/fabric-broker`, `fabric-broker-core` and the host): key source `test`, the
  synthetic agent, `Broker::seed`, `Seed`, `FABRIC_DESKTOP_DATA_DIR`, test key forwarding, and the core's acceptance
  tests. Every CI build is a debug build, so `scripts/verify-desktop.sh` adds, on macOS, `cargo check --release
  --locked -p fabric-broker` (compile only) and `cargo test --release --locked -p fabric-broker --lib` (the release
  configuration refuses the test key and the synthetic agent; parsing only, no store, no Keychain). CD-W11 tests the
  shipped release broker.
- `cfg(unix)`: modes 0700/0600 and the data-directory mode check; the unreadable-file tests. No owner-uid check.
- Everything else (schema, migrations, ledger, replay, reducer, owner channel, crash harness, process tests) runs on
  all three runners. Process tests use `FABRIC_BROKER_KEY_SOURCE=test` everywhere (SQLCipher with the test key on
  macOS, plain SQLite elsewhere) and never start a macOS broker with `keychain`.
- `OPENSSL_DIR`, `OPENSSL_LIB_DIR`, `OPENSSL_INCLUDE_DIR` silently switch SQLCipher to OpenSSL. `scripts/build-desktop.sh`
  and `scripts/verify-desktop.sh` unset them before building the broker; CI never sets them; the `cipher_provider`
  assertion catches any other path (exit 78).
- Cost: one SQLite amalgamation compile per cold runner (about 16 s); `sqlcipher-src` (13 MB) fetched on every runner.
- Unverified: Linux/Windows compile of the target split; MSVC with `-U` in `LIBSQLITE3_FLAGS` (low risk: the build
  script passes the tokens unchanged and `cl.exe` accepts `-` options). Fallback if MSVC refuses:
  `-DSQLITE_TEMP_STORE=3` alone on non-macOS.
- `#[allow(clippy::large_enum_variant, reason = "...")]` on `Mutation`, `Input`, `Job`, `ports::AgentInput` and the
  facade's private state; proof results travel boxed between threads in the executable.
- The Tauri sub-workspace does not link the store; the broker is built from the root workspace, so `[env]` applies.

---------------------------------------------------------------------------------------------------------------------

## 9. Test plan

No test touches the Keychain or the network unless `#[ignore]`d; nobody runs an ignored test except the owner (M1).
Temporary directories are made by hand under `std::env::temp_dir()` (pid + counter), removed on `Drop`; the data
directory a test passes is a not-yet-existing child of its temporary directory, so the store creates it with mode
0700. Every core acceptance test ends with `verify_chain()` and `verify_replay()`. The tests start at
`T0 = 1_795_190_400_000` (2026-11-20T16:00:00.000Z). The ids (s, c, p) below are the ones the code's comments use; a
test whose id is shared proves part of the row named.

### 9.1 Unit tests (in source files)

Each name states what the test proves.

`crates/fabric-storage/src/`:
- `apply.rs` (the guards of 3.5): `a_request_moves_only_along_the_lifecycle`, `a_revision_never_skips`,
  `an_ended_grant_never_changes`, `an_authorization_is_consumed_once`, `an_expired_authorization_is_never_consumed`,
  `an_authorization_of_another_run_is_never_consumed`, `a_voided_authorization_is_never_consumed_or_voided_again`,
  `an_authorization_is_issued_in_the_current_run_on_its_agents_grants`,
  `a_dispatch_intent_needs_the_consumption_of_its_authorization`, `a_counter_never_decreases`,
  `a_relied_grant_belongs_to_the_requests_agent`, `only_a_create_reserves_its_event_id_and_always`,
  `a_profile_version_never_touches_the_profiles_revision`, `a_session_starts_live_in_the_current_run_and_ends_once`,
  `re_proof_counts_never_decrease`, `a_pending_request_carries_its_card_and_its_decision`,
  `a_grant_belongs_to_its_origin_requests_agent`, `recipient_sets_rise_by_one_and_stay_deleted`,
  `replay_rebuilds_the_projection_and_finds_a_change_made_beside_it`,
  `a_corrupted_column_is_a_decode_fault_without_its_content`.
- `key.rs` (3.2): `a_generated_key_is_new_each_time`, `the_sql_literal_is_the_raw_key_in_lowercase_hex`,
  `a_key_from_a_slice_takes_exactly_32_bytes`, `the_test_key_is_the_public_constant`,
  `each_key_error_maps_to_its_unavailable_reason`, `a_random_source_failure_has_its_own_message_and_is_retried`,
  `key_kinds_have_their_stored_names`.
- `keychain.rs` (3.2 status mapping, no Keychain call): `a_denial_is_refused_and_never_retried`,
  `a_locked_keychain_and_any_other_status_are_retried`.
- `ledger.rs` (section 4): `the_envelope_hash_covers_the_canonical_envelope` (hash vectors),
  `a_row_without_actor_agent_idempotency_or_run_hashes_nulls`, `operation_ids_are_op_and_32_lowercase_hex`,
  `verify_chain_accepts_equal_times_and_refuses_a_time_that_goes_back`, `a_changed_ledger_row_fails_the_chain`,
  `a_truncated_tail_fails_the_head_check`, `ledger_rows_cannot_be_changed_or_deleted`,
  `a_row_of_an_unknown_type_fails_the_chain`.
- `lib.rs` (D37): `faults_and_errors_print_no_key_and_no_content`, `a_wrong_key_error_prints_neither_key` (macOS),
  `a_failed_backup_prints_no_key`.
- `read.rs`: `next_due_is_the_earliest_due_request`, `proof_ids_are_distinct_newest_first_and_of_one_kind`,
  `an_open_effect_names_the_event_it_targets`, `counts_and_lookups_follow_the_requests`.
- `store.rs` (3.1-3.4, 3.6): `the_read_write_connection_holds_every_pragma`,
  `a_new_store_is_created_at_the_newest_schema_with_one_ledger_row`, `a_store_reopens_with_its_state`,
  `the_directory_and_files_are_private` (Unix), `a_data_directory_open_to_others_is_a_configuration_error` (Unix),
  `a_second_open_of_the_same_directory_is_in_use`, `an_existing_key_without_a_file_is_reused_and_never_created_again`,
  `no_key_and_no_file_creates_the_key_once`, `a_file_without_its_key_is_key_missing_and_no_key_is_created`,
  `a_keychain_failure_is_its_unavailable_reason`, `a_store_refuses_a_key_of_another_kind`,
  `a_wrong_key_is_unavailable_and_the_file_is_never_plaintext` (macOS), `a_plaintext_store_file_is_refused` (macOS),
  `a_passing_io_failure_at_open_is_retried_not_an_integrity_failure`,
  `an_unreadable_store_file_is_unavailable_until_it_can_be_read` (Unix),
  `a_newer_schema_is_refused_and_the_file_is_unchanged`, `a_migration_row_is_dated_no_earlier_than_the_last_commit`,
  `the_backup_opens_with_the_store_key_and_verifies`, `migrations_run_in_order_and_a_gap_is_refused`,
  `a_failed_migration_changes_nothing`, `a_store_left_without_tables_is_created_on_the_next_open`.
- `time.rs` (moved): `known_instants`, `a_later_instant_is_written_as_the_last_one`,
  `the_contract_crate_reads_every_day_back`.
- `tx.rs` (section 4, D34): `a_write_appends_one_ledger_row_and_a_refused_write_appends_none`,
  `a_write_without_a_mutation_appends_no_ledger_row`, `an_audit_only_write_appends_no_ledger_row`,
  `notices_take_the_next_sequences_and_a_refusal_takes_none`, `a_write_dated_before_the_last_commit_is_refused`,
  `an_unknown_operation_type_is_refused`, `creation_and_migration_mutations_are_the_runner_s`,
  `the_run_of_a_write_is_the_run_it_starts`, `a_write_that_goes_on_after_a_refused_mutation_commits_only_the_rest`,
  `a_fault_that_ends_the_transaction_fails_the_whole_write_even_when_the_caller_goes_on` (the poisoned write).

`crates/fabric-broker-core/src/`:
- `ids.rs`: `a_new_id_is_the_prefix_and_32_hex_digits`, `a_provider_event_id_is_26_base32hex_digits`,
  `base32hex_writes_every_bit`.
- `owner/groups.rs` (D19): `removing_members_or_domains_narrows_a_set`, `a_new_member_or_domain_widens_a_set`.
- `owner/profiles.rs` (D20, 6.1): `the_same_rows_or_narrower_rows_are_no_expansion`,
  `every_wider_row_is_listed_in_matrix_order`, `block_to_ask_is_an_expansion_and_allow_to_block_is_not`,
  `the_expansion_digest_is_over_the_listed_rows`, `a_built_in_name_in_any_case_is_taken`,
  `a_built_in_name_with_spaces_or_invisible_characters_is_taken`, `a_new_profile_id_is_custom_and_16_hex_digits`.
- `owner/subscribe.rs` (5.4, D5, D6, D38): `a_cursor_ahead_of_the_store_needs_a_snapshot`,
  `a_cursor_below_the_base_needs_a_snapshot`, `a_cursor_equal_to_the_base_after_a_restart_is_accepted`,
  `a_cursor_older_than_the_ring_needs_a_snapshot`, `a_subscription_sends_the_ringed_notices_after_its_cursor`,
  `nothing_is_written_before_a_subscription`, `a_step_of_256_notices_is_sent_whole_and_257_needs_a_snapshot`,
  `a_frame_over_the_bound_is_never_written`, `a_new_run_empties_the_ring_and_ends_the_subscription`.
- `owner/tests.rs` (5.3, 6.1): `hello_reports_the_broker_the_build_and_the_limits`,
  `hello_with_another_schema_version_is_refused`, `refusals_by_layer`, `an_oversized_line_is_malformed`,
  `connect_google_is_unknown_in_this_build_and_not_stored`,
  `a_duplicate_command_gets_the_same_bytes_and_changes_nothing`, `a_duplicate_refusal_is_answered_again`,
  `a_reused_command_id_is_refused`, `stale_unknown_and_narrowed_decisions_are_refused`,
  `activation_raises_the_profiles_revision_and_the_revocation_epoch_once`,
  `a_custom_profile_named_like_a_built_in_is_refused`, `an_expansion_needs_the_owner_to_acknowledge_its_digest`,
  `saving_a_custom_profile_raises_the_revision_and_only_the_active_one_the_epoch`,
  `a_subscription_at_the_base_after_a_restart_is_accepted_and_below_it_is_not`, `get_proof_hides_a_synthetic_report`,
  `proof_ids_are_distinct_and_newest_first`, `full_details_carry_the_action_and_redacted_ones_do_not`,
  `disconnect_google_sends_the_account_notice_last_and_answers_with_its_sequence`,
  `pausing_all_agents_raises_each_revision_and_the_pause_epoch`, `a_revoked_grant_ends_and_a_second_revoke_is_conflict`,
  `a_warning_calendar_must_name_a_stored_calendar`.
- `service/activity.rs` (D35): `a_session_with_no_call_idles_1800000_ms_after_its_connect`,
  `a_call_moves_the_idle_deadline_and_records_the_last_tool_call`, `an_ended_session_has_no_deadline`.
- `service/clock.rs` (D34): `a_clock_starts_at_the_wall_clock_when_it_is_ahead`,
  `a_clock_starts_at_the_last_commit_when_the_wall_clock_is_behind`, `a_backward_step_keeps_now_and_grows_the_offset`,
  `a_forward_step_moves_now_forward_and_keeps_the_offset`, `now_never_decreases`.
- `service/epochs.rs` (5.6): `a_fence_holds_only_while_every_version_is_unchanged`.
- `service/facts.rs` (10.3): `a_one_request_grant_is_offered_only_to_its_origin_request`,
  `a_classification_is_offered_no_ended_grant`, `a_fresh_proof_is_offered_every_relied_grant_even_ended`,
  `a_pending_effect_names_its_calendar_and_its_target_event`.
- `service/grants.rs` (5.5): `an_ended_grant_of_any_term_is_never_valid`,
  `a_one_request_grant_is_valid_only_while_its_origin_is_open`, `an_until_revoked_grant_is_valid_until_it_ends`,
  `a_session_grant_needs_its_session_live_in_this_run`, `a_session_grant_idles_after_1800000_ms_of_activity`,
  `a_session_with_no_call_idles_1800000_ms_after_its_connect`, `a_grant_names_its_sets_and_its_accounts`.
- `service/limits.rs` (5.9, D24): `each_limit_refuses_at_its_maximum_and_not_below`, `the_total_is_checked_first`,
  `the_rate_waits_until_the_oldest_call_leaves_the_window`, `a_busy_window_is_one_row_per_agent_with_its_count`,
  `quit_flushes_every_open_window`.
- `service/pipeline.rs` (5.7, D16, D17): `the_grant_check_refuses_any_id_outside_the_facts`,
  `only_an_isabelle_permit_allows_a_classification`, `a_fresh_permit_authorizes_only_when_everything_it_needs_is_there`,
  `block_and_need_approval_need_a_ruling`, `the_reproof_counters_are_per_purpose_and_stop_at_the_maximum`,
  `transient_failures_wait_2_4_and_8_s_then_end`, `a_provider_hint_longer_than_the_backoff_is_used_within_the_budget`.
- `service/reducer.rs` (5.7): `the_reducer_never_yields_a_move_the_lifecycle_refuses` (all 23 x 23 pairs),
  `the_reducer_drives_exactly_the_transitions_of_the_w04_table` (25 of 39), `no_event_moves_a_terminal_request`,
  `the_transitions_left_to_later_packages_are_not_driven`.
- `service/tests.rs` (in-process service): `facts_hold_every_connected_account_and_its_selected_calendars` (11.3 W2),
  `a_classification_has_no_relied_list_and_a_fresh_proof_the_stored_one`,
  `an_ended_relied_grant_is_offered_to_the_fresh_proof_marked_ended`,
  `a_one_request_grant_is_offered_only_to_its_origin_request`,
  `a_result_naming_a_grant_outside_its_facts_is_undecided_for_every_purpose_and_verdict`,
  `the_watchdog_ends_a_proof_past_35_s_and_a_late_result_is_ignored`,
  `a_fresh_permit_is_authorized_then_handed_off_in_its_own_commit`,
  `a_transient_attempt_waits_and_the_retry_proves_again_on_the_relied_list`,
  `recovery_ends_the_dead_runs_sessions_and_leaves_its_dispatch_unknown`, `a_restart_with_nothing_open_sends_no_notice`,
  `a_repeated_call_returns_its_request_and_writes_nothing`, `a_call_past_a_limit_is_busy_and_counted_in_the_agents_window`,
  `a_connect_that_replaces_a_session_raises_the_revision_once_and_closes_the_old_connection`,
  `a_call_on_a_connection_without_a_session_is_not_permitted`, `quit_ends_live_sessions_and_their_grants_with_app_quit`,
  `an_idle_session_ends_with_its_grants_after_1800000_ms`,
  `a_discarded_classification_is_proved_again_until_its_counter_is_spent`,
  `a_backward_clock_step_is_reported_once_and_never_lowers_now`,
  `an_authorization_issued_before_a_clock_step_expires_60_s_later_on_the_floored_clock` (the authorization half of
  c29), `a_fence_changed_while_queued_voids_the_authorization`, `a_grant_ended_while_queued_voids_the_authorization`,
  `an_agent_paused_while_queued_voids_the_authorization` (the three `queued` voids of c08).

`apps/fabric-broker/src/`:
- `config.rs` (3.3, D27): `the_test_key_and_a_directory_are_a_configuration` (debug),
  `the_synthetic_agent_runs_with_the_test_key` (debug), `the_synthetic_agent_is_refused_without_the_test_key`,
  `a_missing_variable_is_named`, `a_relative_data_directory_is_refused`, `a_data_directory_that_is_a_file_is_refused`,
  `an_unknown_value_is_refused_without_printing_it`, `a_release_build_refuses_the_test_key` (release),
  `a_release_build_refuses_the_synthetic_agent` (macOS release),
  `the_variable_names_are_those_the_host_and_the_relay_spell`, `an_unknown_broker_variable_is_refused`,
  `variables_outside_the_prefix_are_ignored`, `the_keychain_is_refused_off_macos` (not macOS),
  `the_keychain_is_a_key_source_on_macos` (macOS).
- `reader.rs` (unchanged): `splits_at_line_feeds`, `a_line_at_the_bound_is_a_frame`,
  `a_longer_line_is_reported_once_and_skipped`, `an_unterminated_last_line_is_reported`,
  `end_inside_an_oversized_line_is_the_end`, `carriage_returns_are_kept`.
- `synthetic.rs` (5.10): `a_proposal_is_a_valid_create_on_the_synthetic_calendar`,
  `the_seed_names_the_synthetic_agent_and_account`, `the_classifier_never_answers_a_permit`,
  `a_new_agent_has_nothing_due`.

### 9.2 Storage acceptance (`crates/fabric-storage/tests/`)

| Id | File | Test | Proves |
|---|---|---|---|
| s01 | `open.rs` | `pragmas_hold_on_every_connection` | WAL and the 250 ms busy timeout hold on every open (3.1) |
| s02 | `encryption.rs` (macOS) | `no_plaintext_in_db_wal_shm_or_open_files` | no byte of a marker in `broker.db`, `-wal`, `-shm`, any file of the directory or any open file, store open and closed; a control shows the scanner sees it (3.1; R12, R13) |
| s02 | `encryption.rs` (macOS) | `compile_options_have_codec_and_temp_store_3` | the linked SQLite is SQLCipher with `TEMP_STORE=3` |
| s02 | `encryption.rs` (macOS) | `cipher_provider_is_commoncrypto` | the provider is CommonCrypto (SPIKE option (c)) |
| s03 | `encryption.rs` (macOS) | `a_wrong_key_is_unavailable_never_plaintext` | a wrong key is `WrongKey`, final; the file is byte-identical |
| s03 | `encryption.rs` (macOS) | `a_plaintext_file_at_the_store_path_is_refused` | a plaintext SQLite file is `NotEncrypted` and left untouched |
| s04 | `open.rs` | `an_existing_store_without_its_key_is_key_missing_and_unchanged` | 3.2: no new key over an existing file |
| s04b | `open.rs` | `a_key_left_without_a_store_is_reused_and_never_created_again` | 3.2: a key left by a crash is reused; `create` never called |
| s05 | `open.rs` | `a_store_refuses_a_key_of_another_kind` | 3.4 step 8: `KeyKindMismatch`, store unchanged |
| s05b | `open.rs` | `a_test_store_refuses_the_keychain_kind_whatever_key_it_holds` | 3.2: a test-key store meets a Keychain-kind provider as `KeyKindMismatch` with no key and with its own key; a Keychain-kind store whose key is gone stays `KeyMissing` |
| s06 | `open.rs` | `a_second_open_of_the_same_directory_is_in_use` | one process per data directory; the lock frees when the store is gone |
| s06 | `crash.rs` | `the_lock_is_free_after_its_process_is_killed` | the OS releases `broker.lock` when its process dies |
| s07 | `open.rs` | `a_newer_schema_is_refused_and_the_file_is_unchanged` | 3.4 step 5: byte-identical file, no backup |
| s08 | `migration.rs` | `the_v1_store_migrates_with_a_verified_backup` | one migration transaction after one backup; chain and replay verify |
| s08 | `migration.rs` | `the_backup_opens_with_the_same_key_and_verifies_chain_and_replay` | the backup is keyed with the store key and verifies as a store |
| s08 | `migration.rs` | `the_backup_name_binds_schema_and_ledger_head` | the name of 3.6 |
| s08 | `migration.rs` | `restoring_the_backup_reopens_version_1` | a downgrade is restoring the backup by hand |
| s08 | `migration.rs` | `a_migration_with_a_clock_behind_the_last_commit_keeps_ledger_time_non_decreasing` | D34 at a migration; the next start verifies |
| s09 | `migration.rs` | `migration_1_matches_the_frozen_schema` | migration 1 and a store made by `Store::open` have the schema of `fixtures/schema-v1.sql` |
| s09 | `migration.rs` | `the_frozen_schema_reads_the_same_from_a_crlf_checkout` | the comparison holds on a CRLF checkout |
| s10 | `ledger.rs` | `a_write_appends_one_ledger_row_and_a_refused_write_appends_none` | section 4, D3 |
| s10 | `ledger.rs` | `a_write_without_mutations_appends_no_ledger_row` | section 4 |
| s10 | `ledger.rs` | `an_audit_only_write_appends_no_ledger_row` | D24 |
| — | `ledger.rs` | `ledger_rows_carry_the_envelope_and_hashes_of_section_4` | the envelope fields and the hash rule |
| s11 | `ledger.rs` | `a_changed_ledger_row_makes_the_store_unavailable` | a changed row is named; the next open is `Integrity` |
| s11 | `ledger.rs` | `a_truncated_tail_fails_the_head_check` | the head in `meta` catches truncation |
| s11 | `ledger.rs` | `a_ledger_time_that_goes_backward_fails_verification` | D34 in `verify_chain` |
| s11 | `ledger.rs` | `ledger_and_audit_rows_cannot_be_updated_or_deleted` | the append-only triggers hold for a second keyed connection |
| s12 | `ledger.rs` | `a_failed_commit_leaves_no_row_and_the_head_unchanged` | 5.2, 7.2: an injected trigger rolls the whole write back |
| s13 | `replay.rs` | `replay_rebuilds_every_projection_table` | seeded random writes over every projection table replay equal, before and after a reopen |
| s13 | `replay.rs` | `replay_detects_a_projection_changed_outside_apply` | a projection change beside `apply` fails replay while the chain verifies |
| s14 | `ledger.rs` | `notice_sequences_rise_by_one_and_survive_reopen` | D5 |
| s15 | `crash.rs` | `an_abort_inside_a_write_leaves_nothing` | 7.2 |
| s15 | `crash.rs` | `a_sigkill_inside_a_write_leaves_nothing` | 7.2 |
| s15 | `crash.rs` | `a_committed_write_survives_sigkill` | 7.1, `synchronous = FULL` |
| s15 | `crash.rs` | `a_store_left_without_tables_is_created_on_the_next_open` | 3.4 step 7, both setups of 7.2 |
| s16 | `fitness.rs` | `only_apply_writes_projection_tables` | D3: a source scan of `crates/fabric-storage/src` |
| — | `fitness.rs` | `the_scan_finds_projection_writes_and_ignores_comments` | the scan itself |
| s17 | `secrecy.rs` | `failures_print_no_key_and_no_content` | D37: a wrong key (macOS), an injected trigger, a JSON column corrupted to hold a marker (as a JSON string, which serde would quote), a failed backup; no Display, Debug or owner message holds the key's hex or the marker |
| M1 | `keychain.rs` (macOS, ignored) | `keychain_key_round_trip` | 3.2: the key goes into the Keychain and comes back unchanged |

### 9.3 Core acceptance (`crates/fabric-broker-core/tests/`; in-process, test key)

The tests start the broker with `Attachments { prover: Some(ProverIdentity { kind: Isabelle, .. }), provider_present:
true }` where needed and feed `ProofDone` / `AttemptDone`. The shared harness is `tests/common/mod.rs`.

| Id | File | Test | Proves |
|---|---|---|---|
| c01 | `grants.rs` | `restart_ends_session_grants_with_broker_restart` | 5.5, 5.8; an until-revoked grant is untouched |
| c02 | `grants.rs` | `quit_ends_session_grants_with_app_quit` | 5.8; a clean end of the run |
| c03 | `grants.rs` | `until_revoked_survives_restart_and_revoked_stays_revoked` | 5.5, 5.8 |
| c04 | `grants.rs` | `a_one_request_grant_ends_consumed_with_its_request_and_serves_no_other` | D18 |
| c05 | `dispatch.rs` | `an_authorization_from_before_a_restart_is_never_handed_off` | DEF-4: consumed stays consumed; issued is voided `broker_restart`; within 60 s |
| c06 | `sessions.rs` | `an_idle_session_ends_at_1800_s_with_session_idle_timeout` | 5.5 |
| c06 | `sessions.rs` | `a_status_poll_keeps_a_session_alive` | D35, R20 |
| c06 | `sessions.rs` | `a_session_with_no_call_has_last_active_at_its_start_and_idles_1800_s_after_connect` | D35; `last_tool_call_at` null |
| c07 | `races.rs` | `revoke_while_a_proof_runs_is_acknowledged_first_and_the_permit_is_discarded` | 5.1, 5.6: the permit is discarded and the re-proof blocks |
| c08 | `races.rs` | `pause_while_queued_refuses_the_handoff` | a pause before the handoff: no dispatch, no consumed authorization, the fresh proof sees the pause. The `queued` setting has no public entry (5.1 pumps after every input); its void reasons are unit tests (9.1) |
| c09 | `races.rs` | `revoke_before_a_retry_refuses_the_retry` | the retry's fresh proof sees the relied grant ended; Block ends `blocked` |
| c10 | `races.rs` | `a_revoke_after_handoff_leaves_the_attempt_visible` | 6.1 `revoke_grant` |
| c11 | `grants.rs` | `an_approval_binds_the_digest_and_the_order_names_one_action` | 6.1 `decide_request`, 5.5 |
| c11 | `grants.rs` | `a_session_grant_binds_the_current_live_session` | 5.5, D22 |
| c12 | `faults.rs` | `a_failed_authorize_or_dispatch_write_hands_off_nothing_and_halts` | triggers on `request.authorize`, `request.dispatch`, `agent.call`, `owner.revoke_grant`, `owner.decide_request` |
| c13 | `faults.rs` | `a_busy_store_answers_store_unavailable_and_halts` | 5.2, 7.2 |
| c14 | `faults.rs` | `a_halted_output_carries_no_job_and_a_committed_dispatch_restarts_as_outcome_unknown` | D7 |
| c14 | `faults.rs` | `a_halted_step_drops_the_job_of_an_earlier_commit_in_the_same_step` | D7, deterministically: a due retry commits and queues its proof, the owner write of the same step fails, no job leaves |
| c15 | `faults.rs` | `a_kill_after_dispatch_intent_restarts_as_outcome_unknown_with_the_same_provider_event_id` | 5.8, 3.5 `effects` |
| c16 | `owner_channel.rs` | `duplicate_commands_after_restart_get_the_same_bytes` | D23 |
| c16 | `owner_channel.rs` | `a_reused_command_id_is_command_reused` | 5.3, audited |
| c16 | `owner_channel.rs` | `a_stale_expected_revision_is_conflict` | 5.3 step 6 |
| c17 | `owner_channel.rs` | `subscribe_gaps_and_the_256_bound` | 5.4, D6 |
| c17 | `owner_channel.rs` | `after_a_restart_a_cursor_at_the_base_is_accepted` | 5.4 ring base |
| c18 | `proofs.rs` | `no_prover_ends_every_request_undecided_with_no_report` | D14 |
| c18 | `proofs.rs` | `a_synthetic_permit_is_never_accepted` | D13; its report is never shown as a proof |
| c18 | `proofs.rs` | `a_proof_past_35_s_is_undecided_and_a_late_result_is_ignored` | D16 watchdog |
| c18 | `proofs.rs` | `permit_without_a_provider_is_undecided` | D17 |
| c18 | `proofs.rs` | `a_fresh_permit_with_no_authorizing_grant_is_undecided` | L7(d), D17 |
| c18 | `proofs.rs` | `a_result_naming_a_grant_outside_its_facts_is_undecided_and_never_halts` | D17: recorded, relied list unchanged, `undecided`, no halt, a restart repeats nothing |
| c18 | `proofs.rs` | `a_report_bound_to_another_job_neither_authorizes_nor_halts` | D39: request, purpose or action digest of another job; nothing recorded under either request |
| c19 | `proofs.rs` | `a_blocked_request_keeps_rule_and_proof_after_restart` | DEF-7 store half |
| c19b | `proofs.rs` | `a_request_blocked_by_its_fresh_proof_keeps_rule_and_proof_after_restart` | DEF-7 for an action denied when authorizing |
| c20 | `limits.rs` | `busy_limits_create_no_request` | 5.9: the four limits |
| c20 | `limits.rs` | `busy_refusals_are_audited_once_per_agent_per_minute` | D24 |
| c20 | `limits.rs` | `the_open_card_limit_is_checked_only_at_creation` | 5.9, R18 |
| — | `limits.rs` | `the_owner_records_are_bounded_so_a_snapshot_fits_in_one_frame` | D38: the budget for sets, profiles and approvals; refusals store nothing; deny still works |
| — | `limits.rs` | `an_answer_over_the_frame_bound_is_refused_and_the_broker_runs_on` | D38: `internal` with id and method, no halt |
| c21 | `grants.rs` | `grants_and_calendars_are_namespaced_by_account` | 3.5, 11.3 W2: every connected account and every selected calendar, each named with its account; a disconnect ends that account's grants only |
| c22 | `sessions.rs` | `an_unknown_agent_gets_no_session_and_no_request` | 5.5, section 4 access audit |
| c23 | `grants.rs` | `widening_or_deleting_a_recipient_set_ends_the_grants_that_name_it` | 5.6, D19 |
| c24 | `load.rs` | `one_writer_under_load_every_record_verifies` | PLAN 5 R09 and the DEF-1 sequences: 500 agent calls interleaved with owner commands through one in-process writer (a second writer is ruled out by s06, p02) |
| c25 | `faults.rs` | `an_unavailable_store_answers_hello_and_refuses_everything_else` | D8, 3.4: owner methods and the agent entry |
| c26 | `dispatch.rs` | `the_retry_budget_ends_in_retry_budget_exhausted` | D16: waits of 2, 4, 8 s; the fourth Transient ends `terminal_failure`; a hint past 120 s ends it at once |
| c27 | `faults.rs` | `a_reopen_after_in_use_runs_recovery` | D9, D30, 5.8: the dead run's grant and authorization end before the frame is answered |
| c27b | `faults.rs` (Unix) | `a_reopen_after_a_passing_io_failure_opens_the_store` | D9 `Io`: unavailable with the `Io` message, reopened after 5 s |
| c28 | `proofs.rs` | `an_allowed_request_rests_on_the_classification_grants` | D36 |
| c28 | `proofs.rs` | `an_approved_request_rests_on_the_approval_grant` | D36 |
| c28 | `proofs.rs` | `the_authorization_names_the_first_authorizing_grant_and_the_store_keeps_the_list` | L7(c) |
| c28 | `proofs.rs` | `each_accepted_fresh_proof_replaces_the_relied_list` | D36 |
| c28 | `proofs.rs` | `a_discarded_fresh_result_leaves_the_relied_list_unchanged` | D36, 11.3 W1 |
| c28 | `proofs.rs` | `a_relied_grant_that_ended_is_offered_to_the_proof_as_ended` | 5.7, 10.3 |
| c29 | `dispatch.rs` | `a_backward_clock_step_neither_halts_nor_lowers_time` | D34, 5.1 (the authorization half is a unit test, 9.1) |
| c30 | `sessions.rs` | `repeated_calls_and_status_polls_write_no_ledger_row` | D25 |
| c30 | `sessions.rs` | `activity_never_raises_the_agent_revision` | 5.4, D35 |
| c30 | `sessions.rs` | `a_connect_that_replaces_a_live_session_raises_the_agent_revision_once` | 5.4, 5.5 |
| c31 | `proofs.rs` | `the_reproof_counter_is_per_purpose_and_survives_a_retry` | D16 |
| — | `fitness.rs` | `forbidden_dependencies_are_absent` | the table of section 2 |
| — | `fitness.rs` | `the_manifest_scan_reads_every_dependency_form` | the scan itself |

### 9.4 Process and shell tests

`apps/fabric-broker/tests/process.rs`: every start removes each inherited variable whose name starts
`FABRIC_BROKER_` (`FABRIC_BROKER_UNDER_TEST` among them), then passes `FABRIC_BROKER_DATA_DIR` = its own new
directory and `FABRIC_BROKER_KEY_SOURCE=test`. After a kill it waits up to 2 s for `broker.lock`.
- Kept from CD-W02: `malformed_frames`, `a_line_of_exactly_the_bound_is_read_as_a_frame`,
  `an_oversized_line_is_answered_once_and_skipped`, `wrong_protocol`, `schema_refusals_carry_the_id_and_method`,
  `hello_with_another_schema_version_is_refused`, `subscribe_with_a_cursor_ahead_needs_a_snapshot`,
  `an_argument_is_a_usage_error`, `end_of_input_stops_the_broker_and_stdout_holds_only_frames`,
  `end_of_input_before_anything_exits_zero`, `a_write_error_ends_the_broker`.
- Changed: `hello_reports_the_limits_of_the_contract` (name, build), `the_first_run_snapshot` (`store: ready`, cursor
  0), `connect_google_is_not_available_until_cd_w05`, and with the synthetic agent after subscribing:
  `an_approval_of_a_synthetic_request_creates_a_grant_and_ends_undecided`, `refused_decisions_and_a_denial`.
- New:

| Id | Test | Proves |
|---|---|---|
| p01 | `configuration_errors_exit_78_before_any_frame` | each refused setting exits 78 with one stderr line, before any frame; never a macOS broker with `keychain` |
| p02 | `a_second_broker_on_the_same_directory_answers_store_unavailable` | the lock keeps a second broker out; it answers `hello` |
| p03 | `a_newer_schema_store_is_refused_but_hello_answers` | the store is left alone; stdout holds frames only |
| p04 | `state_survives_sigkill_and_restart` | a pending request and an until-revoked grant survive; a session grant ends `broker_restart`; sequences continue. The store-survival check of the packaged run (L1) |
| p05 | `a_duplicate_command_after_restart_gets_the_same_bytes` | the command table is durable |
| p06 | `end_of_input_ends_sessions_and_exits_within_the_bound` | quit in one commit within the stop bound |
| p07 | `the_synthetic_agent_waits_for_subscribe_asks_and_an_approval_ends_undecided` | 5.10 |
| p08 | `stdout_and_stderr_carry_no_event_content` | PLAN 5 R13: no event content outside a full-detail read; no key on stderr |

`apps/fabric-broker/tests/relay_round_trip.rs`: `start_at` builds its environment without any `FABRIC_BROKER_*`
name, then the three variables (own directory, test key, synthetic). Tests: `echo_through_the_relay`,
`notices_are_pushed_over_time`, `approval_then_denial`, `a_duplicate_command_is_answered_once` (the synthetic agent
after subscribing; ids read from notices; sequences rise by one, no fixed list), `a_refused_frame_never_reaches_the_broker`,
`the_death_of_the_broker_ends_the_session`, `quit_stops_the_broker_without_a_kill`, `a_broker_in_a_directory_with_spaces`.

Relay unit tests: `allowlist_names_the_broker_variables`, `broker_variables_are_named_by_the_prefix`. Host tests: the
existing `Host::start` tests pass a per-test directory and the test key, and the real-broker round trip also restarts
with the same store; new: `a_refused_store_setting_is_a_configuration_error_and_nothing_starts`,
`a_debug_host_takes_the_test_key_only_with_a_scratch_directory`, `a_release_host_ignores_all_three`,
`every_broker_variable_the_host_sets_passes_the_relay_allowlist`, `the_broker_environment_drops_inherited_broker_variables`.
`verify-desktop.sh --packaged` (10.6) and `harness/serve.ts` use a scratch data directory and the test key.

---------------------------------------------------------------------------------------------------------------------

## 10. Interfaces

The public interfaces are documented in the code. `sh scripts/run.sh cargo doc --workspace --no-deps --locked` builds
the rustdoc of `fabric_storage`, `fabric_broker_core` and `fabric_broker`; this section says what each part is for.

### 10.1 Where the code is

Section 2 lists the modules. The three crates have one direction of dependency: the executable composes the core,
the core uses the store, the store uses only the contract crate. Nothing outside `fabric-storage` opens a
connection to the store, except the tests' second connection for fault injection (7.2).

### 10.2 `fabric-storage`

- Keys: `KeyProvider` (`kind`, `read`, `create`), `TestKey`, `KeychainKey` (macOS; `product()` with the names of
  D28, `with_names` for the ignored test), `StoreKey` (32 bytes in zeroed-on-drop memory; no Debug, no Display),
  `KeyError`, `KeyKind`, `TEST_KEY`.
- Opening: `Store::open(OpenOptions { data_dir, key, now_ms })` runs 3.4; `Store::open_with_migrations` takes
  another migration list (tests rehearse N -> N+1 and a failing migration 1). Errors: `OpenError::Config` (exit 78)
  and `OpenError::Unavailable(Unavailable)`, whose `store_reason()`, `retry_may_help()` and `message()` drive 3.4's
  table.
- Writing: `Store::write(OpMeta, |tx: &mut WriteTx| ...)` returns `Committed { value, ledger_seq, notices }` or
  `TxError::{Refused, Fault}`; inside, `WriteTx::apply(Mutation)`, `notify(Change)`, `audit(AuditEntry)`, `read()`.
  `Store::write_audit` writes audit rows only. `OpMeta` carries the operation type, the actor, the idempotency key,
  causation, correlation and `now_ms` (section 4).
- Reading: `Store::read(|r: &ReadTx| ...)`, one consistent view: meta, counters, runs, profiles, preferences,
  recipient sets, accounts, agents, sessions, requests (by id, by call, open, due, counts), grants, proofs,
  authorizations, effects (`open_effects` with each effect's target event, `OpenEffect`), attempts, commands,
  `next_due_ms`.
- Checking: `Store::verify_chain`, `Store::verify_replay`.
- Records: `rows` (`AgentRow`, `SessionRow`, `RequestRow`, `RequestRecord`, `Reproofs`, `ProofRecord`, `ProverKind`,
  `Fence`, `AuthorizationRecord`, `EffectRow`, `OpenEffect`, `AttemptRow`, `CommandRow`, `Meta`, `Counters`) and
  `mutation` (`Mutation`, `Counter`, `SessionEndReason`, `VoidReason`; serialized with tag `op`, payload version 1).
- Constants: `DATABASE_FILE`, `LOCK_FILE`, `BACKUP_DIR`, `PROVER_DIR`, `SCHEMA_VERSION`, `MIGRATIONS`; `time`:
  `format_timestamp`, `MAX_TIMESTAMP_MS`; `ledger::operation_id`.

### 10.3 Ports (`crates/fabric-broker-core/src/ports.rs`)

- Prover: `Prover::{identity, prove}` runs on a worker thread. A `ProofJob` carries the job id, request id, purpose,
  `ProofAction::Agent(Action)`, the contract `action_digest` (`sha256:` + 64 hex; the W03 adapter strips the prefix),
  `TrustedFacts`, the `Fence`, the start and the deadline (start + 35 s). A `ProofDone` carries the job id, request
  id, purpose, an optional `ProofReport`, an optional `Ruling` (required for block and need_approval),
  `authorizing_grants` in the prover's order, and `undecided_reason` and `check_theory_text`, kept only in the
  encrypted store. A prover never errs: every failure of the prover itself is a `ProofDone` without a permit.
  `ProverIdentity { kind, health }` is fixed at start (D13).
- Provider: `Provider::dispatch(&DispatchOrder) -> AttemptDone` runs on a worker thread. A `DispatchOrder` carries the
  consumed `Authorization`, the action, account, calendar, attempt number, the provider event id reserved for a
  create, and the deadline (start + 30 s). `AttemptReport` is `Applied`, `Transient { retry_after_ms }`,
  `Rejected { code, explanation }` or `NoAnswer`. CD-W05 implements and extends it (reconcile, token revocation).
- Agent adapter: `AgentInput::{Connect, Call, Disconnect}` on a `ConnectionId`; answers come back as `AgentReply`.

`TrustedFacts` holds broker records only, read in one transaction plus the core's activity (THEORY section 1; L6);
nothing from an agent or a model. Where each W03 `Context` field comes from (the Isabelle wiring maps field for
field):

| `Context` (L6) | `TrustedFacts` / `ProofJob` | Source |
|---|---|---|
| actor | `actor` | the request's agent; `System` only for later broker actions |
| paused | `paused` | `agents.paused` |
| now (UTC ms) | `now_ms` | the core clock (D34) |
| session id and last active | `session` | live session of this run; activity in memory |
| profile in force | `profile` | the active profile's rows |
| connected account ids | `connected` | every connected account (11.3 W2) |
| selected calendars (account, id, writable, Agent Drafts) | `calendars` | every selected calendar of those accounts (11.3 W2) |
| warning calendars | `warning_calendars` | `preferences` |
| grants (id, tools, duration incl. one-request digest and used, or session id, or until revoked; ended; calendars; window; fields; recipient rule; send_updates) | `grants` (`GrantFact { grant, used, ended }`) | the agent's live grants (a one-request grant only for its origin request) and every relied grant even when ended; `ended` = ended or failing the validity predicate (5.5) now; `used` from the origin request's state |
| recipient sets the grants name | `recipient_sets` | `recipient_set_versions` (current) |
| observation of the named event | `event` | CD-W05 (None now) |
| action digest (64 hex) | `ProofJob.action_digest` with `sha256:` stripped | the contract's `action_digest` |
| relied (list) | `relied` | `requests.relied_grant_ids` (empty for classification) |
| result_held | `result_held` | CD-W05/W06 (false now) |
| pending effects (account, calendar, event) | `pending_effects` | `ReadTx::open_effects`: effects whose request is `dispatching`, `outcome_unknown` or `reconciling`; the event is the reserved id for a create and the action's target (`EventRef.event_id`, or `OccurrenceRef.instance_id` for a response to one occurrence) for every other tool |
| pause and revocation epochs | `pause_epoch`, `revocation_epoch` | `counters` |

### 10.4 Broker API (`crates/fabric-broker-core/src/broker.rs`)

`Broker::start(opener, settings, attachments, now_ms)` runs `Core::open` and returns the broker with its first
`Output`, or the unavailable mode on `OpenError::Unavailable`; `StartError::Config` is exit 78 and
`StartError::Halt` exit 75. `Broker::handle(Input, now_ms) -> Output` takes one input (`OwnerLine`, `OwnerOversized`,
`Agent`, `ProofDone`, `Attempt`, `ProverHealth`, `Tick`); `next_due_ms` is the earliest of the store's due requests,
live sessions' idle deadlines, proof watchdogs and `busy` windows; `quit` is the end of stdin (5.8); `seed` (debug
builds) creates agents and accounts if absent; `store_ready` and `subscribed` report state. An `Output` holds owner
frames, agent replies, connections to close, jobs (`Job::Prove`, `Job::Dispatch`), stderr lines and the halt.
`Settings::cd_w04(build)` uses `Limits::CD_G0` and `RetryPolicy::PLAN`; `Attachments` names the prover and whether a
provider is attached. The constants are `BROKER_NAME`, `PROOF_WATCHDOG_MS` (35 000), `MAX_REPROOFS` (3),
`PROVIDER_CALL_MS` (30 000), `REOPEN_INTERVAL_MS` (5 000), `SESSION_IDLE_MS` (1 800 000), `BUSY_AUDIT_WINDOW_MS`
(60 000). `ids::new_id(prefix)` is prefix + `-` + 32 hex; `ids::provider_event_id()` is 26 base32hex digits from 128
bits.

### 10.5 Service and owner channel (crate-private)

The service (`src/service/`) owns `Core` (store, settings, attachments, run id, clock, activity, in-flight jobs,
connections, `busy` windows) and every write of the pipeline, the agent entry, recovery and quit. Its helpers for the
owner channel are `requests::move_request`, `grants::{create_grant, end_grant, end_grants_naming_set,
end_grants_of_account, grant_is_valid}`, `epochs::raise`, `sessions::{end_session, agent_view, raise_agent,
start_session}`, `relied::set_relied` and `stale_pending_of_account`. A step's in-memory effects (jobs, session
changes) are collected in `AfterCommit` and applied only after commit. The owner channel (`src/owner/`) owns
`OwnerChannel` (the subscription, the ring and its base, the build label): `begin_run` after every `Core::open`,
`answer` for one line against a ready core, `answer_unavailable` in the unavailable mode, `publish` for the service's
notices, `fit_frames` for the frame bound (D38).

### 10.6 Composition and shell

Broker `main.rs`: arguments -> 64; `config_from(std::env::vars_os())` -> 78; opener from `KeychainKey::product()` or
`TestKey`; attachments: in synthetic mode `prover = SyntheticClassifier.identity()`, otherwise none; no provider;
`Broker::start` (`StartError::Config` -> 78, `Halt` -> 75). One bounded `sync_channel` (`QUEUED_LINES` = 4) carries
stdin lines from the reader thread and worker results. Jobs go to workers over unbounded channels, so main never
blocks on a worker; a worker may block on the full result channel until main drains it, which cannot deadlock. A
prover worker exists only in synthetic mode; no provider worker. The loop waits on `min(broker.next_due_ms(),
synthetic.next_due_ms())`; after every broker call `synthetic.take_replies`; `Output.stderr` lines go to stderr;
frames are written one at a time; `Output.halt` -> write frames, one stderr line with the fault's Display, exit 75;
end of input -> `quit`, exit 0.

Host (`apps/fabric-desktop/src-tauri/src/host.rs`): `BrokerStore::from_environment(app_data_dir, vars)` returns the
root (the lock directory) and the store settings (`data_dir = root/broker`). Release builds: root = `app_data_dir`,
Keychain, no synthetic agent, the three names ignored. Debug builds: root = `FABRIC_DESKTOP_DATA_DIR` if set (it must
be absolute; alone it moves the store and keeps the Keychain); `FABRIC_BROKER_KEY_SOURCE=test` and
`FABRIC_BROKER_SYNTHETIC_AGENT=1` are honoured only with `FABRIC_DESKTOP_DATA_DIR` set, the synthetic agent only with
the test key, and unknown values are refused; a refusal is a configuration error and nothing is locked or started.
`Host::start(lock_directory, broker, store)` keeps the settings for `restart()` and builds the broker's environment
from the host's own without any `FABRIC_BROKER_*` name (compared without case on Windows), then the store's
variables. The trace's connected line gains ` key=<keychain|test>[ synthetic]`.

Relay (`apps/fabric-desktop/relay/src/config.rs`): `ENVIRONMENT_ALLOWLIST` holds the three broker variables (and
`SystemRoot` on Windows), built from the shared constants of 3.3.

`scripts/verify-desktop.sh`: unsets the OpenSSL variables before any broker build (section 8). Sources mode on macOS
adds the release check and the release configuration tests of the broker. `--packaged`: step 1 runs the process
tests and the relay round trip against the bundled broker (p04 there proves the store survives a kill). Steps 2, 3
and 5 start the host with `FABRIC_DESKTOP_DATA_DIR="$WORK/data" FABRIC_BROKER_KEY_SOURCE=test
FABRIC_BROKER_SYNTHETIC_AGENT=1 FABRIC_DESKTOP_TRACE=1`, the same directory every time, so step 3 meets the first
launch's lock and no launch opens the real store. Step 2 waits up to 20 s for a line matching
` <- notice sequence=[0-9]+ change=request ` and checks that the trace's notice sequences rise by one each, from
whatever the first one is. There is no cursor check (L1).

`scripts/build-desktop.sh` unsets the OpenSSL variables before the broker build; it still builds the broker in
debug, so the test key and the synthetic agent remain available in the packaged MVP (R5). `harness/serve.ts` makes
one temporary directory per harness run and passes the three variables. The banner in `ui/index.html` says the
broker keeps its state in an encrypted store, that no rule prover runs, and that synthetic requests appear only in a
test run with a scratch directory and the test key. `apps/fabric-desktop/README.md` describes the store and where it
lives, that restart keeps pending requests, the synthetic demo (the command of 1.4), D18, the Keychain identifiers,
and the SQLCipher BSD-3 and rusqlite MIT notices to ship in CD-W11.

---------------------------------------------------------------------------------------------------------------------

## 11. Risks and owner questions

### 11.1 Risks

- R1 Keychain from a child with an empty environment (no `HOME`) is unverified; M3 checks. Fallback: allowlist
  `HOME` for the broker and keep process tests passing an explicit data dir.
- R2 Ad-hoc-signed rebuilds create a new code identity, so the Keychain prompts once per rebuild (accepted, L3, Q3;
  M2 records it). Denial -> `keychain_refused`, never plaintext.
- R3 Linux/Windows compile of the target split and MSVC `-U` unverified (section 8).
- R4 Keyed backup (`sqlcipher_export`) was untested by SPIKE; s08 now proves it on macOS.
- R5 The packaged MVP is a debug build and contains the test-key and synthetic seams, inert without their variables;
  the synthetic agent also needs the test key and a scratch directory. The release broker is compiled and its
  configuration tested in release by `verify-desktop.sh`, but not run; CD-W11 must build release and check the
  shipped broker refuses `FABRIC_BROKER_KEY_SOURCE=test` and `FABRIC_BROKER_SYNTHETIC_AGENT`.
- R6 Halt-on-write-failure turns a full disk into a stopped broker (deliberate, D7).
- R7 Start-up time grows with history (full chain verification, `cipher_integrity_check`); a verified-head checkpoint
  can come later.
- R8 Global epochs cause extra re-proofs once Isabelle runs (bounded by `MAX_REPROOFS` per purpose).
- R9 Synchronous stdout: a host that stops reading stalls the broker, including agent traffic after CD-W06.
- R10 W03 fit: `TrustedFacts` follows L6 and L7, not W03's code. The gate writes each check theory as a plaintext
  `.thy` file holding calendar data into its work directory (THEORY 4 item 6); the wiring sets it to
  `<data_dir>/prover` and must delete each file after its check, or R13 breaks. W03's `used` flag must mean "the
  permitted change was applied" (5.5).
- R11 Merge conflict with CD-W03 in root `Cargo.toml` and `Cargo.lock`.
- R12 `-shm` is not encrypted (SPIKE 2: no row content observed).
- R13 Ledger payloads hold calendar content inside the encrypted file; audit deletion (CD-Q11h) needs a compaction
  design later.
- R14 The chain does not detect a whole-file rollback to an older copy.
- R15 Clock (D34): a backward wall-clock step counts as zero elapsed time, so lifetimes running across it end later in
  wall time by up to the step; a forward step ends them early (fail-closed).
- R16 A crash loses the count of each agent's open `busy` audit window (at most 60 s per agent).
- R17 `last_tool_call_at` is null after a restart until the agent's first call (D35; CCR-16). A session's
  `last_active_at` starts at its start, so a session with no call idles 30 min after its connect.
- R18 A re-ask (`proving_fresh_permit -> pending_approval`) may make a sixth open card for an agent (5.9).
- R19 A one-request grant whose request is `applied` or `outcome_unknown` stays live through W04 (no reconciliation
  until CD-W05) and shows in the snapshot as live (D18; desktop README).
- R20 A status poll is activity, so a polling agent keeps its session (and session grants) alive indefinitely.
- R21 A Permit with an empty `authorizing_grants` ends `undecided`; the grant-less release of a change's outcome and
  the broker's own reads (D33) need a separate path in a later package.
- R22 Replay is not version-aware: it applies this build's `apply` code to every earlier ledger row. A migration that
  changes a table `apply` writes needs version-aware replay first; until then migrations may only add (3.6). This
  must be settled before CD-W05's migration 2.
- R23 `session.last_active_at` and `agent.last_tool_call_at` change without a notice, outside the cursor (D35,
  CCR-17); a page sees them only in its next snapshot.
- R24 The owner-state budget (D38) covers recipient sets, profiles and live grants only. Accounts (CD-W05) and
  agents are not budgeted, and the open requests are assumed to fit; at extreme sizes `get_snapshot` answers
  `internal`: the broker runs on, but the page cannot load.
- R25 The Keychain framework's own copy of the key is freed but not wiped (no `unsafe`).
- R26 A Keychain store opened with the test key reports `WrongKey`, not `KeyKindMismatch` (3.2).
- R27 A real `SQLITE_IOERR`, `FULL` or `NOMEM` from this SQLCipher build was not produced in a test; the `Io` mapping
  is tested with an unreadable file and by code. On Linux and Windows the probe's `CANTOPEN`, not the plaintext
  check, is the path that fires, and it is not run there yet.

### 11.2 Questions only the owner can answer

They stay open for the owner; the code runs on the default after each (L3).

- Q1 RD-14: accept the single-device envelope of section 4 (actor field covered by a SHA-256 chain, not signed; no
  space, key epoch, HLC or signature until multi-device work)? Default: implemented as written; ADR-115 (Proposed)
  asks it.
- Q2 Keychain item: service `ai.paracels.fabric.calendar.store`, account `database-key-v1`? Default: these names.
- Q3 Developer builds: accept one Keychain prompt per ad-hoc rebuild, or sign debug builds with a stable identity
  (would add an optional `FABRIC_DEV_SIGN_IDENTITY` to `build-desktop.sh`)? Default: accept the prompt.
- Q4 Keep the debug-only synthetic agent (labelled, never permits, never dispatches) opt-in, or on by default in debug
  builds? Default: opt-in, and only with the test key and an explicit scratch data directory.
- Q5 Expansion baseline: previous version of the profile, Careful for a new one? Default: yes.
- Q6 A recipient-set edit that adds members, or a delete, ends the grants naming the set; narrowing applies at once.
  Accept? Default: yes.
- Q7 Activating a stricter profile ends no grant (every later proof applies it). Or end affected grants with
  `authority_reduced`? Default: ends none.
- Q8 May CD-W04 proceed without the missing owner commands (CCR-11), which RM 254 says need a decision "before CD-W04
  and CD-W08"? Default: proceed.
- Q9 Who wires `IsabelleProver` after CD-W03 merges? Default: a follow-up branch after CD-W03 merges, outside this
  package. (Packaging Isabelle is G1/Q09.)
- Q10 The store-half PLAN 14 row "poll a completed read after revocation" moves to CD-W05/W06 because W04 stores no
  results (D33). Accept for G2a? Default: accepted for this package; the owner's answer is needed before G2a is put
  to him.

### 11.3 Readings of the W03 interface

They do not wait on the owner; the code runs on the reading written here.

- W1 `Context.relied` for a fresh proof is the `authorizing_grants` of the request's previous accepted check, or the
  grant a just-made approval created. A result not accepted, for any reason (a report bound to another job, a changed
  fence, an authorizing grant invalid at acceptance, an id outside the job's facts, a late or unknown job), leaves the
  stored list unchanged (5.7, D36). If W03 means the previous check whatever its fate, `ReliedGrantSet` also goes into
  the commit that records a discarded result.
- W2 Decided: `Context`'s connected accounts hold every connected account and its calendars every selected calendar
  of those accounts, as L6 words it; they are not filtered to the accounts the action names. Per-account namespacing
  (3.5) holds because every calendar is named with its account. The filter would live in `service/facts.rs` alone if
  the theory ever wants it.

---------------------------------------------------------------------------------------------------------------------

## Appendix A. Inputs outside the repository

The code cites two working inputs that are not in the repository. What it relies on from them is here.

### A.1 SPIKE: the SQLCipher storage spike (2026-11-16)

- Option (c), chosen: `rusqlite =0.43.0` with `bundled-sqlcipher` only as the macOS target dependency (SQLCipher
  4.20.0 community, SQLite 3.54.1, Apple CommonCrypto, no OpenSSL), and the same rusqlite with `bundled` elsewhere,
  so schema, migrations and crash tests run on all three runners while encryption and key handling are macOS code.
- 1, build: links `Security`, `CoreFoundation` and a static `sqlcipher`; no OpenSSL. `OPENSSL_*` variables would
  switch the crypto provider.
- 2, encryption at rest: with a raw 256-bit key SQLCipher encrypts the main file and the WAL; temporary databases
  and sorter spill files are plaintext when temp storage goes to files, so `SQLITE_TEMP_STORE=3` at compile time and
  `temp_store = MEMORY`; `-shm` is not encrypted but held no row content; a wrong key is `SQLITE_NOTADB`. One fsync'd
  commit with `fullfsync = ON` costs about 5 ms.
- 3, crash: uncommitted writes are absent after `abort` and SIGKILL; committed writes survive both.
- 5, other platforms: plain bundled SQLite compiles the same schema; Linux and Windows were not built.
- Not tested by the spike: keyed backup (`sqlcipher_export`), now s08; `locking_mode = EXCLUSIVE` (D29).

### A.2 THEORY: the W03 theory interface as read for this package

- Section 1, the trusted context: the theory's context holds only facts the broker derived from its own records,
  nothing an agent or a model said; times are UTC ms; digests are bare 64 hex. Its fields are the left column of the
  10.3 table.
- Section 4 item 4: a proof id is a hash of the check theory text, and check theory names restart per gate
  instance, so a proof id is not unique by construction; it is never a primary key (`proof_reports_id` is not
  unique; `get_proof` takes the newest row).
- Section 4 item 6: the gate writes each check theory as a plaintext `.thy` file into its work directory (R10).
- Section 4 item 9: no digest is defined for the broker's own actions (CCR-14).
- Note B: the theory defines `session_idle_timeout_ms = 30 * 60 * 1000` and the gate does not read it back, so the
  broker's constant is kept equal by hand (5.5).
