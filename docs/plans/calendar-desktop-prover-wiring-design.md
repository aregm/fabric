# Prover wiring design: live prover, prover configuration, practice calendar, practice run

**What this is.** The design of the step of the Calendar Desktop plan that attaches the real rule prover (Isabelle,
through `crates/fabric-broker-policy`) to the broker, as built: the prover adapter (`apps/fabric-broker/src/prover/`),
the three Isabelle settings the broker checks before any frame (`apps/fabric-broker/src/config.rs`), the debug-only
practice calendar (`apps/fabric-broker/src/practice.rs`), the synthetic agent's practice script
(`apps/fabric-broker/src/synthetic.rs`), the routing the executable and the live tests share
(`apps/fabric-broker/src/shell.rs`), the relay's and the host's part (`apps/fabric-desktop/relay`,
`apps/fabric-desktop/src-tauri/src/host.rs`, `apps/fabric-desktop/harness/serve.ts`), the live proofs
(`apps/fabric-broker/tests/live.rs`, `sh scripts/verify-desktop.sh --live-proofs`) and one line on the page. It
describes the code as it is, not a plan for it. It builds on the CD-W04 design (W4D below), which describes the store,
the authority core and the broker executable this step extends.

**How code comments cite it.** A citation of this document in a code comment carries the prefix `PW`: `PW 2.4` is a
section, `PW X8` or `PW P1` a decision of section 0, `PW c40`, `PW sh01`, `PW p44` or `PW L1` a test id of section 7,
`PW K1` a risk, `PW T1` and `PW E1` requests, and `PW O1` an owner question of section 9. A number without the prefix
in the same crates cites W4D (`docs/plans/calendar-desktop-w04-design.md`), as before; in particular the live tests
`PW L1` to `PW L19` are not W4D's package decisions L1 to L7. A file that cites both says so in its module
documentation.

**Who decides.** Every question of section 9.3 is the owner's; the code runs on the default named there. The
Calendar Desktop plan and the contract (`contracts/desktop/v1`) win wherever they differ from this document. The
practice calendar is an exception to W4D's decision D13 ("no prover or provider double in product code"): the
amendment of D13 and D26 in W4D is proposed with this step's pull request and is the owner's to accept (O1).

Date 2026-11-16. Status: as built, describing the branch `claude/cd-prover-wiring` before merge.

Abbreviations (paths relative to the repository root):

| Short | Means |
|---|---|
| W4D | `docs/plans/calendar-desktop-w04-design.md` (the CD-W04 design; its decisions are D1 to D39 and L1 to L7) |
| PLAN | `docs/plans/calendar-desktop-plan.md` (section 5 holds the requirements R01 to R14 and the quality targets, section 14 the required scenarios) |
| RM | `docs/roadmap.md` |
| CR | `contracts/desktop/v1/README.md` (the contract) |
| GATE | `crates/fabric-broker-policy/src/gate.rs` (`BrokerGate`, `GateConfig`, `Check`) |
| MODEL | `crates/fabric-broker-policy/src/model.rs` (`Context`, `Action`, `Grant`, `Rule`) |
| PORTS | `crates/fabric-broker-core/src/ports.rs` (`ProofJob`, `ProofDone`, `Provider`, `DispatchOrder`) |
| THY | `isabelle/Fabric_Broker/Fabric_Broker.thy` (the rule theory), with `isabelle/ROOTS` and `isabelle/Fabric_Broker/ROOT` |
| S1 to S7 | the scenarios this step can show (section 7) |

Safety rule of the whole design. The adapter emits a `ProofReport` only when `BrokerGate::check` returned `Ok(Check)`
for exactly this job, the check ran on the rule theory this broker was built with, the check names this job's action
and context digests, and every conversion succeeded. Every other path is `report: None`, which the core turns into
`undecided` with nothing recorded. Only the core turns a result into authority, and it already refuses an unbound
result (W4D D39), a non-Isabelle permit (W4D D13), a fresh permit with no grant, no provider or a paused agent (W4D
D17), a changed fence, and a late result (the job id is gone after the core's 35 s watchdog). The practice calendar
acts only on a `DispatchOrder`, which exists only after a consumed authorization resting on an accepted Isabelle
permit. No test double of the gate ever returns a `Check`: doubles exist only under `#[cfg(test)]` and can only fail
or block.

---------------------------------------------------------------------------------------------------------------------

## 0. Decisions

| # | Topic | Chosen | Why |
|---|---|---|---|
| X1 | Isabelle location | Three variables `FABRIC_BROKER_ISABELLE_HOME`, `FABRIC_BROKER_ISABELLE_USER_HOME`, `FABRIC_BROKER_RULES_DIR`, all or none | One root variable would hard-wire the developer layout (`GateConfig::for_repository`) into the broker's interface, and PLAN forbids an application pointing at `.tooling`. Three paths map one to one to `GateConfig` and serve a packaged runtime (PLAN Q09) unchanged |
| X2 | Practice calendar switch | No variable; on exactly when `cfg!(debug_assertions) && synthetic_agent && isabelle.is_some()` (`Config::practice`) | The synthetic agent already carries practice rules P1 and P2 (4.1): debug builds only and the test key (W4D D26); the host passes it only with a scratch directory. A fourth variable adds refusals and no protection |
| X3 | Practice state | In memory only (`Arc<Mutex<_>>`), seeded at broker start, no file | Practice rule P6 forbids plaintext practice files. Cost: practice events do not survive a broker restart (4.2) |
| X4 | Fault injection | In-process `Faults` passed to `PracticeCalendar::new`; `Plan::of` always passes `Faults::default()` | W4D D12: no product hook reachable from outside |
| X5 | Practice identities | The synthetic agent's `synthetic-account` / `synthetic-calendar`; one seeded event Fabric did not make | One seed; the ids are already in the theory's identifier charset |
| X6 | Observation for update and delete | `PracticeCalendar::observation(&agent::Action) -> Option<ObservedEvent>`; the shell sets only `job.facts.event`, only when the core left it `None`, in practice runs only | An update or delete needs the observed event (`event_known` in MODEL); the core sets `facts.event` to `None` until CD-W05 adds the core's observation port and `Fence.event_revision` (W4D 1.2). Fail-closed: `None` unless the target is a live practice event; `context_of` refuses an observation of another calendar; the provider checks the revision again at dispatch. It departs from PORTS ("read in one transaction"): owner question O2 |
| X7 | Adapter shape | `prover/{mod.rs, map.rs}`; a crate-private `Gate` trait and starter; doubles only under `#[cfg(test)]`; one starter thread per start, one dropper thread per broken gate; no watcher thread, but an overrun poll the shell calls (2.3) | Isabelle stays out of CI (PLAN RD-7), so every failure path is unit-tested without it; the seam is not reachable from outside the crate (W4D D12, D13) |
| X8 | Restart policy | After the n-th consecutive failure (a failed start or a broken gate) wait `RESTART_WAITS[n-1]` (10 s, 60 s) and start again; after a third failure stay `unavailable` for the run; the count resets on `Ready`; a start never begins while a dropped gate is still stopping | Bounded; a stuck stop is never reused. Owner question O3 |
| X9 | Stop at quit | Bound 1.5 s (`QUIT_BOUND`); main drains its event channel while it waits; the work directory is removed in every state, by the worker or, without an acknowledgement in time, by the calling thread | The relay kills the broker 2 s after closing its stdin (`QUIT_DRAIN`) |
| X10 | Work directory cleanup | `remove_dir_all(<data_dir>/prover)` then a non-recursive create (mode 0700 on Unix) before every gate start, and removal at every stop; once the store is open and its lock held, the shell removes it once in every configuration, with or without a prover | One call covers a killed broker's `Check_*.thy` and the server logs; `remove_dir_all` does not follow a symbolic link; the non-recursive create can never make `<data_dir>` itself |
| X11 | Deadline skip | Skip when wall `now_ms() >= job.deadline_ms`, no margin | The core clock is wall time plus a never-negative offset (W4D D34), so the skip never fires before the core's deadline; a late result is ignored anyway |
| X12 | Reason texts | `no_report(job, &'static str)` with fixed texts; no `PolicyError` text reaches a reason or stderr | A no-report reason is never stored or shown. An `Undecided` check can carry an Isabelle message into `undecided_reason`; that is stored only in the encrypted store |
| X13 | Agent script | Create, blocked delete of the foreign event, update, delete; lookup by the create's request id | The block appears without two more approvals; the lookup by request id is exact |
| X14 | Agent API | `SyntheticAgent::practice(..)` and `set_prover_health(..)`; the proposal limit is a field (`PROPOSALS_PER_RUN` = 2 for `new`, `PRACTICE_PROPOSALS` = 4 for `practice`) | W04's callers and its pinned process tests are unchanged |
| X15 | Live tests | One file, `apps/fabric-broker/tests/live.rs`; the tests of 7.3 | Every live test is in one place and runs by name |
| X16 | Host forwarding | A debug host forwards the three Isabelle variables when all three are set and absolute, with any store; a release host ignores them; the packaged check unsets them in its launches | With a Keychain store no provider is attached, so a fresh permit ends `undecided`. Owner question O6 |
| X17 | Release broker | Reads the three variables in every build; only the host decides not to pass them in release | Owner question O4 |
| X18 | The broker's `PATH` | On Unix the relay gives the broker the fixed `PATH=/usr/bin:/bin:/usr/sbin:/sbin`, never the host's; the harness does the same | Isabelle's scripts call system tools by name (`sysctl` in `/usr/sbin`), and with no `PATH` their shell's default lacks `/usr/sbin`, so the session start fails (L11). The broker cannot set it itself: the policy crate's server start sets only `USER_HOME`, and `set_var` is `unsafe`, which the workspace forbids |
| P1 | Theory pin, shared shell, store first, stop | (a) Theory pin: the broker embeds THY and the two session files with `include_str!`; before a gate starts, the worker compares the SHA-256 of `<RULES_DIR>/Fabric_Broker/Fabric_Broker.thy` with the built theory's and `<RULES_DIR>/ROOTS` and `<RULES_DIR>/Fabric_Broker/ROOT` byte for byte with the built ones, and a started gate's `theory_sha256()` with the built hash; a mismatch leaves the prover `unavailable` for the run with one fixed stderr line, and nothing is proved; a `Check` whose `theory_sha256` differs yields no report (2.1 to 2.4). (b) `shell.rs` holds the routing that `main.rs` and `tests/live.rs` both use (2.2, 8). (c) The Isabelle worker starts only after the store is open and its lock is held (2.2). (d) Stopping removes the whole work directory in every state; without an acknowledgement within the bound the calling thread removes it itself (2.5) | Rust derives the outcome from the 18 rule results, which is sound only for the theory the Rust mirror was tested against; the session files decide which theory Isabelle loads. A mismatch is a prover failure, not a configuration error (exit 78) |
| P2 | Smaller safeguards | `Gate`, the starter and `ProverOptions.starter` are crate-private, doubles only under `cfg(test)`; the observation hook returns `Option<ObservedEvent>`; health leaves `ready` while a check is past its deadline; `done_of` checks the check's action and context digests against the job's; Unix-only checks behind `cfg(unix)`, plain `is_file` elsewhere; the proposal limit is a field; the packaged check unsets the three Isabelle variables; practice updates and deletes notify nobody (`send_updates: none`) | Each is placed in the section it changes |
| P3 | The page | Not built in this step. The screens of CD-W08 are the page; this step adds one static line under the build line, `#practice-line` in `ui/index.html` (6). Section 9.4 lists what the page still lacks for a practice run | Another work package owns the screens |
| P4 | Owner questions | O1 to O7 stay open (9.3); the code runs on their defaults. O1: the practice calendar is built (debug builds only, only with the synthetic agent, the test key and a configured Isabelle; state in memory), and the amendment of W4D D13 and D26 is proposed with this step's pull request. O5: the synthetic classifier stays for synthetic runs without Isabelle, so the W04 process tests and CI are unchanged | The owner decides |
| P5 | Requests to other owners | T1 to T9 (theory and policy) and E1 to E4 (contract) are requests (9.2); nothing under `crates/fabric-broker-policy`, `isabelle` or the contract changes | One owner per area |
| P6 | Live proofs | Never run by CI (PLAN RD-7). Each is `#[ignore]` with a reason and runs by name; `sh scripts/verify-desktop.sh --live-proofs` runs them on a developer's Mac. They take the Isabelle location from the same three variables, with the repository's development layout as the default when all three are absent | RM keeps Isabelle out of CI; PLAN 14 names the flag |

---------------------------------------------------------------------------------------------------------------------

## 1. Scope and what the owner sees

In: (1) `IsabelleProver`, the adapter from the core's prover port to `BrokerGate`, pinned to the rule theory the broker
was built with; (2) the three Isabelle variables checked before anything starts; prover absence ends every request
`undecided`; (3) `PracticeCalendar`, a debug-only provider on the synthetic account; (4) the synthetic agent's practice
script; (5) `shell.rs`, the routing shared by `main.rs` and the live tests; (6) the relay's three names and fixed
`PATH`, the debug host's forwarding, the harness's forwarding, and the `--live-proofs` mode of
`scripts/verify-desktop.sh`. Out: Google (CD-W05), the MCP bridge (CD-W06), the screens (CD-W08; section 9.4 lists
what a practice run still needs from them), reconciliation of `applied`, the core's observation port and
`Fence.event_revision` (CD-W05), a packaged Isabelle (PLAN Q09, CD-W11). No change to `contracts/`, `isabelle/`,
`crates/fabric-broker-policy`, `crates/fabric-desktop-contracts`, `crates/fabric-broker-core`, `crates/fabric-storage`,
`apps/fabric-demo`, `docs/PRD.md`, `docs/architecture.md`, `docs/calendar.md`.

Practice run (the debug application, from the checkout's root, Isabelle installed beforehand by hand with
`sh scripts/setup-demo.sh`; one new scratch directory per run, 4.2). `apps/fabric-desktop/README.md`, "The practice
run", has the same command and the browser variant through the harness:

```sh
sh scripts/build-desktop.sh --configuration debug
D="$(mktemp -d)"
FABRIC_DESKTOP_DATA_DIR="$D" FABRIC_BROKER_KEY_SOURCE=test FABRIC_BROKER_SYNTHETIC_AGENT=1 \
FABRIC_BROKER_ISABELLE_HOME="$PWD/.tooling/isabelle/Isabelle2025-2" \
FABRIC_BROKER_ISABELLE_USER_HOME="$PWD/.tooling/isabelle/home" \
FABRIC_BROKER_RULES_DIR="$PWD/isabelle" \
  "target/debug/bundle/macos/Fabric Calendar.app/Contents/MacOS/fabric-desktop"
```

The window shows the texts of the CD-W08 screens. "The calendar" in the request states means the practice calendar
in this run, which the practice line under the title says.

| # | Window | Owner |
|---|---|---|
| 0 | Build line "Development build. Broker: fabric-broker, build 0.1.0 CD-W04 synthetic practice."; under it the practice line. The strip: "The rule checker is starting." (the first start can build Isabelle's session heap for several minutes) | - |
| 1 | The strip's entry goes; Status shows "Running." under "Rule checker"; "Synthetic agent (test build)" is connected. The agent proposes only once the prover is ready | - |
| 2 | A `propose_create`: "Received, checking", then "Waiting for your decision"; the card names "Rule: authority", and "Show the proof" opens the Isabelle proof | Allow, only this request |
| 3 | A: "You allowed it; Fabric checks it once more before it is done", "Fabric checks it once more before it is done", "Allowed; waiting to be sent to the calendar", "Being sent to the calendar", "Done: the calendar accepted it. Fabric has not confirmed it by reading it back yet" (no confirmation comes before CD-W05) | - |
| 4 | B `propose_delete` of the seeded event Fabric did not make: "Blocked by a rule", `Rule fabric_created: Only events that Fabric created may be deleted.`, and the proof's id; "Show the proof" shows outcome "Blocked" | - |
| 5 | C `propose_update` of A's event (30 minutes later, nobody notified): card, Allow once, then done as A | Allow |
| 6 | D `propose_delete` of A's event (nobody notified): card (a delete always asks, theory `mode_of`), Allow once, then done | Allow |

Failure views: the three variables absent: a synthetic run keeps the classifier (W04: a card, then "Not done: Fabric
could not prove that it is allowed"); one or two of them, or a relative path: the host starts no broker and the strip
says "Fabric Calendar is not installed correctly."; a path that holds no Isabelle or no rule theory: the broker exits
78 at start, the strip says "The broker stopped." with the exit code, and the broker's line naming the variable and
the path goes to the terminal's stderr; a rule theory or session file that is not the built one: the rule checker is
not running for the run, with one stderr line; Isabelle killed or a check past its deadline: the request ends "Not
done: Fabric could not prove that it is allowed", health goes `unavailable`, `starting`, `ready`, and nothing is
dispatched.

---------------------------------------------------------------------------------------------------------------------

## 2. The prover adapter

### 2.1 Placement and the theory pin

`apps/fabric-broker/src/prover/mod.rs` (worker, options, `Gate`, theory pin) and `prover/map.rs` (pure mapping), in
every build. `apps/fabric-broker/Cargo.toml` gains `fabric-broker-policy` as a path dependency and nothing else;
`Cargo.lock` changes in the `fabric-broker` entry only. The fitness test forbids the policy crates in the core and the
store and forbids `apps/fabric-broker` a direct dependency on `fabric-policy`, `fabric-token` and `fabric-demo`
(`crates/fabric-broker-core/tests/fitness.rs`): `PolicyError` comes only through the `fabric_broker_policy`
re-export. The worker does not implement `ports::Prover`; the shell gives the core `IsabelleProver::identity()` for
`Attachments.prover` and hands jobs to the worker.

Theory pin (P1 a). `prover/mod.rs` holds `BUILT_THEORY` (THY by `include_str!`), `BUILT_SESSION_FILES` (`ROOTS` and
`Fabric_Broker/ROOT`, the same way), and `built_theory_sha256()`, the `fabric_broker_policy::report::sha256_hex` of
`BUILT_THEORY`, computed once (`OnceLock`). The gate hashes the raw bytes of the same file, so the two hashes are equal
exactly when the bytes are. Cargo rebuilds the broker when the files change. The pin is enforced in three places: the
files under `RULES_DIR` before each gate start (2.2), the started gate's `theory_sha256()` (2.2), and each `Check`'s
`theory_sha256` (2.4). Why the session files too: `ROOTS` and the session's `ROOT` decide which theory Isabelle loads
as the session `Fabric_Broker`, so a changed `ROOT` could load another theory while the pinned file stays in place.
Residuals: the gate reads the hash before `session_start` builds the heap from the same directory, so an edit between
the two is not caught (T8); another `ROOT` that also defines `Fabric_Broker` is left to Isabelle's refusal of a
duplicate session.

### 2.2 Worker

When it starts (P1 c). The shell holds the `ProverOptions` until the first `route` in which `broker.store_ready()` is
true, and only then calls `IsabelleProver::spawn`. The store has then created the data directory and holds its lock,
so the lock guards `<data_dir>/prover` and the server name: a second broker on the same directory runs in the
unavailable mode, never starts a prover while the first holds the lock, and never touches the first one's `prover/`
or server (p44). `Broker` never leaves `Ready` except by halting, so the worker's health notices always reach a ready
core. Until the worker starts, the identity's health `starting` stands. In the same first `route` with the store open,
before the worker starts, the shell removes `<data_dir>/prover` once in every configuration (X10), so a check theory a
killed broker left goes even in a run without a prover (sh05, p45).

One worker thread for the broker's life (`fabric-prover`), owning the gate; channel `Msg { Job(Box<ProofJob>),
Started(Result<Box<dyn Gate>, PolicyError>), Dropped, Stop(Sender<()>) }`; while `Down` with a retry it also wakes at
the retry time.

| State | Health on entry | `Job(job)` | Transition |
|---|---|---|---|
| `Starting` | `starting` | `no_report(job, STARTING)` at once | `Started(Ok(g))`: if `g.theory_sha256() != built_theory_sha256()`, `g` to a dropper and `Down` for the run (line T below); else `Ready`, failures = 0. `Started(Err(e))`: failure |
| `Ready(g)` | `ready` | 2.3 | `GateFate::Restart`: `g` to a dropper (its stop may block over a minute), then failure |
| `Down { retry_at }` | `unavailable` | `no_report(job, NOT_RUNNING)` at once | When `retry_at` is set and past and no dropper is alive (each dropper sends `Dropped` after its drop; the wait has no bound): `Starting` |
| any | | | `Stop(ack)`, the channel closed, or the main loop gone (`emit` returns false): 2.5 |

Failure rule (X8). failures += 1. If failures <= `restart_waits.len()`: `Down { retry_at: now + restart_waits[failures -
1] }`, stderr `fabric-broker: the rule prover could not start (<category>); it is tried again in <n> s` (or `the rule
prover failed (<category>); ...` for a broken gate). Else `Down { retry_at: None }`, stderr `fabric-broker: the rule
prover failed again (<category>); it is not tried again in this run; quit Fabric Calendar and open it again`.
`<category>` is one of `I/O`, `protocol`, `Isabelle`, `JSON`, `render`; no error text, path or calendar value reaches
stderr.

Entering `Starting` (precondition: no dropper alive):
1. Remove `work_dir` with `remove_dir_all` (NotFound ignored), then create it with a non-recursive `DirBuilder`, mode
   0700 under `cfg(unix)`. An error is a failure (`I/O`).
2. Read `<rules_dir>/Fabric_Broker/Fabric_Broker.thy`, `<rules_dir>/ROOTS` and `<rules_dir>/Fabric_Broker/ROOT`. A read
   error is a failure (`I/O`). If the theory's `sha256_hex` differs from `built_theory_sha256()`, or a session file
   differs from the built one: `Down` for the run and line T: `fabric-broker: the rule theory or its session files in
   FABRIC_BROKER_RULES_DIR are not the ones this broker was built with; the rule prover does not run in this run`. No
   gate is started.
3. A starter thread runs `(options.starter)(&options.gate)` and sends `Started`. A send after the worker is gone fails,
   and the starter stops the gate itself.

Dropper (every "`g` to a dropper" above). A thread that calls `g.stop()` (result ignored), then drops `g`, then sends
`Dropped`. It calls `stop` itself, not only through `Drop`, because a `Gate` double's `Drop` does not call
`Gate::stop`; for `BrokerGate` the later `Drop` calls `stop` again, which returns at once.

Health is emitted only on change, through `ProverEvent::Health`, and the overrun poll (2.3) shares the same record of
the last health sent. `shell::input_of` turns `Event::Health(h)` into `Input::ProverHealth(h)`, and `Shell::turn` also
tells the synthetic agent (`set_prover_health`).

Pending health (the shell's own inputs). `emit` and `route` get only `&Broker`, so they cannot feed an input, and the
main thread must never send into its own bounded channel (it is the only receiver: a full channel deadlocks). So
`Shell` holds `pending_health: Option<ProverHealth>`:
- a failed `prove` send (the worker is gone) sets it to `Unavailable` once (`prover_lost` stops repeats; the core's
  watchdog ends the open proofs);
- the overrun poll (2.3) sets it when `poll` returns `Some`.

`Shell::next_due_ms` returns `Some(0)` while it is `Some`, so `next_event` returns at once and main feeds
`Input::Tick`. `Shell::turn` first runs the overrun poll, then takes `pending_health` and, if there was one, calls
`broker.handle(Input::ProverHealth(h), now_ms)`, tells the agent and emits, and only then handles its own input.

### 2.3 One job in `Ready`

```
if now_ms() >= job.deadline_ms            -> no_report(job, PAST_DEADLINE)
(context, action) = context_of(&job)      // Err(JobRefused(t)) -> no_report(job, t); gate kept
mark the check running until job.deadline_ms (shared record)
result = gate.check(&context, &action)
clear the mark
(done, fate) = done_of(&job, &context, built_theory_sha256(), result); emit Done(done)
fate == Restart -> failure (2.2); else, if the overrun poll had reported `unavailable`, emit Health(Ready)
```

Overrun. While a check runs, `IsabelleProver::next_due_ms()` returns the job's `deadline_ms`; the shell includes it in
the main loop's due time and calls `IsabelleProver::poll(now_ms)` at the start of every `turn`. `poll` returns
`Some(Unavailable)` once when a check is still running at or past its job's deadline, and records it as the last
health sent; the shell puts it in `pending_health` (2.2), which the same `turn` feeds to the broker and the agent
before its own input. No thread is added: the worker is blocked in `check`, so the main loop does the timing. The
synthetic agent proposes nothing while health is not `ready`. `Health(Ready)` after an overrun comes only with fate
`Keep`, that is after `Ok(..)` or `Err(Render)`; after `Err(Io | Protocol | Isabelle | Json)` the gate restarts and
health goes `unavailable`, `starting`, `ready` through 2.2.

| Bound | Value | Ends as |
|---|---|---|
| core watchdog | 35 s from the facts commit (W4D) | `undecided`; a late `ProofDone` is ignored by job id |
| gate check limit | `CHECK_TIMEOUT`, 30 s from the check's start (PLAN 5), also Isabelle's watchdog | report with outcome Undecided |
| socket read | the check limit plus 30 s per message | `Err(Io)`: no report, restart |
| gate start | 120 s for the server, 600 s for the session | health `starting`; jobs are answered at once |
| quit | 1.5 s (`QUIT_BOUND`) | 2.5 |

### 2.4 Mapping (`prover/map.rs`, pure)

`context_of(job) -> Result<(Context, Action), JobRefused>`; every failure is a fixed text, never a panic.

| `Context` field (MODEL) | From (PORTS) | Rule / refusal text |
|---|---|---|
| `actor` | `facts.actor` | `Agent(id)` to `Actor::Agent(id)`; `System` to `Actor::System` |
| `paused`, `now_ms`, `result_held`, `pause_epoch`, `revocation_epoch` | same | copied |
| `session` | `facts.session` | `ActiveSession { id: session_id, last_active_ms }` |
| `profile` | `facts.profile` | `Profile::from(&rows)` |
| `connected` | `facts.connected` | ids as strings |
| `calendars` | `facts.calendars` | `Calendar { account, id, writable, drafts: agent_drafts }` |
| `warning_calendars` | `facts.warning_calendars` | `CalendarKey::new(account, calendar)` |
| `grants` | `facts.grants` | table below |
| `recipient_sets` | `facts.recipient_sets` | `RecipientSet { id, members, domains }` as written (not lowercased: can only match less) |
| `event` | `facts.event` | `None` to `None`; `Some(o)`: `o.calendar` must equal the action's account and calendar, else OTHER_CALENDAR; `Event { id, revision, fabric_created, recurring, default_type, timed, invitation, attendees, occurrence }`, each attendee `Email::new` then `Recipient::from_email`, else BAD_ATTENDEE |
| `digest` | `job.action_digest` | strip `sha256:`; the rest must be 64 lowercase hex characters, else BAD_DIGEST |
| `relied` | `facts.relied` | strings |
| `pending_effects` | `facts.pending_effects` | `PendingEffect { account, calendar, event_id: provider_event_id }` |

| `model::Grant` | From `GrantFact { grant, used, ended }` |
|---|---|
| `id` | `grant.grant_id` |
| `tools` | `calendar_list` to `Tool::List`, `calendar_availability` to `Availability`, `calendar_read` to `Read`, `propose_create` to `Create`, `propose_update` to `Update`, `propose_invitation` to `Invite`, `propose_response` to `Respond`, `propose_delete` to `Delete`; the status and result tools are dropped (narrows) |
| `duration` | `OneRequest { action_digest }` to `OneRequest { digest: hex (same check), used }`; `Session { session_id }` to `Session { session_id }`; `UntilRevoked` to `UntilRevoked` |
| `ended` | `ended` (the core's "ended or no longer valid") |
| `calendars` | `scope.calendars` as `CalendarKey` |
| `window` | `scope.window`: `None` to `None`; each end through `unix_millis()`, `None` there is BAD_WINDOW |
| `fields` | `scope.fields`: `None` is empty (covers nothing) |
| `recipients` | `Exact(a)` to `Exact` as written; `RecipientSets(ids)` to `Sets(ids)` |
| `send_updates` | `From` |

Action: `ProofAction::Agent(a)` to `Action::from_tool_call(a)`, `Err` is NO_ACTION; any other `ProofAction` (the enum is
`#[non_exhaustive]`) is UNKNOWN_ACTION. Identifiers outside the theory's charset are not pre-checked: `check` returns
`Err(Render)`, so no report, and the gate is kept.

`done_of(job, context, expected_theory, result) -> (ProofDone, GateFate)`; the first matching row decides:

| `result` | `ProofDone` | Fate |
|---|---|---|
| `Err(Render)` | `no_report(job, RENDER)` | Keep |
| `Err(Io)`, `Err(Protocol)`, `Err(Isabelle)`, `Err(Json)` | `no_report(job, FAILED)` | Restart |
| `Ok(c)`, `c.theory_sha256 != expected_theory` | `no_report(job, OTHER_THEORY)` | Restart (the next start meets the pin) |
| `Ok(c)`, `c.action_digest != context.digest` or `c.context_digest != fabric_broker_policy::context_digest(context)` | `no_report(job, UNBOUND)` | Keep |
| `Ok(c)`, `proof_report` or `ruling` fails, or an `Id::new` fails | `no_report(job, NOT_FIT)` | Keep |
| `Ok(c)`, otherwise | `report: Some(proof_report(&c, &job.request_id, job.purpose, &format_timestamp(job.started_at_ms)))`, `ruling: ruling(&c)`, `authorizing_grants`: `Id::new` each, `undecided_reason: Some(r)` exactly when the verdict is `Undecided { reason: r }`, `check_theory_text: Some(c.theory_text)` | Keep |

`no_report` is `report: None, ruling: None, authorizing_grants: [], undecided_reason: Some(text), check_theory_text:
None`, with `job_id`, `request_id` and `purpose` copied. `proof_report` restores `sha256:` on the action digest, so the
core's binding (W4D D39) compares equal strings; the `UNBOUND` row makes the binding hold for values the adapter did not
copy itself.

Fixed texts (`pub const` in `map.rs`): STARTING "the rule prover is starting"; NOT_RUNNING "the rule prover is not
running"; PAST_DEADLINE "the proof's deadline passed before its check started"; OTHER_CALENDAR "the observation names
another calendar"; BAD_ATTENDEE "an observed attendee is not an address"; BAD_DIGEST "the action digest is not
sha256"; BAD_WINDOW "a grant window is not a time"; NO_ACTION "no broker action for this tool call"; UNKNOWN_ACTION
"this adapter does not know the action"; OTHER_THEORY "the check ran on another rule theory"; UNBOUND "the check is
bound to another action or context"; NOT_FIT "the proof did not fit the contract"; RENDER "a fact could not be written
for Isabelle"; FAILED "the rule prover failed". They are never stored or shown (X12); the owner's view of an absent
prover is the health.

### 2.5 Stop

Every exit path of main after the shell exists (end of input, read failure, halt, stdout failure) ends with
`shell.stop(&events)`, after the last frames are written. Configuration and argument errors happen before any thread.

`Shell::stop` drops the provider's channel and its own sender of the event channel, then calls
`IsabelleProver::stop(QUIT_BOUND, drain)`, where `drain` empties the main event channel, so a worker blocked on the
full channel can read its `Stop`. `stop` sends `Stop(ack)` and, until the bound, waits for the acknowledgement in 20 ms
slices and calls `drain` between them. Without an Isabelle worker it returns at once (sh03).

Worker on `Stop` (P1 d): `Ready` stops its gate (errors ignored), then every state removes `work_dir`; then it
acknowledges and exits. The same happens when the main loop is gone or the handle is dropped without `stop` (its `Drop`
sends `Stop` without waiting). A starter or dropper still alive keeps its own thread until the process ends.

No acknowledgement within the bound (P1 d): the calling thread removes `work_dir` itself with `remove_dir_all`, which
removes a `Check_*.thy` that a running check still holds (Unix allows removing an open file), and `stop` returns false.
This finishes inside the relay's 2 s drain. A server not stopped in time may outlive the broker (K1): the next start
with the prover finds it by its stable name and uses it (L4). A SIGKILL from the relay leaves files; the next start
removes them once the store is open (X10).

### 2.6 Gate configuration

`gate_config(s, data_dir)` = `GateConfig { isabelle_home, user_home, rules_dir` from `s`, `work_dir:
data_dir.join("prover")`, `server_name: server_name(data_dir)`, `check_timeout: CHECK_TIMEOUT` (30 s),
`consolidate_delay_seconds: 0.1 }`. `server_name` is `fabric-broker-` and the first 16 characters of
`sha256_hex(data_dir.as_os_str().as_encoded_bytes())`. `ProverOptions::new` uses it, `RESTART_WAITS`, and the real
starter (`BrokerGate::start(config.clone())`, boxed as `dyn Gate`). Its `gate` and `restart_waits` fields are public,
so a live test can shorten `check_timeout`; any `GateConfig` still runs the real `BrokerGate` and still meets the theory
pin.

---------------------------------------------------------------------------------------------------------------------

## 3. Configuration

### 3.1 Broker: exit 78, one stderr line naming variable and path, before stdin or threads

| Variable | Refused when |
|---|---|
| `FABRIC_BROKER_ISABELLE_HOME` | not absolute; `<it>/bin/isabelle` is not a file (`metadata`, which follows a symbolic link); under `cfg(unix)` also when no execute bit is set |
| `FABRIC_BROKER_ISABELLE_USER_HOME` | not absolute; not an existing directory (the broker never creates it) |
| `FABRIC_BROKER_RULES_DIR` | not absolute; `ROOTS`, `Fabric_Broker/ROOT` or `Fabric_Broker/Fabric_Broker.thy` is not a file |
| the three | one or two set: "fabric-broker: the rule prover needs FABRIC_BROKER_ISABELLE_HOME, FABRIC_BROKER_ISABELLE_USER_HOME and FABRIC_BROKER_RULES_DIR together, or none" |

Read in every build (X17). Nothing heavier than `metadata` runs here; the theory is checked by the worker (2.2), so a
wrong theory is a prover failure, not exit 78 (P1). An unusable distribution or an unwritable user home shows as a
failed start (`unavailable`). The refusal of an unknown `FABRIC_BROKER_*` name and the usage line list six names
(W4D D27 named three). `Config` has `isabelle: Option<IsabelleSettings>` and `practice()`. `EXIT_IO` (74) lives in
`config.rs`, so the shell and main share it.

### 3.2 Who sets them

Development and tests: the caller passes the paths explicitly; the broker never discovers `.tooling` and never
searches `PATH`. Live tests read the three variables and default to the repository's layout when all three are absent;
a partial set fails the test (P6). Host, debug: `BrokerStore::from_environment` reads the three names and forwards them
in `variables()` when all three are set and absolute, with any store (X16); one or two of them, or a relative path, is
a host configuration error ("nothing was started"). Host, release: ignores them (no prover until PLAN Q09). The host's
trace line gains ` prover` when it passes them. Relay: `ENVIRONMENT_ALLOWLIST` holds the three names beside the three
W04 names, with the constants `BROKER_ISABELLE_HOME`, `BROKER_ISABELLE_USER_HOME`, `BROKER_RULES_DIR`, which
`relay/src/lib.rs` re-exports and the host imports, so the names are spelt once on the desktop side; the relay does not
depend on the broker crate, so its tests pin the text. The relay clears the environment, then sets exactly the
allowlisted names and, on Unix, the fixed `PATH=/usr/bin:/bin:/usr/sbin:/sbin` (X18); `PATH` itself stays out of the
allowlist, so the host's value never passes. The harness (`harness/serve.ts`) passes the three names when all three
are set in its own environment, exits 64 with one or two of them, and gives the broker the same fixed `PATH`. The
packaged check (`verify-desktop.sh --packaged`) unsets the three names in its launches, so values exported in a
terminal never turn its run into a practice run.

### 3.3 Attachments and health (`Plan::of`, 8)

| Configuration | `Attachments.prover` | `provider_present` | Requests |
|---|---|---|---|
| no Isabelle, no synthetic agent | `None` | false | `undecided` at once, no report (W4D D14) |
| no Isabelle, synthetic agent (debug) | `SyntheticClassifier` (unchanged, O5) | false | card, then `undecided` (W04) |
| Isabelle, no synthetic agent | `{Isabelle, Starting}` | false | proved; a fresh Permit ends `undecided` (no provider) |
| Isabelle and the synthetic agent (debug): practice | `{Isabelle, Starting}` | true | the whole path |

Agent calls are not refused at entry while the prover is absent (the contract has no error for it, E4); each ends
`undecided` within one turn of the main loop. The build label in `hello` is `<version> CD-W04`, with ` synthetic` or
` synthetic practice` after it.

### 3.4 Data directory

| Path | Written by | Removed |
|---|---|---|
| `<data_dir>/prover/`, mode 0700 on Unix | the worker, non-recursively, only after the store is open | whole: once by the shell when the store opens (every configuration), before every gate start, and at every stop (2.5) |
| `prover/Check_NNNNNN.thy` (plain calendar data) | the gate, per check | by the gate's guard after each check; with the directory, by the worker or the stopping thread |
| `prover/isabelle-server-std{out,err}.log` | the gate at start (created even when the child exits at once) | with the directory: at the next start or stop |
| `<user_home>/.isabelle/...` heaps, registry | Isabelle | never by the broker |
| Isabelle's temporary prefix (`ISABELLE_TMP_PREFIX`, under `/tmp` by default) | Isabelle | never by the broker (K13) |

No practice file exists (X3).

---------------------------------------------------------------------------------------------------------------------

## 4. The practice calendar

### 4.1 Existence

`apps/fabric-broker/src/practice.rs`, declared `#[cfg(debug_assertions)] pub mod practice;` in `lib.rs`. Created only
by `Plan::of` when `config.practice()` (or by a live test). Guards: a release build does not compile it and refuses the
synthetic agent (W4D 3.3); the synthetic agent needs the test key (same); the host passes the synthetic agent only with
`FABRIC_DESKTOP_DATA_DIR` (W4D D26). It is a provider double in product code, which W4D D13 excludes: the amendment of
D13 and D26 that makes it a debug-only exception is proposed with this step's pull request, and merging waits for the
owner's answer (O1). It never counts as evidence for a Google gate (practice rule P7).

The practice rules it keeps:
- practice rule P1: compiled only with `debug_assertions`; a release build refuses what would start it (the synthetic
  agent, exit 78);
- practice rule P2: only with the test key and a scratch data directory chosen by the person running it; never with
  the Keychain store;
- practice rule P3: it carries out only what the core dispatches, which needs a consumed authorization resting on a
  live Isabelle permit; without a prover nothing is dispatched;
- practice rule P4: it answers `Provider::dispatch` as a provider would: applied, transient, rejected or no answer; a
  create uses the reserved event id, and a retry of an applied effect is the same effect; it changes only its own
  seeded calendar;
- practice rule P5: it is labelled where it shows: the synthetic agent's and account's names say "test build", the
  build label says `synthetic practice`, and the practice line on the page says what a practice run does; the contract
  has no marker that tells practice records from Google outcomes (E3);
- practice rule P6: its state lives in memory only; no plaintext file, no network;
- practice rule P7: it never counts as evidence for a Google gate.

### 4.2 State

In memory: `Arc<Mutex<State>>`, shared by the provider worker, the shell's observation hook and the synthetic agent.
Seeded by `PracticeCalendar::new(now_ms, faults)`: on (`synthetic-account`, `synthetic-calendar`) one event Fabric did
not make, `FOREIGN_EVENT_ID = "practice-foreign-1"`, revision `"1"`, timed, single, default type, no attendees, title
`FOREIGN_TITLE` = "Practice: dentist (not made by Fabric)", seven days after `now_ms` at 09:00 UTC, one hour long.
Revisions are decimal strings outside (a counter inside). State per event: id, revision, title, start, end (contract
values as received, never parsed), `fabric_created`, the creating request's id, `deleted`; per applied effect: its
`EventRef`; a dispatch counter. A poisoned mutex: `dispatch` answers `NoAnswer`, `observation` returns `None`. No title
reaches stderr or the `Debug` form.

A broker restart starts a fresh practice calendar, while the store keeps its `applied` requests (they never reconcile,
K6). The README tells the person to use a new scratch directory per practice run; on a reused one, earlier requests
show done for events the practice calendar no longer holds.

### 4.3 `Provider::dispatch`, on the provider worker (`shell.rs`)

First failing row decides; `finished_at_ms` is the wall clock; every call increments the dispatch counter.

| # | Check | Answer |
|---|---|---|
| 1 | `now_ms() >= order.deadline_ms` | `NoAnswer`, no change |
| 2 | the order holds together: the authorization's `request_id` and `action_digest` are the order's, `consumed_at` is set, the order's account and calendar are the action's, and a create carries a reserved id that is a valid provider reference | `Rejected { ProviderRejected, "the order does not hold together" }` |
| 3 | the account and calendar are the synthetic pair | `Rejected { ProviderRejected, "the practice calendar has only the synthetic calendar" }` |
| 4 | the `effect_id` was applied before | `Applied { event: Some(the same EventRef) }`, no change (a retry, PLAN 11) |
| 5 | the tool is create, update or delete | `Rejected { ProviderRejected, "the practice calendar carries out create, update and delete only" }` |
| 6 | fault `transient > 0` (decremented) | `Transient { retry_after_ms: None }`, no change |
| 7 | create: no event has the reserved id | else `Rejected { ProviderRejected, "an event with this id exists" }`; add it at revision 1, `fabric_created: true`, with its request id |
| 7 | update or delete: the event exists and is not deleted, and the action's revision is the current one | else `Rejected { InvalidArguments, "no such event" }` or `Rejected { RevisionConflict, "the event changed" }`; an update sets the given title, start and end and adds one to the revision; a delete marks it deleted and adds one to the revision |
| 8 | fault `lose_answers > 0` (decremented) | the change is kept, `NoAnswer` |
| 9 | | `Applied { event: Some(EventRef { event_id, revision }) }`; the effect is recorded |

### 4.4 Observation hook (X6)

`PracticeCalendar::observation(&self, action: &agent::Action) -> Option<ObservedEvent>`. `Some` only when the action is
`propose_update` or `propose_delete` on the synthetic pair and the named event exists and is not deleted:
`ObservedEvent { calendar: the synthetic pair, event_id, revision: the current one, fabric_created, recurring: false,
default_type: true, timed: true, invitation: false, attendees: [], occurrence_start_ms: None }`. The shell, for every
proof job of a practice run: if the core left `job.facts.event` `None` and the job's action is an agent call, it sets
`job.facts.event` to the observation. Nothing else of the job changes (`job.fence.event_revision` stays `None`). It goes
when CD-W05 adds the core's observation port.

---------------------------------------------------------------------------------------------------------------------

## 5. The synthetic agent

- `SyntheticAgent::new()` is unchanged (W04: two creates with the classifier; its limit field is `PROPOSALS_PER_RUN`,
  2). `SyntheticAgent::practice(calendar)` runs the script below with the limit `PRACTICE_PROPOSALS`, 4.
- In practice mode it proposes only once the page has subscribed and the store is ready, and while the last health from
  `set_prover_health` is `Ready`. A request is done when it is terminal or `applied` (`applied` stays open until
  CD-W05).
- Seed: `SyntheticAgent::seed()` is unchanged (the agent and the synthetic account).
- The agent remembers A's request id (from the reply) and A's start (it computed it for the proposal).

| Step | Proposal | Expected | Owner |
|---|---|---|---|
| A | `propose_create` as in W04 (title `TITLE`) | `need_approval`, rule `authority`; card; a fresh Permit on the one-request grant; `queued`, `dispatching`, `applied` | Allow once |
| B | `propose_delete` of `FOREIGN_EVENT_ID` at its revision, `send_updates: none` | `blocked`, failing rule `fabric_created` (the first failing block rule in the theory's order) | - |
| C | `propose_update` of A's event (`calendar.event_of_request(A)`: id, revision), start and end 30 minutes later, `send_updates: none` | `need_approval`, Allow, `applied`, revision 2 | Allow once |
| D | `propose_delete` of A's event at revision 2, `send_updates: none` | `need_approval` (a delete always asks), Allow, `applied` | Allow once |

If A ends any other way, or A's practice event is missing, C and D are skipped (stderr `synthetic agent: step A ended
<state>; steps C and D are skipped`) and B still runs; then `synthetic agent: script finished`. Event ids taken from the
practice state are test-build knowledge (W4D D33: no read results yet). `applied` requests stay open and count toward
the 25 open requests per agent (W4D 5.9): about eight runs per scratch directory before `busy`.

---------------------------------------------------------------------------------------------------------------------

## 6. The page (P3: not in this step)

The page is the CD-W08 screens, unchanged but for one static line under the build line in `ui/index.html`:

> In a test build run with the rule checker and the practice calendar, a request that the rule checker proves allowed
> is carried out only on practice data held in memory. Nothing here changes a real calendar.

It has the id `practice-line`, uses the screens' "rule checker" wording and the existing `note` style, and is not
focusable, so the README's keyboard counts hold. No page test pins it. Its wording is conditional, so it stays true in
every build, the product too. `apps/fabric-desktop/README.md` describes the practice run (section 1) and the new
scratch directory per run (4.2). What the page still lacks for a practice run is listed in 9.4.

---------------------------------------------------------------------------------------------------------------------

## 7. Tests

No test gives the core a permit it did not get from a live Isabelle check. `Gate` doubles exist only under
`#[cfg(test)]` in `prover/mod.rs`; their `check` returns only `Err`, or blocks and then returns `Err`; their
`theory_sha256()` returns `built_theory_sha256()` unless the test says otherwise. A double starter may return `Ok` with
such a double (w12, w14, w15): a gate that cannot prove, not a proof. No double ever returns a `Check`. The map tests
u09 to u12, u15 and u16 build `Check` values by hand (u10 with a Permit verdict) and pass them only to `done_of`, a pure
function; they never reach a worker, a `Shell` or a `Broker`. Unix-only assertions (file modes, execute bits, a shell
script standing in for Isabelle) are `#[cfg(unix)]`; CI compiles and tests the workspace on macOS, Linux and Windows
without Isabelle.

The scenarios the "Plan" columns name: S1 to S6 are rows of PLAN 14's required scenarios, S7 is the whole path of PLAN
5's quality targets; R01 to R14 are PLAN 5's requirements.
- S1: "Kill Isabelle or exceed proof budget";
- S2: "Approve an event, then alter time/attendees/body/account";
- S3: "Revoke/pause while proof runs, while queued, or before retry";
- S4: "Expire one-request/session grant; restart broker";
- S5: "Fail audit persistence or lock the database";
- S6: "Invitation recipients include an unapproved address";
- S7: the whole path in the window: received and checking, the card, approval, a fresh Permit, queued, dispatching,
  applied, with the proof id and theory hash on each report, and a blocked request whose denial keeps its failing rule
  (DEF-7).

### 7.1 Unit, CI (`sh scripts/run.sh cargo test`)

`apps/fabric-broker/src/prover/map.rs`:

| Id | Test | Shows | Plan |
|---|---|---|---|
| u01 | `facts_map_to_the_context_field_by_field` | every `Context` field of 2.4 | R06 |
| u02 | `the_digest_loses_its_prefix_and_comes_back_in_the_report` | the digest round trip of the binding (W4D D39) | R06 |
| u03 | `a_digest_that_is_not_sha256_fails_the_job` | BAD_DIGEST | R06 |
| u04 | `status_and_result_tools_are_dropped_from_grants` | a grant covers less, never more | R07 |
| u05 | `a_window_that_is_not_a_time_fails_the_job` | BAD_WINDOW | R06 |
| u06 | `set_domains_are_passed_as_written` | recipient sets as written | R07 |
| u07 | `an_observation_of_another_calendar_or_a_bad_attendee_fails_the_job` | OTHER_CALENDAR, BAD_ATTENDEE | R06 |
| u08 | `request_result_fails_the_job` | NO_ACTION | R06 |
| u09 | `a_block_check_maps_to_report_and_ruling_with_proof_id` | a Block check's report and ruling | R06, DEF-7 |
| u10 | `a_permit_check_names_its_authorizing_grants_in_order` | a Permit check's grants | R06 |
| u11 | `undecided_keeps_its_reason_and_names_no_grant` | an Undecided check | R06 |
| u12 | `a_report_that_does_not_fit_the_contract_is_no_report` | NOT_FIT | R06 |
| u13 | `only_a_render_error_keeps_the_gate` | the five error kinds' fate | S1 |
| u14 | `no_policy_error_text_reaches_the_reason` | X12 | R13 |
| u15 | `a_check_on_another_theory_is_no_report_and_restarts` | P1 a, on each check | R06 |
| u16 | `a_check_bound_to_another_action_or_context_is_no_report` | UNBOUND | R06 |
| u17 | `no_report_copies_the_job_and_names_the_reason` | `no_report` | R06 |

`apps/fabric-broker/src/prover/mod.rs` (`Gate` doubles; tests whose worker reaches a start read the repository's
`isabelle/`, read only):

| Id | Test | Shows | Plan |
|---|---|---|---|
| w01 | `health_is_starting_until_the_gate_starts` | 2.2 | R06 |
| w02 | `the_nth_failure_waits_the_nth_wait_and_the_third_leaves_the_prover_unavailable` | X8 | S1 |
| w03 | `jobs_while_not_ready_are_answered_at_once_with_no_report` | STARTING, NOT_RUNNING | R06 |
| w04 | `a_job_past_its_deadline_is_not_checked` | X11 | S1 |
| w05 | `a_broken_gate_is_dropped_off_the_worker_and_restarted` | 2.2 dropper | S1 |
| w06 | `a_render_error_keeps_the_gate` | 2.4 | S1 |
| w07 | `stop_returns_within_its_bound_while_starting_or_blocked` | X9 | - |
| w08 | `stop_removes_the_work_dir_in_every_state` | P1 d in `Starting`, `Ready`, `Down` | R13 |
| w09 | `the_server_name_is_stable_per_data_dir_and_valid` | 2.6 | - |
| w10 | `a_real_gate_with_no_isabelle_is_unavailable` | the real starter on an empty Isabelle home fails at once on every platform | R06 |
| w11 | `a_rules_dir_with_another_theory_leaves_the_prover_unavailable_and_starts_nothing` | P1 a on the file; a counting starter is never called | R06 |
| w12 | `a_gate_reporting_another_theory_is_dropped_before_any_check` | P1 a on the gate | R06 |
| w13 | `an_unacknowledged_stop_removes_the_check_file_from_the_calling_thread` | P1 d without an acknowledgement | R13 |
| w14 | `a_restart_waits_until_the_broken_gate_is_dropped` | X8, a stop that blocks | S1 |
| w15 | `health_leaves_ready_while_a_check_runs_past_its_deadline` | the overrun poll of 2.3 | S1 |
| w16 | `the_work_dir_is_emptied_and_made_private_before_each_start` | X10, mode 0700 on Unix | R13 |
| w17 | `the_built_theory_is_the_repository_theory` | the embedded theory and session files are the repository's | R06 |
| w18 | `session_files_that_point_elsewhere_leave_the_prover_unavailable_and_start_nothing` | P1 a on `ROOTS` and `ROOT` | R06 |
| w19 | `a_dropped_handle_stops_the_worker` | 2.5 | - |
| w20 | `a_gone_main_loop_stops_the_worker` | 2.5 | - |

`apps/fabric-broker/src/config.rs`:

| Id | Test | Shows | Plan |
|---|---|---|---|
| c40 | `isabelle_variables_are_all_or_none` | 3.1, the three together | DEF-2 |
| c41 | `each_isabelle_path_is_checked_before_anything_starts` | each row of 3.1 (the execute-bit row under `cfg(unix)`) | DEF-2 |
| c42 | `practice_is_debug_synthetic_and_isabelle_only` | X2, debug builds | - |
| c42r | `practice_is_never_on_in_release` | X2 in a release build on macOS (`verify-desktop.sh` runs the release library tests there) | - |
| c43 | `the_refusal_lists_six_names` | 3.1, the unknown-name refusal | - |

`apps/fabric-broker/src/shell.rs`:

| Id | Test | Shows | Plan |
|---|---|---|---|
| sh01 | `the_plan_of_each_configuration_is_the_table_of_plan_of` | the table of 3.3, which is `Plan::of`'s (debug rows under `cfg(debug_assertions)`) | - |
| sh02 | `the_isabelle_worker_does_not_start_while_the_store_is_in_use` | P1 c: with the store held, the broker runs in the unavailable mode and no `<data_dir>/prover` exists | - |
| sh03 | `stop_without_an_isabelle_worker_returns_at_once` | 2.5 | - |
| sh04 | `a_failed_prove_send_feeds_unavailable_once` | pending health of 2.2 | S1 |
| sh05 | `a_left_check_theory_goes_once_the_store_is_open_in_every_configuration` | X10, and kept while another holder has the store | R13 |
| sh06 | `a_request_while_the_isabelle_prover_is_down_ends_undecided` | a down prover: undecided, no proof, no authorization, no attempt | R06 |

`apps/fabric-broker/src/practice.rs`:

| Id | Test | Shows | Plan |
|---|---|---|---|
| pr01 | `create_uses_the_reserved_id_at_revision_1` | 4.3 row 7 | R12 |
| pr02 | `a_retry_of_an_applied_effect_is_applied_once` | 4.3 row 4 | R12 |
| pr03 | `update_needs_the_current_revision` | 4.3 row 7 | PLAN 14 "Provider event changed after preview" |
| pr04 | `delete_leaves_a_tombstone` | 4.3 row 7 | - |
| pr05 | `other_calendars_and_other_tools_are_rejected` | 4.3 rows 3 and 5 | - |
| pr06 | `an_order_that_does_not_hold_together_is_rejected` | 4.3 row 2 | R08 |
| pr07 | `past_the_deadline_is_no_answer_and_no_change` | 4.3 row 1 | - |
| pr08 | `a_lost_answer_applies_and_answers_no_answer` | 4.3 row 8 | R12 |
| pr09 | `transient_changes_nothing` | 4.3 row 6 | - |
| pr10 | `the_observation_names_live_update_and_delete_targets_only` | 4.4 | - |
| pr11 | `the_seed_holds_one_foreign_event_a_week_ahead_at_nine_utc` | 4.2 | - |
| pr12 | `the_debug_form_names_no_title` | 4.2 | R13 |
| pr13 | `a_poisoned_calendar_answers_nothing_and_observes_nothing` | 4.2 | - |

`apps/fabric-broker/src/synthetic.rs` (the W04 tests are unchanged):

| Id | Test | Shows | Plan |
|---|---|---|---|
| sy01 | `the_practice_script_is_create_block_update_delete` | section 5 | S7 |
| sy02 | `it_proposes_only_while_the_prover_is_ready` | section 5 | - |
| sy03 | `applied_counts_as_done` | section 5 | - |
| sy04 | `an_unexpected_end_of_a_skips_c_and_d` | section 5 | - |
| sy05 | `without_practice_the_w04_script_is_unchanged` | X14 | - |
| sy06 | `practice_updates_and_deletes_notify_nobody` | P2 | S6 |

The relay and the host:

| Id | File | Test | Shows |
|---|---|---|---|
| h01 | `apps/fabric-desktop/src-tauri/src/host.rs` | `a_debug_host_passes_the_prover_paths_only_all_three_and_absolute` | X16, 3.2 |
| h02 | `apps/fabric-desktop/src-tauri/src/host.rs` | `a_release_host_ignores_the_prover_variables` | X16 |
| h03 | `apps/fabric-desktop/relay/src/config.rs` | `the_child_gets_a_fixed_path_on_unix_and_never_the_hosts` | X18 |
| h04 | `apps/fabric-desktop/relay/tests/relay.rs` (its own harness) | `prover_names_are_exported_and_allowlisted` | 3.2 |

Extended in this step: `allowlist_names_the_broker_variables` and `broker_variables_are_named_by_the_prefix`
(`relay/src/config.rs`, six names; Isabelle's own `ISABELLE_HOME` and `USER_HOME` are never allowlisted), and
`environment_holds_only_allowlisted_names` (`relay/tests/relay.rs`, which on Unix now requires exactly the fixed
`PATH` and still refuses every other name). No page test (P3).

### 7.2 Process, CI (`apps/fabric-broker/tests/process.rs`, `tests/relay_round_trip.rs`)

| Id | Test | Shows | Plan |
|---|---|---|---|
| p40 | rows of `configuration_errors_exit_78_before_any_frame` | each refusal of 3.1 before any frame (the execute-bit row under `cfg(unix)`) | DEF-2 |
| p41 | `an_isabelle_that_cannot_start_leaves_the_prover_unavailable_and_proposes_nothing` (`cfg(unix)`) | with `bin/isabelle` a script exiting 1, the repository's `isabelle/` and a temporary user home, the broker reports `prover: unavailable` (snapshot or notice), the practice agent proposes nothing, and after quit no `<data_dir>/prover` is left | S1 |
| p42 | `the_synthetic_agent_waits_for_subscribe_asks_and_an_approval_ends_undecided` (W4D p07, unchanged) | a classifier run without Isabelle is unchanged | - |
| p43 | `the_relay_passes_the_three_isabelle_names` (`tests/relay_round_trip.rs`) | through the relay, the broker reads all three (its build label says `synthetic practice`) | - |
| p44 | `a_second_broker_on_a_locked_directory_leaves_the_first_brokers_prover_alone` (`cfg(unix)`) | P1 c across processes: only after the first broker reports `prover: unavailable` does the test put a marker in `<data_dir>/prover`; a second broker answers `store_unavailable`; the marker is still there. It concludes only within the first restart wait less a second and fails with "too slow to conclude" otherwise, because the first broker's next start would remove the marker | - |
| p45 | `a_start_without_isabelle_removes_a_check_theory_a_killed_broker_left` | X10 across processes | R13 |

### 7.3 Live (real Isabelle; never in CI; P6)

`apps/fabric-broker/tests/live.rs`, `#![cfg(all(unix, debug_assertions))]`. Each test is `#[ignore = "live: needs
Isabelle; run sh scripts/verify-desktop.sh --live-proofs"]`, takes one static `LIVE` lock, uses the test key and a
temporary data directory of its own (hence a server name of its own), and never reaches the Keychain. Isabelle paths:
the three variables when all are set, else the repository's layout; a partial set, or no `bin/isabelle`, fails the
test with "run sh scripts/setup-demo.sh by hand". The in-process tests build `Broker::start(shell::opener(..), ..,
plan.attachments(), ..)` and a `Shell` from a `Plan` holding `ProverChoice::Isabelle(ProverOptions::new(..))`, a
`PracticeCalendar` they keep a clone of and, where the script runs, `SyntheticAgent::practice`; they drive owner frames
and worker events through `shell::next_event`, `shell::input_of` and `Shell::turn`, the same code as `main.rs` (P1 b).
The process tests among them run the built broker. Every test is a top-level function, so its name is its full test
path and `--exact <name>` matches it.

How to run them: all of them, after the first test alone and the rule theory's corpus, with
`sh scripts/verify-desktop.sh --live-proofs` (7.4); one of them with `sh scripts/run.sh cargo test --locked -p
fabric-broker --test live -- --ignored --exact <name> --nocapture`. Run them on an otherwise idle Mac: the in-process
stop must be acknowledged within `QUIT_BOUND`, and the margin is thin (K15). On 2026-11-16 the whole mode took about six
minutes on one Mac: the corpus about 215 s, the live binary about 140 s.

| Id | Test | Shows | Plan |
|---|---|---|---|
| L1 | `the_practice_script_is_asked_approved_permitted_applied_and_blocked` | the practice script: three `applied`, one `blocked` on `fabric_created`; `get_proof` reports with Isabelle `Isabelle2025-2`, the theory hash `sha256:` and `built_theory_sha256()`, 18 verdicts; the practice event created, at revision 2, deleted; the foreign event untouched; after quit no file under `<data_dir>/prover`; no calendar text (titles; account, calendar, agent and event ids, plain and as the check theory's byte lists) in a file changed during the test under the work directory, the user home or Isabelle's temporary prefix | S7, DEF-7, R06, R13 |
| L2 | `killing_isabelle_mid_check_is_undecided_then_ready_again` | the server killed while A's fresh-permit check theory is on disk: A ends `undecided` with no dispatch and no authorization, no check theory is left, health `unavailable` then `ready`, the next request is decided | S1 |
| L3 | `a_check_over_budget_is_undecided` | a 1 ms check limit: outcome Undecided, no failing rule, no grant, no authorization, no dispatch, the gate kept | S1 |
| L4 | `a_killed_broker_restarts_with_an_empty_work_dir_and_one_server` | the built broker killed with SIGKILL mid-run; the next start on the same directory empties `prover/`, finds the orphan server by its name or starts one, never a second, reports ready, and leaves nothing at quit | R12, K1 |
| L5 | `a_lost_answer_is_outcome_unknown_and_never_sent_again` | `Faults { lose_answers: 1 }`: the calendar changes once, the request ends `outcome_unknown` and is never sent again | PLAN 14 "Google accepted insert; response lost" (no reconciliation yet), R12 |
| L6 | `a_transient_failure_is_proved_again_before_the_retry` | `Faults { transient: 1 }`: two attempts, each on its own Isabelle permit, the second applied | PLAN 5 "a fresh proof per retry" |
| L7 | `revoke_during_a_live_proof_dispatches_nothing` | revoke while the fresh proof runs: the result is not used, the request is proved again and blocked on `relied_grant_live`, with a fresh Isabelle denial proof | S3, R08 |
| L8 | not written | `pause_while_queued_voids_the_authorization` cannot be live: the core dispatches an accepted permit in the same step, so `queued` has no public entry. Covered with a gate double by `crates/fabric-broker-core/tests/races.rs` `pause_while_queued_refuses_the_handoff` and `revoke_before_a_retry_refuses_the_retry`, and `crates/fabric-broker-core/src/service/tests.rs` `a_fence_changed_while_queued_voids_the_authorization`, `a_grant_ended_while_queued_voids_the_authorization`, `an_agent_paused_while_queued_voids_the_authorization` | S3 |
| L9 | not written | `approve_then_alter_needs_a_new_approval` needs agent calls the practice script does not make | S2 |
| L10 | not written | `restart_keeps_a_consumed_authorization_used` needs a restart of `Broker` and `Shell` on one directory with one `PracticeCalendar`, which the live harness does not offer | S4 |
| L11 | `the_built_broker_starts_isabelle_with_an_empty_environment` | the built broker with the relay's environment (its variables and the fixed `PATH`, nothing of the test's) starts Isabelle and reports ready; it runs first and alone | X18 |
| L12 | not written | `an_invitation_to_a_stranger_asks_with_no_strangers` needs agent calls the practice script does not make | S6 |
| L13 | `quit_stops_isabelle_within_the_relay_drain` | once the built broker reports ready, end of input stops it within the relay's 2 s drain, with its server, and `prover/` is gone | X9 |
| L14 | `checks_are_timed` | prints the prover's start and the mean, median and maximum of the script's checks; a mean over 1.5 s fails, because W03 then reopens the proof timeout | PLAN 5 |
| L15 | `pause_during_a_live_proof_dispatches_nothing` | pausing every agent while the fresh proof runs: nothing dispatched, blocked on `agent_not_paused` with a fresh denial proof | S3, R08 |
| L16 | `killing_isabelle_ends_the_open_request_undecided_then_ready_again` | the server killed between checks while a request is open: `undecided`, nothing dispatched, health back to ready, the next request decided | S1 |
| L17 | `revoke_before_a_live_retry_blocks_the_retry` | `Faults { transient: 1 }`, revoke while the request waits for its retry: the retry's fresh proof is blocked on `relied_grant_live`; one dispatch, one attempt, one permit | S3, R08 |
| L18 | `the_theory_pin_refuses_a_changed_rule_file` | P1 a: a theory one byte different is refused before any gate starts; no server, no proposal, nothing proved | R06 |
| L19 | `the_theory_pin_refuses_session_files_that_load_another_theory` | P1 a: `ROOTS` or the session's `ROOT` pointing at a copied, changed theory is refused the same way | R06 |

S5 (audit failure, a locked database) does not depend on the prover and is left to the W04 core tests (which of them
cover it was not checked).

### 7.4 `scripts/verify-desktop.sh --live-proofs`

A third mode beside the sources and `--packaged`; macOS only; no Node needed. One or two of the three variables set
stops it with the all-or-none line, and a relative path stops it too; with none, it exports the repository's layout
(`.tooling/isabelle/Isabelle2025-2`, `.tooling/isabelle/home`, `isabelle`). Isabelle must be installed in the
repository's layout in every case (the corpus uses it) and at the given paths; otherwise it stops with "run sh
scripts/setup-demo.sh by hand" (it never installs Isabelle). Then:
1. `sh scripts/isabelle.sh version` must print `Isabelle2025-2`.
2. L11 alone: `sh scripts/run.sh cargo test --locked -p fabric-broker --test live -- --ignored --exact
   the_built_broker_starts_isabelle_with_an_empty_environment`, its output in a file under `mktemp -d`, printed; the
   step fails unless the command succeeded and the file holds `test result: ok. 1 passed`, so a name that matches
   nothing cannot pass (no pipe: `set -e` does not see a failure inside one).
3. The corpus: `cargo test --locked -p fabric-broker-policy --test corpus -- --ignored --test-threads=1`.
4. The live tests: `cargo test --locked -p fabric-broker --test live -- --ignored --test-threads=1 --nocapture`.

No Keychain. `.github/workflows/ci.yml` is unchanged.

---------------------------------------------------------------------------------------------------------------------

## 8. Modules and interfaces

The rustdoc is the reference: `sh scripts/run.sh cargo doc -p fabric-broker --no-deps --document-private-items` (the
crate-private gate seam needs `--document-private-items`), and `-p fabric-desktop-relay` for the relay. In short:

- `config` (`apps/fabric-broker/src/config.rs`): `config_from` reads the six variables; `ISABELLE_HOME`,
  `ISABELLE_USER_HOME`, `RULES_DIR`; `IsabelleSettings { isabelle_home, user_home, rules_dir }`; `Config::isabelle`
  and `Config::practice()`; the exit codes `EXIT_IO` (74), `EXIT_HALT` (75), `EXIT_CONFIG` (78).
- `prover` (`prover/mod.rs`): `CHECK_TIMEOUT`, `RESTART_WAITS`, `QUIT_BOUND`; `built_theory_sha256()`; `gate_config`,
  `server_name`; `ProverOptions { gate, restart_waits }` and `ProverOptions::new`; `ProverEvent { Done, Health }` and
  `Emit`; `IsabelleProver` with `identity`, `spawn`, `prove`, `next_due_ms`, `poll` and `stop`. Crate-private: `Gate`,
  `Starter`, `remove_work_dir`.
- `prover::map` (`prover/map.rs`, pure): `JobRefused`, the fixed texts, `context_of`, `GateFate`, `done_of`,
  `no_report`.
- `practice` (`practice.rs`, debug builds): `FOREIGN_EVENT_ID`, `FOREIGN_TITLE`, `Faults`, `PracticeEvent`,
  `PracticeCalendar` with `new`, `provider`, `observation`, `event`, `event_of_request`, `dispatches`;
  `PracticeProvider`, which implements the core's `Provider`.
- `synthetic` (`synthetic.rs`, debug builds): `SyntheticAgent::practice`, `set_prover_health`, `PRACTICE_PROPOSALS`.
- `shell` (`shell.rs`): `QUEUED_EVENTS`, `now_ms`, `Event`, `ProverChoice { None, Classifier (debug), Isabelle }`,
  `Plan { prover, work_dir, practice (debug), agent (debug) }` with `Plan::of` (main's only way to make one) and
  `attachments`; `opener`, `build_label`, `next_event`, `input_of`; `Shell` with `new`, `emit`, `route`, `turn`,
  `quit`, `next_due_ms`, `stdout_mut` and `stop`.
- `main.rs` keeps the argument and configuration refusals, `Broker::start`, the channel, the stdin reader thread, the
  loop over `shell::next_event` (`None` is `Input::Tick`), its handling of `Event::End` (`shell.quit`, exit 0) and
  `Event::Failed` (stderr, exit 74) before `shell::input_of`, then `shell.turn`; the exit codes; and `shell.stop` on
  every exit path.
- Relay (`apps/fabric-desktop/relay`): `BROKER_ISABELLE_HOME`, `BROKER_ISABELLE_USER_HOME`, `BROKER_RULES_DIR`,
  re-exported from the crate root; `ENVIRONMENT_ALLOWLIST` with six names (and `SystemRoot` on Windows);
  `BrokerConfig::command` adds the fixed `PATH` on Unix.
- Host (`apps/fabric-desktop/src-tauri/src/host.rs`): `BrokerProver { isabelle_home, isabelle_user_home, rules_dir }`
  and `BrokerStore::prover`, filled by `BrokerStore::from_environment` in a debug build.

---------------------------------------------------------------------------------------------------------------------

## 9. Risks, requests, questions, not checked

### 9.1 Risks

- K1 An Isabelle server can outlive its broker: after a SIGKILL of the broker, or a stop past `QUIT_BOUND`, the server
  keeps running. The next start with the prover configured finds it by its stable name and uses it, then stops it at
  quit (L4 showed both server processes surviving a kill and being reused). A start without the prover configured
  cannot stop it, having no `isabelle`; a check theory left behind is still removed by every start once the store is
  open. A panic of the main thread drops the prover's handle, which sends the stop without waiting.
- K2 A hung check holds the worker up to a minute per read; health turns `unavailable` at the job's deadline (2.3) and
  the request ends `undecided` by the core's watchdog.
- K3 The broker's `PATH`: Isabelle does not start under an empty environment (L11 failed before X18). The relay now
  gives the broker a fixed `PATH=/usr/bin:/bin:/usr/sbin:/sbin` on Unix, so "the broker's environment holds only its
  own variables" is no longer true on Unix; the value is fixed, never the host's, and the relay's tests pin it. The live
  test L11 still says "empty environment" in its name; the second step of `--live-proofs` says "the relay's
  environment (its variables and a fixed PATH)".
- K4 The first start may build Isabelle's session heap for minutes (health `starting`).
- K5 The observation of an update or delete is filled in by the shell, not in the core's facts transaction (X6); debug
  builds only.
- K6 `applied` never reconciles before CD-W05: `busy` after about eight runs per scratch directory, and a restart shows
  done for events the practice calendar lost (the README says so).
- K7 Isabelle's server logs are not read; they go with `prover/` at every start and stop. L1 scans the work directory,
  the user home and Isabelle's temporary prefix (files changed during the test) for the calendar data a check theory
  carries: the account, calendar and agent ids and the event ids, plain and as byte lists, and the titles. It does not
  cover `$TMPDIR` (the JVM's own files), nor what a real Google account's ids would look like, and it was never shown to
  fail on a real leak.
- K8 One worker, one check at a time: a burst pushes later jobs past their deadline (`undecided`).
- K9 W4D D13 is contradicted by code until the owner accepts the amendment (O1); the merge waits for it.
- K10 Texts on the page that name Google can show for practice requests (9.4); the practice line says what a practice
  run does.
- K11 A stop while a starter is still inside `BrokerGate::start` may let that starter write its logs after the work
  directory was removed, just before the process ends; the next start removes them. L13 waits for `ready` before it
  quits for that reason.
- K12 Other sessions running live Isabelle tests with the same user home at the same time share Isabelle's server
  registry; the policy crate serializes server starts only within one process.
- K13 Isabelle's temporary prefix is `/tmp/isabelle-<user>` by default, which another account on the Mac could create
  first; Isabelle creates it without an owner check. The relay cannot change it (Isabelle's settings assign it
  unconditionally); the only override is a line in the user home's Isabelle settings, such as
  `ISABELLE_TMP_PREFIX="$ISABELLE_HOME_USER/tmp"` in `<user_home>/.isabelle/Isabelle2025-2/etc/settings`, which the
  broker could ensure before the gate starts, or `config.rs` could require and `scripts/setup-demo.sh` write. Not done.
- K14 Under the Careful profile a grant until revoked does not serve a create (the theory's `standing_serves`), so
  approving a create with "Until I take it back" sends the request back to `pending_approval`; the page offers that
  duration. The theory's owner answered on 2026-11-16 that the rule is deliberate, as that owner reads the frozen
  matrix (the product owner has not confirmed the reading): under a row that asks, a standing grant serves only reading
  event details (CD-M02) and editing a Fabric-created event in Agent Drafts (CD-M05). For every other row that asks, the
  card should offer only "this request only"; the contract has no field for the durations a card may offer.
- K15 The stop bound's margin is thin: the in-process stop took 0.8 s to 1.3 s against `QUIT_BOUND` of 1.5 s on an idle
  Mac (`BrokerGate::stop` is a session stop, a shutdown and a wait in the policy crate, not changeable here); the built
  broker's quit took 0.9 s to 1.2 s against the relay's 2 s drain. Past the bound the calling thread removes the work
  directory and the server may outlive the broker (K1). The live tests print the time and fail when the stop is not
  acknowledged.
- K16 Live proofs never run in CI (PLAN RD-7): CI compiles `tests/live.rs` on Unix and runs none of it, so a change
  that breaks the prover is found only when someone runs `--live-proofs` by hand. Windows and Linux builds were checked
  only by emulating their `cfg` on a Mac, not on those targets.
- K17 The live test L14 `checks_are_timed` fails the live mode when the mean check exceeds 1.5 s, and its margin is
  thin too: idle means were 1.22 s to 1.29 s, and one run on a loaded Mac (load average 6 to 10) failed at 1,529 ms
  and passed when repeated. Like the stop bound (K15), it needs an otherwise idle Mac.
- K18 Only the rule theory, `ROOTS` and `Fabric_Broker/ROOT` are pinned (2.1). The Isabelle distribution (`bin/isabelle`
  and what it loads) and the Isabelle user home (its `etc/settings` and `etc/components`, which Isabelle sources) are
  trusted as given, and a debug host forwards the three paths with any store (X16). A killed Isabelle also leaves its
  scripts under its temporary prefix (K13); the live test L2 does so on every run.

### 9.2 Requests to other owners

To the owner of the rule theory and the policy crate (the design works without them):
- T1 a health probe, or a cancel, for a running check;
- T2 a typed "gate broken" error;
- T3 `BrokerGate::start` removes stale `Check_*.thy` files itself;
- T4 a `GateConfig` for a packaged layout, and a `ROOTS` that names only `Fabric_Broker`;
- T5 a bounded `stop` and `Drop`;
- T6 identifiers that admit `@` (CD-W05 calendar ids); answered on 2026-11-16: not needed, because a `calendar_id` is
  an opaque id issued by the broker, so CD-W05 issues a broker id for each Google calendar and keeps Google's id in its
  store;
- T7 export the expected theory hash, or make `BrokerGate::start` take one and refuse another theory;
- T8 hash and load the theory from the same bytes (the window between the hash and `session_start`, 2.1);
- T9 a private temporary prefix for Isabelle (K13).

To the owner of the contract:
- E1 a reason and a remedy beside `health.prover` (not configured, failed to start, wrong theory);
- E2 an owner-visible reason on `undecided`;
- E3 a marker for practice accounts, attempts and reports;
- E4 an agent error "rule prover unavailable".

### 9.3 Questions that are the product owner's

Each with the default the code runs on today:
- O1 Amend W4D D13 for a debug-only practice provider (in memory; only with the synthetic agent, the test key, a scratch
  directory and Isabelle)? Default: built as designed; the amendment of W4D D13 and D26 is proposed with this step's
  pull request; the merge waits for the answer.
- O2 Accept the observation filled in by the shell until CD-W05 (X6)? Default: yes.
- O3 The restart policy: 10 s, 60 s, then down until the application restarts (X8)? Default: yes.
- O4 Does the release broker accept the Isabelle variables now, or refuse them until PLAN Q09 (X17)? Default: it
  accepts them; the host does not pass them in a release build.
- O5 Keep the synthetic classifier for synthetic runs without Isabelle? Default: keep.
- O6 Does a debug host forward the Isabelle paths with a Keychain store (X16)? Default: yes; with no provider a fresh
  permit ends `undecided`.
- O7 Confirm PLAN RD-7 (live proofs by hand, `--live-proofs`) and RD-8 for this step; no later record was found.
  Default: live proofs by hand only.

### 9.4 What the page still lacks for a practice run

For the owner of the screens (CD-W08); nothing here is built in this step:
1. The practice line is static: it shows in every build, the product too, and the page cannot tell a practice run
   apart except by the broker's build label in the build line. A line or badge shown only in a practice run needs the
   model or the contract (E3).
2. `applied` never ends in this build (no reconciliation before CD-W05): practice requests A, C and D count as in
   progress for ever, and their one-request permissions stay "In force." Nothing on the page says why.
3. `get_request_details` always has `provider_event` empty (the core's views), so "Event in the calendar" never shows
   the practice event, although the practice calendar made one.
4. For a partial or relative prover setting, the strip says "Fabric Calendar is not installed correctly. ... Installing
   the app again may fix it." That advice is wrong for a test run started from a terminal; the test-key and
   scratch-directory refusals have the same text (older than this step).
5. A bad Isabelle path shows only "The broker stopped." with exit code 78; the reason is on stderr in the terminal.
6. Texts that name Google show for practice requests too: "Who is notified" says "an email from Google" for `all` and
   `external_only`. The practice script notifies nobody, so the scripted run never reaches them.
7. `undecided` shows no reason (E2). The page could at least say that the rule checker was not ready when that was the
   health at the time.
8. The strip says only that the rule checker is not running; it cannot tell "not configured" from "failed to start" or
   "wrong rule theory" until E1.
9. The approval cards of update and delete: the practice script's C and D are the first cards that are not creates.
   Whether every field the card shows has a text for them was not checked here.
10. Page tests for the practice run: every prover health has a text; no state text reachable in a practice run names
    Google; a denial shows its rule and proof; health is read from the snapshot and from health changes.

### 9.5 Not checked

Windows and Linux targets (only this Mac; their `cfg` was emulated for clippy). That another `ROOT` defining
`Fabric_Broker` is refused by Isabelle (2.1). That L1's scan fails on a real leak (K7). Which W04 core tests cover S5.
The edge of p44's bound and the drop of a failing live process (both timing). The page's cards for update and delete
(9.4 item 9). Checked on 2026-11-16 on one Mac: `sh scripts/verify-desktop.sh --live-proofs` passed three runs in a row
and once more after the last fixes; a manual start of the debug broker under an emptied environment with the fixed
`PATH` went from approval through a fresh permit to `applied`, left no check file, and left no server after quit.
