# CD-W05 and CD-W06, phase 2: the store, the core and the broker binary

**What this is.** The design of phase 2 of work packages `CD-W05` (Google Calendar) and `CD-W06` (the agent channel)
of the Calendar Desktop plan, for the parts that the lead's session builds: the encrypted store
(`crates/fabric-storage`), the authority core (`crates/fabric-broker-core`) and the broker executable
(`apps/fabric-broker`). It is a plan, not a description of code: nothing here is built. It is step P1 of ruling P12
(documents only) and edits no other document; every amendment of an accepted document is written here as proposed
text and is applied in the step that builds it, after the owner's answer.

**What it answers.** Every request that the CD-W05 design (W05 sections 9 to 12: four amendments, PO1 to PO8, ST1 to
ST11, CO1 to CO14) and the CD-W06 design (W06 7.1 and 7.2: LR-S1 to LR-S4, LR-C1 to LR-C7, LR-B1 to LR-B6) make of
the store, the core and the binary, under the lead's thirteen rulings of 2026-11-17 (P1 to P13). Section F
gives the answer to each id in one table. Sections G and H are the requests this design makes of the contract
session, the screens session, the CD-W05 session and the CD-W06 session. Section I holds every open point, with
whose it is, a recommendation and, for each point the lead ruled on later on 2026-11-17, that ruling; E.3 holds the
owner's decisions (P13), which stay open except OD1, OD2 and OD24, answered 2026-11-18.

**How it is organized and how ids are cited.**

| Part | What |
|---|---|
| 0 | The decisions of the whole design in one table (0.2), where each shared shape is defined (0.3), the enum values that are stored or sent (0.4), the core types more than one section uses (0.5), and the numbers (0.6) |
| A | The store: the rule set for migrations, migration 2 in full, migration 3, content kept outside the ledger, fixtures |
| B | Results, held answers and proofs: the release rule, the lifecycles of a list, a read and a release, system proofs, the fence, order deadlines, question 10 |
| C | The agent channel: enrollment, the list permission, the handshake, sessions, the listener, the configuration text |
| D | The provider port and Google in the core: targets, proofs on orders, observation, reconciliation, accounts, calendars, the budget |
| E | The practice mode for a real agent, the order of pull requests, the owner's decisions, risks |
| F to I | The answer to every request id; requests to the contract session; requests to the other sessions; open points |

A section's decisions carry its letter: V1 to V23 are the design's shared decisions (0.2), AD for A, BD for B, CD for
C, DD for D, ED for E, and X1 to X25 are the decisions taken while the sections were made to agree (0.2). Other ids:

| Ids | Meaning |
|---|---|
| P1 to P13 | the lead's rulings of 2026-11-17 |
| step P0 to step P7 (P5a, P5b, P7a, P7b) | the pull requests of ruling P12, ordered in E.2.1; always written "step Pn" or named as steps |
| G1 to G9 | the branches of existing ledger variants gated on store version 2 (A.4.5) |
| I1 to I27 | the open invariants of the store (A.4.7, A.5) |
| A.2 R1 to R11 | the store's rules for migrations; inside section A written R1 to R11 |
| B.2 R1 to R6 | the release rule's invariants; inside section B written R1 to R6 |
| VC1 to VC4 | the validation checks of `fabric_request_result` (B.5.1) |
| A1 to A18 | the authority checks on the path of a read (B.15) |
| Q1 to Q21 | the cases of question 10 (B.16.1) |
| P2-1 to P2-34, CQ-D1 to CQ-D4 | the requests and questions to the contract session (section G) |
| SR-n, W5-n, W6-n | the requests to the screens, CD-W05 and CD-W06 sessions (section H) |
| OD1 to OD26 | the owner's decisions (E.3) |
| OP, AO, BOP, OPC, CQ-D, OQ-D | open points of the shared vocabulary and of sections A, B, C and D, collected in section I with the lead's rulings on them |
| ER1 to ER14 | the risks of the plan (E.4) |
| ct, bt, h, r, f, o, p, s, m, e, v, Q | test ids of sections A to E |
| PW practice rule P1 to P7 | the practice rules of PW 4.1 (not the rulings) |

**Not merged yet, and designed on top of.** None of this is relied on in its exact code.

| Work | Where | What this design takes from it |
|---|---|---|
| Step P0, part one: the broker part of contract schema version 2 | branch `claude/cd-broker-prover-reasons` (`07625b1`, `a73b786`) | `ports::ProverStatus`, `ports::ProverCause`, `ProofDone.cause`, one function `undecide()` as the only way into `undecided`, the cause stored in an optional member of `RequestMoved`, a tolerant reader |
| Step P0, part two: the duration rule | not started in the lead's chain; the page's part is `claude/cd-w08-page-duration-ceiling` (`0977f69`) | the card's proposed duration computed in the core, a longer one refused, approval null while a request waits again |
| The contract draft of schema version 3 (pull request 36) | `origin/claude/contract-v3-owner-commands`; read at `29cdfdf`, now at `b94b58f` | enrollment, the handshake, the owner commands, `needs_owner_resolution`, `resolved_as_*`. Since `29cdfdf` the draft added the state `completed` (P2-1), the stored-values test of P2-14 and the `I1` edit for `completed` (`b94b58f`), and the meaning of a null approval (`63e8018`) |
| CD-W05 phase 1 | `origin/claude/cd-w05-google-provider-27ca35` (`e0196b3`) | the W05 design; `crates/fabric-google-calendar` and `crates/fabric-google-fake` |
| CD-W06 phase 1 | `origin/claude/cd-w06-mcp-bridge-f95843` (`2f1d746`) | the W06 design; `apps/fabric-mcp-bridge` and its `config_text` |

**Who decides.** The lead's rulings bind this document, and the contract draft wins on the wire and on stored
contract types (P1). Where a ruling cannot be built as written, the section says so with file and line and gives the
nearest form that can; section I lists each such case. Every decision that is the owner's is in E.3 with the default
the design runs on. The owner answered OD1, OD2 and OD24 on 2026-11-18, each with the default (PLAN 15.3).

**The lead's rulings on section I.** On 2026-11-17, after reading section I, the lead ruled on the points that are
the lead's; each ruled row of section I says "Ruled 2026-11-17" and what was chosen, and the sections state the
ruled form. Four of these rulings change a first ruling. OP1 (with AO2, A-13 and E-11) keeps observations and the
event columns of `effects` in migration 2, so migration 3 holds only `provider_accounts` and `provider_calendars`;
this changes P3's list. E-10 starts step P4 after step P3 merges (E.2.3); this changes P12's "P4, in parallel with
P3". E-9 puts observe, reconcile, the fence fix and the order watchdogs into step P5, as P5a then P5b; this changes
P12's content of P5. D-10 lets a delete found at another revision count as absent after one read; this is the one
exception to P10's two reads. The owner's points in E.3 (OD1 to OD26) stay open except OD1, OD2 and OD24, answered
2026-11-18.

**The answers of the CD-W06 session.** On 2026-11-18 the CD-W06 session accepted W6-1 to W6-9 and sent three
corrections, two points for the lead and one question; the lead ruled on them the same day (I.8). The rulings correct
the bridge's words for `completed`, `applied` and `reconciled` (B.3.1, H.3) and for a handshake `not_permitted` that
means "wait" (C.6.4), add request P2-30 (a cancel message, B.8.4) and test h16, settle the merge order of pull
requests 36 and 38 (E.2.4), and tell the owner the residual of a long user name (OD22).

**The CD-W05 session's five points.** On 2026-11-18 the lead ruled on the five points the CD-W05 session raised on
this design (a delete's read-back, a yes with other content, the evidence bound, spans, the account link), and the
session accepted all five as written (I.9). Each is a ruled row of I.5 (D-13 to D-17), the requests to the session
are H.2's W5-11 to W5-15, and the sections state the ruled form (V9, V23, DD17, DD18, B.3.4, D.2, D.6.2, D.7,
D.8); two of them add owner decisions, OD25 and OD26.

Date 2026-11-17. Status: Proposed. Code read at main `2d41210`.

---------------------------------------------------------------------------------------------------------------------

## 0. Decisions, names and numbers

### 0.1 Short names

Paths are relative to the repository root. Code is cited at main `2d41210`.

| Short | Means |
|---|---|
| W04 | `docs/plans/calendar-desktop-w04-design.md` |
| PW | `docs/plans/calendar-desktop-prover-wiring-design.md` |
| ADR-115 | `docs/adr/ADR-115-broker-ledger-envelope.md` |
| PLAN | `docs/plans/calendar-desktop-plan.md` (section 15.2 holds the owner's decisions of 2026-11-17, section 15.3 those of 2026-11-17 evening and 2026-11-18) |
| W05 | `docs/plans/calendar-desktop-w05-design.md` on `origin/claude/cd-w05-google-provider-27ca35` |
| W06 | `docs/plans/calendar-desktop-w06-design.md` on `origin/claude/cd-w06-mcp-bridge-f95843` |
| DRAFT | the contract draft of schema version 3, `origin/claude/contract-v3-owner-commands` at `29cdfdf` (pull request 36); "README" in sections B to E means its `contracts/desktop/v1/README.md` |
| LC | DRAFT `contracts/desktop/v1/data/request-lifecycle.json` |
| REV-port, REV-store, REV-core, REV-results, REV-order | the five reviews of the CD-W05 requests, cited by item id. A working file of the lead's session, not in the repository |
| PART-grants, PART-results, PART-agents, PART-practice | the four parts of the first design of the agent channel. A working file of the lead's session, not in the repository |
| J-store, J-auth | the two judgements of those parts (store and coherence; authority and flow). Same file |
| ST | `crates/fabric-storage/src/` |
| CORE | `crates/fabric-broker-core/src/` |
| BR | `apps/fabric-broker/src/` |
| CC | `crates/fabric-desktop-contracts/src/` |
| GC | `crates/fabric-google-calendar/` (CD-W05 branch) |
| FAKE | `crates/fabric-google-fake/` (CD-W05 branch) |
| THEORY, THY | `isabelle/Fabric_Broker/Fabric_Broker.thy` |
| EX | `isabelle/Fabric_Broker_Examples/Broker_Examples.thy` |
| P0 | step P0 of ruling P12: the broker part of contract schema version 2 and the duration rule (head of this document) |

### 0.2 Decisions

One row per decision. The text of each is in the section named; where a row was changed while the sections were
made to agree, the X row says how.

| Id | Decision | Where it is explained |
|---|---|---|
| V1 | Observations (W05 ST3) and the event columns of `effects` (what remains of ST5) are in migration 2; migration 3 holds `provider_accounts` and `provider_calendars` only. P11 needs the practice provider to answer observe and reconcile before step P6, and P10 needs every observation stored before a proof reads it. Departs from P3's list; ruled by the lead 2026-11-17 (OP1) | A.1 AD3, A.4; I.1 OP1 |
| V2 | ST5 is reduced to `effects.notification` beside P4's two columns; `revision_before`, `confirmed`, `reconcile_reads`, `next_reconcile_at_ms` are dropped (the authorization's fence, V9, `observations` and `requests.due_at_ms` hold them) | A.4.1, A.9 ST5 |
| V3 | A held result's digest is kept only in `held_result_payloads` (not projection); no ledger payload names a digest of provider content, because it would outlive the purge and let a key holder test a guess | A.7, B.4.3 |
| V4 | A held result is purged at its first release, whatever grant released it (cause `released`) | B.9, BD9; open OP2 |
| V5 | A read whose result fails the authority check at the storing commit, or that the provider answered `Stopped`, goes `dispatching -> retry_wait` and stores nothing | B.3.4, BD8; ruled 2026-11-17 (OP3) |
| V6 | The storing commit of a read checks the whole fence, the read authorization's grants and the pause, as the handoff does; measured on the practice calendar in step P5 | B.3.4; ruled 2026-11-17 (OP4, BOP3) |
| V7 | No read after an applied change: a confirmed `Applied` moves `dispatching -> applied -> reconciled` in the attempt's commit. W05 ST7 is not built; the theory's pending effects are unchanged. A delete is confirmed by the adapter's one GET of the event inside the delete's own dispatch order, after a 200 or 204 (D.2); that GET is part of carrying out the order, not a read after an applied change | B.14, D.7.1 DD8, D.2; ruled 2026-11-17 (OP5; W5-6, agreed by the CD-W05 session 2026-11-18); the delete's GET ruled 2026-11-18 by the lead |
| V8 | System proofs are a job of their own (`Job::ProveSystem`), never routed through `accepted()`, filed in `system_proofs` | B.11, A.4.1 |
| V9 | An `Applied` whose event does not fit the order (another id; for a create an id other than the reserved one; an event on a delete, or none on a non-delete) is handled as `NoAnswer`, with one audit row `provider_mismatch`. An `Applied` delete that is not confirmed is handled as `NoAnswer` with no audit row. A fitting, unconfirmed `Applied` of a create, update, invitation or response follows V23 | B.14 BD16, D.7.1; ruled 2026-11-18 by the lead |
| V10 | A list or release whose call no longer waits is cancelled (`CallGone`): from `proving_fresh_permit` at the release, and from `received`, `validating`, `proving_classification`, `allowed` and `approved` as soon as its held answer left unsettled or was dropped; recovery cancels every open one. Nothing is built, no release row is written, the operation type is `request.proof` or `request.advance` | B.1 BD3, BD4, B.4.1 |
| V11 | `AgentInput::Connect` and `AgentInput::Call` exist only under `cfg(debug_assertions)`; a release build reaches a session only through `agent_prove` | C.1 CD12, C.11 |
| V12 | A frame before an approved handshake gets `not_permitted`, then the connection is closed and counted `frame_before_hello` | C.1 CD11, C.6.2 |
| V13 | A seeded debug agent has an `agents` row and no `enrollments` row and counts as approved; the debug seed gives it the list grant through the helper of `approve_agent`. It has no key: the handshake refuses it `not_permitted` (counted `unknown_id` with its id), `approve_agent` answers `conflict`, `remove_agent` `invalid_params` at `/params/agent_id`, `get_agent_configuration` `not_found` | C.1 CD1, CD6, C.3.4, C.6.4 row b, E.1.3 |
| V14 | The contract revision of an agent is `enrollments.revision + agents.revision` (a missing row counts 0) | C.1 CD2, C.2.1 |
| V15 | The read one-request grant ends `consumed` at its release; at a purge `account_disconnected` or `agent_removed` when that caused it; at the 24 h expiry `result_expired` once P2-4 merges, `consumed` until then | B.7.1, A.4.8; open OP8 |
| V16 | `provider_accounts` gets its row at `connect_google`, with `subject` null until the first `Connected` outcome, under a partial unique index | A.5, D.8.1 |
| V17 | `can_carry_out` replaces `Attachments.provider_present`: true for (`Practice`, no `provider_accounts` row) and (`Google`, a `provider_accounts` row), false otherwise | D.12, E.0 ED7, 0.5.3 |
| V18 | `backup::take` writes the copy under a `.partial` name, deletes the rows of `held_result_payloads` in it when the copy has that table (its version argument, ST `backup.rs:25`, is at least 2; a version 1 copy gets no delete), verifies it, then renames it; it first removes every file in `backups/` whose name contains `.partial`: the copy and its `-journal`, `-wal` and `-shm` files | A.7; open OP14 |
| V19 | Reconcile read-backs and observations are one table, `observations`, with `purpose` `observe` or `reconcile`; the PD20 read count is the count of `reconcile` rows | D.7.3, A.4.1 |
| V20 | Order answers carry the order's `order_id`, unique in the run; every order kind has a core watchdog | B.13, D.5 |
| V21 | The list grant's helper is `grants::create_enrollment_grant(tx, cx, agent_id)` | C.4.1 |
| V22 | `enrollment_grants` allows one row per agent ever; after a revoke the permission comes back only through a card, into `grants` | C.4.1, A.4.1 |
| V23 | A fitting `Applied` of a create, update, invitation or response that is not confirmed is applied and differs from the approval. It is never sent again, never reconciled by reading, and can never end `absence_confirmed`. Its commit records the event's id and revision (`EffectEventRecorded`), writes one audit row `provider_differs` and moves the request to `needs_owner_resolution`, which the owner can resolve only as applied. `confirmed` means equal to the approval after only the normalisations of a closed, named list in the adapter, empty until the live run shows each one (W05 6.12, 18.7, LV18). The event as Google returned it is the attempt's evidence, at most `MAX_EVIDENCE_BYTES`: past it the id and revision are still recorded, the request still waits for the owner and no evidence row is written; it is never refused, because the change has happened. A delete never follows this row (V9). Until P2-31 to P2-34 merge it is reachable only by a practice fault; they are needed before any Google dispatch | B.14, D.7.1, D.7.3; E.3 OD25; section G; ruled 2026-11-18 by the lead |
| AD1 | Migrations 2 and 3 are additive under the rules A.2 R1 to R11; replay does not become version-aware now | A.1, A.2 |
| AD2 | Migration 2 is one file, `0002_agents_and_results.sql`, landed by the lead after contract schema version 3 merges (step P3) | A.1, A.4 |
| AD3 | Observations and the event columns of `effects` are in migration 2 (V1) | A.1; ruled 2026-11-17 (AO2) |
| AD4 | Every branch of an existing variant that touches a table of migration 2 is gated on `user_version >= 2` and named, G1 to G9 | A.4.5 |
| AD5 | The apply of `HeldResultPurged` deletes the payload row; there is no `WriteTx::delete_held_payload` | A.4.3 |
| AD6 | `Store::write` commits nothing when a `HeldResultStored` has no payload row or an `ObservationRecorded` names missing evidence | A.4.4 |
| AD7 | No projection table has a foreign key to a non-projection table | A.4.1 |
| AD8 | `ResultReleased` and `HeldResultStored` check in the store that the proof's or authorization's fence epochs equal the counters of the commit | A.4.3 |
| AD9 | A grant id is unique across `grants` and `enrollment_grants` (G4 and invariant I24) | A.4.5, A.4.7 |
| AD10 | A new store runs each migration in its own transaction, so a new version 2 store has two creation rows | A.8.4 |
| AD11 | Open invariants carry `since` and `until`; `check_invariants` skips an entry outside the store's version | A.4.7 |
| AD12 | The frozen version 1 stores are text dumps made by the version 1 code at two commits, plus one SQLCipher file made on macOS | A.8.1 |
| AD13 | After a commit that deletes held payloads, `PRAGMA wal_checkpoint(TRUNCATE)` (`Store::checkpoint`) | A.7; open AO4 |
| AD14 | `owner.resolve_outcome` carries the request id as `correlation_id` | A.6 |
| AD15 | `agent.prove_key` rows have actor `agent` with the enrollment id | A.6, C.6.4 (X2) |
| BD1 | The release rule is six invariants, B.2 R1 to R6, each tested | B.2 |
| BD2 | A list and a release end `proving_fresh_permit -> completed` in one commit `request.release`, with no authorization and no `effects` row | B.1, B.4.1 |
| BD3 | A release happens only while `Held::waits_for` is true in the step of the release commit; otherwise `cancelled` | B.1, B.4.1 |
| BD4 | A list or release whose held answer left unsettled or was dropped is cancelled at once from six states; recovery cancels every open one | B.1, B.3.7 |
| BD5 | A list approved through a card ends `cancelled`; the agent lists again; a one-request approval of a list delivers nothing, so under `ask` no list completes in version 1 | B.1; ruled by the lead for version 1 2026-11-17 (BOP1); owner OD24 answered 2026-11-18: accepted for version 1 |
| BD6 | A read's own answer never carries its result; the agent calls `fabric_request_result` | B.1; owner OD1 answered 2026-11-18: two calls |
| BD7 | A read is authorized and handed off like a change, with `ReadIntent` and `read_attempts` in place of `effects` | B.3.3 |
| BD8 | The storing commit re-checks authority; a failed check, `Stopped`, a transient failure and the watchdog go `retry_wait` under `retry_due` | B.3.4 |
| BD9 | Every purge ends the read's live one-request grant in the same commit | B.7.1, B.9 |
| BD10 | `result_held` is a predicate read with the facts; the payload is read only in the release commit | B.5.3 |
| BD11 | The outcome of a change is released with no grant, only to its agent, only while applied or reconciled with a recorded event, under every block rule | B.6; owner B.18 |
| BD12 | `fabric_request_result` never opens a card in version 1 | B.3.5; owner OD2 answered 2026-11-18: no card |
| BD13 | A call that would create a request is refused `prover_unavailable` while the prover is not ready | B.10, C.8 |
| BD14 | `Fence` keeps its shape; `fence_now(counters, observed_revision)` puts the stored observed revision into the current fence | B.12, D.10 |
| BD15 | Every provider order carries `order_id` and a deadline; the core acts at `deadline_ms + ORDER_WATCHDOG_MARGIN_MS`; the worker never starts an order past the monotonic bound the shell stamped on it (X23) | B.13, D.5 |
| BD16 | An unfitting `Applied`, or an unconfirmed delete, is `NoAnswer` (V9); a fitting, unconfirmed `Applied` of a create, update, invitation or response follows V23 | B.14 |
| BD17 | System proofs are accepted by their own rule and recorded whatever their outcome, with operation type `system.proof` | B.11 |
| CD1 | The contract `Agent` is built from the `enrollments` row and the `agents` row; a pending agent has no `agents` row | C.2.1 |
| CD2 | The contract revision is the sum of the two rows' revisions (V14) | C.2.1 |
| CD3 | `add_agent` sets `destination` from the client by a fixed table in the core (P2-22) | C.3.1 |
| CD4 | `approve_agent` refuses in a fixed order, then one commit: `AgentPut`, `EnrollmentPut` approved, the list grant, notices `Agent` then `Grant` | C.3.2 |
| CD5 | `remove_agent` follows P4's order and raises `revocation_epoch` at least once | C.3.3 (X1) |
| CD6 | A seeded debug agent cannot be approved or removed | C.3.2, C.3.3 |
| CD7 | Every decision of the handshake is in the core, `CORE service/handshake.rs` | C.6 |
| CD8 | `verify_strict` of `ed25519-dalek` 2.2.0; only a `Proven` value opens the store read | C.6.4 |
| CD9 | While pending, the same key proved again writes nothing; another key at most once a minute and 20 times in all | C.6.4 |
| CD10 | Successful handshakes are limited to 6 per agent in 60 s | C.6.4 row e |
| CD11 | A frame before an approved handshake is answered and closed (V12) | C.6.2 |
| CD12 | `Connect` and `Call` are debug-only (V11) | C.11 |
| CD13 | The core keeps the ids it closed until their `Disconnect` | C.6.2 |
| CD14 | A challenge has a deadline on the core's clock | C.6.3 |
| CD15 | The listener binds only while the store is open, under its lock | C.9.1 |
| CD16 | The peer's user id is checked on every connection through `nix` | C.9.3; open OPC1 |
| CD17 | The binary's handshake counts change only on the main thread, except the accept thread's increment | C.9.2 |
| CD18 | One writer thread per connection, 32 frames and 4 MiB | C.9.6 |
| CD19 | At quit the main thread waits at most 500 ms for the writers | C.9.6 |
| CD20 | `prover_unavailable` while the prover is not ready; status always answered | C.8 |
| CD21 | A list request has a scope that names no calendar, so it can get a card; a grant from such a card has purpose `""` | C.4.2 |
| CD22 | `get_agent_configuration` is answered through `ConfigurationTexts`, which calls the bridge crate's `config_text` | C.10; ruled 2026-11-17 (OPC2); owner OD21 open |
| CD23 | The snapshot carries every agent; `add_agent` counts the agents' records into the owner budget | C.3.1 |
| CD24 | Listener connection ids start at `1 << 32` | C.9.2 |
| DD1 | Every provider order that names a calendar carries one `CalendarTarget` (a read order a list of them), resolved by the core in the sending commit; a list order names its account only | D.3 |
| DD2 | Every order carries an `OrderProof`; the worker refuses a digest that is not its own | D.4 |
| DD3 | Every order has an id and a deadline; the core's watchdog ends it | D.5 |
| DD4 | Before a proof on an action that names an event, the core observes it under a system proof and stores the observation | D.6 |
| DD5 | `fence_now` with the request's stored observed revision; `fence_holds` stays equality | D.10 |
| DD6 | `Transient` means nothing happened; `NoAnswer`, a 5xx, an unfitting `Applied` and an unconfirmed delete are unknown outcomes; a fitting `Applied` of a create, update, invitation or response, confirmed or not, means it happened (V23) | D.7.1 |
| DD7 | Reconcile answers report what the provider holds; the core owns the table; two reads 60 s apart before absent | D.7.3 |
| DD8 | No read after an applied change (V7) | D.7.1 |
| DD9 | Account commands and commits raise `revocation_epoch` once when the account record changes; one test pins `NARROWING_COMMANDS` and `NARROWING_COMMITS` | D.9.4 |
| DD10 | `save_preferences` raises `revocation_epoch` when `warning_calendars` change | D.9.4 |
| DD11 | Every account counts at `ACCOUNT_RESERVE_BYTES`; a connect past the budget is refused | D.11; owner OD4 |
| DD12 | "Fabric created this event" is read from the store | D.0, D.7.3; owner OD6 |
| DD13 | Google is attached only when reconciliation in the core is merged and tested | D.0, E.2.1 |
| DD14 | The practice provider implements the whole port before a real agent uses it | D.12, E.1.4 |
| DD15 | An `AuthLost` is applied even when its result is ignored | D.8.2 |
| DD16 | The broker's own reads are proven; a dispatch order's own GET of its target event is not a broker read (D.4) | D.0, D.4; owner OD7 |
| DD17 | A read answer's span is typed (`ReadSpan`: timed in milliseconds, or all-day with Google's two dates); the core alone widens an all-day span by `ZONE_SLACK_MS` for its overlap check and stores none | D.0, B.3.4, D.2; ruled 2026-11-18 by the lead |
| DD18 | A sign-in is superseded only under the lock of its token write; a disconnect's revoke and forget are one `unlink` job on a worker of its own, repeated at start while a revocation is pending | D.0, D.8; ruled 2026-11-18 by the lead |
| ED1 | Practice is on exactly with a debug build, the test key, Isabelle and no Google provider | E.1.1 |
| ED2 | The test key with Isabelle and a Google provider is refused, exit 78 | E.1.1; ruled 2026-11-17 (I.6 E-1) |
| ED3 | The shell seeds the practice account, twice in the first ready turn | E.1.3 |
| ED4 | The practice provider answers the whole port in step P4; the shell's hook goes in step P5 | E.1.4 |
| ED5 | The provider-neutral core of CO6 and CO8, the fence fix and the order watchdogs are built in step P5 | E.0, E.2.1 |
| ED6 | The practice provider's dispatch follows P10: `Transient` only when nothing changed, notifying changes refused, a new fault `unknown_not_applied`; row 4 removed | E.1.4 (X6) |
| ED7 | The practice target and `can_carry_out` for an account with no `provider_accounts` row in a practice run | E.0, D.3 |
| ED8 | `ProviderKind::Practice` exists only in debug builds | E.1.7 |
| ED9 | Labels for the owner and the agent; P2-19 asked | E.1.6 |
| ED10 | `<data_dir>/bridge-keys` (0700) in debug builds with the test key, named in the configuration text | E.0, C.9.1, C.10 |
| ED11 | A practice run is never evidence for gate G2b or G2c | E.1.11 |
| ED12 | The amendments of E.1.9 are applied in the step that builds them, after the owner's answer | E.1.9 |
| X1 | The removal order is C.3.3's seven steps. A.4.8 had six and one notice: step 1 already sends an agent notice (CORE `service/sessions.rs:102-104`), and a pending agent ends no grant, so P10 needs a raise of `revocation_epoch` when nothing ended. A.4.8 now matches C.3.3 | A.4.8, C.3.3 |
| X2 | `agent.prove_key` is committed with actor `agent` and the enrollment id (AD15), not `broker`. ADR-115:42 names the agent id "the agent channel authenticated", which the verified signature does; the ledger has no foreign key on `actor_agent_id` (`crates/fabric-storage/migrations/0001_initial.sql:20, 25`). C.6.4's note changed. Closes AO6 | A.6, C.6.4 |
| X3 | The payload delete belongs to the apply of `HeldResultPurged` (AD5). B.4.1 step 5 no longer names `delete_held_payload` | A.4.3, B.4.1 |
| X4 | The fence fix lands in step P5 (part P5a), not step P4: no code writes an `observations` row before step P5, and E.2.1 and D.10 say so. B.12 changed | B.12, D.10, E.2.1 |
| X5 | Contract request numbers: section C's P2-21, P2-22 and P2-23 keep theirs; section E's optional capability request, also numbered P2-21, becomes P2-24; the editor adds P2-25 (the `I1` edit and the R7 records) and P2-26 (version 2 agent records and stored `pause_agent` answers, found in DRAFT at `b94b58f`) | section G |
| X6 | Practice dispatch row 4 ("an effect applied before answers the same event") is removed, as D.12 says, against ED6's first form. Under DD6 the core never sends a change again; with row 4 a resend by mistake would look like success, without it the resend meets row 7 and is refused as at Google. Neither form repeats a change | D.12, E.1.4 |
| X7 | A practice update or delete of a missing event answers `revision_conflict` (D.12; W05 6.7, 6.8: 404 and 410), not `invalid_arguments`. E.1.4 row 7 changed | D.12, E.1.4 |
| X8 | The account budget is D.11's: a reserve of 96 KiB per account, 7 accounts with no other records. E.3 OD4's "about 60 KiB, twelve accounts" is replaced | D.11, E.3 OD4 |
| X9 | Operation types: 34 at store version 2, 39 at version 3, 40 only if P2-11 adds `owner.refresh_calendars`. E.3 OD5 and step P7a changed | A.6, D.16 |
| X10 | Gated branches are G1 to G9 everywhere (E.2.1 step P3 and section C's head said G1 to G6) | A.4.5 |
| X11 | The text of `prover_unavailable` is B.10's ("The rule checker is not ready. Nothing was recorded. Try again later."); C.8 changed. LR-C3 is answered in C.8, and B.10 says where the check sits | B.10, C.8 |
| X12 | P2-19 (`practice: bool` in the prove result) is asked, not optional (ED9) | section G |
| X13 | `CallGone` leaves six states, not one (BD4); 0.4.5 changed | B.3.7, 0.4.5 |
| X14 | `Held::waits_for` takes the connections and the activity, and `forget_connection` returns the dropped entries (B.8.1); 0.5.1 points there | B.8.1 |
| X15 | A release's proof is a fresh proof with purpose `fresh_permit` (B.3.5, B.4.1; `ProofPurpose` has two values, CC `owner.rs:1058-1061`), so the `ResultReleased` guard of A.4.3 stands. Closes section A's question to B | A.4.3, A.11 |
| X16 | The core code that reads `grant.origin.request_id` (CORE `service/pipeline.rs:276`, `service/requests.rs:49`, `service/facts.rs:38, 70`) learns `GrantOrigin::Enrollment` in step P5b, with `create_enrollment_grant`, so that a list can be authorized in step P5; C.4.1 says so | C.4.1, A.4.6, E.2.1 |
| X17 | Row 8 of E.1.2 (debug, test key, no Isabelle, Google) is accepted as D.13 sets the variables: the client file in every build, the loopback port only in debug with the test key. With no Isabelle nothing is permitted | D.13, E.1.2 |
| X18 | Section D has no table of steps; E.2.1 is the only order of work, and D.13's "step P7" means step P7b. D's question where reconciliation goes is answered by E.2.1 (step P5) | E.2.1 |
| X19 | Three audit kinds are new and need no migration: `handshake_failed` (C.6.5), `provider_mismatch` (B.14) and `provider_differs` (B.14, V23; added by the lead's ruling of 2026-11-18); A.4.2 names all three | 0.4.8 |
| X20 | The draft moved during this design, from `29cdfdf` to `b94b58f`. Citations of DRAFT stay at `29cdfdf`; section G marks what `b94b58f` already meets | head, section G |
| X21 | No store row type names a core type. `SystemAction`, `SystemActionKind`, `ObservedEvent`, `Notification`, `ConnectProblem` and `AuthLostCause` are defined in ST `rows.rs` with serde, snake_case, shipped in step P3; CORE `ports.rs` re-exports them, and `ObservedEvent` moves there from CORE `ports.rs:76`. The storage crate depends only on the contract crate (`crates/fabric-storage/Cargo.toml:9`) and the core depends on storage (`crates/fabric-broker-core/Cargo.toml:10`), so a row type cannot name a core type; `ObservedEvent` has no serde form today (CORE `ports.rs:75`). Step P4 therefore needs step P3 merged | 0.5.1, 0.5.2, A.4.3, E.2.1 |
| X22 | A request of a removed agent that would move `dispatching -> retry_wait` (a `Transient` change, a read whose storing check fails or that the provider stopped, recovery) moves on to `cancelled` in the same commit (reducer event `AgentRemoved` from `retry_wait`). The other form, `dispatching -> terminal_failure` (`retry_budget_exhausted`), was not chosen: the cause would be false, and `cancelled` is what removal does to every request not yet handed over (C.3.3 step 3). `Transient` means the change certainly did not happen (P10), so this is no false rollback | C.3.3, B.3.3, B.3.4, D.7.1, A.4.8 |
| X23 | The worker bounds an order by a monotonic stamp the shell takes when it takes the job from `Output.jobs`, not by `deadline_ms` on the core clock, which runs ahead of the raw clock by an offset that only grows (CORE `service/core.rs:211-238`). `deadline_ms` stays in the order for the record and the core's watchdog | B.13, D.2, D.5, E.1.4 |
| X24 | A dispatch the worker did not start in time answers `Transient { retry_after_ms: None }`, because nothing was sent (DD6, D.5, D.12, E.1.4 C1); B.13 said `NoAnswer` | B.13 |
| X25 | The owner's decisions are OD1 to OD26: OD15 to OD24 collect the owner rows that sections B, C, D and I named without an OD, and the list approved for one request (BD5); OD25 and OD26 come from the lead's rulings of 2026-11-18 on the CD-W05 session's points (V23, the evidence bound) | E.3, section I |

### 0.3 Where each shared shape is defined

Each shape is defined once; the other sections point to it.

| Shape | Defined in |
|---|---|
| The rules for migrations (A.2 R1 to R11) and the amendments of ADR-115 and W04 | A.2, A.3 |
| The tables of migration 2 (SQL, columns, what writes and reads each) | A.4.1, A.4.2 |
| The tables of migration 3 | A.5 |
| New mutation variants and their apply guards | A.4.3 (migration 2), A.5 (migration 3) |
| Existing variants and the gated branches G1 to G9 | A.4.5 |
| Store calls outside `apply`, the commit check, `Store::checkpoint` | A.4.4 |
| The readers that see both grant tables | A.4.6 |
| Open invariants I1 to I27 | A.4.7, A.5 |
| Commit orders: `owner.approve_agent`, `owner.remove_agent`, `request.read`, `request.release`, `timer.result_purge`, `request.attempt`, a read's `request.dispatch` | A.4.8; the removal and approval also in C.3.2, C.3.3 |
| Ledger operation types 1 to 40 | A.6 |
| Content kept outside the ledger, its deletion, what never enters the store | A.7 |
| The release rule and the lifecycles of a list, a read, a release | B.2, B.3 |
| Held answers (`Held`) | B.8.1 (and `HeldAnswer` in 0.5.1) |
| Agent ports (`AgentInput`, `AgentReply`, `Output.bound`) | C.5 |
| The provider port (`CalendarTarget`, the orders, the answers, `Provider`, `AccountLink`, `ConnectOutcome`) | D.2 |
| `Job` and `Input` | 0.5.4 |
| The configurations of the binary | E.1.2 |
| The order of pull requests | E.2.1 |

### 0.4 Enum values that are stored or sent

#### 0.4.1 `SessionEndReason` (ST `mutation.rs:258`), stored text

`disconnected`, `replaced`, `idle_timeout`, `app_quit`, `broker_restart`, **`agent_removed` (new)**.
`grant_end_reason(AgentRemoved) = GrantEndReason::AgentRemoved` (P4).

#### 0.4.2 `VoidReason` (ST `mutation.rs:290`), stored text

`expired`, `broker_restart`, `fence_changed`, `grant_invalid`, `agent_paused`, **`agent_removed` (new)**.

#### 0.4.3 `GrantEndReason` (contract, CC `owner.rs:536`)

No value is added by phase 2 except the request P2-4: **`result_expired` (proposed)**. The enrollment grant ends
`revoked_by_owner` or `agent_removed`; the read one-request grant as V15.

#### 0.4.4 Undecided causes (join P0's cause type; stored in `RequestMoved`'s optional member)

| Cause (stored text) | When | Contract `UndecidedReason` shown |
|---|---|---|
| `observation_failed` | an observe failed and the CO6 table says `undecided` (D.6.3) | `provider_unreachable` after P2-7; `failed` until then |
| `result_not_held` | the release commit finds no held result or no event (purged or mismatched since the proof) | `failed` |
| `result_too_large` | the answer with the result would pass `MAX_FRAME_BYTES` | `failed` |
| `release_not_authorizable` | a release permit with no authorizing grant for a read or a list, or naming a grant for a change's outcome (B.6.1) | `failed` |

The four values are added to the stored cause type in the step P3 pull request and first written in step P5 (A.2 R6,
A.4.5).

#### 0.4.5 Request states and reducer events

DRAFT's 25 (LC; CC `lifecycle.rs:40`): `received`, `validating`, `proving_classification`, `allowed`,
`pending_approval`, `blocked`, `undecided`, `approved`, `denied`, `expired`, `cancelled`, `stale`,
`proving_fresh_permit`, `queued`, `dispatching`, `applied`, `reconciled`, `retry_wait`, `outcome_unknown`,
`terminal_failure`, `reconciling`, `absence_confirmed`, `needs_owner_resolution` (open in version 3),
`resolved_as_applied`, `resolved_as_absent`.

Added (P2-1, in DRAFT since `b94b58f`): **`completed`**, terminal, one edge `proving_fresh_permit -> completed`: a
request the broker completes itself (a list, a release). 26 states, 42 transitions. Invariant I1 gains it in the
same change (P3; A.4.7).

Reducer events added by phase 2 (core only, not stored):

| Event | From | To | Owner |
|---|---|---|---|
| `Release` | `proving_fresh_permit` | `completed` | B.4.1 |
| `CallGone` | `received`, `validating`, `proving_classification`, `allowed`, `approved`, `proving_fresh_permit` | `cancelled` | B.3.7 (BD3, BD4) |
| `ReadStored` | `dispatching` | `applied`, then `reconciled` | B.3.4 |
| `ReadRetry` | `dispatching` | `retry_wait` | B.3.4 |
| `AgentRemoved` | `received`, `validating`, `proving_classification`, `allowed`, `approved`, `proving_fresh_permit`, `queued`, `retry_wait`, `pending_approval` | `cancelled` | C.3.3; also from `retry_wait` in the commit that moves a removed agent's request `dispatching -> retry_wait` (X22) |
| `ObservedStale`, `ObservedInvalid` | `validating`, `proving_fresh_permit` (and `pending_approval` for CO12, deferred) | `terminal_failure`, `stale` | D.6.3 |
| `ReconcileDue`, `ReconcileVerdict(Verdict)` | `outcome_unknown`, `reconciling` | `reconciling`, `applied`, `absence_confirmed`, `needs_owner_resolution` | D.7 |
| `AccountGone` (reused for a calendar) | `pending_approval` | `stale` | D.9.2 |
| `Resolved(Resolution)` | `needs_owner_resolution` | `resolved_as_applied`, `resolved_as_absent` | D.7.5 |

#### 0.4.6 `HandshakeFailure` (core, audit detail text)

`frame_before_hello`, `oversized`, `unsupported_schema`, `too_many`, `timed_out`, `peer_user`, `bad_signature`,
`unknown_id`, `removed`, `wrong_key`, `key_in_use`, `key_change_too_soon`, **`too_many_keys`** (C.6.4 row j),
`too_many_sessions`. PART-agents' `rejected` is dropped (P1: no rejected state). What each answers is in C.6.6.

#### 0.4.7 Other stored enums (store row types; snake_case text)

| Enum | Values | Where |
|---|---|---|
| `EnrollmentState` | `pending`, `approved`, `removed` | `enrollments.state`, `EnrollmentRow` |
| `ReleaseKind` | `calendars`, `availability`, `events`, `effect` | `releases.kind` (the contract's four result kinds) |
| `PurgeCause` | `released`, `expired`, `account_disconnected`, `agent_removed` | `held_results.purge_cause` |
| `ReadOutcome` | `held`, `stopped`, `stale`, `transient`, `rejected`, `no_answer_in_time` | `read_attempts.outcome`; `stale` is V6's failed check; `no_answer_in_time` is the order watchdog |
| `Notification` | `sent`, `not_requested`, `unknown` | `effects.notification` (PO6; contract P2-10) |
| `ObservationPurpose` | `observe`, `reconcile` | `observations.purpose` |
| `ObservationOutcome` | `found`, `cancelled`, `absent` | `observations.outcome` |
| `SystemActionKind` | `refresh`, `reconcile`, `list_calendars` | `system_proofs.action_kind` |
| `ConnectProblem` (type in step P3, column in migration 3) | `timed_out`, `superseded`, `browser_not_opened`, `google_error`, `client_rejected`, `network`, `identity_check_failed`, `unexpected_scope`, `no_refresh_token`, `token_store`, `already_an_account` (set only by the core, D.2) | `ProviderAccountRow.last_connect_problem` (W05 V3-2) |
| `RevocationState` (migration 3) | `not_requested`, `pending`, `revoked`, `not_confirmed` | `ProviderAccountRow.revocation` (W05 V3-3) |
| `AuthLostCause` (type in step P3, column in migration 3) | `grant_ended`, `client_rejected` | `ProviderAccountRow.auth_lost`, account signal (W05 3.8) |

#### 0.4.8 Audit kinds (no migration; `audit.kind` has no CHECK, `0001_initial.sql`)

| Kind | Detail | Where |
|---|---|---|
| `handshake_failed` | `{cause, enrollment_id or null, window_start_ms, window_end_ms, count}`, one row per cause and known id per 60 s | C.6.5 |
| `provider_mismatch` | the request id, no content | B.14 |
| `provider_differs` | the request id, no content; one row in the commit of a fitting, unconfirmed `Applied` (V23) | B.14 |

### 0.5 Core types and functions used by more than one section

Signatures only. `Id`, `Digest`, `EventRef`, `CalendarRef`, `IanaZone`, `ProviderRef`, `Action`, `ToolResult`,
`Frame`, `Grant`, `Authorization`, `ProofOutcome` are the contract's.

#### 0.5.1 Held answers, proof actions and system proofs (CORE `service/held.rs`, `ports.rs`; section B)

```rust
pub(crate) struct HeldAnswer {
    pub(crate) seq: u64,
    pub(crate) connection: ConnectionId,
    pub(crate) session_id: Id,          // valid only while the connection is bound to this session
    pub(crate) call_id: Id,
    pub(crate) tool: ToolName,
    pub(crate) request_id: Id,
    pub(crate) deadline_ms: i64,        // core clock: now + AGENT_ANSWER_HOLD_MS
}
// `Held` and its methods: B.8.1.
pub(crate) fn settled(state: RequestState) -> bool; // terminal, pending_approval, applied, outcome_unknown
impl Core { pub(crate) fn wake_held(&mut self) -> Result<(), StoreFault>; }

pub enum ProofAction { Agent(Action), Disclose(Action) }   // Disclose: never RequestResult or CalendarList
// defined in ST rows.rs (0.5.2), shipped in step P3; ObservedEvent moves there from CORE ports.rs:76 (X21)
pub use fabric_storage::{AuthLostCause, ConnectProblem, Notification, ObservedEvent, SystemAction, SystemActionKind};
pub struct SystemProofJob {
    pub job_id: u64, pub action: SystemAction, pub action_digest: Digest, pub facts: TrustedFacts,
    pub fence: Fence, pub serves: Option<Id>,
    pub expects: Option<RequestState>,  // the state of `serves` when the job started (B.11.3); None with no request
    pub started_at_ms: u64, pub deadline_ms: u64,
}
pub struct SystemProofDone {
    pub job_id: u64, pub proof_id: Option<Id>, pub outcome: ProofOutcome, pub action_digest: Option<Digest>,
    pub failing_rule: Option<String>, pub undecided_reason: Option<String>, pub check_theory_text: Option<String>,
}
pub trait Prover: Send {
    fn identity(&self) -> ProverIdentity;
    fn prove(&mut self, job: &ProofJob) -> ProofDone;
    fn prove_system(&mut self, job: &SystemProofJob) -> SystemProofDone;
}
pub fn system_action_digest(action: &SystemAction) -> Digest; // pub: the provider workers check it (DD2, C2)
```

#### 0.5.2 Store row types and readers (ST `rows.rs`, `read.rs`; section A)

```rust
// shared with the core, which re-exports them from ports.rs (X21); serde, snake_case; step P3
pub enum SystemActionKind { Refresh, Reconcile, ListCalendars }
pub enum SystemAction {                                       // stored JSON: A.4.3
    Refresh { calendar: CalendarRef },
    Reconcile { calendar: CalendarRef, event_id: ProviderRef },
    ListCalendars { account_id: Id },                        // TH1 (P2-6); not issued before it merges
}
// the members of CORE ports.rs:76-97, unchanged; stored JSON: A.4.3
pub struct ObservedEvent {
    pub calendar: CalendarRef, pub event_id: String, pub revision: String, pub fabric_created: bool,
    pub recurring: bool, pub default_type: bool, pub timed: bool, pub invitation: bool,
    pub attendees: Vec<String>, pub occurrence_start_ms: Option<i64>,
}
pub enum ConnectProblem { /* the eleven values of 0.4.7 */ }
pub enum AuthLostCause { GrantEnded, ClientRejected }
pub enum EnrollmentState { Pending, Approved, Removed }
pub struct EnrollmentRow {
    pub enrollment_id: Id, pub revision: u64, pub registered_name: Label, pub destination: Destination,
    pub state: EnrollmentState, pub public_key: Option<PublicKey>, pub added_at_ms: i64,
    pub key_proven_at_ms: Option<i64>, pub approved_at_ms: Option<i64>, pub removed_at_ms: Option<i64>,
}
pub enum ReleaseKind { Calendars, Availability, Events, Effect }
pub struct ReleaseRow {                                       // the release record (P6)
    pub release_id: Id, pub request_id: Id, pub subject_request_id: Id, pub agent_id: Id,
    pub kind: ReleaseKind, pub proof_id: Id, pub grant_ids: Vec<Id>, pub held_result_id: Option<Id>,
    pub result_digest: Option<Digest>, pub released_at_ms: i64,
}
pub struct HeldResultRow {
    pub held_result_id: Id, pub request_id: Id, pub agent_id: Id, pub account_id: Id, pub kind: ReleaseKind,
    pub bytes: u64, pub stored_at_ms: i64, pub purge_due_at_ms: i64,
}
pub enum PurgeCause { Released, Expired, AccountDisconnected, AgentRemoved }
// read types, as SessionRow.ended (ST rows.rs:89)
pub struct HeldResultRecord { pub row: HeldResultRow, pub purged: Option<(i64, PurgeCause)> }
pub struct ReadAttemptRow { pub request_id: Id, pub number: u32, pub authorization_id: Id, pub started_at_ms: i64 }
pub struct ReadAttemptRecord { pub row: ReadAttemptRow, pub finished: Option<(i64, ReadOutcome)> }
pub enum ReadOutcome { Held, Stopped, Stale, Transient, Rejected, NoAnswerInTime }
pub struct AgentView { pub enrollment: Option<EnrollmentRow>, pub agent: Option<AgentRow> }   // read type (C.2.1)
pub enum Notification { Sent, NotRequested, Unknown }
pub struct SystemProofRow {
    pub proof_id: Id, pub action_kind: SystemActionKind, pub account_id: Id, pub serves_request_id: Option<Id>,
    pub action_digest: Digest, pub prover_kind: ProverKind, pub record: SystemProofRecord, pub recorded_at_ms: i64,
}
pub struct SystemProofRecord {
    pub action: SystemAction, pub outcome: ProofOutcome, pub failing_rule: Option<String>,
    // as ProofRecord keeps them
    pub fence: Fence, pub undecided_reason: Option<String>, pub check_theory_text: Option<String>,
}
pub struct ObservationRow {
    pub observation_id: Id, pub request_id: Id, pub purpose: ObservationPurpose, pub proof_id: Id,
    pub account_id: Id, pub calendar_id: Id, pub event_id: String, pub outcome: ObservationOutcome,
    pub revision: Option<String>, pub facts: Option<ObservedEvent>, pub matches_approved: Option<bool>,
    pub evidence_id: Option<Id>, pub observed_at_ms: i64,
}
// migration 3; the members beyond these are open until migration 3 merges (A.5)
pub struct ProviderAccountRow {
    pub account_id: Id, pub revision: u64, pub provider: String /* "google" */, pub subject: Option<String>,
    pub granted_scopes: Vec<String>, pub token_ref: Option<String>, pub connect_generation: u64,
    pub state_before_connect: Option<AccountState>, pub connected_at_ms: Option<i64>,
    pub refresh_token_ends_at_ms: Option<i64>, pub calendars_listed_at_ms: Option<i64>,
    pub last_connect_problem: Option<ConnectProblem>, pub auth_lost: Option<AuthLostCause>,
    pub revocation: Option<RevocationState>,
}
pub struct ProviderCalendarRow {
    pub account_id: Id, pub calendar_id: Id, pub provider_calendar_id: String, pub revision: u64,
    pub access_role: String, pub is_primary: bool, pub time_zone: IanaZone, pub listed: bool,
    pub first_seen_ms: i64, pub last_seen_ms: i64,
}
```

Readers (ST `read.rs`): `enrollment(&Id)`, `enrollment_by_key(&PublicKey)`, `agent_views() -> Vec<AgentView>`
(pending, approved, removed, seeded; order of C.2.1), `agent_view(&Id) -> Option<AgentView>` (both rows of one agent,
for approve, remove and pause), `effect_event(&Id) -> Option<(Option<EventRef>, Notification)>` (`None` when the
effect has no row or its `notification` is NULL, that is before any `EffectEventRecorded`; NULL is never read as
`unknown`), `created_event(&CalendarRef, event_id: &str) -> Option<Id>` (the request id of the `effects` row with
that `account_id`, `calendar_id` and `provider_event_id`; index `effects_provider_event`, `0001_initial.sql:111`),
`created_events(&CalendarRef) -> Vec<(String, Id)>` (every reserved id of the calendar with its request id, in id
order), `release_of(&Id)`, `releases_of_subject(&Id)`, `held_result_of(&Id) -> Option<HeldResultRecord>`,
`unpurged_held_results_of_account(&Id)`, `unpurged_held_results_of_agent(&Id)`, `next_purge_due_ms() -> Option<i64>`
(the earliest `purge_due_at_ms` of an unpurged row), `held_payload(&Id) -> Option<Vec<u8>>` (digest checked),
`evidence(&Id)` (digest checked), `read_attempts(&Id) -> Vec<ReadAttemptRecord>`, `system_proofs_of(&Id)`,
`observations_of(&Id)`, `latest_observation(&Id)` (the latest row with purpose `observe` and outcome `found`;
reconcile rows never feed the fence, B.12), `due_purges(now_ms)`, `schema_at_least(version)`; migration 3:
`provider_account(&Id)`, `provider_accounts() -> Vec<ProviderAccountRow>` (in account id order; D.8.1 and D.8.4 read
the placeholder, the row with `subject` null, and the known subjects from it), `provider_account_by_subject(&str)`,
`provider_calendars(&Id)`.

`created_event` and `created_events` read version 1 columns only, no guard uses them, they need no gate (R3), and
they ship in step P3. The core applies DD12's state test and D.6.3's five-minute test with `ReadTx::request` on that
id.

Ids made by the core and carried in the mutation (A.2 R8): grants `grant-` and 32 hex; releases `release-` and 32
hex; held results `held-` and 32 hex; observations `obs-` and 32 hex; accounts `account-` and 32 hex; calendars
`cal-` and 32 hex; agents through `ids::new_id("agent")` (C.3.1).

#### 0.5.3 Shared functions

```rust
// grants.rs (section C uses it in approve_agent; section E's debug seed uses it; P5, V21)
pub(crate) fn create_enrollment_grant(tx: &mut WriteTx<'_>, cx: &mut Cx<'_>, agent_id: &Id)
    -> Result<(Grant, PositiveCount), StoreFault>;
// whether this run can carry out an order on this account (V17; D.12, E.0 ED7):
// (Practice, provider_account == false) and (Google, provider_account == true) are true; all else false
pub(crate) fn can_carry_out(attachments: &Attachments, account: &Account, provider_account: bool) -> bool;
pub struct Attachments { pub prover: Option<ProverIdentity>, pub provider: Option<ProviderKind> }
pub enum ProviderKind { #[cfg(debug_assertions)] Practice, Google }   // ED8
// the calendar target of an order, in the sending commit (D.3)
pub(crate) fn resolve_target(r: &ReadTx<'_>, calendar: &CalendarRef, attachments: &Attachments)
    -> Result<CalendarTarget, StoreFault>;
// the prover's readiness gate of P8 (B.10, C.8): true exactly when a prover is attached and either its status
// (P0's ProverStatus) is Ready or its kind is ProverKind::Synthetic (debug builds only; the classifier answers every
// job at once and reports its status as unavailable by design, BR synthetic.rs:93-99)
pub(crate) fn prover_ready(&self) -> bool;
// the fence of P10 (B.12, D.10): epochs and the request's stored observed revision
pub(crate) fn fence_now(counters: Counters, observed_revision: Option<String>) -> Fence;
pub(crate) fn fence_holds(proved: &Fence, now: &Fence) -> bool; // unchanged: proved == now
pub(crate) fn observed_revision(r: &ReadTx<'_>, request_id: &Id) -> Result<Option<String>, StoreFault>;
// results (section B)
pub(crate) fn result_source(r: &ReadTx<'_>, agent_id: &Id, target: &Id) -> Result<Option<ResultSource>, StoreFault>;
pub(crate) enum ResultSource { Effect { effect_id: Id, event: Option<EventRef> }, Held { held_result_id: Id, kind: ReleaseKind } }
pub(crate) fn list_result(r: &ReadTx<'_>) -> Result<ToolResult, StoreFault>;
pub fn result_digest(result: &ToolResult) -> Digest; // CORE service/results.rs (step P5b), domain line
// "fabric-broker-result-v1\n"; the store only keeps the value
// the one test of P10 iterates both lists (D.9.4). The nine owner commands: pause_agent (when it pauses; a resume
// raises nothing), revoke_grant, save_profile (an activation, or a new version of the active profile), save_group
// (any change, deletion included), save_preferences (when warning_calendars change), connect_google,
// disconnect_google, select_calendars, remove_agent
pub(crate) const NARROWING_COMMANDS: &[OwnerMethod];
// the broker commits, by operation type: provider.connect, provider.calendars (when the record changes),
// provider.auth_lost, broker.started (when an account changed, D.8.4); and the AuthLost branch of every result
// commit, which the test runs as its own case
pub(crate) const NARROWING_COMMITS: &[&str];
// every other owner method, each with one line of reason (D.9.4)
pub(crate) const NOT_NARROWING: &[OwnerMethod];
// the order watchdog (B.13, D.5): an order past deadline_ms + ORDER_WATCHDOG_MARGIN_MS on the core clock; the
// worker's own bound is the shell's monotonic stamp plus PROVIDER_CALL_MS (X23)
pub(crate) fn order_overdue(order_deadline_ms: u64, now_ms: u64) -> bool;
// the configuration texts (C.10): built by the binary's shell::configuration_texts(&Config)
pub type ConfigurationTexts = Box<dyn Fn(&Id) -> Option<AgentConfiguration> + Send>;
```

#### 0.5.4 Jobs and inputs (CORE `ports.rs`)

```rust
pub enum Job { Prove(ProofJob), ProveSystem(SystemProofJob), Dispatch(DispatchOrder), Observe(ObserveOrder),
    Read(ReadOrder), Reconcile(ReconcileOrder), List(ListOrder), Connect(ConnectOrder),
    Unlink { account_id: Id, revoke: bool }, Forget(Id) }
pub enum Input { /* as today, plus */ SystemProofDone(SystemProofDone), Observed(ObserveDone), ReadDone(ReadDone),
    Reconciled(ReconcileDone), Listed(ListDone), Connected(ConnectOutcome), Unlinked(Id, Option<RevokeOutcome>) }
```

The order and answer types are D.2's. `Connect` and `Forget` run on the sign-in worker, `Unlink` on the unlink
worker (D.1); `Unlink` and `Unlinked` replace `Revoke(Id)` and `Revoked(Id, RevokeOutcome)` (DD18, ruled
2026-11-18 by the lead). `ProviderRef` validation of W05 5.3 runs in the adapter on the observe order,
before classification (REV-order SEC-event-ids).

### 0.6 Numbers and fixed names

| Constant | Value | Where | Source |
|---|---|---|---|
| `AGENT_ANSWER_HOLD_MS` | 20_000 | CORE `broker.rs` | P7 |
| `MAX_HELD_PER_CONNECTION` | 16 | CORE `service/held.rs` | P7 |
| `WRITER_QUEUE_FRAMES` | 32 (`MAX_HELD_PER_CONNECTION + 16`) | BR `listener.rs` | P7 |
| `WRITER_QUEUE_BYTES` | 4_194_304 (4 x `MAX_FRAME_BYTES`) | BR `listener.rs` | P7 "bounded by bytes"; value accepted by the lead 2026-11-17 (OP13) |
| `QUIT_WRITE_BOUND_MS` | 500 | BR | C.9.6 (CD19): how long the main thread waits at quit for the writers |
| `SESSIONS_PER_AGENT_WINDOW` | 6 | CORE `service/handshake.rs` | P7 |
| `SESSION_START_WINDOW_MS` | 60_000 | CORE `service/handshake.rs` | P7 |
| `HANDSHAKE_MS` | 5_000 | BR `listener.rs`, CORE (challenge deadline) | P9 |
| `MAX_HANDSHAKES` | 8 | CORE and BR | P9 |
| `MAX_AGENT_CONNECTIONS` | 32 | BR `listener.rs` | PART-agents 3.1; ruled by the lead 2026-11-17 (OP9) |
| `FIRST_LISTENER_CONNECTION` | `1 << 32` | BR `listener.rs` | P4 (the synthetic agent keeps `ConnectionId(1)`) |
| `SUN_PATH_MAX` | 103 on macOS, 107 on Linux | BR `listener.rs` | C.9.1 |
| `HANDSHAKE_AUDIT_WINDOW_MS` | 60_000 (= `BUSY_AUDIT_WINDOW_MS`) | CORE | P9 |
| `KEY_CHANGE_INTERVAL_MS` | 60_000 | CORE | PART-agents 2.3 |
| `MAX_KEY_CHANGES` | 20, counted as `enrollments.revision - 2` while pending | CORE `service/handshake.rs` | C.6.4 row j; open OPC10 |
| `NONCE_BYTES` | 32 | CORE | DRAFT README section 8 (256 bits) |
| `HELD_RESULT_LIFETIME_MS` | 86_400_000 | CORE | W05 OQ5; owner OD3. The core sets `purge_due_at_ms` from it and a core test pins it; no store guard compares with it (A.2 R8) |
| `RECONCILE_FIRST_READ_DELAY_MS` | 10_000 | CORE | P10 / W05 section 7 |
| `RECONCILE_SECOND_READ_MS` | 60_000 | CORE | P10 |
| `ORDER_WATCHDOG_MARGIN_MS` | 5_000 | CORE `broker.rs` | V20; matches `PROOF_WATCHDOG_MS - PROVIDER_CALL_MS` (CORE `broker.rs:20, 24`) |
| `PROVIDER_CALL_MS` | 30_000 | CORE `broker.rs:24` | unchanged; every order's deadline, and the bound of an unlink's one revocation attempt (W05 3.9), which has no watchdog: `revocation: pending` is its durable marker (D.8.3, D.8.4) |
| `PROOF_WATCHDOG_MS` | 35_000 | CORE `broker.rs:20` | unchanged; also system proofs |
| `CONNECT_WATCHDOG_MS` | 365_000 (`SIGN_IN_WAIT_MS` + 2 x `PROVIDER_CALL_MS` + `ORDER_WATCHDOG_MARGIN_MS`) | CORE | D.8.6 |
| `RetryPolicy::PLAN` | 3 retries, 2_000 / 4_000 / 8_000 ms, budget 120_000 | CORE `broker.rs:45-50` | one copy for dispatches, reads, observes and reconciles (REV-core CO9 d) |
| `FRESH_CREATE_ABSENT_MS` | 300_000 | CORE | W05 CO6 table ("created in the last five minutes") |
| `SIGN_IN_WAIT_MS` | 300_000 | BR | W05 OQ2 |
| `MAX_CALENDARS_PER_ACCOUNT` | 250; a list past it fails as a whole | GC and CORE (the list commit, D.9.2; no store guard, A.2 R8) | W05 PD23, ST2; REV-store ST2 e fixes W05 6.2 |
| `MAX_LIST_PAGES` | 20; a calendar list past it fails as a whole | GC and CORE | W05 6.2; D.2 |
| `MAX_READ_RESULT_BYTES` | 786_432; a read result past it is `rejected` | CORE | W05 PD23, 6.4; B.3.4 step 2.1 |
| `ZONE_SLACK_MS` | 50_400_000 (14 h, the widest UTC offset); an all-day span is widened by it each way for the overlap check of a read only, never stored or released | CORE | the lead's ruling of 2026-11-18 (DD17; B.3.4 step 2.1); exact bounds wait for a zone database, CD-W09's (I.5 D-18) |
| `AGENT_WRITE_TIMEOUT_MS` | 5_000, the write timeout of a connection's writer thread | BR `listener.rs` | C.9.6 |
| `ACCEPT_RETRY_MS` | 100, the accept thread's sleep after an accept error | BR `listener.rs` | C.9.2 |
| `MAX_EVIDENCE_BYTES` | 1_048_576, the whole evidence `{"before", "raw"}` of one observation, and the evidence of a V23 attempt; past it an observation is refused (D.6.2) and a V23 attempt keeps no evidence (B.14) | CORE | the lead's ruling of 2026-11-18, replacing W05 ST3's 262_144; owner OD26 |
| `MAX_BEFORE_VIEW_BYTES` | 524_288, the before-view in canonical form, and the after-view of V23 (P2-32); lowered (to about 400 KiB) if the frame test of D.6.2 shows that both views and the rest of `get_request_details` do not fit `MAX_FRAME_BYTES` | CORE | the lead's ruling of 2026-11-18; owner OD26 |
| `ACCOUNT_RESERVE_BYTES` | 98_304 (96 KiB) per account in `owner_state_bytes`; a list whose account record would pass it fails as a whole | CORE `owner/views.rs` | D.11 (DD11); owner OD4 |
| `OWNER_STATE_BUDGET_BYTES` | 737_280 (720 KiB), now also counting every account at `ACCOUNT_RESERVE_BYTES` and the agents' records (removed agents stay in the snapshot); `add_agent` past it is refused `invalid_params` at `/params/registered_name`, `connect_google` at `/params/account_id` | CORE `owner/views.rs:33` | P10; C.3.1, D.11 |
| `CARD_REOBSERVE_MS` | 600_000 (deferred) | CORE | W05 CO12 |
| `MAX_FRAME_BYTES` | 1_048_576 | CC `common.rs:263` | unchanged |
| `QUEUED_EVENTS` | 4 | BR `shell.rs:47` | unchanged |
| `PRACTICE_CALENDAR_NAME` | `"Practice calendar (test build)"` | BR `practice.rs` (debug) | E.1.3 |
| `BRIDGE_KEY_DIR` | `"bridge-keys"` under the data directory, mode 0700, debug builds with the test key | BR | E.0 ED10, C.9.1 |

---------------------------------------------------------------------------------------------------------------------

## A. The store

**What this section is.** The store's part of phase 2 of CD-W05 and CD-W06: the rule set that settles ADR-115
"Open" for migrations 2 and 3, the amendments it needs in ADR-115 and in the W04 design, migration 2 in full,
migration 3 as far as the CD-W05 design allows, the content kept outside the ledger, the fixtures and the
rehearsal, and an answer to each store request (ST1 to ST11, LR-S1 to LR-S4). It binds the pull request of step P3
(migration 2, by the lead) and step P7a (migration 3). It is proposed text: nothing here is applied to ADR-115 or
to the W04 design before the owner approves it (P2, P13).

Names follow section 0. Where this section adds or corrects a name, A.12 lists it.
Code is cited at main `2d41210`. Short names: ST `crates/fabric-storage/src/`, CORE
`crates/fabric-broker-core/src/`, CC `crates/fabric-desktop-contracts/src/`, W04, PW, ADR-115, PLAN, W05, W06,
DRAFT (the contract draft at `29cdfdf`), REV-store (the store review in `w05/review.json`), J-store (the
store-and-coherence judgement in `w06/integration-result.json`).

Two version numbers must not be confused. The **store schema version** is `SCHEMA_VERSION` in ST `migrate.rs:4`
(1 today) and `PRAGMA user_version`. The **contract schema version** is `SCHEMA_VERSION` in CC `common.rs:15` (2
today, 3 with DRAFT), sent in `hello`. This section raises only the first: migration 2 makes store version 2 while
the contract is at 3.

---------------------------------------------------------------------------------------------------------------------

### A.1 Decisions

| Id | Decision | Why |
|---|---|---|
| AD1 | Migrations 2 and 3 are made additive under the rule set R1 to R11 of A.2. Replay does not become version-aware now | P2. Replay already runs each migration's SQL at its `SchemaMigrated` row and sets `user_version` in the replay database (ST `ledger.rs:350-375`), so rows before a migration replay against the older schema. The rules make this build's apply code give the same result on those rows |
| AD2 | Migration 2 is one file, `0002_agents_and_results.sql`, holding every table of A.4.1, landed in one pull request by the lead after contract schema version 3 merges (step P3) | P3; J-store "there is no single migration 2". `EnrollmentGrantInserted` carries a version 3 `Grant` with an enrollment origin, which main cannot express (CC `owner.rs:557-563`) |
| AD3 | Observations and the event columns of `effects` are in migration 2; migration 3 holds `provider_accounts` and `provider_calendars` only | V1 (the practice provider must answer observe and reconcile before step P6). Departs from P3's list; ruled by the lead 2026-11-17 (AO2, OP1) |
| AD4 | Every branch of an existing variant that touches a table of migration 2 is gated on `user_version >= 2` and named: G1 to G9 (A.4.5) | P2 second rule. The first draft's G1 to G6, G4 widened, and three more (G7 to G9) that close a pending or removed agent and a read with an effect at the store |
| AD5 | The payload of a held result is deleted by the apply of `HeldResultPurged`, not by a separate call. `WriteTx::delete_held_payload` is dropped | A commit that records a purge and keeps the content would be the worst failure of the purge. In the apply it cannot be forgotten. Replay deletes nothing from an empty table, which is harmless |
| AD6 | `Store::write` refuses to commit a transaction in which a `HeldResultStored` has no payload row, or an `ObservationRecorded` names an `evidence_id` with no evidence row | The content and the record that names it are one SQLite transaction (P2), checked, not assumed |
| AD7 | No projection table has a foreign key to a non-projection table. A non-projection table may reference a projection table | Replay runs with `foreign_keys = ON` (ST `ledger.rs:347-349`) and never writes the non-projection tables, so such a key would fail the replay |
| AD8 | `ResultReleased` and `HeldResultStored` check, in the store, that the releasing proof's (or the read authorization's) fence epochs equal the counters of the commit | A store-level backstop for "never release content under stale authority": ending a grant or pausing raises an epoch (W04 5.6), so a stale proof or authorization is refused at commit even if the core missed it |
| AD9 | A grant id is unique across `grants` and `enrollment_grants` (guards on both inserts, invariant I24) | Every reader of grants looks in both tables by id (P5); a duplicate would make the answer depend on the order of the lookup |
| AD10 | A new store still runs each migration in its own transaction (ST `store.rs:300-314`). A new version 2 store therefore has two ledger rows: `{SchemaMigrated 0->1, StoreCreated}` and `{SchemaMigrated 1->2}` | No change to the runner. The tests that pin a head of 1 after creation change (A.8.4) |
| AD11 | The open invariants become `(check, since, until, sql)`; `check_invariants` takes the store's `user_version` and skips an entry outside its range | P2; J-store "the rehearsal tests cannot be built as written": a version 1 store opened with a short migration list would fail on a table it does not have |
| AD12 | The frozen version 1 store is a text dump made by the version 1 code at two commits, plus one SQLCipher file made on macOS (A.8) | REV-store ST11: a version 1 store built by the version 2 code proves nothing about a store main wrote |
| AD13 | After a commit that deletes held payloads, the store runs `PRAGMA wal_checkpoint(TRUNCATE)`; a busy checkpoint is retried at the next purge and at close | Older WAL frames keep the payload's pages until a checkpoint (REV-store ST4 c) |
| AD14 | `owner.resolve_outcome` carries the request id as `correlation_id`, as `owner.decide_request` does (CORE `owner/dedup.rs` `meta`) | ADR-115 D2: the correlation id is the request id of a row tied to a request |
| AD15 | `agent.prove_key` rows have actor `agent` with the enrollment id | ADR-115 D2: the agent id is the one the channel authenticated; the signature authenticated possession of the key for that id. The row does not say the agent is approved |

---------------------------------------------------------------------------------------------------------------------

### A.2 The rule set (proposed text for ADR-115 "Open" and for W04 3.6)

This text replaces the fourth bullet of ADR-115 "Open" (ADR-115:146) and the last bullet of W04 3.6
(`calendar-desktop-w04-design.md:521-526`), and closes W04 R22 for migrations 2 and 3. It is written once and
copied word for word into both.

> **Migrations without version-aware replay (amendment, Proposed 2026-11-17).** Replay applies this build's apply
> code to every ledger row, and runs each migration's SQL at its `SchemaMigrated` row, so a row committed before
> migration N is applied while the replay database's `user_version` is below N. Until replay chooses its apply and
> decode rules by version, every migration and every change to replay code keeps these rules.
>
> - **R1 Only add.** A migration only creates tables, adds nullable or constant-defaulted columns
>   (`ALTER TABLE ... ADD COLUMN`), and creates indexes and triggers on its own tables. It never drops, renames or
>   retypes a column, changes a constraint, backfills other than by a constant default, or changes a seed of an
>   earlier migration. Released migration SQL is frozen; its number is taken when it merges to main and is never
>   reused.
> - **R2 New tables, new variants.** A new table is written only by mutation variants added with it, and, for a
>   table outside the projection, by the store calls R9 names. An existing variant's apply writes the same tables
>   and columns as before and never refuses a row it accepted before. It may read or write a table of migration N,
>   or refuse more, only on a branch taken when the database's `user_version` is at least N; a row committed before
>   N is applied below N and cannot take the branch. Each branch is named here with its test (the replay of the
>   frozen stores of version N-1 and a unit test of the branch at version N). For migration 2: `ReadTx::grant` and
>   `ReadTx::grant_agent` also look in `enrollment_grants` (G1, G2); `GrantEnded` ends a row of `enrollment_grants`
>   (G3); `GrantInserted`, `SessionStarted` and `RequestInserted` refuse an agent whose enrollment is not approved,
>   and `GrantInserted` a grant id that `enrollment_grants` holds (G4, G5, G8); `AgentPut` accepts a new row for an
>   id that has an enrollment only while that enrollment is pending with a key, and refuses an update once it is
>   removed (G7); `EffectReserved` refuses a tool that changes no calendar (G9). A reader that no guard uses may read
>   a later table behind the same gate (G6: `live_grants`, `grants_of_agent`). Migration 3 names none.
> - **R3 Readers of guards.** No reader used by a guard during replay names a column of a later migration.
>   `EFFECT_COLUMNS` (`crates/fabric-storage/src/read.rs:212`), read by the guard of `DispatchIntent`, keeps the
>   ten columns of version 1; later columns of `effects` are read by queries of their own.
> - **R4 Constraints of new tables.** A new table carries no CHECK that lists values or fixes a text format; apply
>   guards and one open invariant check those. Structural CHECKs (paired NULL columns, `revision >= 1`, `number >=
>   1`) and UNIQUE indexes stay. A projection table never has a foreign key to a table outside the projection.
> - **R5 Invariants by version.** Each open invariant states the schema version it starts at (`since`) and, when a
>   later version replaces it, the last version it applies to (`until`). The check at open skips an invariant
>   outside the store's version.
> - **R6 Stored shapes only widen.** A stored JSON shape (a row type, a contract type kept in a `record` column or
>   in a ledger payload, a `Mutation` variant) gains only optional members, left out when absent and defaulted when
>   read. An enum gains only values. A new `Mutation` variant ships with the migration that creates what it
>   writes; no code that writes one merges before it. A new optional member and a new value of a stored enum that
>   need no new table or column ship no later than the first code that writes them, with the record R7 asks.
> - **R7 Contract code is replay code.** The contract crate's encoders of stored types, and the contract functions
>   that apply calls (`RequestState::is_terminal`, `RequestState::can_transition_to`, the decoders), may only widen
>   from one schema version to the next. A change that is not a widening records why no committed row is affected.
>   A test in the contract crate pins every version 2 fixture of a stored type byte for byte.
> - **R8 Values come from the core.** Ids, times and every other new value are made in the core and carried in the
>   mutation, never made while applying. An apply guard never compares a value with a constant that may change (a
>   lifetime, a size, a count); the core checks those.
> - **R9 Content outside the ledger.** Content kept outside the ledger and the projection lives in tables named in
>   `NOT_PROJECTION`, written by named store calls in the same SQLite transaction as the mutation that names it.
>   The ledger names such content by a random id, never by a digest of the content.
> - **R10 Operation types.** A new operation type is added by a dated amendment of D2 and ships in the pull request
>   of the migration that lets it be written. `payload_version` stays 1.
> - **R11 What a migration's pull request carries.** The raised `SCHEMA_VERSION`, the frozen `schema-vN.sql`, a
>   frozen store of version N-1 written by the code of version N-1, the rehearsal on it, and every list that names
>   tables: `PROJECTION_TABLES` (fitness and replay tests), `replay_differs`, `NOT_PROJECTION`, the replay model.
>
> Version-aware replay is not built now. It is needed before the first migration that cannot keep R1 to R9. It
> takes: apply rules and guard readers chosen by the replay database's `user_version`; one decode type per
> `payload_version`, with `verify_chain` accepting every known version; and a rehearsal migration that alters a
> projection table.

What the rules protect, in one line each: R1 and R2 keep the replay of old rows equal to what was committed; R3
keeps an old row's guard from failing on a column its replay database lacks (REV-store ST5); R4 keeps a later value
from needing a changed constraint; R5 keeps a short-list test store openable; R6 and R7 keep a re-encoded old record
byte-identical (REV-store "REPLAY" rule 2); R8 keeps replay deterministic (ST `apply.rs:3-5`); R9 keeps a purge
real (V3); R10 keeps the type list a reviewed decision (ADR-115 Consequences).

Two known cases that R7 covers and records:

| Change | Why no committed row is affected |
|---|---|
| DRAFT makes `needs_owner_resolution` not terminal (DRAFT `lifecycle.rs` `is_terminal`) | Main drives no move into `reconciling` or `needs_owner_resolution`: the test at CORE `service/reducer.rs:196-217` asserts the transitions left undriven are those into `cancelled`, into and out of `reconciling`, and into `reconciled` |
| DRAFT's `GrantOrigin` becomes an enum (DRAFT `owner.rs:556-625`) | The approval branch encodes as `{"approval_digest","request_id"}`, as the version 2 struct did; the branches have no tag member |

One pending change is bound by R2 and R6 and is not relied on here: P0's optional cause member of `RequestMoved`.
The duration rule's "approval null while a request waits again" is on the read side only (the lead's ruling of
2026-11-17, AO3, closed): `get_request_details` returns `approval` null while a request waits again after an
earlier approval, as DRAFT states at `63e8018` (README line 117, choice 67). It changes nothing that is written: no
variant, row, column or stored shape. So R2 is not touched.

---------------------------------------------------------------------------------------------------------------------

### A.3 Amendments of ADR-115 and of the W04 design (proposed text)

Each block replaces or extends the text it names. All of them go to the owner together, as one Proposed amendment
dated 2026-11-17 (P13).

**ADR-115 D1** (ADR-115:25-30). Add after the last bullet:

> - Amendment (Proposed 2026-11-17). Two tables hold content that is neither projection nor ledger:
>   `held_result_payloads` (a read result held for its release) and `observation_evidence` (an event as the
>   provider returned it, and its before-image). They are named in `NOT_PROJECTION` with `meta`, `ledger` and
>   `audit`; replay neither builds nor compares them. A row of either is written only by `WriteTx::put_held_payload`
>   or `WriteTx::put_evidence`, in the same transaction as the mutation that names its id, and the commit fails if a
>   mutation names an id whose row is missing. A held payload is deleted only by the apply of `HeldResultPurged`
>   and in each backup copy. A reader checks the stored digest and reads a missing or mismatched row as absent.

**ADR-115 D2** (ADR-115:58). Add after the list of types:

> Amendment (Proposed 2026-11-17). Store schema version 2 adds `owner.add_agent`, `owner.approve_agent`,
> `owner.remove_agent`, `owner.select_calendars`, `owner.resolve_outcome`, `agent.prove_key`, `request.read`,
> `request.release`, `request.reconcile`, `system.proof`, `provider.observation` and `timer.result_purge` (34 types).
> Store schema version 3 adds `owner.connect_google`, `provider.connect`, `provider.calendars`,
> `provider.auth_lost` and `provider.revoke` (39 types), and `owner.refresh_calendars` if the contract adds that
> command (40). A type is written only by a build whose store version includes it; an older build refuses the newer
> store before it reads the ledger.

**ADR-115 D2, the actor** (ADR-115:42). Add:

> Amendment (Proposed 2026-11-17). An `agent.prove_key` row names as its actor the enrollment id whose key the
> connection proved; while the enrollment is pending the id is claimed, not approved.

**ADR-115 D6** (ADR-115:99). Add:

> Amendment (Proposed 2026-11-17). Two more kinds: `handshake_failed`, one row per cause and known id per one-minute
> window, written as `agent_busy` is; `provider_mismatch`, one row in the commit of the attempt it concerns.
>
> Amendment (Proposed 2026-11-18). One more kind: `provider_differs`, one row in the commit of the attempt it
> concerns, when Google applied a change and returned content that differs from the approval.

**ADR-115 D7** (ADR-115:118). Replace the row `payload_blob_id` with:

> | `payload_blob_id` | The ledger names no blob. Since store schema version 2 the store keeps two kinds of content outside the ledger, in the same encrypted file (D1 amendment): held read results, deleted at release, after 24 hours, at the account's disconnect or at the agent's removal; and observation evidence, kept with its request. Each is named in the ledger by a random id, never by a digest of its content, because a digest of low-entropy content would outlive the purge and let a key holder test a guess. |

**ADR-115 "Open"** (ADR-115:146). Replace the fourth bullet with the rule set of A.2.

**W04 D3** (`calendar-desktop-w04-design.md:46`). Replace the "Chosen" cell with:

> Typed `Mutation` applied only through `WriteTx::apply`; one ledger row per transaction that applied a mutation;
> `verify_replay` rebuilds the projection. The other writers: a migration's SQL (3.6), and, since store version 2,
> `WriteTx::put_held_payload` and `WriteTx::put_evidence`, which write only the two non-projection tables of
> ADR-115 D1, and `backup::take`, which deletes held payloads in the backup copy.

**W04 3.5** (`calendar-desktop-w04-design.md:461`). Replace the first sentence with:

> Projection tables: every table except `meta`, `ledger`, `audit`, `held_result_payloads` and
> `observation_evidence` (`NOT_PROJECTION`).

**W04 3.6** (`calendar-desktop-w04-design.md:497-526`). Change the first bullet to `SCHEMA_VERSION = 2`. Replace the
third bullet whole: "Released migration SQL is frozen: a test compares migration 1 on an empty database with
`schema-v1.sql`, and migrations 1 and 2 on an empty database and a store made by `Store::open` with
`schema-v2.sql`." Add to the backup bullet: "Since store version 2 the rows of `held_result_payloads` are deleted in
the copy before it is verified, when the copy has that table (A.7)." Replace the last bullet with the rule set of
A.2.

**W04 9.2** (`calendar-desktop-w04-design.md:1216-1222`). Replace rows s08 and s09 with the tests of A.8.2.

**W04 section 4** (`calendar-desktop-w04-design.md:576`). After the list of operation types add: "Store schema
version 2 and 3 add the types of the ADR-115 D2 amendment."

**W04 R22** (`calendar-desktop-w04-design.md:1535`). Add: "Settled for migrations 2 and 3 by the rule set of 3.6
(2026-11-17); version-aware replay stays a prerequisite for the first migration that cannot keep it."

---------------------------------------------------------------------------------------------------------------------

### A.4 Migration 2

`crates/fabric-storage/migrations/0002_agents_and_results.sql`. `SCHEMA_VERSION = 2` (ST `migrate.rs:4`);
`MIGRATIONS` gains `Migration { to: 2, sql: include_str!("../migrations/0002_agents_and_results.sql") }` (ST
`migrate.rs:15-18`). It seeds nothing.

#### A.4.1 The SQL as it will be written

```sql
-- Store schema version 2: the agent channel and results (phase 2 design, section A). Only adds (A.2 R1).
-- No CHECK lists values or fixes a text format (R4); apply and the open invariants check them.

-- A pending, approved or removed agent (P1, P3). The id is the agent's id. Rows are never deleted.
CREATE TABLE enrollments (
  enrollment_id TEXT PRIMARY KEY,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  state TEXT NOT NULL,                       -- pending | approved | removed
  public_key TEXT,                           -- 'ed25519:' + 64 lowercase hex; null until a key is proved
  record TEXT NOT NULL,                      -- EnrollmentRow
  added_at_ms INTEGER NOT NULL) STRICT;
CREATE UNIQUE INDEX enrollments_public_key ON enrollments(public_key) WHERE public_key IS NOT NULL;
CREATE INDEX enrollments_pending ON enrollments(added_at_ms) WHERE state = 'pending';

-- The list permission given at approval (P5). One row per agent, ever.
CREATE TABLE enrollment_grants (
  grant_id TEXT PRIMARY KEY,
  agent_id TEXT NOT NULL REFERENCES agents,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  record TEXT NOT NULL,                      -- contract Grant, origin Enrollment { enrollment_id }
  ended_at_ms INTEGER,
  end_reason TEXT,                           -- revoked_by_owner | agent_removed
  CHECK ((ended_at_ms IS NULL) = (end_reason IS NULL))) STRICT;
CREATE UNIQUE INDEX enrollment_grants_agent ON enrollment_grants(agent_id);

-- A read result held for its release (CD-W05 ST4 and CD-W06 LR-C4 are one thing).
CREATE TABLE held_results (
  held_result_id TEXT PRIMARY KEY,
  request_id TEXT NOT NULL UNIQUE REFERENCES requests,
  agent_id TEXT NOT NULL REFERENCES agents,
  account_id TEXT NOT NULL,
  kind TEXT NOT NULL,                        -- availability | events
  bytes INTEGER NOT NULL,
  stored_at_ms INTEGER NOT NULL,
  purge_due_at_ms INTEGER NOT NULL,
  purged_at_ms INTEGER,
  purge_cause TEXT,                          -- released | expired | account_disconnected | agent_removed
  CHECK ((purged_at_ms IS NULL) = (purge_cause IS NULL))) STRICT;
CREATE INDEX held_results_due ON held_results(purge_due_at_ms) WHERE purged_at_ms IS NULL;

-- Not projection (NOT_PROJECTION). The content of a held result; deleted by the apply of HeldResultPurged.
CREATE TABLE held_result_payloads (
  held_result_id TEXT PRIMARY KEY REFERENCES held_results,
  digest TEXT NOT NULL,                      -- sha256 of content; checked on read
  content BLOB NOT NULL) STRICT;

-- One row per release (P6).
CREATE TABLE releases (
  release_id TEXT PRIMARY KEY,
  request_id TEXT NOT NULL UNIQUE REFERENCES requests,   -- the releasing request
  subject_request_id TEXT NOT NULL REFERENCES requests,  -- whose result
  agent_id TEXT NOT NULL REFERENCES agents,
  kind TEXT NOT NULL,                        -- calendars | availability | events | effect
  proof_id TEXT NOT NULL,
  grant_ids TEXT NOT NULL,                   -- JSON list; [] only for effect
  held_result_id TEXT REFERENCES held_results,
  result_digest TEXT,                        -- calendars and effect only
  released_at_ms INTEGER NOT NULL) STRICT;
CREATE INDEX releases_subject ON releases(subject_request_id);

-- The record of a read attempt. A read uses no effects row.
CREATE TABLE read_attempts (
  request_id TEXT NOT NULL REFERENCES requests,
  number INTEGER NOT NULL CHECK (number >= 1),
  authorization_id TEXT NOT NULL UNIQUE REFERENCES authorizations,
  started_at_ms INTEGER NOT NULL,
  finished_at_ms INTEGER,
  outcome TEXT,                              -- held | stopped | stale | transient | rejected | no_answer_in_time
  PRIMARY KEY (request_id, number),
  CHECK ((finished_at_ms IS NULL) = (outcome IS NULL))) STRICT;

-- Proofs of the broker's own actions (V8). Append-only, as proof_reports.
CREATE TABLE system_proofs (
  system_proof_row INTEGER PRIMARY KEY,
  proof_id TEXT NOT NULL,                    -- not unique, as proof_reports.proof_id
  action_kind TEXT NOT NULL,                 -- refresh | reconcile | list_calendars
  account_id TEXT NOT NULL,
  serves_request_id TEXT REFERENCES requests,
  action_digest TEXT NOT NULL,
  prover_kind TEXT NOT NULL,
  record TEXT NOT NULL,                      -- SystemProofRecord
  recorded_at_ms INTEGER NOT NULL) STRICT;
CREATE INDEX system_proofs_id ON system_proofs(proof_id);
CREATE INDEX system_proofs_request ON system_proofs(serves_request_id) WHERE serves_request_id IS NOT NULL;
CREATE TRIGGER system_proofs_no_update BEFORE UPDATE ON system_proofs BEGIN SELECT RAISE(ABORT, 'append-only'); END;
CREATE TRIGGER system_proofs_no_delete BEFORE DELETE ON system_proofs BEGIN SELECT RAISE(ABORT, 'append-only'); END;

-- What the broker saw at the provider (observe and reconcile reads).
CREATE TABLE observations (
  observation_id TEXT PRIMARY KEY,
  request_id TEXT NOT NULL REFERENCES requests,
  purpose TEXT NOT NULL,                     -- observe | reconcile
  proof_id TEXT NOT NULL,
  account_id TEXT NOT NULL,
  calendar_id TEXT NOT NULL,
  event_id TEXT NOT NULL,
  outcome TEXT NOT NULL,                     -- found | cancelled | absent
  revision TEXT,
  facts TEXT,                                -- ObservedEvent, observe and found only
  matches_approved INTEGER,                  -- reconcile and found only
  evidence_id TEXT,                          -- names observation_evidence; no foreign key (R4)
  observed_at_ms INTEGER NOT NULL) STRICT;
CREATE INDEX observations_request ON observations(request_id, observed_at_ms);

-- Not projection (NOT_PROJECTION). The event as the provider returned it and its before-image.
CREATE TABLE observation_evidence (
  evidence_id TEXT PRIMARY KEY,
  digest TEXT NOT NULL,
  content BLOB NOT NULL) STRICT;

-- The event of an applied change (P4), and whether invitations were sent.
ALTER TABLE effects ADD COLUMN event_id_after TEXT;
ALTER TABLE effects ADD COLUMN revision_after TEXT;
ALTER TABLE effects ADD COLUMN notification TEXT;  -- sent | not_requested | unknown
```

Notes for the builder:

- `ALTER TABLE ... ADD COLUMN` rewrites the stored `CREATE TABLE` text of `effects`, so `schema-v2.sql` is the
  whole normalized dump after migrations 1 and 2, not version 1 plus additions (J-store).
- The `ALTER` gives existing rows NULL in the live store, and replay gives them NULL too, because replay applies
  every earlier row before it runs this SQL (ST `ledger.rs:350-381`).
- `held_result_payloads` references `held_results`, so the payload is written after `HeldResultStored` in the same
  transaction. `observations.evidence_id` has no foreign key (R4, AD7).
- No table of this migration is seeded.

#### A.4.2 The tables, what writes them and what answers which request

| Table | Projection | Written by | Read by (store) | Answers |
|---|---|---|---|---|
| `enrollments` | yes | `EnrollmentPut` | `enrollment`, `enrollment_by_key`, `agent_views`, `agent_view`; G4, G5, G7, G8 | LR-S1, LR-S2, LR-C1 |
| `enrollment_grants` | yes | `EnrollmentGrantInserted`; `GrantEnded` (G3) | G1, G2, G6 | LR-S3, CO13 |
| `held_results` | yes | `HeldResultStored`, `HeldResultPurged` | `held_result_of`, `unpurged_held_results_of_account`, `unpurged_held_results_of_agent`, `next_purge_due_ms`, `due_purges` | ST4, LR-C4, CO5 |
| `held_result_payloads` | no | `WriteTx::put_held_payload`; deleted by the apply of `HeldResultPurged` and by `backup::take` | `held_payload` | ST4 |
| `releases` | yes | `ResultReleased` | `release_of`, `releases_of_subject` | LR-C4, LR-C5, ST4, CO13 |
| `read_attempts` | yes | `ReadIntent`, `ReadFinished` | `read_attempts` | CO5, PO3 |
| `system_proofs` | yes | `SystemProofRecorded` | `system_proofs_of` | CO9, PO8 |
| `observations` | yes | `ObservationRecorded` | `observations_of`, `latest_observation` | ST3, PO2, PO4, CO6, CO8 |
| `observation_evidence` | no | `WriteTx::put_evidence` | `evidence` | ST3 |
| `effects` (3 columns) | yes | `EffectEventRecorded` | `effect_event` only, never `EFFECT_COLUMNS` (R3) | ST5, P4 |

New values in existing TEXT columns, no migration needed (none of these columns has a CHECK on values,
`0001_initial.sql`): `sessions.end_reason` `agent_removed`; `authorizations.void_reason` `agent_removed`;
`requests.state` `completed`, `resolved_as_applied`, `resolved_as_absent`. The audit kind `handshake_failed` needs
no migration, nor do `provider_mismatch` and `provider_differs` (B.14) (`audit.kind` has no CHECK; 0.4.8).

#### A.4.3 New variants and their guards

Every member that is an id or a time is made by the core (R8). Every row type is a new type, so its encoding is
free now and frozen from the merge (R6). A refused guard is `StoreFault::Invariant` and halts (W04 3.5).

| Variant | Members | Writes | Guards |
|---|---|---|---|
| `EnrollmentPut` | `enrollment: EnrollmentRow` | `enrollments` | Columns equal the row (`state`, `public_key`, `revision`, `added_at_ms`). A new row: state `pending`, revision 1, no key, `key_proven_at_ms`, `approved_at_ms`, `removed_at_ms` all null, and no `agents` row has its id. An existing row: revision rises by exactly 1; `enrollment_id`, `registered_name`, `destination`, `added_at_ms` never change. Moves: `pending -> pending` (the key may change; `key_proven_at_ms` is set exactly when the key is set or changed), `pending -> approved` (key present and equal to the previous row's, `approved_at_ms` set, an `agents` row of that id exists), `pending -> removed` and `approved -> removed` (key unchanged, `removed_at_ms` set). Nothing leaves `removed`. A query refuses a key another enrollment holds, with `StoreFault::Invariant`; a removed enrollment keeps its key, so a removed agent's key is never accepted again; a key replaced while pending is free. The unique index stays as the second line |
| `EnrollmentGrantInserted` | `grant: Grant` | `enrollment_grants` | Revision 1, not ended; origin `Enrollment { enrollment_id }` with `enrollment_id == grant.agent_id`; tools exactly `[fabric_calendar_list]`; `scope.calendars` empty and `window`, `fields`, `recipients`, `send_updates` null; term `until_revoked`; purpose `""`; the agent has an `agents` row and either an `approved` enrollment or none (a seeded agent, V13); no earlier row for the agent, tested by a query that refuses with `StoreFault::Invariant` (the unique index stays as the second line); no `grants` row has the same `grant_id` (AD9) |
| `ResultReleased` | `release: ReleaseRow` | `releases` | The releasing request exists, belongs to `agent_id`, is in `proving_fresh_permit` and has no release yet. The subject exists and belongs to the same agent. The agent's enrollment row, when it has one, is `approved`. `proof_id` names the newest `proof_reports` row of the releasing request (the `ProofRecorded` before it in this transaction, B.4.1 step 1; proof ids alone repeat, `0001_initial.sql:91`), and that row has purpose `fresh_permit`, outcome `permit`, prover `isabelle`; `grant_ids` equals its `authorizing_grants`; its fence `pause_epoch` and `revocation_epoch` equal `counters` now (AD8). `grant_ids` are grants of the agent (`grants_of`, G2). `grant_ids` is empty exactly when `kind` is `effect`. The kind fits the tools: `calendars` exactly when the releasing tool is `fabric_calendar_list`, and then `subject_request_id = request_id`; otherwise the releasing tool is `fabric_request_result` and `json_extract(action, '$.arguments.request_id')` of the releasing request equals `subject_request_id`; `availability` and `events` need the subject's tool `fabric_calendar_availability` and `fabric_calendar_read` and a held result of that kind; `effect` needs a subject whose tool is one of the five change tools, in `applied` or `reconciled`, with an `effects` row whose `notification` is set. `held_result_id` is set exactly for `availability` and `events` and names an unpurged held result of the subject. `result_digest` is set exactly for `calendars` and `effect` |
| `HeldResultStored` | `held: HeldResultRow` | `held_results` | The request is a `fabric_calendar_availability` or `fabric_calendar_read` of `agent_id`, in `dispatching`; its last `read_attempts` row is finished with outcome `held` (the `ReadFinished` before it in this transaction, A.4.8); the authorization that row names is consumed and its fence epochs equal `counters` now (AD8); no held result for the request yet; not purged; `kind` matches the tool; `purge_due_at_ms > stored_at_ms` (the core sets it from `HELD_RESULT_LIFETIME_MS`, R8). The commit check of A.4.4 requires its payload row |
| `HeldResultPurged` | `held_result_id: Id, at_ms: i64, cause: PurgeCause` | `held_results`; deletes the `held_result_payloads` row (AD5) | The row exists and is not purged. Sets `purged_at_ms` and `purge_cause`, then `DELETE FROM held_result_payloads WHERE held_result_id = ?` (no row is not an error: replay and a backup copy have none) |
| `ReadIntent` | `attempt: ReadAttemptRow` | `read_attempts` | The request is an availability or read request in `queued`. The authorization is the request's, `consumed` (the `AuthorizationConsumed` before it in this transaction), and named by no `attempts` or `read_attempts` row. `number` is one more than the request's last attempt, and that attempt is finished |
| `ReadFinished` | `request_id: Id, number: u32, finished_at_ms: i64, outcome: ReadOutcome` | `read_attempts` | The attempt exists and is unfinished; `finished_at_ms >= started_at_ms`. `finished_at_ms` is the commit's time on the core clock (`cx.now_ms`), as `AttemptFinished` takes it (CORE `service/pipeline.rs:924-929`), never `ReadDone.finished_at_ms` |
| `EffectEventRecorded` | `effect_id: Id, event: Option<EventRef>, notification: Notification, at_ms: i64` | `effects.event_id_after`, `revision_after`, `notification` | The effect exists; its request is in `dispatching` (an attempt's answer) or `reconciling` (a reconcile answer). `event` is none exactly for a delete. For a create `event.event_id` equals `provider_event_id`; for update, invitation and response it equals the id that `open_effects` computes (ST `read.rs:927-938`): the action's target event, and `instance_id` for a response to one occurrence. May overwrite an earlier value |
| `SystemProofRecorded` | `proof: SystemProofRow` | `system_proofs` | `serves_request_id`, when set, exists; `prover_kind` is a known kind; `action_kind` matches `record.action` |
| `ObservationRecorded` | `observation: ObservationRow` | `observations` | The request exists. `proof_id` names a `system_proofs` row serving this request with outcome `permit` and prover `isabelle`. `revision` is set exactly when `outcome` is `found` or `cancelled`; `facts` only for `observe` and `found`; `matches_approved` only for `reconcile` and `found`. The commit check of A.4.4 requires the evidence row when `evidence_id` is set |

The row types (ST `rows.rs`), as 0.5.2 fixes them: `EnrollmentState`,
`EnrollmentRow`, `ReleaseKind`, `ReleaseRow`, `HeldResultRow`, `PurgeCause`, `ReadAttemptRow`, `ReadOutcome`,
`Notification`, `SystemProofRow`, `SystemProofRecord`, `ObservationRow` and the enums `ObservationPurpose`,
`ObservationOutcome`, `SystemActionKind`, all snake_case text. Their `Option` members are written as `null`: these
are new types whose canonical form is fixed at merge, and R6 applies to members added after.

Two shared types are stored as JSON and fixed here (X21; canonical form, keys in order):

| Type | Stored JSON |
|---|---|
| `ObservedEvent` (`observations.facts`) | `{"attendees": [text], "calendar": {"account_id": id, "calendar_id": id}, "default_type": bool, "event_id": text, "fabric_created": bool, "invitation": bool, "occurrence_start_ms": integer or null, "recurring": bool, "revision": text, "timed": bool}` |
| `SystemAction` (`SystemProofRecord.action`) | tagged by `kind`: `{"calendar": calendar_ref, "kind": "refresh"}`, `{"calendar": calendar_ref, "event_id": text, "kind": "reconcile"}`, `{"account_id": id, "kind": "list_calendars"}` |

Two unit tests pin the stored guard of `HeldResultStored`: `ReadFinished(held)`, `HeldResultStored`,
`put_held_payload` and `RequestMoved` applied in that order commit; `HeldResultStored` after `ReadFinished(stale)` is
refused (A.8.2).

#### A.4.4 Store calls outside `apply`, and the commit check

```rust
impl WriteTx<'_> {
    /// Writes a held result's content. Refused unless this transaction applied `HeldResultStored` with this id.
    /// The store computes the sha256 of `content` and refuses a `digest` that differs.
    pub fn put_held_payload(&mut self, held_result_id: &Id, digest: &Digest, content: &[u8]) -> Result<(), StoreFault>;
    /// Writes observation evidence, at most MAX_EVIDENCE_BYTES (1_048_576). Same digest rule. The caller checks
    /// the size first (D.6.2), because a fault here poisons the write.
    pub fn put_evidence(&mut self, evidence_id: &Id, digest: &Digest, content: &[u8]) -> Result<(), StoreFault>;
}
impl ReadTx<'_> {
    /// The content, or None when the row is missing or its digest does not match (fail closed).
    pub fn held_payload(&self, held_result_id: &Id) -> Result<Option<Vec<u8>>, StoreFault>;
    pub fn evidence(&self, evidence_id: &Id) -> Result<Option<Vec<u8>>, StoreFault>;
    /// user_version >= version, read with PRAGMA user_version at each call (apply::user_version, ST apply.rs:770-775);
    /// never kept across mutations, because replay raises user_version between two mutations of one connection
    /// (ST ledger.rs:353-378). The gate of G1 to G9.
    pub(crate) fn schema_at_least(&self, version: u32) -> Result<bool, StoreFault>;
}
impl Store {
    /// PRAGMA wal_checkpoint(TRUNCATE); Ok(false) when busy (AD13). Not a fault.
    pub fn checkpoint(&mut self) -> Result<bool, StoreFault>;
}
```

The commit check (AD6), in `Store::write` after the closure and before the ledger row is appended (ST
`store.rs:400-410`): for every `HeldResultStored` among the transaction's mutations a `held_result_payloads` row of
that id exists; for every `ObservationRecorded` with an `evidence_id` an `observation_evidence` row of that id
exists; and every evidence row written by `put_evidence` in this transaction is named by an `ObservationRecorded` of
it. A failure is `StoreFault::Invariant` and nothing commits. Both calls go through `usable()` and poison the
write on a fault, as `apply` does (ST `tx.rs:244-264`).

An `ObservationRecorded` may name an evidence row that an earlier commit wrote (the dedupe of D.6.2); the check
then finds that row. So one evidence row may be named by observations of several requests. No evidence row is
deleted while any observation names it; a later deletion per request must first check that no other observation
names the row (ruled 2026-11-18 by the lead). The evidence of a V23 attempt (B.14) is named by no observation;
the name the ledger gives it is fixed with P2-32, before the live run, and this check is widened then.

The fitness test s16 (`crates/fabric-storage/tests/fitness.rs`) gains a second scan: the two non-projection tables
are written only in `tx.rs` (the two calls), `apply.rs` (the delete of AD5) and `backup.rs` (the delete of A.7).
The scan's prefix list (`crates/fabric-storage/tests/fitness.rs:63-66`, today `main` and `temp`) gains `backup`, so
`DELETE FROM backup.held_result_payloads` is seen.

#### A.4.5 Existing variants, and the branches gated at version 2

No existing variant gains a member. Changes are new enum values and the gated branches below. Every branch tests
`ReadTx::schema_at_least(2)`. "Unreachable before 2" is the reason no row committed before migration 2 can take the
branch; the test is the replay of the frozen version 1 stores (A.8) plus a unit test of the branch at version 2.

| # | Existing code | Branch at `user_version >= 2` | Unreachable before 2 because |
|---|---|---|---|
| G1 | `ReadTx::grant` (ST `read.rs:750`), used by the apply of `GrantEnded` (ST `apply.rs:512-546`) | When `grants` has no row of the id, look in `enrollment_grants` | Before migration 2 the table does not exist in the replay database, and no earlier row names an id that is only in it (I24) |
| G2 | `ReadTx::grant_agent` (ST `read.rs:762`), used by `grants_of` (ST `apply.rs:822-834`) for `ReliedGrantSet`, `AuthorizationIssued` and the new `ResultReleased` | Same lookup | Same |
| G3 | The apply of `GrantEnded` | When the grant is in `enrollment_grants`, update that row (revision rises by 1, `record`, `ended_at_ms`, `end_reason`); the reason must be `revoked_by_owner` or `agent_removed` | No earlier row ends a grant of that table |
| G4 | The apply of `GrantInserted` (ST `apply.rs:481-510`, as DRAFT edits it) | Refuse when the agent has an `enrollments` row that is not `approved`, or when `enrollment_grants` holds the same `grant_id`. DRAFT's refusal of an enrollment origin stays | No `enrollments` or `enrollment_grants` row exists before migration 2 |
| G5 | The apply of `SessionStarted` (ST `apply.rs:105-125`) | Refuse when the agent has an `enrollments` row that is not `approved` | Same |
| G6 | `ReadTx::live_grants`, `ReadTx::grants_of_agent` (ST `read.rs:772, 785`); read by the core only, never by a guard | Union with `enrollment_grants`, in insertion order of each table, `grants` first | Not a replay path; the gate only keeps a short-list test store readable |
| G7 | The apply of `AgentPut` (ST `apply.rs:86-103`) | A new `agents` row whose id has an `enrollments` row is allowed only when that enrollment is `pending` with a key (the approval commit, A.4.8). An update of an agent whose enrollment is `removed` is refused | No `enrollments` row exists before migration 2 |
| G8 | The apply of `RequestInserted` (ST `apply.rs:316-347`) | Refuse when the agent has an `enrollments` row that is not `approved` | Same |
| G9 | The apply of `EffectReserved` (ST `apply.rs:641-669`) | Refuse a tool that changes no calendar: `fabric_calendar_list`, `fabric_calendar_availability`, `fabric_calendar_read`, `fabric_request_result` | Main accepts a fresh permit only for a change (`context.change`, CORE `service/pipeline.rs:159-168`; `is_change`, CORE `service/requests.rs:108-117`), and only that path reserves an effect (CORE `service/pipeline.rs:848-872`) |

Other existing variants:

| Variant | Change |
|---|---|
| `SessionEnded` | `reason` may be `SessionEndReason::AgentRemoved` (text `agent_removed`); `grant_end_reason(AgentRemoved) = GrantEndReason::AgentRemoved` (ST `mutation.rs:258-285`) |
| `AuthorizationVoided` | `reason` may be `VoidReason::AgentRemoved` (ST `mutation.rs:290-301`) |
| `RequestMoved` | P0 adds its optional cause member; phase 2 adds the undecided causes and the state `completed` (0.4.4, 0.4.5). The four causes are added to the stored cause type in step P3 and first written in step P5. `can_transition_to` and `is_terminal` are the contract's (R7) |
| `AccountPut`, `AttemptFinished`, `DispatchIntent`, `ProofRecorded` and the rest | Unchanged. `AttemptFinished` keeps its shape; the applied event is recorded by `EffectEventRecorded` in the same `request.attempt` commit (P4) |

`SessionEndReason::AgentRemoved`, `VoidReason::AgentRemoved` and the four undecided causes of 0.4.4 are added in the
step P3 pull request; nothing writes them before steps P5 and P6 (R6).

`EFFECT_COLUMNS` (ST `read.rs:212`) gains no column (R3). A unit test pins its text.

#### A.4.6 The readers that see two tables

Every reader of grants sees both tables at version 2 (P5).

| Store reader | Gate | Its callers |
|---|---|---|
| `ReadTx::grant` | G1 | apply of `GrantEnded` (ST `apply.rs:518`); revoke (CORE `owner/commands.rs:227`); `end_grant` (CORE `service/grants.rs:93, 109`); grant validity before the handoff (CORE `service/pipeline.rs:272`) |
| `ReadTx::grant_agent` | G2 | `grants_of` for `ReliedGrantSet`, `AuthorizationIssued`, `ResultReleased` (ST `apply.rs:829`) |
| `ReadTx::live_grants` | G6 | the snapshot and its budget (CORE `owner/views.rs:68, 76`: an enrollment grant counts into the 720 KiB budget, so `approve_agent` may be refused `invalid_params`, section C); the endings by set and by account (CORE `service/grants.rs:137, 160`) |
| `ReadTx::grants_of_agent` | G6 | the facts for proofs (CORE `service/facts.rs:124`); the session's endings (CORE `service/sessions.rs:86`); the one-request ends at a terminal move (CORE `service/requests.rs:44`) |
| Invariants I6b, I10, I24 | since 2 | open (ST `store.rs:316-317`) |
| Invariant I4 (session grant without its session) | none | reads `grants` only; an enrollment grant is never a session grant, so it needs no change |

Core code that reads `grant.origin.request_id` (CORE `service/pipeline.rs:276`, `service/requests.rs:49`,
`service/facts.rs:38, 70`) must handle `GrantOrigin::Enrollment` (no origin request): an enrollment grant has no open
origin and is never a one-request grant. That is section C's edit; the store gives the record as stored.

Branches that read `enrollments` only: G4, G5, G7, G8, and the new readers `enrollment`, `enrollment_by_key`,
`agent_view` and `agent_views` (pending, approved and removed agents, and seeded agents with no enrollment row).

#### A.4.7 Open invariants

`INVARIANTS` (ST `store.rs:922-966`) becomes a list of `Invariant { check, since, until: Option<u32>, sql }`;
`check_invariants(connection, version)` (ST `store.rs:968-976`) is called with the store's `user_version` and
skips an entry with `since > version` or `until < version`. Each query must return no row.

| # | Check | Since | Until | SQL (abridged where it repeats today's) |
|---|---|---|---|---|
| I1 | request state | 1 | | today's query with the 26 states of 0.4.5; terminal set: `blocked`, `undecided`, `denied`, `expired`, `cancelled`, `stale`, `reconciled`, `terminal_failure`, `absence_confirmed`, `resolved_as_applied`, `resolved_as_absent`, `completed`. The edit rides the contract's pull request that adds the states (DRAFT already edits these lines, DRAFT `store.rs:926-933`; `completed` comes with P2-1), not migration 2 (P3) |
| I2 to I5, I7 | as today | 1 | | unchanged |
| I6 | relied grant of another agent | 1 | 1 | today's query (`grants` only) |
| I6b | relied grant of another agent | 2 | | `SELECT 1 FROM requests r, json_each(r.relied_grant_ids) j WHERE NOT EXISTS (SELECT 1 FROM grants g WHERE g.grant_id = j.value AND g.agent_id = r.agent_id) AND NOT EXISTS (SELECT 1 FROM enrollment_grants e WHERE e.grant_id = j.value AND e.agent_id = r.agent_id) LIMIT 1` |
| I8 | agent row of a never approved enrollment | 2 | | `SELECT 1 FROM agents a JOIN enrollments e ON e.enrollment_id = a.agent_id WHERE json_extract(e.record, '$.approved_at_ms') IS NULL LIMIT 1` |
| I9 | approved enrollment without its agent | 2 | | `SELECT 1 FROM enrollments e WHERE json_extract(e.record, '$.approved_at_ms') IS NOT NULL AND NOT EXISTS (SELECT 1 FROM agents a WHERE a.agent_id = e.enrollment_id) LIMIT 1` |
| I10 | live grant of an agent not approved | 2 | | `SELECT 1 FROM enrollments e WHERE e.state <> 'approved' AND (EXISTS (SELECT 1 FROM grants g WHERE g.agent_id = e.enrollment_id AND g.ended_at_ms IS NULL) OR EXISTS (SELECT 1 FROM enrollment_grants x WHERE x.agent_id = e.enrollment_id AND x.ended_at_ms IS NULL)) LIMIT 1` |
| I11 | live session of an agent not approved | 2 | | `SELECT 1 FROM sessions s JOIN enrollments e ON e.enrollment_id = s.agent_id WHERE s.ended_at_ms IS NULL AND e.state <> 'approved' LIMIT 1` |
| I12 | enrollment state | 2 | | `SELECT 1 FROM enrollments WHERE state NOT IN ('pending', 'approved', 'removed') OR state IS NOT json_extract(record, '$.state') OR public_key IS NOT json_extract(record, '$.public_key') OR revision IS NOT json_extract(record, '$.revision') LIMIT 1` |
| I13 | enrollment grant shape | 2 | | `SELECT 1 FROM enrollment_grants WHERE json_extract(record, '$.origin.enrollment_id') IS NOT agent_id OR json_extract(record, '$.tools') IS NOT '["fabric_calendar_list"]' OR json_array_length(record, '$.scope.calendars') IS NOT 0 OR json_extract(record, '$.scope.window') IS NOT NULL OR json_extract(record, '$.scope.fields') IS NOT NULL OR json_extract(record, '$.scope.recipients') IS NOT NULL OR json_extract(record, '$.scope.send_updates') IS NOT NULL OR json_extract(record, '$.term.kind') IS NOT 'until_revoked' OR json_extract(record, '$.purpose') IS NOT '' LIMIT 1` |
| I14 | enrollment grant end | 2 | | `SELECT 1 FROM enrollment_grants WHERE end_reason IS NOT NULL AND end_reason NOT IN ('revoked_by_owner', 'agent_removed') LIMIT 1` |
| I15 | completed without one release | 2 | | `SELECT 1 FROM requests r WHERE r.state = 'completed' AND NOT EXISTS (SELECT 1 FROM releases x WHERE x.request_id = r.request_id) UNION ALL SELECT 1 FROM releases x JOIN requests r ON r.request_id = x.request_id WHERE r.state <> 'completed' LIMIT 1` |
| I16 | release of another agent | 2 | | `SELECT 1 FROM releases x JOIN requests a ON a.request_id = x.request_id JOIN requests b ON b.request_id = x.subject_request_id WHERE a.agent_id <> x.agent_id OR b.agent_id <> x.agent_id LIMIT 1` |
| I17 | held result of a non-read | 2 | | `SELECT 1 FROM held_results h JOIN requests r ON r.request_id = h.request_id WHERE json_extract(r.action, '$.tool') NOT IN ('fabric_calendar_availability', 'fabric_calendar_read') OR r.agent_id <> h.agent_id LIMIT 1` |
| I18 | payload of a purged result | 2 | | `SELECT 1 FROM held_result_payloads p JOIN held_results h ON h.held_result_id = p.held_result_id WHERE h.purged_at_ms IS NOT NULL LIMIT 1` |
| I19 | read with an effect | 2 | | `SELECT 1 FROM effects WHERE tool IN ('fabric_calendar_list', 'fabric_calendar_availability', 'fabric_calendar_read', 'fabric_request_result') LIMIT 1` |
| I20 | observation evidence missing | 2 | | `SELECT 1 FROM observations o WHERE o.evidence_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM observation_evidence v WHERE v.evidence_id = o.evidence_id) LIMIT 1` |
| I21 | effect event of a create | 2 | | `SELECT 1 FROM effects WHERE tool = 'fabric_calendar_propose_create' AND event_id_after IS NOT NULL AND event_id_after IS NOT provider_event_id LIMIT 1` |
| I24 | grant id in both tables | 2 | | `SELECT 1 FROM grants g JOIN enrollment_grants e ON e.grant_id = g.grant_id LIMIT 1` |
| I25 | authorization of two attempts | 2 | | `SELECT 1 FROM read_attempts r JOIN attempts a ON a.authorization_id = r.authorization_id LIMIT 1` |
| I26 | release kind and grants | 2 | | `SELECT 1 FROM releases x JOIN requests s ON s.request_id = x.subject_request_id WHERE (x.kind = 'effect') <> (json_extract(s.action, '$.tool') LIKE 'fabric_calendar_propose_%') OR (x.grant_ids = '[]') <> (x.kind = 'effect') LIMIT 1` |
| I27 | stored enum text | 2 | | `SELECT 1 FROM held_results WHERE kind NOT IN ('availability', 'events') OR purge_cause NOT IN ('released', 'expired', 'account_disconnected', 'agent_removed') UNION ALL SELECT 1 FROM releases WHERE kind NOT IN ('calendars', 'availability', 'events', 'effect') UNION ALL SELECT 1 FROM read_attempts WHERE outcome NOT IN ('held', 'stopped', 'stale', 'transient', 'rejected', 'no_answer_in_time') UNION ALL SELECT 1 FROM system_proofs WHERE action_kind NOT IN ('refresh', 'reconcile', 'list_calendars') OR prover_kind NOT IN ('isabelle', 'synthetic') UNION ALL SELECT 1 FROM observations WHERE purpose NOT IN ('observe', 'reconcile') OR outcome NOT IN ('found', 'cancelled', 'absent') UNION ALL SELECT 1 FROM effects WHERE notification NOT IN ('sent', 'not_requested', 'unknown') LIMIT 1`. A NULL column yields no row |

Not an invariant, on purpose: an unpurged held result without its payload. A backup copy has none (A.7), and the
reader fails closed. At version 2 the list has 26 entries and 25 apply (I6 is skipped).

#### A.4.8 Commit orders the store checks

Each is one SQLite transaction and one ledger row. The order matters where a guard reads what an earlier mutation
wrote.

| Operation type | Order |
|---|---|
| `owner.approve_agent` | (1) `AgentPut`, new row, revision 1, `enrolled_at_ms` the approval time (G7 allows it: the enrollment is `pending` with a key); (2) `EnrollmentPut` `approved` (needs the `agents` row and the unchanged key); (3) the notice `Change::Agent`; (4) `EnrollmentGrantInserted` through `grants::create_enrollment_grant` (CORE), which sends `Change::Grant`. The contract revision of the agent is `enrollments.revision + agents.revision` (V14) |
| `owner.remove_agent` | (1) `SessionEnded` `agent_removed` for a live session and the ends of its session grants; (2) `GrantEnded` `agent_removed` for every other live grant of the agent, the enrollment grant (G3) and read one-request grants included; (3) `AuthorizationVoided` `agent_removed` for each issued authorization, then `RequestMoved` to `cancelled` for every request not yet handed to the provider; (4) `HeldResultPurged { cause: AgentRemoved }` for each unpurged held result of the agent; (5) when no grant ended in (1) and (2), one `CounterRaised` of `revocation_epoch` (C.3.3 step 5; P10); (6) `EnrollmentPut` `removed`; (7) notices, the last of which is `Change::Agent` with the removed record (an agent notice from (1) may come earlier, C.3.3; X1). Each grant end raises `revocation_epoch` (`CounterRaised`) |
| `request.read` | `ReadFinished`; for outcome `held`: `HeldResultStored`, then `put_held_payload`, then `RequestMoved` `dispatching -> applied` and `applied -> reconciled`; otherwise the moves of section B. A move `dispatching -> retry_wait` of a request whose agent's enrollment is not approved is followed in the same commit by `RequestMoved` `retry_wait -> cancelled` (`AgentRemoved`, X22) |
| `request.release` | `ProofRecorded`, `ReliedGrantSet`, then `ResultReleased` before any `GrantEnded` or `CounterRaised` (its fence check reads `counters` before them); `HeldResultPurged { cause: Released }` for a held result; `GrantEnded` `consumed` of the read's one-request grant and its `CounterRaised`; `RequestMoved` to `completed` (B.4.1) |
| `timer.result_purge` | per due row: `HeldResultPurged { cause: Expired }`, then `GrantEnded` of the read one-request grant (`consumed` until P2-4 merges, then `result_expired`; V15) |
| `request.attempt` | as today, plus `EffectEventRecorded` after `AttemptFinished` when the provider answered an `Applied` that fits the order: a confirmed one before `dispatching -> applied -> reconciled`, an unconfirmed one of V23 before its audit row `provider_differs` and its moves to `needs_owner_resolution` (B.14). A `Transient` that moves a removed agent's change `dispatching -> retry_wait` also moves it `retry_wait -> cancelled` (X22) |
| `request.dispatch` (a read) | `AuthorizationConsumed`, `ReadIntent`, `RequestMoved` `queued -> dispatching` |

After a commit that applied any `HeldResultPurged`, the core calls `Store::checkpoint()` (AD13).

#### A.4.9 What replay does with a store written by main

A store written by main (store version 1) opened by the build of migration 2, in the order of `Store::open` (ST
`store.rs:240-332`):

1. Step 4 probes `user_version` 1; step 5 passes (1 is not above 2).
2. Step 8 checks the created store. `verify_chain` accepts it: the version 2 list of types is a superset of the 22
   (ST `ledger.rs:25-48, 291-293`), and `payload_version` stays 1.
3. Step 9 takes the backup `broker-v1-{store_id}-h{head}-{at_ms}.db` (ST `backup.rs:21-56`), with no delete of held
   payloads (its version argument is 1, so the copy has no such table, A.7), and verifies it:
   `verify_chain`, then `verify_replay` with the version 2 migration list. The replay database runs migration 1's
   SQL at row 1, sets `user_version` 1, and applies every row with this build's apply. No gate is open (G1 to G9
   are closed below 2), no table of migration 2 exists in the replay database, and `EFFECT_COLUMNS` names only
   version 1 columns (R3). The projection compared is the 17 tables of version 1 in both databases.
4. Migration 2 runs in one transaction with the row `{SchemaMigrated 1->2}`, dated at least the last commit (W04
   3.6).
5. Step 10 runs the invariants for version 2: I6 skipped; I15, I19 and I21 read version 1 rows and find none; the rest
   of I6b and I8 to I26 run on empty tables.

Later replays of that store (`Store::verify_replay`, and the backup before migration 3) apply the rows before the
migration row at `user_version` 1, run migration 2's SQL at that row (existing `effects` rows get NULL in the three
new columns, as the live store did), set `user_version` 2, and apply the later rows with the gates open.
`verify_replay` of the live store does not run at every open; it runs on the backup before a migration and in
tests (ST `store.rs:288-297`, `backup.rs:146-171`).

Records written by main are re-encoded by the version 3 contract crate during that replay. R7 and the contract
test P2-14 make that re-encoding byte-identical; the fixture test of A.8 shows it on real rows.

#### A.4.10 What an older build does with a store written after it

- **A store at version 2, opened by a version 1 build.** Step 5 refuses it with `Unavailable::NewerSchema { found:
  2, supported: 1 }` before anything is read or written, and the file stays byte-identical (ST `store.rs:265-267`;
  `crates/fabric-storage/tests/open.rs:230-260`). The broker stays up, answers `hello` and answers every other
  owner method `store_unavailable` (W04 D8; `crates/fabric-broker-core/tests/faults.rs:300-330`). No version 1 code
  ever decodes a version 2 type, value or row.
- **The way back.** There is no reverse migration. The owner restores the backup taken before the migration by hand
  (W04 3.6). That backup is a version 1 store; everything written after the migration is lost.
- **A version 1 store written by a later version 1 build** (for example after P0 adds its optional member) and
  opened by an earlier one is not refused. That case is P0's tolerant reader and is not relied on here.

---------------------------------------------------------------------------------------------------------------------

### A.5 Migration 3

`crates/fabric-storage/migrations/0003_google.sql`, `SCHEMA_VERSION = 3`, frozen `schema-v3.sql`, built with the
Google core (step P7a, with step P7b after it). It is designed as far as W05 section 11 and the review allow.

```sql
-- Store schema version 3: the Google side of an account (W05 ST1, ST2). Only adds (A.2 R1). Holds no token.
CREATE TABLE provider_accounts (
  account_id TEXT PRIMARY KEY REFERENCES accounts,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  subject TEXT,                              -- Google's subject; null until the first Connected outcome
  record TEXT NOT NULL) STRICT;              -- ProviderAccountRow
CREATE UNIQUE INDEX provider_accounts_subject ON provider_accounts(subject) WHERE subject IS NOT NULL;

CREATE TABLE provider_calendars (
  account_id TEXT NOT NULL REFERENCES provider_accounts,
  calendar_id TEXT NOT NULL,                 -- 'cal-' + 32 hex, made by the core
  provider_calendar_id TEXT NOT NULL,        -- Google's id; never reaches an agent or the page
  revision INTEGER NOT NULL CHECK (revision >= 1),
  record TEXT NOT NULL,                      -- ProviderCalendarRow
  PRIMARY KEY (account_id, calendar_id)) STRICT;
CREATE UNIQUE INDEX provider_calendars_google ON provider_calendars(account_id, provider_calendar_id);
```

| Variant | Members | Guards |
|---|---|---|
| `ProviderAccountPut` | `row: ProviderAccountRow` | The `accounts` row exists. A new row is revision 1; an existing row's revision rises by exactly 1. `subject`, once set, never changes. The row type has no token member; `token_ref` is the Keychain account name `refresh-token-v1.<account id>` (W05 section 4). `last_connect_problem` is a fixed label (`ConnectProblem`), never Google's text |
| `ProviderCalendarPut` | `row: ProviderCalendarRow` | The `provider_accounts` row exists. A new row is revision 1; an existing row's revision rises by exactly 1. `calendar_id` and `provider_calendar_id` never change. Rows are never deleted; a lost calendar is `listed: false`. No count is checked here (R8): the core's list commit refuses a list past `MAX_CALENDARS_PER_ACCOUNT` (D.9.2) |

| # | Check | Since | SQL |
|---|---|---|---|
| I22 | provider calendar of a record | 3 | `SELECT 1 FROM accounts a JOIN provider_accounts p ON p.account_id = a.account_id, json_each(a.record, '$.calendars') c WHERE NOT EXISTS (SELECT 1 FROM provider_calendars pc WHERE pc.account_id = a.account_id AND pc.calendar_id = json_extract(c.value, '$.calendar_id')) LIMIT 1` |
| I23 | connected account without subject | 3 | `SELECT 1 FROM accounts a JOIN provider_accounts p ON p.account_id = a.account_id WHERE a.state = 'connected' AND p.subject IS NULL LIMIT 1` |

Migration 3 names no gated branch (R2): no existing variant reads a table of migration 3. The core reads them; the
practice account and every account with no `provider_accounts` row is "not a Google account" (REV-store ST1). If
the CD-W05 work needs an existing variant to read them, the branch is added to R2's list before the migration
merges. `PROJECTION_TABLES` becomes 26 names; `NOT_PROJECTION` is unchanged.

`observations` and `observation_evidence` stay in migration 2: the lead accepted AD3 on 2026-11-17 (OP1).

The operation types of migration 3 are those of A.6. Its frozen store of version 2 is made by the author of step
P7a's pull request from the last main commit at store version 2, as A.8 describes for version 1. The secrecy test of
ST9 (marker tokens) ships with it.

What stays open for migration 3:

| Point | Owner |
|---|---|
| The members of `ProviderAccountRow` and `ProviderCalendarRow` beyond those of 0.5.2; they are frozen at merge and only widen after | CD-W05 session with section D |
| `ConnectOutcome`'s variants: written in D.2 (OP15 closed); the content of a `provider.connect` commit is D.8.2's | section D |
| `owner.refresh_calendars` exists only if the contract adds the command (P2-11) | contract session |
| Whether `missing_scope` of the snapshot (P2-9) is derived from `granted_scopes` (this section's assumption) or stored | section D |
| The cap on accounts (ST10, OQ14) | owner |

---------------------------------------------------------------------------------------------------------------------

### A.6 Ledger operation types (one dated amendment of ADR-115 D2)

Today's 22 (ST `ledger.rs:25-48`) stay. `OPERATION_TYPES` becomes `[&str; 34]` with migration 2 and `[&str; 39]`
(40 with P2-11) with migration 3. A type ships in the pull request of its migration (R10).

| # | Type | Since | Actor | Correlation | What the commit holds |
|---|---|---|---|---|---|
| 1-22 | today's | 1 | as today | as today | as today, with the additions below |
| 23 | `owner.add_agent` | 2 | owner | null | `EnrollmentPut` pending; `CommandAnswered` |
| 24 | `owner.approve_agent` | 2 | owner | null | A.4.8 |
| 25 | `owner.remove_agent` | 2 | owner | null | A.4.8 |
| 26 | `owner.select_calendars` | 2 | owner | null | `AccountPut`, the grant ends and stales of P10, `CounterRaised` |
| 27 | `owner.resolve_outcome` | 2 | owner | the request id (AD14) | `RequestMoved` to `resolved_as_applied` or `resolved_as_absent` and the ends of its one-request grants (D.7.5) |
| 28 | `agent.prove_key` | 2 | agent, the enrollment id (AD15) | null | `EnrollmentPut` `pending -> pending` with the proved key |
| 29 | `request.read` | 2 | broker | the request id | A.4.8 |
| 30 | `request.release` | 2 | broker | the releasing request id | A.4.8. A commit entered as a release: it holds a `releases` row and `completed`, or `undecide(result_not_held \| result_too_large)` and no row (B.4.1 steps 2 and 3). A release whose call is gone is `request.proof` (V10) |
| 31 | `request.reconcile` | 2 | broker | the request id | `ObservationRecorded`, `EffectEventRecorded`, `RequestMoved` |
| 32 | `system.proof` | 2 | broker | the served request id or null | `SystemProofRecorded`; when the proof does not permit, also the move or the new due time of the served request (B.11.3, D.6.3, D.7.4) |
| 33 | `provider.observation` | 2 | broker | the request id | `ObservationRecorded` and the moves of W05 CO6 |
| 34 | `timer.result_purge` | 2 | broker | null | A.4.8 |
| 35 | `owner.connect_google` | 3 | owner | null | W05 CO1 |
| 36 | `provider.connect` | 3 | broker | null | a sign-in outcome (W05 CO1, PO7) |
| 37 | `provider.calendars` | 3 | broker | null | a calendar list answer, reductions included (W05 CO7) |
| 38 | `provider.auth_lost` | 3 | broker | null | an account signal `AuthLost` (W05 CO11) |
| 39 | `provider.revoke` | 3 | broker | null | the revocation answer of an `Unlink` job (`Unlinked` with an outcome; W05 CO4 d; DD18) |
| 40 | `owner.refresh_calendars` | 3 | owner | null | only if P2-11 merges |

Existing types that carry new mutations: `request.dispatch` (a read's handoff), `request.attempt`
(`EffectEventRecorded`), `owner.revoke_grant` (an enrollment grant, G3), `owner.disconnect_google`
(`HeldResultPurged { cause: AccountDisconnected }` and its grant ends), `broker.started` (purges due at start),
`broker.seed` (a seeded agent's enrollment grant, V13), `request.proof` (a release whose call is gone,
V10). `get_agent_configuration` is a query and writes no row. `delete_custom` (inside `save_profile`) and
`list_ended_requests` are not built in phase 2; they keep DRAFT's `unknown_method` answer (DRAFT CORE
`owner/profiles.rs:50-51`, `owner/mod.rs:153-165`). Deleting a custom profile needs a place in the store: a table
`deleted_profiles(profile_id TEXT PRIMARY KEY, deleted_at_ms INTEGER NOT NULL) STRICT`, a variant
`CustomProfileDeleted { profile_id, at_ms }`, and gated branches in `ProfileVersionAdded`, `ProfileStateSet` and
`ReadTx::profiles` (today `ProfileStateSet`'s guard reads `profile_versions` directly and would accept a deleted
profile). `list_ended_requests` needs a reader over terminal requests ordered by `received_at_ms` and `request_id`.
`deleted_profiles` is not in migration 2 nor in phase 2 (the lead's ruling of 2026-11-17, AO8). Idempotency keys
stay as ADR-115 D2 says: `owner_command` on the owner types, none on the others.

---------------------------------------------------------------------------------------------------------------------

### A.7 Content outside the ledger, its deletion, and what never enters the store

| Content | Where | Named in the ledger by | Deleted |
|---|---|---|---|
| A held read result (availability, event details) | `held_result_payloads.content`, its digest beside it | `held_result_id` (random), never a digest (V3) | By the apply of `HeldResultPurged`: at its release (V4), 24 hours after it was stored (`HELD_RESULT_LIFETIME_MS`, the owner's default, P13), at its account's disconnect, at its agent's removal; due rows at start ride `broker.started`. Then `PRAGMA wal_checkpoint(TRUNCATE)` (AD13). In every backup copy that has the table (version 2 and later), before the copy is verified and named (V18) |
| Observation evidence (the event as the provider returned it, the before-image), at most 1 MiB, the before-image at most 512 KiB (`MAX_EVIDENCE_BYTES`, `MAX_BEFORE_VIEW_BYTES`); an observation whose evidence equals that of the latest found observation of the same calendar, event and revision names that row and writes none (D.6.2). The event a V23 attempt returned is bounded the same way (B.14) | `observation_evidence.content` | `evidence_id` (random) | Never by a disconnect (CD-Q11h); it stays with its request. One row may be named by observations of several requests (the dedupe), so it is never deleted while any observation names it, and a later deletion per request must first check that no other observation names it. Kept outside the ledger for size only (REV-store ST3 d) |

How the purge holds:

- `secure_delete` is on for the store connection (ST `store.rs:846-847`), so a deleted payload's cells are
  overwritten in the database file.
- Pages written before the delete can remain in older WAL frames until a checkpoint. AD13 truncates the WAL after
  every purge commit; a busy checkpoint returns `false`, is not a fault, and is retried at the next purge and at
  close. With the Keychain key the WAL is encrypted by SQLCipher like the database; with the test key off macOS it
  is plain (ST `key.rs:7`). The `-shm` file holds no content (W04 R12).
- A backup is taken only before a migration (ST `store.rs:288-292`), at the version found, and one backup serves
  every pending migration. `backup::take` deletes the rows of `held_result_payloads` in the copy when the copy has
  that table, that is when its version argument (ST `backup.rs:25`) is at least 2; a version 1 copy gets no delete
  statement (V18). For a version 2 or later copy: on macOS, after `sqlcipher_export('backup')` and before `DETACH`,
  it runs `PRAGMA backup.secure_delete = ON; DELETE FROM backup.held_result_payloads;` (ST `backup.rs:100-127`);
  elsewhere it opens the `VACUUM INTO` copy read-write with the key, sets `secure_delete`, and deletes the rows
  before `verify` (ST `backup.rs:131-143, 146-171`). The table is not projection, so `verify_replay` of the copy is
  unaffected. A backup restored by hand then reads every unpurged held result as not held (OP14). The first backup
  that can hold payloads is the one before migration 3.
- The copy is never left with payloads. `take` writes it as `<name>.partial`, deletes the payload rows in it,
  verifies it, then renames it to `<name>`. Before it starts, `take` removes every file in `backups/` whose name
  contains `.partial`: the copy and its `-journal`, `-wal` and `-shm` files.
  Today a copy is removed only when `take` returns an error (ST `backup.rs:52-54`) and the next attempt writes a new
  name (the name holds `at_ms`), so a process that died between the export and the delete would otherwise leave a
  copy with payloads that no purge reaches.
- What a key holder still sees after a purge: the `held_results` row (kind, size, times, cause) and the
  `releases` row. They say a result existed, not what it was.

What never enters the store, in any table, ledger payload, audit row, notice, backup or error text:

| Never stored | Where it lives instead |
|---|---|
| Google's refresh token | Keychain, service `ai.paracels.fabric.calendar.google`, account `refresh-token-v1.<account id>`, this Mac only; in memory (`MemoryTokenStore`) with the test key (W05 section 4) |
| Google's access token | Memory only (`AccessCache`) |
| The identity token, the client secret, the PKCE verifier, `state`, the OIDC nonce, the authorization code | Memory of the sign-in only (W05 PD11) |
| Google's or a library's error text | Fixed labels only (`ConnectProblem`; W04 D37; W05 PD27) |
| An agent's private key or seed | The bridge's key store only (W06 5.4) |
| The handshake nonce, the signature, the handshake text | The core's memory for one connection (P9); the audit row of a failure holds the cause and a known id only |
| A held result's content or digest in the ledger | `held_result_payloads` only |

The pairing code has no column of its own. It is part of the agent record, so it is inside stored command answers
(`commands.answer` and the ledger payload of `CommandAnswered`) and inside agent notices. It is computed from the
public key, which the store keeps (DRAFT README section 8), and is not a secret.

The store keeps `provider_accounts.token_ref`, the Keychain account name, which is not a secret. A database whose
account says `connected` while the Keychain has no item becomes `reauthentication_required` at the first call (W05
section 4).

---------------------------------------------------------------------------------------------------------------------

### A.8 Fixtures and the rehearsal

#### A.8.1 The frozen version 1 stores

The lead makes them, as the first commit of the step P3 pull request, before the commit that adds migration 2.

| Fixture | Made from | Why |
|---|---|---|
| `crates/fabric-storage/tests/fixtures/store-v1-2d41210.sql` | main `2d41210`, today's main, before P0 | Rows without P0's optional member of `RequestMoved` |
| `crates/fabric-storage/tests/fixtures/store-v1-<base>.sql` | the base commit of the P3 pull request: the last main commit at store version 1, after P0, the duration rule and contract version 3 have merged | Rows as the newest version 1 code writes them |
| `crates/fabric-storage/tests/fixtures/store-v1-<base>.macos.db` | the same commit, made on macOS: a SQLCipher file keyed with `TEST_KEY` | The encrypted page format (PLAN 14 "Upgrade encrypted fixture database"); its test runs on macOS only |

The `.sql` form is a text dump of every table (`meta`, `ledger`, `audit`, and the 17 projection tables) as
`INSERT` statements in rowid order, preceded by the schema from `sqlite_schema` and ending with `PRAGMA
user_version = 1`. The test loads it into an empty file keyed with `TEST_KEY` (SQLCipher on macOS, plain SQLite
elsewhere, ST `key.rs:7`), closes it, and opens it as a store. One text form serves every platform and shows in a
diff (J-store).

The generator is an `#[ignore]`d test, `write_frozen_store`, in `crates/fabric-storage/tests/replay.rs`, beside the
replay model it uses (`run_sequence`, `crates/fabric-storage/tests/replay.rs`), writing into the directory named by an
environment variable of the test. It uses only the version 1 public API and `tests/common`, and builds every mutation
and contract value from JSON text (`serde_json::from_value`), so one file compiles at both commits. For `2d41210`, where
it does not exist, the lead copies the file into a temporary worktree of that commit and runs it there; if it does not
compile there, the `2d41210` fixture is made now from a generator kept as
`tests/fixtures/write_frozen_store-2d41210.rs.txt`. The generator lives in the storage crate and cannot ask the core's
reducer which states it reaches, so the states are listed. It asserts before writing that the ledger holds each of the
30 version 1 variants (ST `mutation.rs:24-243`), every value of `SessionEndReason`, `VoidReason`, `GrantEndReason` and
`AuthorizationState`, and every request state of contract version 2 (CC `lifecycle.rs`) except `cancelled`,
`reconciling`, `reconciled`, `absence_confirmed` and `needs_owner_resolution`; that no request is in
`needs_owner_resolution` or `completed`, every `effects` row has a tool that changes a calendar, and one request is in
`applied` and one in `outcome_unknown` (rows main's core can write; a row the store API accepts but main's core cannot
write would fail I1, I15 or I19 after the migration); and that every projection table is non-empty (as s13 does,
`crates/fabric-storage/tests/replay.rs:966-983`). `tests/fixtures/README.md` records for each fixture the commit, the
date, the command, the seed and the row counts. A fixture is never regenerated after it merges.

#### A.8.2 The rehearsal

| Test | What it shows |
|---|---|
| `a_store_written_by_main_migrates_and_replays` (`tests/migration.rs`, one case per fixture) | Opening with the version 2 build takes the version 1 backup `broker-v1-...-h{head}-...` with no delete of held payloads, verifies it, migrates, and `verify_chain`, `verify_replay` and the invariants pass. Every row of the 17 version 1 tables is byte-identical to the fixture, except the three new NULL columns of `effects`. Then a version 2 write (add, prove, approve an agent; end its enrollment grant) and `verify_replay` again, which takes G1 to G3 after the migration row |
| s08 rehearsal (`tests/migration.rs`) | Migration 2 to a test-only version 3 (`WITH_V3`: migrations 1 and 2 and a `rehearsal` table at 3), as today's `WITH_V2` does for 1 to 2 |
| s09 frozen schema | Migration 1 alone equals `schema-v1.sql`; migrations 1 and 2 on an empty database, and a store made by `Store::open`, equal `schema-v2.sql` |
| one test per gate G1 to G9 | The branch at version 2, and no change at version 1 (a store opened with `&MIGRATIONS[..1]`); and a replay of one ledger in which rows before and after the migration row take the two sides of the gate |
| `effect_columns_stay_version_1` (unit, `read.rs`) | Pins the text of `EFFECT_COLUMNS` (R3) |
| `a_held_payload_is_gone_after_its_purge` | After `HeldResultPurged` the payload row is gone, the reader answers None, and a backup copy holds no payload row (on a version 2 store migrated to the test-only version 3) |
| `a_partial_backup_is_removed_at_the_next_migrating_open` | A `.partial` file left in `backups/` is gone after the next open that migrates |
| `a_stored_read_commits_and_a_stale_one_is_refused` (unit, `apply.rs`) | `ReadFinished(held)`, `HeldResultStored`, `put_held_payload`, `RequestMoved` in that order commit; `HeldResultStored` after `ReadFinished(stale)` is refused |
| `a_store_written_by_main_has_no_request_in_applied_after_its_first_start` (core, `crates/fabric-broker-core/tests/`, on the frozen stores; built in step P5a with B.14's recovery, not in step P3) | After the first start of the version 2 core no request is in `applied` (B.14) |
| commit check | A `HeldResultStored` without `put_held_payload`, and an `ObservationRecorded` naming missing evidence, commit nothing |

#### A.8.3 Lists that grow

| Where | Today | After migration 2 | After migration 3 |
|---|---|---|---|
| `PROJECTION_TABLES`, `crates/fabric-storage/tests/fitness.rs:14` and `crates/fabric-storage/tests/replay.rs:30` | 17 | 24 | 26 |
| `NOT_PROJECTION`, ST `ledger.rs:51` | 3 | 5 | 5 |
| `replay_differs`, ST `ledger.rs:451-471` | 17 names | + 7 | + 2 |
| `OPERATION_TYPES`, ST `ledger.rs:25` | 22 | 34 | 39 (40) |
| `INVARIANTS`, ST `store.rs:922` | 7 | 26 | 28 |
| `Mutation` variants, ST `mutation.rs:24` | 30 | 40 | 42 |
| replay model, `crates/fabric-storage/tests/replay.rs` | every table non-empty | extended so every new table is non-empty | same |

#### A.8.4 Tests that pin version numbers today and must change

Once `SCHEMA_VERSION` is 2, a store made by `Store::open` is a version 2 store with two creation rows (AD10), and a
test-only "migration 2" collides with the real one. Each of these changes in the P3 pull request. "N+1" means
`SCHEMA_VERSION + 1`.

| File:line | Today | Change |
|---|---|---|
| ST `migrate.rs:4`, `:15-18` | `SCHEMA_VERSION = 1`, one migration | 2, two migrations |
| ST `store.rs:1066-1087` | head 1 (`:1073`), `verify_chain` 1 (`:1076`) | head 2, chain 2; row 2 is `schema.migrated` 1 -> 2 |
| ST `store.rs:1323-1342` | `user_version = 2` (`:1329`), `found: 2, supported: 1` (`:1337-1338`) | N+1, `supported: SCHEMA_VERSION` |
| ST `store.rs:1345-1381` | `TWO`, `GAP`, `FAILING_TWO` with versions 2 and 3 | `THREE`, `GAP` (to 4), `FAILING_THREE`, each built on both real migrations |
| ST `store.rs:1383-1424` | version 2 (`:1410`), chain 3 (`:1411`), `file_name(1, .., 2, ..)` (`:1417`) | versions and heads one higher |
| ST `store.rs:1426-1442` | `file_name(1, ..)` (`:1431`), `user_version` 1 (`:1435`), chain 2 (`:1436`) | one higher |
| ST `store.rs:1444-1473` | two rows, row 2 `1 -> 2` (`:1469-1472`) | three rows, row 3 `2 -> 3` |
| ST `store.rs:1475-1489` | version 1 (`:1487`), chain 1 (`:1488`) | 2 and 2 |
| ST `store.rs:1491-1503` | chain 1 (`:1502`) | 2 |
| ST `ledger.rs:660` | chain 1 | 2 |
| ST `tx.rs:572-599` | run row at seq 2 (`:588`), migration row at seq 1 (`:595`) | run row at seq 3; seqs 1 and 2 are migration rows with null run |
| ST `tx.rs:622` | `ledger_seq` `Some(3)` | `Some(4)` |
| ST `tx.rs:441`, `:449` | `ledger_seq` `Some(2)`; 2 ledger rows | `Some(3)`; 3 |
| ST `tx.rs:470`, `:484`, `:539` | 1 ledger row | 2 |
| ST `tx.rs:686` | `(ledger_head_seq, notice_sequence)` `(1, 0)` | `(2, 0)` |
| ST `ledger.rs:640`, `:645` | deletes seq 2 of a world store and expects "the head in meta" at seq 2 | deletes seq 3, the last row; with three rows a delete of seq 2 leaves a gap and the fault is "a gap in seq" |
| ST `lib.rs:195-216` | `TWO` with a backup directory that is a file | `THREE` |
| `crates/fabric-storage/tests/migration.rs:1-5` | module comment: the v1 store is built at test time, no fixture database | version 1 comes from the frozen fixtures (A.8.1) |
| `crates/fabric-storage/tests/migration.rs:22-31` | `WITH_V2` | `WITH_V3` |
| `crates/fabric-storage/tests/migration.rs:53-75` | `v1_store` made by this build, asserts version 1 (`:66`) | `v2_store` asserting `SCHEMA_VERSION`; version 1 comes from the fixtures (A.8.1) |
| `crates/fabric-storage/tests/migration.rs:127-160` | version 2 (`:138`), `1 -> 2` (`:151`), `user_version` 2 (`:156`) | 3 and `2 -> 3` |
| `crates/fabric-storage/tests/migration.rs:195` | backup `user_version` 1 | 2 |
| `crates/fabric-storage/tests/migration.rs:226` | name `broker-v1-...` | `broker-v2-...` |
| `crates/fabric-storage/tests/migration.rs:232-257` | restores version 1 | restores version 2 |
| `crates/fabric-storage/tests/migration.rs:336-366` | a created store equals `schema-v1.sql` (`:360`) | migration 1 alone equals `schema-v1.sql`; a created store equals `schema-v2.sql` |
| `crates/fabric-storage/tests/open.rs:240-253` | `user_version = 2` as newer, `found: 2` | N+1 |
| `crates/fabric-storage/tests/crash.rs:337` | `schema_version` 1 | `SCHEMA_VERSION` |
| `crates/fabric-storage/tests/secrecy.rs:76-85`, `:231` | `WITH_V2` | `WITH_V3` |
| `crates/fabric-storage/tests/secrecy.rs:248` | `schema_version` 1 | `SCHEMA_VERSION` |
| `crates/fabric-storage/tests/fitness.rs:14`, `crates/fabric-storage/tests/replay.rs:30` | 17 tables | 24 (A.8.3) |
| `crates/fabric-broker-core/tests/faults.rs:311`, `:323-324` | `user_version = 2` as newer | N+1 |
| `crates/fabric-broker-core/tests/faults.rs:408` | restores `user_version = 1` | `SCHEMA_VERSION` |
| `apps/fabric-broker/tests/process.rs:1283-1300` (p03) | `TWO` writes a "newer" store of version 2 | a list on `MIGRATIONS` with a test version N+1 |

No change needed: ST `lib.rs:131-132` and `crates/fabric-storage/tests/secrecy.rs:104-105` (message texts of an
arbitrary `NewerSchema`); ST `tx.rs:562` (`SchemaMigrated` is refused outside the runner whatever its numbers).
Owned by the contract session, not by this step: the contract `schema_version` 2 in `hello` frames
(`crates/fabric-broker-core/tests/faults.rs:19, 320`, `crates/fabric-broker-core/tests/limits.rs:367-369`,
`apps/fabric-broker/tests/process.rs:538`, CORE `owner/tests.rs:366`, and the relay and host tests).

A run id is the head plus one at open (CORE `service/core.rs:109`), so the first run of a new store is 3, not 2. No
test was found that pins a run id by number (searched `run_id, 1`, `run_id: 1`, `run_id), Some(1)` in the core and
the binary); the builder searches again in step P3 for pins of `seq = ` and run ids when the change compiles (ruled
2026-11-17, A-12).

---------------------------------------------------------------------------------------------------------------------

### A.9 Answers to the store requests

| Id | Answer | What |
|---|---|---|
| ST1 | Accepted with this change | `provider_accounts` in migration 3 (A.5). The row is made at `connect_google` with `subject` null until the first `Connected` outcome, under a partial unique index (V16). No CHECK on provider, revocation or problem (R4). Written by `ProviderAccountPut` carrying the whole `ProviderAccountRow`. No token; `token_ref` only. Invariant I23 |
| ST2 | Accepted with this change | `provider_calendars` in migration 3. `cal-` ids are made by the core and carried in the mutation (R8). Rows never deleted; a lost calendar is `listed: false`. Bound 250 per account, and a list past it fails as a whole (W05 6.2 is fixed to that bound, REV-store ST2 e). Invariant I22 |
| ST3 | Accepted with this change | In migration 2, not 3 (AD3; AO2, ruled 2026-11-17). Facts in `observations`, keyed by `observation_id` (`proof_reports.proof_id` is not unique, `0001_initial.sql:91`). Raw evidence in `observation_evidence`, named by a random `evidence_id`; its digest only beside it. One evidence table for observe and reconcile reads (V19). Invariant I20. Never purged by a disconnect; outside the ledger for size only. At most 1 MiB for each observation, its before-view at most 512 KiB, and byte-equal evidence of one event at one revision kept once (D.6.2; ruled 2026-11-18 by the lead; owner OD26) |
| ST4 | Accepted with this change | `held_results`, `held_result_payloads` and `releases` in migration 2, one design for CD-W05 and CD-W06. `released_count` is replaced by `releases` rows. The ledger names a random id, never a digest. A read uses `read_attempts`, not `effects` (G9, I19). Purge: A.7. The 24 h lifetime is the owner's (P13) |
| ST5 | Accepted with this change | Only `event_id_after`, `revision_after` (P4) and `notification`, written by `EffectEventRecorded`, read only by `ReadTx::effect_event` (R3). `revision_before`, `confirmed`, `reconcile_reads` and `next_reconcile_at_ms` are dropped (V2) |
| ST6 | Accepted | No schema change. Two readers, `created_event` and `created_events` (0.5.2), ship in step P3; section D applies DD12's state test. Store note: a later history deletion (CD-Q11h) must keep these rows |
| ST7 | Not built | Not built (V7): `open_effects` is not widened. The read after an applied change is dropped (the lead's ruling of 2026-11-17, OP5; agreed by the CD-W05 session 2026-11-18, W5-6). A delete is confirmed by the adapter's one GET in the delete's own dispatch order (D.2), which is not a read after an applied change; the theory question is the contract session's |
| ST8 | Accepted with this change | One list for both packages (A.6), one dated amendment of ADR-115 D2 (A.3). Added beyond W05's list: `timer.result_purge`, `system.proof`, `provider.revoke`, and the CD-W06 types. Existing variants gain no required member (R6) |
| ST9 | Accepted | True by ST1 and A.7. Migration 3 adds a secrecy test: the fake Google server issues marker refresh and access tokens, and neither marker appears in the database file, the WAL, a backup or a ledger payload (`crates/fabric-storage/tests/secrecy.rs` pattern). A backup holds token references and observations, and no held payload |
| ST10 | Decision needed | Not a store change. The owner decides the cap on accounts (P13; W05 OQ14); section D counts accounts into the 720 KiB budget and refuses a connect past it (P10) |
| ST11 | Accepted with this change | A.8: frozen version 1 stores made by the version 1 code (two commits, text and SQLCipher), the rehearsal on them, and the listed test changes. R22 is settled by the rule set, not by version-aware replay |
| LR-S1 | Accepted with this change | The key, the state and the four times live in `enrollments` and `EnrollmentRow`, not on `AgentRow` (0.5.2; J-store). The key is the contract text `ed25519:` plus 64 lowercase hex, held by at most one enrollment; a removed enrollment keeps its key, so it is never accepted again, while a key replaced while pending is free. `SessionEndReason::AgentRemoved` maps to `GrantEndReason::AgentRemoved` (P4) |
| LR-S2 | Accepted with this change | Migration 2 holds it with everything else (AD2). Types `owner.add_agent`, `owner.approve_agent`, `owner.remove_agent`, `agent.prove_key` (A.6); the order of the two packages' migrations is settled by P3 |
| LR-S3 | Accepted with this change | Form (a), a grant until revoked, in DRAFT's shape, held in `enrollment_grants`, a second table, so that `grants.origin_request_id NOT NULL REFERENCES requests` (`0001_initial.sql:79`) is untouched (R1). Every reader of grants sees both tables (A.4.6) |
| LR-S4 | Accepted with this change | Audit only, no migration: kind `handshake_failed`, actor broker, detail `{cause, enrollment_id or null, window_start_ms, window_end_ms, count}`, one row per cause and known id per 60 s window, written by `Store::write_audit` with no ledger row (ADR-115 D6). An id that names no enrollment or agent is recorded as null, so random ids cannot grow the table. The counting is section C's |

---------------------------------------------------------------------------------------------------------------------

### A.10 Open points of this section

| # | Point | Default the design runs on | Owner |
|---|---|---|---|
| AO1 | The amendments of A.3: the rule set, content outside the ledger (D1, D7, W04 D3), the operation types (D2) | As written; nothing is applied before approval | owner (P13; OD5, OD20) |
| AO2 | Observations and the event columns in migration 2 against P3's list (OP1) | Migration 2. Ruled 2026-11-17: accepted | lead |
| AO3 | The duration rule's "approval null while a request waits again" must not change what an existing variant writes for a row main could commit (R2) | Closed. Ruled 2026-11-17: the null approval is on the read side only, in `get_request_details`; nothing written changes, so R2 is not touched (A.2) | lead (P0) |
| AO4 | A purged payload stays in older WAL frames until the checkpoint of AD13 succeeds; with the test key off macOS those frames are plain | Accepted residual, stated to the owner with the 24 h lifetime (OD3, OD22) | owner |
| AO5 | A backup restored by hand reads unpurged held results as not held (OP14) | Accepted; a restore is a manual downgrade | owner (OD5) |
| AO6 | The actor of `agent.prove_key` (AD15) | `agent` with the enrollment id; closed by X2, C.6.4 follows it | section C, lead |
| AO7 | `owner.refresh_calendars` exists only with P2-11 | Not in the list until P2-11 merges | contract session |
| AO8 | Whether `deleted_profiles` (A.6) goes into migration 2, so that `delete_custom` can be built later without a migration of its own | Not in phase 2; `delete_custom` and `list_ended_requests` keep `unknown_method`. Ruled 2026-11-17: not in phase 2, so `deleted_profiles` is not in migration 2 | lead, with the contract and screens sessions |
| AO9 | V22 is a table constraint: `enrollment_grants_agent` allows one row per agent ever. Giving the list permission again at a new approval would be a changed constraint (R1) | Keep V22; a later form uses a partial index (`WHERE ended_at_ms IS NULL`) in a new table, or an apply guard. Ruled 2026-11-17: keep the constraint | lead |

---------------------------------------------------------------------------------------------------------------------

### A.11 What this step needs from others

| From | What |
|---|---|
| Contract session | P2-14 (the byte-for-byte test of every version 2 stored fixture); R7's record for each non-widening change, as A.2 does for `needs_owner_resolution`; the `I1` edit for every state it adds, in the same pull request |
| Section B | Confirmed (X15): a release's proof is a fresh proof, purpose `fresh_permit` (B.3.5, B.4.1); the `ResultReleased` guard stands |
| Section C | The budget refusal of `approve_agent` (A.4.6); the handshake audit counting (LR-S4); `GrantOrigin::Enrollment` handled where the core reads `origin.request_id` |
| Section D | `ConnectOutcome` (now D.2) and the final members of the migration 3 row types (W5-7) |
| Owner | Approval of A.3 before it is applied |

---------------------------------------------------------------------------------------------------------------------

### A.12 Names this section adds or corrects

- G4 widened: refuses an agent whose enrollment is not `approved` (the first draft of section 0 said `removed`), and
  a `grant_id` already in `enrollment_grants`. DRAFT README section 7: a pending agent "holds no grant".
- G7 (`AgentPut`), G8 (`RequestInserted`), G9 (`EffectReserved`): new gated branches.
- I24 (grant id in both tables) and I25 (authorization of two attempts): new invariants. I10 widened to "not
  approved", I12 also checks the columns against the record.
- `WriteTx::delete_held_payload` dropped; the apply of `HeldResultPurged` deletes the payload (AD5).
- `ReadTx::schema_at_least`, `ReadTx::evidence`, `Store::checkpoint`, the commit check of A.4.4.
- `ResultReleased` also checks its proof row and fence (AD8); `HeldResultStored` checks its authorization's fence;
  `ObservationRecorded` requires a permitted Isabelle system proof; `ReadIntent` refuses an authorization named by
  an attempt.
- `owner.refresh_calendars`, conditional type 40.
- Correlation of `owner.resolve_outcome` (AD14) and actor of `agent.prove_key` (AD15).
- I26 (release kind and grants): new invariant.
- `INVARIANTS` count 26 at version 2, 28 at version 3; `Mutation` variants 40 and 42.
- Readers `provider_accounts`, `agent_view`, `unpurged_held_results_of_account`, `unpurged_held_results_of_agent`,
  `next_purge_due_ms`; read types `HeldResultRecord`, `ReadAttemptRecord`, `AgentView`; `SystemProofRecord` gains
  `fence`, `undecided_reason`, `check_theory_text` (0.5.2).

---------------------------------------------------------------------------------------------------------------------

## B. Results, held answers and proofs

**What this section decides.** How a result leaves the broker (ruling P6) and everything that rule needs: the
lifecycles of a list, a read and a release; the terminal state `completed`; the release record;
`fabric_request_result` under a `Disclose` proof; the narrow lifts of W04 D17; the amendments of D18 and of `used`;
the successor of D33; system proofs; held answers (P7); the answer `prover_unavailable` (P8); the fence with a stored
event revision; the deadline of every provider order; keeping the applied event. It ends with the full path of a
read, the tests of question 10, the answers to the requests this section owns, and the owner's decision on two calls
for a read.

**Sources and short names.** As in 0.1: W04, PW,
ADR-115, PLAN, W05, W06, DRAFT (the contract draft at `29cdfdf`), REV-* (scratchpad `w05/review.json`), PART-* and
J-* (scratchpad `w06/integration-result.json`), CORE (`crates/fabric-broker-core/src/`), ST
(`crates/fabric-storage/src/`), BR (`apps/fabric-broker/src/`), CC (`crates/fabric-desktop-contracts/src/`), THY
(`isabelle/Fabric_Broker/Fabric_Broker.thy`), EX (`isabelle/Fabric_Broker_Examples/Broker_Examples.thy`), LC (DRAFT
`contracts/desktop/v1/data/request-lifecycle.json`). Code is main `2d41210`. P0 is the broker part of schema version 2
now in progress (`ProverStatus`, `ProverCause`, `ProofDone.cause`, `undecide()`); this section builds on it and does
not rely on its exact code.

**Names.** Every table, mutation, enum value, constant and core type named here is section 0's or that of the section
0.3 names. Names this section adds are marked "(added)" where they first appear and are listed in B.19.

---------------------------------------------------------------------------------------------------------------------

### B.1 Decisions of this section

| Id | Decision | Why |
|---|---|---|
| BD1 | The release rule is written as six invariants, R1 to R6 (B.2). Each has a test | P6 binds; invariants make it checkable |
| BD2 | A list and a release end `proving_fresh_permit -> completed` in one commit, `request.release`. They get no authorization row and no `effects` row. The release commit is their linearization point: the checks `judge` runs in the same step (fence, every authorizing grant valid now, agent not paused, prover Isabelle) are the handoff checks | P6; W04 D33's "not dispatches" stays true; `Authorization.grant_id` is required (CC `owner.rs:1033`), so a change's outcome could not have one |
| BD3 | A release happens only when `Held::waits_for` is true in the step of the release commit: a held answer exists for the releasing request, its connection is still bound to the session it was held for, that session is the agent's live one, and its deadline has not passed. Otherwise the request goes `proving_fresh_permit -> cancelled` (`CallGone`), with operation type `request.proof` and no release row (V10) | P6 "only while that call still waits"; J-auth results minor (bound session, deadline) |
| BD4 | A list or a release whose held answer leaves before the request settles (the 20 s deadline) or is dropped (disconnect, replaced or idle session, removal) is cancelled in the same step (`CallGone`) when it is in `received`, `validating`, `proving_classification`, `allowed`, `approved` or `proving_fresh_permit`. Recovery (`broker.started`) cancels every open list and release in those states. A list in `pending_approval` keeps its card | No call can ever wait for it again, so every further proof is wasted prover time (J-auth results minor 2) |
| BD5 | A list that needed the owner's card is answered `pending_approval`; after the approval no call waits for it, so it ends `cancelled` at its fresh permit and its one-request grant ends `consumed` as today. The agent calls `fabric_calendar_list` again. Under a one-request approval that call asks again. A one-request approval of a list delivers nothing: the request is cancelled and the grant ends `consumed`. Under `list_and_availability` in mode `ask` (where the card proposes only `one_request`, DRAFT README section 10) no list completes in version 1. The lead accepted this for version 1 on 2026-11-17 (BOP1), with the card's text (SR-7) and P2-29; the owner accepted it on 2026-11-18 (OD24) | Safe and simple; P1 gives a list a card; P6 forbids a release with no waiting call |
| BD6 | A read's own answer never carries its result. A read ends `dispatching -> applied -> reconciled` in the commit that stores its held result; the agent then calls `fabric_request_result` | P6 (two calls in version 1); the owner chose two calls on 2026-11-18 (B.18, OD1) |
| BD7 | A read is authorized and handed off like a change: `proving_fresh_permit -> queued` with an authorization, `queued -> dispatching` with `ReadIntent` in place of `DispatchIntent`, one `ReadOrder` | REV-results read-dispatch-record; PLAN 5 R04: no read reaches the provider without a consumed authorization |
| BD8 | The commit that stores a read checks the whole fence, the read authorization's grants and the pause again (V6). A failed check, `Stopped`, a transient failure and an order past its watchdog all finish the attempt and go `dispatching -> retry_wait` under `retry_due` counted over `read_attempts`; past the budget `terminal_failure` (`retry_budget_exhausted`). `Rejected` and a result whose shape does not fit the order go `terminal_failure`. A request of a removed agent never retries: the same commit moves it on `retry_wait -> cancelled` (X22) | V5, V6; `retry_due` bounds a loop that a fence that keeps changing would otherwise make |
| BD9 | A held result is purged at its first release whatever grant released it (V4), at its account's disconnect, at its agent's removal and 24 h after it was stored. Every purge ends the read's live one-request grant in the same commit, with the reason of V15 | One held result, one release; a grant that can serve nothing must not show as live |
| BD10 | `result_held` for a read target is a predicate read with the facts: the `held_results` row exists, belongs to the agent, is not purged, and `now < purge_due_at_ms`. The payload and its digest are read only in the release commit; a missing or mismatched payload there ends the request `undecided` (cause `result_not_held`) | REV-results retention-OQ5: a late sweep can never release an expired result; V18: a missing payload reads as "not held" |
| BD11 | The outcome of a change is released with no grant (lift c of D17), as often as asked, only to the agent that made the change, only while the change is `applied` or `reconciled` with a recorded event, and only under every block rule of the theory (pause, account connected, calendar selected). It is metadata: an event id and a revision, or null after a delete | THY `:373`, EX `:569`; P6; the agent needs the id to update or delete what it made (W06 LR-C4 (2)) |
| BD12 | `fabric_request_result` never opens a card in version 1. A NeedApproval on a release ends `undecided` | P6 "A release never asks the owner in version 1"; request P2-3 |
| BD13 | A call that would create a request is refused with `prover_unavailable` while `prover_ready()` is false, after the repeat check and before the limits. A status call and a repeated call id are always answered | P8 |
| BD14 | `Fence` keeps its shape. `fence_now(counters, observed_revision)` puts the request's stored observed revision into the current fence, and `fence_holds` stays equality of the three members. The fix lands before any code stores an observed revision | P10; REV-core "highest risk": today every observed proof would be discarded (CORE `service/epochs.rs:19-31`, `pipeline.rs:236, 471`) |
| BD15 | Every provider order carries `order_id` and `deadline_ms = issue + PROVIDER_CALL_MS`. The core keeps each order in flight and acts at `deadline_ms + ORDER_WATCHDOG_MARGIN_MS`. The worker never starts an order past `stamp + PROVIDER_CALL_MS`, where `stamp` is the monotonic instant the shell took the job (X23). A late answer is ignored by `order_id` | P10; V20; today a provider that never answers leaves a request in `dispatching` for the whole run (CORE `pipeline.rs:567-577` schedules nothing there) |
| BD16 | An `Applied` whose event does not fit the order is handled as `NoAnswer`, with one audit row `provider_mismatch`, and an unconfirmed delete as `NoAnswer` with no audit row (V9): the attempt is finished `outcome_unknown` and the request goes to reconciliation. A confirmed, fitting `Applied` writes `EffectEventRecorded` and moves `dispatching -> applied -> reconciled` (V7). A fitting `Applied` of a create, update, invitation or response that is not confirmed writes `EffectEventRecorded` and waits for the owner, never reconciled by reading (V23). Ruled 2026-11-18 by the lead | P10 "a change counts as applied only when Google plainly said so"; a mismatched id must never be released as the agent's event; a fitting 200 means Google made the change, and reading it back could end it absent and let a new request repeat a change that notifies guests |
| BD17 | System proofs are their own job (`Job::ProveSystem`), accepted by their own rule, recorded in `system_proofs` whatever their outcome, with operation type `system.proof`. An accepted one issues its order in the same commit | V8; REV-core CO9 |

---------------------------------------------------------------------------------------------------------------------

### B.2 The release rule (P6)

A result is: the calendar list (`result_calendars`), availability (`result_availability`), event details
(`result_events`), or the outcome of an applied change (`result_effect`) (DRAFT `schemas/agent-channel.schema.json`
`tool_result`, lines 431-437 of the draft). The rule, as six invariants:

| # | Invariant | Where it is enforced |
|---|---|---|
| R1 | A result leaves the core only in the `tool_outcome.result` of the answer to a `fabric_calendar_list` or a `fabric_request_result` call | `answer_reply` is the only function that writes a non-null `result`; `status_reply` keeps `None` (CORE `service/agent.rs:55-76`); only `wake_held` calls `answer_reply` with a result (B.8) |
| R2 | The result in that answer was released by that call's own fresh proof: for a list its own `Permit` on `Agent(CalendarList)`, for `fabric_request_result` its own `Permit` on `Disclose(target)` | `Accepted::Release` (added) exists only for those two tools (B.6) |
| R3 | The release happens only while the call waits (BD3). Release and delivery happen in the same step or not at all | the release commit reads `Held::waits_for`; `Held.released` is cleared at the end of every `wake_held` (B.8) |
| R4 | Each release writes one `releases` row (`ResultReleased`) in the commit that moves the releasing request to `completed`; the answer leaves only after that commit returned | W04 5.7 "audit before effect"; the answer is pushed by `wake_held`, which runs after the commit, and leaves through the drain after it (CORE `broker.rs:226-256`) |
| R5 | A status never carries content: `fabric_request_status`, a repeated call id, a held answer of any other tool, an answer at quit, every error, every notice and every owner frame | `status_reply` everywhere else; tests check the frame bytes, not the Rust type (B.16) |
| R6 | A release uses authority as it is at the release commit: the fence of the releasing proof equals the current fence, every authorizing grant is valid now, the agent is not paused, and the target's result is still held | `judge` (CORE `pipeline.rs:208-264`) in the same step; the release commit re-reads the result source (B.4) |

The transport limit (OP13): "a `releases` row means the result left the core". A writer queue that is full drops
the frame after the commit (section C's queue). Because of V4 and BD9 the agent must then read again. This is the
weaker form of R3 that the design can promise, and the owner sees it in B.18 and E.3 OD17.

---------------------------------------------------------------------------------------------------------------------

### B.3 Lifecycles

Every edge below is checked against LC (DRAFT `data/request-lifecycle.json`, 25 states, 41 transitions; line numbers
are LC's). "Exists" means the transition is in LC. `completed` is the one new state (P2-1). Operation types are those
of A.6.

#### B.3.1 The terminal state `completed`

| Item | Value |
|---|---|
| State | `completed`, terminal, plain (no extra member in a status) |
| Edge | `proving_fresh_permit -> completed`, basis `added`, reason "plan 9.2: a request the broker completes itself (a calendar list, a result release) has no provider step" |
| Who reaches it | only a `fabric_calendar_list` or a `fabric_request_result` request, only in a commit that writes its `releases` row (I15) |
| Store | `requests.state` text `completed`, `terminal` 1. The open invariant I1 (`request state`, ST `store.rs:924`) gains it in the same pull request as the reducer row (J-store blocker 1) |
| Contract | request P2-1: `RequestState::Completed`, `is_terminal`, `can_transition_to`, `PlainRequestState`, the agent and owner status schemas; README section 9: 26 states, 18 added transitions |
| Page | the screens session words `completed` before the first bundled run (J-store minor, "the page learns `completed` too late") |
| Bridge | W06 3.7 gains a row: `completed`, `isError` false. The answer to the list call or to the `fabric_request_result` call that completed carries the result: "The result follows." A later `fabric_request_status` call on that request shows `completed` with no result, because a status never carries one (R5): "The request is complete." (W6-3, ruled 2026-11-18) |

#### B.3.2 A list (`fabric_calendar_list`)

| From -> to | LC | Driver and commit | Mutations | Checks |
|---|---|---|---|---|
| (new) -> `received` | initial | `agent.call` | `RequestInserted` | session bound (CORE `agent.rs:177-201`); repeat (D25); `prover_ready()` (B.10); limits in the insert transaction (`agent.rs:287-291`) |
| `received -> validating -> proving_classification` | 32, 33 | pump, `request.advance` | `RequestMoved` x2 | world checks: none apply to a list (`calendars_of` is `None`, CORE `requests.rs:80`) |
| `proving_classification -> allowed` | 34 | `request.proof` | `ProofRecorded`, `ReliedGrantSet`, `RequestMoved` | D39 binding, grant check, fence, Isabelle `Permit` (CORE `pipeline.rs:208-264`). Authority rests on the enrollment grant (P5) or an approval grant |
| `proving_classification -> pending_approval` | 35 | `request.proof` | `CardOpened`, `RequestMoved` | NeedApproval with a ruling; the card's scope names no calendar (P1, DRAFT README line 228). Needs `scope_of(CalendarList)` to return `Scope { calendars: [], window: null, fields: null, recipients: null, send_updates: null }` instead of `None` (CORE `requests.rs:123-124`); that is a change to `scoped` for the list only |
| `proving_classification -> blocked / undecided` | 36, 37 | `request.proof` | as today | Block with a ruling; Undecided, watchdog, missing report |
| `pending_approval -> approved / denied / expired / cancelled / stale` | 38-42 | as W04 5.7 | as today | the owner's `decide_request` |
| `allowed / approved -> proving_fresh_permit` | 43, 44 | pump | `RequestMoved` | relied list is the stored one |
| `proving_fresh_permit -> completed` | P2-1 | `request.release` | `ProofRecorded`, `ReliedGrantSet`, `ResultReleased`, `RequestMoved`; a one-request grant whose origin is this list ends `consumed` (`move_request`, CORE `requests.rs:41-55`) | `Accepted::Release`: Isabelle `Permit`, agent not paused, `authorizing_grants` not empty, each valid now, fence holds; `waits_for` true; the answer frame fits `MAX_FRAME_BYTES` |
| `proving_fresh_permit -> cancelled` | 62 | `request.proof` | `ProofRecorded`, `RequestMoved` | `Accepted::Release` but `waits_for` false (BD3) |
| `proving_fresh_permit -> undecided / blocked / pending_approval / stale` | 66-69 | as W04 5.7 | as today | Undecided with a cause from B.6; a re-ask opens a new card |
| `received / validating / proving_classification / allowed / approved / proving_fresh_permit -> cancelled` | 57-62 | `request.advance` (runtime) or `broker.started` (recovery) | `RequestMoved` | BD4: the call has left |

The list's content (B.4.2) is built in the release commit and never stored; only its digest is (V3).

#### B.3.3 A read (`fabric_calendar_availability`, `fabric_calendar_read`)

| From -> to | LC | Driver and commit | Mutations | Checks |
|---|---|---|---|---|
| (new) -> `received` | initial | `agent.call` | `RequestInserted` | as the list |
| `received -> validating -> proving_classification` | 32, 33 | pump | `RequestMoved` x2 | world checks: account known and connected, every calendar known and selected, window start before end (CORE `pipeline.rs:341-384`) |
| `validating -> terminal_failure` | 65 | pump | `RequestMoved` | a world check failed (`invalid_arguments`) |
| `proving_classification -> allowed / pending_approval / blocked / undecided` | 34-37 | `request.proof` | as today | the read's scope makes a card possible |
| `allowed / approved -> proving_fresh_permit` | 43, 44 | pump | `RequestMoved` | |
| `proving_fresh_permit -> queued` | 45 | `request.authorize` | `ProofRecorded`, `ReliedGrantSet`, `AuthorizationIssued` (60 s, `grant_id` = first authorizing grant), `RequestMoved`; no `EffectReserved` | `Accepted::Authorize` widened to reads (lift a, B.6.1): Isabelle `Permit`, `can_carry_out` for the account (V17), agent not paused, `authorizing_grants` not empty and each valid now, fence holds |
| `queued -> dispatching` | 46 | pump, `request.dispatch` | `AuthorizationConsumed`, `ReadIntent { number: next }`, `RequestMoved`; after commit `Job::Read(ReadOrder)` | `handoff()` unchanged (CORE `pipeline.rs:448-488`): issued, this run, unexpired, fence (B.12), grants valid, agent not paused. `dispatch()` branches on `holds_result(tool)` (added) at the `effect_of` read (`pipeline.rs:600-603`) |
| `queued -> proving_fresh_permit` | 70 | `request.dispatch` | `AuthorizationVoided`, `RequestMoved` | a handoff check failed, as today |
| `dispatching -> applied -> reconciled` | 47, 48 | `Input::ReadDone` `Done`, `request.read` | `ReadFinished(held)`, `HeldResultStored` + `put_held_payload`, `RequestMoved` x2 (event `ReadStored`) | the storing check (B.3.4) passes and the result fits the order |
| `dispatching -> retry_wait` | 49 | `request.read` | `ReadFinished(stale / stopped / transient / no_answer_in_time)`, `RequestMoved` (event `ReadRetry`) with `due` from `retry_due` | BD8; for an agent whose enrollment is not approved, then `retry_wait -> cancelled` (`AgentRemoved`) in the same commit (X22) |
| `dispatching -> terminal_failure` | 51 | `request.read` | `ReadFinished(rejected)` or the last `ReadFinished`, `RequestMoved` | `Rejected { code }`; a result that does not fit the order (`provider_rejected`); `retry_due` is `None` (`retry_budget_exhausted`) |
| `retry_wait -> proving_fresh_permit` | 71 | pump | `RequestMoved` | a fresh proof per retry (PLAN 5) |
| `dispatching -> retry_wait` at recovery | 49 | `broker.started` | `ReadFinished(no_answer_in_time)`, `RequestMoved`, due now | a read changes nothing, so never `outcome_unknown` (REV-core CO5 c). CORE `service/recovery.rs:50-72` branches on the tool. A removed agent's read goes on to `cancelled` in the same commit (X22) |
| any non-terminal state before `dispatching` -> `cancelled` | 57-64 | `owner.remove_agent` | per A.4.8 | removal |

`dispatching -> outcome_unknown` (LC 50) is never taken by a read. A read never gets an `effects` row (invariant I19).

#### B.3.4 The commit that stores a read (`request.read`)

`Core::read_done(done: ReadDone)` (added), one commit:

1. Match the order: `done.order_id` names a read order in flight of this run; its request is in `dispatching`; its
   last `read_attempts` row is unfinished. Otherwise ignore the answer and write nothing.
2. `Done { result, spans }`:
   1. Shape: (a) the result's kind is `availability` for `fabric_calendar_availability` and `events` for
      `fabric_calendar_read`; (b) every `calendar_id` in it is one of the order's targets and its canonical bytes are
      at most `MAX_READ_RESULT_BYTES` (786 432, W05 6.4, PD23); (c) for `events`, in every `EventView` a member is
      non-null only when the action's `fields` name it (`time` and `occurrence` for time, `title`, `location`,
      `description`, `attendees`, `organizer`, `own_response`); (d) every busy interval lies inside the action's
      window (start >= window start, end <= window end; W05 6.3), and every event overlaps it; the overlap is tested
      on `spans` (D.2 `ReadSpan`), one span for each event in order (empty for availability), never on the view.
      The number of spans equals the number of events; a `Timed` span has `start_ms <= end_ms` and is checked
      exactly; an `AllDay` span has `start_date < end_date` and is checked as [`start_date` at 00:00 UTC minus
      `ZONE_SLACK_MS`, `end_date` at 00:00 UTC plus `ZONE_SLACK_MS`] (14 h, the widest UTC offset), a bound that
      exists only inside this check. A span overlaps when its start is before the window's end and its end is
      after the window's start (half-open, as Google's `timeMin` and `timeMax`, W05 6.4). When the action grants
      `time`, the view's time equals the span: a timed view's two offset date-times, as instants, equal
      `start_ms` and `end_ms`; an all-day view's dates equal the span's dates. A recurring occurrence is checked
      by its own start and end, never by its original start; a timed event with a zone of its own by its
      instants, which need no zone arithmetic. A result that fails
      (a) to (d) is `ReadFinished(rejected)` and `terminal_failure` (`provider_rejected`); nothing is stored, and
      `spans` never is. The adapter still cuts the fields twice (W05 PD24); this is the core's own check, so a miss
      in the adapter is never released (W5-2). The widening lets through only an all-day event of the day just
      before or after the window, with the granted fields, and only when the adapter's own window was wrong;
      exact bounds in the calendar's zone wait for a zone database, CD-W09's (I.5 D-18). Spans: ruled 2026-11-18
      by the lead (DD17, I.5 D-16).
   2. Authority (V6): the consumed authorization's fence equals `fence_now(counters, None)`; every id in its
      `grant_ids` is valid now (`grant_valid_now`); the agent is not paused. Otherwise `ReadFinished(stale)` and BD8.
   3. Store: `ReadFinished(held)`; `HeldResultStored { held_result_id: new, request_id, agent_id, account_id, kind,
      bytes, stored_at_ms: now, purge_due_at_ms: now + HELD_RESULT_LIFETIME_MS }`; `put_held_payload(id,
      sha256 of the canonical bytes, bytes)`; `RequestMoved` to `applied`, then to `reconciled`.
   4. The read's one-request grant is not ended by the move to `reconciled` (B.7.1).
3. `Stopped`, `Failed(Transient)`: `ReadFinished(stopped | transient)`, then BD8.
4. `Failed(Rejected { code })`: `ReadFinished(rejected)`, `terminal_failure` with `code`.
5. The order's watchdog (B.13) produces `ReadFinished(no_answer_in_time)`, then BD8.

BD8, exactly: `retry_due(policy, attempts_started = number of read_attempts rows, first_attempt_at_ms =
read_attempts[0].started_at_ms, retry_after_ms, now)` (CORE `pipeline.rs:96-117`, unchanged). `Some(due)`:
`dispatching -> retry_wait` at `due` (for `stale` and `stopped` the hint is `None`). `None`: `dispatching ->
terminal_failure` (`retry_budget_exhausted`). The next fresh proof sees authority as it is then. When the request's
agent has an enrollment that is not approved (removed), the same commit moves it on `retry_wait -> cancelled`
(`AgentRemoved`, X22): a removed agent's request never retries, so it never reaches a fresh proof or a card.

The `still_allowed` flag of the read order is a hint that saves pages, not a guard (REV-core "one read order"). The
guard is step 2.2 here and R6 at the release. Where the core keeps and clears it: B.13.

#### B.3.5 A release (`fabric_request_result`)

| From -> to | LC | Driver and commit | Mutations | Checks |
|---|---|---|---|---|
| (new) -> `received` | initial | `agent.call` | `RequestInserted` (scope `None`) | as the list |
| `received -> validating -> proving_classification` | 32, 33 | pump | `RequestMoved` x2 | the validation of B.5.1 passes |
| `validating -> terminal_failure` | 65 | pump | `RequestMoved` | B.5.1 fails: `invalid_arguments` |
| `proving_classification -> allowed` | 34 | `request.proof` | `ProofRecorded`, `ReliedGrantSet` (empty for a change's outcome), `RequestMoved` | Isabelle `Permit` on `Disclose(target)` |
| `proving_classification -> undecided` | 37 | `request.proof` | as today | NeedApproval: no scope, no card (BD12, CORE `pipeline.rs:170-173`); Undecided |
| `proving_classification -> blocked` | 36 | `request.proof` | as today | `result_held`, `agent_not_paused`, `account_connected`, `calendar_selected` and the other block rules |
| `allowed -> proving_fresh_permit` | 44 | pump | `RequestMoved` | |
| `proving_fresh_permit -> completed` | P2-1 | `request.release` | B.4.1 | `Accepted::Release` (B.6.1); `waits_for`; result still held; frame fits |
| `proving_fresh_permit -> cancelled` | 62 | `request.proof` | `ProofRecorded`, `RequestMoved` | `waits_for` false (BD3) |
| `proving_fresh_permit -> undecided` | 67 | `request.proof` for `release_not_authorizable`, and for `result_not_held` when the source is absent before the commit (B.4.1); `request.release` for `result_not_held` and `result_too_large` found inside the release commit (B.4.1 steps 2 and 3) | `ProofRecorded`, `RequestMoved` with the cause | causes `release_not_authorizable`, `result_not_held`, `result_too_large` (0.4.4) |
| `proving_fresh_permit -> blocked / stale` | 66, 69 | as today | | `relied_grant_live` after a revocation; re-proofs exhausted |
| `proving_fresh_permit -> pending_approval` | 68 | never | | a NeedApproval with no scope is `undecided`; a release has no scope |
| BD4 cancellations | 57-61, 62 | as the list | | |

A release never reaches `pending_approval`, `approved`, `queued` or `dispatching`.

#### B.3.6 A change, where this section touches it

Only two commits change for a change (section D owns the reconcile rows):

| From -> to | LC | Commit | What changes |
|---|---|---|---|
| `dispatching -> applied -> reconciled` | 47, 48 | `request.attempt` | a confirmed, fitting `Applied`: `AttemptFinished(applied)`, `EffectEventRecorded`, two `RequestMoved` (V7, BD16) |
| `dispatching -> outcome_unknown` | 50 | `request.attempt` or the order watchdog | `NoAnswer`, an unfitting `Applied`, an unconfirmed delete, a dispatch order past its watchdog |
| `dispatching -> outcome_unknown -> reconciling -> needs_owner_resolution` | 50, then the edges D.8.3 uses | `request.attempt` | a fitting, unconfirmed `Applied` of a create, update, invitation or response (V23): `AttemptFinished(applied)`, `EffectEventRecorded`, the audit row `provider_differs`, three `RequestMoved`, no read; until P2-33 adds `dispatching -> needs_owner_resolution` |

#### B.3.7 Reducer events added (core only, not stored)

| Event | From | To | Driver |
|---|---|---|---|
| `Release` | `proving_fresh_permit` | `completed` | B.4.1 |
| `CallGone` | `received`, `validating`, `proving_classification`, `allowed`, `approved`, `proving_fresh_permit` | `cancelled` | BD3, BD4; only for a list or a release |
| `ReadStored` | `dispatching` | `applied`, then `reconciled` | B.3.4 step 2.3 (two moves, one event each move) |
| `ReadRetry` | `dispatching` | `retry_wait` | BD8; recovery |
| `AgentRemoved` | as 0.4.5 | `cancelled` | section C's removal |

`Event::Fail` (`dispatching -> terminal_failure`) and `Event::Authorize` (`proving_fresh_permit -> queued`) are reused
for reads. `DRIVEN` and the left-over test (CORE `service/reducer.rs:121-148, 195-217`) gain these rows. Every row is
in LC or is P2-1's `completed`.

---------------------------------------------------------------------------------------------------------------------

### B.4 The release record and the release commit

#### B.4.1 The release commit

`proof_done` (CORE `pipeline.rs:655-692`) learns, before its commit and in the same step: the judgement (`judge`,
one `store.read`); for `Accepted::Release`, `let waiting = self.held.waits_for(&job.request_id, &self.connections,
&self.activity, now)` (memory) and the result source (`result_source`, in the same `store.read`). It chooses the
operation type from them (J-store results minor):

| Judgement and reads | Operation type | Commit |
|---|---|---|
| `Accept(Release)`, waiting, source present | `request.release` | release (below) |
| `Accept(Release)`, not waiting | `request.proof` | `ProofRecorded`, `RequestMoved` to `cancelled` (`CallGone`) |
| `Accept(Release)`, waiting, source absent | `request.proof` | `ProofRecorded`, `undecide(result_not_held)` |
| any other | as today | as today |

`release(tx, cx, job, done)` (added, CORE `service/results.rs`):

1. `ProofRecorded` and `ReliedGrantSet(authorizing_grants)`, as `settle` does for every accepted fresh proof
   (CORE `pipeline.rs:773-775`).
2. Build the result:
   - list: `list_result(&tx.read())` (B.4.2);
   - `fabric_request_result` of a read: `held_payload(held_result_id)`, digest checked; decode it as the contract's
     `ToolResult`. Missing, mismatched or undecodable: `undecide(result_not_held)` and stop;
   - `fabric_request_result` of a change: `ToolResult::Effect { event }` from `effect_event(effect_id)`.
3. Size: encode the whole answer frame (`AgentResponse::Result` with the status `completed` and the result). Over
   `MAX_FRAME_BYTES` (CC `common.rs:263`): `undecide(result_too_large)` and stop. Nothing is purged and no grant ends.
4. `ResultReleased { release }` with `release_id` new, `request_id` the releasing request, `subject_request_id` the
   list itself or the target, `agent_id`, `kind`, `proof_id` the fresh proof's, `grant_ids` the proof's
   `authorizing_grants`, `held_result_id` for `availability` and `events`, `result_digest` = `result_digest(result)`
   for `calendars` and `effect` (V3), `released_at_ms` now.
5. For a read target: `HeldResultPurged { cause: Released }`, whose apply deletes the payload row (A.4.3, AD5); the
   target's live one-request grant, if any, ends `consumed` (`end_grant`, raising `revocation_epoch`, CORE
   `grants.rs:98-106`).
6. `RequestMoved` to `completed` (`Release`). For a list, `move_request` ends the list's own one-request grant
   `consumed` as today.
7. `cx.after.released.push((request_id, result))`. `after_commit` moves it into `Held.released` (B.8.3).

`ResultReleased`'s apply guards are A.4.3's, among them: the releasing request belongs to the agent, is in
`proving_fresh_permit` and has no release; the subject belongs to the same agent; `grant_ids` are grants of the agent
(`grants_of`, as `AuthorizationIssued` checks, ST `apply.rs:554-577`); empty exactly for `effect`; `held_result_id`
names an unpurged held result of the subject exactly for `availability` and `events`. The store refuses a second
release of one request (`releases.request_id` UNIQUE) and the invariants I15 and I16 tie `completed` to its row.

#### B.4.2 The list's content

`list_result(r)` builds `result_calendars` from `accounts.record` (CC `owner.rs:655-662`): every account in state
`connected` that has at least one selected calendar, in account id order, with its selected calendars in calendar id
order, each as `{calendar_id, name, access, agent_drafts}` (CC `agent.rs:405-421`). No provider call (W05 CO13). After
migration 3 the same source holds, because the calendar list answer of section D writes `accounts.record` (CO7); ST2
is the map to Google's ids and is never read for a list.

#### B.4.3 What a release record holds and how long

| Column | Content | Calendar content? |
|---|---|---|
| ids, kind, times | ids and `ReleaseKind` | no |
| `grant_ids` | the authorizing grants | no |
| `held_result_id` | the purged held result | no; the payload is gone after the release |
| `result_digest` | `sha256:` over the domain line `fabric-broker-result-v1\n` and the canonical result, only for `calendars` and `effect` | a digest of a low-entropy calendar list; kept, since the list's names are the broker's own records |

A release row is kept with its request for good, like `attempts`. It is what `used` reads (B.7.2) and what the
owner's request details will show (screens session, later).

---------------------------------------------------------------------------------------------------------------------

### B.5 `fabric_request_result` under a `Disclose` proof

#### B.5.1 Validation (`validating`)

`invalid_arguments` (CORE `pipeline.rs:341-384`) takes the agent id. For `Action::RequestResult { request_id: t }` it
replaces the fixed refusal at `pipeline.rs:346-350` with these checks, in order. Each failure is `terminal_failure`
with `invalid_arguments`:

| # | Check | Sentence |
|---|---|---|
| VC1 | `t` names a request of this agent | "No request of this agent has that id." (the same for a foreign and an unknown id, so existence never leaks) |
| VC2 | `t` is not a `fabric_calendar_list` | "A list keeps no result; call fabric_calendar_list again." |
| VC3 | `t` is not a `fabric_request_result` (no nesting: the HOL renders `Disclose (Disclose ...)`, `crates/fabric-broker-policy/src/hol.rs:433-434`) | "A result request has no result of its own." |
| VC4 | `result_source(r, agent, t)` is `Some` | "No result is held for that request." |

VC4 is a fast refusal only. `result_held` is read again with every proof's facts and the source again in the release
commit.

#### B.5.2 The proof job

`proof_job` (CORE `service/facts.rs:105-187`) gains a branch for a request whose action is `RequestResult(t)`:

| Member | Value |
|---|---|
| `action` | `ProofAction::Disclose(target.row.action)` (added variant; never `RequestResult` or `CalendarList`) |
| `action_digest` | the target's action digest (as `ports.rs:158-159` foresees and `ctx_digest` means, THY `:200-204`); so `InFlight.action_digest` and the D39 binding check the target's digest |
| `facts.grants` | `offered_grants` as amended (B.7.1): the agent's live grants, the target's one-request grant when the target holds a result, and every relied grant |
| `facts.result_held` | B.5.3 |
| `facts.relied` | empty for classification; the stored list for the fresh proof |
| `facts.event` | `None` |
| everything else | as for any agent proof |

The prover map (BR `prover/map.rs:58-63`) gains the arm `ProofAction::Disclose(call) =>
Action::from_tool_call(call).map_err(|_| JobRefused(NO_ACTION))?.disclosed()` (`disclosed()` exists,
`crates/fabric-broker-policy/src/model.rs:721-723`). `from_tool_call` still refuses `RequestResult`. The same file
gains `prove_system`'s mapping: `SystemAction::Refresh` to the policy model's `SystemRefresh` and
`SystemAction::Reconcile` to `SystemReconcile` (`ListCalendars` after TH1), with the digest of
`system_action_digest` (B.11.1); test m02.

#### B.5.3 What `result_held` means for each kind of target

| Target tool | `result_source` | `result_held` | Kind released |
|---|---|---|---|
| `fabric_calendar_list` | `None` (refused at VC2) | false | none |
| `fabric_request_result` | `None` (refused at VC3) | false | none |
| `fabric_calendar_availability`, `fabric_calendar_read` | `Held { held_result_id, kind }` when the `held_results` row of `t` exists, belongs to the agent, is not purged and `now < purge_due_at_ms` (BD10) | true exactly then | `availability`, `events` |
| `fabric_calendar_propose_create`, `_update`, `_invitation`, `_response` | `Effect { effect_id, event: Some(e) }` when `t` is `applied` or `reconciled` and `effect_event(effect_id)` is `Some((Some(e), _))` | true exactly then | `effect` with the event |
| `fabric_calendar_propose_delete` | `Effect { effect_id, event: None }` when `t` is `applied` or `reconciled` and `effect_event` is `Some((None, _))` | true exactly then | `effect` with `event: null` |
| any change in another state, `resolved_as_applied` included | `None` | false | none: the owner's statement names no event (DRAFT README section 9) |

#### B.5.4 What the theory then decides

`rows c (Disclose a) = (if is_read a then rows c a else [])` (THY `:373`) and `Result_held` is a block rule
(THY `:549-554, 578, 606-607`):

| Situation | Outcome | Request ends |
|---|---|---|
| result not held | Block `result_held` | `blocked` |
| agent paused | Block `agent_not_paused` | `blocked` |
| account disconnected, calendar deselected | Block `account_connected` / `calendar_selected` | `blocked` |
| read target, every row authorized by a live grant | Permit, `authorizing_grants` not empty | `allowed`, then `completed` |
| read target, a grant ended (revoked, session gone, purged) | NeedApproval on `authority` (THY `:909-920`) | `undecided` (no scope, BD12) |
| change target | Permit with `authorizing_grants` empty (EX `:569-571`) | `allowed`, then `completed` |
| relied grant ended between classification and fresh proof | Block `relied_grant_live` (THY `:577, 604`) | `blocked` |

---------------------------------------------------------------------------------------------------------------------

### B.6 The narrow lifts of D17 (AM-D17)

#### B.6.1 `accepted()` after the lifts

`Context` (CORE `pipeline.rs:136-144`) gains `kind_of_request: RequestKind` (added) in place of `change: bool`:
`Change`, `Read` (availability, event read), `List`, `ReleaseOfRead`, `ReleaseOfChange`. `judge` sets it from the
request's tool and, for a release, the target's tool. `provider_present` becomes `can_carry_out` for the request's
account (V17). The `FreshPermit` arm of `accepted` (CORE `pipeline.rs:157-169`) becomes, in this order:

| Kind | Accepted when (all) | Result | Otherwise |
|---|---|---|---|
| any | the D39 binding and the grant check pass first (unchanged, `judge`) | | `undecided` |
| `Change` | Isabelle, `can_carry_out`, not paused, `authorizing` not empty | `Authorize` | `undecided` (unchanged) |
| `Read` | Isabelle, `can_carry_out`, not paused, `authorizing` not empty | `Authorize` (lift a) | `undecided` |
| `List` | Isabelle, not paused, `authorizing` not empty | `Release` (no provider needed) | `undecided`, cause `release_not_authorizable` when `authorizing` is empty |
| `ReleaseOfRead` | Isabelle, not paused, `authorizing` not empty | `Release` (lift b) | `undecided`, cause `release_not_authorizable` when empty |
| `ReleaseOfChange` | Isabelle, not paused, `authorizing` empty | `Release` (lift c, metadata only) | `undecided`, cause `release_not_authorizable` when not empty: the prover contradicts EX `:569-571` |

`judge` checks every id in `authorizing` valid now before `accepted` runs (CORE `pipeline.rs:239-246`), so a
`Release` rests only on grants valid at the release commit. System proofs never pass through `accepted` (B.11).

#### B.6.2 The text that replaces W04 D17

> **D17 (amended 2026-11-17, CD-W05/W06 phase 2).** A fresh Permit is accepted only from the Isabelle prover, only
> when the agent is not paused, and only after the binding check (D39) and the grant check. Then, by the request:
> a change or a read (availability, event details) issues an authorization when this run can carry it out for its
> account and `authorizing_grants` is not empty; a calendar list, and the release of a read's result, are released
> in their own commit (`completed`) when `authorizing_grants` is not empty; the release of the outcome of the same
> agent's own applied change is released when `authorizing_grants` is empty, and it discloses only the event's id and
> revision. Every other fresh Permit goes `undecided` through `undecide()` with its cause. A permit with no
> authorizing grant is accepted only for that release and for a system proof (B.11), which permits only the
> broker's own read order and never an agent's action. A result whose `authorizing_grants` names an id outside the
> job's facts is refused as before.

---------------------------------------------------------------------------------------------------------------------

### B.7 The amendments of D18 and of `used`

#### B.7.1 D18 (AM-D18)

Four touch points (REV-core CO14, REV-results D18). "Holds a result" means the origin's tool is
`fabric_calendar_availability` or `fabric_calendar_read` (`holds_result(tool)`, added); a list does not.

| Where | Today | Amended |
|---|---|---|
| `offered_grants` (CORE `facts.rs:24-42`) | a one-request grant only to its origin | also to a `fabric_request_result` request of the same agent whose argument names the origin, when the origin holds a result. To no other request |
| `grant_is_valid`, `grant_valid_now` (CORE `grants.rs:181-198`, `pipeline.rs:266-283`) | valid iff the origin is open | origin holds a result: valid iff not ended and not `used` (B.7.2), whatever the origin's state. Otherwise unchanged |
| `move_request` (CORE `requests.rs:41-55`) | ends origin's one-request grants `consumed` at any terminal move | does not end it when the origin holds a result and the move is to `reconciled` (a held result exists). Every other terminal move of a read (blocked, undecided, failed, cancelled, stale, denied, expired) still ends it `consumed` |
| ends | at the origin's terminal move | at the release (`consumed`, B.4.1 step 5), at every purge (V15: `consumed` at a release purge; `account_disconnected`, `agent_removed`; at the 24 h purge `result_expired` once P2-4 merges, `consumed` until then), and by every owner path that ends grants (revoke, disconnect, authority reduced, removal). Each end raises `revocation_epoch` |

Text that replaces W04 D18:

> **D18 (amended 2026-11-17).** A one-request grant is offered to its origin request. When the origin is a read
> (availability or event details), it is also offered to a `fabric_request_result` request of the same agent that
> names that read, and to no other. A one-request grant ends `consumed` in the commit that makes its origin terminal,
> except a read's grant when the read ends `reconciled` with a held result: that grant stays live until the release
> of the result (`consumed`), the purge of the result (with the purge's reason), or any owner action that ends
> grants. It is live meanwhile, also while its origin is `applied` or `outcome_unknown`.

Side effects to record (REV-results D18): such a grant shows as live in the snapshot for up to 24 h after its read
ended; it counts in D38's live-grant budget; the owner can revoke it. W04 5.5's `one_request` row, the desktop README
(`apps/fabric-desktop/README.md:100`) and W04 test c04
(`a_one_request_grant_ends_consumed_with_its_request_and_serves_no_other`) change with it. No stored shape changes:
grant ends are their own `GrantEnded` mutations.

#### B.7.2 `used` (AM-USED)

| Origin | `GrantFact.used` |
|---|---|
| a read that holds a result | a `releases` row exists with `subject_request_id` = the origin (`releases_of_subject`) |
| a change | the origin reached `applied` or `reconciled` (unchanged, CORE `facts.rs:72-77`) |
| a list | the origin reached `completed` |

`used` is computed in `grant_facts`, never stored. Since BD9 ends the grant in the same commit, `ended` and `used`
agree. This finishes W04 L6 ("for a read, by the release of its result") and 5.5, not a new decision.

---------------------------------------------------------------------------------------------------------------------

### B.8 Held answers (P7, LR-C4)

#### B.8.1 Type and place

`CORE service/held.rs` (added file), memory only, a field `held: Held` of `Core` beside `connections` (CORE
`service/core.rs:86`); empty on every open. `HeldAnswer` and `settled` are 0.5.1's; `Held`, with two return types
fixed here:

```rust
pub(crate) struct Held {
    answers: BTreeMap<u64, HeldAnswer>,       // by seq
    next_seq: u64,
    pub(crate) released: HashMap<Id, ToolResult>, // results released in this step, by releasing request
}
impl Held {
    pub(crate) fn hold(&mut self, answer: HeldAnswer) -> bool;  // false at MAX_HELD_PER_CONNECTION on that connection
    pub(crate) fn waits_for(&self, request_id: &Id, connections: &HashMap<ConnectionId, Connection>,
                            activity: &Activity, now_ms: i64) -> bool;
    pub(crate) fn forget_connection(&mut self, connection: ConnectionId) -> Vec<HeldAnswer>; // dropped, no reply
    pub(crate) fn next_deadline_ms(&self) -> Option<i64>;
    pub(crate) fn take_all(&mut self) -> Vec<HeldAnswer>;       // quit
}
pub(crate) fn settled(state: RequestState) -> bool; // is_terminal, pending_approval, applied, outcome_unknown
pub(crate) fn answer_reply(connection: ConnectionId, call_id: Id, tool: ToolName, request: &Request,
                           result: Option<ToolResult>) -> AgentReply; // status_reply with a result parameter
```

An entry is valid only while `connections[entry.connection]` exists, its `session_id` equals `entry.session_id`, and
that session is the agent's live one (`activity.sessions`). `waits_for` is true when a valid entry exists for the
request and `now_ms < entry.deadline_ms`. At most one entry per request exists, because a repeat is never held.

#### B.8.2 What is held and what is answered at once

| Call | Held? |
|---|---|
| an action whose request was inserted (`propose`, CORE `agent.rs:300-306`, on `Ok`) | held with `deadline_ms = now + AGENT_ANSWER_HOLD_MS`; when `hold` returns false (16 on that connection), answered at once with its status |
| a repeated call id with the same action (`agent.rs:238-253`) | at once, status only, never a result |
| `fabric_request_status` (`agent.rs:204-219`) | at once, status only |
| `invalid_params`, `busy`, `internal`, `not_permitted`, `prover_unavailable` | at once |
| the handshake (`agent_hello`, `agent_prove`) | at once (section C) |

#### B.8.3 What wakes a held answer

`Core::wake_held()` (added) runs at the end of `step_ready`, after the second `pump` and before the last `drain`
(CORE `broker.rs:252-254`), and after the pump of `Broker::seed`. Every lifecycle move of the step has happened by
then, the pump's included (PART-results F4). It:

1. reads every held request in one `store.read`;
2. for each valid entry in `seq` order: when `settled(state)` or `now >= deadline_ms`, pushes `answer_reply(...,
   self.held.released.remove(&request_id))` and removes the entry;
3. removes invalid entries without a reply;
4. clears `released` (a result never outlives its step; an entry for it cannot be missing, see below);
5. returns the entries of a list or release that left unsettled; the core cancels each (BD4) in one commit
   `request.advance` per request.

The deadline is on the core clock, which never goes back (CORE `core.rs:212-223`). `next_due_raw_ms`
(`core.rs:227-238`) adds `held.next_deadline_ms()`, so the main loop ticks for it.

Why a released result always finds its entry: the release commit read `waits_for` true in the same step. Within one
step nothing removes an entry before `wake_held` but `end_idle_sessions` in the pump, which ends a session only when
`now > last_active + SESSION_IDLE_MS`; the call was activity at most `AGENT_ANSWER_HOLD_MS` before `now`, and
20 000 < 1 800 000. A test pins this (B.16 h15).

#### B.8.4 Every way a connection ends, and quit

| Event | Held answers | Request |
|---|---|---|
| disconnect (`agent.rs:144-171`) | `forget_connection` first: dropped, no reply | goes on; a list or release is cancelled (BD4) |
| session replaced or idle (`after_commit` stale loop, `core.rs:294-309`) | `forget_connection` for each stale connection | as above |
| agent removed (section C closes it) | `forget_connection`, then close | removal cancels what was not handed over (A.4.8) |
| a call the client cancelled (MCP `notifications/cancelled`; the bridge drops the call) | with P2-30: the adapter's cancel names the call id, and that one entry is dropped with no reply, as `forget_connection` drops a connection's entries; a cancel for an id that is not held is ignored | goes on as for a disconnect: a list or release is cancelled (BD4), and a release that finds no waiting call releases nothing (BD3). The cancel is not a request and creates none |
| handshake failure, frame before hello | none can exist (no session) | none |
| quit (`recovery.rs:103-124`) | before the `broker.quit` commit: `let held = self.held.take_all()` and one `store.read` of their requests, keeping only entries that are valid then (B.8.1). After the commit returns: push `answer_reply(.., None)` with that status for each kept entry, then the closes. A fault of the commit answers nothing. The commit ends every live session, so `after_commit`'s stale loop (CORE `service/core.rs:294-309`) removes every bound connection inside `write`; taken after it, `Held` and `connections` would be empty | stays as it is; at the next start recovery cancels open lists and releases (BD4) |
| halt | nothing reaches an agent (C.9.6) | recovered at the next start |
| restart | lost with the connections | recovered (W04 5.8) |

Every path that removes a connection from `Core.connections` calls `forget_connection` first; at quit it finds
nothing, because `take_all` ran before the commit. `Output.agent` is written before `Output.close` (C.9.6), so the
answers at quit leave before the closes. The entry's
`session_id` check (B.8.1) is the second line when a path forgets to (J-auth results minor).

A call the client cancelled. An MCP client can cancel a call (`notifications/cancelled`); the bridge then drops it,
but schema version 3 as drafted has no message that tells the broker. Request P2-30 asks for one on the agent
channel; when it arrives the core drops the held answer as the row above says (test h16). Until the message exists,
and for a cancel that arrives after the release was committed, the broker may release a result into an answer the
bridge throws away; under a permission for one request that permission is then used up. That is a residual of the
same kind as OD17 (a result whose answer is lost in the instant it is sent); the CD-W06 design calls it R14 (ruled
2026-11-18).

#### B.8.5 Limits

| Limit | Value | Source |
|---|---|---|
| held answers per connection | `MAX_HELD_PER_CONNECTION` = 16; past it the answer is given at once, never refused | P7 |
| per agent | 16, since an agent has one live session and an ended session's connections are closed | `crates/fabric-storage/migrations/0001_initial.sql:58`; CORE `core.rs:294-309` |
| writer queue per connection | `WRITER_QUEUE_FRAMES` = 32 and `WRITER_QUEUE_BYTES` = 4 MiB (section C) | P7 |
| successful handshakes | `SESSIONS_PER_AGENT_WINDOW` = 6 in `SESSION_START_WINDOW_MS` (section C) | P7 |
| new requests | 15 per minute per agent, 25 open per agent (CC `common.rs:275-277`) | so the 16 cap is a second line; test h10 fills it with a test limit |

Answers on one connection may leave in any order; the bridge matches by id (W06 3.6).

---------------------------------------------------------------------------------------------------------------------

### B.9 Held results: storing, purging, keeping

| Event | Commit (operation type) | Mutations |
|---|---|---|
| read stored | `request.read` | B.3.4 |
| released | `request.release` | B.4.1 |
| account disconnected | `owner.disconnect_google` | for each unpurged held result with that `account_id` (`unpurged_held_results_of_account`): `HeldResultPurged { AccountDisconnected }` + payload delete; the read grants end `account_disconnected` (most already end by `end_grants_of_account`, CORE `grants.rs:153-174`) |
| agent removed | `owner.remove_agent` | step (4) of the removal order (A.4.8), over `unpurged_held_results_of_agent` |
| 24 h passed | `timer.result_purge` at `purge_due_at_ms` (`due_purges(now)`; `next_purge_due_ms()` joins `next_due_raw_ms`) | `HeldResultPurged { Expired }`, the grant ends (V15) |
| start | `broker.started` | every held result past its due time is purged as above |
| backup | `backup::take` | V18: every payload row deleted in a copy of version 2 or later, before it is verified and named |

A revocation does not purge: PLAN 9.2 allows a later release under "an existing still-valid broader grant and fresh
proof" (PLAN `:333`). The release is still refused unless such a grant exists (B.5.4). The owner may choose the stricter
purge (B.18, second question; E.3 OD16). The purge is a store fact, the predicate of BD10 is the guard: a late sweep can
never release an expired result.

---------------------------------------------------------------------------------------------------------------------

### B.10 The answer `prover_unavailable` (P8, LR-C3)

| Item | Rule |
|---|---|
| When | `propose` (CORE `agent.rs:226-330`), after the repeat check (`agent.rs:235-254`) and before the limits transaction (`agent.rs:287`): `if !self.prover_ready()` answer `error_reply(.., AgentFault::ProverUnavailable, "The rule checker is not ready. Nothing was recorded. Try again later.")` (`AgentFault::ProverUnavailable` exists, CC `error.rs:366`) |
| `prover_ready()` | true exactly when a prover is attached and either its status is `Ready` (P0's `ProverStatus`; today `Core.prover_health == ProverHealth::Ready`, CORE `core.rs:83, 395-403`) or its kind is `ProverKind::Synthetic` (debug builds only; the classifier answers every job at once and reports its status as unavailable by design, BR `synthetic.rs:93-99`). Ruled by the lead 2026-11-17 (I.3 B-8): the exemption is by the prover's kind, in debug builds only |
| What it creates | nothing: no request, no ledger row, no audit row, no activity beyond the call's own (`record_call` already ran, `agent.rs:202`) |
| Which calls | every action, `fabric_request_result` and `fabric_calendar_list` included. Not `fabric_request_status`, not a repeated call id, not the handshake |
| Held? | no, answered at once |
| No prover attached | not ready, so every call that would create a request is refused and nothing is recorded. This amends W04 D14 for agent calls (E.1.9, owner OD18); a request already open when a proof is due and no prover is attached still ends `undecided` at once with no report. Tests that change: W04 c18, `no_prover_ends_every_request_undecided_with_no_report` (`crates/fabric-broker-core/tests/proofs.rs:38`), and every core test that sends an action to a core with no prover attached. The synthetic classifier of E.1.2 row 2 reports its status as unavailable (BR `synthetic.rs:93-99`; CORE `service/core.rs:111-114` copies it), so `prover_ready()` counts the kind `Synthetic` as ready and its tests do not change |
| Open requests at a restart | unchanged: their proofs end `undecided` with P0's cause until the planned follow-up holds them while Isabelle starts (P8) |
| Retry | the contract's recovery is "retry later" (CC `error.rs:149`); the bridge relays it and does not retry (W06 3.7) |

---------------------------------------------------------------------------------------------------------------------

### B.11 System proofs (CO9)

#### B.11.1 The action

`SystemAction` (0.5.1): `Refresh { calendar }` (theory `System_refresh acct cal`), `Reconcile { calendar,
event_id }` (`System_reconcile acct cal ev`), `ListCalendars { account_id }` (TH1, request P2-6; not issued before it
merges). `system_action_digest(action)` is the contract's digest formula (README section 4) over the broker's own
canonical JSON `{"system": kind, "account_id", "calendar_id" | null, "event_id" | null}` (W04 CCR-14), so the
prover's binding check compares it. This digest is the broker's own canonical form, not the contract's: DRAFT README
section 15 lists "actions the broker starts itself ... and the formal context with its digest" as not covered. It is
computed only by `system_action_digest`, which the provider workers and the prover adapter (BR `prover/map.rs`,
B.5.2) call.

#### B.11.2 The facts

`TrustedFacts` read in one transaction: `actor: FactActor::System`; `paused: false`; `session: None`; `grants: []`;
`recipient_sets: []`; `relied: []`; `result_held: false`; `event: None`; `connected`, `calendars`,
`warning_calendars`, `profile` as for an agent proof; `pending_effects` from `open_effects` (which `System_reconcile`
needs, THY `:522-523`); both epochs. Fence: `fence_now(counters, None)`. The theory needs nothing else
(`system_authorized`, THY `:519-524`; the rules that apply to a system action, THY `:567-588`).

#### B.11.3 The job and its acceptance

`SystemProofJob` and `SystemProofDone` as 0.5.1; `Prover::prove_system`. The core keeps system jobs in
flight by `job_id` with `deadline_ms = start + PROOF_WATCHDOG_MS`, each with `expects`, the state of the request it
serves when it started (`validating` or `proving_fresh_permit` for `Refresh`, `reconciling` for `Reconcile`). Every
result, accepted or not, is first matched: the job is in flight in this run, and when `serves` names a request,
that request is still in `expects`; otherwise the result is ignored and nothing is recorded. A matched result is
accepted when all hold: `done.action_digest == job.action_digest`; the prover is Isabelle; the outcome is `Permit`;
the fence holds. No grant is involved; `accepted()` is never called.

| Result | Commit `system.proof` |
|---|---|
| accepted | `SystemProofRecorded`, then the order (`Job::Observe`, `Job::Reconcile` or `Job::List`) with `proof: OrderProof::System { proof_id, action_digest }`, after commit (PD29, PO8) |
| fence changed | `SystemProofRecorded`; a new system proof for the same need, at most `MAX_REPROOFS` per (serving request, action kind) in this run; then as refused. The new system proof starts only through the checks of D.6.1 (D.7.2 step 1 for a reconcile); when one fails, none starts and the request goes on as D.6.1 says |
| Block, NeedApproval, Undecided, no report, watchdog, another prover kind | `SystemProofRecorded` when a report exists; the serving request is handled as a refused observation or reconcile read (section D's tables) |

The core starts no system proof while the prover is not Ready (D.6.1, D.7.2); a job the worker still answers with no
report is the third row.

#### B.11.4 Where their reports are stored and shown

In `system_proofs` (A.4): one row per report, never updated or deleted (triggers as on `proof_reports`),
`serves_request_id` naming the request the order serves, null for a list at connect. The owner sees them in the
request's details through request P2-5 (`system_proofs: [system_proof_report]`). A list at connect serves no request
and is shown nowhere in version 1; its row is kept for the record.

---------------------------------------------------------------------------------------------------------------------

### B.12 The fence with a stored event revision (P10)

| Item | Rule |
|---|---|
| Shape | `Fence { pause_epoch, revocation_epoch, event_revision }` unchanged (ST `rows.rs:25-32`); stored proofs and authorizations keep it |
| The proof's fence | `proof_job` sets `event_revision` to the revision of the request's latest `observations` row with `purpose = observe` and `outcome = found` (`latest_observation`), read in the facts' transaction; `None` when the request has none |
| The current fence | `fence_now(counters, observed_revision)` with the same reader for the same request. Replaces `epochs::fence(counters)` at CORE `pipeline.rs:236` (judge), `:471` (handoff), the read storing commit (B.3.4) and the system proof acceptance (with `None`) |
| Comparison | `fence_holds(proved, now)`: all three members equal (the existing equality, CORE `epochs.rs:28-31`). It now holds for an observed proof because `now` carries the stored revision, not `None` |
| What changes it | a pause, any grant end and every narrowing command (`revocation_epoch`, P10's one test); a new observation of that request with another revision |
| Order | this change merges before any code writes an `observations` row (step P5 of P12, in P5a; E.2.1, X4); a test pins that an observed proof is accepted while the stored revision is unchanged and discarded after another revision is stored |

---------------------------------------------------------------------------------------------------------------------

### B.13 The deadline of every provider order (P10, V20)

| Order | Deadline | Watchdog at | On the watchdog |
|---|---|---|---|
| `DispatchOrder` | issue + `PROVIDER_CALL_MS` (30 s; today set at CORE `pipeline.rs:632`, never watched) | `deadline_ms + ORDER_WATCHDOG_MARGIN_MS` (35 s) | as `NoAnswer`: `AttemptFinished(outcome_unknown)`, `dispatching -> outcome_unknown`, then reconciliation (section D). Never a repeat |
| `ReadOrder` | same | same | `ReadFinished(no_answer_in_time)`, BD8 |
| `ObserveOrder` | same | same | as `Failed(Transient { None })` in section D's CO6 table |
| `ReconcileOrder` | same | same | as a failed read-back (section D) |
| `ListOrder` | same | same | as `Failed(Transient { None })` (section D) |

The core keeps `orders: BTreeMap<u64, OrderInFlight { kind, request_id: Option<Id>, expects: Option<RequestState>,
account_id, deadline_ms, still_allowed: Option<Arc<AtomicBool>> }>` (added) and `next_order_id`, both memory only.
`expects` is the state the order's request is in when the order is sent (`dispatching` for a dispatch and a read,
`validating` or `proving_fresh_permit` for an observe, `reconciling` for a reconcile; `None` for a list). An answer
whose request is no longer in `expects` is ignored as D.5 says. `still_allowed` is the read order's flag (D.2), kept
for a read only: `after_commit` stores `false` into the flag of every read order in flight when the commit raised
`pause_epoch` or `revocation_epoch`, before the step's jobs leave. It is a hint that saves pages; the guard is B.3.4
step 2.2. `next_due_raw_ms` adds the earliest `deadline_ms + margin`. An answer
whose `order_id` is not in flight is ignored and not recorded; this replaces today's match by effect and attempt
number for dispatches (CORE `pipeline.rs:696-720`). After a restart no order is in flight; recovery handles requests
left in `dispatching` (B.3.3; W04 5.8).

The worker's bound is not `deadline_ms`. `deadline_ms` is on the core clock, which is the raw wall clock plus an
offset that only grows: whenever the wall clock goes back, and at start when the last commit is ahead of it (CORE
`service/core.rs:211-238`). With an offset of d ms a worker that tested `now < deadline_ms` on the raw clock would
start an order up to d ms late. So the shell stamps every job with `std::time::Instant::now()` when it takes it from
`Output.jobs`, in the turn of the sending commit, and passes `until = stamp + PROVIDER_CALL_MS` to the provider
method with the order (D.2). The worker refuses to start an order past `until` and bounds every HTTP call by
`until - Instant::now()`. `deadline_ms` stays in the order for the record and for the core's watchdog (X23). A
worker that did not start in time answers at once: `Transient { retry_after_ms: None }` for a dispatch, because
nothing was sent (DD6, D.5, D.12; X24); `Failed(Transient)` for the others. So an order that waited behind a stuck
one cannot act on authority more than 30 s old. A worker that starts late after the core gave up changes nothing
the core relies on (a dispatch is then reconciled; a read answer is ignored). The core's watchdog fires 35 s after
the sending commit on the core clock, which is at least 35 s of real time unless the wall clock jumps forward; then
it can fire before the worker's bound, the request goes to reconciliation, and absence still needs two reads 60 s
apart on the core clock (DD7). The smaller alternative, the core writing `deadline_ms - clock.offset_ms()` into the
order as `next_due_raw_ms` already converts, is not chosen: a wall clock that goes back after the order was sent
still moves the worker's bound (I.7 X-4; the lead ruled for the monotonic stamp on 2026-11-17).

---------------------------------------------------------------------------------------------------------------------

### B.14 Keeping the applied event

`finish_attempt` (CORE `pipeline.rs:917-989`), arm `Applied { event, confirmed, notification }`:

| Case | Fits when | Commit `request.attempt` |
|---|---|---|
| create | `confirmed` and `event.event_id == effects.provider_event_id` | `AttemptFinished(applied)`, `EffectEventRecorded { effect_id, event: Some(e), notification, at_ms }`, `RequestMoved` to `applied`, then to `reconciled` |
| update, invitation, response | `confirmed` and `event.event_id` equals the action's target event | same |
| delete | `confirmed` and `event` is `None`. `confirmed` comes from the adapter's one GET in the delete's own dispatch order: true only when it finds the event gone (D.2) | same, with `event: None` |
| create, update, invitation, response, not confirmed (V23) | `event` fits as in the rows above; `confirmed` is false | `AttemptFinished(applied)`, `EffectEventRecorded { effect_id, event: Some(e), notification, at_ms }` while the request is in `dispatching`, one audit row `provider_differs` with the request id and no content, then `RequestMoved` `dispatching -> outcome_unknown -> reconciling -> needs_owner_resolution` with no read, until P2-33 adds the edge `dispatching -> needs_owner_resolution` |
| delete, not confirmed | `event` is `None`; `confirmed` is false | as `NoAnswer` (BD16): `AttemptFinished(outcome_unknown)`, `dispatching -> outcome_unknown`; no audit row |
| anything else (`event` missing on a non-delete, an event on a delete, another id) | | as `NoAnswer` (BD16): `AttemptFinished(outcome_unknown)`, `dispatching -> outcome_unknown`; one audit row `provider_mismatch` with the request id and no content |

`EffectRow` and `AttemptFinished` keep their shapes (they are inside version 1 ledger payloads, J-store effect event
defect). The columns `event_id_after`, `revision_after`, `notification` are read only by `effect_event` (A.4 and
A.4.2). The reconcile commit of section D writes `EffectEventRecorded` too when it finds the approved change. The
event is kept for good: the agent needs it to update or delete what it made, and reconciliation needs it.

A delete has no body. Its `confirmed` comes from one GET of the same event that the adapter makes in the same
order, only after a 200 or 204 (D.2). An unconfirmed delete is not a mismatch and writes no audit row; it is
reconciled by reading (D.7). Ruled 2026-11-18 by the lead.

A fitting `Applied` that is not confirmed (V23) means Google made the change and returned content that differs
from the approval; the adapter's comparison names the member as a fixed label with no content (W05 18.7). It is
never sent again, never read back and can never end `absence_confirmed`: under the first rule a PATCH that
Google normalised without moving the revision could end absent after two reads (D.7.3, "not yet"), and a new
request could then repeat a change that notifies guests. The owner sees what Google holds and can resolve it
only as applied (P2-34; E.3 OD25). The chain through `outcome_unknown` and `reconciling` is an interim until the
contract adds the edge (P2-33); it writes no observation, no order and no due time. The event as Google returned
it is the attempt's evidence, at most `MAX_EVIDENCE_BYTES` (1 048 576): past it the id and revision are still
recorded, the request still goes to `needs_owner_resolution` and no evidence row is written; it is never
refused, because the change has already happened. The owner is shown it as `private.after` (P2-32), at most
`MAX_BEFORE_VIEW_BYTES`. The port member that carries that event to the core and the ledger's name for the
attempt's evidence row are not drawn in D.2 or A.4 yet; they are fixed with P2-32, before the live run. P2-31 to
P2-34 are needed before any Google dispatch, so until they merge V23 is reachable only by a practice fault
(E.1.4 row 8a). Ruled 2026-11-18 by the lead.

A store written by main holds changes in `applied`: on main `applied` ends a change and is not terminal ("`applied`
stays open until CD-W05", PW 4.1 at `calendar-desktop-prover-wiring-design.md:594`), and recovery does nothing for
it. No code of phase 2 moves a request out of `applied` but the attempt's own commit (V7). So recovery
(`broker.started`) moves every request it finds in `applied` to `reconciled` (the edge exists, CC `lifecycle.rs:145`),
with no `EffectEventRecorded`: such a change has no recorded event and its outcome is never released (BD11, B.5.3).
Otherwise they would stay open for good and count against the limits of 100 open requests and 25 per agent (CC
`common.rs:275-276`). Test:
`a_store_written_by_main_has_no_request_in_applied_after_its_first_start` (A.8.2).

---------------------------------------------------------------------------------------------------------------------

### B.15 The full path of a read, with every authority check

Checks are numbered A1 to A18. "Fails" says what happens; nothing in a failure path discloses content.

| Step | Where | Check | Fails |
|---|---|---|---|
| 1. The agent's bridge connects and proves its key | section C, handshake in the core | A1 the signature verifies, the agent is approved, the key is its approved key, the nonce is fresh | `not_permitted`, no session |
| 2. `tools/call` `fabric_calendar_read` | CORE `agent.rs:175-201` | A2 the connection is bound to the agent's live session | `not_permitted`, audited |
| | `agent.rs:235-254` | A3 a repeated call id returns the status only | |
| | B.10 | A4 `prover_ready()` | `prover_unavailable`, nothing recorded |
| | `agent.rs:287-299` | A5 limits in the insert transaction | `busy`, nothing recorded |
| 3. `received -> validating` | `pipeline.rs:341-384` | A6 account known and connected, every calendar known and selected, window valid | `terminal_failure` |
| 4. Classification proof on `Agent(read)` | `facts.rs:105-187`, `judge` | A7 D39 binding; A8 grant check; A9 fence (B.12); then the theory: actor, pause, mode, account, calendar, authority under the live grants | `undecided`, re-proof, `blocked`, or a card |
| 5. Owner's card (if asked) | `decide_request` | A10 the owner approves the scope or narrows it, picks the duration | `denied`, `expired`, `stale` |
| 6. Fresh proof | `judge`, `accepted` | A9 again; A11 every authorizing grant valid now; A12 the D17 rule for reads (B.6.1); the theory sees `relied` | re-proof, `stale`, `blocked`, `undecided` |
| 7. Handoff | `handoff()` `pipeline.rs:448-488` | A13 authorization issued, this run, unexpired, fence, grants valid, not paused | void, back to a fresh proof |
| 8. The read order | provider worker | A14 the order names its proof (PO8); the worker refuses an order past its monotonic bound (B.13); `still_allowed` before each page (hint) | `Stopped`, nothing returned |
| 9. Storing commit | B.3.4 | A15 shape; fence, grants and pause again (V6) | nothing stored, `retry_wait` |
| 10. The read's held answer | B.8.3 | settled at `reconciled`, answered with the status only (R5) | |
| 11. `tools/call` `fabric_request_result` | as step 2 | A2 to A5 again | as step 2 |
| 12. Validation | B.5.1 | A16 the target is this agent's read with a held result | `terminal_failure`, one sentence for foreign and unknown ids |
| 13. Classification proof on `Disclose(read)` | B.5.2 | A7 to A9; the theory: `result_held`, pause, account, calendar, authority of every row of the read under the grants now | `blocked`, `undecided` (no card) |
| 14. Fresh proof on `Disclose(read)` | `judge`, `accepted` | A9, A11; A17 the D17 rule for a release of a read; `relied_grant_live` | re-proof, `stale`, `blocked`, `undecided` |
| 15. Release commit | B.4.1 | A18 the call still waits (BD3); the payload is there and its digest matches; the frame fits | `cancelled` or `undecided`, nothing released |
| 16. The answer | `wake_held` in the same step | R1 to R4 | |
| 17. The agent holds the content | the bridge relays it and keeps nothing (W06 A12) | | |

Any owner action between steps raises an epoch or ends a grant, so the next check that reads the fence (A9, A13,
A15, the release's A9) sees it. Between a check and the commit that relies on it nothing else commits: the core is one
thread and each check runs in the step of its commit.

---------------------------------------------------------------------------------------------------------------------

### B.16 Tests

#### B.16.1 Question 10: "polling a completed read after revocation discloses nothing"

Where: `crates/fabric-broker-core/tests/q10.rs` (added), the core in process with a scripted provider double that
answers read orders and a gate double for proofs; cases Q2, Q3 and Q8 also run with live Isabelle and the practice
provider once it answers read orders (P11). The same file runs against FAKE in step P7b of P12.

The plant, for every case: one event on a selected calendar whose title is `q10-` and 32 random hex digits (the
marker), read with `fields` including `title`; for Q17 the marker is a calendar's name. The marker is random per run,
so no fixture can contain it by accident.

The scan, `assert_marker_only_where_allowed(run, allowed)` (added test helper), looks at the bytes of: every
`Output.agent` frame; every `Output.owner` frame, notices and `get_request_details` answers both redacted and full;
every `Output.stderr` line; every `ledger` row's payload; every `audit` row; every projection table, dumped as text;
`held_result_payloads`; and, where the case says, the files of a backup. `allowed` names the places the marker may be
in at the end: none, or `held_result_payloads` (the result is still held), or exactly one agent frame (the release
answer of the positive case). Each case first checks the positive control: after the read the marker is in
`held_result_payloads`, so the plant worked.

| # | Case | Plants and does | Expects | Scan: marker allowed in |
|---|---|---|---|---|
| Q1 | Positive control, one-request grant | read approved `one_request`; `reconciled`; `fabric_request_result` | one answer with the result; `releases` row; grant ended `consumed`, `used`; payload purged (`released`); a second `fabric_request_result` ends `terminal_failure` "No result is held for that request." | exactly that one agent frame |
| Q2 | Revoke a one-request grant | Q1's read; owner `revoke_grant` before any release; `fabric_request_status` on the read; then `fabric_request_result` | status `reconciled`, `result` null; the release request's own status polls carry null; it ends `undecided` (NeedApproval, no card); no `releases` row; grant not `used` | `held_result_payloads` only |
| Q3 | Revoke an `until_revoked` grant | read under `until_revoked`; revoke; as Q2 | as Q2 | `held_result_payloads` only |
| Q4 | Session grant ended | read under a `session` grant; end the session by (a) disconnect, (b) idle timeout (clock past 1 800 000 ms), (c) quit and restart; then reconnect and `fabric_request_result` | `undecided` in each; status polls null | `held_result_payloads` only |
| Q5 | Pause | read under `until_revoked`; owner pauses the agent; `fabric_request_result` | `blocked`, denial `agent_not_paused` with its explanation | `held_result_payloads` only |
| Q6 | Account disconnected | read; `disconnect_google` of its account; `fabric_request_result` | purge `account_disconnected`; grant ended; `terminal_failure` "No result is held for that request." | none |
| Q7 | 24 h passed | read; clock past `purge_due_at_ms`; one tick; `fabric_request_result` | purge `expired`; grant ended (V15); `terminal_failure` | none |
| Q8 | Revoke between the release's two proofs | read; `fabric_request_result` classified `allowed`; revoke before the fresh proof's facts are read | fresh proof blocks `relied_grant_live`; no release | `held_result_payloads` only |
| Q9 | Revoke while the release's fresh proof runs | gate double holds the fresh proof; revoke; release the proof's `Permit` | `judge` discards (fence); re-proof blocks; no release; nothing in any agent frame | `held_result_payloads` only |
| Q10 | The call left before the release | gate double delays the fresh proof past 20 s | the held answer leaves with an in-between state and `result` null; the request ends `cancelled` (BD4) or at the release check (BD3); no `releases` row; grant unused; result still held | `held_result_payloads` only |
| Q11 | Disconnect during the release | disconnect while the fresh proof runs | no reply; `cancelled`; no `releases` row | `held_result_payloads` only |
| Q12 | Repeat after a release | Q1; repeat the release call's id; poll status of both requests | status `completed` and `reconciled`, `result` null | the one frame of Q1 |
| Q13 | Another agent's read | agent B asks `fabric_request_result` and `fabric_request_status` for agent A's read id | `terminal_failure` with the same sentence as an unknown id; `not_found` for the status | `held_result_payloads` only |
| Q14 | Revoke during paging | provider double with three pages; revoke after page one | `Stopped` or the storing check fails; `ReadFinished(stopped or stale)`; nothing stored; the next fresh proof blocks | none |
| Q15 | Revoke after the last page, before the storing commit | provider double returns `Done`; revoke is processed first | `ReadFinished(stale)`; nothing stored | none |
| Q16 | The release commit fails | a store fault injected in the release commit | halt; no agent frame carries a result (W04 c12 to c14) | `held_result_payloads` only |
| Q17 | The list after its grant is revoked | list completes (marker in a calendar name); owner revokes the enrollment grant; poll status; call the list again; call `fabric_request_result` on the first list | status `completed`, `result` null; the new list opens a card with an empty scope (P1), answered `pending_approval` with `result` null; the result request ends `terminal_failure` | the first list's one answer frame |
| Q18 | Agent removed (lands in step P6 with ct17, since `remove_agent` is P6's) | read; owner `remove_agent` | purge `agent_removed`; connection closed with no reply to held calls | none |
| Q19 | Backup | read held; close the core; reopen the data directory with `Store::open_with_migrations` and a test-only version N+1 (as p03 does, `apps/fabric-broker/tests/process.rs:1283-1300`), which takes the backup | the backup copy holds no payload row (V18) | `held_result_payloads` of the live store only, not the backup files |
| Q20 | A change's outcome after every grant is revoked | create applied; revoke every grant; `fabric_request_result` | `completed` with `event_id` and revision, `grant_ids` `[]` (BD11); after deselecting the calendar, `blocked` | (no marker: metadata only) |
| Q21 | A paused agent and an event it names (also the core test `a_paused_agent_causes_no_observe_and_gets_agent_not_paused_for_every_event_state`) | agent paused; an update naming an event (a) at the named revision, (b) at another revision, (c) absent; (d) the pause arrives while the first system proof of an update runs | one answer for all four: `blocked`, `agent_not_paused`; `orders` stays empty and no `observations` row is written (D.6.1) | none |

Q20 pins the owner-visible consequence of lift c (J-auth results minor). Q17 needs CD21 and the enrollment grant,
which step P5b builds (E.2.1); Q18 lands in step P6. G2a is put to the owner without these rows
(W04 Q10 default); they are proven at G2b and G2c.

#### B.16.2 Other tests of this section

| # | Test | Catches |
|---|---|---|
| h01 | `an_action_is_first_answered_when_it_settles_not_at_received` | the answer at insert (`agent.rs:300-306`) |
| h03 | `a_held_answer_leaves_at_20_s_on_the_core_clock_with_the_state_of_that_moment` | a deadline never checked or on the raw clock |
| h04 | `next_due_includes_the_earliest_hold_order_and_purge_deadline` | the loop never ticking |
| h06 | `a_disconnect_drops_held_answers_and_cancels_a_waiting_list` | a reply to a closed connection |
| h08 | `quit_answers_each_held_call_with_its_status_before_its_close` | dropped answers at quit |
| h09 | `a_repeated_call_id_is_answered_at_once_and_never_held` | two entries for one request |
| h10 | `the_seventeenth_held_call_on_a_connection_is_answered_at_once` (test limits above 16 per minute) | no cap |
| h11 | `fabric_request_status_never_carries_a_result_in_any_of_26_states` (frame bytes) | R5 |
| h15 | `a_released_result_always_finds_its_held_answer` | B.8.3 argument |
| h16 | `a_cancelled_held_call_is_dropped_and_a_later_release_for_it_releases_nothing_and_spends_no_permission` (P2-30) | a release into an answer the bridge threw away (B.8.4) |
| r01 | `a_list_under_the_list_grant_completes_with_connected_accounts_and_selected_calendars_only` | content beyond the selection |
| r03 | `a_list_whose_call_left_ends_cancelled_with_no_release_row_and_operation_type_request_proof` | V10 |
| r04 | `the_request_release_row_commits_before_the_answer_and_its_digest_is_the_answers_result` | R4 |
| r05 | `the_accepted_table_of_b6` (one test per row of B.6.1) | a lift wider than written |
| r07 | `the_applied_event_is_recorded_and_a_mismatched_one_goes_to_reconciliation` | BD16 |
| r11 | `a_completed_state_survives_close_and_open` | I1 not updated |
| r16 | `the_disclose_job_carries_the_target_digest_and_result_held` | a digest of the request_result action |
| r21 | `a_read_never_gets_an_effects_row_and_recovery_moves_it_to_retry_wait` | I19; `outcome_unknown` for a read |
| r22 | `a_stale_read_retries_within_the_budget_then_fails` | an unbounded loop |
| r23 | `a_read_result_with_a_field_or_an_interval_outside_the_action_is_not_stored`: a provider double returns one extra member, one interval outside the window, one interval that starts before the window, and a title-only event outside it (its span outside the window); also an all-day event two days before the window, a span count that differs from the event count and a timed view whose time differs from its span, each rejected; and, accepted, an all-day event on the day before the window (the known residual of `ZONE_SLACK_MS`, I.5 D-18) and an all-day event on the window's edge day in a calendar at `Pacific/Kiritimati` and in one at `Pacific/Pago_Pago` | a field cut missed by the adapter released under a narrower grant; an all-day span widened by the adapter, or not widened by the core |
| r24 | `a_read_without_time_puts_no_instant_of_an_event_anywhere`: an event planted at a random distinctive second, read with `fields` naming `title` only; the scan of B.16.1 looks for its RFC 3339 text and its millisecond value; allowed places: none | a span stored, logged or shown, or a time member filled when `time` is not granted (DD17) |
| f01 | `an_observed_proof_holds_while_the_stored_revision_is_unchanged` | today's equality against `None` |
| v01 | `an_update_whose_start_is_after_the_events_end_gets_the_same_first_answer_as_one_before_it`: an agent with no grant, one event at its revision, two updates that set only a start, one after the event's end and one before it: both open a card | a start-before-end check before classification, which tells the agent the event's times |
| o01 | `every_order_kind_is_watched_and_a_late_answer_is_ignored` | an order left in flight |
| o02 | `the_worker_does_not_start_an_order_past_its_deadline` | late action under old authority |
| o03 | `an_order_held_past_its_monotonic_bound_is_not_started_whatever_the_clock_offset`: a core whose clock offset is 120 000 ms; an order held 31 s before the worker takes it is not started and answers `Transient` | a worker bound on the raw clock (X23) |
| p01 | `a_call_is_refused_prover_unavailable_while_not_ready_and_nothing_is_written` | P8 |
| p02 | `a_synthetic_classifier_run_still_creates_requests_and_opens_cards` | `prover_ready()` read as status `Ready` only, which the synthetic classifier never reports |
| s01 | `a_system_proof_is_recorded_whatever_its_outcome_and_only_a_permit_issues_its_order` | an order without a proof |
| m01 | `a_disclose_job_maps_to_disclose_of_its_target_with_the_target_digest` (BR `prover/map.rs`) | `UNKNOWN_ACTION` still returned |
| m02 | `a_system_job_maps_to_system_refresh_or_reconcile_with_the_system_action_digest` (BR `prover/map.rs`) | a system proof mapped to an agent action or bound to another digest |

---------------------------------------------------------------------------------------------------------------------

### B.17 Answers to the requests this section owns

| Id | Asks | Answer | Where |
|---|---|---|---|
| CO5 | Reads per W05 section 9: a read order after a fresh permit, the held result, `fabric_request_result` as `Disclose`, `result_held` and `used` in the facts, the release in the outcome only, R08 by the flag | Accepted with changes. A read is authorized and handed off like a change, with `read_attempts` in place of `effects` (BD7); it ends `reconciled` in the storing commit, which re-checks authority (BD8, V6); `Stopped` and a failed check go `retry_wait`; recovery never makes a read `outcome_unknown`. The release is its own request on `Disclose(read)`, ending `completed` (BD2). The flag is a hint; the guards are the storing commit and the release | B.3.3, B.3.4, B.3.5, B.4, B.5 |
| CO9 | System proofs: `FactActor::System`, the system actions, their digest, a job without a request and with no grant, the retry rule | Accepted as a separate path (BD17): `Job::ProveSystem`, its own acceptance rule (Isabelle `Permit`, bound digest, fence), never `accepted()`; recorded in `system_proofs` whatever the outcome; the order is issued in the same commit and names the proof. Retries reuse `retry_due`. `ListCalendars` waits for TH1 (P2-6) | B.11 |
| CO13 | `fabric_calendar_list` from the store after its proof, no provider call | Accepted. Its own fresh `Permit` on the enrollment grant (or an approval grant) is its release; the list is built from `accounts.record` in the release commit; no provider is needed; ends `completed`. A list that needs approval opens a card whose scope names no calendar (P1) | B.3.2, B.4.2, BD5 |
| CO14 | The amendments D17, D18, D33, `used`; reducer rows | Accepted. D17 as three narrow lifts (B.6); D18 and `used` (B.7); D33 replaced (B.17.1). Reducer rows of this section in B.3.7; section D owns the reconcile and `stale` rows. The owner accepts the D17 and D18 texts, which are owner-visible rules of the desktop README (W04 L2) | B.3.7, B.6, B.7 |
| AM-D17 | Lift both D17 conditions for a read order and a release | Lifted narrowly, three cases only: a read with a grant (a), a read's release with a grant (b), a change's outcome with no grant (c). Everything else in D17 stays | B.6 |
| AM-D18 | A read's one-request grant stays live after the read, offered to the release, ending at release or purge | As asked, with the exact touch points and end reasons | B.7.1 |
| AM-D33 | D33 goes | Replaced by a written successor | B.17.1 |
| AM-USED | `used` for a read means "released" | As asked: a `releases` row with that subject | B.7.2 |
| LR-C3 | `prover_unavailable` while the prover is not ready | Answered in C.8 (section C owns it); B.10 says where the check sits in `propose`: after the repeat check and before the limits; status always answered | C.8, B.10 |
| LR-C4 | (1) Held answers; (2) results for G2c: the list, an applied change's outcome, `fabric_request_result` with its own proof, then reads | (1) Memory only, 16 per connection, woken at the end of every step on settle or 20 s, dropped at disconnect, replacement, idle and removal, answered at quit. `outcome_unknown` settles (P7, against W06's list). (2) All three, plus reads; the first step on the practice calendar needs no Google | B.8, B.3, B.4 |
| LR-C5 | The rule of S8: a status never carries content; release only by a fresh proof under current authority; the poll test | R1 to R6 and the twenty-one cases of question 10, scanning bytes, not types | B.2, B.16.1 |

#### B.17.1 The successor of D33 (text)

> **D33 (replaced 2026-11-17, CD-W05/W06 phase 2): Results and their release.** A result leaves the broker only in
> the answer to a `fabric_calendar_list` or `fabric_request_result` call whose own fresh proof released it, and only
> while that call still waits; the release and the answer happen in the same step or not at all, and each release
> writes one `releases` row before the answer leaves. A status never carries content. The calendar list is released
> by its own fresh Permit and built from the broker's records. A read (availability, event details) is authorized
> and handed off like a change: its handoff consumes an authorization and records a read attempt, and no read reaches
> the provider without both. The commit that stores a read's result checks the fence, the read authorization's grants
> and the pause again; the read then ends `reconciled` with a held result, and its own answer carries no result.
> `fabric_request_result` names a request of the same agent that is a read with a held result or a change with a
> recorded outcome; anything else ends `terminal_failure` (`invalid_arguments`) with one sentence for an unknown and a
> foreign id. Its proofs are on `Disclose` of the named request with that request's digest. It ends `completed` in
> the commit that releases. A release never asks the owner in version 1. The broker's own reads (observe,
> reconcile, list at connect) each take a system proof and are carried out only by an order that names it. A held
> result is purged at its first release, its account's disconnect, its agent's removal and after 24 h.

---------------------------------------------------------------------------------------------------------------------

### B.18 Decision for the owner: two calls for a read

**The question.** PLAN 9.2 says "Allow returns the proven filtered read result or the effect lifecycle" (PLAN `:331`).
Under P6, version 1 departs from that first sentence for reads: an allowed read is answered `reconciled` with no
result, and the agent calls `fabric_request_result`, whose own `Disclose` proof releases the result (PLAN `:333`:
"Reading a completed result is a disclosure action ... fresh live proof").

**Default the design runs on: A, two calls.**

| | A: two calls (default) | B: a `Disclose` proof inside the hold of the read's own answer |
|---|---|---|
| How | read, then `fabric_request_result` (B.3.3 to B.4) | after the storing commit, while the read's held answer still waits, the core runs one `Disclose(read)` proof for the read itself and releases into the read's answer; if the hold ends first, the result stays held and the agent uses path A |
| Isabelle checks per read | 4: classification and fresh permit on the read, classification and fresh permit on `Disclose` | 3: no classification of the release (no card is possible for it) |
| Prover time per read | 4.0 s to 7.2 s (one check 1.0 s to 1.8 s, PW L3 at `calendar-desktop-prover-wiring-design.md:815`) | 3.0 s to 5.4 s; saves 1.0 s to 1.8 s |
| Agent calls per read | 2, so one more model turn (its time is the client's, not measured here) | 1 when everything fits in 20 s; otherwise 2 |
| Fits in the 20 s hold | each call separately: about 2 s to 3.6 s of checks plus the read order (up to 30 s) for the first | checks of 3.0 s to 5.4 s plus the read order must all fit in 20 s, behind any other proof in the queue (proofs run one at a time) |
| Release paths | one | two (in the read's answer, and A as fallback); twice the tests of question 10 |
| Contract | P2-1, P2-2 only | also a proof purpose `release` (today `ProofPurpose` has two values, CC `owner.rs:1058-1061`) or a lifecycle edge out of `reconciled`, and README wording for a read whose answer may or may not carry its result |
| Store | as designed | a proof report of a third purpose filed under the read; I15 and the release guards widened to a read releasing its own result |
| Safety | the same rule R1 to R6 | the same rule R1 to R6, on two paths |

Recommendation: A for version 1. It is one path, it is the stricter reading of PLAN 9.2, and B can be added later
without changing a stored shape that A writes. If the owner chooses B, the contract session must add the purpose or
edge first (one more request to it), and steps P2 and P5 of P12 grow by that path and its tests.

**The owner's answer.** On 2026-11-18 the owner chose A, two calls (OD1; PLAN 15.3, NOV18-READ-CALLS). He also
chose no card for a release (OD2; NOV18-RELEASE-CARD).

**Other owner questions this section raises** (collected with P13 in E.3 as OD2, OD3 with OD16, OD15 and OD17):

| Question | Default | If he chooses otherwise |
|---|---|---|
| No card for a release (BD12) | none in version 1: a release that needs approval ends `undecided`; a new read can ask | the contract session adds a card for a release (V3-10); the card would show a scope for content the owner cannot see |
| How long a held result is kept | 24 h, purged at its first release, at disconnect, at removal | a shorter life is one constant; the stricter option also purges when every grant the read relied on has ended (REV-results retention-OQ5), so a revoked read can never be released later even under another grant |
| The outcome of a change is released with no grant (BD11) | yes, until a pause, a disconnect, a deselect or the agent's removal | revoking every grant would not stop it; to stop it the theory's `rows (Disclose change)` must change (contract session) |
| A lost answer after a release (OP13) | the result is gone; the agent reads again | none cheaper in version 1 |

---------------------------------------------------------------------------------------------------------------------

### B.19 Open points, changes to section 0 and names added

#### B.19.1 Open points

| # | Point | Recommendation | Owner |
|---|---|---|---|
| BOP1 | A list approved through a card ends `cancelled` (BD5); under a one-request approval the next list asks again, so under `ask` no list completes. The owner accepted that for version 1 on 2026-11-18 (OD24) | If he does not accept it: keep an unused one-request list grant live when its origin was cancelled by `CallGone`, and offer it to the agent's next list (the list action has no arguments, so the digest is the same); one disclosure still happens; built in step P5b. Default: not in version 1. Ruled 2026-11-17 by the lead: accepted for version 1, with the card's text (SR-7) and P2-29; the owner accepted it on 2026-11-18 (OD24) | owner (OD24), lead for the build |
| BOP2 | The cancellation of BD4 at 20 s also cancels a release that a slow prover queue delayed | Accept: the agent calls again; the held result and its grant are untouched. Ruled 2026-11-17: accepted | lead |
| BOP3 | The `still_allowed` flag and the storing check use the global fence, so any grant end anywhere stops reads in flight (REV-core "one read order") | Keep V6 in version 1; measure restarts on the practice calendar (OP4). Ruled 2026-11-17: the whole fence in version 1, measured on the practice calendar in step P5 | lead |

#### B.19.2 Changes to section 0 made by this section

1. `CallGone` (0.4.5) moves to `cancelled` from `received`, `validating`, `proving_classification`,
   `allowed`, `approved` and `proving_fresh_permit`, not only from `proving_fresh_permit`; recovery uses it (BD4).
2. `Held::forget_connection` returns the dropped entries (`Vec<HeldAnswer>`), and `Held::waits_for` takes the
   connections and the activity (`waits_for(&self, request_id, connections, activity, now_ms)`), so that an entry
   counts only while its connection is bound to its live session.
3. P2-2 (README section 8) also replaces "and an Allow returns the result" (DRAFT README line 190) with "an Allow of
   a list returns the list; an allowed read is answered when its result is held, and the agent then calls
   `fabric_request_result`", and adds "its outcome is unknown" to the states that answer an action (P7).
4. `latest_observation(&Id)` returns the latest row with `purpose = observe` and `outcome = found` (B.12); reconcile
   rows never feed a fence.
5. V9 widens: an `Applied` whose event does not fit the order is handled as `NoAnswer` too (BD16), with one audit row
   `provider_mismatch` (a new audit kind, no migration).
6. The operation type `system.proof` names every recorded system proof (accepted with its order, or refused), not only
   accepted ones.

#### B.19.3 Names added

`Accepted::Release`; `RequestKind { Change, Read, List, ReleaseOfRead, ReleaseOfChange }` and `Context.kind_of_request`
(replacing `Context.change`); `ProofAction::Disclose` is 0.5.1's; `answer_reply`; `holds_result(tool)`;
`Core::read_done`; `Core::wake_held` (0.5.1); `release(tx, cx, job, done)` in `service/results.rs`; `OrderInFlight`
(with `expects` and `still_allowed`) and `Core.orders`; the shell's monotonic stamp and `until` (X23); tests o03, r23,
m02, Q21, h16 and r24; the test file `crates/fabric-broker-core/tests/q10.rs` and its helper
`assert_marker_only_where_allowed`; the audit kind `provider_mismatch`; and `provider_differs` (V23, ruled
2026-11-18).

---------------------------------------------------------------------------------------------------------------------

## C. The agent channel

**What this section covers.** The agent channel of the broker after phase 2: the enrollment of an agent on the
owner channel (add, approve with the pairing code, remove), the list permission that the approval gives, the
handshake that every agent connection makes before its first tool call, sessions and the closing of connections,
the refusal of new requests while the rule prover is not ready, the Unix socket listener in the broker binary, the
configuration text for the three clients, and what a release build must not contain. It answers the lead requests
LR-C1, LR-C2, LR-C3, LR-C6, LR-C7, LR-S4 and LR-B1 to LR-B5 of W06 7.1 and 7.2 (table in C.13), and ends with the
requests to the contract, screens and bridge sessions.

**What binds it.** Rulings P1, P4, P5, P7, P8 and P9; the names of section 0 (V11 to V14, V21, V22, 0.4.6, 0.5.3 and
0.6) and the commit orders of A.4.8. The store tables `enrollments` and `enrollment_grants`, the mutations
`EnrollmentPut` and `EnrollmentGrantInserted`, the gated branches G1 to G9 and the invariants I8 to I14 and I24 are
section A's; this section uses them. Held answers and releases (`Held`, `HeldAnswer`, `waits_for`,
`forget_connection`) are section B's; this section says where the agent channel calls them. The practice mode and the
debug seed are section E's.

**Status.** Proposed, documents only (step P1 of P12). Built in step P6 (the core and the binary), on top of P3
(migration 2) and P5 (held answers); the list grant's helper `create_enrollment_grant`, X16 and CD21 come earlier, in
step P5b, so that a list can complete in step P5 (E.2.1). Code read at main `2d41210`; the contract draft read at
`29cdfdf`.

Abbreviations as in 0.1: CORE `crates/fabric-broker-core/src/`, BR `apps/fabric-broker/src/`, ST
`crates/fabric-storage/src/`, CC `crates/fabric-desktop-contracts/src/`, DRAFT the contract draft at `29cdfdf`,
W04, W06, PART-agents, J-store, J-auth. "README" means `contracts/desktop/v1/README.md` of DRAFT.

---------------------------------------------------------------------------------------------------------------------

### C.1 Decisions

| Id | Decision | Why |
|---|---|---|
| CD1 | The contract `Agent` is built from two store rows: the `enrollments` row (state, key, times) and the `agents` row (paused, the session's foreign key). A pending agent has no `agents` row. A seeded debug agent has an `agents` row and no `enrollments` row and counts as approved (V13) | P1, P3; `agents` and `AgentPut` stay untouched (ST `apply.rs:86-104`), so replay of version 1 rows is unchanged (PART-agents 1.1) |
| CD2 | The contract revision of an agent is `enrollments.revision + agents.revision`, a missing row counting 0 (V14). Every `expected_revision` check and every `conflict` detail of an agent uses it, `pause_agent` included | P3. A new `AgentPut` row must start at revision 1 (ST `apply.rs:86-88`), so the sum is the only form that never goes back across approval (J-store, revision defect) |
| CD3 | `add_agent` sets `destination` from the client by a fixed table in the core (C.3.1) | DRAFT README section 7 ("sets `destination` from the kind of client") fixes the rule, not the labels; the labels go to the contract session as P2-22 |
| CD4 | `approve_agent` refuses in this order: `not_found`, `conflict` (revision), `conflict` (not pending), `invalid_params` at `/params/pairing_code` (no key, or another code), `invalid_params` at `/params` (owner budget). Then one commit: `AgentPut`, `EnrollmentPut` approved, `create_enrollment_grant`, notices `Agent` then `Grant`. The result carries the grant | P1 (DRAFT choice 61), P5, A.4.8; J-store notice-order defect |
| CD5 | `remove_agent` follows P4's order in one commit and raises `revocation_epoch` at least once, also when no grant ended (a pending agent) | P4, P10 (`remove_agent` is in `NARROWING_COMMANDS`); the same rule `disconnect_google` keeps (CORE `owner/commands.rs:258-261`) |
| CD6 | A seeded debug agent cannot be approved or removed: `approve_agent` answers `conflict` (not pending), `remove_agent` answers `invalid_params` at `/params/agent_id` | V13: a seeded agent has no `enrollments` row, and `EnrollmentPut` accepts a new row only as `pending` (A.4.3). Debug builds only |
| CD7 | Every decision of the handshake is in the core, in `CORE service/handshake.rs`. The binary passes raw lines in (`AgentInput::Line`) and frames out (`AgentReply`) | P9 |
| CD8 | The signature is checked with `ed25519-dalek` 2.2.0 `VerifyingKey::verify_strict`, by a pure function that returns a type `Proven` whose fields are private to `handshake.rs`. Only a function that takes `Proven` reads the store | P9 ("verifies Ed25519 strictly with the crate already in the lock file", "reads the store only after the signature verifies"); `ed25519-dalek 2.2.0` is at `Cargo.lock:330-332` |
| CD9 | While pending, the same key proved again writes nothing; another key replaces it at most once per `KEY_CHANGE_INTERVAL_MS` and at most `MAX_KEY_CHANGES` (20) times per enrollment | Bounds the ledger: a waiting agent cannot add a row per tool call (PART-agents 2.3), and a process that knows the id cannot add rows for ever (J-auth, handshake cost defect) |
| CD10 | Successful handshakes are limited per agent to `SESSIONS_PER_AGENT_WINDOW` (6) in `SESSION_START_WINDOW_MS` (60 s), counted in memory. Past it, `agent_prove` gets `not_permitted` and a close, with no commit, counted `too_many_sessions` | P7; J-auth (a connect and close loop otherwise grows the ledger by two rows per loop: CORE `service/agent.rs:130-139`, `service/sessions.rs:33-60`) |
| CD11 | A frame on a connection that has not ended its handshake with `approved` gets `not_permitted` when it decodes, the layer's error when it does not, and in both cases the connection is closed and counted `frame_before_hello` | V12; DRAFT README section 8 ("A tool frame on a connection whose handshake did not end with `approved` gets `not_permitted`") |
| CD12 | `AgentInput::Connect` and `AgentInput::Call` exist only under `cfg(debug_assertions)`. A release build reaches `start_session` from an agent only through a successful `agent_prove` | V11; J-auth "missing" (release-build test) |
| CD13 | The core keeps the ids it closed until their `Disconnect` arrives (`closing`), and drops lines on them without an answer or a count | J-auth listener race (3): a line queued behind a close would otherwise be read as a new connection |
| CD14 | A challenge has a deadline on the core's clock, `HANDSHAKE_MS` after its hello; the pump drops a late one, closes the connection and counts `timed_out`. This is a second guard behind the binary's read deadline | P9 ("a handshake has 5 s"), kept true by the core whatever the threads do |
| CD15 | The listener binds only while the store is open, at the first `Shell::route` that sees `broker.store_ready()`. A stale socket file is removed only then, under the store's `broker.lock` | P9; ST `store.rs:251-252` takes the lock for the life of the store; a store never goes back from ready to unavailable (a failed write halts, CORE `broker.rs:407-412`) |
| CD16 | The peer's user id is compared with the broker's effective user id on every accepted connection, through the crate `nix` (unix targets, features `user` and `socket`). `run/` and a stale socket must also be owned by that user | P9, LR-B3. `std` has no stable call, `unsafe` is forbidden (`Cargo.toml:29`), `nix` is not in `Cargo.lock` (not found), `rustix 1.1.5` is (`Cargo.lock:1203-1205`) but offers the Linux call only (PART-agents 3.7). New dependency: owner's review (C.16) |
| CD17 | The binary's counts of connections in a handshake and of open connections are changed only by the main thread, after the core handled the input, except the accept thread's increment. Refusals at accept are counted in atomics that the main thread drains each turn into `AgentInput::Refused` | J-auth listener race (2): the binary then never counts fewer handshakes than the core does, and the accept thread never sends on the main channel |
| CD18 | Each connection has a writer thread with a queue of `WRITER_QUEUE_FRAMES` (32) frames and `WRITER_QUEUE_BYTES` (4 MiB). A frame that does not fit shuts the connection; the main thread then feeds `Disconnect` straight into the broker | P7; J-auth (8 frames was below 16 held answers) |
| CD19 | At quit the main thread waits at most `QUIT_WRITE_BOUND_MS` (500 ms) for the writer threads to flush, then removes the socket file | P7 ("quit answers each with its status and then closes"); J-auth (nothing waited for the last answers) |
| CD20 | While `prover_ready()` is false, a call that would create a request gets `prover_unavailable` at once and nothing is written. A repeated call id still gets its request's status, and `fabric_request_status` is always answered | P8 |
| CD21 | A calendar list request has a scope that names no calendar, so a list that needs approval gets a card (C.4.2). A grant made from such a card has purpose `""` | P1, DRAFT README section 10 and choice 50; today `scope_of` gives a list no scope (CORE `service/requests.rs:120-157`) and `create_grant` takes the purpose from `task_of`, which is `None` for a list (`service/grants.rs:50-51`, `service/requests.rs:93-95`): that would halt the broker |
| CD22 | `get_agent_configuration` is answered by a function the binary gives the broker at start (`ConfigurationTexts`), which calls `fabric_mcp_bridge::config_text::config_text`. The broker binary depends on the bridge's library crate | P9 (LR-B5); W06 A24 (one function makes the text) |
| CD23 | The snapshot carries every agent: pending, approved, removed and seeded. `add_agent` counts the agents' records into the owner budget and refuses past it | P1 (a removed agent stays in the snapshot), so agents only grow; the frame bound needs a cap (`OWNER_STATE_BUDGET_BYTES`, CORE `owner/views.rs:28-33`) |
| CD24 | Connection ids of the listener start at `FIRST_LISTENER_CONNECTION` (`1 << 32`); the synthetic agent keeps `ConnectionId(1)` | P4; BR `synthetic.rs:284` |

---------------------------------------------------------------------------------------------------------------------

### C.2 The agent record

#### C.2.1 From two rows to one contract `Agent`

`agent_view` (CORE `service/sessions.rs:122-155`) is rewritten to read both rows. The DRAFT version of it fills the
new members as if every agent were approved (DRAFT `service/sessions.rs`, "an agent the store holds is
approved"); that stays only for a seeded agent.

| `Agent` member (DRAFT CC `owner.rs`) | Pending | Approved | Removed | Seeded (debug, V13) |
|---|---|---|---|---|
| `agent_id` | `enrollment_id` | same | same | `agents.agent_id` |
| `revision` | `e.revision` | `e.revision + a.revision` | same | `a.revision` |
| `registered_name`, `destination` | from `EnrollmentRow` | from `EnrollmentRow` (equal to `AgentRow`) | same | from `AgentRow` |
| `enrollment` | `pending` | `approved` | `removed` | `approved` |
| `pairing_code` | `pairing_code(key)` or null before a key | `pairing_code(key)` | `pairing_code(key)` or null | null |
| `added_at` | `added_at_ms` | same | same | `enrolled_at_ms` |
| `key_proven_at` | `key_proven_at_ms` or null | same | same or null | null |
| `approved_at` | null | `approved_at_ms` | `approved_at_ms` or null (removed while pending) | `enrolled_at_ms` |
| `removed_at` | null | null | `removed_at_ms` | null |
| `paused` | false | `a.paused` | `a.paused` or false | `a.paused` |
| `session` | null | the live session, as today | null (I11) | as today |
| `last_tool_call_at` | null | from memory (W04 D35) | from memory or null | from memory |

`e` is the `enrollments` row, `a` the `agents` row. `pairing_code` is `fabric_desktop_contracts::handshake::
pairing_code` (DRAFT CC `handshake.rs:83-93`); the code is never stored, only the key. The reader is
`ReadTx::agent_views()` (0.5.2): pending, approved, removed and seeded agents, ordered by `added_at` (for a
seeded agent `enrolled_at_ms`), then by id.

`raise_agent` (CORE `service/sessions.rs:15-29`) keeps raising `agents.revision`; the contract revision rises with
it because it is a sum. A step that writes `EnrollmentPut` and `AgentPut` in one commit raises the sum by two; the
page sees one notice per change (C.2.2) and only ever a higher revision.

Every reader of `agents` (ST `read.rs:576` `agents`, `:589` `agent`) and what it does at version 2:

| Call site | Reads | Decision |
|---|---|---|
| the snapshot, CORE `owner/views.rs:50` | `agents()` | reads `agent_views()` instead: pending, approved, removed and seeded |
| `pause_agent` one agent, CORE `owner/commands.rs:182` | `agent(id)` | reads `agent_view(id)`; a pending or removed agent gets `conflict` (C.3.5) |
| `pause_agent(AllAgents)`, CORE `owner/commands.rs:191` | `agents()` | reads `agent_views()` and targets approved and seeded agents only |
| `raise_agent`, CORE `service/sessions.rs:22` | `agent(id)` | unchanged. Safe because removal ends the session (step 1 of C.3.3) before `EnrollmentPut` removed (step 6), and no `AgentPut` is applied to a removed agent after that (G7). This order is a rule of the removal |
| `agent_view`, CORE `service/sessions.rs:127` | `agent(id)` | rewritten to read `agent_view(id)` of the store (C.2.1 table) |
| the facts for proofs, CORE `service/facts.rs:118` | `agent(id)` | unchanged: the requesting agent's own row. A removed agent that was approved keeps its `agents` row (I9), so a request that runs on (`dispatching`, `outcome_unknown`, `reconciling`) still finds it |
| `judge` and `handoff`, CORE `service/pipeline.rs:247`, `:480` | `agent(id)` | unchanged, for the same reason; the pause read there stays the authority check |
| the debug `Core::connect`, CORE `service/agent.rs:121` | `agent(id)` | accepts only a seeded agent: an `agents` row and no `enrollments` row. Any other id gets the `unknown_agent` audit row and a close. Today it accepts any id with an `agents` row, so a debug `Connect` of a removed agent would commit `SessionStarted`, G5 would refuse it and the broker would halt; an approved enrolled agent would get a session with no handshake |
| the debug `Core::seed`, CORE `service/agent.rs:343-346` | `agent(id)` | skips an agent that exists, as today; it also gives the list grant through `create_enrollment_grant` to a seed agent that has an `agents` row, no `enrollments` row and no `enrollment_grants` row, so a store written before migration 2 gets it once; a revoked one is not given again (V22) |
| the apply of `AgentPut` and of `SessionStarted`, ST `apply.rs:87`, `:112` | `agent(id)` | unchanged; G7 and G5 are their gated branches |

Every reader of grants sees both grant tables (P5; section A, G1 to G3 and G6).

#### C.2.2 What the snapshot and the notices carry

- `get_snapshot.agents`: every agent of `agent_views()`, removed ones included (P1; DRAFT choice 52).
  `get_snapshot.grants`: the live grants of both tables, the list grant among them (G6).
- The only notice for an agent is `Change::Agent` with the whole record (P1). It is sent at: `add_agent`, every
  new proved key (`agent.prove_key`), `approve_agent`, `remove_agent`, a session start or end (as today, through
  `raise_agent`), and `pause_agent`.
- `Change::Grant` is sent for the list grant at approval (after the agent notice, CD4) and for every grant that a
  removal or a revoke ends (as today, `service/grants.rs:85-115`).
- A proved key that is the agent's current key sends nothing and writes nothing (CD9).

---------------------------------------------------------------------------------------------------------------------

### C.3 The owner commands (LR-C1)

All four are wired in `CORE owner/dedup.rs` (`operation_type` maps a method to `owner.<method>`, `dedup.rs:32-48`) and
`owner/mod.rs`, replacing DRAFT's "not implemented in this build" arms. The three commands carry a command id and
are deduplicated as every owner command is; `get_agent_configuration` is a query and writes no row. The command
functions go in a new file `CORE owner/agents.rs`.

#### C.3.1 `add_agent` (operation type `owner.add_agent`)

Params `{registered_name, client}` (DRAFT CC `owner.rs:1898-1905`).

1. `enrollment_id = ids::new_id("agent")`. If `agents` or `enrollments` already has it, that is an invariant fault
   (128 random bits), not a refusal.
2. `destination` from the client (CD3):

   | `client` | `destination` |
   |---|---|
   | `claude_desktop` | `{kind: cloud, processor: "Anthropic"}` |
   | `claude_code` | `{kind: cloud, processor: "Anthropic"}` |
   | `codex_desktop` | `{kind: cloud, processor: "OpenAI"}` |

3. `EnrollmentPut` with a new row: `pending`, revision 1, no key, `added_at_ms = now`.
4. Budget (CD23): if the agents' records now take the owner budget past `OWNER_STATE_BUDGET_BYTES`, refuse
   `invalid_params` at `/params/registered_name` with a fixed message ("Fabric Calendar holds too many agents to
   show them in one snapshot; this build cannot add another."). `owner_state_bytes` (CORE `owner/views.rs:72-77`)
   gains the agents' records, as P10 makes it gain the accounts (section D).
5. Notice `Change::Agent(view)`. Result `{commit_sequence, agent}`.

#### C.3.2 `approve_agent` (operation type `owner.approve_agent`)

Params `{agent_id, expected_revision, pairing_code}` (DRAFT CC `owner.rs:1915-1922`). Refusals, first one wins (CD4):

| # | Check | Answer |
|---|---|---|
| 1 | No `enrollments` row and no `agents` row with that id | `not_found` |
| 2 | `expected_revision` is not the contract revision (CD2) | `conflict` with the current revision |
| 3 | The state is not `pending` (approved, removed, or a seeded agent) | `conflict` with the current revision; message "This agent is not waiting for approval." |
| 4 | No key proved, or `pairing_code(key)` differs from the code given | `invalid_params` at `/params/pairing_code` (DRAFT choice 61) |
| 5 | After step (3) of the commit, the grants take the owner budget past its bound (`within_budget`, CORE `owner/commands.rs:57-66`) | `invalid_params` at `/params`, the existing budget message |

A key proved after the owner looked raised the revision, so check 2 refuses it before check 4 (PART-agents 2.4).

The commit, in this order (A.4.8):

1. `AgentPut` with a new row: `agent_id = enrollment_id`, revision 1, the name and destination of the enrollment,
   `paused: false`, `enrolled_at_ms = now`.
2. `EnrollmentPut`: `approved`, the same key, `approved_at_ms = now`, revision + 1. Its guard requires the
   `agents` row of step 1 (A.4.3).
3. Notice `Change::Agent(view)`.
4. `grants::create_enrollment_grant(tx, cx, &agent_id)` (C.4.1): `EnrollmentGrantInserted` and its notice
   `Change::Grant(grant)`.

Result: `{commit_sequence, agent, grant}`; `commit_sequence` is the grant notice's. DRAFT's
`ApproveAgentResult` has no `grant` (DRAFT CC `owner.rs:1924-1930`); P2-21 asks for it (C.15.1). After the approval
the key is fixed: `EnrollmentPut` refuses any later key change of an approved row (A.4.3).

#### C.3.3 `remove_agent` (operation type `owner.remove_agent`)

Params `{agent_id, expected_revision}`. Refusals: no row at all `not_found`; a seeded agent `invalid_params` at
`/params/agent_id` (CD6); `expected_revision` not current `conflict`; already `removed` `conflict` (message "This
agent was removed already.").

The commit, in P4's order (A.4.8):

1. A live session of the agent ends: `end_session(tx, cx, &session_id, SessionEndReason::AgentRemoved)`. Its
   session grants end `agent_removed` through `grant_end_reason` (section A adds the mapping), and `raise_agent`
   sends an agent notice (CORE `service/sessions.rs:102-104`).
2. Every other live grant of the agent ends `GrantEndReason::AgentRemoved` through `end_grant`: grants until revoked,
   one-request grants (read grants included), and the list grant in `enrollment_grants` (G1, G3).
3. Every authorization of the agent in state issued gets `AuthorizationVoided { reason: VoidReason::AgentRemoved }`;
   then every open request of the agent in `received`, `validating`, `proving_classification`, `allowed`,
   `approved`, `proving_fresh_permit`, `queued`, `retry_wait` or `pending_approval` moves to `cancelled` by the
   reducer event `AgentRemoved` (0.4.5). Requests in `dispatching`, `outcome_unknown`, `reconciling`,
   `applied` and `needs_owner_resolution` run on (DRAFT choice 52; OP10). A request of a removed agent never
   retries: where B.3.4 (BD8), D.7.1 (`Transient`) or recovery would later move it `dispatching -> retry_wait`,
   the same commit moves it on `retry_wait -> cancelled` (reducer event `AgentRemoved`; X22). So no request of a
   removed agent reaches a fresh proof or opens a card; `outcome_unknown`, `reconciling` and
   `needs_owner_resolution` open none.
   Then every entry of `orders` and every system proof in flight that serves a request this commit moved is
   dropped; a late answer or result for it is ignored (B.11.3, D.5).
4. `HeldResultPurged { cause: AgentRemoved }` for each unpurged held result of the agent (section B).
5. If no grant ended in steps 1 and 2, `epochs::raise(tx, Counter::RevocationEpoch)` once (CD5).
6. `EnrollmentPut`: `removed`, the key unchanged, `removed_at_ms = now`, revision + 1.
7. Notice `Change::Agent(view)` with the removed record. It is the last notice of the commit.

Result `{commit_sequence, agent, ended_grant_ids}` (DRAFT CC `owner.rs:1931-1945`). A pending agent has no
session, grant or request, so its removal writes steps 5 to 7 only.

After the commit: the removed agent's connection is closed in the same step's `Output.close` (C.7.3); its held
answers are dropped without a reply (P7, `Held::forget_connection`); a proof or an attempt result for a request
cancelled in step 3 changes nothing, because `judge` ignores a request no longer in its proving state (CORE
`service/pipeline.rs:216-222`); a challenge open for that id fails at its `agent_prove` with `removed`.

#### C.3.4 `get_agent_configuration` (a query)

Params `{agent_id}`. A pending or approved agent: `{configuration: texts}` where `texts` is the result of the
binary's `ConfigurationTexts` for that id, or null when the broker cannot name an installed bridge (C.10). An
unknown id, a removed agent, or a seeded agent: `not_found` (a removed id is never accepted again, so a text naming
it would mislead). No ledger row and no audit row.

#### C.3.5 Commands that change because of enrollment

| Command | Change |
|---|---|
| `pause_agent` (CORE `owner/commands.rs:172-215`) | One agent: the revision check uses CD2; a pending or removed agent gets `conflict` with the current revision ("This agent is not approved."). `AllAgents` targets approved and seeded agents only; with none it answers `not_found` as today |
| `decide_request` (CORE `owner/commands.rs:75`) | Answers `stale_request` when the request's agent has an enrollment that is not approved. Removal cancels the agent's `pending_approval` requests in the same commit, and a removed agent's request never retries (C.3.3), so no card should open after a removal; this refusal keeps an owner's click from reaching G4, which stays the store's second line |
| `revoke_grant` (CORE `owner/commands.rs:222-243`) | Revokes the list grant like any other, through `end_grant` with `revoked_by_owner` (G1, G3); the agent then gets a card for its next list (C.4.2) |

---------------------------------------------------------------------------------------------------------------------

### C.4 The list permission (P5)

#### C.4.1 The grant at approval

`grants::create_enrollment_grant(tx, cx, agent_id) -> Result<(Grant, PositiveCount), StoreFault>` (V21) is the only
writer of `enrollment_grants`. `approve_agent` calls it; the debug seed calls it for a seeded agent (V13, section E).
The grant, in DRAFT's shape (DRAFT README section 10, choices 49 and 50):

| Member | Value |
|---|---|
| `grant_id` | `ids::new_id("grant")` |
| `revision` | 1 |
| `agent_id` | the agent |
| `purpose` | `""` (P2-15) |
| `tools` | `[fabric_calendar_list]` |
| `scope` | `{calendars: [], window: null, fields: null, recipients: null, send_updates: null}` |
| `term` | `until_revoked` (`expires_at: null`) |
| `origin` | `Enrollment { enrollment_id: agent_id }` |
| `policy` | the active profile and its version, as `create_grant` reads them (CORE `service/grants.rs:52`) |
| `revocation_epoch` | the current one |
| `created_at` | `now` |
| `ended` | null |

It writes `EnrollmentGrantInserted` and sends `Change::Grant`. It raises no epoch (a grant that widens authority
does not, as `create_grant` today). It ends only `revoked_by_owner` or `agent_removed` (I14). An agent gets at most
one such row ever (V22); after a revoke the permission comes back only through a card, into `grants`.

The core code that reads `grant.origin.request_id` (CORE `service/pipeline.rs:276`, `service/requests.rs:49`,
`service/facts.rs:38, 70`) learns `GrantOrigin::Enrollment` in step P5b (X16), with `create_enrollment_grant` and the
debug seed's call of it, so that a list can be authorized in step P5 (r01, e14, Q17): an enrollment grant has no origin
request, is never a one-request grant and has no open origin. It counts into the owner budget (CORE
`owner/views.rs:76`; A.4.6), which is why `approve_agent` has check 5 of C.3.2.

What it permits: under a row of `list_and_availability` whose mode is `allow` (both built-in profiles, DRAFT
`data/policy-matrix.json`), the calendar list. The list's answer is built from the broker's records and released
by the list's own fresh permit (P6, section B). Because its scope names no calendar it serves no tool that names
one (DRAFT README section 10). It covers calendars that the owner selects later, and accounts connected later:
the approval step must say so (C.15.2, SR-4).

#### C.4.2 The card for a list that needs approval

Today a list request has no scope (`scope_of` returns `None` for `CalendarList`, CORE `service/requests.rs:120-127`),
so a `need_approval` verdict ends `undecided` (`service/pipeline.rs:170-172` and `:254`, `scoped` false). Changes
(CD21):

1. `scope_of(Action::CalendarList(_))` returns `Some(Scope { calendars: [], window: None, fields: None, recipients:
   None, send_updates: None })`. `RequestResult` keeps `None`: a release that needs approval ends `undecided` with
   no card in version 1 (P6; P2-3).
2. The request digest of a new list request is computed over that scope (`request_digest_of`,
   `service/requests.rs:163-173`). A list request stored before the change keeps its stored digest and scope
   `None`, and still ends `undecided` on `need_approval`. No stored value changes.
3. `open_card` (`service/pipeline.rs:879-905`) needs no change: the card's scope is the request's.
   `proposed_duration` comes from the duration rule (P0): under `ask` it is `one_request`; under `allow` with no grant
   (the list grant revoked) the rule may propose `until_revoked`. The page selects `one_request` first
   (NOV17-DURATION). A one-request approval of a list delivers nothing: the request is cancelled at its fresh permit
   (BD5) and the grant ends `consumed`. Under `list_and_availability` in mode `ask` no list completes in version 1
   (owner OD24, accepted 2026-11-18; BOP1).
4. `decide_request`: `selected` is null or the same empty scope; there is nothing to narrow.
5. `create_grant` (`service/grants.rs:19-82`) takes the purpose from `task_of`, which is `None` for a list. It
   becomes: `task_of(action)`, or `""` when the tool is `fabric_calendar_list`; any other `None` stays an invariant
   fault. The grant goes into `grants` with origin `Approval { request_id, approval_digest }` and tools
   `[fabric_calendar_list]`.

---------------------------------------------------------------------------------------------------------------------

### C.5 The ports (LR-C7)

`CORE ports.rs` (today `ports.rs:271-310`):

```rust
/// What the agent adapter hands to the core.
pub enum AgentInput {
    /// One raw line of a listener connection, without its line feed, at most MAX_FRAME_BYTES.
    Line { connection: ConnectionId, line: Vec<u8> },
    /// A line over MAX_FRAME_BYTES on a listener connection.
    Oversized { connection: ConnectionId },
    /// Refusals the adapter made before any frame reached the core: too_many, timed_out, peer_user.
    Refused { cause: HandshakeFailure, count: u32 },
    /// The connection closed.
    Disconnect { connection: ConnectionId },
    /// The synthetic agent only: a session with no handshake.
    #[cfg(debug_assertions)] Connect { connection: ConnectionId, agent_id: Id },
    /// The synthetic agent only: one decoded frame.
    #[cfg(debug_assertions)] Call { connection: ConnectionId, request: AgentRequest },
}
pub struct AgentReply { pub connection: ConnectionId, pub frame: Frame }   // unchanged
pub enum HandshakeFailure { /* the 14 values of 0.4.6, TooManyKeys among them (C.6.6) */ }
```

`broker.rs` `Output` and the core's `Outbox` gain `pub bound: Vec<ConnectionId>`: the connections whose
`agent_prove` this step answered `approved`. `drain` (CORE `broker.rs:206-213`) moves it. The answers to the
handshake are ordinary `AgentReply` frames: `Frame::AgentResponse(AgentResponse::Hello { id, result })` and
`AgentResponse::Prove { id, result }` (DRAFT CC `agent.rs`), or `AgentResponse::Error`.

`Broker::handle_unavailable` (CORE `broker.rs:344-405`): `Line` and `Oversized` push the connection to
`out.close` and answer nothing; `Refused` and `Disconnect` are dropped; the debug `Connect` closes as today
(`broker.rs:387`) and the debug `Call` answers `internal` as today. The listener binds only while the store is open
(CD15), so this path serves only tests (ct26).

`Broker::start` gains one parameter, `texts: ConfigurationTexts` (C.10).

---------------------------------------------------------------------------------------------------------------------

### C.6 The handshake in the core (LR-C2, LR-S4)

#### C.6.1 The phases of a connection

Memory of the core (`CORE service/core.rs`), all cleared by `quit`:

| Field | Holds |
|---|---|
| `connections: HashMap<ConnectionId, Connection>` | bound connections, as today (`core.rs:56-61`) |
| `handshakes: HashMap<ConnectionId, Challenge>` | connections that sent a hello; at most `MAX_HANDSHAKES` (8) |
| `closing: HashSet<ConnectionId>` | connections the core closed and whose `Disconnect` has not come (CD13) |
| `handshake_windows: Windows<(HandshakeFailure, Option<Id>)>` | the audit windows of failed handshakes |
| `session_starts: HashMap<Id, VecDeque<i64>>` | per agent, the start times of sessions in the last 60 s (CD10) |

`Challenge { nonce: [u8; 32], hello_id: Id, hello: AgentHelloParams, deadline_ms: i64 }`.

`BusyWindows` (CORE `service/limits.rs:113-175`) becomes the generic `Windows<K: Ord + Clone>`; `BusyWindows =
Windows<Id>` keeps its behaviour and its `agent_busy` rows. Every place the core closes a connection pushes it to
`outbox.close` and to `closing`, and calls `Held::forget_connection` (section B).

#### C.6.2 One line, by phase

`AgentInput::Line { connection: c, line }`:

| Phase of `c` | Line | Answer and effect |
|---|---|---|
| in `closing` | any | dropped: no answer, no count |
| bound | decodes as `agent_request` with a tool | `call(c, request)`, as today (CORE `service/agent.rs:173-223`) with C.8's gate |
| bound | decodes as `agent_hello` or `agent_prove` | `not_permitted` (DRAFT test "the handshake is refused even on a connection with a session"); stays open, no count |
| bound | does not decode | the layer's error (`malformed_frame`, `unsupported_protocol`, `unknown_method`, `invalid_params`) with the id and method when they can be read, as the owner channel answers; stays open |
| new | `agent_hello` | `hello(c, id, params)` (C.6.3) |
| new | `agent_request` with a tool, or `agent_prove` | `not_permitted`; close; count `(frame_before_hello, None)` (CD11) |
| new | does not decode | the layer's error; close; count `(frame_before_hello, None)` |
| challenged | `agent_prove` | `prove(c, id, params)` (C.6.4) |
| challenged | anything else | the challenge is removed; `not_permitted` (or the layer's error); close; count `(frame_before_hello, None)` |

`AgentInput::Oversized { connection: c }`: new or challenged, close with no answer and count `(oversized, None)`;
bound, answer `malformed_frame` with id null and stay open; in `closing`, dropped.

`AgentInput::Disconnect { connection: c }`: remove `c` from `closing` and `handshakes`, then today's `disconnect`
(CORE `service/agent.rs:142-171`), which ends the session as `disconnected` when it is still the agent's live one.

`AgentInput::Refused { cause, count }`: add `count` to the window `(cause, None)`.

#### C.6.3 `agent_hello`

1. If `handshakes.len() >= MAX_HANDSHAKES`: close with no answer, count `(too_many, None)`. The binary never lets
   this happen (CD17); it is the second guard.
2. If `params.schema_version` is not `fabric_desktop_contracts::SCHEMA_VERSION`: `unsupported_schema_version` with
   `{received, supported}`; close; count `(unsupported_schema, None)`.
3. `getrandom::fill(&mut [0u8; NONCE_BYTES])` (`getrandom 0.3` is already a dependency, CORE `Cargo.toml`). A
   failure: `internal`; close; no count.
4. Insert `Challenge { nonce, hello_id: id, hello: params, deadline_ms: now + HANDSHAKE_MS }`.
5. Answer `{schema_version: SCHEMA_VERSION, broker_nonce: hex(nonce)}`, 64 lowercase hex digits.

No store read, no write, no audit, whatever the id: a hello tells nobody which ids exist (DRAFT README section 8).

#### C.6.4 `agent_prove`: every decision, in order

1. `let challenge = handshakes.remove(&c)`. The nonce leaves memory before anything is checked, so it is used once.
   A second `agent_prove` on the same connection finds no challenge and is the "new" row of C.6.2.
2. `verify(&challenge, &params.signature) -> Option<Proven>`, a pure function:
   - the text is `handshake_text(challenge.hello.schema_version, &enrollment_id, &public_key, &nonce)` (DRAFT CC
     `handshake.rs:65-78`);
   - the 32 key bytes from the 64 hex digits of `public_key`; `VerifyingKey::from_bytes` failing is a bad signature;
   - the 64 signature bytes from its 128 hex digits; `verifying_key.verify_strict(text.as_bytes(), &signature)`.
   `None`: `not_permitted`; close; count `(bad_signature, None)`. No store read.
3. From here the core holds `Proven { enrollment_id, public_key }`. One read of the store:
   `enrollment(&id)`, `agent(&id)`, `enrollment_by_key(&key)`.
4. Decide, first match wins:

| # | Found | Answer | Effect | Count |
|---|---|---|---|---|
| a | no enrollment, no agent | `not_permitted`, close | none | `(unknown_id, None)` |
| b | no enrollment, an agent (seeded, debug only) | `not_permitted`, close | none | `(unknown_id, Some(id))` |
| c | `removed` | `not_permitted`, close | none | `(removed, Some(id))` |
| d | `approved`, `key` is not the stored key | `not_permitted`, close | none; the stored key stays | `(wrong_key, Some(id))` |
| e | `approved`, the stored key, 6 sessions started in the last 60 s | `not_permitted`, close | none | `(too_many_sessions, Some(id))` |
| f | `approved`, the stored key | `{enrollment: approved}` | commit `agent.connect`: `start_session` (CORE `service/sessions.rs:33-61`), which ends an older live session `replaced`; bind `c`; push `c` to `Output.bound`; record the start in `session_starts` | none |
| g | `pending`, `enrollment_by_key` names another enrollment (any state) | `not_permitted`, close | none: the unique index would turn the write into a store fault and halt (PART-agents 2.3) | `(key_in_use, Some(id))` |
| h | `pending`, `key` is its current key | `{enrollment: pending}`, close | none | none |
| i | `pending`, another key, and the current key was proved less than `KEY_CHANGE_INTERVAL_MS` ago | `not_permitted`, close | none | `(key_change_too_soon, Some(id))` |
| j | `pending`, another key, `e.revision - 2 >= MAX_KEY_CHANGES` | `not_permitted`, close | none | `(too_many_keys, Some(id))` |
| k | `pending`, no key yet or another key | `{enrollment: pending}`, close | commit `agent.prove_key`: `EnrollmentPut` with the key, `key_proven_at_ms = now`, revision + 1; notice `Change::Agent` | none |

Notes on the table:

- Row b: a seeded agent has no key and connects only through the debug `Connect` (V13).
- Row d: nothing is recorded but the count; the owner sees a key mismatch only in the audit (OPC4).
- Row e is P7's limit. The answer is `not_permitted` until P2-17 lets it be `busy` with a wait.
- Rows e and i mean "wait". What the agent is told: the bridge's sentence for that `not_permitted` says to wait a
  minute and call again, beside saying that the owner can look (W6-1, ruled 2026-11-18). With P2-17 (`busy` with its
  wait) the bridge needs no change.
- Row h: the durable time is `key_proven_at_ms`, the first proof of the current key; nothing records later proofs
  (PART-agents 2.3).
- Row j: while an agent is pending, only a key change raises its enrollment revision; `add_agent` writes revision 1
  with no key and the first proved key makes revision 2 (row k), so `e.revision - 2` is the number of keys proved
  after the first. After 20 the owner removes the agent and adds it again.
- Row k commits `agent.prove_key` with actor `Actor::agent(id)`, the enrollment id the signature authenticated
  (AD15, X2; ADR-115:42; the ledger has no foreign key on `actor_agent_id`, `0001_initial.sql:20, 25`); the row does
  not say the agent is approved. Row f commits `agent.connect` with `Actor::agent(id)`, as today (CORE
  `service/agent.rs:130`).
- A paused approved agent gets its session (row f); its calls are blocked by the rule `agent_not_paused` (W06 4.6).
- The prover's readiness does not gate a handshake. It gates request creation only (C.8).

#### C.6.5 The audit of failed handshakes (LR-S4)

Audit kind `handshake_failed`, actor broker, detail `{cause, enrollment_id | null, window_start_ms, window_end_ms,
count}`, one row per `(cause, known id or null)` per `HANDSHAKE_AUDIT_WINDOW_MS` (60 s), as `agent_busy` is
(A.4.2; 0.4.8). An id is recorded only when the store knows it (rows b to k), so a
claimed id never reaches the audit and the number of keys is bounded by the causes times the known ids plus one.
`Core::pump` writes the windows that ended next to `flush_busy` (CORE `service/core.rs:341-353`); `quit` writes all
open ones in its transaction (`service/recovery.rs:103-124`); `next_due_raw_ms` (`core.rs:227-238`) includes their
ends. The debug `Connect`'s `unknown_agent` row per attempt stays for the synthetic agent only.

#### C.6.6 Causes and limits in one place

| `HandshakeFailure` | Where found | Answer |
|---|---|---|
| `frame_before_hello` | core, C.6.2 | `not_permitted` or the layer's error, close |
| `oversized` | core, before the session | close, no answer |
| `unsupported_schema` | core, hello | `unsupported_schema_version`, close |
| `too_many` | binary at accept (8 in a handshake or 32 open), core at hello (8) | close, no answer |
| `timed_out` | binary (read deadline), core (challenge deadline, CD14) | close, no answer |
| `peer_user` | binary at accept | close, no answer |
| `bad_signature`, `unknown_id`, `removed`, `wrong_key`, `key_in_use`, `key_change_too_soon`, `too_many_keys`, `too_many_sessions` | core, prove | `not_permitted`, close |

`too_many_keys` is this section's addition to 0.4.6. Numbers used: `HANDSHAKE_MS` 5_000,
`MAX_HANDSHAKES` 8, `MAX_AGENT_CONNECTIONS` 32, `NONCE_BYTES` 32, `KEY_CHANGE_INTERVAL_MS` 60_000,
`MAX_KEY_CHANGES` 20 (new), `SESSIONS_PER_AGENT_WINDOW` 6, `SESSION_START_WINDOW_MS` 60_000,
`HANDSHAKE_AUDIT_WINDOW_MS` 60_000.

What a handshake costs: at most 8 run at once; each failed prove costs one signature check and one frame on the
main thread; nothing else of it is written but the audit counts. There is no limit per second on accepts; the cap
of 8 and the 5 s deadline bound the work (OPC6).

---------------------------------------------------------------------------------------------------------------------

### C.7 Sessions, pause and closing (LR-C6)

#### C.7.1 Sessions

A session starts only at row f of C.6.4 in a release build. One agent has one live session (the store's
`sessions_one_live`); a second approved connection of the same agent replaces the first, whose connection
`after_commit` closes (CORE `service/core.rs:285-309`). A session ends: at disconnect (`disconnected`), at
replacement (`replaced`), after `SESSION_IDLE_MS` (30 min) with no call (`idle_timeout`), at quit (`app_quit`), at a
restart (`broker_restart`, recovery), and at removal (`agent_removed`, new). A repeated call id after a reconnect
returns the existing request and writes nothing (W04 D25): `UNIQUE(agent_id, call_id)` spans sessions, so a bridge
that sends a call again after its connection broke never makes a second request.

#### C.7.2 Pause

Unchanged: no state in the binary or the bridge, the session stays, the rule `agent_not_paused` blocks every call
(W06 4.6), and pausing voids issued authorizations as today (`VoidReason::AgentPaused`). For a call that names an
event, the core checks the pause before it observes: a paused agent causes no system proof and no Calendar read, and
gets `agent_not_paused` whatever the event's state (D.6.1, Q21).

#### C.7.3 When the core closes a connection

| Event | How the close reaches `Output.close` |
|---|---|
| A handshake fails (C.6.2 to C.6.4), or a `pending` answer | pushed by the handshake code, after the answer |
| The session ends by idle time, replacement or removal | `after_commit` closes every bound connection whose session is no longer the live one (CORE `service/core.rs:294-309`), in the same step |
| Quit | `quit` closes every bound connection after answering its held answers (P7, section B; `service/recovery.rs:120-121`) |
| A challenge passes its deadline | the pump closes it (CD14) |

The binary writes the frames queued before a close first (C.9.6). Removal therefore closes the agent's connection
in the same step as its commit (test ct17, bt13). A disconnect, a replaced or idle session, or a removal drops held
answers without a reply (P7).

---------------------------------------------------------------------------------------------------------------------

### C.8 Requests while the prover is not ready (LR-C3, P8)

`Core::prover_ready(&self) -> bool`: true exactly when a prover is attached and either its status is `Ready` (P0's
`ProverStatus`; on main `prover_health == ProverHealth::Ready`, CORE `service/core.rs:83, 111-114`) or its kind is
`ProverKind::Synthetic` (debug builds only; the classifier answers every job at once and reports its status as
unavailable by design, BR `synthetic.rs:93-99`). The lead ruled this exemption on 2026-11-17 (I.3 B-8): it is by the
prover's kind and exists only in debug builds. In `Core::call` (CORE `service/agent.rs:173-223`), for an action,
after the session check and the activity record:

1. A call id already used by this agent: its request's status, as today (`agent.rs:235-254`). No request is created.
2. `!prover_ready()`: the agent error `prover_unavailable` (retryable, no detail; DRAFT CC `error.rs:355-367`),
   message "The rule checker is not ready. Nothing was recorded. Try again later." (B.10's text, X11). Nothing is
   written.
3. Otherwise the limits and the insert, as today.

`fabric_request_status` is answered at once in every state. A `fabric_request_result` creates a request and is
gated like any action. Requests already open across a restart are not affected; holding their proofs while Isabelle
starts is the broker's planned follow-up (P8).

---------------------------------------------------------------------------------------------------------------------

### C.9 The listener in the binary (LR-B1, LR-B2, LR-B3)

New file `BR listener.rs`, `#[cfg(unix)]`. On other platforms no listener exists and the bridge answers
`broker_not_running` (W06 9).

#### C.9.1 The path and its checks (bind)

`AgentSocket::bind(data_dir: &Path, euid: u32) -> Result<AgentSocket, SocketProblem>`, called from `Shell::route` at
the first call where `broker.store_ready()` is true, the place Isabelle starts (BR `shell.rs:343-361`):

1. `path = data_dir/run/agent.sock`. Its length in bytes must be at most `SUN_PATH_MAX` (103 on macOS, 107 on
   Linux); otherwise `PathTooLong`. The installed path is 85 bytes plus the user's short name (W06 4.1), so a name of
   more than 18 bytes is refused (W06 R7; OPC5). The broker never changes its working directory, so it binds by
   the absolute path.
2. `run/`: absent, created with `DirBuilder::new().mode(0o700)`, not recursive (the data directory exists: the store
   is open). Present, `symlink_metadata` must show a directory, not a link, `mode & 0o777 == 0o700`, and
   `uid() == euid`; otherwise `RunDirUnsafe`. The broker never changes a mode.
3. A file at `agent.sock`: if `symlink_metadata` shows a socket (`FileTypeExt::is_socket`) owned by `euid`, it is a
   stale socket and is removed; the store's lock says no other broker serves this directory (CD15). Anything else:
   `StaleNotSocket`, and nothing is removed.
4. `UnixListener::bind(&path)`; `set_permissions(0o600)`; read the metadata again: a socket, mode `0o600`, owner
   `euid`. Otherwise remove it and `SocketMode`. Between bind and chmod the socket has the umask's mode, inside a
   `run/` that only this user can enter.
5. In a debug build with the test key source, create `data_dir/bridge-keys` with mode 0700 if absent, with the
   same checks as `run/`. It is the bridge's `--key-dir` of the configuration text (C.10).
6. Spawn the accept thread.

A `SocketProblem` (`PathTooLong`, `RunDirUnsafe`, `StaleNotSocket`, `SocketMode`, `BindFailed`) is one fixed stderr line
naming the check and the path; the broker keeps serving the owner channel, and `health` shows it through P2-18 (asked,
section G; W06 LR-B2). A second broker on the same directory gets `Unavailable::InUse`, is never ready, and never binds
or removes anything (bt03). While the store is unavailable there is no listener, and the bridge answers
`broker_not_running` (W06 4.4 expected `internal` there; this form needs no second lock).

At quit, after the last frames (CD19), `Shell::stop` removes `run/agent.sock` if it is still a socket. The store,
and so its lock, is still held: `broker` lives until the end of `main` (BR `main.rs:92-95`).

#### C.9.2 Accept

The accept thread loops over `listener.incoming()`; an error (such as `EMFILE`) sleeps `ACCEPT_RETRY_MS` (100 ms) and
goes on. For each
stream, in this order:

1. The peer's user id (C.9.3) is not `euid`, or cannot be read: drop it, add 1 to the `peer_user` atomic.
2. `open >= MAX_AGENT_CONNECTIONS` or `in_handshake >= MAX_HANDSHAKES`: drop it, add 1 to the `too_many` atomic.
3. Otherwise increment `open` and `in_handshake`, take `connection = next.fetch_add(1)` from an `AtomicU64` that
   starts at `FIRST_LISTENER_CONNECTION` (CD24), and spawn the reader.

The accept thread never sends on the main channel. The main thread reads and zeroes the atomics at the start of
each `turn` and feeds each non-zero count as `Input::Agent(AgentInput::Refused { cause, count })` (CD17).

#### C.9.3 The peer's user id

`fn peer_uid(stream: &UnixStream) -> io::Result<u32>` and `fn own_euid() -> u32` in `listener.rs`, through `nix`
(CD16): on macOS `nix::unistd::getpeereid(stream)`; on Linux `nix::sys::socket::getsockopt(stream,
sockopt::PeerCredentials)?.uid()`; `nix::unistd::geteuid()`. `apps/fabric-broker/Cargo.toml` gains `nix` under
`[target.'cfg(unix)'.dependencies]` with `default-features = false, features = ["user", "socket"]`, at the newest
release when P6 is built, pinned by `Cargo.lock`. It adds `nix` and `cfg_aliases` to the lock (`bitflags` and
`libc` are there already, PART-agents 3.7). The same `euid` checks the owner of `run/`, of a stale socket and of the
bound socket (C.9.1). The check is not deferred: P9 asks for it on every accepted connection.

#### C.9.4 One reader per connection

Each reader thread:

1. Wraps the stream in `Deadline { stream, deadline: accepted + HANDSHAKE_MS, proven: Arc<AtomicBool> }`. Its
   `Read::read`, while `proven` is false, sets `set_read_timeout(deadline - now)` and returns `TimedOut` once the
   deadline has passed. A `TimedOut` that finds `proven` set clears the timeout and reads again (J-auth race 1).
   After `proven` no timeout applies; idle is the core's 30 minutes.
2. Starts the connection's writer thread (C.9.6) and sends `Event::AgentOpened { connection, writer }` with a
   blocking send, before any line, on the same channel.
3. Loops over `LineReader::new(BufReader::new(deadline_stream), MAX_FRAME_BYTES)` (BR `reader.rs`):
   `Line::Frame(b)` sends `Event::AgentLine { connection, line: b }`; `Line::Oversized` sends
   `Event::AgentOversized { connection }`; `End`, `Unterminated` or an error ends the loop. A `TimedOut` while not
   proven adds 1 to the `timed_out` atomic first.
4. On exit: `shutdown(Both)` and a blocking send of `Event::AgentClosed { connection }`.

The reader changes no counter (CD17).

#### C.9.5 How frames enter the single loop, and back-pressure

All agent events go on the main loop's one `sync_channel(QUEUED_EVENTS)` of 4 (BR `shell.rs:47`, `main.rs:89`).
Every reader send blocks when it is full: the reader stops reading its socket, the kernel buffer fills, and the
agent's writes block. Nothing in the broker holds more than 4 events, one line per blocked reader (at most 32) and
each writer's queue. The main thread never sends on that channel (BR `shell.rs:17-19`).

`shell::input_of` (BR `shell.rs:236-245`) maps: `AgentLine` to `Input::Agent(Line)`, `AgentOversized` to
`Input::Agent(Oversized)`, `AgentClosed` to `Input::Agent(Disconnect)` after the main thread removed the writer and
lowered `open` (and `in_handshake` when the connection was still in it). `AgentOpened` inserts the writer into
`Shell.connections: HashMap<ConnectionId, AgentWriter>` and makes no input.

Owner lines and agent lines share the queue; a flooding agent can delay an owner line by the events ahead of it
(risk, OPC7; test bt12 bounds it).

#### C.9.6 The writer queue (P7)

`AgentWriter { sender: SyncSender<WriterMsg>, queued_bytes: Arc<AtomicUsize>, stream: UnixStream, proven:
Arc<AtomicBool>, handshaking: bool }`, `WriterMsg = Frame(String) | Close`. The writer thread owns a `try_clone` of the
stream with `set_write_timeout` of `AGENT_WRITE_TIMEOUT_MS` (5 s), writes each frame and subtracts its length from
`queued_bytes`; on a write error, or on `Close` after the frames before it, it calls `shutdown(Both)` and ends.

In `Shell::emit` (BR `shell.rs:321-341`), which today writes only `owner` frames, after the synthetic agent took
`ConnectionId(1)`'s replies:

| Output | Main thread |
|---|---|
| each `AgentReply` | if `queued_bytes + len > WRITER_QUEUE_BYTES` or `try_send(Frame)` is full or disconnected: `shutdown(Both)`, remove the writer, push the connection on a worklist; else add `len` to `queued_bytes` |
| each id in `bound` | `proven.store(true)`; lower `in_handshake` once |
| each id in `close` | `try_send(Close)` so the answer queued before it is written first; if that fails, `shutdown(Both)`; remove the writer |

`Shell::emit` handles `output.agent`, `output.bound` and `output.close` after its `output.halt` test (BR
`shell.rs:335-338`): a halting step writes no agent frame. Calls after the halt get nothing (CORE
`broker.rs:300-307`).

The rows run in this order, so `Output.agent` is written before `Output.close` (B.8.4 relies on it at quit).
`turn` then feeds each worklist id straight into `broker.handle(Input::Agent(Disconnect { connection }))` and emits
its output, until the worklist is empty; it never goes through the channel. The main thread only calls `try_send`
and `shutdown`, so it never blocks on a peer. The queue holds `WRITER_QUEUE_FRAMES` (32 =
`MAX_HELD_PER_CONNECTION + 16`) frames and `WRITER_QUEUE_BYTES` (4 MiB) (0.6). A release whose answer is
dropped by a full queue was recorded but not received (OP13): "a release row means the result left the core".

At quit (CD19) `Shell::quit` emits the core's last answers and closes, then `Shell::stop` joins each writer thread
for at most `QUIT_WRITE_BOUND_MS` (500 ms) in all, then removes the socket file.

#### C.9.7 Handshake limits in the binary

| Limit | Value | Where |
|---|---|---|
| Time from accept to `approved` | `HANDSHAKE_MS` 5 s, a total, not per read | reader's `Deadline` (and the core's challenge deadline, CD14) |
| Connections in a handshake | `MAX_HANDSHAKES` 8 | accept thread, counted by the main thread (CD17); the core again at hello |
| Open connections | `MAX_AGENT_CONNECTIONS` 32 (OP9) | accept thread |
| Line size | `MAX_FRAME_BYTES` 1 MiB | `LineReader` |

---------------------------------------------------------------------------------------------------------------------

### C.10 The configuration text (LR-B5)

The owner method `get_agent_configuration` needs a text that names the bridge's path; the core knows no path.

- `CORE ports.rs`: `pub type ConfigurationTexts = Box<dyn Fn(&Id) -> Option<AgentConfiguration> + Send>;`
  (`AgentConfiguration` is DRAFT's, CC `owner.rs`). `Broker::start` takes it and keeps it in `Broker`, beside the
  opener; the owner channel calls it for `get_agent_configuration` after the agent check of C.3.4.
- `BR shell.rs`: `pub fn configuration_texts(config: &Config) -> ConfigurationTexts` builds the closure once at
  start. It finds the bridge beside the broker: `std::env::current_exe()`'s parent joined with
  `fabric-mcp-bridge`, which must be a file; otherwise the closure returns `None` for every id. For each id it calls
  `fabric_mcp_bridge::config_text::config_text(client, bridge_path, id, debug_extras, &rule)` for the three clients
  (W06 6; bridge branch `apps/fabric-mcp-bridge/src/config_text.rs:196-202`); any `Err` makes the whole answer
  `None` (DRAFT choice 63: all three texts or null).
- `apps/fabric-broker/Cargo.toml` gains `fabric-mcp-bridge = { path = "../fabric-mcp-bridge" }`. It brings no new
  package to the lock beyond the bridge's own (`ed25519-dalek`, `getrandom`, `sha2`, `zeroize`, and `rustix` on unix,
  bridge branch `Cargo.toml`). No cycle: the bridge depends on the contract crate only.
- Release build: `rule = PathRule { debug_build: false, home: None }`, `debug_extras = None`. The broker's environment
  has no `HOME` (the relay passes only `PATH` and the six `FABRIC_BROKER_*` names, `apps/fabric-desktop/relay/src/
  config.rs:40-79`), so only a bridge under `/Applications/` gets a text; one under `$HOME/Applications/` gets null
  (OPC3).
- Debug build: `rule = PathRule { debug_build: true, home: None }`: any absolute path, and the text starts with the
  bridge's test line. With the test key source it also passes `DebugPaths { socket: data_dir/run/agent.sock,
  key_dir: data_dir/bridge-keys }`, so a client started from the text reaches this scratch broker with a key in a
  file (W06 A23); with the Keychain key source it passes none.
- LR-B4: the relay's allowlist needs no new name for the socket, which is under `FABRIC_BROKER_DATA_DIR`.

---------------------------------------------------------------------------------------------------------------------

### C.11 What a release build must not contain

| Must not contain | How it is kept out |
|---|---|
| `AgentInput::Connect` and `AgentInput::Call` (a session or a call with no handshake) | `#[cfg(debug_assertions)]` on both variants and on `Core::connect` and the decoded `Call` arm (CD12) |
| `Broker::seed`, the synthetic agent, the practice calendar, the test key | as today (W04 D11; CORE `broker.rs:87-107, 451-477`; BR `lib.rs:38-44`) |
| `DebugPaths` in a configuration text | `configuration_texts` passes them only under `cfg(debug_assertions)` and the test key source; `config_text` refuses them when `debug_build` is false (bridge `config_text.rs:204-207`) |

The test, `ct30`, is in a new file CORE `service/release_tests.rs`, declared in `service/mod.rs` as
`#[cfg(all(test, not(debug_assertions)))] mod release_tests;`, because `service/tests.rs` is compiled only with
`debug_assertions` (CORE `service/mod.rs:27-28`). `scripts/verify-desktop.sh` gains one line in its release step,
after line 115: `sh "$RUN" cargo test --release --locked -p fabric-broker-core --lib`. Today that step tests only
`-p fabric-broker --lib`. The builder gates on `debug_assertions` any other test module of the core that does not
compile in release:

```rust
#[cfg(not(debug_assertions))]
#[test]
fn a_release_core_reaches_a_session_only_through_the_handshake() {
    // Compile-time half: this match is exhaustive only if Connect and Call do not exist.
    fn inputs(input: AgentInput) { match input {
        AgentInput::Line { .. } | AgentInput::Oversized { .. }
        | AgentInput::Refused { .. } | AgentInput::Disconnect { .. } => {} } }
    let _ = inputs;
    // Run-time half: an approved agent's tool frame on a fresh connection, with no handshake,
    // gets not_permitted and a close; the ledger has no agent.connect row.
}
```

The run-time half drives a core on a test-key store in a scratch directory through `add_agent`, a prove with a fixed
test seed and `approve_agent`, then sends a tool frame on a new connection.

---------------------------------------------------------------------------------------------------------------------

### C.12 What this section amends

Each amendment is Proposed and goes to the owner with the design (P13), as E.3 OD19.

| Decision | Today | New wording |
|---|---|---|
| W04 D25 "Agent entry" | `AgentInput::{Connect, Call(AgentRequest), Disconnect}` | "`AgentInput::{Line, Oversized, Refused, Disconnect}`; `Connect` and `Call` in debug builds only, for the synthetic agent. `UNIQUE(agent_id, call_id)` as before: a repeated call returns its request and writes nothing, across sessions" |
| W04 D11 "Developer seams" | key source `test`, the synthetic agent, `Broker::seed`, `FABRIC_DESKTOP_DATA_DIR` | add: "`AgentInput::Connect` and `AgentInput::Call`, and the configuration text's debug paths" |
| W04 D28 "Locations and names" | `<data_dir>/prover` reserved | add: "`<data_dir>/run` (0700) holds `agent.sock` (0600); in a debug build with the test key, `<data_dir>/bridge-keys` (0700) holds the bridge's test keys" |
| W04 L2 "no owner-uid check, mode checks kept on Unix" | no owner check | "no owner-uid check on the data directory; the agent socket checks the peer's user id on every connection and the owner of `run/` and of the socket" |
| W04 5.5 Sessions | a connect of an enrolled agent opens a session | "a successful `agent_prove` of an approved agent opens a session (C.6.4 row f), at most 6 per agent per minute; a session also ends `agent_removed`" |
| W04 6.1 "The 13 methods" | 13 owner methods | the 20 of DRAFT; this section implements `add_agent`, `approve_agent`, `remove_agent`, `get_agent_configuration` |
| W04 D33 (a call's answer holds no result) | replaced by P6 (section B) | no further change here |

---------------------------------------------------------------------------------------------------------------------

### C.13 Answers to the lead requests

| Id | Asked (W06 7.1, 7.2) | Answer | Where |
|---|---|---|---|
| LR-S4 | Audit of failed handshakes, one row per minute | `handshake_failed`, one row per (cause, known id or null) per 60 s; no migration | C.6.5 |
| LR-C1 | Enrollment: add, approve with code and revision giving the list permission, remove; a proved key becomes the pending agent's key; readers of a grant's origin | `add_agent`, `approve_agent`, `remove_agent` as DRAFT shapes them, with the revision of CD2, the list grant of C.4.1 and P4's order; the key effect is C.6.4 rows g to k; grant readers are section A's G1 to G3 and G6 | C.2 to C.4, C.6.4 |
| LR-C2 | Handshake decisions after the signature verifies; key equal to the approved key; nonce used once; session only after them | All in `service/handshake.rs`; nonce removed before the check; `verify_strict`; `Proven` gates the store read; session only at row f | C.6 |
| LR-C3 | `prover_unavailable` while the prover is not ready | Refused at once, nothing written; status always answered | C.8 |
| LR-C6 | Close a connection when its agent is removed | `after_commit` closes it in the removal's step; held answers dropped | C.3.3, C.7.3 |
| LR-C7 | Ports for the handshake | `AgentInput::Line`, `Oversized`, `Refused`; `Output.bound`; answers are `AgentReply` frames | C.5 |
| LR-B1 | Listener: bind, accept, reader per connection, handshake first, frames in and out, 5 s, 8 at once | C.9.1 to C.9.7 | C.9 |
| LR-B2 | Path, modes, length, stale socket only when no other broker serves the directory | `data_dir/run/agent.sock`, 0700 and 0600, length check, stale socket removed only under the store lock | C.9.1 |
| LR-B3 | Peer user id on every connection; the crate | `nix` (`getpeereid`, `PeerCredentials`), at accept, not deferred | C.9.3 |
| LR-B4 | No new name in the relay's allowlist | Confirmed for the socket; `HOME` would be needed for `$HOME/Applications` (OPC3) | C.10 |
| LR-B5 | The configuration text: who calls `config_text`, the dependency, debug extras | `ConfigurationTexts` closure from the binary; the binary depends on the bridge's library; debug test line and paths | C.10 |

---------------------------------------------------------------------------------------------------------------------

### C.14 Tests

All tests use the test key in scratch directories (`testing::TempDir`, CORE `service/core.rs:425-450`); none touches
the Keychain. Keys are fixed test seeds (DRAFT `fixtures/handshake.json` and the RFC 8032 test key, DRAFT CC
`handshake.rs:100-106`). Core tests feed `Input`s in process; binary tests start the real binary on Unix in debug.

#### C.14.1 Core (`CORE service/tests.rs`, `owner/tests.rs`)

| Id | Shows | Catches |
|---|---|---|
| ct01 | `add_agent` gives a pending agent at revision 1, one `owner.add_agent` row, one agent notice, the destination of C.3.1 | wrong type, missing notice, wrong destination |
| ct02 | `agent_hello` for an unknown, a pending, an approved and a removed id: answers of one shape, a fresh 64-hex nonce each; ledger head and audit unchanged | a store read or a decision at hello |
| ct03 | hello with schema version 2: `unsupported_schema_version` with `{received: 2, supported: 3}`, close, count `unsupported_schema` | a missing version check |
| ct04 | a one-bit-flipped signature, and a non-canonical one that `verify` accepts and `verify_strict` refuses: `not_permitted`, close, no session, no row, count `(bad_signature, null)` | skipped or weak verification |
| ct05 | a signature recorded on connection A, replayed on B with its own nonce: `not_permitted` | a nonce outside the signed text, or reused |
| ct06 | a second `agent_prove` on the same connection: `not_permitted`, close, count `frame_before_hello` | a nonce kept after use |
| ct07 | pending, key K proved: one `agent.prove_key` row, the key, `key_proven_at`, `pending`, close, an agent notice with the pairing code | missing write, close or notice |
| ct08 | the same key again: `pending`, ledger head unchanged | a row per tool call |
| ct09 | another key within 60 s: `key_change_too_soon`, key unchanged; after 60 s on the core clock it replaces K and the revision rises; after 20 changes `too_many_keys` | missing rate or cap |
| ct10 | a key held by another enrollment, pending or removed: `key_in_use`, `Output.halt` is none | a unique violation halting the broker |
| ct11 | `approve_agent` with a wrong code (`invalid_params` at `/params/pairing_code`), no key (same), a stale revision (`conflict`); the right code: `agents` row with the id, approved enrollment, one list grant of C.4.1, notices agent then grant, result with the grant, one `owner.approve_agent` row | approving an unseen key, wrong order |
| ct12 | approved agent, right key: `approved`, `Output.bound == [c]`, a session; a following tool line goes through `call` | a session without a handshake |
| ct13 | approved agent, another valid key: `wrong_key`, the stored key unchanged | rewriting the key after approval |
| ct14 | 100 failed proofs for unknown ids in 60 s: one audit row `{unknown_id, null, count: 100}` at the window's end; claimed ids appear nowhere | a row per attempt, attacker text in the audit |
| ct15 | a 9th hello while 8 are challenged: close, `too_many` | the core's cap |
| ct16 | 100 good handshakes of one agent in 60 s: at most 6 `agent.connect` rows, the rest `too_many_sessions` | an unbounded connect loop |
| ct17 | `remove_agent` in one commit: session ended `agent_removed` and its connection in the same `Output.close`; session, until-revoked, read and list grants ended `agent_removed`; a queued request's authorization voided `agent_removed` and the request cancelled; a pending_approval request cancelled; a dispatching request untouched; held results purged; `revocation_epoch` raised; the agent still in the snapshot as `removed`; a later prove gets `removed`; then a dispatching change answered `Transient` and a dispatching read answered `Done` end `cancelled` and open no card, and G4 is never reached; `decide_request` on a card of a removed agent answers `stale_request` | any step left out; a card for a removed agent (X22) |
| ct18 | a `ProofDone`, an `AttemptDone`, a `SystemProofDone`, an `ObserveDone` and a `ReconcileDone` for a request cancelled by removal, and a `SystemProofDone` and a `ReconcileDone` for a request that `disconnect_google` moved to `needs_owner_resolution`: no row, no order, no halt | a late answer moving a request that left the state it served (B.11.3, D.5) |
| ct19 | removing a pending agent: `revocation_epoch` raised once, record removed, no session or grant touched | CD5 |
| ct20 | the contract revision never falls across add, prove, approve, pause, session start and end, and remove | CD2 |
| ct21 | `pause_agent(AllAgents)` with pending, approved and removed agents changes only the approved one; pausing a removed agent gets `conflict` | stale readers of `agents()` |
| ct22 | a tool frame before hello: `not_permitted`, close, `frame_before_hello`; an undecodable line before hello: the layer's error and close | V12 |
| ct23 | oversized during the handshake: close and count; on a bound connection: `malformed_frame`, stays open | |
| ct24 | a line on a connection the core closed, before its `Disconnect`: dropped, no count | CD13 |
| ct25 | a challenge past 5 s on the core clock: closed at the next pump, `timed_out` | CD14 |
| ct26 | unavailable mode: `Line` and `Oversized` close, `Refused` dropped, nothing answered | |
| ct27 | prover `Starting`: a new action gets `prover_unavailable` and no row; a repeated call id gets its status; `fabric_request_status` answered | P8 |
| ct28 | a list request after the list grant was revoked, under `allow`: a card whose scope names no calendar; approving it makes a grant with purpose `""` in `grants`; the broker does not halt (step P5b) | CD21 |
| ct29 | the list grant of an approved agent permits a list under Careful and Flexible; it serves no tool that names a calendar | P5 |
| ct30 | release only: C.11 | a handshake-free session in release |
| ct31 | debug `Connect` for a removed or pending id: closed; for a bound `ConnectionId`: the old binding is closed first | J-auth removed-agent defect |
| ct32 | `get_agent_configuration`: texts for pending and approved, `not_found` for removed and unknown, null when the closure returns none; no row | C.3.4 |

#### C.14.2 Binary (`apps/fabric-broker/tests/agent_socket.rs`, unix, debug, test key, scratch data directory)

| Id | Shows | Catches |
|---|---|---|
| bt01 | after start `run/` is 0700 and `run/agent.sock` a 0600 socket of this user; after a clean quit the socket is gone | modes, leftovers |
| bt02 | a broker killed with SIGKILL leaves the socket; the next broker removes it and binds; a client completes a handshake | stale socket kept |
| bt03 | a second broker on the same directory neither binds nor removes; the first broker's client still connects | removal without the lock |
| bt04 | a regular file at `run/agent.sock`: left as is, one stderr line, owner `hello` answered | removing a non-socket |
| bt05 | `run/` at 0755: refused, mode unchanged | widening or fixing modes |
| bt06 | a socket path over `SUN_PATH_MAX`: no bind, one stderr line, owner channel works | a bind error that halts |
| bt07 | a silent client and a client that drips a byte a second: both closed at 5 s (1 s margin) | per-read timeout |
| bt08 | 8 silent connections: the 9th closed at once; after one closes, a new one is accepted | a leaking counter |
| bt09 | a client enrolls end to end over the owner and agent channels (`add_agent`, prove: `pending`, `approve_agent` with the code, prove: `approved`, `fabric_request_status` answered) | `Output.agent` dropped by `emit` |
| bt10 | two agents on two connections each get only their own replies; closing one leaves the other working | id mix-up |
| bt11 | a client that never reads while flooding status calls: its writer fills, it is closed, owner `hello` answered within 1 s | the main thread blocking |
| bt12 | an agent flooding lines while owner `hello`s are sent: every owner `hello` answered within 1 s | starvation (OPC7) |
| bt13 | the owner removes an agent: its socket reads EOF in the same turn | LR-C6 at the socket |
| bt14 | 15 held answers settle in one step: all arrive, the connection stays open | the writer queue bound |
| bt15 | quit with held answers: each arrives before the process exits | CD19 |
| bt16 | `peer_uid` of one end of `UnixStream::pair()` equals `own_euid()` (a peer of another user needs root; stated, not faked) | reading the wrong id |
| bt17 | `get_agent_configuration` from a debug broker beside a built bridge: three texts with the test line and the scratch socket and key paths | C.10 |
| bt18 (CI) | the Windows build compiles without `listener.rs` and without `nix` | unix code in shared paths |

---------------------------------------------------------------------------------------------------------------------

### C.15 Requests to other sessions

#### C.15.1 To the contract session (against DRAFT at `29cdfdf`)

| # | Request |
|---|---|
| P2-15 | As section G (the list grant's shape and ends, in README section 10) |
| P2-16 | As section G (close after every `agent_prove` that does not answer `approved`, `pending` included, and every refused `agent_hello`) |
| P2-17 | Optional, as section G (`sessions_per_minute_per_agent` so C.6.4 row e can answer `busy`) |
| P2-18 | Asked, as section G (`health.agent_channel`), so that a failed path check is visible in `health` (W06 LR-B2); add `run_dir_owner` to its problems if the owner check of C.9.1 is reported apart |
| P2-21 (new) | `owner#/$defs/result_approve_agent` gains `grant: grant` (the list grant made in the same commit); CC `ApproveAgentResult` gains `pub grant: Grant`. P5: "approve_agent returns the grant" |
| P2-22 (new) | A data table of the destination of each client of `add_agent` (`data/agent-clients.json`, or a README table): `claude_desktop` and `claude_code` `{kind: cloud, processor: "Anthropic"}`, `codex_desktop` `{kind: cloud, processor: "OpenAI"}`. Until it exists the core's table of C.3.1 is the rule |
| P2-23 (new) | README section 15: the handshake limits are now the broker's (5 s, 8 at once, 6 sessions per agent per minute, one key change per minute and 20 in all while pending) and what it records of a failed one (one audit row per minute per cause and known id); remove that line from "Not covered" |
| P2-28 (new) | README section 7 states seven answers the core gives: `conflict` for approving an agent that is not pending and for removing one that is removed; `conflict` for pausing a pending or removed agent; `invalid_params` at `/params` for the owner budget at approval; `not_found` for a removed agent's configuration; `invalid_params` at `/params/registered_name` when `add_agent` would pass the owner budget; `invalid_params` at `/params/account_id` when `connect_google` would; and "A proved key that differs from the agent's current key raises the agent's revision; proving the current key again changes nothing" in place of "Every proved key raises the agent's revision" |

#### C.15.2 To the screens session: the Agents screen (S07)

| # | Request |
|---|---|
| SR-1 | Add an agent: the client (Claude Desktop, Claude Code, Codex desktop) and a name; then `get_agent_configuration` and the text for that client with a copy control and W06 6's words for it. When `configuration` is null, say that Fabric Calendar cannot find its bridge and must be in `/Applications` |
| SR-2 | A pending agent: "Waiting for the client to connect" while `pairing_code` is null; then the code in three groups, `key_proven_at`, and Approve and Remove. Tell the owner to compare the code with the one the agent shows, or with `fabric-mcp-bridge code --enrollment <id>` in a terminal. When the code changes while the screen is open, show the new code and require a new look before Approve |
| SR-3 | `approve_agent` answers: `conflict` means the agent changed (a new key): reload and compare again; `invalid_params` at `/params/pairing_code` means the code is not the current key's |
| SR-4 | The approval step must tell the owner, before he approves, in plain words (P5): (a) the permission: "This agent may list your calendars at any time, without asking you each time"; (b) the destination, as a label the owner gave, not a checked fact: "You said this agent is {client}. {client} normally sends what it reads to {processor}. Fabric Calendar cannot check which program uses this agent's key or where that program sends data." (from `destination`; OPC9); (c) that it covers calendars and accounts selected later: "This includes calendars you select or accounts you connect later"; (d) what a calendar list holds: for each selected calendar of each connected account, the account, the calendar's id and name, whether the owner can write to it, and whether it is the Agent Drafts calendar; no events. (e) That he can revoke this permission alone under the agent's permissions, and that the agent then asks him the next time |
| SR-5 | An approved agent: name, destination notice, connected or not, last tool call, Pause and Resume, Remove, and its permissions with the list permission first ("List your calendars, until you revoke it"). "For this session" is worded "until this client disconnects" (W06 4.5) |
| SR-6 | A removed agent stays in the list, marked removed, with its history; its requests still open (`dispatching`, `outcome_unknown`, `reconciling`, `applied`, `needs_owner_resolution`) are shown under its name. Remove asks for confirmation and says that requests already handed to Google run to their end |
| SR-7 | A card for a calendar list: its scope names no calendar; show "List your calendars (names only, no events)" and the duration choices as for any card. Say that approving for one request lets nothing through (the list is cancelled, BD5) and that "until revoked" restores listing (OD24) |
| SR-8 | Google's credentials stay with Fabric Calendar; the agent never receives them (PLAN S07) |

#### C.15.3 To the CD-W06 bridge session

- Treat a connection closed with no answer during the handshake (`too_many`, `timed_out`, `peer_user`) as
  `broker_not_running` for that call and try again at the next call.
- `pending` is answered for the same key with nothing written. `not_permitted` from rows e and i of C.6.4 (too many
  sessions, a key change within a minute) means "wait": the bridge's sentence says to wait a minute and call again,
  beside saying that the owner can look (ruled 2026-11-18). With P2-17 (`busy` with its wait) the bridge needs no
  change.
- Keep `config_text` free of platform code and public in the library, with the signature of C.10; the broker binary
  depends on it.
- Expect connection ids and frames as C.6; the handshake types are DRAFT's (`handshake.rs`).

---------------------------------------------------------------------------------------------------------------------

### C.16 Open points and residuals

| # | Point | Recommendation | Owner |
|---|---|---|---|
| OPC1 | `nix` is a new dependency of the broker binary (with `cfg_aliases`) | Accept, unix targets, features `user` and `socket` only | owner (dependency review, OD21) |
| OPC2 | `fabric-mcp-bridge` becomes a dependency of the broker binary for `config_text` | Accept; if its weight matters, the bridge session moves `config_text` into a small crate of its own. Ruled 2026-11-17: accepted; the smaller crate only if the weight matters; OD21 stays open | lead with the CD-W06 session; owner for the dependency (OD21) |
| OPC3 | No `HOME` in the broker's environment, so a bridge under `$HOME/Applications/` gets no text | Keep: the installer puts the app in `/Applications`; adding `HOME` to the relay's allowlist is the alternative | owner (OD22) |
| OPC4 | Repeated `wrong_key` or `bad_signature` against an approved agent are visible only in the audit | Accept for version 1; a later health or agent member can show it | owner (OD22), with the screens session |
| OPC5 | A user short name over 18 bytes makes the installed socket path too long; no agent can connect for that user | Accept with the stderr line and P2-18; bind by a relative name after `chdir` only if a real user needs it. Ruled 2026-11-17: accepted. Ruled 2026-11-18: still accepted for version 1 and told to the owner in OD22; the broker binding by the relative name is decided at CD-W11 | lead; told to the owner (OD22) |
| OPC6 | No limit on accepts per second; each failed prove costs a signature check on the main thread | Accept: 8 at once and 5 s bound it. Ruled 2026-11-17: accepted | lead |
| OPC7 | Owner and agent lines share the queue of 4; a flooding agent can delay an owner line | Accept, with bt12 as the bound; a separate owner channel is the fix if bt12 fails. Ruled 2026-11-17: accepted | lead |
| OPC8 | The pairing code is 48 bits. A key whose code matches the real bridge's code, proved before the owner looks, would be approved; the real bridge then fails with `wrong_key` | Residual of DRAFT choice 62; the revision check stops a swap after the owner looked | owner (OD22; contract) |
| OPC9 | Same-user software that runs the bundled bridge with the agent's id has its key and passes every check (W06 T3) | Residual of PLAN 4.3 | owner (OD22) |
| OPC10 | `MAX_KEY_CHANGES` (20) lets a process that knows a pending id force the owner to remove and add the agent again | Accept: the alternative is unbounded ledger growth; only the owner creates ids, and the id is in the client's configuration only | owner (OD22) |

---------------------------------------------------------------------------------------------------------------------

## D. The provider port and Google in the core

**What this section is.** The provider port after the review of PO1 to PO8 (ruling P10), and what the core gains so
that Google can be attached: accounts and calendars, the observation before a proof, reconciliation out of
`outcome_unknown`, retries, recovery and the account budget. It also says what the practice provider must answer to
stay a true implementation of the port. Names and numbers are section 0's, cited by number;
where this section adds or corrects one, D.16 lists it.

**Sources and how they are cited.** Code at main `2d41210`, paths from the repository root. `W05` is the CD-W05 design
(`origin/claude/cd-w05-google-provider-27ca35`), `DRAFT` the contract draft of schema version 3 (`29cdfdf`),
`REV-port` and `REV-core` the reviews in `w05/review.json` by item id, `W04` and `PW` the designs in `docs/plans/`.
`CORE` is `crates/fabric-broker-core/src/`, `BR` is `apps/fabric-broker/src/`, `CC` is
`crates/fabric-desktop-contracts/src/`, `THEORY` is `isabelle/Fabric_Broker/Fabric_Broker.thy`.

**What it depends on.** The tables `observations`, `observation_evidence`, `system_proofs`, `read_attempts` and the
columns `effects.event_id_after`, `revision_after`, `notification` of migration 2 (A.4, V1); `provider_accounts`
and `provider_calendars` of migration 3 (A.5). System proofs as a job of their own (`Job::ProveSystem`, 0.5.1,
V8; section B). P0's `undecide()` with a stored cause. The DRAFT's `select_calendars`, `resolve_outcome` and the
states `resolved_as_applied`, `resolved_as_absent`.

---------------------------------------------------------------------------------------------------------------------

### D.0 Decisions

| Id | Decision | Why |
|---|---|---|
| DD1 | Every provider order that names a calendar carries one `CalendarTarget` (a read order a list of them), resolved by the core from the store in the commit that sends the order, never at authorize; a list order names its account only | REV-port PO1; P10. The calendar map may change between authorize and dispatch |
| DD2 | Every order carries a non-optional `OrderProof`. The worker refuses an order whose `action_digest` is not the digest of the order's own action or system action | REV-port PO8; W05 PD29. The real guarantee stays the core's: commit first, job after (CORE `service/pipeline.rs:581-634`) |
| DD3 | Every order has an `order_id` unique in the run and a deadline of `PROVIDER_CALL_MS` from the sending commit. The core's watchdog ends an order past `deadline_ms + ORDER_WATCHDOG_MARGIN_MS`: a dispatch as `NoAnswer`, every read kind as a failed read | P10; REV-port cross-cutting "no watchdog on a provider call" (CORE `service/pipeline.rs:567-577`, BR `shell.rs:375-377`) |
| DD4 | Before every proof on an action that names an event, the core observes the event under a system proof on `System_refresh`, stores the observation as a broker record, and starts the proof in the commit that stores it. A fresh-permit proof started again (a discard, a void, a retry) observes again; a classification proof started again uses the stored observation (D.6.1) | P10; REV-port PO2 answer 1; W04 10.3 ("broker records only, read in one transaction") |
| DD5 | The fence is computed by `fence_now(counters, observed_revision)`, where `observed_revision` is the revision of the request's latest stored `observe` observation that found the event. `epochs::fence(counters)` goes. `fence_holds` stays plain equality, now over two values built the same way | P10 "the fence compares epochs and the stored revision". Today's equality against `event_revision: None` (CORE `service/epochs.rs:19-31`) would discard every observed proof (CORE `service/pipeline.rs:236`) and void every handoff (`:471`) |
| DD6 | `Transient` means it certainly did not happen. `NoAnswer`, a 5xx on a change, an `Applied` that does not fit the order and an unconfirmed delete all move the request to `outcome_unknown`; a change in doubt is never sent again. A fitting `Applied` of a create, update, invitation or response, confirmed or not, means it happened: an unconfirmed one follows V23 and is never sent again or read back. A delete never follows V23 | P10; W05 PD17, 6.10; V9, V23; ruled 2026-11-18 by the lead |
| DD7 | The answer of a reconcile order reports what the provider holds (`ReadBack`). The core owns the table of what follows (D.7.3), on lifecycle edges that exist. After an unknown outcome a change counts as absent only after two successful reads at least 60 s apart. One exception: a delete whose read-back finds the event at another revision is absent at once, because its `If-Match` can never match again (W05 PD14, W05 section 7 table; D.7.3; I.5 D-10) | P10; REV-port PO4; REV-core rule PD20 |
| DD8 | No read after an applied change. A plainly confirmed `Applied` moves `dispatching -> applied -> reconciled` in the attempt's commit. A delete's `confirmed` comes from the adapter's one GET inside the delete's own dispatch order (D.2); that GET is not a read after `applied` | V7; REV-core CO8 c; THEORY 206-207 ("not yet seen applied"); the delete's GET ruled 2026-11-18 by the lead (W5-6 agreed) |
| DD9 | `connect_google`, `disconnect_google`, `select_calendars`, a calendar list that changes an account, a sign-in outcome and an account signal each raise `revocation_epoch` once in their commit when the account record changes. One test pins this for every command and commit in `NARROWING_COMMANDS` and `NARROWING_COMMITS` (D.9.4) | P10; W04 5.6 ("changes an account's state or calendars") |
| DD10 | `save_preferences` joins the rule when it changes `warning_calendars`: it raises `revocation_epoch` | Warning calendars are proof facts (CORE `service/facts.rs:160`) and widen `System_refresh` (THEORY 520-521); today the command raises nothing (CORE `owner/preferences.rs:15-54`) |
| DD11 | Every account counts into the owner budget at a fixed reserve, `ACCOUNT_RESERVE_BYTES`. A `connect_google` that would add an account past `OWNER_STATE_BUDGET_BYTES` is refused `invalid_params` and stores nothing. A calendar list whose account record would pass the reserve fails as a whole | P10; W04 D38, R24; W05 ST10, OQ14 |
| DD12 | "Fabric created this event" is read from the store (`ReadTx::created_event`, and `created_events` for a read order, 0.5.2): a create effect on that account and calendar whose reserved id is the event id, and whose request is in `applied`, `reconciled`, `resolved_as_applied`, `absence_confirmed` or `resolved_as_absent` | W05 PD19, ST6; OP12 (keep the reserved id of an absent create on record). The residual risk goes to the owner (D.14) |
| DD13 | Google is attached to a run only when the core's reconciliation (D.7) is merged and tested against the practice provider and FAKE. Until then `can_carry_out` is false for every Google account | P10; REV-port "Condition 1" of rule PD17 |
| DD14 | The practice provider implements the whole port (dispatch, observe, read, reconcile, list) with the answers of D.12 before a real agent uses it | P11; REV-port rule PD17 "Condition 2" |
| DD15 | An account signal (`AuthLost`, `AccessChanged`) is applied in the commit of the result that carries it. When that result is ignored (a late or unknown order), an `AuthLost` is still applied, in a commit of its own (`provider.auth_lost`) | REV-port PO6 answer 3. The adapter has already deleted the token (W05 3.8), so the account must not stay `connected` |
| DD16 | The broker's own reads are proven (W05 OQ9 default): observe, reconcile and list each run under an accepted Isabelle system proof. The list waits for TH1 (P2-6). A dispatch order's own GET of its target event, before an invitation or a response or after a 2xx delete, is not a broker read and needs no system proof (D.4) | W05 PD29; REV-port PO5; P13; the GET ruled 2026-11-18 by the lead |
| DD17 | A read answer's span is typed: `ReadSpan::Timed` with start and end in milliseconds from Google's `dateTime`, or `ReadSpan::AllDay` with Google's two dates exactly. The adapter asks Google for `start(date,dateTime)` and `end(date,dateTime)` of every event whatever the grant, uses them only for the span, builds it from the occurrence's own start and end (never `originalStartTime`), and neither widens nor does zone arithmetic. The core alone widens an all-day span by `ZONE_SLACK_MS` (14 h) each way for its overlap check (B.3.4 step 2.1); that bound is never stored or released. Exact bounds wait for a zone database, CD-W09's (I.5 D-18) | W5-2. The span is needed most when `time` is not granted, which is when a title could leave without its time (r23). The workspace has no zone database, and the zone in which Google filters all-day events is unverified until LV17. If the adapter widened, the core's own check would mean nothing. Ruled 2026-11-18 by the lead (I.5 D-16) |
| DD18 | The account link. The shell supersedes a sign-in only through the adapter's flow handle, under the lock of the token write, keyed by the order's account and the core's generation (D.2, D.8.1). A disconnect's revoke and forget are one job, `Unlink { account_id, revoke }`, on a worker of its own that is not the sign-in worker: drop the cached access token, one revocation attempt with the token still in the store, then forget (D.8.3). A disconnect raises `connect_generation`. At start every pending revocation is unlinked again, and every other Google account that is not connected is unlinked without revoking (D.8.4) | The revocation reads the token from the store and never deletes it, so a forget first would lose the revocation; behind a sign-in the token could stay 300 s in the store; a supersede without the lock lets a flow that passed its last check still write. Ruled 2026-11-18 by the lead (I.5 D-17) |

---------------------------------------------------------------------------------------------------------------------

### D.1 The port today and what changes

Today the port is one call, `Provider::dispatch(&DispatchOrder) -> AttemptDone` (CORE `ports.rs:265-269`). The order
names the broker's account and calendar only (`ports.rs:203-224`), copied from the effect row in the handoff commit
(`service/pipeline.rs:623-633`). `AttemptReport::Applied { event }` is dropped by the core
(`service/pipeline.rs:943-946`). `ProofAction` has only `Agent` (`ports.rs:25-31`). No order but the dispatch exists.
No watchdog ends a provider call: a request in `dispatching` gets `due_at_ms: None` (`service/pipeline.rs:567-577`).
`Attachments.provider_present` is one flag for the whole run (`broker.rs:76-82`).

After this section the port has five orders on the provider worker (dispatch, observe, read, reconcile, list) and a
second trait, `AccountLink`, on two workers of its own (W05 PO7; DD18): the sign-in worker runs `connect` and the
`forget` that follows a sign-in outcome (D.8.2, D.8.6); the unlink worker runs `unlink` for `disconnect_google` and
at start (D.8.3, D.8.4), so a disconnect never waits behind a sign-in. Each worker has its own `AccountLink` value
over the one token store and access-token cache, which take their own locks (the CD-W05 session, 2026-11-18).
Each order carries a
proof, an id and a deadline, and every order but the list a target. Each answer carries the order's id and an
optional account signal.

### D.2 The types (CORE `ports.rs`)

These are the port's shapes, defined here; the corrections of D.16 are marked (corrected). `Job` and `Input` are
0.5.4's. `Notification`, `ConnectProblem`, `AuthLostCause`, `ObservedEvent` and `SystemAction` are store row types
of ST `rows.rs` that `ports.rs` re-exports (X21), so this file builds only after step P3.

```rust
/// One calendar as the provider must name it. Resolved by the core from the store (D.3).
pub struct CalendarTarget {
    pub calendar: CalendarRef,          // the broker's account and calendar ids
    pub provider_calendar_id: String,   // Google's id from provider_calendars; never from an action
    pub time_zone: IanaZone,            // the calendar's zone (W05 6.9, PD33)
    pub is_primary: bool,               // the account's primary calendar (W05 6.1, PD31)
}

/// The proof that permits an order (PO8).
pub enum OrderProof {
    Agent(Authorization),                           // consumed in the commit that sent the order
    System { proof_id: Id, action_digest: Digest }, // an accepted system proof, filed in system_proofs
}

pub struct DispatchOrder { pub order_id: u64, pub request_id: Id, pub effect_id: Id, pub attempt: u32,
    pub proof: OrderProof, pub action: Action, pub target: CalendarTarget,
    pub provider_event_id: Option<String>, pub deadline_ms: u64 }
pub struct ObserveOrder { pub order_id: u64, pub request_id: Id, pub target: CalendarTarget,
    pub event_id: ProviderRef, pub fabric_created: bool, pub proof: OrderProof, pub deadline_ms: u64 }
pub struct ReadOrder { pub order_id: u64, pub request_id: Id, pub targets: Vec<CalendarTarget>,
    pub action: Action, pub fabric_created: Vec<(CalendarRef, String)>, pub proof: OrderProof,
    pub still_allowed: Arc<AtomicBool>, pub deadline_ms: u64 }
pub struct ReconcileOrder { pub order_id: u64, pub request_id: Id, pub effect_id: Id, pub target: CalendarTarget,
    pub event_id: ProviderRef, pub action: Action, pub approved_revision: Option<String>,
    pub proof: OrderProof, pub deadline_ms: u64 }
pub struct ListOrder { pub order_id: u64, pub account_id: Id, pub proof: OrderProof, pub deadline_ms: u64 }

pub enum AttemptReport {
    Applied { event: Option<EventRef>, confirmed: bool, notification: Notification },
    Transient { retry_after_ms: Option<u64> },
    Rejected { code: FailureCode, explanation: Explanation },
    NoAnswer,
}
pub struct AttemptDone { pub order_id: u64, pub request_id: Id, pub effect_id: Id, pub attempt: u32,
    pub report: AttemptReport, pub finished_at_ms: u64, pub account_signal: Option<AccountSignal> }

pub enum ReadFailure { Transient { retry_after_ms: Option<u64> }, Rejected { code: FailureCode } }
pub enum ObserveAnswer { Found { event: ObservedEvent, before: EventView, evidence: Vec<u8> }, Absent,
    Failed(ReadFailure) }
pub struct ObserveDone { pub order_id: u64, pub answer: ObserveAnswer, pub finished_at_ms: u64,
    pub account_signal: Option<AccountSignal> }
// spans: one per event of an events result, in order; empty for availability. Google's start and end as given,
// asked whatever the grant and used for nothing else; built from the occurrence's own start and end, never from
// originalStartTime; no widening and no zone arithmetic in the adapter. An all-day end date is exclusive. `Date`
// is the contract's date scalar. The core checks them (B.3.4 step 2.1) and stores, logs and shows none (DD17)
pub enum ReadSpan { Timed { start_ms: i64, end_ms: i64 }, AllDay { start_date: Date, end_date: Date } }
pub enum ReadAnswer { Done { result: ToolResult, spans: Vec<ReadSpan> }, Stopped, Failed(ReadFailure) }
pub struct ReadDone { pub order_id: u64, pub answer: ReadAnswer, pub finished_at_ms: u64,
    pub account_signal: Option<AccountSignal> }
pub enum ReadBack {
    Found { revision: String, matches_approved: bool, evidence: Vec<u8> },
    Cancelled { revision: String },
    NotFound,
    Failed(ReadFailure),
}
pub struct ReconcileDone { pub order_id: u64, pub read_back: ReadBack, pub finished_at_ms: u64,
    pub account_signal: Option<AccountSignal> }
pub struct ListedCalendarEntry { pub provider_calendar_id: String, pub name: Label, pub access_role: String,
    pub is_primary: bool, pub time_zone: IanaZone }
pub enum ListAnswer { Listed(Vec<ListedCalendarEntry>), Failed(ReadFailure) }
pub struct ListDone { pub order_id: u64, pub account_id: Id, pub answer: ListAnswer, pub finished_at_ms: u64,
    pub account_signal: Option<AccountSignal> }

pub struct AccountSignal { pub account_id: Id, pub kind: AccountSignalKind }
pub enum AccountSignalKind { AuthLost(AuthLostCause), AccessChanged }

/// `until` is the shell's monotonic stamp of the job plus PROVIDER_CALL_MS (B.13, X23): the worker starts no order
/// past it and bounds every HTTP call by `until - Instant::now()`.
pub trait Provider: Send {
    fn dispatch(&mut self, order: &DispatchOrder, until: Instant) -> AttemptDone;
    fn observe(&mut self, order: &ObserveOrder, until: Instant) -> ObserveDone;
    fn read(&mut self, order: &ReadOrder, until: Instant) -> ReadDone;
    fn reconcile(&mut self, order: &ReconcileOrder, until: Instant) -> ReconcileDone;
    fn list(&mut self, order: &ListOrder, until: Instant) -> ListDone;
}

// Sign-in, on its own worker (PO7).
pub struct ConnectOrder { pub account_id: Id, pub generation: u64, pub subject: Option<String>,
    pub known_subjects: Vec<String> }
pub struct ConnectOutcome { pub account_id: Id, pub generation: u64, pub result: ConnectResult } // (corrected)
pub enum ConnectResult {                                                                      // (corrected)
    Connected { subject: String, email: Email, scopes: Vec<String>,
                refresh_token_ends_at_ms: Option<i64>, token_ref: String },
    ConsentDenied,
    PartialGrant { missing: Vec<String> },
    AccountMismatch,
    Cancelled(ConnectProblem),  // every value of 0.4.7 but `already_an_account`, which only the core sets
}
pub enum RevokeOutcome { Revoked, NotConfirmed }
pub trait AccountLink: Send {
    fn connect(&mut self, order: &ConnectOrder) -> ConnectOutcome;
    /// One job, in this order on one thread (DD18): drop the cached access token; when `revoke`, the one revocation
    /// attempt with the token still in the store (30 s at most, W05 3.9); then forget. None when `revoke` is false.
    fn unlink(&mut self, account_id: &Id, revoke: bool) -> Option<RevokeOutcome>;
    fn forget(&mut self, account_id: &Id);
}

pub fn system_action_digest(action: &SystemAction) -> Digest; // (corrected) pub, so a worker can check DD2
```

What each answer means, and what a failure is:

| Order | Success | `Transient` / `Failed(Transient)` | `Rejected` / `Failed(Rejected)` | No answer at all |
|---|---|---|---|---|
| dispatch | `Applied`: a yes with the event (W05 6.6-6.8). `confirmed` is true when the event equals the approval after only the normalisations of the adapter's closed, named list, empty until the live run (W05 6.12, 18.7); a fitting `Applied` that is not confirmed is still applied (V23). For a delete, which has no body, `confirmed` is true only when the adapter's one GET of the same event, in the same order, under the same proof and `until`, made only after a 200 or 204, with the access token the DELETE used and with no retry, finds the event gone (404, 410 or `status: cancelled`, at any revision); otherwise it is false. That GET is mapped by its own function, never turns `Applied` into another class, and returns no event, revision, content or evidence | certainly nothing was changed: nothing sent, or refused before acting (W05 6.10) | a plain refusal with a contract `FailureCode` | `NoAnswer`: unknown. Also every 5xx, every unreadable 2xx, every answer past the deadline (W05 PD17) |
| observe | `Found` with the facts, the before-image and the raw event; `Absent` for 404, 410 or a cancelled event (W05 6.5) | a read that may pass, including a 5xx and no answer (W05 6.10 "For a read ... there is no NoAnswer") | a refusal, with its code (`invalid_arguments` for an id of the wrong form, W05 5.3; for a cut attendee list, W05 6.5) | the watchdog makes it `Failed(Transient)` |
| read | `Done { result, spans }` only when every page of every calendar arrived (W05 6.4, PD23); `Stopped` when the flag was false before a page | as observe | as observe; past the bounds of PD23 too | the watchdog: `ReadOutcome::NoAnswerInTime` (section B) |
| reconcile | `Found` with `matches_approved` computed by the provider's own `matches` (W05 6.12); `Cancelled`; `NotFound` | as observe | as observe | the watchdog makes it `Failed(Transient)` |
| list | `Listed` with every entry of every page; past `MAX_LIST_PAGES` (20) pages or `MAX_CALENDARS_PER_ACCOUNT` entries it is `Failed(Rejected { provider_rejected })` as a whole (W05 6.2; 0.6) | as observe | as observe | the watchdog makes it `Failed(Transient)` |
| connect | `Connected` with the checks of W05 3.5 and 3.6 passed and the token stored | not used: one flow, no retry | the other `ConnectResult` values | the core's watchdog (D.8.6) |
| unlink | `Some(Revoked)`; `None` when `revoke` was false (it only forgets) | not used: one attempt (W05 3.9) | `Some(NotConfirmed)`, also when no token is in the store | `Some(NotConfirmed)` |

Every answer may carry one `AccountSignal`. `AuthLost(GrantEnded)` or `AuthLost(ClientRejected)` comes from a failed
refresh or a 403 `insufficientPermissions` (W05 3.8, 6.10); `AccessChanged` from any other 403 (W05 6.10).

**The sign-in flow's handle (DD18).** The shell keeps a clone of the adapter's flow handle, taken before the link
moves to the sign-in worker; it is the adapter's type, not a port type. `Flow::supersede_latest()` serves a new
connect and quit, `Flow::supersede(generation)` supersedes that one flow only, and `PendingFlow::generation()` names
it. Each takes the lock under which a flow writes the token store, so once a supersede returns, the flow it named
never writes the store. The link keys a supersede, and the check of a queued `Connect`, by the order's `account_id`
and the core's `generation`, not by the flow's own count (the CD-W05 session's build note, agreed 2026-11-18). A
queued `Connect` that is already superseded answers `Cancelled(Superseded)` without starting. The shell never
supersedes through a bare flag. Ruled 2026-11-18 by the lead (I.5 D-17).

### D.3 The calendar target (PO1)

`resolve_target(r: &ReadTx, calendar: &CalendarRef, attachments: &Attachments) -> Result<CalendarTarget, StoreFault>`
in CORE `service/provider.rs` (new):

| The account | Target |
|---|---|
| has a `provider_accounts` row (a Google account) | `provider_calendar_id`, `time_zone` and `is_primary` from its `provider_calendars` row, whatever `listed` says (a reconcile may run on a calendar Google no longer lists) |
| is the practice account under a practice run (P11) | the practice calendar's fixed target: `provider_calendar_id` equal to the broker's calendar id, zone `Etc/UTC`, `is_primary: true` |
| anything else | an invariant fault |

It is called in the commit that sends the order, never earlier: the handoff commit `request.dispatch` (CORE
`service/pipeline.rs:593-634`) for a dispatch and a read; the commit `system.proof` that accepts the system proof for
an observe and a reconcile. A calendar lost between authorize and dispatch raises `revocation_epoch` (D.9),
so the handoff's fence check (`service/pipeline.rs:471`) voids the authorization with `fence_changed` before the
target is resolved. A system proof's fence is checked in the same commit. An unresolvable pair after a fence that
held is therefore a store fault (REV-port PO1 answer 3), never a fallback.

### D.4 Proofs on orders (PO8)

- A dispatch and a read carry `OrderProof::Agent` with the authorization consumed in the same commit, as today
  (CORE `service/pipeline.rs:595-627`).
- An observe carries `OrderProof::System` for `SystemAction::Refresh { calendar }`; a reconcile for
  `SystemAction::Reconcile { calendar, event_id }`; a list for `SystemAction::ListCalendars { account_id }` (0.5.1).
  The commit `system.proof` files the proof (`SystemProofRecorded`, A.4.3) and sends the order. Section B owns how a
  system proof is judged; this section relies on: Isabelle only, bound to its job by the action digest, fence
  unchanged at acceptance.
- The worker recomputes the digest of the order's action (`Action::digest`) or of the system action the order
  implies (`system_action_digest`) and refuses a mismatch: a dispatch `Rejected { provider_rejected }`, a read kind
  `Failed(Rejected { provider_rejected })`. A test of the core (with FAKE) asserts that every request FAKE logs
  matches one consumed authorization or one `system_proofs` row of the store (REV-port PO8).
- A dispatch order may make, under its own consumed authorization, one GET of its target event before the change
  (invitation, response) or after a 2xx delete; such a GET is not a broker read and needs no system proof (DD16,
  OD7). The FAKE log test therefore accepts, per consumed authorization, one changing request plus one GET before
  it (invitation, response) or one GET after it (delete), and nothing else. Ruled 2026-11-18 by the lead.

### D.5 Order ids, deadlines and the watchdog

- `order_id` comes from one counter of the run, separate from proof job ids. The core keeps `orders:
  BTreeMap<u64, OrderInFlight>` in memory (B.13: `kind`, `request_id`, `expects`, `account_id`, `deadline_ms`,
  `still_allowed`).
- `deadline_ms` is the sending commit's core time plus `PROVIDER_CALL_MS` (30 000, CORE `broker.rs:24`); the core's
  watchdog reads it. The adapter's own bound is `until`, the shell's monotonic stamp plus `PROVIDER_CALL_MS` (B.13,
  X23): it sends nothing past it (W05 5.2) and answers `Transient` when it passed before sending (W05 6.10).
- In the sending commit the request's `due_at_ms` becomes `deadline_ms + ORDER_WATCHDOG_MARGIN_MS` (5 000). The
  core's `step` then finds an order still in `orders` past that time and ends it: removes it from `orders` and
  handles it as below. An answer whose `order_id` is not in `orders` is ignored, except for its `AuthLost` (DD15).
- An answer whose request is not in the order's `expects` state is ignored: nothing is stored, the order leaves
  `orders`, and an `AuthLost` is still applied (DD15). A removal or a disconnect also drops the orders of the
  requests it moved (C.3.3, D.8.3).

| Order past its watchdog | Handled as |
|---|---|
| dispatch | `NoAnswer`: the attempt finishes `outcome_unknown`, the request goes `outcome_unknown` (D.7) |
| observe | `Failed(Transient)` (D.6.3) |
| read | `ReadOutcome::NoAnswerInTime`, `dispatching -> retry_wait` (section B, V5) |
| reconcile | a failed reconcile read (D.7.4) |
| list | a failed list: nothing changes (D.9) |

A list has no request; its watchdog is a core timer read by `pump` (the earliest `orders` deadline joins the next
due time). The prover's `PROOF_WATCHDOG_MS` (35 000) applies to system proofs as to agent proofs.

### D.6 The observation (PO2, CO6)

#### D.6.1 When

The actions that name an event are `fabric_calendar_propose_update`, `propose_invitation`, `propose_response` and
`propose_delete` (`ReadTx::open_effects` names the same four, `crates/fabric-storage/src/read.rs:925-936`). For these:

| The request enters | Instead of starting the proof at once | Then |
|---|---|---|
| `validating`, after `invalid_arguments` passed (CORE `service/pipeline.rs:387-434`) | it stays in `validating`; the core starts a system proof on `Refresh { calendar }`, after the checks below | the observe order; its answer decides (D.6.3) |
| `proving_fresh_permit` from `allowed`, `approved`, `retry_wait` or `queued` (a void or a missing authorization, `service/pipeline.rs:636-648`) | the same, inside `proving_fresh_permit` | as above |
| a fresh-permit proof again after `Judgement::Discard` (`service/pipeline.rs:767-770`) | the same: observe again, in `proving_fresh_permit` | as above |
| a classification proof again in `proving_classification` (after `Judgement::Discard`, or after a restart, `service/recovery.rs:74-85`, `service/pipeline.rs:500-538`) | no new observe: the proof starts at once from the request's stored `observe` row (D.6.2 item 3; the fence carries its revision) | the lifecycle has no edge from `proving_classification` to `terminal_failure` or `stale`; the observe before the fresh permit catches a changed event before any dispatch |

A core test, `a_discarded_classification_proof_of_an_update_is_proved_again_with_no_second_observe`, pins the last
row.

Checks before an observe, in the same commit, from facts that need no event:

- The agent is not paused, the active profile does not block the action's row, and the named calendar's access is
  `owner` or `writer`. The row is read from `ReadTx::active_profile` (`crates/fabric-storage/src/read.rs:463`), the
  profile `TrustedFacts.profile` carries (CORE `service/facts.rs:157`): `respond_to_invitation` for a response,
  `send_or_change_invitations` for an invitation, and for an update both `edit_fabric_created_event` and
  `edit_preexisting_event` (the edit row depends on the event, THEORY 364-366, so only both blocking is known
  without it). A delete is never refused here: its row always asks (THEORY 665, `delete_row_always_asks`). If one
  check fails, no system proof and no observe start. The request goes to its proof at once with `facts.event: None`
  and a fence revision of `None`. The theory blocks; the first failing rule in theory order (THEORY 550-554) is then
  one of `agent_not_paused`, `mode_not_blocked`, `account_connected`, `calendar_selected` or `calendar_writable`,
  never one that reads the event. So a paused agent, or one whose profile blocks the row, drives no Calendar read
  and learns nothing of the event's state (Q21; C.7.2). Ruled by the lead 2026-11-17 (I.5 D-12; D-11 closed). A core
  test, `a_blocked_row_causes_no_observe_and_gets_mode_not_blocked_for_every_event_state`, pins it for a response,
  an invitation and an update whose two edit rows block, each with the event at the named revision, at another
  revision and absent: `orders` stays empty and no `observations` row is written.
- `can_carry_out(attachments, account, provider_account)` is true. When it is false, no system proof and no
  observe order start; the proof starts with `facts.event: None` (D.6.2 item 3), which never permits. A run with no
  provider (E.1.2 rows 1 and 3, or a debug run without the Google client file) therefore never sends a job no
  worker would take.

No contract state exists for "observing" (REV-port PO2 answer 3). The request waits in `validating` or
`proving_fresh_permit` with `due_at_ms` at the system proof's deadline plus 1, then at the observe order's watchdog
time (D.5). If the broker stops meanwhile, recovery makes both states and `proving_classification` due at once (CORE
`service/recovery.rs:74-85`); the first two start again with a new system proof and a new observe, and
`proving_classification` proves again from the stored observation (D.6.1): a restart only adds reads. The re-proof
counter (`MAX_REPROOFS`, W04 D16) counts the proofs as today; an observe is not a proof and has its own retry count
(D.6.3).

While the prover's status is not Ready, or no prover is attached, no system proof starts. The request is handled as
the row "the system proof does not permit" of D.6.3, with P0's cause for a prover that is not ready in place of
`observation_failed`, as an agent proof ends today (B.10). Holding it while Isabelle starts is the planned follow-up
(P8).

#### D.6.2 The record

The observe answer is handled in one commit, `provider.observation`:

1. The answer is matched first (D.5): its order is in flight and its request is still in the order's `expects` state;
   otherwise it is ignored. `ObservationRecorded` with `purpose: observe`, `proof_id` of the system proof, the
   broker's account and calendar ids, the event id of the order, `outcome` `found` or `absent` (`cancelled` is
   written only by reconcile rows, D.7.2), and for `found` the `revision` and `facts`. Before storing, the core
   overwrites two members of the provider's `ObservedEvent` with its own values: `calendar` (the order's target) and
   `fabric_created` (DD12, read in this commit by `ReadTx::created_event`). An answer whose `event_id` differs from
   the order's is handled as `Failed(Rejected { provider_rejected })`.
2. For `found`, the before-image and the raw event go to `observation_evidence` by `WriteTx::put_evidence` in the
   same transaction (A.4.4). The content is the canonical JSON `{"before": EventView, "raw": text}`, where `raw` is
   the provider's event as UTF-8 text; for a reconcile row it is `{"before": null, "raw": text}`.
   `MAX_BEFORE_VIEW_BYTES` (524 288) bounds the before-view and `MAX_EVIDENCE_BYTES` (1 048 576) the whole, both
   measured in canonical form before `put_evidence` is called, because a fault there poisons the write (A.4.4).
   Past either, the core stores no observation and no evidence, handles the answer as
   `Failed(Rejected { invalid_arguments })` and gives the fixed explanation "Google's copy of this event is too
   large for Fabric to keep a record of. Nothing was changed. Make this change in Google Calendar." In
   `validating` that is `terminal_failure` `invalid_arguments`; in `proving_fresh_permit` it is `undecide()` with
   cause `observation_failed` (D.6.3). No card opens and nothing is dispatched: no proof runs without a
   before-image the owner can be shown. The card's attendee list is never cut, and `ObservedEvent.attendees`
   stays complete. A stated limit of version 1 (owner OD26). When the latest found observation of the same
   account, calendar, event and revision has evidence of the same digest, the new `ObservationRecorded` names
   that `evidence_id` and `put_evidence` is not called (A.4.4, A.7); no ledger payload carries the digest (V3).
   Ruled 2026-11-18 by the lead.
3. The move or the proof of D.6.3. When the proof starts, `facts::proof_job` reads, in this same transaction, the
   request's latest `observe` row with outcome `found` and builds `TrustedFacts.event` from its `facts` (today always
   `None`, CORE `service/facts.rs:163`) and the fence from its `revision` (DD5). Facts and fence come from one read.
   If no such row exists the event stays `None`; THEORY then fails `Event_known` (595-596), which never permits.

The before-image is read only by `get_request_details` with `full` redaction, for the owner's card (`private.before`,
W05 CO6). It never enters `TrustedFacts`, a status, a notice or a ledger payload (V3). The shell's observation
hook (BR `shell.rs:378-392`; PW 4.4) is removed in the same pull request.

Core tests of the bounds (ruled 2026-11-18 by the lead):
`an_observe_answer_one_byte_past_either_bound_stores_nothing_and_fails` (in `validating`: `terminal_failure`
`invalid_arguments` with the fixed explanation, no `observations` row and no evidence row; in
`proving_fresh_permit`: `undecided` `observation_failed` and no dispatch);
`two_observations_of_one_event_at_one_revision_leave_one_evidence_row`; and
`the_largest_request_details_answer_fits_one_frame`: a before-view at `MAX_BEFORE_VIEW_BYTES`, an after-view at
the same bound once P2-32 adds `private.after` (V23), an invitation action of 200 addresses of 320 bytes, 200
proof ids, 20 attempts and the warnings at their maximum fit `MAX_FRAME_BYTES`. If they do not, the core lowers
`MAX_BEFORE_VIEW_BYTES` (to about 400 KiB) until they do. The after-view is bounded by `MAX_BEFORE_VIEW_BYTES`
too.

#### D.6.3 What an observation leads to

If the agent is paused at the commit that handles an observe answer, or the active profile then blocks the action's
row as D.6.1 reads it, this table is not applied: the proof starts with the stored observation when it is `found`,
otherwise with `event: None`, and the theory blocks `agent_not_paused` or `mode_not_blocked` (D.6.1; I.5 D-12).

Only edges of DRAFT `request-lifecycle.json` are used: `validating -> terminal_failure`, `validating ->
proving_classification`, `proving_fresh_permit -> stale`, `proving_fresh_permit -> undecided` (CC
`lifecycle.rs:155-170`). Reducer events `ObservedStale` and `ObservedInvalid` are added (0.4.5); this section owns
those rows.

| Observation | In `validating` (before classification) | In `proving_fresh_permit` |
|---|---|---|
| `found`, the revision the action names (for a response to an occurrence, also the original start, W05 CD-Q05b) | `Checked { valid: true }`, then the classification proof in the same commit. | the fresh-permit proof in the same commit |
| `found`, another revision | `ObservedInvalid`: `terminal_failure` `revision_conflict` | `ObservedStale`: `stale` |
| `absent` (a cancelled event included) | `ObservedInvalid`: `terminal_failure` `invalid_arguments` "There is no such event." For an event that DD12 calls Fabric-created and whose create reached `applied` less than `FRESH_CREATE_ABSENT_MS` (300 000) ago, first a retry as for `Transient` (W05 CO6: a 404 may pass) | `ObservedStale`: `stale` |
| `Failed(Transient)`, or the watchdog | stays; observe again at `retry_due` (CORE `service/pipeline.rs:96-117`, `RetryPolicy::PLAN`); when it returns `None`: `ObservedInvalid`, `terminal_failure` `retry_budget_exhausted` | stays; observe again by the same rule; at the end: `undecide()` with cause `observation_failed` |
| `Failed(Rejected { code })` | `ObservedInvalid`: `terminal_failure` with that code | `undecide()` with cause `observation_failed` |
| the system proof does not permit (block, need-approval, undecided, discarded past its re-proofs) | `undecide()` with cause `observation_failed` (the state `validating` has no edge to `undecided`, so the core first takes `Checked { valid: true }` to `proving_classification` in the same commit) | `undecide()` with cause `observation_failed` |

For the row "`Failed(Transient)`, or the watchdog": the new system proof starts only through the checks of D.6.1
(D.7.2 step 1 for a reconcile); when one fails, none starts and the request goes on as D.6.1 says.

The observe's retry count and the time of its first try are in memory, per request (no failed observe is stored:
`ObservationOutcome` has no failure value, 0.4.7). A restart loses them and so can only add tries.
`observation_failed` shows the agent and the owner `failed` until P2-7 adds `provider_unreachable` (0.4.4).

For the row "`validating`, system proof does not permit": `validating -> undecided` is not an edge; `validating ->
proving_classification -> undecided` both are (CC `lifecycle.rs:128-131`), and the request has passed its argument
checks, so the move is true to what happened.

### D.7 Reconciliation out of `outcome_unknown` (PO4, CO8)

#### D.7.1 Entering it

| Attempt answer (`request.attempt`) | Commit |
|---|---|
| `Applied { confirmed: true, event, notification }` | attempt finished `applied`; `EffectEventRecorded { event, notification }`; `dispatching -> applied -> reconciled` (DD8). For a delete, `event: None`, and `confirmed` comes from the adapter's one GET in the order (D.2) |
| `Applied { confirmed: false, event }` that fits the order, for a create, update, invitation or response (V23) | attempt finished `applied`; `EffectEventRecorded { event, notification }` while the request is in `dispatching`; one audit row `provider_differs`; then `dispatching -> outcome_unknown -> reconciling -> needs_owner_resolution` in the same commit, with no read and no due time, until P2-33 adds the edge `dispatching -> needs_owner_resolution` (B.14). Never sent again, never read back, never `absence_confirmed` |
| `Applied { confirmed: false, event: None }` of a delete | as `NoAnswer` (V9), with no audit row; D.7.2 reads it back |
| an `Applied` that does not fit the order (V9) | as `NoAnswer`, with one audit row `provider_mismatch`. Nothing is written to the effect's event columns |
| `NoAnswer`, or the dispatch watchdog | attempt finished `outcome_unknown` (CORE `service/pipeline.rs:984-987`); `dispatching -> outcome_unknown`; `due_at_ms` = this commit's time + `RECONCILE_FIRST_READ_DELAY_MS` (10 000) |
| `Transient` | as today: `retry_wait` while `retry_due` allows, else `terminal_failure` `retry_budget_exhausted` (`service/pipeline.rs:947-970`). For an agent whose enrollment is not approved (removed), the same commit moves it on `retry_wait -> cancelled` (`AgentRemoved`, X22) |
| `Rejected` | as today: `terminal_failure` with its code (`service/pipeline.rs:971-983`) |

If the account is `disconnected` when the answer arrives, a request that would stay in `outcome_unknown` continues in
the same commit `outcome_unknown -> reconciling -> needs_owner_resolution` (D.8.3).

Core tests (ruled 2026-11-18 by the lead): `a_confirmed_delete_is_applied_and_reconciled_in_its_attempts_commit`;
`an_unconfirmed_delete_goes_to_reconciliation_with_no_audit_row`; and
`a_fitting_unconfirmed_applied_waits_for_the_owner_with_its_event_recorded_and_no_read`, the interim chain on a
practice fault: the fault `applied_differs` (E.1.4 row 8a) answers a create and an update with a fitting
`Applied { confirmed: false }`; each commit records the event, writes one `provider_differs` row and moves the
request through `outcome_unknown` and `reconciling` to `needs_owner_resolution`; no order follows, no
`observations` row is written, and the request recovered at `broker.started` gets no due time. An `Applied`
with another id still goes to reconciliation with `provider_mismatch`.

#### D.7.2 One reconcile read

`step` gains arms for `outcome_unknown` and `reconciling` (today the fallback arm clears their due time, CORE
`service/pipeline.rs:567-577`). When such a request is due:

1. If its account is not `connected`, or the prover is not `Ready`, or `can_carry_out` is false for the account:
   `due_at_ms: None`. Recovery of a run that can carry it out makes it due (D.7.4). Nothing is read and no budget
   is used. The commit that makes the account `connected` again (D.8.2) sets every `outcome_unknown` and
   `reconciling` request of the account due at its time + 10 000; the prover's change to `Ready` makes them due at
   once. THEORY blocks `System_reconcile` on an account that is not connected (`Account_connected`, 569 and 591).
2. Otherwise, in one commit `request.advance`: from `outcome_unknown` the move `ReconcileDue` to `reconciling`
   (`lifecycle.rs:146`); in `reconciling` no move. Then the system proof on `Reconcile { calendar, event_id }`, where
   `event_id` is `OpenEffect.event_id` (the reserved id of a create, the target of every other tool;
   `read.rs:925-936`). THEORY permits it while the triple is a pending effect (522-523), which holds in both states
   (`read.rs:917`).
3. Commit `system.proof`: the proof filed and the `ReconcileOrder` sent with `approved_revision` (the action's
   revision; `None` for a create) and the target (D.3).
4. Commit `request.reconcile` on `ReconcileDone`, matched first as D.5 says (the request still in `reconciling`):
   `ObservationRecorded` with `purpose: reconcile`, the outcome,
   `revision`, `matches_approved`, and the evidence for `found` (`put_evidence`); then the verdict of D.7.3.
   A `Found` read-back whose evidence passes `MAX_EVIDENCE_BYTES` writes no row and is a failed read (D.7.4); it
   is never `NotFound` and never a store fault. Byte-equal evidence is named again as in D.6.2.

#### D.7.3 What a read-back leads to

`n` is the number of `reconcile` rows of `observations` for the request before this one (V19): durable across a
restart. The edges used are `reconciling -> applied | absence_confirmed | needs_owner_resolution` and
`applied -> reconciled` (CC `lifecycle.rs:141-150`); "wait" is no move. The table is W05 section 7, owned by the core
(REV-port PO4); the provider supplies only the read-back.

| Tool | Read-back | Verdict | Commit |
|---|---|---|---|
| create | `Found`, `matches_approved` | applied | `EffectEventRecorded { (reserved id, revision), not_requested }`; `-> applied -> reconciled` |
| create | `Found`, not matching | needs owner | `-> needs_owner_resolution` |
| create | `Cancelled` | needs owner | `-> needs_owner_resolution` |
| create | `NotFound` | not yet | `n = 0`: wait, due at this commit + `RECONCILE_SECOND_READ_MS` (60 000). `n >= 1`: `-> absence_confirmed` |
| update, invitation, response | `Found`, `matches_approved`, any revision | applied | `EffectEventRecorded { (target id, revision), N }`; `-> applied -> reconciled` |
| update, invitation, response | `Found`, revision equals `approved_revision`, not matching | not yet | as the create's `NotFound` |
| update, invitation, response | `Found`, another revision, not matching | needs owner | `-> needs_owner_resolution` |
| update, invitation, response | `Cancelled` or `NotFound` | needs owner | `-> needs_owner_resolution` |
| delete | `Cancelled` or `NotFound` | applied | `EffectEventRecorded { None, N }`; `-> applied -> reconciled` |
| delete | `Found`, revision equals `approved_revision` | not yet | as the create's `NotFound` |
| delete | `Found`, another revision | absent | `-> absence_confirmed` at the first read: the conditional delete cannot apply to a changed event (W05 PD14). The one exception to the two-read rule (DD7; I.5 D-10) |
| any | `Failed(_)`, the watchdog, or the system proof does not permit | failed read | D.7.4 |

`N` is `not_requested` when the action's `send_updates` is `none` (or the action has none), otherwise `unknown`
(W05 section 7 "Notification"; PLAN 11). "Not yet" twice means two successful reads: the second commit also checks
that its `observed_at_ms` is at least 60 000 after the first `not yet` row's; if not, it waits again. Times are the
core clock, which never goes back (W04 D34).

A request in `needs_owner_resolution` is open in schema version 3 and leaves only by `resolve_outcome` (DRAFT
README section 9). The reason the owner sees is derived from the latest `reconcile` observation and the tool; nothing
new is stored. `absence_confirmed` is the broker's two reads, not Google's statement: a late PATCH or DELETE can
still land (REV-core rule PD20); the screens' wording must say "not found", not "confirmed absent".

A request moved to `needs_owner_resolution` by V23 has no `reconcile` observation. Its reason is "applied,
differs", derived from its effect's recorded event and its attempt's outcome `applied`; the contract carries
that reason with P2-31, before any Google dispatch (B.14). Ruled 2026-11-18 by the lead.

Why this is safe against a repeat. Update, invitation, response and delete carry `If-Match` with the approved
revision (W05 PD14), so a late original and a later new request for the same change cannot both apply. A create has
no such guard across requests: a new request gets a new reserved id (CORE `service/pipeline.rs:864-866`). The two
reads make a duplicate less likely, not impossible (W05 RK3). DD12 keeps the absent create's id on record so that a
late event under it is recognised as Fabric's and shown. The residual goes to the owner (D.14, OQ-D1).

A residual of the delete's one-read exception: a delete applied and then restored at a new revision before the
first reconcile read ends `absence_confirmed`, although the delete was made and notices may have gone out. After
a confirmed delete (D.2) this cannot happen; the window is the unknown outcome alone. Whether Google can restore
a deleted event under its id is the CD-W05 session's live check LV15. Told to the owner beside OQ-D1 (D.14), not
asked.

#### D.7.4 Failed reads, budgets and recovery

- A failed reconcile read (a `Failed` read-back, the watchdog, a system proof that does not permit) writes no
  observation. The request stays in `reconciling` and is due again at `retry_due(RetryPolicy::PLAN, failed + 1,
  first_failed_at, hint, now)`; the count and `first_failed_at` are in memory and restart at zero after a restart
  (they only add reads). When `retry_due` returns `None`: `-> needs_owner_resolution` (OP11). A `Found`
  read-back whose evidence passes `MAX_EVIDENCE_BYTES` is such a failed read; it never counts as `NotFound` or
  `Cancelled`, nor toward the two reads of D.7.3 (D.7.2 step 4). A core test,
  `an_oversized_reconcile_read_back_is_a_failed_read_and_never_absence_confirmed`, pins it: at the end of the
  budget the request is in `needs_owner_resolution`. Ruled 2026-11-18 by the lead.
- If the failure carried `AuthLost`, or the account is not `connected` at the commit, the request waits as in D.7.2
  step 1 and its failed-read count restarts when the account is connected again. A lost sign-in is not Google's
  answer about the change.
- Recovery (`broker.started`, CORE `service/recovery.rs:22-90`) keeps today's `dispatching -> outcome_unknown` and
  gives it a due time; it gives `outcome_unknown` and `reconciling` a due time too (today they get none, `:86`):

| State at start | Due at |
|---|---|
| `dispatching` (a change) | moved to `outcome_unknown` as today; recovery time + 10 000 |
| `applied` (a change committed by main, B.14) | moved to `reconciled` at once, with no event recorded |
| `outcome_unknown` | recovery time + 10 000 |
| `reconciling` with no `reconcile` row | recovery time + 10 000 |
| `reconciling` with a `not yet` row | the later of recovery time + 10 000 and that row's `observed_at_ms` + 60 000 |

  A recovered request never skips a read: the count of successful reads is in the store, and a lost order or a
  lost answer only causes another read. A request in `needs_owner_resolution` gets no due time, as today; a V23
  request is never in `outcome_unknown` or `reconciling` across a commit (B.14), so recovery never makes it due.

#### D.7.5 `resolve_outcome`

The owner's statement about a request in `needs_owner_resolution` (DRAFT README section 7). Operation type
`owner.resolve_outcome`, correlation the request id (AD14). Built in step P5a: it needs only migration 2 and works
on the practice account, where D.7.4 and D.8.3 first move requests into `needs_owner_resolution`.

| # | Check | Answer |
|---|---|---|
| 1 | No request with that id | `not_found` |
| 2 | `expected_revision` is not the request's revision, or its state is not `needs_owner_resolution` | `stale_request` (DRAFT choice 55; params `{request_id, expected_revision, resolution}`, DRAFT CC `owner.rs:1875-1879`) |

Then one commit: `RequestMoved` by the reducer event `Resolved(Resolution)` (0.4.5) to `resolved_as_applied` or
`resolved_as_absent`; the terminal move ends the request's one-request grants `consumed` through `move_request`
(CORE `service/requests.rs:41-55`). No proof, no order, no change at Google. One request notice; the result is
`{commit_sequence, request}` (DRAFT CC `owner.rs:1884-1887`). The request leaves
the open-request limits. `resolved_as_applied` names no event, so its outcome is never released (B.5.3).

For a request of V23 the contract will refuse `absent` (P2-34). The core adds no interim check of its own: until
P2-31 to P2-34 merge, V23 is reachable only by a practice fault, and they are needed before any Google dispatch.
Ruled 2026-11-18 by the lead.

### D.8 Accounts in the core (CO1 to CO4, CO11)

#### D.8.1 `connect_google` (CO1, CO2)

One commit, `owner.connect_google`; DRAFT's params carry only `account_id` (CC `owner.rs` draft,
`ConnectGoogleParams`).

1. `account_id: null`. If a `provider_accounts` row with `subject` null exists (read from `provider_accounts()`,
   0.5.2), that account is reused (CO2: at most
   one account that never connected). Otherwise the budget check (D.11) runs, then a new account: id
   `crate::ids::new_id("account")`, `AccountPut { state: awaiting_consent, revision 1, email: null, calendars: [] }`,
   `ProviderAccountPut { subject: null, connect_generation: 1, provider: "google" }`.
2. `account_id` given. Unknown: `not_found`. An account with no `provider_accounts` row (the practice account):
   `invalid_params` "This account is not a Google account." Otherwise `state_before_connect` keeps the state it had.
3. In both cases: `connect_generation` + 1, state `awaiting_consent`, account revision + 1, `revocation_epoch` + 1
   (W04 5.6), notices `Change::Account` and the provider account change (P2-9), the answer, and after the commit
   `Job::Connect(ConnectOrder { account_id, generation, subject: the stored subject, known_subjects: every other
   non-null subject of `provider_accounts()` })`.
4. One sign-in at a time (W05 PD5): when the shell queues a `Connect` it first calls `Flow::supersede_latest()`
   (D.2), which takes the lock of the token write; the old flow answers `Cancelled(Superseded)` with its own
   generation and, once the call has returned, never writes the token store. A queued `Connect` that is already
   superseded (keyed by its `account_id` and the core's `generation`), or that the worker takes while the shell is
   quitting, answers `Cancelled(Superseded)` without starting. The sign-in worker is one thread, so jobs run in
   order. No `Connect` is sent for an account in the core's in-memory set `unlink_in_flight` (D.8.3): the
   `connect_google` commit still commits, and its `Connect` job leaves when the account leaves the set. At quit the
   shell calls `Flow::supersede_latest()` and does not wait for the unlink worker; a revocation still pending stays
   `pending` in the store (D.8.4). Ruled 2026-11-18 by the lead (DD18).

A `connected` account given to `connect_google` is not connected for up to `SIGN_IN_WAIT_MS` (300 000): every proof
sees it as not connected and its agents' work stops (REV-core CO1 c). The contract's state machine has no
"connected, re-consenting"; the screens must say so (OP7, owner).

#### D.8.2 The sign-in outcome (PO7, CO11)

One commit, `provider.connect`. An outcome whose `generation` is not the account's current one is ignored, except
that the core sends `Forget(account_id)` after it when it was `Connected` (the worker stored a token no record uses).
When a newer `Connect` of the same account is queued or running, that `Forget` is not sent: sent then, it would run
after the newer flow and delete the token that flow stored. The newer flow's write replaces the token, and when
the newer outcome is not `Connected`, `Forget(account_id)` follows it in any case (DD18).

| Result | The core checks, in the commit | Then |
|---|---|---|
| `Connected` | the scopes are exactly W05 3.6's five; for a reconnect, `subject` equals the stored one; for any account, no other `provider_accounts` row has this subject (`provider_account_by_subject`) | all hold: `subject` set (first time), scopes, `token_ref`, times, `last_connect_problem: null`, `auth_lost: null`; account `connected`, email set; then D.9's list (after TH1), and the account's waiting reconcile requests due (D.7.2) |
| `Connected`, a check fails | | a subject already an account: `account_mismatch`, `last_connect_problem: already_an_account`; another subject on reconnect: `account_mismatch`; scopes: `partial_grant` or `cancelled` (`unexpected_scope`). In each case `Forget(account_id)` after the commit |
| `ConsentDenied`, `PartialGrant`, `AccountMismatch`, `Cancelled(problem)` | | the state of W05 3.7, `last_connect_problem` for `Cancelled`; if the account had a token (a reconnect), `Forget(account_id)` (W05 3.7, OQ12 default: fail closed) |

Each case: account revision + 1, `revocation_epoch` + 1, notices. The core's own subject check comes first because
the store's partial unique index on `subject` (A.5) would make a duplicate a `StoreFault`, which halts the run
(W04 D7; REV-port PO7).

An account signal `AuthLost(cause)` (DD15): `provider_accounts.auth_lost = cause`, account
`reauthentication_required`, revision + 1, `revocation_epoch` + 1, notices; no new job. The token is already gone
(W05 3.8). Requests of the account: queued authorizations are voided at the handoff by the epoch (CORE
`service/pipeline.rs:471`); new fresh proofs fail `Account_connected`; reconcile reads wait (D.7.2).

#### D.8.3 `disconnect_google` (CO4)

Today: grants end `account_disconnected`, `pending_approval` requests go `stale`, the account `disconnected`, the
epoch raised, `revoke_provider_token` ignored (CORE `owner/commands.rs:241-273`). Steps 1 and 3 below are built in
step P5a and step 2 in step P5b (B.9): they need only migration 2 and apply to the practice account as to a Google
account. Steps 4 and 5 are built in step P7b. Added, in the same commit `owner.disconnect_google`:

1. Every request of the account in `outcome_unknown`: `-> reconciling -> needs_owner_resolution`; in `reconciling`:
   `-> needs_owner_resolution` (REV-core CO4 a; `outcome_unknown -> needs_owner_resolution` is not an edge). The
   account may never come back, and an open request would wait for good.
2. Held results of the account purged, `HeldResultPurged { cause: account_disconnected }` (section B).
3. In-flight observe, reconcile and list orders of the account are dropped from `orders`, and every system proof in
   flight that serves a request this commit moved; their answers and results are then ignored (B.11.3, D.5; an
   `AuthLost` among them changes nothing on a disconnected account).
4. `provider_accounts.revocation`: `pending` when `revoke_provider_token`, else `not_requested`; and
   `connect_generation` + 1, so the outcome of a sign-in of this account already under way is ignored and followed
   by `Forget` (D.8.2). A disconnect never supersedes the sign-in of another account.
5. After the commit: when the sign-in running or queued is this account's, the shell first calls
   `Flow::supersede` for it (D.2); then `Job::Unlink { account_id, revoke: revoke_provider_token }` goes to the
   unlink worker, which is not the sign-in worker, and the account joins `unlink_in_flight`. The unlink drops the
   cached access token, makes the one revocation attempt with the token still in the store (30 s at most, W05 3.9)
   and then forgets at once, so a disconnected account's token stays in the store for that attempt at most, not
   behind a sign-in for 300 s. `Unlinked(account_id, Some(outcome))` is recorded in a later commit
   `provider.revoke`: `revoked` or `not_confirmed`, with the provider account change. `Unlinked(account_id, None)`
   (no revocation asked) writes nothing. Either way the account leaves `unlink_in_flight`, and a `Connect` held
   back for it leaves (D.8.1). Ruled 2026-11-18 by the lead (DD18, I.5 D-17).

A request in `dispatching` at the disconnect runs on (DRAFT README section 7, `remove_agent`: no false rollback). Its
answer is applied by D.7.1, including the rule for a disconnected account. The unlink may remove the token before the
call is made; the call then fails before sending (W05 3.8) and the answer is `Rejected` (nothing was sent).
Observations are kept (W05 CD-Q11h). Whether to warn the owner first is W05 OQ16 (default: no warning).

#### D.8.4 Recovery of accounts (CO3)

In `broker.started`, for every account in `awaiting_consent` (its `provider_accounts` row read from
`provider_accounts()`): with `subject` null, `cancelled`; with a subject, the
state in `state_before_connect` (REV-core CO3: a restart must not throw away a working connection). The sign-in flow
wrote a token only after every check, including the subject (W05 3.2, 3.7), so a token found later is of the same
subject. `revocation_epoch` + 1 once if any account changed, and one account notice each. Then, for every
`connected` Google account, D.9's list once the prover is `Ready` (after TH1).

After that commit, before D.9's lists and before any `Connect`, the core sends `Job::Unlink { account_id, revoke:
true }` for every Google account whose `provider_accounts.revocation` is `pending`, and `Job::Unlink { account_id,
revoke: false }` for every other Google account that is not `connected` after this recovery, cancelled ones
included; each joins `unlink_in_flight`. A broker that died between a disconnect and its revocation so finds the
token and revokes now. Repeating is safe: deleting a token that is not there is `Ok` (I.9). If the broker stopped
after the unlink's forget but before the `provider.revoke` commit, the repeat finds no token and records
`not_confirmed`, which never claims a revocation; the owner is told to remove the access in his Google account
(W05 3.9). With the memory token store a restart finds no token, so a pending revocation ends `not_confirmed`
too. Ruled 2026-11-18 by the lead (DD18).

#### D.8.5 Ids

Accounts: `account-<32 hex>` from `crate::ids::new_id("account")` (CORE `ids.rs:28`), made in the commit and carried
in the mutation (P2). Calendars: `cal-<32 hex>` made by the core when a list first shows the pair (account, Google
calendar id), kept for the life of the account record also while unlisted (W05 PD21, 8; A.5). An agent and the
page never see a Google calendar id (W05 8).

#### D.8.6 The sign-in watchdog

An account in `awaiting_consent` whose generation has had no outcome for `CONNECT_WATCHDOG_MS` (365 000:
`SIGN_IN_WAIT_MS` + two `PROVIDER_CALL_MS` for the token and key requests + `ORDER_WATCHDOG_MARGIN_MS`) is handled as
`Cancelled(timed_out)` in a commit `provider.connect`: generation + 1, so a late outcome is ignored, and
`Forget(account_id)` after it. Before that `Forget` the shell calls `Flow::supersede` for that flow when it is still
running or queued (D.2), so the `Forget` follows any write the flow made and no longer depends on 365 s being later
than its last possible write (DD18).

Tests of D.8's account link (step P7b; the shell and the adapter against FAKE in process, W5-9):
`a_disconnect_during_its_own_sign_in_ends_with_no_token_and_the_account_disconnected` (consent completed after the
disconnect); `a_reconnect_queued_behind_an_unlink_of_the_same_account_keeps_its_new_token`;
`a_quit_during_an_unlink_ends_revoked_at_restart_with_the_keychain_double_and_not_confirmed_with_the_memory_store`;
`a_stop_after_the_unlinks_forget_and_before_its_commit_records_not_confirmed_at_restart`;
`a_quit_during_a_sign_in_that_wrote_a_token_for_a_new_account_leaves_no_token_after_restart`;
`a_superseded_queued_connect_answers_cancelled_without_starting`. That the forget never runs before the revocation
has read the token is the adapter's own test (W5-15).

### D.9 Calendars (PO5, CO7, ST2)

#### D.9.1 When the broker lists

At a `Connected` outcome, at each start for every connected Google account, on `AccessChanged`, and on the owner's
`refresh_calendars` (P2-11; operation type `owner.refresh_calendars`, D.16); in each case only when `can_carry_out`
is true for the account. Each list is a system proof on
`ListCalendars { account_id }` and a `ListOrder`. That system action needs TH1 (P2-6): until it merges no list order
is built, and Google connection waits for it (OP6; REV-port PO5). The practice account gets its calendars from
the seed and is never listed.

#### D.9.2 The list commit (`provider.calendars`)

1. Each entry is matched by `provider_calendar_id` in `provider_calendars`. A new one gets `cal-<32 hex>` and
   `ProviderCalendarPut` revision 1; a known one is updated when its role, primary flag, zone or name changed. An
   unlisted known one gets `listed: false`. Rows are never deleted.
2. The account record's `calendars` is rebuilt: one `Calendar` per listed entry, access by W05 6.2's table
   (`writerWithoutPrivateAccess` as `reader` until P2-8), `selected` and `agent_drafts` kept as the owner set them; a
   new or returning calendar is not selected; `agent_drafts` is cleared on a calendar that is no longer writable. An
   unlisted calendar is left out of the record, so it leaves `TrustedFacts.calendars` (CORE
   `service/facts.rs:129-147`; REV-core CO7 a).
3. A reduction is: a selected calendar left out, a selected calendar whose access fell (`owner` > `writer` >
   `reader` > `free_busy_reader`), or Agent Drafts cleared. For each reduced calendar: every live grant whose scope
   names it ends `authority_reduced` (`GrantEndReason::AuthorityReduced`, CC `owner.rs:536-546`); every
   `pending_approval` request naming it goes `stale` (a calendar variant of `stale_pending_of_account`, CORE
   `service/sessions.rs:157-180`, reducer event `AccountGone`).
4. If the record changed in any way: account revision + 1, `revocation_epoch` + 1 once (W04 5.6), notices. If
   nothing changed: only `ProviderAccountPut` with `calendars_listed_at_ms`, no epoch, no account notice (REV-core
   CO7 d: a list at each start must not void every queued authorization).
5. If the rebuilt account record would pass `ACCOUNT_RESERVE_BYTES` in canonical form, the list fails as a whole and
   nothing changes (DD11).
6. A list of more than `MAX_CALENDARS_PER_ACCOUNT` (250) entries, or one that would give the account more than 250
   `provider_calendars` rows, fails as a whole and nothing changes. This is the core's check; the store has no
   guard on the count (A.2 R8, A.5).

A failed list changes nothing; the owner can ask again. The core does not retry a list on its own: the next start,
signal or command lists again.

#### D.9.3 `select_calendars` and Agent Drafts

One commit, `owner.select_calendars` (DRAFT README section 7). Built in step P5a: it needs only migration 2 and
works on the practice account. `not_found` for an unknown account or calendar;
`conflict` when `expected_revision` is not the account's; `invalid_params` when `agent_drafts` is not in `selected` or
its access is not `owner` or `writer`. Then:

- `selected` and `agent_drafts` set for every calendar of the record; account revision + 1.
- The draft bounds `selected` at 200 ids (DRAFT CC `owner.rs:1850`), below `MAX_CALENDARS_PER_ACCOUNT` (250); at most
  200 calendars of one account can be selected until the contract session raises the bound (CQ-D4).
- For each calendar no longer selected: its live grants end `authority_reduced`; its `pending_approval` requests go
  `stale`.
- For each calendar whose `agent_drafts` changed (named or no longer named): its `pending_approval` requests go
  `stale`. Its grants stay: a grant for a session or until revoked serves an `ask` row only where the matrix says so
  (DRAFT README section 10), and the next proof sees the new Agent Drafts.
- If anything changed: `revocation_epoch` + 1 once (P10 "naming Agent Drafts raises the revocation epoch").

It applies to the practice account as to a Google account.

#### D.9.4 The one rule and its test

Three lists in 0.5.3 hold the rule. `NARROWING_COMMANDS: &[OwnerMethod]` lists the owner commands that narrow
authority or change a fact a proof or a card reads: `pause_agent` (when it pauses; it raises `pause_epoch`, W04 5.6;
a resume raises nothing), `revoke_grant`, `save_profile` (an activation, or a new version of the active profile),
`save_group` with any of its changes, deletion included (it already raises, CORE
`owner/groups.rs:40-67`), `save_preferences` when `warning_calendars` change (DD10), `disconnect_google`,
`connect_google`, `select_calendars`, `remove_agent`. `NARROWING_COMMITS: &[&str]` lists the broker commits by
operation type: `provider.connect`, `provider.calendars` (when it changes the record), `provider.auth_lost`,
`broker.started` when an account changed (D.8.4); the `AuthLost` branch of every result commit is a case of its
own. `NOT_NARROWING: &[OwnerMethod]` lists every other owner method with one line of reason: `hello`,
`get_snapshot`, `subscribe`, `get_request_details`, `get_proof`, `list_ended_requests` and `get_agent_configuration`
write nothing; `add_agent` adds a pending agent that holds nothing; `approve_agent` and `decide_request` only widen;
`resolve_outcome` changes no fact a proof reads (its grant ends raise the epoch anyway, D.7.5). One core test,
`every_narrowing_commit_raises_an_epoch_of_the_fence`, first asserts that every value of `OwnerMethod::ALL` (20, DRAFT
CC `owner.rs`) is in exactly one of the two owner lists, so a command added later cannot be left out; then it runs
each narrowing entry and the `AuthLost` case from a store with a queued authorization and a proof in flight and
asserts that the fence of each changed and the authorization is voided at its handoff with `fence_changed`.
The test is built in step P5a with the partition of `OwnerMethod::ALL` and the entries built so far (DD10,
`select_calendars` and `disconnect_google` among them); step P6 adds `remove_agent`, step P7b `connect_google` and
`NARROWING_COMMITS` (E.2.1).

### D.10 The fence (DD5)

```rust
pub(crate) fn fence_now(counters: Counters, observed_revision: Option<String>) -> Fence;
pub(crate) fn fence_holds(proved: &Fence, now: &Fence) -> bool; // unchanged: proved == now
pub(crate) fn observed_revision(r: &ReadTx<'_>, request_id: &Id) -> Result<Option<String>, StoreFault>;
```

`observed_revision` is the `revision` of the request's latest `observe` row with outcome `found`, or `None`. It is
read in the transaction of each use: `proof_job` (the proved fence), `judge` (CORE `service/pipeline.rs:236`) and
`handoff` (`:471`). Between a proof and its handoff only a new observation of the same request can change it (a later
proof start, or CO12), and then the authorization must not be used. The handoff does not read Google again; `If-Match`
guards the gap (W05 RK9). `Fence` and its stored form are unchanged (`crates/fabric-storage/src/rows.rs:23-32`). The
unit test `a_fence_holds_only_while_every_version_is_unchanged` (CORE `service/epochs.rs:44-56`) keeps its asserts
with `fence_now`. This change lands before or with the first code that stores an observation.

### D.11 The account budget (DD11, ST10)

`owner_state_bytes` (CORE `owner/views.rs:73-77`) adds `accounts × ACCOUNT_RESERVE_BYTES`, counting every account the
snapshot carries (disconnected ones too: they stay in the snapshot). An account always counts at its reserve, not at
its size, so a later list never pushes the owner's records past the budget; D.9.2 step 5 keeps each account within
its reserve.

- `connect_google` that would create an account is refused `invalid_params` (`/params/account_id`, "Fabric cannot
  hold another account.") when `owner_state_bytes + ACCOUNT_RESERVE_BYTES > OWNER_STATE_BUDGET_BYTES`, and stores
  nothing. A reconnect or a reuse of the placeholder adds nothing and is never refused.
- Commands that grow recipient sets, profiles or grants now meet the accounts' reserve in the same check (CORE
  `owner/commands.rs:62-63`, `owner/profiles.rs:149`, `owner/groups.rs:105`).
- `ACCOUNT_RESERVE_BYTES = 98_304` (96 KiB): an account record with 250 calendars of about 130 bytes of members each
  plus names averaging up to about 260 bytes, the email and its `provider_accounts` snapshot entry (P2-9).
- What that means: with nothing else, 7 accounts fit the 720 KiB budget; with the debug practice account, one fewer.
  With one recipient set at the schema's maximum (about 660 KB, W04 6.1 text after the table), no account fits. The
  numbers go to the owner (D.14, OQ-D3).
- No command removes an account record, so an account's reserve is never given back. A disconnected account can be
  reconnected, and failed new connects reuse one placeholder (CO2).
- Test (limits): `a_connect_past_the_budget_is_refused_and_stores_nothing`; the existing
  `the_owner_records_are_bounded_so_a_snapshot_fits_in_one_frame` (W04 9.3) is rerun with seeded accounts.

### D.12 What the practice provider must answer (DD14)

The practice provider (BR `practice.rs`) is the port's second implementation (W05 10; P11). Section E owns its code
and when it ships; these are the answers it must give under D.2's shapes.

| Order or rule | Answer |
|---|---|
| any order | refuse when `proof`'s digest is not the order's (D.4); refuse a target that is not the practice calendar (today's row 3, `practice.rs:261-263`, now on `CalendarTarget`) |
| dispatch row 1, the bound `until` passed before acting (`practice.rs:252-255`; B.13) | `Transient { retry_after_ms: None }`, not `NoAnswer`: nothing changed (W05 6.10 row 1; REV-port PD17 (i)) |
| dispatch row 4, an effect applied before (`practice.rs:264-269`) | removed: under DD6 the core never sends again after `Applied` or `NoAnswer`; a resend meets row 7 as at Google |
| dispatch, update or delete of a missing event (`practice.rs:356-362`) | `Rejected { revision_conflict }` (W05 6.7, 6.8: 404 and 410), not `invalid_arguments` |
| dispatch, an update whose start would not be before its end | `Rejected { InvalidArguments, "the new start is not before the end" }`, as Google answers 400 (W05 6.9, 6.10); the core does not check it before (D.13 CO6) |
| dispatch, applied | create and update: `Applied { event: Some(EventRef), confirmed: true, notification: not_requested }`; delete: `Applied { event: None, confirmed: true, notification: not_requested }`, from its own locked state with no read. Today a delete answers `Some(reference)` (`practice.rs:297-300`), which B.14 would treat as a mismatch |
| fault `lose_answers` (applied, answer lost) | kept: `NoAnswer` after the change |
| new fault `unknown_not_applied` | `NoAnswer` with no change, so reconciliation's absence path runs |
| new fault `applied_differs` | the change is made; `Applied { event: Some(EventRef), confirmed: false, notification: not_requested }` for a create or an update, so V23's interim chain runs (D.7.1); never for a delete |
| a poisoned lock (`practice.rs:248-250`) | kept: `NoAnswer` (the state is unknown) |
| observe | `Found` from its state with its own `fabric_created` (the core overwrites it with DD12's value before storing, D.6.2), `before` built from the stored event, evidence the event's canonical JSON; `Absent` for a missing or deleted event |
| read | availability (each interval cut to the window) and event details over its one calendar, every field the action grants, with one `ReadSpan::Timed` for each event (it holds timed events only); `Stopped` when `still_allowed` is false |
| reconcile | `Found { revision, matches_approved, evidence }` by its own `matches` of W05 6.12 for a live event, `Cancelled { revision }` for a deleted one, `NotFound` when no event has the id (E.1.4's form, which matches Google's cancelled events) |
| list | its one calendar; never issued by the core for the practice account (D.9.1) |

Tests that change with these rows: `past_the_deadline_is_no_answer_and_no_change` (`practice.rs:794-806`),
`a_retry_of_an_applied_effect_is_applied_once` (`:650-664`), the last two asserts of
`a_lost_answer_applies_and_answers_no_answer` (`:820-822`) and the missing-event assert at `:676-680` (REV-port
"for_the_lead"), and the delete asserts at `:695-696` and `:890-891`, which expect an event for a delete. The
shell's observation hook goes (D.6.2).

`can_carry_out(attachments, account, provider_account)` (0.5.3, V17) replaces `Attachments.provider_present` in
`accepted()` (CORE `service/pipeline.rs:159-167`): a fresh permit authorizes only when the run's provider can serve
the request's account: `Practice` for the practice account (no `provider_accounts` row), `Google` for an account with
a `provider_accounts` row. Never both in one run (P11).

### D.13 The answers to the requests

| Id | Asked | Answer | Where |
|---|---|---|---|
| PO1 | a calendar target on every order, resolved by the core | Accepted as `CalendarTarget`, resolved in the commit that sends the order; an unresolvable pair after a held fence is a store fault | D.2, D.3 |
| PO2 | an observe order under `System_refresh` | Accepted. Stored (`observations`, `observation_evidence`) before the proof; the proof starts in that commit; the fence carries the stored revision; the shell hook goes | D.6, D.10 |
| PO3 | a read order, every page, a `still_allowed` flag | Accepted. Its own order with no effect row; the flag is an early stop, kept in `OrderInFlight` and cleared by `after_commit` after every commit that raises an epoch, before that step's jobs leave (B.13); the core checks authority again in the storing commit (section B, V6) | D.2, B.13 |
| PO4 | a reconcile order; the core applies `reconcile::decide` | Accepted with the review's form: the provider answers `ReadBack`; the core owns the table. GC's `reconcile::decide` becomes a test oracle | D.7.3 |
| PO5 | a calendar-list order under a system proof | Accepted; not built before TH1 (P2-6); Google connection waits for it | D.9.1 |
| PO6 | `Applied { confirmed, notification }`; account signals | Accepted. The core stores the event (`EffectEventRecorded`). An `Applied` that does not fit the order, or an unconfirmed delete, is an unknown outcome (V9); a fitting, unconfirmed `Applied` of a create, update, invitation or response is applied and waits for the owner (V23); a delete is confirmed by the adapter's one GET in its order (D.2), agreed with the CD-W05 session (W5-6). One optional signal on every answer, applied in the result's commit or alone (DD15). Ruled 2026-11-18 by the lead | D.2, D.7.1, D.8.2 |
| PO7 | sign-in on its own worker | Accepted as `AccountLink`. The core rechecks subject uniqueness and scopes in `provider.connect` and sends `Forget` on a mismatch. Two workers: sign-in, and unlink (revoke and forget as one job) for a disconnect and at start; a sign-in is superseded under the lock of its token write (DD18, ruled 2026-11-18 by the lead) | D.2, D.8 |
| PO8 | every order names its proof | Accepted as a type (`OrderProof`); the worker checks the digest; the FAKE log test | D.4 |
| CO1 | `connect_google` | Accepted with a connect generation, one epoch raise, the budget check | D.8.1 |
| CO2 | at most one never-connected account | Accepted: "never connected" is `provider_accounts.subject` null; reused in the same commit | D.8.1 |
| CO3 | recovery cancels `awaiting_consent` | Changed: a never-connected account is cancelled; a re-consent returns to its state before | D.8.4 |
| CO4 | `disconnect_google` with revoke, forget, purge, ends of open effects | Accepted with `outcome_unknown -> reconciling -> needs_owner_resolution` (no direct edge) and the rule for an answer that arrives after it; `applied -> reconciled` is not needed (DD8); revoke and forget are one `unlink` job, repeated at start while the revocation is pending (DD18, ruled 2026-11-18) | D.8.3, D.8.4 |
| CO6 | observation before each proof; its table | Accepted on existing edges, with the stored observation, the fence of DD5, `observation_failed` through `undecide()`; the start-before-end check of W05 CO6 is not built in the core: Google refuses it at dispatch (W05 6.9, 6.10), after a consumed authorization | D.6 |
| CO7 | calendar list; a reduction ends grants and stales cards | Accepted: the account record is rebuilt so a lost calendar leaves the proof facts; a list that changes nothing raises no epoch | D.9.2 |
| CO8 | reconciliation | Accepted with durable read counts (`observations`), due times through recovery, no read after `applied`, waits while the account is not connected | D.7 |
| CO10 | configuration of the client file and of FAKE's loopback port | Accepted as the binary's, an amendment of W04 D27 for the owner: `FABRIC_BROKER_GOOGLE_CLIENT_FILE` in every build; `FABRIC_BROKER_GOOGLE_LOOPBACK_PORT` compiled only under `cfg(debug_assertions)` and accepted only with `FABRIC_BROKER_KEY_SOURCE=test`; a configuration with a Google provider and the practice mode is refused (P11). The token store follows the key source (W05 4). The binary's wiring is section E's (step P7b, E.2.1) | D.13 |
| CO11 | `AuthLost` | Accepted; the epoch rise is in the commit of the state change; `client_rejected` kept in `provider_accounts.auth_lost` until P2-9 shows it | D.8.2 |
| CO12 | re-observe a waiting card | Deferred. When built: an observe scheduled by the pump every `CARD_REOBSERVE_MS` per waiting card (never from `get_request_details`, which must not write), under a system proof; another revision moves `pending_approval -> stale` (event `ObservedStale`). The fresh-permit observation of D.6 already catches a stale approval before any dispatch | D.6 |
| CO14 (rows) | reducer rows | `ReconcileDue` (`outcome_unknown -> reconciling`), `ReconcileVerdict(Verdict)` (`reconciling -> applied \| absence_confirmed \| needs_owner_resolution`), `Applied` from `reconciling` and `applied -> reconciled`, `ObservedStale`, `ObservedInvalid`, `AccountGone` reused for a calendar; `DRIVEN` and the left-over test of CORE `service/reducer.rs:121-217` updated | D.6.3, D.7.3 |
| ST6 | "Fabric created this" | DD12 | D.0 |
| ST7 | `applied` in `open_effects` | Not built (DD8); agreed with the CD-W05 session 2026-11-18 (W5-6); a delete is confirmed by the adapter's read in the order (D.2) | D.7.1 |
| ST10 | the snapshot budget | DD11 | D.11 |

### D.14 Questions

For the contract session (theory owner):

| # | Question | Default the design runs on |
|---|---|---|
| CQ-D1 | What the theory calls a pending effect. THEORY 206-207 says "handed to Google and not yet seen applied". The store gives `dispatching`, `outcome_unknown` and `reconciling` (`read.rs:904-918`) and is not widened (P10). Schema version 3 makes `needs_owner_resolution` open: it fits the text but is not a pending effect, so no `System_reconcile` is proved for it. Please confirm the text as "in `dispatching`, `outcome_unknown` or `reconciling`", or say what else belongs | the store's three states; no read after `applied` (V7) |
| CQ-D2 | System refresh. `System_refresh acct cal` (THEORY 264, 520-521) names no event, so one proof permits a read of any event of the calendar while the worker reads one. Should it name the event, as `System_reconcile` does? And is the observe that precedes an agent's classification the broker's own action (`ctx_actor = System`), although an agent's request caused it? | `Refresh { calendar }` as THEORY has it; the observe is a system action |
| CQ-D3 | TH1 (P2-6): `System_list acct`, authorized when the account is connected | no list, no Google connection, until it merges |
| CQ-D4 | `params_select_calendars.selected` allows 200 ids (DRAFT CC `owner.rs:1850`), below `MAX_CALENDARS_PER_ACCOUNT` (250, W05 PD23). Please let it allow 250 ids | at most 200 calendars of one account can be selected (D.9.3) |

For the owner (P13):

| # | Question | Default | If he chooses otherwise |
|---|---|---|---|
| OQ-D1 | The residual risk of a duplicate after a create whose outcome was unknown: Google may finish a POST after two reads 60 s apart found nothing, and the agent's new request makes a second event (W05 RK3; REV-core rule PD20). DD12 marks a late event under the old id as Fabric's, so it can be seen and removed. For information beside it, not asked: a delete applied and then restored at a new revision before the first reconcile read ends `absence_confirmed` (D.7.3); after a confirmed delete this cannot happen | accept the residual | a longer wait between the reads (costs time on every unknown create), or no automatic `absence_confirmed` for creates: every unknown create ends `needs_owner_resolution` and he answers each |
| OQ-D2 | Whether the broker's own reads are proven (W05 OQ9, PD29) | yes: observe, reconcile and list each run under an Isabelle system proof (about two seconds more per check, W05 RK13); no account can be set up or reconciled while the prover is not ready | no system proofs: the reads run on the core's own checks; TH1 is not needed; "no Calendar call without a proof" no longer holds |
| OQ-D3 | The cap on accounts (DD11) | the budget alone: at most 7 accounts with no other records, fewer as recipient sets grow, none beside a recipient set at the schema's maximum | a fixed count beside the budget, or a smaller reserve with a lower calendar limit per account |

### D.15 What this amends

These go to the owner as E.3 OD20 (D27 as OD12).

| Document | Item | New wording |
|---|---|---|
| W04 | 5.6, the fence | "`Fence { pause_epoch, revocation_epoch, event_revision }`, where `event_revision` is the revision of the request's latest stored observation of the event its action names, is read with the facts of each proof and again at acceptance and at dispatch from the store. A proof or an authorization holds only while the current fence equals it." |
| W04 | 5.6, epoch raises | adds: "and once in every commit that changes an account's record by a sign-in outcome, a calendar list or `select_calendars`, or changes the warning calendars" |
| W04 | 5.7, `dispatching` | "`Applied` with `confirmed: true` moves `dispatching -> applied -> reconciled` in one commit and records the event; a delete is confirmed by one read of the event inside its own order. A fitting `Applied` of a create, update, invitation or response that is not confirmed records the event and waits for the owner in `needs_owner_resolution`; it is never sent again or read back. `NoAnswer`, an `Applied` that does not fit, an unconfirmed delete and the order watchdog move it to `outcome_unknown`, due in 10 s for reconciliation (D.7)." |
| W04 | 5.8, recovery | adds the due times of D.7.4 and the account rules of D.8.4 |
| W04 | D38 and R24 | "Accounts count at `ACCOUNT_RESERVE_BYTES` each; a `connect_google` that would pass the budget is refused `invalid_params`." |
| W04 | D27 | the two names of CO10 (D.13) |
| PW | 4.3 rows 1 and 4, 4.4 | D.12 |

### D.16 Names added or corrected

| Name | Change | Why |
|---|---|---|
| `ConnectOutcome` | a struct `{ account_id, generation, result: ConnectResult }`, with `ConnectResult` the W05 3.2.2 variants (OP15 asked D to write them) | putting `account_id` and `generation` in every variant repeats them five times |
| `system_action_digest` | `pub`, not `pub(crate)` | the workers in other crates check it (DD2) |
| `NARROWING_COMMANDS`, `NARROWING_COMMITS`, `NOT_NARROWING` | the owner list also has `save_group`, `save_preferences` (when warning calendars change), `connect_google`; the broker commits are a list of operation types of their own, with `broker.started` when an account changed and the `AuthLost` branch of result commits; every other owner method is listed as not narrowing, with its reason | DD10; P10 "changes the facts a proof or a card reads"; one test for all (D.9.4) |
| `Resolved(Resolution)`, D.7.5 | new reducer event and the design of `resolve_outcome` | DRAFT README section 7; requests in `needs_owner_resolution` exist from step P5a |
| `ACCOUNT_RESERVE_BYTES` | new, 98 304 | DD11 |
| `CONNECT_WATCHDOG_MS` | new, 365 000 | D.8.6; P10 "every order has a deadline in the core" applied to sign-in |
| `owner.refresh_calendars` | new operation type, since 3, for P2-11 | a command commits and returns `commit_sequence` (DRAFT README section 7) |
| `observed_revision` | new core reader feeding `fence_now` | D.10 |
| `resolve_target` | new core function | D.3 |
| `unknown_not_applied` | new practice fault | D.12 |
| `applied_differs` | new practice fault | V23; D.12, D.7.1 |
| `provider_differs` | new audit kind | V23; B.14 |
| `MAX_BEFORE_VIEW_BYTES` | new, 524 288 | D.6.2; owner OD26 |
| `MAX_EVIDENCE_BYTES` | 1 048 576, was 262 144 | D.6.2; owner OD26 |
| `ReadSpan` | new port type, `Timed { start_ms, end_ms }` or `AllDay { start_date, end_date }`; replaces `spans: Vec<(i64, i64)>` | DD17; B.3.4 step 2.1 |
| `ZONE_SLACK_MS` | new, 50 400 000 (14 h) | DD17; B.3.4 step 2.1 |
| `Job::Unlink`, `Input::Unlinked`, `AccountLink::unlink` | replace `Job::Revoke`, `Input::Revoked` and `AccountLink::revoke`; `unlink` takes whether to revoke | DD18; D.8.3 |
| `unlink_in_flight` | new in-memory set of the core: the accounts whose unlink has not answered; no `Connect` is sent for them | DD18; D.8.1, D.8.3 |

---------------------------------------------------------------------------------------------------------------------

## E. The practice mode, the order of work, and the decisions for the owner

**What this section is.** Three parts. E.1 designs the practice mode for a real agent (ruling P11) and answers
CD-W06's request LR-B6. E.2 orders the work into pull requests (ruling P12). E.3 collects every decision the owner
must take for this phase (ruling P13), one row each. E.4 and E.5 give the risks of the whole plan and what was not
checked. The names, numbers and shapes are those of section 0 and of the sections that 0.3 names; where this section
needs a name of its own, E.6 lists it.

Code is cited at main `2d41210` with the short names of 0.1 (CORE, ST, BR, CC). W04 and PW are cited by
decision id and section, not by line, because the broker part of schema version 2 (step P0) edits both documents
and moves their lines (`git diff --stat origin/main...claude/cd-broker-prover-reasons`: 118 lines of W04 and 315 of
PW). "PW practice rule P1" means the practice rule P1 of PW 4.1; "PW decision P2" means row P2 of PW section 0.

---------------------------------------------------------------------------------------------------------------------

### E.0 Decisions of this section

| Id | Decision | Why |
|---|---|---|
| ED1 | Practice is on exactly when the build is a debug build, the key source is `test`, Isabelle is configured and no Google provider is configured. `FABRIC_BROKER_SYNTHETIC_AGENT=1` only adds the synthetic script on top. No new variable | P11. Practice rules P1 and P2 draw their protection from debug-only code and the test key, never from the synthetic agent: the host passes the test key only with an explicit `FABRIC_DESKTOP_DATA_DIR` (`apps/fabric-desktop/src-tauri/src/host.rs:212-222`). A fourth variable adds refusals and no guard (PW X2's argument) |
| ED2 | A debug configuration with the test key, Isabelle and a Google provider is refused: the broker exits 78 with one stderr line, and the debug host refuses it first with "nothing was started". Built in step P7b, in the pull request that adds the Google variables | P11 "a configuration with both is refused". Without it one run would hold the practice account and a Google account, both `connected`, with one provider slot (BR `shell.rs:270-274`), and the label "nothing real changes" would be false |
| ED3 | The shell seeds the practice account through `Broker::seed` with an account-only seed, in the first turn in which the store is ready: before the turn's input is routed and again after it, before the synthetic agent's tick. The calendar is named "Practice calendar (test build)" in that seed only; `SyntheticAgent::seed()` is unchanged | P11 "the shell seeds the practice account". A real agent must find the account without the script. Seeding twice in the turn closes the case where the input that reopens the store is the `subscribe` that starts the script (W04 D9), so the practice name always wins. The synthetic run without Isabelle keeps its own name (J-store practice minor) |
| ED4 | The practice provider implements the whole `Provider` trait of D.2 (dispatch, observe, read, reconcile, list) in step P4. The shell's observation hook (BR `shell.rs:384-392`) goes in step P5, in the pull request in which the core first sends `ObserveOrder` | P11: the extended port before a real agent uses it (step P6). Removing the hook before the core observes would leave every practice update and delete without `facts.event`, so the theory would block them (MODEL `event_known`; PW X6) |
| ED5 | The provider-neutral core of CO6 (observe before a proof, the CO6 table) and of CO8 (reconcile after an unknown outcome, the PD20 rule) is built in step P5, with the fence fix of P10 and the order watchdogs (V20). Step P7b adds only what is Google's | V1 puts `observations` in migration 2 for this reason; P10 "Google is not attached before reconciliation exists in the core"; REV-port cross-cutting "fence equality": setting `event_revision` without the fix discards every observed proof. Building them on the practice provider tests them before Google exists |
| ED6 | The practice provider's dispatch table changes for P10 (E.1.4): a passed deadline answers `Transient`; an update or delete whose `send_updates` is not `none` is refused; a new fault answers `NoAnswer` with no change; `Applied` carries `confirmed: true` and `notification: not_requested`. Row 4 (a retry of an applied effect answers the same `EventRef`) is removed, as D.12 says: a resend meets row 7 and is refused, as at Google (X6) | P10: `Transient` means "certainly did not happen". Practice events have no attendees and the practice calendar notifies nobody, so a change that asks to notify would report a false outcome. Under DD6 the core never sends a change again; with row 4 a resend by mistake would look like success, without it the resend is refused. Neither form repeats a change |
| ED7 | For an account with no `provider_accounts` row in a practice run the core resolves the order's `CalendarTarget` as `{calendar, provider_calendar_id: the broker's calendar id, time_zone: "Etc/UTC", is_primary: true}`, and `can_carry_out(attachments, account, false)` is true only when `attachments.provider` is `Practice`. The practice provider still refuses every pair but (`synthetic-account`, `synthetic-calendar`) | V17 and P10 (one target on every order, resolved by the core). The practice account is the only account a store can hold without a `provider_accounts` row once `connect_google` exists (V16), and practice row 3 is the second guard |
| ED8 | `ProviderKind::Practice` exists only under `cfg(debug_assertions)`; so do `practice.rs`, the practice seed, `Plan.practice` and the label | A release build cannot name the variant, so no release code path can attach it (W04 D11) |
| ED9 | Labels: the `hello` build label ends ` practice` (` synthetic practice` with the script); `main.rs` writes one fixed stderr line; the practice calendar's name; the agent's `agent_prove` answer carries `practice: true` (P2-19, asked, no longer optional) and the bridge says so to the model; the page's practice line shows only in a practice run | P11 "labelled as practice for the owner and for the agent". Without P2-19 an agent sees practice only after its first list, and an `applied` change looks like a real effect (J-auth practice minor) |
| ED10 | In a debug build with the test key the broker creates `<data_dir>/bridge-keys` (mode 0700, non-recursive, after the store is open and its lock held) and the configuration text of `get_agent_configuration` carries `--socket <data_dir>/run/agent.sock` and `--key-dir <data_dir>/bridge-keys` | The debug bridge requires an existing 0700 key directory (W06 5.4) and W06 section 6 leaves its place open. Inside the scratch data directory it goes away with the run and never touches the Keychain |
| ED11 | A practice run is never evidence for gate G2b or G2c. A G2c report names the provider of each effect it cites; a practice effect is listed only under "wiring" | PLAN 15.2 NOV17-PRACTICE; PW practice rule P7 |
| ED12 | The amendments of E.1.9 are applied to W04 and PW in the pull request that builds them (P5 for the hook and W04 D14, P6 for the rest), after the owner's answer (OD9, OD18), never before | The standing rule: spec changes are shown before they are applied. P0 edits both documents first |

---------------------------------------------------------------------------------------------------------------------

### E.1 The practice mode for a real agent (P11)

#### E.1.1 The predicate

Today: `Config::practice()` is `cfg!(debug_assertions) && self.synthetic_agent && self.isabelle.is_some()`
(BR `config.rs:82-87`). The only way in for a real agent is blocked by `self.synthetic_agent`.

New, in step P6 (`apps/fabric-broker/src/config.rs`):

```rust
impl Config {
    /// True when this run uses the practice calendar: a debug build, the test key, a configured
    /// Isabelle and no Google provider (ED1). The synthetic agent is not needed (LR-B6).
    #[cfg(debug_assertions)]
    #[must_use]
    pub fn practice(&self) -> bool {
        self.key_source == KeySource::Test && self.isabelle.is_some() /* && self.google.is_none(), step P7b */
    }
    #[cfg(not(debug_assertions))]
    #[must_use]
    pub fn practice(&self) -> bool { false }
}
```

`KeySource::Test` exists only in debug builds (BR `config.rs:52-54`), so the release body cannot name it; it is the
constant `false`. Step P7b adds `Config.google: Option<GoogleSettings>` (section D names the CO10 variables and the
type) and the third term, and in `config_from` the refusal of ED2:

> `fabric-broker: <the Google variable> is set together with FABRIC_BROKER_KEY_SOURCE=test and the rule prover; that is a practice run, which has no Google provider`

The refusal is checked after the six present names are read and before stdin or any thread (PW 3.1). It never
prints a value other than a path. Nothing else in `config_from` changes: `synthetic_from` still needs the test key
in debug and refuses `1` in release (BR `config.rs:334-363`); the test key is refused in release (BR
`config.rs:303-332`).

ED2 stays literal (the lead's ruling of 2026-11-17, I.6 E-1): CD-W05's tests against FAKE run in process (W5-9).

Until P2-19 (`practice: true` in the prove result) and the bridge's practice text (W6-2) are both merged, row 4 of
E.1.2 is not switched on: the test key and Isabelle without the synthetic agent stay row 3 (no provider). P11 asks
that the run be labelled for the agent, and those two are the only label that reaches the agent before its first
list (ED9).

#### E.1.2 The configurations

`Plan::of` (BR `shell.rs:104-150`) and `Plan::attachments` (BR `shell.rs:152-168`) become this table.
`Attachments.provider_present: bool` becomes `Attachments.provider: Option<ProviderKind>` (0.5.3) in step
P4. Rows marked P7b exist from that step.

| # | Build | Key | Isabelle | Synthetic | Google | Prover | Provider | Script | Label suffix | Status |
|---|---|---|---|---|---|---|---|---|---|---|
| 1 | any | any | no | no | no | none | none | none | (none) | every call that would create a request is refused `prover_unavailable` (B.10); a request left open by an earlier run ends `undecided` (W04 D14) |
| 2 | debug | test | no | yes | no | synthetic classifier | none | `SyntheticAgent::new` | ` synthetic` | unchanged (PW O5) |
| 3 | any | keychain | yes | refused | no | Isabelle | none | none | (none) | a fresh permit of a change or a read ends `undecided` (no provider); a list and a release need no provider and complete (B.6.1) |
| 4 | debug | test | yes | no | no | Isabelle | `Practice` | none | ` practice` | **new**: practice for a real agent |
| 5 | debug | test | yes | yes | no | Isabelle | `Practice` | `SyntheticAgent::practice` | ` synthetic practice` | unchanged but for the seed (ED3) and, from step P5a, the script's test of step A (E.1.3) |
| 6 | debug | test | yes | any | yes | (refused, exit 78) | | | | **new**, P7b (ED2) |
| 7 | any | keychain | yes | refused | yes | Isabelle | `Google` | none | (none) | P7b, section D |
| 8 | debug | test | no | any | yes | none or classifier | section D | as rows 1, 2 | as rows 1, 2 | P7b; accepted (D.13, X17): the client file in every build, the loopback port only in debug with the test key; with no Isabelle nothing is permitted |

Row 3 was "Isabelle, no synthetic agent" in PW 3.3 and covered the test key too. With the test key it is now row
4, so the "Isabelle, no provider" path stays reachable through a debug Keychain store (PW X16) and in core tests
built with `Attachments { provider: None, .. }` (J-store practice major: keep that in-process test).

What the host, the relay and the harness accept and refuse:

| Part | Accepts | Refuses or ignores | Change |
|---|---|---|---|
| Host, debug (`host.rs` `BrokerStore::from_environment`, 114-148) | `FABRIC_DESKTOP_DATA_DIR` absolute, `FABRIC_BROKER_KEY_SOURCE=test`, the three Isabelle paths absolute, no synthetic variable; passes them in `variables()` (250-275) | the test key or the synthetic agent without `FABRIC_DESKTOP_DATA_DIR` (212-222); the synthetic agent without the test key (226-233); one or two Isabelle paths or a relative one (296-310) | none for practice. P7b: forwards the Google names (section D) and refuses the test key with all three Isabelle paths and a Google name, "the test key and the rule prover make a practice run, which has no Google provider; nothing was started" (ED2) |
| Host, release (161-171, 290-292) | nothing of the six names; the Keychain store at `app_data_dir` | ignores all six names | none: practice is impossible behind a release host |
| Relay (`relay/src/config.rs:40-70`, `ENVIRONMENT_ALLOWLIST`) | the six names | every other name is cleared | none for practice: the socket is under `FABRIC_BROKER_DATA_DIR` (W06 LR-B4). P7b adds the Google names (REV-order S16-5) |
| Harness (`harness/serve.ts:67`) | always the test key and `FABRIC_BROKER_SYNTHETIC_AGENT=1`; the three Isabelle names when all are set | one or two Isabelle names (exit 64) | none: a harness run with Isabelle is row 5. A switch to leave the script out is an optional request to the harness's owner |
| `scripts/verify-desktop.sh --packaged` (176-181) | | unsets the three Isabelle names in its launches | none: a packaged check is never a practice run |

#### E.1.3 The seeded practice account

`apps/fabric-broker/src/practice.rs` (debug only, `lib.rs:38`) gains:

```rust
pub const PRACTICE_CALENDAR_NAME: &str = "Practice calendar (test build)";
/// The practice account: `synthetic-account`, connected, email `synthetic@example.invalid`, one calendar
/// `synthetic-calendar` named PRACTICE_CALENDAR_NAME, access owner, selected, not Agent Drafts.
pub fn account() -> Account;
/// The shell's seed of a practice run: the account and no agent.
pub fn seed() -> Seed { Seed { agents: vec![], accounts: vec![account()] } }
```

The record is today's (`synthetic.rs:347-361`) with the calendar's name changed. `SyntheticAgent::seed()` stays as
it is, with "Synthetic calendar (test build)" (ED3). `Seed` has separate lists and the core skips records that exist
(CORE `broker.rs:87-106`; `service/agent.rs:332-382`), so an account-only seed is valid and a repeated one writes
nothing.

The shell (`apps/fabric-broker/src/shell.rs`) gains a debug field `practice_seeded: bool` and a step
`seed_practice(broker, now_ms)`: when `self.practice.is_some() && !self.practice_seeded && broker.store_ready()`, it
calls `broker.seed(&practice::seed(), now_ms)`, emits the output, and sets `practice_seeded` only when the output has
no halt. `Shell::turn` (BR `shell.rs:415`) calls it first, and again after the input is routed and before the
agent's tick. A store that is unavailable at start is seeded after its reopen; until then the flag stays false.

What it writes: one `broker.seed` ledger row with one `AccountPut` and one raise of `revocation_epoch` (CORE
`service/agent.rs:376-378`), in the first turn of a new store, before any grant exists, so the raise ends nothing.
A reused scratch store already has the account and nothing is written. The synthetic agent's later seed adds only
its agent. No migration, no new operation type, no new mutation (the existing `broker.seed`).

The synthetic agent, when it runs, is still seeded by its own `tick` after `subscribe` (BR `synthetic.rs:391-406`).
From step P5b its seed also gives it the list grant through `create_enrollment_grant`, since a seeded agent counts as
approved (V13, P5); C.2.1 says how a store written before migration 2 gets it once.

From step P5a a confirmed change ends `applied -> reconciled` in the attempt's commit (V7), so the script never sees
`applied`. `Practice::a_event` (BR `synthetic.rs:232-242`) therefore accepts `reconciled` as well as `applied` for
step A; otherwise the script skips C and D. The script's unit tests (BR `synthetic.rs:871-913`, `:948-995`) and the
live test `the_practice_script_is_asked_approved_permitted_applied_and_blocked`
(`apps/fabric-broker/tests/live.rs:713-800`, which waits for `RequestState::Applied` at `:735`, `:791` and `:800`)
change in the same pull request.

The practice calendar's events (the seeded foreign event `practice-foreign-1`, BR `practice.rs:32-34`) stay in
memory as today (PW X3, 4.2).

#### E.1.4 The practice provider on the extended port

`PracticeProvider` (BR `practice.rs:430`) implements `Provider` of D.2. Every method counts one call in
the existing counter, takes the lock (a poisoned lock answers as the "lock" column says), and checks rows in order;
the first failing row answers.

Common rows, every order:

| # | Check | Answer when it fails |
|---|---|---|
| C1 | `Instant::now() < until`, the shell's monotonic stamp plus `PROVIDER_CALL_MS` (B.13, X23) | dispatch: `Transient { retry_after_ms: None }` (ED6); observe, read, reconcile, list: `Failed(ReadFailure::Transient { retry_after_ms: None })`. Nothing changes |
| C2 | the order's proof is of the right kind: `OrderProof::Agent(authorization)` for dispatch and read, with today's `holds_together` checks (authorization's `request_id` and `action_digest` are the order's, `consumed_at` set; BR `practice.rs:372`); `OrderProof::System { proof_id, action_digest }` for observe, reconcile and list, with `action_digest` equal to `system_action_digest` of the system action the order stands for (PO8) | `Rejected { ProviderRejected, "the order does not hold together" }` (dispatch); `Failed(ReadFailure::Rejected { code: ProviderRejected })` (others) |
| C3 | every target is (`synthetic-account`, `synthetic-calendar`) with `provider_calendar_id` `synthetic-calendar` | as C2, with "the practice calendar has only the synthetic calendar" for a dispatch |

Dispatch (`dispatch(&DispatchOrder) -> AttemptDone`), after C1 to C3; today's rows 4 to 9 (BR `practice.rs:264-300`;
PW 4.3) with these changes:

| # | Check | Answer |
|---|---|---|
| 4 | removed (D.12, X6): an order for an effect applied before is not answered by a row of its own | it meets row 7 and is refused as at Google; the core never sends one (DD6) |
| 5 | the tool is create, update or delete | else `Rejected { ProviderRejected, "the practice calendar carries out create, update and delete only" }` (invitation and response end `terminal_failure`) |
| 5a | **new**: an update or delete has `send_updates: none` | else `Rejected { ProviderRejected, "the practice calendar notifies nobody; propose with send_updates none" }` |
| 6 | fault `transient > 0` | `Transient { retry_after_ms: None }`, no change |
| 6a | **new** fault `unknown_not_applied > 0` (decremented) | `NoAnswer`, no change (shows the path to `absence_confirmed`) |
| 7 | create: no event has the reserved id; update or delete: the event exists, is not deleted, and the action's revision is current; update: the new start is before the new end (a start or an end the update leaves out is the event's own) | else `Rejected { ProviderRejected, "an event with this id exists" }`, `Rejected { RevisionConflict, "no such event" }` (W05 6.7, 6.8; D.12, X7), `Rejected { RevisionConflict, "the event changed" }` or `Rejected { InvalidArguments, "the new start is not before the end" }` (W05 6.9, 6.10; D.12); then the change as today |
| 8 | fault `lose_answers > 0` | the change is kept, `NoAnswer` (shows the path to `reconciling`) |
| 8a | **new** fault `applied_differs > 0` (decremented), create or update only | the change is kept, `Applied { event: Some(EventRef { event_id, revision }), confirmed: false, notification: NotRequested }` (shows V23's interim chain to `needs_owner_resolution`, D.7.1) |
| 9 | | create and update: `Applied { event: Some(EventRef { event_id, revision }), confirmed: true, notification: NotRequested }`; delete: `Applied { event: None, confirmed: true, notification: NotRequested }`, from its own locked state with no read (D.12) |

The poisoned lock answers `NoAnswer` (a panic may have left a change half made). A missing target answers
`revision_conflict`, as D.12 fixes it (REV-port for_the_lead). Faults stay in-process only (PW X4): `Plan::of` always
passes `Faults::default()`.

Observe (`observe(&ObserveOrder) -> ObserveDone`), after C1 to C3: the named event exists and is not deleted:
`Found { event: ObservedEvent, before: EventView, evidence }`, where `ObservedEvent` is what
`PracticeCalendar::observation` builds today (BR `practice.rs:187`; PW 4.4: current revision, `fabric_created`,
timed, single, default type, no invitation, no attendees); `before` holds the title, start and end and every other
member null; `evidence` is the canonical JSON of the practice event, at most `MAX_EVIDENCE_BYTES`. Otherwise
`Absent`. `order.fabric_created` is not trusted; the provider reports its own flag, which the core overwrites with
DD12's value before storing (D.6.2).

Read (`read(&ReadOrder) -> ReadDone`), after C1 to C3: if `still_allowed` is false, `Stopped`. Otherwise one "page":
for `fabric_calendar_availability`, `Done { result: ToolResult::Availability { observed_at: now, busy }, spans: [] }`
with one interval per live event that overlaps the window, each interval cut to the window; for
`fabric_calendar_read`, `Done { result: ToolResult::Events { observed_at: now, events }, spans }` with one `EventView`
per live overlapping event, holding only the fields the action names (the filter of W05 6.4, section D), and its
span, always `ReadSpan::Timed` (the practice calendar holds timed events only; no change in behaviour).
`still_allowed` is read again after the result is built; false gives `Stopped`. The core checks
authority again in the commit that stores the result (P10, V6).

Reconcile (`reconcile(&ReconcileOrder) -> ReconcileDone`), after C1 to C3: event live: `Found { revision,
matches_approved, evidence }`, where `matches_approved` compares title, start and end with the approved create or
update; event deleted: `Cancelled { revision }`; no such event: `NotFound`.

List (`list(&ListOrder) -> ListDone`), after C1 and C2, for `synthetic-account` only: `Listed` with one entry
(`provider_calendar_id: "synthetic-calendar"`, name `PRACTICE_CALENDAR_NAME`, `access_role: "owner"`, `is_primary:
true`, `time_zone: "Etc/UTC"`). It is never issued on practice: the practice account gets its calendars from the seed,
and no list order is sent before TH1 merges (OP6).

What it never does: read or write a file, open a socket, act on another account, or carry out an order whose proof
check fails. It is reached only through `Job::Dispatch`, `Job::Observe`, `Job::Read`, `Job::Reconcile` and
`Job::List`, which the core makes only after a consumed authorization resting on an accepted Isabelle permit
(dispatch, read) or an accepted Isabelle system proof (observe, reconcile, list; V8).

The hook goes (ED4): `Shell::prove` (BR `shell.rs:384-392`) hands the job to the prover unchanged, the shell's
`practice` field and `PracticeCalendar::observation` lose their only caller outside the provider, and
`facts.event` comes only from a stored observation (P10).

#### E.1.5 What each tool does and returns on the practice calendar

From step P6, with steps P5 and P6 built. A request names `synthetic-account` and `synthetic-calendar`; any other
pair fails the core's world checks (W04 5.7) or, if it ever reached an order, C3. While the prover is not `ready`
every call that would create a request is refused `prover_unavailable` and creates none (P8).

| Tool | Path | What the agent gets |
|---|---|---|
| `fabric_calendar_list` | classification under the enrollment grant (P5); its own fresh permit is its release (P6); `ResultReleased`, `RequestMoved` to `completed`; never an order | in the call's answer: `Calendars { accounts: [{account_id: "synthetic-account", calendars: [{calendar_id: "synthetic-calendar", name: "Practice calendar (test build)", access: "owner", agent_drafts: false}]}] }`, built from the broker's records (`list_result`) |
| `fabric_calendar_availability` | classification, card or grant, fresh permit, read order, `HeldResultStored`, `dispatching -> applied -> reconciled` (V7 `ReadStored`) | the read's answer: status `reconciled`, `result: null`. Then `fabric_request_result` on it: a `Disclose` proof; release; `Availability` with the practice events' intervals; the held result is purged (V4) |
| `fabric_calendar_read` | as availability | as availability, `Events` with the fields asked |
| `fabric_calendar_propose_create` | card, approval, fresh permit, dispatch, row 9, `EffectEventRecorded`, `dispatching -> applied -> reconciled` (confirmed, V7, V9) | status `reconciled`. `fabric_request_result` on it: `Effect { event: Some(EventRef { event_id: the reserved id, revision: "1" }) }` (released with no grant, P6) |
| `fabric_calendar_propose_update` | observe under `System_refresh` (system proof), `ObservationRecorded`, classification with `facts.event`, card, fresh permit (observed again, CO6), dispatch | status `reconciled`; result `Effect { event: Some(the new EventRef) }`. With `send_updates` other than `none`: `terminal_failure`, `provider_rejected` (row 5a) |
| `fabric_calendar_propose_delete` | as update; a delete always asks (theory `mode_of`); a delete of `practice-foreign-1` is `blocked` by `fabric_created` | status `reconciled`; result `Effect { event: None }`. Blocked: the denial with its rule |
| `fabric_calendar_propose_invitation`, `..._response` | whatever the theory decides; if authorized, dispatched and refused by row 5 | `terminal_failure`, `provider_rejected`. On purpose: an invitation faked in memory would look like evidence for a Google-only row |
| `fabric_request_status` | answered at once, never content (P6) | state only |
| `fabric_request_result` | a second release of the same read: the held result was purged at the first (V4), so validation fails (VC4) | `terminal_failure`, `invalid_arguments`, "No result is held for that request."; nothing released |

The agent learns event ids from its own changes' results; `practice-foreign-1` (revision `"1"`) is in the README for
the block. Practice requests end `reconciled`, so they no longer stay open and count toward the 25 open requests per
agent (PW 5's "about eight runs" goes).

#### E.1.6 Labels

For the owner:
- `build_label` (BR `shell.rs:202-213`): ` synthetic practice` when practice and the synthetic agent; ` practice`
  when practice alone (new); ` synthetic` when only the synthetic agent; nothing otherwise. The page's build line
  shows it.
- One fixed stderr line from `main.rs` after `Plan::of` when `plan.practice.is_some()`: `fabric-broker: practice run:
  approved changes go to a practice calendar in memory; nothing real changes`. No id, no title.
- The account shows `synthetic@example.invalid` and the calendar "Practice calendar (test build)".
- Request to the screens session: show `#practice-line` (`apps/fabric-desktop/ui/index.html:15`) only when the
  `hello` build ends with ` practice` (PW 9.4 item 1: today it shows in every build, release included). Optional
  contract request P2-24 (section G) would replace the string test by a capability.

For the agent:
- P2-19 (asked, ED9): `result_agent_prove` gains `practice: bool`, true exactly in a practice run, the constant false
  in a release build. Request to the CD-W06 session: when it is true, the bridge's fixed first text block of every
  tool result (W06 A8, A27) begins "Practice run: changes go to a practice calendar in memory; nothing real changes."
- The calendar's name in every list result. A list result is data in the second text block (W06 A27), so the name
  cannot act as an instruction.
- The debug configuration text's first line says it is a test configuration (W06 section 6).

#### E.1.7 What keeps it out of a release build

| Guard | Where |
|---|---|
| `practice` and `synthetic` modules compile only with `debug_assertions` | BR `lib.rs:38`, `lib.rs:43` |
| `Plan.practice`, the practice seed, `practice_seeded`, `seed_practice` are debug-only | BR `shell.rs:96-98` and the new code |
| `Config::practice()` is the constant `false` in release | E.1.1 |
| `KeySource::Test` does not exist in release; `test` is refused, exit 78 | BR `config.rs:52-54`, `config.rs:303-332` |
| The synthetic agent is refused in release, exit 78 | BR `config.rs:344-351` |
| `Broker::seed` and `Seed` are debug-only | CORE `broker.rs:87-106`, `broker.rs:451-453` |
| `ProviderKind::Practice` is debug-only (ED8) | CORE `broker.rs` (new) |
| `AgentInput::Connect` and `Call` are debug-only, so a release session starts only through `agent_prove` | V11 (section C) |
| The release host ignores the six names and uses the Keychain | `host.rs:161-171`, `290-292` |
| The release bridge refuses `--socket` and `--key-dir`, exit 64 | W06 A23 |
| `verify-desktop.sh` compiles the release broker and runs its configuration tests and, from step P6, the core's release tests (ct30) in release | W04 D11; PW c42r |

The tests are e01 to e04 of E.1.12.

#### E.1.8 How the owner starts it

From step P6 (it also needs the screens session's Agents screen, SR-1 to SR-3, and the debug bundle with the bridge
of W06 7.5). From the checkout's root, Isabelle installed beforehand with `sh scripts/setup-demo.sh`:

```sh
sh scripts/build-desktop.sh --configuration debug
D="$(mktemp -d)"
FABRIC_DESKTOP_DATA_DIR="$D" FABRIC_BROKER_KEY_SOURCE=test \
FABRIC_BROKER_ISABELLE_HOME="$PWD/.tooling/isabelle/Isabelle2025-2" \
FABRIC_BROKER_ISABELLE_USER_HOME="$PWD/.tooling/isabelle/home" \
FABRIC_BROKER_RULES_DIR="$PWD/isabelle" \
  "target/debug/bundle/macos/Fabric Calendar.app/Contents/MacOS/fabric-desktop"
```

`FABRIC_BROKER_SYNTHETIC_AGENT` is left out. Then:
1. The build line ends `CD-W04 practice`; the terminal shows the practice stderr line; the strip says the rule
   checker is starting. Wait for "Running."; until then every tool call answers `prover_unavailable` (P8).
2. Agents screen: add an agent (client kind, name). Copy the configuration text: it names the bundle's bridge,
   `--enrollment <id>`, `--socket "$D/broker/run/agent.sock"` and `--key-dir "$D/broker/bridge-keys"` (ED10).
3. Paste it into the client, for Claude Code a `.mcp.json` in a folder of its own (W06 section 6), and start the
   client there.
4. The first tool call answers that the agent waits, with its pairing code. Compare it with the Agents screen (or
   `fabric-mcp-bridge code --enrollment <id> --key-dir "$D/broker/bridge-keys"`), read the approval text (OD8) and
   approve.
5. Ask the agent to list calendars, read availability, create an event, move it, delete it, delete
   `practice-foreign-1`; pause and resume the agent; remove it.

Use a new `D` for every run: the practice calendar lives in memory (PW X3), so a reused store names events the next
run does not have; their next observe answers `Absent` and the request ends `terminal_failure` or `stale` (CO6), never
a change. The socket path under `mktemp -d` on macOS is about 71 bytes, below the 103-byte limit; the listener
checks it anyway (W06 LR-B2, section C). The Keychain is not touched: the store has the test key and the bridge
the key directory.

#### E.1.9 Amendments to W04 and PW (new wording; OD9)

Proposed. Applied in the pull request named in the last column, after the owner's answer (ED12).

| Decision | New wording | Step |
|---|---|---|
| W04 D13, the amendment sentence | "Amendment, accepted by the owner on 2026-11-17 (PLAN 15.2 NOV17-PRACTICE) and widened by phase 2: one debug-only exception, the practice calendar (`apps/fabric-broker/src/practice.rs`), an in-memory provider on the practice account, compiled only in debug builds and attached only with the test key, a configured Isabelle and no Google provider (`Config::practice`), for the synthetic agent's script and for agents enrolled through the owner commands alike. It answers dispatch, observe, read, reconcile and list orders; it carries out only orders the core makes after a consumed authorization resting on an Isabelle permit, or after an accepted Isabelle system proof; it never counts as evidence for a Google gate (phase 2 design, E.1)" | P6 |
| W04 D14 | "With no prover attached, or while its status is not Ready (the synthetic classifier of a debug build counts as ready), a call that would create a request is refused `prover_unavailable` and nothing is recorded (phase 2, P8). A request already open when a proof is due and no prover is attached ends `undecided` at once with no report, as before." (B.10; OD18; the synthetic exemption ruled by the lead 2026-11-17, I.3 B-8) | P5 |
| W04 D26, the amendment sentence | "Amendment: with Isabelle configured as well, the run is a practice run (D13) and the classifier is not attached; the agent then runs its practice script. A practice run without the synthetic agent has the practice account, seeded by the shell, and no script (phase 2 design, E.1.3)" | P6 |
| W04 5.10, first paragraph, after "creates agent ... and account ..." | "In a practice run the shell has seeded the account already, so the agent's seed adds only the agent, and from phase 2 its list grant (V13)." | P6 |
| PW X2 | Chosen: "No variable; on exactly when `cfg!(debug_assertions)`, the test key, `isabelle.is_some()` and no Google provider (`Config::practice`). The synthetic agent only adds its script; a configuration with the test key, Isabelle and Google is refused (exit 78)". Why: "Practice rules P1 and P2 come from debug-only code and the test key, which the host passes only with a scratch directory. The synthetic agent added no guard. A fourth variable adds refusals and no protection" | P6 (the Google clause in P7b) |
| PW X5 | "The practice account `synthetic-account` / `synthetic-calendar`, the calendar named 'Practice calendar (test build)', seeded by the shell in every practice run (`practice::seed`); one event Fabric did not make" | P6 |
| PW X6 | "Superseded in phase 2: the core observes the event through the provider port (`ObserveOrder`, CO6) under a system proof and stores the observation before any proof reads it. The shell's hook is removed" | P5 |
| PW decision P2, the clause "the observation hook returns `Option<ObservedEvent>`" | struck, with "(removed in phase 2, X6)" | P5 |
| PW decision P4, O1 clause | "O1 answered by the owner on 2026-11-17 (PLAN 15.2 NOV17-PRACTICE): the practice calendar is accepted, and a real enrolled agent may use it, in debug builds only, with the test key and a configured Isabelle; it is evidence for the wiring only" | P6 |
| PW 3.3, the table | the eight rows of E.1.2; the sentence "Agent calls are not refused at entry while the prover is absent" becomes "While the prover is not ready a call that would create a request is refused `prover_unavailable` (phase 2, P8)" | P6 |
| PW 4.1, guards | "Guards: a release build does not compile it and refuses the test key (exit 78), and its `Config::practice()` is the constant false; it needs the test key, which the host passes only with `FABRIC_DESKTOP_DATA_DIR` (W4D D26); it is not attached when a Google provider is configured (refused, exit 78)" | P6 |
| PW practice rule P1 | "compiled only with `debug_assertions`; a release build refuses what would start it (the test key, exit 78) and its `Config::practice()` is the constant false" | P6 |
| PW practice rule P2 | "only with the test key, which the host passes only with a scratch data directory chosen by the person running it; never with the Keychain store, never with a Google provider. The synthetic agent is not needed" | P6 |
| PW practice rule P3 | "it carries out only what the core orders: a dispatch or a read after a consumed authorization resting on a live Isabelle permit, an observe, a reconcile or a list after an accepted Isabelle system proof; without a prover nothing is ordered" | P5 |
| PW practice rule P4 | "it answers each order as a provider would, under the outcome rule of phase 2 (`Transient` only when nothing happened); a create uses the reserved event id, and an order for an effect already applied is refused as Google would refuse it; it notifies and invites nobody and refuses a change that asks to; it changes only its own seeded calendar" | P5 |
| PW practice rule P5 | "it is labelled where it shows: the build label says `practice` (`synthetic practice` with the script), the broker writes one fixed stderr line, the calendar is named 'Practice calendar (test build)', the agent's handshake answer says `practice: true` and the bridge tells the model, and the page's practice line shows only in a practice run; the contract has no marker for practice records (E3)" | P6 |
| PW 4.2, first sentence | "In memory: `Arc<Mutex<State>>`, shared by the provider worker and, when it runs, the synthetic agent" | P5 |
| PW 4.3 | the dispatch table of E.1.4 and the other four orders | P5 |
| PW 4.4 | "Removed in phase 2 (X6)" | P5 |
| PW 5, last sentence | "Practice changes end `reconciled` once applied (phase 2, V7), so they no longer stay open" | P5 |
| PW 9.3 O1, O2 | O1: answered (above). O2: "moot: the hook is removed in phase 2" | P6 |
| PW 9.4 items 1 to 3 | item 1 answered by the screens request of E.1.6; item 2 by V7; item 3 by `EffectEventRecorded` and request details | P6 |

The source comments that repeat the old wording change with them: BR `lib.rs:13-17`, `practice.rs:1-14`,
`config.rs:82-84`, `shell.rs:104-114`. `apps/fabric-desktop/README.md` "The practice run" (118-160) and the harness
header (`serve.ts:10-19`) are not the lead's files (J-store practice minor): they are written requests to their
owner, the screens session, unless the owner widens the lead's list.

#### E.1.10 Answer to LR-B6

W06 7.2 asks for "a practice mode for a real agent (S5): the practice account and calendar in a debug build with the
test key, without the synthetic agent's own proposals". E.1.1 to E.1.8 give it: row 4 of E.1.2, the shell's seed,
no script, enrollment through `add_agent` and `approve_agent` as in a release build (P11). W06 5.6's second blocker,
"a practice read has no path to a result", is answered by P6's two-call read on the practice provider's `read`
(E.1.4, E.1.5). No relay name is needed (W06 LR-B4 holds). The key directory W06 section 6 left open is ED10.

#### E.1.11 Gate G2c: what a practice run shows and what only Google shows

G2c is "three external clients, pending/allow/block/pause paths" (PLAN 13 row W06), with "read, Ask, owner
approval, actual Google effect, pause test" per client (PLAN 9.3) and the list of W06 10.4.

| Item of G2c | Practice run, per client | Only Google |
|---|---|---|
| The client starts the bridge, negotiates a revision, accepts the ten tool schemas | yes (W06 10.4 first two bullets) | |
| Enrollment: pending, pairing code, approval, the list permission | yes | |
| A removed or unknown key gets `not_permitted` | yes | |
| A real availability read and an event read | the path and the two calls, on memory data | the content of a real calendar, Google's paging, zones, recurring events (W05 LV8) |
| A denied request with its proof | yes: delete of `practice-foreign-1`, rule `fabric_created`, Isabelle | |
| A pending approval and an approved create | yes, carried out in memory | "actual Google effect" (PLAN 9.3) |
| An edit of an existing test event | yes, of the agent's own event, ids from its effect result | conditional writes against Google's etags (LV3) |
| An invitation to an owned test recipient | no: refused by row 5 | yes, a real message (LV7) |
| Pause, resume, revocation while polling (Q10 cases) | yes | the same on a real read |
| Removal, idle end, reconnect | yes | |
| An unknown outcome and its reconciliation | only with in-process faults (6a, 8), never in a run | a real lost answer (LV4) |
| The destination notice for a cloud agent's approved read | the screen text only | approved real data reaching the client's cloud (PLAN 14, R14) |
| Proof id and theory hash on each report | yes | |

Nothing of the left column counts for G2c (ED11). It is the wiring check that lets the three client runs at G2c
start from a known-good channel; G2c itself runs after G2b, on Google (REV-order S16-7).

#### E.1.12 Tests

Unit tests in BR, CI (no Isabelle) unless marked. Each names what it catches.

| Id | Test | Catches | Step |
|---|---|---|---|
| e01 | `practice_is_debug_test_key_and_isabelle` (replaces PW c42): true for test key and Isabelle, with and without the synthetic agent; false without Isabelle; on macOS false with the Keychain and Isabelle | keeping `&& self.synthetic_agent`; dropping the key check; dropping the Isabelle check | P6 |
| e02 | PW c42r kept: a release `Config` with the Keychain, the synthetic agent and Isabelle has `practice()` false | removing the release constant | P6 |
| e03 | release, macOS: `build_label` of that configuration has no `practice` | a label that tests the variables directly | P6 |
| e04 | debug: test key, Isabelle and the Google variable are refused, exit 78, naming the Google variable and printing no value; test key and Google without Isabelle are not refused by this rule; the debug host refuses the same three | a Google run with the practice provider; the refusal missing on one side | P7b |
| e05 | the `Plan::of` table: each row of E.1.2 gives its prover, provider, script and label; row 4 has `agent: None` and `provider: Some(Practice)` | building the script whenever practice is on; labels that do not tell rows 4 and 5 apart | P6 |
| e06 | a practice run with no synthetic agent, test key, temporary directory, no `subscribe`: after one turn the store has `synthetic-account` with one selected calendar named "Practice calendar (test build)", no `synthetic-agent`, one `broker.seed` row | a seed gated on the script or on `subscribe`; a seed carrying the agent | P6 |
| e07 | a practice run with the synthetic agent: one `AccountPut`, one epoch raise, and the calendar named with the practice name, also when the `subscribe` that starts the script is the frame that reopens the store | the script's seed winning the race | P6 |
| e08 | a practice run with no synthetic agent, `subscribe`d and `Ready`, run for 10 s of injected time: no request, `dispatches() == 0` | a path that proposes A to D without the script | P6 |
| e09 | the store unavailable at start (a held lock, W04 D12), then reopened: seeded once after the reopen; `practice_seeded` false before | the flag set before a successful seed | P6 |
| e10 | in-process, the core fed `Input::ProofDone` as W04 D13 says: an agent seeded by a test `Seed` (not `synthetic-agent`), connected on a listener connection id at or above `1 << 32` through the debug `Connect`, proposes a create; after approval and a permit it ends `reconciled` and `event_of_request` holds it with `fabric_created` true | provider or observation tied to the synthetic agent; row 3 changed to check the agent | P6 |
| e11 | each method of `PracticeProvider` against C1 to C3 and its own rows: wrong proof kind, a system digest that does not match, another pair, a passed deadline, `still_allowed` false before and after building | an order acted on without its proof; a read past a revocation | P4 |
| e12 | dispatch rows 5 to 9 with 5a, 6a and 8a, a delete answered `event: None` and `confirmed: true`, and a resend of an applied effect refused by row 7 (row 4 removed); the existing unit tests that change with them (REV-port for_the_lead: BR `practice.rs` tests near 650-664, 676-680, 794-806, 820-822; and the delete asserts near 695-696 and 890-891) | a false notification outcome; `Transient` after a change; a practice delete that B.14 treats as a mismatch | P4 |
| e13 | `Shell::prove` passes a job whose `facts.event` is `None` to the prover unchanged in a practice run | the hook left in place | P5 |
| e14 | list on practice: `Calendars` with the practice calendar only, request `completed`, `dispatches()` unchanged | dispatching a list; listing an unselected calendar | P5 |
| e15 | availability on practice: the read ends `reconciled` with `result: null`; `fabric_request_result` returns `Availability`; a second release ends `terminal_failure` (`invalid_arguments`) | content in a read's own answer; two releases of one held result | P5 |
| e16 | the Q10 cases of section B run on the practice provider as well as with `Input::Attempt` (REV-order Q10-timing) | a release under stale authority | P5 |
| e17 | live, `#[ignore]`, Isabelle, by name: the built broker in row 4 and the real debug bridge with `--socket` and `--key-dir`: the stderr line and ` practice` label; enroll, approve, list, create, its effect result, update it, delete it, delete `practice-foreign-1` (blocked), pause (blocked by `agent_not_paused`), remove (connection closed, next call `not_permitted`); `agent_prove` answers `practice: true`, and the first text block of the first tool result begins with the practice sentence (W6-2) | regressions across the listener, enrollment, results and practice; an unlabelled practice run | P6 |
| e18 | the live tests of PW 7.3 that start the broker with the test key and Isabelle and without the synthetic agent now run as practice runs: each is run by name once in P6, and any whose outcome changes is listed in the pull request | an unnoticed change of what a live test shows | P6 |
| e19 | after step A ends `reconciled` the script proposes C and D | the script stopping after B once V7 lands | P5 |

---------------------------------------------------------------------------------------------------------------------

### E.2 The order of the work (P12)

#### E.2.1 Steps

Each step ends in one pull request that the owner merges, unless the row says two. "Lead" is the session that owns
`crates/fabric-storage`, `crates/fabric-broker-core` and `apps/fabric-broker` (PLAN 15.2 NOV17-SESSIONS).

| Step | What | Files | Needs first | Writer |
|---|---|---|---|---|
| P0 | In progress: the broker part of schema version 2 (`ProverStatus`, `ProverCause`, `ProofDone.cause`, `undecide()`, the cause in `RequestMoved`), then the duration rule | CORE `ports.rs`, `broker.rs`, `service/{core,pipeline,reducer,requests,tests}.rs`, `owner/{views,tests}.rs`, tests; ST `{apply,ledger,mutation,read,rows,testing,tx}.rs`; BR `prover/{map,mod}.rs`, `shell.rs`, `synthetic.rs`, tests; W04 and PW designs (`git diff --stat origin/main...claude/cd-broker-prover-reasons`) | contract schema version 2 (merged, PR 28) | lead |
| P1 | This design document; documents only. No existing document is edited | the design document only | nothing; runs beside P0 | architect, writers A to E, editor |
| P2 | Contract schema version 3: PR 36 finished from P1's shapes (section G, P2-1 to P2-30; P2-31 to P2-34 in it or in a later pull request, merged before the G2b live run, V23) and rebased on P0. TH1 and TH2 with a real Isabelle run may follow as a second pull request; it is needed before P7b | `contracts/desktop/v1/**`, `crates/fabric-desktop-contracts/**`, `isabelle/**` and the policy crate's mirror for TH1; PR 36's hunks in CORE and ST (`git diff --stat origin/main...origin/claude/contract-v3-owner-commands`: `broker.rs`, `owner/{dedup,frames,mod,profiles,tests}.rs`, `service/{agent,facts,grants,pipeline,reducer,requests,sessions,tests}.rs`, ST `apply.rs`, `store.rs`); `Cargo.lock` (PR 36 adds `ed25519-dalek` to the contracts crate) | P0 merged; P1 merged; the lead has re-read DRAFT's diff since `29cdfdf` (I.7 X-3); OD1, OD2 and OD24 answered (2026-11-18, PLAN 15.3) | contract session |
| P3 | Migration 2 (section A): the tables, mutations, operation types 23 to 34, invariants with `since`, the gated branches G1 to G9, the v2 fixture, the rehearsal on an encrypted v1 fixture; the shared types of X21 (`SystemAction`, `SystemActionKind`, `ObservedEvent`, `Notification`, `ConnectProblem`, `AuthLostCause`) in ST `rows.rs` with serde; the four undecided causes of 0.4.4 in the stored cause type, `SessionEndReason::AgentRemoved` and `VoidReason::AgentRemoved` (A.4.5); the ADR-115 and W04 amendments of A.3 as approved (OD5, OD20) | `crates/fabric-storage/**`; two test files that pin version numbers (`crates/fabric-broker-core/tests/faults.rs`, `apps/fabric-broker/tests/process.rs`, A.8.4); CORE `owner/views.rs` (`reason_of` and its test: the match over the stored cause is exhaustive on the step P0 branch, so the four causes get the values of 0.4.4 in the same pull request); `docs/adr/ADR-115-broker-ledger-envelope.md`; W04 D3, 3.5, 3.6, section 4, 9.2 and R22 (A.3) | P2 merged (`EnrollmentGrantInserted` carries a version 3 `Grant`); OD5, OD7 and OD20 (its A.3 part) answered; P2-26 met (I.7 X-2) | lead |
| P4 | The provider port and the practice provider: D.2 types and `Job`/`Input` variants, with `until` on every `Provider` method (X23); `system_action_digest` and the re-exports of X21 in CORE `ports.rs`; `Attachments.provider`; the core ignores the new inputs (no order of those kinds exists yet); the practice provider of E.1.4; the shell routes the new jobs to the provider worker and stamps each job (B.13); the dispatch side of DD1 to DD3 in the core: `dispatch()` builds D.2's `DispatchOrder` with `order_id` from `next_order_id`, `proof: OrderProof::Agent(authorization)` and `target` from `resolve_target` (D.3, practice row); `attempt_done` and `finish_attempt` read D.2's `AttemptDone` and treat `Applied { event, confirmed, notification }` as today's `Applied { event }` (V9, BD16 and B.14 land in step P5); `can_carry_out` (0.5.3) replaces `provider_present` | CORE `ports.rs`, `broker.rs`, `service/pipeline.rs` (`:581-634`, `:667`, `:696-720`, `:917-989`), new `service/provider.rs` (`resolve_target`, D.3), `service/core.rs` (the field `next_order_id`; the test attachments at `:464-472`), `service/mod.rs` (`mod provider`); BR `practice.rs`, `shell.rs` | P0, P2 and P3 merged (P0 and P2 write `broker.rs`, `pipeline.rs`, `ports.rs`; D.2 names `Notification`, `ConnectProblem`, `AuthLostCause` and `SystemAction`, which P3 ships, X21) | lead |
| P5 | Results, held answers and system proofs (section B): CO5, CO9, CO13, the D17, D18, D33 and `used` amendments, held answers (P7), releases, purges, P8's `prover_unavailable`; and, provider-neutral (ED5), the fence fix, the order watchdogs, observe before a proof (CO6) and reconcile (CO8, PD20); the hook goes (ED4). The Q10 tests on `Input::Attempt` and on the practice provider. Two pull requests in this order (the lead's ruling of 2026-11-17, I.6 E-9): P5a (fence, watchdogs, system proofs with `Prover::prove_system`, observe, reconcile, hook, `resolve_outcome` (D.7.5) and `select_calendars` (D.9.3), both of which need only migration 2 and work on the practice account; steps 1 and 3 of D.8.3 (a disconnect moves the account's requests in `outcome_unknown` and `reconciling` to `needs_owner_resolution` and drops its orders and the system proofs that serve a moved request), which need only migration 2 and apply to the practice account; DD10, and the test of D.9.4 with the partition of `OwnerMethod::ALL` and the entries built so far; step P6 adds `remove_agent`; step P7b adds `connect_google` and `NARROWING_COMMITS`) and P5b (results, held answers, release, the purge at `disconnect_google` (D.8.3 step 2, B.9), `ProofAction::Disclose`, Q10; and the list's authority: `grants::create_enrollment_grant` (C.4.1), the debug seed's call of it (E.1.3), X16's handling of `GrantOrigin::Enrollment`, and CD21: `scope_of(CalendarList)` and the `""` purpose in `create_grant`, so that r01, e14 and Q17 can pass in step P5). `list_ended_requests` and `delete_custom` stay `unknown_method` in phase 2 (A.6, AO8) | CORE `service/{pipeline,facts,grants,reducer,requests,agent,core,recovery,epochs}.rs`, new `service/held.rs`, new `service/results.rs` (B.4.1), `service/provider.rs` (the calls of D.3 for system orders), `ports.rs` (`ProofAction::Disclose`, `Prover::prove_system`, `SystemProofJob`, `SystemProofDone`), `owner/{commands,dedup,mod}.rs` (`resolve_outcome`, `select_calendars`), `owner/preferences.rs` (DD10), `broker.rs`, new `tests/q10.rs`; BR `prover/map.rs` (B.5.2), `prover/mod.rs` (the Isabelle handle's path for `Job::ProveSystem`), `synthetic.rs` (`SyntheticClassifier::prove_system`, which answers with no report; `Practice::a_event` accepts `reconciled`, V7), `apps/fabric-broker/tests/live.rs`, `shell.rs` (the hook), `practice.rs` comments; W04 D14, D17, D18, D33, 5.6, 5.7, 5.8; PW X6, 4.2 to 4.4, decision P2, practice rules P3 and P4 | P3 and P4 merged; OD3, OD9 (the hook), OD11, OD15, OD16, OD17, OD18 and OD20 (W04 5.6 to 5.8) answered; the screens' text for `completed` merged | lead |
| P6 | The agent channel (section C) in the core and the binary, with the practice mode: enrollment commands, the handshake in the core, removal, `approve_agent`'s call of `create_enrollment_grant` (the helper is P5b's), the listener, the peer check, the configuration text with ED10's debug extras; E.1.1 to E.1.8. Q18 lands here with ct17 | CORE new `service/handshake.rs`, new `service/release_tests.rs` (ct30), `service/mod.rs`, `service/{agent,sessions,grants,core,limits,recovery,reducer,requests,pipeline,facts}.rs` (C.6.1, C.3.3, C.2.1), new `owner/agents.rs` (C.3), `owner/{commands,views,dedup,mod}.rs`, `ports.rs` (`AgentInput`, LR-C7), `broker.rs` (`Output.bound`), `crates/fabric-broker-core/Cargo.toml` (`ed25519-dalek` for CD8; the core does not depend on it today); BR new `listener.rs`, `config.rs`, `shell.rs`, `main.rs`, `practice.rs`, `synthetic.rs`, `Cargo.toml` (bridge crate for `config_text`, `nix` for the peer's user id); root `Cargo.toml`, `Cargo.lock`; `scripts/build-desktop.sh` (bundle the bridge, W06 7.5); `scripts/verify-desktop.sh` (the core's release tests, ct30); W04 D11, D13, D25, D26, D28, L2, 5.5, 5.10, 6.1; PW X2, X5, 3.3, 4.1, practice rules P1, P2, P5, P4/O1, 9.3, 9.4 | P5 merged; the CD-W06 session's bridge on the contract's handshake types merged; P2-19 merged and the bridge's practice text (W6-2) merged (P11: the run is labelled for the agent); OD8, OD9, OD19, OD21 and OD22 (its OPC rows) answered; the Agents screen merged for the owner's run | lead, with the CD-W06 session reviewing |
| P7a | Migration 3 (ST1, ST2; V1): `provider_accounts`, `provider_calendars`, operation types 35 to 39 (and 40, `owner.refresh_calendars`, if P2-11 merges), invariants I22, I23, the v3 fixture and a v2 to v3 rehearsal | `crates/fabric-storage/**` only | P3 merged; runs beside P5 and P6 (they do not edit storage, E.2.3) | lead |
| P7b | The Google core and binary (section D): CO1 to CO3, and steps 4 and 5 of D.8.3 (CO4: `provider_accounts.revocation`, `connect_generation`, `Unlink`, `provider.revoke`) with the unlinks at start of D.8.4 (DD18), CO7, CO10, CO11, `can_carry_out` for Google, the sign-in, unlink and provider workers, the Google variables and ED2, the relay's and the host's names, process tests against FAKE. CO12 (re-observe a waiting card) stays deferred (D.13) | CORE `service/*`, `owner/*`, `broker.rs`; BR `config.rs`, `shell.rs`, `main.rs`, new workers, `Cargo.toml`, tests; `apps/fabric-desktop/relay/src/config.rs`, `apps/fabric-desktop/src-tauri/src/host.rs`; W04 D27; PW X2's Google clause | P6, P7a, the CD-W05 session's GC phase 2 merged; TH1 merged (P2-6); OD4, OD6, OD12, OD13 and OD20 (W04 D38, R24) answered | lead, with the CD-W05 session |

Outside the lead's chain, each its own pull request:

| Work | Files | Needs first | Writer |
|---|---|---|---|
| CD-W05 phase 1 (pull request 41, merged 2026-11-18) | `crates/fabric-google-calendar`, `crates/fabric-google-fake`, root `Cargo.toml`, `Cargo.lock`, ADR-111, ADR-113, W05 design | nothing | CD-W05 session |
| CD-W06 phase 1 (open) | `apps/fabric-mcp-bridge`, root `Cargo.toml`, `Cargo.lock`, ADR-115 (checkbox 4), W06 design | nothing | CD-W06 session |
| GC phase 2a (the lead's rulings of 2026-11-18, I.9): the delete's read-back (W5-11); the member label, the empty list of normalisations and FAKE's two faults (W5-12); start and end asked always and the adapter's typed span (W5-14); the 20-page cap on a calendar list; the supersede methods (W5-15). No type of the port and nothing in the core, the store or the broker binary; it also carries the W05 design's sections 18.6 and 18.7 | `crates/fabric-google-calendar/**`, `crates/fabric-google-fake/**`, the W05 design | nothing: built now from main, as one pull request of its own, ahead of step P4 | CD-W05 session |
| GC phase 2: the `Provider` and `AccountLink` implementations (the port mapping, `ReadSpan`, `unlink`), the dependency on the core, `KeychainTokenStore`, `SystemBrowser` | `crates/fabric-google-calendar/**` and `Cargo.lock` | P4 merged | CD-W05 session |
| Bridge phase 2: the contract's handshake types, `practice` (P2-19) in the first text block, the words for `completed` with and without a result and the one sentence for `applied` and `reconciled` (W6-3, W6-4), the cancel of P2-30 when a client cancels a call (once P2-30 merges), the Keychain key store | `apps/fabric-mcp-bridge/**` and `Cargo.lock` | P2 merged | CD-W06 session |
| Screens for version 3: S07 Agents (SR-1 to SR-8) with the approval text (OD8); the text for `completed`; a grant whose scope names no calendar; requests of a removed agent; `select_calendars`, `resolve_outcome`; the conditional practice line; the README practice section and the harness header | `apps/fabric-desktop/ui/**`, `apps/fabric-desktop/README.md`, `apps/fabric-desktop/harness/serve.ts` | P2 merged; the `completed` text before P5, the rest before P6 | screens session |
| Screens for Google: S01 account states, `provider_accounts` (P2-9), `undecided_reason` `provider_unreachable` (P2-7), `notification` (P2-10), `refresh_calendars` (P2-11) | `apps/fabric-desktop/ui/**` | P2 merged; before the G2b live run | screens session |
| G2a put to the owner, with Q10 (OD10) | none | P5 merged | lead |
| The G2b live run (W05 15.3) and the independent review | evidence only | P7b; RUNBOOK done; OD22 (D-4), OD23, OD25 and OD26 answered; P2-31 to P2-34 merged and taken into the core (V23: no Google dispatch before them); the Google screens | owner with the lead and the CD-W05 session |
| The G2c runs with the three clients | evidence only | G2b; bridge phase 2 | owner with the CD-W06 session |

#### E.2.2 What each step asks of others

| Step | Contract session | Screens session | CD-W05 session | CD-W06 session | Owner |
|---|---|---|---|---|---|
| P0 | none | the page's duration choice (branch `claude/cd-w08-page-duration-ceiling`) | none | none | merge |
| P1 | read section G | read E.2 | adopt V1, V2 and V20; drop its own migration number and ST5 columns but `notification`; adopt `releases` and `request.release`; build GC phase 2a from main (I.9) | adopt V12, V14, P2-19; the guidance changes | approve the design; show-before-apply edits come with each step |
| P2 | P2-1 to P2-30 (section G), P2-31 to P2-34 by the G2b live run (row P7b); rebase PR 36 on P0; P2-14 pins every v2 fixture; TH1/TH2 with a real Isabelle run | none | review P2-6 to P2-13 | review P2-15 to P2-19 and P2-30; if PR 38 merged first, write the patch to `apps/fabric-mcp-bridge/src/mcp/render.rs` and its test r01 for the new states, carried in PR 36 (E.2.4) | OD1, OD2 and OD24 answered 2026-11-18 (two calls, no card for a release, a list approved for one request is cancelled); the merge is his approval of schema version 3 (NOV17-SCHEMA3) |
| P3 | none | none | review the store shapes it asked (ST1 to ST5 in V1, V2) | review the enrollment tables | OD5, OD7 and OD20 (A.3) before the merge |
| P4 | none | none | start GC phase 2 on the merged port: the port mapping, `unlink`, the Keychain store and the browser (GC phase 2a went ahead) | none | merge |
| P5 | none | the `completed` text merged first (J-store results minor) | none | none | OD3, OD9 (hook), OD11, OD15 to OD18 and OD20 (W04 5.6 to 5.8) before the merge; G2a with OD10 after it |
| P6 | P2-19 merged | S07 merged before the owner's practice run | none | bridge phase 2 merged, with W6-2; review the listener and the configuration text | OD8, OD9, OD19, OD21 and OD22 (its OPC rows) before the merge; the practice run (E.1.8) after it |
| P7a | none | none | review | none | merge |
| P7b | TH1 merged; P2-31 to P2-34 merged before the G2b live run (V23) | the Google screens before G2b | GC phase 2 merged; FAKE as a dev-dependency of `apps/fabric-broker`; the CO10 names | none | OD4, OD6, OD12, OD13, OD20 (W04 D38, R24) before the merge; OD22 (D-4), OD23, OD25, OD26 and the RUNBOOK before G2b; OD14 before the release client is created |

#### E.2.3 What may run at the same time

One writer per file at a time; a step that writes a file starts after the last step before it that writes the
same file has merged.

| Together | Why it is safe |
|---|---|
| P0 and P1 | P1 writes only its own new document; P0's two design edits are W04 and PW |
| P1 and both phase 1 pull requests | disjoint; the two phase 1 pull requests share the root `Cargo.toml` and `Cargo.lock`: merge one, rebase the other |
| P2 to P4 and GC phase 2a | GC phase 2a writes only `crates/fabric-google-calendar/**`, `crates/fabric-google-fake/**` and the W05 design, which no step of the lead's chain writes, and changes no type of the port |
| P3, P4, P5, P6 and the screens for version 3 | the screens write `apps/fabric-desktop/ui/**` only |
| P3 to P6 and bridge phase 2 | the bridge writes `apps/fabric-mcp-bridge/**`, and `Cargo.lock` for its new dependencies; P6 writes `Cargo.lock` too: merge one, rebase the other |
| P5, P6 and GC phase 2 | GC writes `crates/fabric-google-calendar/**`, and `Cargo.lock` for its new dependencies; P6 writes `Cargo.lock` too: merge one, rebase the other |
| P5, P6 and P7a | P7a writes storage only, and P5 and P6 do not (P3 writes `crates/fabric-storage`, two test files, `crates/fabric-broker-core/tests/faults.rs` and `apps/fabric-broker/tests/process.rs`, CORE `owner/views.rs`, and the documents of A.3): every store reader and mutation they need is in P3. A reader found missing waits for P7a or goes into it |

Not together: P2 with P0, P3 or P4 (PR 36 writes CORE and ST files); P4 with P0 (`ports.rs`); P5 with P4 or P6
(`pipeline.rs`, `broker.rs`, `shell.rs`); P6 with P7b (`config.rs`, `shell.rs`, `main.rs`, owner files); P7a with P3.

P4 waits for P3 (X21), so P3 and P4 no longer run together. The lead ruled so on 2026-11-17 (I.6 E-10); this
changes P12's "P4, in parallel with P3".

The longest chain: P0, P2, P3, P4, P5, P6, P7b, G2b, G2c.

#### E.2.4 Files that more than one step touches

| File | Steps, in order |
|---|---|
| CORE `ports.rs` | P0, P4 (D.2 types, `system_action_digest`, the re-exports of X21), P5 (`ProofAction::Disclose` in P5b, `Prover::prove_system` and the system proof types in P5a), P6 (`AgentInput`) |
| CORE `broker.rs` | P0, P2, P4, P5, P6, P7b |
| CORE `service/pipeline.rs` | P0, P2, P4, P5, P6, P7b |
| CORE `service/provider.rs` (new) | P4, P5, P7b |
| CORE `service/agent.rs` | P2, P5, P6 |
| CORE `service/facts.rs`, `service/grants.rs` | P2, P5, P6, P7b |
| CORE `service/reducer.rs`, `service/requests.rs` | P0, P2, P5, P6, P7b |
| CORE `service/sessions.rs` | P2, P6 |
| CORE `service/epochs.rs`, `service/limits.rs` | P5 (`epochs.rs`), P6 (both) |
| CORE `owner/agents.rs` (new) | P6 |
| `crates/fabric-broker-core/Cargo.toml` | P6 |
| `crates/fabric-broker-core/tests/faults.rs`, `apps/fabric-broker/tests/process.rs` | P3 (A.8.4), and the steps that add tests |
| CORE `service/core.rs`, `service/tests.rs` | P0, P2, P4 (`core.rs`), P5, P6 |
| CORE `owner/{dedup,mod,frames,views,commands}.rs` | P0 (`views.rs`), P2, P3 (`views.rs`, `reason_of`), P5 (`resolve_outcome`, `select_calendars`), P6, P7b |
| ST `apply.rs`, `store.rs` | P0 (`apply.rs`), P2, P3, P7a |
| ST `{mutation,read,rows,ledger,tx,migrate}.rs` | P0, P3, P7a |
| BR `shell.rs` | P0, P4, P5, P6, P7b |
| BR `prover/map.rs` | P0, P5 |
| BR `prover/mod.rs` | P0, P5 |
| BR `practice.rs` | P4, P5 (comments), P6 |
| BR `config.rs`, `main.rs` | P6, P7b |
| BR `synthetic.rs` | P0, P5, P6 |
| `apps/fabric-broker/Cargo.toml` | P6, P7b |
| root `Cargo.toml`, `Cargo.lock` | CD-W05 phase 1, CD-W06 phase 1, P2 (`Cargo.lock`), bridge phase 2 and GC phase 2 (`Cargo.lock`), P6, P7b |
| `apps/fabric-mcp-bridge/src/mcp/render.rs` and its test r01 (the bridge's r01, not B.16's) | CD-W06 phase 1 (PR 38) and P2 (PR 36), in either order. The bridge's table of request states is one `match` with no default arm, so it does not compile against a contract crate with the new states `completed`, `resolved_as_applied` and `resolved_as_absent`. The one that merges second carries the lines: if PR 38 merges first, PR 36 carries a small patch to this file and r01, written by the CD-W06 session; if PR 36 merges first, the CD-W06 session adds the lines to PR 38. So the second merge does not break `main` (ruled 2026-11-18) |
| `apps/fabric-desktop/relay/src/config.rs`, `src-tauri/src/host.rs` | P7b only (practice needs no change) |
| `docs/plans/calendar-desktop-w04-design.md` | P0, P3 (D3, 3.5, 3.6, section 4, 9.2, R22), P5 (D14, D17, D18, D33, 5.6, 5.7, 5.8), P6 (D11, D13, D25, D26, D28, L2, 5.5, 5.10, 6.1), P7b (D27, D38, R24) |
| `docs/plans/calendar-desktop-prover-wiring-design.md` | P0, P5, P6, P7b |
| `docs/adr/ADR-115-broker-ledger-envelope.md` | CD-W06 phase 1, P3 |
| `apps/fabric-desktop/ui/**` | W08 duration branch, screens for version 3, screens for Google |

---------------------------------------------------------------------------------------------------------------------

### E.3 Decisions for the owner (P13)

Each row: the question in plain words, the default the design runs on, why, what changes if he chooses otherwise,
and when the answer is needed. The code is built on the default; nothing is applied to an accepted document before
his answer.

The owner answered OD1, OD2 and OD24 on 2026-11-18, each with the default. The record is PLAN section 15.3
(NOV18-READ-CALLS, NOV18-RELEASE-CARD, NOV18-LIST-CARD). The other rows stay open.

| Id | Question | Default | Why | If he chooses otherwise | Needed |
|---|---|---|---|---|---|
| OD1 | Should an agent's calendar read take two calls: the read, then `fabric_request_result` for the content? Plan 9.2 says "Allow returns the proven filtered read result" | Two calls. The read's own answer never carries content; the content is released only to `fabric_request_result` under its own fresh `Disclose` proof (P6) | One release path, the theory's own (`a_result_is_released_only_when_held`); matches Q10's default; one path to test (REV-results two-calls-vs-PLAN-9.2) | The alternative: after the read stores its result and while its answer still waits (within 20 s), the core runs one fresh proof on `Disclose(read)` for the read itself, with no classification (3 Isabelle checks per read in place of 4, saving 1.0 s to 1.8 s, PW L3), and releases into that answer. Cost: a new proof purpose or a lifecycle edge out of `reconciled`, a README change (P2-2 reversed), a proof filed on a request that is already `reconciled`, and still the two-call path when the hold runs out, so two release paths to test. Saves one model turn per read | Answered 2026-11-18: two calls (PLAN 15.3). It was needed before P2 merges (P2-2, P2-3, P2-29 write the default into the contract README) |
| OD2 | When releasing a held result needs his approval, should he get a card? | No card in version 1: such a release ends `undecided` (P2-3) | A release card needs a scope for `fabric_request_result` and its own card shapes, which the contract does not have (W05 V3-10) | The contract session designs a release card; P5 waits for it | Answered 2026-11-18: no card (PLAN 15.3). It was needed before P2 merges (P2-2, P2-3, P2-29 write the default into the contract README) |
| OD3 | How long does the broker keep a read's result waiting for its release? | 24 hours; purged at the first release, at the account's disconnect, at the agent's removal, and at 24 h (V4, V15, `PurgeCause`). A one-request read permission shows as in force until then | Short enough to bound what an encrypted store holds; long enough for an agent that polls late (W05 OQ5). One release per held result means no second copy outlives its disclosure (V4, OP2) | A shorter time purges sooner; a longer one keeps content longer. Allowing a later release under a broader permission (plan 9.2) means no purge at release (OP2). Also stated with it: a purged result can stay in older write-ahead-log frames until the next checkpoint (AO4); a result released into a full writer queue is gone, and the agent reads again (OP13, OD17) | before P5 merges |
| OD4 | How many Google accounts may be connected? | No fixed count: every account counts at `ACCOUNT_RESERVE_BYTES` (96 KiB) into the 720 KiB budget of the owner's records (`OWNER_STATE_BUDGET_BYTES` 737,280; P10, D.11), and a connect past it is refused. So at most 7 accounts fit with no other records, none beside a recipient set at the schema's maximum, and no command gives an account's reserve back (OQ-D3; X8) | Every snapshot must fit in one 1 MiB frame; without a cap a sixth large account made the page unable to load (REV-order OQ14) | A fixed count beside the budget, or a smaller reserve with a lower calendar limit per account (D.14 OQ-D3) | before P7b merges |
| OD5 | Accept three amendments of ADR-115: (a) migrations only add, and existing replay code may only widen (P2; also W04 3.6 and R22); (b) read results and observation evidence kept outside the ledger, in tables replay does not rebuild; a read result is deleted at its purge and in every backup copy; evidence (the event as Google returned it, at most 1 MiB for each observation, its before-view at most 512 KiB, and an unchanged event at one revision kept once; OD26) stays with its request and is never deleted (A.7; D1, D7; V3, V18; OP14); (c) the ledger's operation types grow from 22 to 34 at store version 2 and to 39 at version 3, 40 if P2-11 adds `owner.refresh_calendars` (D2; A.6; X9) | Accept | (a) keeps every older ledger replayable without version-aware replay (REV-order ADR-115-Open). (b) lets a purge be real: content in the ledger could never be purged. (c) every new commit needs its type | (a) refused: version-aware replay first, a larger step before P3. (b) refused: no held results, so reads cannot return content (OD1's path is gone) until another store is designed. (c) is needed by any design | before P3 merges |
| OD6 | After a create whose outcome was unknown and then found absent, a late copy can still appear at Google, and a new create can make a second event. Accept this residual risk? | Accept, with the mitigation: the absent create's reserved id stays on record; a later observe or read that sees it records it as made by Fabric and shows it to him (OP12) | Updates, invitations, responses and deletes are guarded by Google's revision check; a create has no such guard across two requests (REV-core PD20) | Not accepting means no automatic "absent" for a create: every unknown create goes to `needs_owner_resolution` for him to decide | before P7b merges |
| OD7 | Should the broker's own calendar reads (refresh before a proof, reconcile, the list at connect) each need a proof from the rule checker? | Yes (V8, W05 PD29 and OQ9). A dispatch order may make, under its own consumed authorization, one GET of its target event before the change (invitation, response) or after a 2xx delete; such a GET is not a broker read and needs no system proof (D.4; ruled 2026-11-18 by the lead) | No calendar call without a proof id; one invariant to test | No: fewer checks and no TH1 wait, but calendar reads with no proof behind them | before P3 merges (migration 2 holds `system_proofs` and `observations.proof_id`) |
| OD8 | What must the approval of an agent tell him? | Five things, on the approval step: the permission (the agent may then list his selected calendars until he revokes that permission or removes the agent); where the client he named normally sends data, and that Fabric cannot check it (OPC9; P2-22); that the permission covers calendars he selects and accounts he connects later; what a calendar list holds (the account, each calendar's name, its access level and whether it is Agent Drafts, and no events); and that he can revoke this permission alone (P5; SR-4) | He gives a standing permission with one click; the text is all he sees of it | Fewer items: he approves without knowing the permission's reach | before P6 merges (the screens session builds it) |
| OD9 | Accept the amendments for the practice mode (E.1.9): a real agent on the practice calendar in debug builds, practice on with the test key and Isabelle without the synthetic agent, the hook removed, and the new labels | Accept | They follow his answer NOV17-PRACTICE and remove no guard (E.1.7) | The practice calendar stays with the synthetic agent only; a real agent's first run is on Google at G2c, without a wiring check before | before P5 (hook) and P6 merge |
| OD10 | G2a is put to you after step P5. The row "poll a completed read after revocation" is then shown by the Q10 tests on the practice provider, not on Google. Accept that at G2a? | Yes; the same cases run on Google at G2b and G2c | NOV17-STORE kept the default and said his answer is needed before G2a; W04 stores no results, so only the practice provider can show the row before Google | No: G2a waits for G2b | before G2a is put to him, after P5 |
| OD11 | Accept the amendments of W04 D17, D18, D33 and the meaning of "used" (section B): a permit with no grant only for the broker's own reads and for the release of an agent's own change's outcome (what that means for him is OD15); a one-request read permission lasts until its result is released or purged. W04 D14 is OD18 | Accept | Reads and releases cannot work under the W04 wording (REV-results D17, D18, D33) | Results stay out of version 1 | before P5 merges |
| OD12 | Accept the amendment of W04 D27 for the Google variables (section D, CO10) and the refusal of the test key with Isabelle and Google together (ED2) | Accept | A misspelt name stays an error; a practice run never meets Google | A configuration with both would need a routing design between two providers | before P7b merges |
| OD13 | When he reconnects a connected Google account, the account shows "waiting for consent" for up to 300 s and its agents stop meanwhile. Accept, with the page saying so? | Accept (OP7) | The contract's `awaiting_consent` is the only state for it; no contract change | A contract change for a second state | before P7b merges |
| OD14 | Google's client switch "Use this client for an AI-powered agent" cannot be changed after a client is created. For the development client you answered on 2026-11-17: off (W05 18.2). Off for the release client too? | Off | Google requires it only for clients that call its hosted Workspace MCP servers; Fabric calls the Calendar API directly (RUNBOOK 6.3) | On: the release client is created with the switch on | before the release client is created (CD-W11) |
| OD15 | After you revoke every permission of an agent it can still fetch the id and revision of each change it made, no title and no time, until you pause or remove it, disconnect the account or deselect the calendar. Accept? | Yes (BD11, lift c; pinned by Q20) | The agent needs the id to update or delete what it made (W06 LR-C4 (2)); it is metadata of its own action | The contract session changes `rows (Disclose change)` in the theory so that a grant is needed; until then the outcome is not released | before P5 merges |
| OD16 | A read result that waits for its release stays in the store when you revoke the permission it was read under, and can still be released if another permission of yours covers the same read. Keep it? | Keep (B.9) | PLAN 9.2 allows a later release under a still-valid broader grant and a fresh proof (PLAN `:333`); the release is refused unless such a grant exists (B.5.4) | Purge it when every grant the read relied on has ended, so a revoked read is never released later (REV-results retention-OQ5) | before P5 merges |
| OD17 | A result whose answer is lost in the instant it is sent (a full writer queue, a broken connection) is gone: it was purged at its release, and the agent reads again. The same holds for a call the client cancelled while its result is released, until P2-30's cancel message exists and for a cancel that arrives after the release commit; under a permission for one request that permission is then used up (B.8.4). Accept? | Accept (OP13; B.2) | A second copy would outlive its disclosure (V4) | Keep the result until the agent's next call confirms receipt: a second release path and a longer life of content | before P5 merges |
| OD18 | Accept the amendment of W04 D14 (E.1.9): with no prover attached, or while it is not ready (the synthetic classifier of a debug build counts as ready), a call that would create a request is refused `prover_unavailable` and nothing is recorded; a request already open still ends `undecided` | Accept (P8; B.10; I.3 B-7). The synthetic exemption is the lead's, ruled 2026-11-17 (I.3 B-8), and is not part of the question | A refusal tells the agent to retry later and records nothing | W04 D14 stays: such calls are recorded and end `undecided` at once | before P5 merges |
| OD19 | Accept the amendments of C.12: W04 D25 (agent entry), D11 (developer seams), D28 (locations), L2 (the peer's user id), 5.5 (sessions), 6.1 (the owner methods) | Accept | They follow P9 and the contract draft | The agent channel cannot be built as P9 asks | before P6 merges |
| OD20 | Accept the amendments of D.15 other than D27 (W04 5.6 twice, 5.7, 5.8, D38 and R24) and A.3's W04 D3, 3.5, section 4, 9.2 and R22 | Accept | They state the fence, the epoch raises, reconciliation, recovery, the account budget and the store's writers as built | The steps that build them wait | before P3 (A.3), P5 (5.6, 5.7, 5.8) and P7b (D38, R24) merge |
| OD21 | Accept two new dependencies of the broker binary: `nix` (unix only, features `user` and `socket`, with `cfg_aliases`) for the peer's user id, and the bridge's library `fabric-mcp-bridge` for `config_text` | Accept (OPC1, OPC2) | `std` has no stable call for the peer's user id and `unsafe` is forbidden (C.1 CD16); one function makes the configuration text (W06 A24) | Without `nix` the peer check of P9 cannot be built; without the bridge library the configuration text is written twice | before P6 merges |
| OD22 | Accept these residuals, one line each: a 48-bit pairing code can collide before you look (OPC8); same-user software that runs the bundled bridge has the agent's key (OPC9); a process that knows a pending id can force you to remove and add the agent again after 20 key changes (OPC10); a purged result stays in older write-ahead-log frames until the next checkpoint (AO4); a user whose short name is longer than 18 bytes cannot use the agent channel of the installed application, because the socket path is too long (OPC5; the alternative, the broker binding by the relative name from inside its `run` directory, changes how the binary starts its listener and is decided at CD-W11). And these defaults: no `HOME` in the broker's environment, so a bridge under `$HOME/Applications` gets no text (OPC3); repeated wrong keys show only in the audit (OPC4); no warning at a disconnect while a change of that account is unknown (D-4, W05 OQ16) | Accept each | Each is bounded as section C, A or D says | OPC5, OPC8, OPC9, OPC10 and AO4 have no alternative in version 1 and are told to you, not asked; OPC3 (add `HOME` to the relay's allowlist), OPC4 (show wrong keys on the Agents screen) and D-4 (warn before a disconnect) are choices | before P6 merges (the OPC rows); before the G2b live run (D-4) |
| OD23 | The G2b live run uses a debug build with a Keychain store in its own `FABRIC_DESKTOP_DATA_DIR`, which keeps real tokens in the Keychain (ER5, W5-9). Accept? | Accept | ED2 refuses the test key with Isabelle and Google, so the run cannot use the test key | Another form of the live run, named by section D | before the G2b live run |
| OD24 | A calendar list approved through a card for one request delivers nothing: the list is cancelled and the permission ends used. Under `list_and_availability` in mode `ask` no calendar list ever completes in version 1. Accept? | Accept for version 1; the card says so (SR-7) | No result is released without a waiting call (P6), and the list's call has left by the time he approves (BD5) | BOP1's carry-over: an unused one-request list permission stays live after the list was cancelled and serves the agent's next list (the same action digest), built in step P5b | Answered 2026-11-18: accepted for version 1 (PLAN 15.3). It was needed before P2 merges (P2-2, P2-3, P2-29 write the default into the contract README) |
| OD25 | When Google applies a change but returns content that differs from what you approved, the request waits for you, and you can only resolve it as applied. Before the live run every difference waits; after it, only the rewrites shown to keep your meaning are accepted silently. Accept? | Accept (V23; B.14, D.7.1). The adapter's list of accepted rewrites is empty until the live run shows each one (W05 18.7, LV18) | A 200 with the right id means Google made the change. Sending it again, or reading it back and finding it "absent", could let a new request repeat a change that notifies guests. Only the live run shows which rewrites Google makes; its documentation names none | Such a change is handled as an unknown outcome and reconciled by reading, as the first rule did: a change Google rewrote without moving its revision can then end absent after two reads, and a new request can repeat it. Or every difference is accepted silently, and a real difference is hidden from you | before the G2b live run |
| OD26 | An event whose record would pass 1 MiB (its before-view 512 KiB, or less if the frame test lowers it) cannot be changed or read through Fabric in version 1. The request ends with the fixed sentence "Google's copy of this event is too large for Fabric to keep a record of. Nothing was changed. Make this change in Google Calendar." and no card opens. Accept this limit? | Accept (D.6.2; `MAX_EVIDENCE_BYTES`, `MAX_BEFORE_VIEW_BYTES`) | Every observation is kept for good, and your card must fit one 1 MiB frame (`MAX_FRAME_BYTES`). An event with some thousand attendees can pass 256 KiB, so the first bound of 256 KiB is raised, and the refusal stays (W05 RK21; the CD-W05 session measures events of 1 000 and 2 000 attendees, LV16) | A larger bound needs a card that does not carry the whole before-view in one frame, or evidence that is cut or deleted: a contract and store change before the live run. A smaller bound refuses more events | before the G2b live run |

---------------------------------------------------------------------------------------------------------------------

### E.4 Risks of the whole plan

| # | Risk | What limits it |
|---|---|---|
| ER1 | The chain is long: P0, P2, P3, P4, P5, P6, P7b, then G2b and G2c. A slip in P2 holds everything but P1 and the phase 1 pull requests | The screens, the bridge and GC run beside the chain (E.2.3) |
| ER2 | PR 36 is a second writer in the lead's core and storage files | P2 starts after P0 and is rebased on it; P3 and P4 start after P2 merges |
| ER3 | P5 is large and every part of it is in `pipeline.rs`: one writer, a long review | the split P5a, P5b; the fence fix and its test in P5a before any revision is set (REV-port cross-cutting) |
| ER4 | If the owner refuses OD5 (b), held results have no store; OD1's path goes with it | OD5 is asked before P3, the first step that builds on it |
| ER5 | ED2 refuses the test key with Isabelle and Google: the CD-W05 process tests against FAKE through the binary cannot reach a permit | they run in process, feeding `Input::ProofDone` (W04 D13); the G2b live run uses a debug build with a Keychain store in its own `FABRIC_DESKTOP_DATA_DIR`, which keeps real tokens in the Keychain (PLAN R13). The lead kept ED2 literal on 2026-11-17 (I.6 E-1) |
| ER6 | From P6 every debug run with the test key and Isabelle is a practice run; live tests started that way seed an account and may change what they show | e18 runs each by name in P6 |
| ER7 | An agent believes a practice effect is real | ED9 labels; P2-19 asked; the calendar's name; G2c reports name the provider (ED11) |
| ER8 | A practice change that asks to notify is refused, where Google would carry it out | the refusal's text says what to do; the alternative would report a notification that never happened |
| ER9 | Each update and delete now costs one more Isabelle check (the observe's system proof), and a read four plus the release; under load a held answer passes 20 s | the answer is then the status and the agent polls (P7); latency is measured on the practice calendar in step P5 (I.6 E-8), not here |
| ER10 | The page and the bridge learn `completed` and the two-call read after the core sends them | the screens' `completed` text before P5; the bridge's guidance in bridge phase 2 before P6 |
| ER11 | TH1 slips: no list order, so no Google connect | the practice run needs no list order (E.1.4); P7b waits on TH1 explicitly |
| ER12 | A reused scratch store from a practice run opened later as a Google run (or the reverse) holds the other run's account | `can_carry_out` and practice row 3 order nothing for it (ED7); the README says one new directory per run |
| ER13 | A create whose outcome was unknown can leave two events (OD6) | the reserved id kept and shown |
| ER14 | The owner's time: twenty-six decisions and two live runs at the Mac | each decision has a default and a "needed" step; OD1, OD2 and OD24, needed before step P2, were answered on 2026-11-18; the first still needed are OD5 (with the A.3 part of OD20) and OD7 before step P3, and these two are the ones he must answer first |

---------------------------------------------------------------------------------------------------------------------

### E.5 Not checked

- No code was built or run, and no test. Every "today" is read from main `2d41210`.
- P0's code was read only as a file list (`git diff --stat`), not its content; nothing here relies on its exact
  shapes (the P0 row of 0.1).
- The contract draft was read at its README, its agent and owner schemas for the handshake and `hello`, and
  `crates/fabric-desktop-contracts/src/agent.rs`; it may move before P2.
- The Google variable names are D.13's (`FABRIC_BROKER_GOOGLE_CLIENT_FILE`, `FABRIC_BROKER_GOOGLE_LOOPBACK_PORT`);
  E.1.1 and ED2 use them by role only.
- That Isabelle proves `System_refresh` for the practice account (connected, one selected calendar) was not run; it
  is checked in step P5 on the practice calendar (I.6 E-8).
- The socket path's length on the owner's Mac; whether `build-desktop.sh` can bundle the bridge in a debug build
  (W06 7.5); the time of the extra system proofs (ER9), measured in step P5 (I.6 E-8).
- Whether the live tests of PW 7.3 change outcome as practice runs (e18 checks it in P6).
- The screens' texts for `completed`, for an empty-scope grant and for a removed agent's requests: not designed here.

---------------------------------------------------------------------------------------------------------------------

### E.6 Names and requests added by this section

| Name or request | Where | Why |
|---|---|---|
| `Config.google: Option<GoogleSettings>` and the practice term `google.is_none()` | BR `config.rs`, step P7b | ED1, ED2; section D names the type and variables |
| `ProviderKind::Practice` under `cfg(debug_assertions)` | CORE, 0.5.3 | ED8 |
| `system_action_digest` public (the first draft of 0.5.1 had `pub(crate)`) | CORE | an adapter outside the core (the practice provider, GC) must check C2 / PO8 |
| `practice::PRACTICE_CALENDAR_NAME`, `practice::account()`, `practice::seed()`, `Shell.practice_seeded`, `Shell::seed_practice` | BR | E.1.3 |
| `Faults.unknown_not_applied: u32` | BR `practice.rs` | ED6, row 6a |
| `Faults.applied_differs: u32` | BR `practice.rs` | V23, row 8a (ruled 2026-11-18 by the lead) |
| `BRIDGE_KEY_DIR = "bridge-keys"` under the data directory, debug builds with the test key | BR | ED10; section C's configuration text carries it |
| The practice target rule and `can_carry_out(Practice, _, false) = true` | CORE, section D | ED7 |
| P2-19 asked, no longer optional | section G | ED9 |
| P2-24 (optional; renumbered from this section's P2-21, X5): the owner's `hello.capabilities` gains `practice_calendar` in a practice run (README section 7: "No capability name is defined yet") | contract session | lets the page show the practice line without reading the build string |
| Test ids e01 to e19 | E.1.12 | |
| Owner decision ids OD1 to OD26, risk ids ER1 to ER14 | E.3, E.4 | |

---------------------------------------------------------------------------------------------------------------------

## F. Appendix: the answer to every request id

One row per id of W05 (sections 9 to 12) and W06 (7.1 and 7.2). "Section" is where the answer is explained;
the first one named owns it.

| Id | Asked (short) | Verdict | Section |
|---|---|---|---|
| PO1 | a calendar target on every order, resolved by the core | Accepted: `CalendarTarget`, resolved in the sending commit; unresolvable after a held fence is a store fault | D.3, D.2 |
| PO2 | an observe order under `System_refresh` | Accepted: stored before the proof, the proof starts in that commit, the fence carries the stored revision, the shell hook goes | D.6, D.10 |
| PO3 | a read order, every page, a `still_allowed` flag | Accepted: own order, no `effects` row; the flag is a hint kept in `OrderInFlight` and cleared after every commit that raises an epoch (B.13), the storing commit re-checks authority | D.2, B.3.3, B.3.4, B.13 |
| PO4 | a reconcile order; the core applies `reconcile::decide` | Accepted in the review's form: the provider answers `ReadBack`, the core owns the table; `decide` becomes a test oracle | D.7.3 |
| PO5 | a calendar-list order under a system proof | Accepted, not built before TH1 (P2-6); Google connection waits | D.9.1; open OP6 |
| PO6 | `Applied { confirmed, notification }`; account signals | Accepted: event recorded by `EffectEventRecorded`; an unfitting `Applied` or an unconfirmed delete is unknown, a fitting unconfirmed one waits for the owner (V23, ruled 2026-11-18); a delete is confirmed by one GET in its order; one optional signal per answer | D.2, D.7.1, D.8.2, B.14 |
| PO7 | sign-in on its own worker | Accepted as `AccountLink` on two workers, sign-in and unlink (ruled 2026-11-18, DD18); the core rechecks subject and scopes | D.2, D.8 |
| PO8 | every order names its proof | Accepted: `OrderProof`, digest checked by the worker, FAKE log test | D.4 |
| ST1 | `provider_accounts` | Accepted with this change: row at `connect_google`, `subject` null until connected (V16), no token | A.5, A.9 |
| ST2 | `provider_calendars` | Accepted with this change: core-made `cal-` ids, never deleted, 250 per account | A.5, A.9 |
| ST3 | observations and evidence | Accepted with this change: in migration 2, not 3 (V1) | A.4, A.9; ruled 2026-11-17 (OP1) |
| ST4 | held results | Accepted with this change: `held_results`, `held_result_payloads`, `releases`; one design for both packages | A.4, A.7, B.9 |
| ST5 | event columns of `effects` | Accepted with this change: three columns only (V2) | A.4.1, A.9 |
| ST6 | "Fabric created this" from the store | Accepted: DD12, no schema change | D.0, A.9 |
| ST7 | `applied` among open effects | Not built (V7); ruled by the lead 2026-11-17, agreed by the CD-W05 session 2026-11-18 (W5-6) | D.13, A.9; OP5 |
| ST8 | one list of operation types | Accepted with this change: one dated amendment of ADR-115 D2, 34, 39, 40 with P2-11 | A.6, A.3 |
| ST9 | no token in the store | Accepted; a secrecy test with marker tokens ships with migration 3 | A.7, A.9 |
| ST10 | the snapshot budget and a cap on accounts | Decision needed: 96 KiB reserve per account, the owner decides the cap | D.11; owner OD4 |
| ST11 | a frozen version 1 store and the rehearsal | Accepted with this change: fixtures made by the version 1 code at two commits plus a SQLCipher file | A.8 |
| CO1 | `connect_google` | Accepted with a connect generation, one epoch raise, the budget check | D.8.1 |
| CO2 | at most one never-connected account | Accepted: `subject` null, reused | D.8.1 |
| CO3 | recovery cancels `awaiting_consent` | Changed: a re-consent returns to its state before | D.8.4 |
| CO4 | `disconnect_google` with revoke, forget, purge | Accepted with `outcome_unknown -> reconciling -> needs_owner_resolution`; revoke and forget as one `unlink` job, repeated at start while pending (ruled 2026-11-18, DD18) | D.8.3, D.8.4 |
| CO5 | reads, held results, `fabric_request_result` as `Disclose` | Accepted with changes: two calls, `read_attempts`, the storing re-check, release as its own request | B.17, B.3, B.4, B.5 |
| CO6 | observation before each proof | Accepted on existing edges, with the stored observation and the fence of DD5 | D.6 |
| CO7 | calendar list; reductions end grants | Accepted: the record is rebuilt; no epoch when nothing changed | D.9.2 |
| CO8 | reconciliation | Accepted: durable read counts, due times through recovery, waits while disconnected | D.7 |
| CO9 | system proofs | Accepted as a separate path (`Job::ProveSystem`) | B.11 |
| CO10 | the client file and FAKE's loopback port | Accepted as the binary's; amendment of W04 D27 for the owner | D.13; owner OD12 |
| CO11 | `AuthLost` | Accepted | D.8.2 |
| CO12 | re-observe a waiting card | Deferred | D.13 |
| CO13 | `fabric_calendar_list` from the store | Accepted: released by its own fresh permit, ends `completed` | B.3.2, B.4.2 |
| CO14 | the amendments and reducer rows | Accepted: amendments in B, reconcile and stale rows in D | B.6, B.7, B.17.1, B.3.7, D.13 |
| AM-D17 | lift D17 for reads and releases | Lifted narrowly, three cases | B.6; owner OD11 |
| AM-D18 | a read's one-request grant lives until release or purge | As asked, with the touch points and end reasons | B.7.1; owner OD11 |
| AM-D33 | D33 goes | Replaced by a written successor | B.17.1; owner OD11 |
| AM-USED | `used` of a read means released | As asked | B.7.2; owner OD11 |
| LR-S1 | the enrollment's key, state and times | Accepted with this change: in `enrollments`, not on `AgentRow` | A.9, A.4.1 |
| LR-S2 | one migration for the agent channel | Accepted with this change: everything in migration 2 | A.9, A.6 |
| LR-S3 | the list permission in the store | Accepted with this change: form (a), table `enrollment_grants` | A.9, C.4.1 |
| LR-S4 | audit of failed handshakes | Accepted: audit only, one row per cause and known id per minute | C.6.5, A.9 |
| LR-C1 | enrollment commands and the list permission | Accepted as DRAFT shapes them | C.2 to C.4 |
| LR-C2 | handshake decisions after the signature | Accepted: all in `service/handshake.rs` | C.6 |
| LR-C3 | `prover_unavailable` while not ready | Accepted: refused at once, nothing written | C.8, B.10 |
| LR-C4 | held answers; results for G2c | Accepted: memory only, 16 per connection, 20 s; the list, a change's outcome, reads | B.8, B.3, B.4 |
| LR-C5 | a status never carries content; the poll test | Accepted: B.2 R1 to R6 and the twenty-one cases of question 10 | B.2, B.16.1 |
| LR-C6 | close a removed agent's connection | Accepted: in the removal's step | C.3.3, C.7.3 |
| LR-C7 | ports for the handshake | Accepted: `Line`, `Oversized`, `Refused`, `Output.bound` | C.5 |
| LR-B1 | the listener | Accepted | C.9 |
| LR-B2 | path, modes, stale socket | Accepted: stale socket removed only under the store's lock; the length check's failure is one stderr line and is visible in `health` through P2-18, which is asked (not optional) | C.9.1; P2-18 |
| LR-B3 | the peer's user id | Accepted: `nix`, at accept | C.9.3; owner OPC1 |
| LR-B4 | no new relay name | Confirmed for the socket; `HOME` would be needed for `$HOME/Applications` | C.10; owner OPC3 |
| LR-B5 | the configuration text | Accepted: `ConfigurationTexts` from the binary, dependency on the bridge library | C.10; ruled 2026-11-17 (OPC2); owner OD21 open |
| LR-B6 | a practice mode for a real agent | Accepted: row 4 of E.1.2, no script, the shell's seed | E.1.10 |

---------------------------------------------------------------------------------------------------------------------

## G. Requests to the contract session

Each is small and exact, against DRAFT at `29cdfdf`. They replace the separate V3 numbering of W05 section 13 and W06
7.3 (REV-order S16-1); W06's V3-1 to V3-7 are met by DRAFT except as below. "Status at `b94b58f`" says what the
draft has done since.

| # | Request | Section | Status at `b94b58f` |
|---|---|---|---|
| P2-1 | Lifecycle: add `{"state": "completed", "terminal": true}` and the edge `proving_fresh_permit -> completed`, basis `added`, reason "plan 9.2: a request the broker completes itself (a calendar list, a result release) has no provider step"; `RequestState::Completed`, `is_terminal`, `can_transition_to`, `PlainRequestState`; the agent and owner status schemas. README: 26 states, 18 added transitions | B.3.1 | Met, with the reason "plan 9.2: Allow returns the proven result; a calendar list and the release of a result have no call to Google" (LC; README choice 68) |
| P2-2 | README section 8, after "When the answer comes": "A `fabric_calendar_list` answer carries the list its own fresh proof released. A `fabric_request_result` answer carries the result its own `Disclose` proof released. Only a call that still waits receives a result; if the call has ended, the request ends `cancelled` and nothing is released. A read's own answer never carries its result; the agent calls `fabric_request_result`. `fabric_request_result` of a list, of another result request, or of a request with no result ends `terminal_failure` with `invalid_arguments`". Also replace "and an Allow returns the result" (README line 190) with "an Allow of a list returns the list; an allowed read is answered when its result is held, and the agent then calls `fabric_request_result`", and add "its outcome is unknown" to the states that answer an action (P7) | B.2, B.3, B.8 | Open |
| P2-3 | README sections 8 and 15: a release that needs the owner's approval ends `undecided`; no card is opened for a release in version 1 (W05 V3-10) | B.3.5 | Open |
| P2-4 | `GrantEndReason` gains `result_expired`: a one-request read grant whose held result passed its life unreleased | B.7.1 | Open |
| P2-5 | System proofs in request details (W05 V3-8): `$defs/system_proof_report` `{proof_id, action: {kind: refresh \| reconcile \| list_calendars, account_id, calendar_id \| null, event_id \| null}, outcome, prover_kind, recorded_at}`; `request_details` gains `system_proofs: [system_proof_report]` | B.11.4 | Open |
| P2-6 | Theory TH1: a system action `System_list acct`, authorized when the account is connected; `system_authorized` and `is_system` cases; a real Isabelle run | D.9.1 | Open |
| P2-7 | `UndecidedReason` gains `provider_unreachable` (W05 V3-9) | D.6.3 | Open |
| P2-8 | `calendar_access` gains `writer_without_private_access` (W05 V3-1) | D.9.2 | Open |
| P2-9 | Snapshot member `provider_accounts: [{account_id, last_connect_problem \| null, missing_scope \| null, refresh_token_ends_at \| null, revocation: not_requested \| pending \| revoked \| not_confirmed}]` beside `accounts`, and a change `provider_account`; `Account` is unchanged (W05 V3-2, V3-3) | D.8 | Open |
| P2-10 | Request details gain `notification: sent \| not_requested \| unknown \| null` for an applied change (W05 V3-4) | D.7.3 | Open |
| P2-11 | An owner command `refresh_calendars {account_id}` (command true) that lists one account's calendars again (W05 V3-5); it commits and returns `commit_sequence`, operation type `owner.refresh_calendars` | D.9.1, A.6 | Open |
| P2-12 | README: `occurrence` in an event view is disclosed with `time` and absent for an occurrence of an all-day series (W05 V3-6) | D | Open |
| P2-13 | Theory TH2: what `ev_invitation` and `ev_attendees` mean on a calendar that is not the account's own (W05 TH2) | D | Open |
| P2-14 | A test in the contract crate that every version 2 fixture of `Grant`, `Request`, `ApprovalCard`, `Approval`, `Account`, `Attempt`, `Authorization`, `ProofReport`, `CustomProfile`, `Preferences`, `RecipientSet`, `Action` and `Scope` decodes under version 3 and re-encodes byte for byte (A.2 R7) | A.2, A.11 | Met by `crates/fabric-desktop-contracts/tests/stored.rs`, except the finding of P2-26 |
| P2-15 | README section 10: a grant made by an agent's approval has purpose `""`, tools `[fabric_calendar_list]`, scope calendars `[]` with every other scope member null, and term `until_revoked`; it ends only `revoked_by_owner` or `agent_removed` | C.4.1 | Open |
| P2-16 | README section 8: after `not_permitted` to a frame before an approved handshake, and after every `agent_prove` that does not answer `approved` (`pending` included) and every refused `agent_hello`, the broker closes the connection (V12; C.6.2 to C.6.4, C.7.3). Today README section 8 lists closes only at session end, removal and quit | C.6.2 | Open |
| P2-17 | Optional: `BusyLimit` gains `sessions_per_minute_per_agent`, so that `agent_prove` past the session rate can answer `busy` with a wait instead of `not_permitted` | C.6.4 row e | Open |
| P2-18 | Asked (not optional; W06 LR-B2 asks for a failure visible in `health`): `health.agent_channel`: `listening`, or `not_listening` with a problem from `path_too_long`, `run_dir_unsafe`, `stale_not_socket`, `socket_mode`, `bind_failed`, and `run_dir_owner` if the owner check of `run/` is reported apart from `run_dir_unsafe` | C.9.1 | Open |
| P2-19 | Asked (not optional): `result_agent_prove` gains `practice: bool`, true exactly in a practice run, so the bridge can tell the agent before its first list (P11, ED9) | E.1.6 | Open |
| P2-20 | Optional: the lemma `a_read_permit_is_its_release` in `isabelle/Fabric_Broker_Examples` (PART-results C3) | B | Open |
| P2-21 | `owner#/$defs/result_approve_agent` gains `grant: grant` (the list grant made in the same commit); CC `ApproveAgentResult` gains `pub grant: Grant` (P5: "approve_agent returns the grant"; DRAFT CC `owner.rs:1924-1930` has `{commit_sequence, agent}` only). Until it merges the page reads the grant from the `Change::Grant` notice of the same commit | C.3.2 | Open |
| P2-22 | A data table of the destination of each `add_agent` client (`data/agent-clients.json`, or a README table): `claude_desktop` and `claude_code` `{kind: cloud, processor: "Anthropic"}`, `codex_desktop` `{kind: cloud, processor: "OpenAI"}`. The table is a label for the client the owner named, not a checked fact: the broker cannot check which program holds the key or where it sends data (OPC9). Until it exists the core's table of C.3.1 is the rule | C.3.1 | Open |
| P2-23 | README section 15: the handshake limits are the broker's (5 s, 8 at once, 6 sessions per agent per minute, one key change per minute and 20 in all while pending) and what it records of a failed one (one audit row per minute per cause and known id); remove that line from "Not covered" | C.6.6 | Open |
| P2-24 | Optional: the owner's `hello.capabilities` gains `practice_calendar` in a practice run (README section 7: "No capability name is defined yet"), so the page need not read the build string | E.1.6 | Open |
| P2-25 | With every request state the contract adds, the same pull request edits invariant `I1` (ST `store.rs:924-933`), and every change to replay code that is not a widening carries the record A.2 R7 asks: why no committed row is affected (as A.2 does for `needs_owner_resolution`) | A.2, A.4.7 | Met for `completed` (DRAFT `store.rs` at `b94b58f`); open as a standing rule |
| P2-26 | A version 2 `Agent` record and a stored `pause_agent` answer must decode under version 3. DRAFT's test `a_version_2_agent_and_a_stored_pause_agent_answer_do_not_decode_under_version_3` records that they do not, because version 3 adds required members to the agent record; the core decodes a stored answer when a command id comes back (CORE `owner/dedup.rs:121-127`), so a repeated `pause_agent` after the upgrade is a `StoreFault` and halts the broker. A.2 R6 asks that the new members be optional and defaulted when read. Needed before step P3 (I.7 X-2, the lead's ruling of 2026-11-17) | A.2 R6, R7 | Open (found at `b94b58f`) |
| P2-27 | `request_details` gains `releases: [{release_id, request_id, kind, proof_id, grant_ids, released_at}]`, the release records of B.4.3 that name the request as releasing request or subject, so the owner sees what left the broker (SR-10) | B.4.3 | Open |
| P2-28 | README section 7 states seven answers: `conflict` for approving an agent that is not pending and for removing one that is removed; `conflict` for pausing a pending or removed agent; `invalid_params` at `/params` for the owner budget at approval; `not_found` for a removed agent's configuration; `invalid_params` at `/params/registered_name` when `add_agent` would pass the owner budget; `invalid_params` at `/params/account_id` when `connect_google` would; and "A proved key that differs from the agent's current key raises the agent's revision; proving the current key again changes nothing" in place of "Every proved key raises the agent's revision" (C.3.1 to C.3.5, C.6.4 row h, D.11) | C.15.1 | Open |
| P2-29 | README section 10 states that a one-request approval of a calendar list delivers nothing in version 1 (the list is cancelled because its call has left), so under a row in mode `ask` no list completes; the section's rule "the broker does not propose a duration whose grant would not serve the request" does not hold for a list (BD5, OD24) | B.1, C.4.2 | Open |
| P2-30 | Agent channel, schema version 3: a cancel message from the adapter, so that a call the MCP client cancelled (`notifications/cancelled`) stops waiting in the broker (the CD-W06 design's R14). The adapter names the id of a call it no longer waits for (for example method `agent_cancel` with params `{call_id}`; the name is the contract session's). The broker drops that held answer without a reply, exactly as for a connection that ended, so a release that finds no waiting call releases nothing and the request ends as B says for that case (BD3, BD4). A cancel for an id that is not held is ignored. It is not a request of its own and creates none. README section 8 states it. Asked by the lead's ruling of 2026-11-18 | B.8.4 | Open |
| P2-31 | Request details gain the reason a request waits in `needs_owner_resolution`, as `owner_resolution_reason: unknown_outcome \| applied_differs \| read_failed \| account_disconnected \| null`, set only in `needs_owner_resolution`, as `undecided_reason` is set only in `undecided` today. `applied_differs` is V23's reason: Google applied the change and returned content that differs from the approval. Without it the owner sees a generic unknown outcome and cannot tell that Google holds something other than he approved. Needed before any Google dispatch, that is before the G2b live run (E.2.1). Asked by the lead's ruling of 2026-11-18 | B.14, D.7.3 | Open |
| P2-32 | `PrivateDetails` gains `after: EventView \| null`: the event as Google returned it, for a request of V23, shown only with `full` redaction, as `private.before` is. It is bounded by `MAX_BEFORE_VIEW_BYTES`, as the before-view is, so that both views and the rest of `get_request_details` fit `MAX_FRAME_BYTES` (D.6.2's frame test). Without it the owner cannot see what Google holds before he resolves the request. Needed before any Google dispatch (the G2b live run). Asked by the lead's ruling of 2026-11-18 | B.14, D.6.2 | Open |
| P2-33 | Lifecycle: the edge `dispatching -> needs_owner_resolution`, basis `added`, reason "Google applied the change and returned content that differs from the approval". README choice 53 says the owner resolves only after the broker's own reconciliation gave up; a V23 request reaches the state with no reconciliation. Until it merges the core uses `dispatching -> outcome_unknown -> reconciling -> needs_owner_resolution` with no read (B.14), which shows a path that did not happen. Needed before any Google dispatch (the G2b live run). Asked by the lead's ruling of 2026-11-18 | B.14, D.7.1 | Open |
| P2-34 | README section 7: for a request whose reason is `applied_differs`, `resolve_outcome` accepts only `applied` (the owner has seen what Google holds); `absent` is refused. Today README line 115 offers both resolutions for every request in `needs_owner_resolution`. The core adds no interim refusal of its own (D.7.5). Needed before any Google dispatch (the G2b live run). Asked by the lead's ruling of 2026-11-18 | D.7.5, B.14 | Open |

Questions to the contract session (theory owner), from D.14:

| # | Question | Default the design runs on |
|---|---|---|
| CQ-D1 | What a pending effect is. Should the theory text read "in `dispatching`, `outcome_unknown` or `reconciling`", now that `needs_owner_resolution` is open? The store is not widened (P10) | the store's three states |
| CQ-D2 | `System_refresh` names no event, so one proof permits a read of any event of the calendar. Should it name the event? Is the observe made for an agent's classification a system action? | as THEORY has it; a system action |
| CQ-D3 | TH1 (P2-6), `System_list acct` | no list order and no Google connection until it merges |
| CQ-D4 | `params_select_calendars.selected` allows 250 ids (W05 PD23) | 200 ids, as DRAFT has it (DRAFT CC `owner.rs:1850`); at most 200 selected calendars per account (D.9.3) |

Not asked, on purpose: W05 V3-7 (a read past its bounds fails as a whole); a nullable `authorization.grant_id` (a
change's outcome is released with no authorization row, recorded only in `releases`); W05 ST7 (V7).

---------------------------------------------------------------------------------------------------------------------

## H. Requests to the screens, CD-W05 and CD-W06 sessions

### H.1 To the screens session

SR-1 to SR-8 are section C's (C.15.2): the Agents screen, the pending agent and its code, the answers of
`approve_agent`, what the approval step must tell the owner (P5, owner OD8), an approved agent, a removed agent, the
card for a list, and Google's credentials. Added by the other sections:

| # | Request | Needed | Section |
|---|---|---|---|
| SR-9 | Word the state `completed` in the page before step P5 merges | before P5 | B.3.1, E.2.2 |
| SR-10 | A read is answered `reconciled` with no result; its content comes in a second call. Request details show a release record once the contract has it (P2-27) | before P6 | B.4.3, B.16 |
| SR-11 | Show `#practice-line` (`apps/fabric-desktop/ui/index.html:15`) only when the `hello` build ends with ` practice` (or, after P2-24, by the capability) | before P6 | E.1.6 |
| SR-12 | `apps/fabric-desktop/README.md` "The practice run" and the harness header (`harness/serve.ts:10-19`) carry the wording of E.1.9 | before P6 | E.1.9 |
| SR-13 | `absence_confirmed` is worded "not found", not "confirmed absent"; the reason shown for `needs_owner_resolution` comes from the latest reconcile observation | before P7b | D.7.3 |
| SR-14 | A connected account given to `connect_google` shows "waiting for consent" for up to 300 s and its agents stop meanwhile; say so (OP7, owner OD13) | before P7b | D.8.1 |
| SR-15 | The Google screens: S01 account states, `provider_accounts` (P2-9), `undecided_reason` `provider_unreachable` (P2-7), `notification` (P2-10), `refresh_calendars` (P2-11), system proofs in request details (P2-5) | before the G2b live run | E.2.1, D, B.11.4 |
| SR-16 | A card for a list proposes `one_request` first (the duration rule; NOV17-DURATION), and says, as SR-7, that approving for one request lets nothing through (OD24) | before P6 | C.4.2 |

### H.2 To the CD-W05 session

| # | Request | Section |
|---|---|---|
| W5-1 | Adopt V1, V2 and V20: drop the design's own migration number and its ST5 columns but `notification`; adopt `releases` and `request.release` | E.2.2, A.9 |
| W5-2 | GC phase 2 implements `Provider` and `AccountLink` of D.2 once step P4 merges: orders carry a `CalendarTarget` and an `OrderProof` whose digest the worker checks (DD2; `system_action_digest` is `pub`); nothing is started or sent past `until`, the shell's monotonic stamp plus `PROVIDER_CALL_MS`, and every HTTP call is bounded by it (B.13, D.5, X23); `Transient` only when nothing was sent or the request was refused before acting, every 5xx and unreadable 2xx `NoAnswer` (DD6); `Applied` carries `confirmed` and `notification`; 404 and 410 on an update or delete answer `revision_conflict`; a list past 250 entries or 20 pages fails as a whole; `ProviderRef` validation on the observe order; the adapter still cuts an event view to the fields the action names and to its window (W05 PD24) and cuts every busy interval to the window (W05 6.3); a read answer carries `spans`, one typed `ReadSpan` for each event, from `start(date,dateTime)` and `end(date,dateTime)` asked of Google for every event whatever the grant, with no widening (D.2, DD17, W5-14); the core checks both again before it stores a read (B.3.4 step 2.1) | D.2, D.4, D.5, 0.5.4 |
| W5-3 | `reconcile::decide` becomes a test oracle; the reconcile order answers `ReadBack` with `matches_approved` from W05 6.12's `matches` | D.7.3 |
| W5-4 | `ConnectOutcome` is D.2's struct; the adapter never sets `already_an_account` | D.2, D.8.2 |
| W5-5 | FAKE as a dev-dependency of `apps/fabric-broker`; the names of CO10 (`FABRIC_BROKER_GOOGLE_CLIENT_FILE`, `FABRIC_BROKER_GOOGLE_LOOPBACK_PORT`) | D.13, E.2.2 |
| W5-6 | Agree that W05 ST7 is not built and no read follows an applied change (OP5, ruled by the lead 2026-11-17). Agreed by the CD-W05 session 2026-11-18, with a delete confirmed by one GET inside its own order (W5-11) | D.13, A.9 |
| W5-7 | The final members of `ProviderAccountRow` and `ProviderCalendarRow`, and whether `missing_scope` is derived or stored, before migration 3 merges | A.5 |
| W5-8 | Review P2-6 to P2-13 | E.2.2 |
| W5-9 | Process tests against FAKE run in process, feeding `Input::ProofDone`, under ED2's literal form; the G2b live run uses a debug build with a Keychain store in its own `FABRIC_DESKTOP_DATA_DIR` (ER5); ruled by the lead 2026-11-17 (I.6 E-1) | E.1.1, E.4 |
| W5-10 | W05 CO6's last sentence (a changed start stays before the end, checked against the before-image) is not built in the core: run before classification, its answer would tell an agent with no `time` field where an event starts and ends. The adapter's mapping of 400 to `Rejected invalid_arguments` stays | D.6.3, D.13 |
| W5-11 | A delete's read-back (W05 18.7 point 1): only after a 200 or 204 to its DELETE, one GET of the same event in the same order, with the access token the DELETE used (no refresh, nothing read from the token store), started only before `until` and bounded by it, with no retry. `confirmed` is true only for 404, 410 or `status: cancelled` at any revision; false for a live event, a failed read, an unreadable body, any other status or no time left. The GET is mapped by its own function, never turns the outcome into `Transient`, `Rejected` or `NoAnswer`, and returns no event, revision, content or evidence. Live check LV15. Agreed 2026-11-18; built in GC phase 2a (E.2.1) | D.2, D.4, DD8 |
| W5-12 | A yes with other content (W05 18.7 point 2): `confirmed` is equality with the approval after only the normalisations of a closed, named list in the adapter, empty until the live run; an entry is added only with a live result and a test against FAKE (LV18). The comparison reports which member differed as a fixed label with no content. FAKE gains two faults that answer a fitting 200, one with a trimmed description and one with a zone alias. Agreed 2026-11-18; built in GC phase 2a | D.2, V23, B.14 |
| W5-13 | The evidence bound (W05 18.7 point 3): keep the 4 MiB bound on what is read from Google; return the raw event as received, uncut, from an observe and from a reconcile read alike; W05 ST3 becomes 1 MiB with the bound on the before-view (`MAX_EVIDENCE_BYTES`, `MAX_BEFORE_VIEW_BYTES`); its risk RK21 states the limit (OD26). Live check LV16. Agreed 2026-11-18 | D.6.2, 0.6 |
| W5-14 | Spans (W05 18.7 point 4): ask Google for `start(date,dateTime)` and `end(date,dateTime)` of every event whatever the grant and use them only for the span; build the span from the occurrence's own start and end, never from `originalStartTime`, `Timed` in milliseconds or `AllDay` with Google's two dates exactly, with no widening and no zone arithmetic; a test that a read without `time` puts no start or end into the view and that there is one span per event. Live check LV17. Agreed 2026-11-18; built in GC phase 2a with the adapter's own span type, mapped to the port's `ReadSpan` after step P4 | D.2, B.3.4, DD17 |
| W5-15 | The account link (W05 18.7 point 5): `Flow::supersede_latest()`, `Flow::supersede(generation)` and `PendingFlow::generation()`, each taking the lock of the token write, with a test that once a supersede returns the flow never writes the token store, keyed by the order's account and the core's generation (build note 5a); `AccountLink::unlink(account_id, revoke)` as one function (drop the cached access token, the one revocation attempt when asked, then forget; build note 5b), with a test that the forget never runs before the revocation has read the token; deleting a missing token-store item is `Ok`, the Keychain store's "item not found" included (I.9). Agreed 2026-11-18; the supersede methods in GC phase 2a, `unlink` and the Keychain store after step P4 | D.2, D.8, DD18 |

**Live checks the CD-W05 session took on (2026-11-18)**, for the G2b live run beside its LV1 to LV14 (W05 15.3):
LV15, what a GET answers right after a 204, and whether a deleted event can be restored under its id from Google's
trash (W5-11, D.7.3); LV16, the bytes of an event with 1 000 and with 2 000 attendees, and whether
`prettyPrint=false` makes the answer smaller (W5-13, OD26); LV17, an all-day event next to the window in a calendar
at `Pacific/Kiritimati` and in one at `Pacific/Pago_Pago` (W5-14, I.5 D-18); LV18, which members Google rewrites on
an insert or a PATCH, each found one becoming a named entry of the comparison's list with a test against FAKE
(W5-12, OD25). Its risk RK21 is the limit of OD26.

### H.3 To the CD-W06 session

| # | Request | Section | CD-W06 session, 2026-11-18 |
|---|---|---|---|
| W6-1 | The four handshake requests of C.15.3: a close with no answer during the handshake is `broker_not_running` for that call; the wording of `pending` and of a key change too soon; `config_text` public with C.10's signature; connection ids and frames as C.6. For rows e and i of C.6.4 the bridge's `not_permitted` sentence says to wait a minute and call again, beside saying that the owner can look; with P2-17 (`busy` with its wait) no change | C.15.3, C.6.4 | Accepted, with the wording for rows e and i (ruled 2026-11-18) |
| W6-2 | When `practice` (P2-19) is true, the bridge's fixed first text block of every tool result begins "Practice run: changes go to a practice calendar in memory; nothing real changes." | E.1.6 | Accepted |
| W6-3 | W06 3.7 gains a row: `completed`, `isError` false. The answer to the list call or to the `fabric_request_result` call that completed carries the result ("The result follows."); a later `fabric_request_status` call shows `completed` with no result, because a status never carries one: "The request is complete." | B.3.1 | Accepted, with the corrected wording (ruled 2026-11-18) |
| W6-4 | On a status the bridge does not know which tool made the request, so `applied` and `reconciled` get one sentence that fits a change and a read: "Fabric Calendar has carried out the request; call `fabric_request_result` with this request id for its result." | B.3.3, E.2.1 | Accepted, with this sentence (ruled 2026-11-18) |
| W6-5 | Relay `prover_unavailable` and do not retry (W06 3.7) | B.10 | Accepted |
| W6-6 | Review the bundling of the bridge in the debug app, which the lead builds in step P6 (`scripts/build-desktop.sh`, W06 7.5; E.2.1); without it `get_agent_configuration` answers null and the practice run cannot start | E.1.8, C.10 | Accepted |
| W6-7 | Adopt V12 and V14, and DRAFT's handshake types (`handshake.rs`) | E.2.2, C.6 | Accepted |
| W6-8 | Accept the debug key directory `<data_dir>/bridge-keys` (0700, made by the broker) and `--socket <data_dir>/run/agent.sock` in the debug configuration text | C.9.1, C.10, E.0 ED10 | Accepted |
| W6-9 | Review P2-15 to P2-19, the listener and the configuration text; if the weight of the bridge library matters, move `config_text` into a small crate (OPC2) | E.2.2, C.16 | Accepted |

The CD-W06 session's answers are in section 13 of its own design (pull request 38). One residual it records, its R14,
belongs here: a call the MCP client cancelled (`notifications/cancelled`) is dropped by the bridge, but the broker
still holds its answer and may release a result into it. Request P2-30 asks the contract session for a cancel
message; until it exists the case is a residual of the kind of OD17 (B.8.4, test h16). The bridge sends the cancel
once P2-30 merges (E.2.1, bridge phase 2).

---------------------------------------------------------------------------------------------------------------------

## I. Open points

Every point that a writer, the architect or the editor left open, with whose it is and the recommendation. The
owner's decisions with their defaults are E.3 (OD1 to OD26); rows here that are the owner's name the OD that asks
them. "Closed" rows were settled while the sections were made to agree; they stay for the record.

The lead ruled on the lead's points on 2026-11-17, after reading this section. Each ruled row ends its
recommendation with "Ruled 2026-11-17:" and what was chosen; unless it says otherwise, the ruling accepts the
recommendation. Rows the lead did not rule on are unchanged. The owner's points stay open except OD1, OD2 and OD24, answered
2026-11-18, and are collected in E.3 (OD1 to OD26); where a row is the lead's and the owner's, only the lead's part is ruled.

### I.1 Points of the shared vocabulary (OP1 to OP15)

| # | Point | Whose | Recommendation |
|---|---|---|---|
| OP1 | V1 moves W05 ST3 (observations) and what is left of ST5 into migration 2, against P3's list. P11 (practice answers observe and reconcile; the hook at BR `shell.rs:378-392` goes before step P6) and P10 (observations stored before any proof reads them) cannot be met otherwise | lead | Accept V1 (also AO2). **Ruled 2026-11-17:** accepted; observations and the event columns of `effects` are in migration 2, migration 3 holds `provider_accounts` and `provider_calendars`; this changes P3's list |
| OP2 | V4 purges a held result at its first release, whatever grant released it; this narrows PLAN 9.2's later release under a broader grant | owner (OD3) | Keep V4; shown with the two-call read (OD1, OD3) |
| OP3 | After `Stopped`, or a failed authority check when a read's result is stored, the read goes `retry_wait` (REV-core); REV-results recommended `terminal_failure` | lead | `retry_wait` (V5); the release still needs its own proof. **Ruled 2026-11-17:** `retry_wait` |
| OP4 | When a read's result is stored, V6 checks the whole fence; the narrower per-order test was not chosen | lead | Whole fence in version 1; measure on the practice calendar (also BOP3). **Ruled 2026-11-17:** the whole fence in version 1, measured on the practice calendar in step P5 |
| OP5 | V7 drops the read after an applied change: W05 ST7 is not built and the theory is unchanged | lead with the CD-W05 session | Drop it (W5-6). **Ruled 2026-11-17:** dropped; W05 ST7 is not built; to be agreed with the CD-W05 session (W5-6). Agreed by it on 2026-11-18, with a delete confirmed by one GET in its own order (I.5 D-13) |
| OP6 | The list order (PO5) waits for TH1 (P2-6); until then the practice account gets its calendars from the seed only | contract session | Build PO5 in the port, issue it only after TH1 |
| OP7 | A connected account sent through `connect_google` again is not connected for up to 300 s | owner (screens wording) | Wording on the page, no contract change (OD13, SR-14) |
| OP8 | Until P2-4 merges, a held result that expires unreleased ends its one-request grant `consumed` | contract session | `consumed`, then `result_expired` |
| OP9 | `MAX_AGENT_CONNECTIONS = 32` is in no ruling | lead | Keep 32. **Ruled 2026-11-17:** 32 connections |
| OP10 | A removed agent's `needs_owner_resolution` requests stay open and count against the total limit | lead | Keep them open; the owner resolves them; shown with the removed agent (SR-6). **Ruled 2026-11-17:** they stay open |
| OP11 | Failed reconcile reads use `RetryPolicy::PLAN` as their budget before `needs_owner_resolution` | lead | `RetryPolicy::PLAN` in version 1. **Ruled 2026-11-17:** `RetryPolicy::PLAN` |
| OP12 | After an unknown create, a duplicate remains possible | owner | Accept with DD12's mark (OD6, OQ-D1) |
| OP13 | `WRITER_QUEUE_BYTES = 4 MiB` is the architect's choice; a release whose answer a full queue drops is recorded but never received | lead (the number); owner (the lost answer, OD17) | Accept; "a release row means the result left the core" (B.2). **Ruled 2026-11-17:** the 4 MiB bound is accepted; the lost answer stays open as OD17 |
| OP14 | V18 deletes held payloads in every backup copy of version 2 or later; a backup restored by hand reads them as not held | owner (OD5) | Accept with the ADR-115 amendment (OD5; also AO5) |
| OP15 | `ConnectOutcome` had to be restated from W05 3.2.2 with `account_id` and `generation` | section D | Closed: D.2 |

### I.2 Points of section A

| # | Point | Whose | Recommendation |
|---|---|---|---|
| AO1 | Approval of the proposed text of A.3: the rule set, the D1 and D7 amendment for content outside the ledger, the dated D2 type list, W04 D3, 3.5, 3.6 and R22 | owner | Approve as written (OD5, OD20) |
| AO2 | Whether observations and the event columns of `effects` stay in migration 2 (V1) | lead | As OP1. **Ruled 2026-11-17:** accepted, as OP1 |
| AO3 | The duration rule's "approval null while a request waits again" must be written through a new optional member or variant; if it changes what an existing variant writes for a row main could commit, it breaks A.2 R2 (DRAFT `63e8018` now states what a null approval means) | lead (step P0) | Write it as a new optional member. **Ruled 2026-11-17:** otherwise: the duration rule returns a null approval on the read side only, in `get_request_details`; it changes nothing that is written, so A.2 R2 is not touched (A.2). Closed |
| AO4 | A purged payload stays in older WAL frames until the TRUNCATE checkpoint succeeds; with the test key off macOS those frames are plain | owner (OD22) | Accept as a residual, stated with the 24 h life (OD3) |
| AO5 | A backup restored by hand reads unpurged held results as not held (OP14) | owner (OD5) | Accept |
| AO6 | The actor of `agent.prove_key` | section C, lead | Closed by X2: `agent` with the enrollment id |
| AO7 | `owner.refresh_calendars` exists only if P2-11 merges | contract session | Answer P2-11 |
| AO8 | Whether `deleted_profiles` (A.6) goes into migration 2, so that `delete_custom` can later be built without a migration of its own; until then `delete_custom` and `list_ended_requests` answer `unknown_method` | lead, with the contract and screens sessions | Not in phase 2; decide before step P3 merges. **Ruled 2026-11-17:** not in phase 2 |
| AO9 | V22 is a table constraint (`enrollment_grants_agent`, one row per agent ever); giving the list permission again at a new approval would be a changed constraint | lead | Keep V22; a later form needs a new table with a partial index or an apply guard. **Ruled 2026-11-17:** keep the constraint |
| A-8 | The purpose name of a `Disclose` proof, which the `ResultReleased` guard assumes is `fresh_permit` | section B | Closed by X15 |
| A-9 | An enrollment grant counts into the 720 KiB owner budget (CORE `owner/views.rs:76`), so `approve_agent` may be refused `invalid_params`; core code that reads `grant.origin.request_id` must handle `GrantOrigin::Enrollment` (CORE `service/pipeline.rs:276`, `service/requests.rs:49`, `service/facts.rs:38, 70`) | section C | Closed: C.3.2 check 5 and X16 (C.4.1) |
| A-10 | P2-14 and an R7 record for each non-widening change | contract session | P2-14 met; P2-25 and P2-26 |
| A-11 | Migration 3: `ConnectOutcome`'s variants, the final members of the two row types, whether `missing_scope` is derived or stored, and the owner's cap on accounts (ST10) | section D, CD-W05 session, owner | `ConnectOutcome` closed by D.2; the members W5-7; the cap OD4 |
| A-12 | The builder must search for tests that pin a run id or a ledger seq after creation: a new store has two creation rows, so the first run id is 3. No pin was found in the core and the binary | lead (step P3) | Search again when the change compiles (A.8.4). **Ruled 2026-11-17:** the builder searches again in step P3 |
| A-13 | P3's list puts observations (ST3) and part of ST5 in migration 3; the section keeps them in migration 2 (V1). Migration 3 then holds only `provider_accounts` and `provider_calendars` | lead | As OP1. **Ruled 2026-11-17:** accepted, as OP1 |
| A-14 | P2's wording "an existing variant's apply may read or write a table of migration N only on a gated branch" cannot be the only mechanism for new terminal and state values. The contract's `is_terminal` change for `needs_owner_resolution` narrows replay code; it holds only because main never drives a request into that state (CORE `service/reducer.rs:196-217`). A.2 records it under R7 as a non-widening change with its reason | lead | Accept A.2 R7's form; P2-25 makes it a standing rule. **Ruled 2026-11-17:** accepted as recorded |

### I.3 Points of section B

| # | Point | Whose | Recommendation |
|---|---|---|---|
| BOP1 | A list approved through a card ends `cancelled` (BD5); under a one-request approval the next list asks again, so under `ask` no list completes. This follows from P1 (a list may get a card) with P6 (no release without a waiting call); no safety rule is broken, but the card invites an approval that has no effect | owner (OD24); lead for the build | Accept for version 1, with the card's text (SR-7) and P2-29; the alternative is to keep an unused one-request list grant live after `CallGone` and offer it to the next list (same digest), built in step P5b. **Ruled 2026-11-17:** the lead accepts it for version 1, with the card's text (SR-7) and the request to the contract session (P2-29); the owner accepted it on 2026-11-18 (OD24) |
| BOP2 | BD4 also cancels a release that a slow prover queue delayed past 20 s | lead | Accept: the agent calls again; the held result and its grant are untouched. **Ruled 2026-11-17:** accepted |
| BOP3 | The global fence stops every read in flight on any grant end anywhere (as OP4) | lead | Keep in version 1; measure on the practice calendar. **Ruled 2026-11-17:** the whole fence in version 1, measured on the practice calendar in step P5 (as OP4) |
| B-4 | Owner (B.18, carried into P13): two calls for a read (default A); no card for a release; the 24 h life and the purges, with the stricter alternative of purging when every relied grant ended; the outcome of a change released with no grant after every grant is revoked (pinned by Q20); a lost answer after a release cannot be re-sent (OP13) | owner | The defaults of B.18 (OD1, OD2, OD3, OD11, OD15, OD16, OD17); OD1 and OD2 answered 2026-11-18 with the defaults |
| B-5 | Section C owns placing `prover_ready()` in `propose`, the `forget_connection` call sites at removal, and the writer queue numbers; section D owns the reconcile and stale reducer rows and what a refused system proof or an overdue observe, reconcile or list order does to its serving request. B states the rule | sections C and D | Closed: C.8, C.3.3, C.9.6; D.5, D.6.3, D.7.4 confirm |
| B-6 | P6 against DRAFT README line 190 ("an Allow returns the result"), and P7 against the same line's list of states that answer an action (no `outcome_unknown`) | contract session | P2-2 as written in section G |
| B-7 | P8 against W04 D14: once a call is refused while the prover is not Ready, a run with no prover refuses every action instead of ending it undecided; W04 c18 and the core tests with no prover change (B.10) | owner (amends D14, OD18) | Accept; buildable as written (B.10); new wording in E.1.9 |
| B-8 | P8 says "while the prover's status is not Ready". The synthetic classifier is never Ready (BR `synthetic.rs:93-99`; CORE `service/core.rs:111-114`), so built literally every action of a synthetic run is refused `prover_unavailable` and no card opens (E.1.2 rows 2 and 5) | lead | Default: it is exempt, debug only: `prover_ready()` counts the kind `Synthetic` as ready (B.10, C.8, E.1.9; test p02). The other form refuses every action of a synthetic run and amends W04 D26 and 5.10, the harness and the process tests. **Ruled 2026-11-17:** accepted: the synthetic classifier of a debug build counts as ready for P8's rule; the exemption is by the prover's kind and exists only in debug builds |

### I.4 Points of section C

| # | Point | Whose | Recommendation |
|---|---|---|---|
| OPC1 | `nix` is a new dependency of the broker binary (unix only, features `user` and `socket`, with `cfg_aliases`); P9 needs it; it is not in `Cargo.lock` | owner (dependency review, OD21) | Accept |
| OPC2 | The broker binary depends on the `fabric-mcp-bridge` library for `config_text` | lead with the CD-W06 session; owner for the dependency (OD21) | Accept; a small crate if its weight matters (W6-9). **Ruled 2026-11-17:** accepted; the smaller crate only if the weight matters; the dependency stays open as OD21 |
| OPC3 | The broker's environment has no `HOME` (relay allowlist, `apps/fabric-desktop/relay/src/config.rs:40-79`), so a bridge under `$HOME/Applications` gets a null configuration | owner (OD22) | Keep; the alternative is `HOME` in the allowlist |
| OPC4 | Repeated `wrong_key` and `bad_signature` against an approved agent show only in the audit | owner (OD22) with the screens session | Accept for version 1 |
| OPC5 | A user short name over 18 bytes makes the installed socket path too long; only a stderr line (and P2-18) shows it | lead; told to the owner (OD22) | Accept. **Ruled 2026-11-17:** accepted. **Ruled 2026-11-18:** still accepted for version 1 and told to the owner in OD22. The bridge is not bound by it (it connects by the relative name from inside `run`); the broker binding by the relative name from inside its `run` directory, a change to how the binary starts its listener, is decided at CD-W11 (the CD-W06 design's R7) |
| OPC6 | No limit on accepts per second | lead | Accept; 8 at once and 5 s bound it (bt07, bt08). **Ruled 2026-11-17:** accepted |
| OPC7 | Owner and agent lines share the queue of 4 | lead | Accept with bt12 as the bound. **Ruled 2026-11-17:** accepted |
| OPC8 | The pairing code is 48 bits; a colliding key proved before the owner looks would be approved (DRAFT choice 62) | owner (OD22; contract) | Accept the residual |
| OPC9 | Same-user software that runs the bundled bridge with the agent's id has its key (W06 T3) | owner (OD22) | Residual of PLAN 4.3 |
| OPC10 | With `MAX_KEY_CHANGES = 20`, a process that knows a pending id can force the owner to remove and add the agent again | owner (OD22) | Accept: the trade is a bounded ledger |
| C-11 | P2-21, P2-22, P2-23 and P2-28 are new requests and need answers | contract session | Section G |
| C-12 | P5 "approve_agent returns the grant": DRAFT's `ApproveAgentResult` has no grant (DRAFT CC `owner.rs:1924-1930`) | contract session | P2-21; until then the grant notice of the same commit |
| C-13 | P4 and the first draft of A.4.8 say one `Change::Agent` notice at the end of the removal; step 1 already sends one (CORE `service/sessions.rs:102-104`), and P4's order has no raise of `revocation_epoch` when nothing ends | lead | Closed by X1 |

### I.5 Points of section D

| # | Point | Whose | Recommendation |
|---|---|---|---|
| CQ-D1, CQ-D2, CQ-D3, CQ-D4 | The theory questions of section G, and the bound on `selected` (CQ-D4) | contract session | The defaults of section G |
| OQ-D1 | The residual risk of a duplicate after a create whose outcome was unknown | owner | Accept; DD12 marks a late event under the old reserved id as Fabric's (OD6) |
| OQ-D2 | Whether the broker's own reads are proven: about 2 s per check, and no account can be set up or reconciled while the prover is not ready | owner | Yes (OD7) |
| OQ-D3 | The cap on accounts: at a 96 KiB reserve, 7 accounts fit with no other records, none beside a recipient set at the schema maximum, and no command frees an account's reserve | owner | The budget alone (OD4) |
| D-4 | Whether `disconnect_google` warns while a change of that account is still unknown (W05 OQ16) | owner (OD22) | No warning |
| D-5 | Reconciliation in the core (D.7) needs migration 2 and the port; it must merge before step P6 and before Google is attached | lead and section E | Closed by X18: step P5 (P5a) |
| D-6 | The W04 D27 amendment for `FABRIC_BROKER_GOOGLE_CLIENT_FILE` and the debug-only `FABRIC_BROKER_GOOGLE_LOOPBACK_PORT` (CO10) | owner | Approve (OD12) |
| D-7 | P10 "every narrowing command raises the revocation epoch": `pause_agent` raises `pause_epoch` (W04 5.6; CORE `owner/commands.rs:200`), so D.9.4's test asserts that an epoch of the fence changed | lead | Accept D.9.4's form. **Ruled 2026-11-17:** accepted in D.9.4's form |
| D-8 | P10 "a 5xx on a change is an unknown outcome" needs an exit from `outcome_unknown`; today there is none (CORE `service/reducer.rs:84-112`, `service/pipeline.rs:567-577`, `service/recovery.rs:86`) | lead | D.7 builds it; DD13 makes it a precondition for Google. **Ruled 2026-11-17:** accepted in D.7's form |
| D-9 | The CO6 table cannot send `validating` straight to `undecided` (not an edge, CC `lifecycle.rs:127-131, 164`); the nearest legal form is `validating -> proving_classification -> undecided` in one commit | lead | Accept D.6.3's form. **Ruled 2026-11-17:** accepted in D.6.3's form |
| D-10 | P10's two-read rule has one exception: a delete whose read-back finds the event at another revision is `absence_confirmed` at the first read, because its `If-Match` can never match again (W05 PD14, W05 section 7 table; DD7, D.7.3) | lead | Accept the exception; the other form makes that row wait for a second read 60 s later, which changes nothing it decides. **Ruled 2026-11-17:** accepted in D.7.3's form (DD7) |
| D-11 | The pre-check before an observe (D.6.1) covers the pause and the calendar's write access only. A row the profile blocks is not pre-checked, so an unpaused agent learns the event's state there (`revision_conflict`, `invalid_arguments`, `stale`) as the theory's own rules would tell it, and its call costs one system proof and one Calendar read. Linked to CQ-D2 | contract session (theory owner) | Keep for version 1; if the theory answers CQ-D2 so that the observe names the event, add `mode_not_blocked` to the pre-check. Closed by D-12 (ruled 2026-11-17): the pre-check refuses a row the profile blocks (D.6.1) |
| D-12 | D-11's gap is in the core's own pre-check and does not wait on CQ-D2. Under a profile that blocks the action's row the theory answers `mode_not_blocked` before `event_known` (THY `block_rules`), whatever the event's state; D.6.1 still observes, so the agent learns `revision_conflict` or `invalid_arguments` and drives a Calendar read on a row the owner blocked. For a response, an invitation and a delete the row is known without the event (THY `rows`); an update is blocked whatever the event when both edit rows block | lead (D-11 was left with the contract session on 2026-11-17) | Add to D.6.1's first check: the profile does not block the action's row (`respond_to_invitation` for a response, `send_or_change_invitations` for an invitation, `edit_fabric_created_event` and `edit_preexisting_event` both for an update; a delete always asks, THY `delete_row_always_asks`); when it does, no system proof and no observe start and the theory answers `mode_not_blocked`. D-11 then closes. **Ruled 2026-11-17:** accepted; applied in D.6.1 and D.6.3; D-11 is closed |
| D-13 | A delete's read-back (the CD-W05 session's point 1, I.9): Google answers a DELETE with no body, so every successful delete would sit in `outcome_unknown` for at least 10 s and be shown so | lead with the CD-W05 session | One GET of the event inside the delete's own order, under its agent permit, with the conditions of D.2; confirmed goes `applied -> reconciled`, unconfirmed is handled as no answer and reconciled by reading, with no audit row (V9). **Ruled 2026-11-18 by the lead:** accepted with those conditions; W5-6 is agreed; W5-11, LV15 |
| D-14 | A yes with other content (point 2): a fitting 200 whose content differs from the approval was an unknown outcome, so a PATCH that Google normalised without moving the revision could end `absent` after two reads, and a new request could repeat a change that notifies guests | lead with the CD-W05 session; owner OD25 | A fitting `Applied` of a create, update, invitation or response is applied, never unknown; when it differs, the event's id and revision are recorded and the request waits for the owner, who can only resolve it as applied (V23); `confirmed` uses a closed, named list of normalisations, empty today. **Ruled 2026-11-18 by the lead:** accepted; P2-31 to P2-34 are a precondition of any Google dispatch; no interim refusal of `absent` in the core (D.7.5); W5-12, LV18 |
| D-15 | The evidence bound (point 3): 262 144 bytes refused an event with about a thousand attendees | lead; owner OD26 | `MAX_EVIDENCE_BYTES` 1 048 576 and `MAX_BEFORE_VIEW_BYTES` 524 288 (lowered if the frame test of D.6.2 fails); past either no observation is stored and the request ends with a fixed sentence. **Ruled 2026-11-18 by the lead:** accepted, a stated limit of version 1 for the owner (OD26); W5-13, LV16 |
| D-16 | Spans (point 4): a span for every event needs Google's start and end whatever the grant, and an all-day event has dates, not instants; the session proposed that the adapter widen | lead with the CD-W05 session | Typed spans from the adapter; the core widens an all-day span by `ZONE_SLACK_MS` for its check only (DD17, B.3.4 step 2.1). **Ruled 2026-11-18 by the lead:** accepted with one change, the core widens, not the adapter; W5-14, LV17; exact bounds are D-18 |
| D-17 | The account link (point 5): `connect` blocks its worker for up to 300 s with no supersede that takes the lock of the token write, and a disconnect's revoke and forget wait behind it; the session offered to forget at once on another thread, but its revocation reads the token from the store | lead with the CD-W05 session | Supersede under the lock of the token write; revoke and forget as one `unlink` job on a worker of its own; a disconnect raises `connect_generation`; the unlinks repeat at start (DD18, D.8). **Ruled 2026-11-18 by the lead:** accepted in this form, not as the session proposed, with the two build notes of I.9; W5-15 |
| D-18 | Exact all-day bounds: the core widens an all-day span by 14 h each way, so an all-day event of the day just before or after the window passes its check when the adapter's own window was wrong (Google's filter is the first barrier) | CD-W09 | Make the bound exact in the calendar's zone once the workspace has a zone database and LV17 has shown in which zone Google filters all-day events. Open, not a blocker; recorded by the lead's ruling of 2026-11-18 (D-16) |

### I.6 Points of section E

| # | Point | Whose | Recommendation |
|---|---|---|---|
| E-1 | ED2 literal (the test key with Isabelle and Google is refused) means the CD-W05 process tests against FAKE through the real binary never reach a permit; the alternative treats a debug loopback FAKE endpoint as a Google run that is not practice | lead with section D | Keep ED2 literal; tests in process (W5-9). **Ruled 2026-11-17:** ED2 literal; CD-W05's tests against FAKE run in process |
| E-2 | The names of the CO10 variables, and row 8 of E.1.2 | section D | Closed by X17 |
| E-3 | Whether "no such event" on a conditional change maps to `revision_conflict` | section D | Closed by X7 |
| E-4 | Confirm ED10 in the configuration text, and `FIRST_LISTENER_CONNECTION = 1 << 32` for test e10 | section C | Closed: C.9.1 step 5, C.10, CD24 |
| E-5 | `apps/fabric-desktop/README.md` and `harness/serve.ts` carry the E.1.9 wording | screens session, unless the owner widens the lead's list | SR-12 |
| E-6 | Bundle the bridge in the debug app by step P6 | lead (step P6); the CD-W06 session reviews | W6-6 |
| E-7 | OD1 to OD26 are the owner's; OD1, OD2 and OD24, needed before step P2, were answered on 2026-11-18 with the defaults; the first still needed are OD5 (with the A.3 part of OD20) and OD7 before step P3. The two he must answer first: OD5, OD7 | owner | E.3 |
| E-8 | Not verified: that Isabelle proves `System_refresh` for the seeded practice account, and the latency of the extra system proofs inside the 20 s hold (ER9) | lead | Check in step P5 on the practice calendar. **Ruled 2026-11-17:** checked in step P5 |
| E-9 | P12's scope for step P5 ("results, held answers and system proofs") is not enough: P11 and P10 require observe before a proof, the fence fix and the watchdogs before step P6. E places them, with reconciliation, in step P5 as P5a then P5b | lead | Accept E.2.1. **Ruled 2026-11-17:** accepted: step P5 is P5a then P5b |
| E-10 | P12's "P4, in parallel with P3" cannot be kept. P4 must start after P2 merges, because PR 36 also writes CORE `broker.rs` and `service/pipeline.rs`; and after P3 merges, because D.2's port types name `Notification`, `ConnectProblem`, `AuthLostCause` and `SystemAction`, which are store row types shipped in step P3 (X21) | lead | Accept E.2.3: P4 after P3. **Ruled 2026-11-17:** accepted: step P4 after migration 2 (step P3); this changes P12's "in parallel" |
| E-11 | P3 puts observations in migration 3; E follows V1 | lead | As OP1. **Ruled 2026-11-17:** accepted, as OP1 |

### I.7 Points the editor adds

| # | Point | Whose | Recommendation |
|---|---|---|---|
| X-1 | Practice dispatch row 4 removed (X6) against ED6's first form | lead | Keep it removed; reverse only if a resend by the core is ever intended. **Ruled 2026-11-17:** row 4 stays removed |
| X-2 | A stored version 2 `pause_agent` answer halts the broker on a repeated command after the upgrade (P2-26) | contract session with the lead | Make the version 3 agent members optional and defaulted when read, before step P3. **Ruled 2026-11-17:** goes to the contract session as a request before step P3 (P2-26) |
| X-3 | DRAFT moved to `b94b58f` while this design was written; the rest of it was read at `29cdfdf` | lead | Re-read DRAFT's diff before step P2 starts. **Ruled 2026-11-17:** the lead re-reads the diff before step P2 |
| X-4 | The worker's bound is a monotonic stamp the shell takes (X23, B.13). The smaller change, the core writing `deadline_ms - clock.offset_ms()` into the order, keeps one number but moves with a wall clock that goes back after the order was sent | lead | Keep the monotonic stamp. **Ruled 2026-11-17:** the monotonic stamp |
| X-5 | A removed agent's request that would retry ends `cancelled` (X22); the other form, `terminal_failure` with `retry_budget_exhausted`, was proposed by a checker | lead | Keep `cancelled`. **Ruled 2026-11-17:** `cancelled` |

### I.8 Answers of the CD-W06 session (2026-11-18)

The CD-W06 session read its rows of this design and accepted W6-1 to W6-9 (H.3); its answers are in section 13 of its
own design (pull request 38). It sent three corrections, two points for the lead to decide and one question. The lead
ruled on them on 2026-11-18:

| # | Point | Ruling | Where |
|---|---|---|---|
| 1 | `completed` and a result (W6-3): the design said `completed` is never reached without a result | The answer to the list call or to the `fabric_request_result` call that completed carries the result; a later `fabric_request_status` shows `completed` with no result, and the bridge says "The request is complete." | B.3.1, H.3 |
| 2 | `applied` and `reconciled` in the bridge (W6-4): on a status the bridge does not know which tool made the request | One sentence for both states: "Fabric Calendar has carried out the request; call `fabric_request_result` with this request id for its result." | H.3, E.2.1 |
| 3 | `not_permitted` at the handshake (W6-1) can mean "wait" | For rows e and i of C.6.4 the bridge says to wait a minute and call again, beside that the owner can look; with P2-17 no change | C.6.4, C.15.3, H.3 |
| 4 | A call the MCP client cancelled (`notifications/cancelled`; the CD-W06 design's R14) | Ask the contract session for a cancel message on the agent channel in schema version 3 (P2-30); until it exists, and for a cancel after the release commit, a residual of the kind of OD17; test h16 | B.8.4, B.16.2, E.3 OD17, G, H.3 |
| 5 | Merge order of PR 38 (bridge phase 1) and PR 36 (schema version 3): the bridge's state table has no default arm | Whichever merges second carries the lines for the new states in `apps/fabric-mcp-bridge/src/mcp/render.rs` and its test r01 | E.2.2, E.2.4 |
| 6 | A user short name over 18 bytes (OPC5; the CD-W06 design's R7) | Accepted for version 1 and told to the owner in OD22; the broker binding by the relative name is decided at CD-W11 | C.16, E.3 OD22, I.4 |

### I.9 Answers of the CD-W05 session (2026-11-18)

The CD-W05 session accepted every row of F for its ids and W5-1 to W5-10, and raised five points before building
its phase 2. The lead ruled on them on 2026-11-18 (I.5 D-13 to D-17) and the session accepted all five as written.
They are section 18.7 of its design (`docs/plans/calendar-desktop-w05-design.md` at `8ff66ec` on branch
`claude/cd-w05-google-provider-27ca35`; not on main yet, a pull request of its own will carry it). It took on the
live checks LV15 to LV18 (H.2) and records the limit of point 3 as its risk RK21.

| # | Point | Ruling | Where |
|---|---|---|---|
| 1 | A delete's read-back | Accepted with conditions: one GET of the same event in the delete's own order, only after a 200 or 204, with the DELETE's access token, inside the order's bound, no retry; confirmed only when the event is gone; the core unchanged; W5-6 agreed | V7, V9, DD8, D.2, D.4, D.7.1, D.7.3; W5-6, W5-11 |
| 2 | A yes with other content | A fitting 200 of a create, update, invitation or response is applied, never unknown; when it differs the request waits for the owner, who can only resolve it as applied; four contract requests before any Google dispatch | V9, V23, DD6, B.14, D.7.1, D.7.5; G P2-31 to P2-34; E.3 OD25; W5-12 |
| 3 | The evidence bound | 1 048 576 bytes for the whole evidence, 524 288 for the before-view (lower if the frame test fails); past either, no observation and a fixed sentence; a stated limit of version 1 | 0.6, A.7, A.9 ST3, D.6.2; E.3 OD26; W5-13 |
| 4 | Spans | Start and end asked always; typed spans with no widening in the adapter; the core widens an all-day span by 14 h for its check only; exact bounds wait for CD-W09 | DD17, 0.6, B.3.4, B.16.2, D.2; I.5 D-18; W5-14 |
| 5 | The account link | (a) supersede under the lock of the token write, through the shell's clone of the flow handle; (b) revoke and forget as one `unlink` on a worker of its own, repeated at start | DD18, 0.5.4, 0.6, D.1, D.2, D.8; W5-15 |
| 6 | Deleting a missing token-store item (asked by the lead) | Confirmed: `Ok` by the contract of the session's `TokenStore` trait; the memory store does so and a test asserts it; the Keychain store, written in phase 2, answers `Ok` on "item not found" and its ignored test covers that case. So the unlinks at start can repeat | D.8.4; W5-15 |
| 7 | Build note (5a) | A supersede is keyed by the order's account and the core's generation, not by the flow's own count, so "superseded while queued" is decided on the core's numbers. Agreed by the lead | D.2, D.8.1 |
| 8 | Build note (5b) | `unlink` takes whether to revoke, since a disconnect without `revoke_provider_token` only forgets. Agreed by the lead | D.2, D.8.3 |
| 9 | The order | The session builds now, from main, as one pull request of its own and ahead of step P4, the parts that need no type of the port: the delete's read-back, the member label, the empty list of normalisations and FAKE's two faults, asking for start and end always and the typed span, the 20-page cap on a calendar list, and the supersede methods. It touches only its two crates and its design. The Keychain store, `unlink`, the port mapping and the browser stay for after step P4. Agreed by the lead | E.2.1 (GC phase 2a), E.2.2, E.2.3 |
