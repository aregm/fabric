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
`PW L1` to `PW L22` are not W4D's package decisions L1 to L7. A file that cites both says so in its module
documentation. Since 2026-11-22 `PW I1` to `PW I12` are the decisions of section 10.

**Who decides.** Every question of section 9.3 is the owner's; the code runs on the default named there. The
Calendar Desktop plan and the contract (`contracts/desktop/v1`) win wherever they differ from this document. The
practice calendar is an exception to W4D's decision D13 ("no prover or provider double in product code"): the
amendment of D13 and D26 in W4D is proposed with this step's pull request and is the owner's to accept (O1).

Date 2026-11-16. Status: as built (branch `claude/cd-prover-wiring`); updated 2026-11-16 by the adapter follow-up
(branch `claude/cd-prover-adapter`) for the gate of pull request 26; updated 2026-11-17 by the kill handle follow-up
(branch `claude/cd-prover-kill-handle`) for the gate's kill handle of pull request 30; updated 2026-11-17 by the broker
part of contract schema version 2 (branch `claude/cd-broker-prover-reasons`): the prover's status says why it is
unavailable (`health.prover_problem`, 2.2), every move to `undecided` stores its cause, which `get_request_details`
gives as `undecided_reason` (2.4, X12), the adapter times its own check limit (2.3), and the shell answers at once every
job it cannot hand over and every job a lost worker held (2.2); the tests u17 to u19, w35 to w40, sh07 to sh11 and p46
to p48. Updated 2026-11-22 by the installed layout (branch `claude/cd-installed-rule-checker`): section 10, which also
amends the older text where it marks the date.

Abbreviations (paths relative to the repository root):

| Short | Means |
|---|---|
| W4D | `docs/plans/calendar-desktop-w04-design.md` (the CD-W04 design; its decisions are D1 to D39 and L1 to L7) |
| PLAN | `docs/plans/calendar-desktop-plan.md` (section 5 holds the requirements R01 to R14 and the quality targets, section 14 the required scenarios) |
| RM | `docs/roadmap.md` |
| CR | `contracts/desktop/v1/README.md` (the contract) |
| GATE | `crates/fabric-broker-policy/src/gate.rs` (`BrokerGate`, `GateConfig`, `Check`, `Fault`, `CHECK_GRACE`) |
| THEORYRS | `crates/fabric-broker-policy/src/theory.rs` (`THEORY_TEXT`, `SESSION_ROOT`, `built_theory_sha256`) |
| MODEL | `crates/fabric-broker-policy/src/model.rs` (`Context`, `Action`, `Grant`, `Rule`) |
| PORTS | `crates/fabric-broker-core/src/ports.rs` (`ProofJob`, `ProofDone`, `Provider`, `DispatchOrder`) |
| THY | `isabelle/Fabric_Broker/Fabric_Broker.thy` (the rule theory), with `isabelle/Fabric_Broker/ROOT` |
| S1 to S7 | the scenarios this step can show (section 7) |

Safety rule of the whole design. The adapter emits a `ProofReport` only when `BrokerGate::check_within` returned
`Ok(Check)` for exactly this job, the check ran on the rule theory this broker was built with, the check names this
job's action and context digests, and every conversion succeeded. A check that returned `Ok` and left the gate broken
keeps its report, and the gate is replaced before any other check. Every other path is `report: None`, which the core
turns into `undecided` with only the result's cause recorded (X12). Only the core turns a result into authority, and it
already refuses an unbound result (W4D D39), a non-Isabelle permit (W4D D13), a fresh permit with no grant, no provider
or a paused agent (W4D D17), a changed fence, and a late result (the job id is gone after the core's 35 s watchdog). The
practice calendar acts only on a `DispatchOrder`, which exists only after a consumed authorization resting on an
accepted Isabelle permit. No test double of the gate ever returns a `Check`: doubles exist only under `#[cfg(test)]` and
can only fail or block.

---------------------------------------------------------------------------------------------------------------------

## 0. Decisions

| # | Topic | Chosen | Why |
|---|---|---|---|
| X1 | Isabelle location | Three variables `FABRIC_BROKER_ISABELLE_HOME`, `FABRIC_BROKER_ISABELLE_USER_HOME`, `FABRIC_BROKER_RULES_DIR`, all or none (the developer mode; amended 2026-11-22: the installed layout passes two other variables instead, 10.1 I1) | One root variable would hard-wire the developer layout (`GateConfig::for_repository`) into the broker's interface, and PLAN forbids an application pointing at `.tooling`. Three paths map one to one to `GateConfig`. They were to serve a packaged runtime (PLAN Q09) unchanged; on 2026-11-22 the packaged runtime got two folders instead, so that the folder names stay in `GateConfig::for_installation` (10.1) |
| X2 | Practice calendar switch | No variable; on exactly when `cfg!(debug_assertions) && synthetic_agent && isabelle.is_some()` (`Config::practice`) | The synthetic agent already carries practice rules P1 and P2 (4.1): debug builds only and the test key (W4D D26); the host passes it only with a scratch directory. A fourth variable adds refusals and no protection |
| X3 | Practice state | In memory only (`Arc<Mutex<_>>`), seeded at broker start, no file | Practice rule P6 forbids plaintext practice files. Cost: practice events do not survive a broker restart (4.2) |
| X4 | Fault injection | In-process `Faults` passed to `PracticeCalendar::new`; `Plan::of` always passes `Faults::default()` | W4D D12: no product hook reachable from outside |
| X5 | Practice identities | The synthetic agent's `synthetic-account` / `synthetic-calendar`; one seeded event Fabric did not make | One seed; the ids are already in the theory's identifier charset |
| X6 | Observation for update and delete | `PracticeCalendar::observation(&agent::Action) -> Option<ObservedEvent>`; the shell sets only `job.facts.event`, only when the core left it `None`, in practice runs only | An update or delete needs the observed event (`event_known` in MODEL); the core sets `facts.event` to `None` until CD-W05 adds the core's observation port and `Fence.event_revision` (W4D 1.2). Fail-closed: `None` unless the target is a live practice event; `context_of` refuses an observation of another calendar; the provider checks the revision again at dispatch. It departs from PORTS ("read in one transaction"): owner question O2 |
| X7 | Adapter shape | `prover/{mod.rs, map.rs}`; a crate-private `Gate` trait and starter; doubles only under `#[cfg(test)]`; one starter thread and one new kill handle (`KillHandle`) per start, the latest held in the shared record (2.2); a broken or refused gate is stopped on the worker within `GATE_STOP_BOUND` (1.2 s); no watcher thread, but an overrun poll the shell calls as a backstop (2.3) | Isabelle stays out of CI (PLAN RD-7), so every failure path is unit-tested without it; the seam is not reachable from outside the crate (W4D D12, D13). A killed handle stays killed (pull request 30), so a retry needs a new one |
| X8 | Restart policy | After the n-th consecutive failure (a failed start or a broken gate) wait `RESTART_WAITS[n-1]` (10 s, 60 s) and start again; after a third failure stay `unavailable` for the run; the count resets on `Ready`; a start begins only after the broken gate's stop returned (it runs on the worker and is bounded); no start begins once a stop was sent | Bounded; a stuck stop is never reused. Owner question O3 |
| X9 | Stop at quit | Bound 1.5 s (`QUIT_BOUND`); before it sends the stop the handle records in the shared record the instant by which the gate's stop must end, `QUIT_BOUND` less `KILL_ALLOWANCE` (0.3 s) from then, that is 1.2 s; the worker gives its gate what is left of it (zero: an immediate kill); jobs queued at the stop are not checked and no check and no start begins; main drains its event channel while it waits; on Unix two kills through the kill handle: the worker kills a gate start in progress before it acknowledges, and main kills the server of a check still running at the stop's instant, on a short-lived thread (2.5); the work directory is removed in every state, by the worker or, without an acknowledgement in time, by the calling thread | The relay kills the broker 2 s after closing its stdin (`QUIT_DRAIN`); the kill's signals must go out before the process ends. No bound of the gate's ends a start or a running check within the quit's bound, so only the handle reaches them in time |
| X10 | Work directory cleanup | `remove_dir_all(<data_dir>/prover)` then a non-recursive create (mode 0700 on Unix) before every gate start, and removal at every stop; once the store is open and its lock held, the shell removes it once in every configuration, with or without a prover; the gate also removes stale `Check_*.thy` at its start (T3); the adapter's cleanup is a superset and stays | One call covers a killed broker's `Check_*.thy` and the server logs; `remove_dir_all` does not follow a symbolic link; the non-recursive create can never make `<data_dir>` itself |
| X11 | Deadline skip | Skip when wall `now_ms() >= job.deadline_ms`, no margin (`PAST_DEADLINE`); otherwise the check's limit is the smaller of the time left before `job.deadline_ms`, from the same clock read, with nothing subtracted, and the adapter's `check_timeout` (`CHECK_TIMEOUT`), passed by the adapter (2.3); the gate's own cap is then a no-op | The core clock is wall time plus a never-negative offset (W4D D34), so the skip never fires before the core's deadline and the limit is never too short; a late result is ignored anyway. A fresh job has 35 s left, so the adapter's cap gives it the plan's 30 s, and the gate's answer or its break comes by 30 s + `CHECK_GRACE` (5 s), the job's deadline. A job that waited behind other checks gets a shorter limit, which ends at its deadline; the gate's answer comes a moment after it (at most `CHECK_GRACE` after), when the core has already ended the job as undecided, so no decided result is lost that would have been used. Subtracting `CHECK_GRACE` from the limit would turn undecided some requests that are decided today (a check takes about 1.2 s). A zero limit would be a `Check` Isabelle never ran (the gate answers it `Ok` undecided without asking) |
| X12 | Reason texts and causes | `no_report(job, Refusal)`, where each `Refusal` is a fixed text with its cause (`ProverCause`, PORTS); no `PolicyError` text reaches a reason or stderr | The text of a no-report result is never stored or shown. Its cause is: the core stores it on the move to `undecided` (`RequestMoved.undecided_cause`, W4D 4 and 5.7), and `get_request_details` maps it to the contract's `undecided_reason` (2.4). An `Undecided` check can carry an Isabelle message into `undecided_reason` of its `ProofRecord`; that is stored only in the encrypted store |
| X13 | Agent script | Create, blocked delete of the foreign event, update, delete; lookup by the create's request id | The block appears without two more approvals; the lookup by request id is exact |
| X14 | Agent API | `SyntheticAgent::practice(..)` and `set_prover_health(..)`; the proposal limit is a field (`PROPOSALS_PER_RUN` = 2 for `new`, `PRACTICE_PROPOSALS` = 4 for `practice`) | W04's callers and its pinned process tests are unchanged |
| X15 | Live tests | One file, `apps/fabric-broker/tests/live.rs`; the tests of 7.3 | Every live test is in one place and runs by name |
| X16 | Host forwarding | A debug host forwards the three Isabelle variables when all three are set and absolute, with any store; a release host ignores them and, since 2026-11-22, passes the installed layout, as a debug host does with none of the three (10.2); the packaged check unsets them in its launches | With a Keychain store no provider is attached, so a fresh permit ends `undecided`. Owner question O6 |
| X17 | Release broker | Reads the three variables in every build; only the host decides not to pass them in release. Since 2026-11-22 it also reads the two variables of the installed layout in every build, and a release host passes those (10.2) | Owner question O4 |
| X18 | The broker's `PATH` | On Unix the relay gives the broker the fixed `PATH=/usr/bin:/bin:/usr/sbin:/sbin`, never the host's; the harness does the same | Isabelle's scripts call system tools by name (`sysctl` in `/usr/sbin`), and with no `PATH` their shell's default lacks `/usr/sbin`, so the session start fails (L11). The broker cannot set it itself: the policy crate's server start sets only `USER_HOME`, and `set_var` is `unsafe`, which the workspace forbids |
| P1 | Theory pin, shared shell, store first, stop | (a) Theory pin: the broker takes THY and the session's `ROOT` from the policy crate (`built_theory_sha256`, `theory::SESSION_ROOT`); before a gate starts, the worker compares the SHA-256 of `<RULES_DIR>/Fabric_Broker/Fabric_Broker.thy` with the built theory's, compares `<RULES_DIR>/Fabric_Broker/ROOT` byte for byte with the built one, and refuses any `<RULES_DIR>/Fabric_Broker/ROOTS`; `<RULES_DIR>/ROOTS` is not read (the gate loads `<RULES_DIR>/Fabric_Broker` alone); it also compares a started gate's `theory_sha256()` with the built hash; a mismatch leaves the prover `unavailable` for the run with one fixed stderr line, and nothing is proved; a `Check` whose `theory_sha256` differs yields no report (2.1 to 2.4). (b) `shell.rs` holds the routing that `main.rs` and `tests/live.rs` both use (2.2, 8). (c) The Isabelle worker starts only after the store is open and its lock is held (2.2). (d) Stopping removes the whole work directory in every state; without an acknowledgement within the bound the calling thread removes it itself (2.5) | Rust derives the outcome from the 18 rule results, which is sound only for the theory the Rust mirror was tested against; the session's `ROOT` and a `ROOTS` beside it decide which theory Isabelle loads, and the gate hashes only the theory. A mismatch is a prover failure, not a configuration error (exit 78) |
| P2 | Smaller safeguards | `Gate`, the starter and `ProverOptions.starter` are crate-private, doubles only under `cfg(test)`; the observation hook returns `Option<ObservedEvent>`; health leaves `ready` while a check is past its deadline; `done_of` checks the check's action and context digests against the job's; Unix-only checks behind `cfg(unix)`, plain `is_file` elsewhere; the proposal limit is a field; the packaged check unsets the three Isabelle variables; practice updates and deletes notify nobody (`send_updates: none`) | Each is placed in the section it changes |
| P3 | The page | Not built in this step. The screens of CD-W08 are the page; this step adds one static line under the build line, `#practice-line` in `ui/index.html` (6). Section 9.4 lists what the page still lacks for a practice run | Another work package owns the screens |
| P4 | Owner questions | O1 to O7 stay open (9.3); the code runs on their defaults. O1: the practice calendar is built (debug builds only, only with the synthetic agent, the test key and a configured Isabelle; state in memory), and the amendment of W4D D13 and D26 is proposed with this step's pull request. O5: the synthetic classifier stays for synthetic runs without Isabelle, so the W04 process tests and CI are unchanged | The owner decides |
| P5 | Requests to other owners | T1 to T9 (theory and policy) and E1 to E4 (contract) are requests (9.2); T1 to T9 were answered by pull request 26 and T10 to T12 by pull request 30, as listed in 9.2; nothing under `crates/fabric-broker-policy`, `isabelle` or the contract changes | One owner per area |
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
`Fence.event_revision` (CD-W05), a packaged Isabelle (PLAN Q09, CD-W11; amended 2026-11-22: an Isabelle installed once
by a script into the application's folder is in, section 10, and how a released application ships it stays CD-W11). No
change to `contracts/`, `isabelle/`, `crates/fabric-broker-policy`, `crates/fabric-desktop-contracts`,
`crates/fabric-broker-core`, `crates/fabric-storage`, `apps/fabric-demo`, `docs/PRD.md`, `docs/architecture.md`,
`docs/calendar.md`.

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

Failure views: the three variables absent (and no Isabelle installed in the scratch folder, 10.2): a synthetic run keeps
the classifier (W04: a card, then "Not done: Fabric
could not prove that it is allowed"); one or two of them, or a relative path: the host starts no broker and the strip
says "Fabric Calendar is not installed correctly."; a path that holds no Isabelle or no rule theory: the broker exits
78 at start, the strip says "The broker stopped." with the exit code, and the broker's line naming the variable and
the path goes to the terminal's stderr; a rule theory or session file that is not the built one: the rule checker is
not running for the run, with one stderr line; Isabelle killed or not answering within a check's limit and grace: the
request ends "Not done: Fabric could not prove that it is allowed", health goes `unavailable`, `starting`, `ready`, and
nothing is dispatched; a check that reaches its limit: with the full 30 s (a fresh job) the request ends undecided with
an undecided report, and health stays `ready`; a job that waited behind other checks has a shorter limit that ends at
its deadline, so the core ends the request undecided there without a report (the late result is ignored), and health
is `unavailable` from the deadline until the gate answers, then `ready` again (2.3).

Since contract schema version 2 the owner channel also says why. An unavailable prover's health names its problem
(2.2): `not_configured` in a run without Isabelle (3.3), `failed_will_retry` while a restart is scheduled, `failed`
once none is, and `wrong_rules` for a rule theory or session file that is not the built one; it is null while a check
runs past its job's deadline, when the cause is not yet known (2.3). The details of an `undecided` request name its
reason (2.4): `prover_unavailable` when the rule checker was starting or not running, `out_of_time` when the check
reached its limit or the proof's deadline, `failed` when the check ran and did not decide, or its report could not be
used, and null when the broker did not record why.

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

Theory pin (P1 a). The texts come from the policy crate (THEORYRS): `prover/mod.rs` re-exports its
`built_theory_sha256()`, the `sha256_hex` of `theory::THEORY_TEXT` (THY by `include_str!`), and compares the session's
`ROOT` with `theory::SESSION_ROOT` (`isabelle/Fabric_Broker/ROOT`, the same way). The gate hashes the raw bytes of the
file it loads, so the two hashes are equal exactly when the bytes are. Cargo rebuilds the broker when the files change.
Before each gate start the worker's `pin` reads `<RULES_DIR>/Fabric_Broker` and nothing else under `RULES_DIR`: the
theory's SHA-256 must be `built_theory_sha256()`; `Fabric_Broker/ROOT` must equal `SESSION_ROOT` byte for byte; and
`Fabric_Broker/ROOTS` must not exist (`symlink_metadata`: a file, a directory or a link is refused). Why the session
files too: Isabelle reads the `ROOT` and the `ROOTS` of every directory it is given, the `dirs` of `session_start`
included (9.5), so a changed `ROOT` could load another theory while the pinned file stays in place, and a `ROOTS` could
add a directory that defines the session. Why not `<RULES_DIR>/ROOTS`: since pull request 26 the gate gives Isabelle
`<RULES_DIR>/Fabric_Broker` alone, so that file is never read; pinning or requiring it would refuse a layout without it
and protect nothing. The pin is enforced in three places: the files under `RULES_DIR` before each gate start (2.2), the
started gate's `theory_sha256()` (2.2), and each `Check`'s `theory_sha256` (2.4); the gate does not enforce the built
hash itself. Since pull request 30 the gate also checks the session files itself (T11): before and after
`session_start` it requires `Fabric_Broker/ROOT` to equal `SESSION_ROOT` and refuses a `Fabric_Broker/ROOTS`, and it
hashes the theory both times and refuses a change between the two. The adapter's `pin` stays: it is what leaves the
prover down for the run with line T; a `ROOT` or a `ROOTS` changed between the `pin` and the gate's own check is a
failed start (`Isabelle`) with a retry (X8), whose `pin` refuses it if the change is still there. Residual: a second
definition of `Fabric_Broker` in a component directory of the Isabelle home or the user home (K18) is left to
Isabelle's refusal of a duplicate session.

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
Started(Result<Box<dyn Gate>, PolicyError>), Stop(Sender<()>) }`; the instant by which a gate's stop must end at quit
is not in the message but in the shared record (`Shared.stop_by`, 2.5), which the handle sets before it sends `Stop`;
while `Down` with a retry and no stop sent it also wakes at the retry time. The shared record also holds `Shared.kill`,
the kill handle of the latest gate start: of the start in progress, or of the gate it gave. It is `None` until the first
start is about to spawn its starter thread, and it is never cleared: the handle of a server that was stopped and reaped
reaches nothing.

| State | Health on entry | `Job(job)` | Transition |
|---|---|---|---|
| `Starting` | `starting`, no problem | `no_report(job, STARTING)` at once | `Started(Ok(g))`: if `g.theory_sha256() != built_theory_sha256()`, `Down` for the run (line T below), then `g` stopped on the worker within `GATE_STOP_BOUND`; else `Ready`, failures = 0. `Started(Err(e))`: failure |
| `Ready(g)` | `ready`, no problem | 2.3 | `GateFate::Restart` or `g.is_broken()` after the check: failure, then `g` stopped on the worker within `GATE_STOP_BOUND` |
| `Down { retry_at }` | `unavailable`, with the problem `failed_will_retry` while `retry_at` is set and no stop was sent, else `failed`; `wrong_rules` when down for the run by line T | `no_report(job, NOT_RUNNING)` at once | When `retry_at` is set and past and no stop was sent: `Starting` |
| any | | | `Stop(ack)`, the channel closed, or the main loop gone (`emit` returns false): 2.5 |

A `Started(Ok(g))` that reaches the worker in another state than `Starting` is stopped on the worker the same way.

Failure rule (X8). failures += 1. If failures <= `restart_waits.len()`: `Down { retry_at: now + restart_waits[failures -
1] }`, stderr `fabric-broker: the rule prover could not start (<category>); it is tried again in <n> s` (or `the rule
prover failed (<category>); ...` for a broken gate). Else `Down { retry_at: None }`, stderr `fabric-broker: the rule
prover failed again (<category>); it is not tried again in this run; quit Fabric Calendar and open it again`.
`<category>` is one of `I/O`, `protocol`, `Isabelle`, `JSON`, `render` (a failed check names its error's), and
`connection` (a check returned `Ok`, but its purge broke the gate); no error text, path or calendar value reaches
stderr.

Entering `Starting` (at the worker's start and when a retry is due, in both cases only while no stop was sent; a stop
sent during steps 1 and 2 ends it before step 3):
1. Remove `work_dir` with `remove_dir_all` (NotFound ignored), then create it with a non-recursive `DirBuilder`, mode
   0700 under `cfg(unix)`. An error is a failure (`I/O`).
2. The pin (2.1): read `<rules_dir>/Fabric_Broker/Fabric_Broker.thy` and `<rules_dir>/Fabric_Broker/ROOT`, and look
   for `<rules_dir>/Fabric_Broker/ROOTS`. A read error, or an error other than "not found" for `ROOTS`, is a failure
   (`I/O`); since 2026-11-22 a theory or `ROOT` that is not found is line T instead (10.3). If the theory's `sha256_hex`
   differs from `built_theory_sha256()`, `ROOT` differs from `SESSION_ROOT`, or a `ROOTS` exists: `Down` for the run and
   line T: `fabric-broker: the rule theory or its session files in "<rules_dir>" are not the ones this broker was built
   with; the rule prover does not run in this run`, where `"<rules_dir>"` is the rules folder the pin read, printed with
   `{:?}` so that the line stays one line (since 2026-11-22, in both modes; before, it named `FABRIC_BROKER_RULES_DIR`,
   which the installed layout does not set). No gate is started.
3. The worker makes a new `KillHandle` for this start (a killed handle stays killed, X7). In one critical section of
   the shared record it returns if a stop was sent, and otherwise stores a clone of the handle in `Shared.kill`; so a
   stop sent before that section starts no gate, and a stop sent after it finds the handle (2.5). A starter thread then
   runs `(options.starter)(&options.gate, &handle)` and sends `Started`. A send after the worker is gone fails, and the
   starter stops its own gate within `stop_bound` (below): `GATE_STOP_BOUND`, or what is left of the quit's instant. On
   Unix the worker killed this start's handle before it acknowledged the stop (2.5): a server the start spawned is
   ended and the start returns an error, and a gate it still returns stops at once, which also sends `shutdown` to a
   server found already running. What is left: if the process ends before the start returns, a found server outlives
   it (K1 case 4). A start that panics is caught on the starter thread and sent as `Started(Err(Io))`, so it is a
   failed start (`I/O`) and goes through the failure rule; a start that never returns stays `Starting` (K22).

Stopping a gate (every "`g` stopped on the worker" above). `stop_gate` calls `g.stop_within(stop_bound(..))` on the
worker, result ignored, then drops `g` (a stopped `BrokerGate`'s `Drop` returns at once). `stop_bound` is
`GATE_STOP_BOUND` (1.2 s: `QUIT_BOUND` less `KILL_ALLOWANCE`), or, once a stop was sent, what is left until
`Shared.stop_by`, at most `GATE_STOP_BOUND` (zero: the gate kills its server at once). Past its bound the gate kills its
server's process groups (Unix; about 0.06 s, at most `KILL_WAIT`, 2 s, more). The failure, or line T, is emitted
first, so the owner sees `unavailable` before the stop and the retry time counts from the failure. Why no thread:
before pull request 26 a gate's stop could block for over a minute, so a dropper thread stopped it off the worker;
`stop_within` is bounded, so the worker now waits for it. Jobs that arrive meanwhile wait in the channel and are
answered `NOT_RUNNING` when it returns (the state is already `Down`), and a restart begins only after the stop returned
(X8). No lock is held across a call of the gate.

Health travels as `ProverStatus { Starting, Ready, Unavailable(Option<ProverProblem>) }` (PORTS): the contract's
`prover` and, while it is unavailable, its `prover_problem`; a problem beside `Starting` or `Ready` cannot be built. It
is emitted only on change, through `ProverEvent::Health`, and the overrun poll (2.3) shares the same record of the last
status sent. Both the worker (`set_health`) and the core (`Core::set_prover_health`) drop a repeat only of the whole
status, so a change of the problem alone is one health notice (W4D 10.3). `fail()` states `failed_will_retry` while a
restart is scheduled and no stop was sent, and `failed` after the last wait or once a stop was sent, since no retry
starts then; a pin that cannot read its files is such a failure too. `other_theory()` and the pin's refusal state
`wrong_rules`, and since 2026-11-22 so does a rule file that is not found (10.3). `shell::input_of` turns
`Event::Health(s)` into `Input::ProverHealth(s)`, and `Shell::turn` also tells the synthetic agent its health
(`set_prover_health(s.health())`).

Pending status and results (the shell's own inputs). `emit` and `route` get only `&Broker`, so they cannot feed an
input, and the main thread must never send into its own bounded channel (it is the only receiver: a full channel
deadlocks). So `Shell` holds `pending_health: Option<ProverStatus>`, `pending_results: Vec<ProofDone>`, and in `handed`
the job id, request and purpose of each job handed to the Isabelle worker whose result has not passed through `turn`
yet:
- at the first sign that the Isabelle worker is lost, once (`prover_lost`): a failed `prove` send (sh09), or
  `IsabelleProver::lost()`, which is true when the worker thread could not be spawned or has ended although no stop was
  sent (a panic, or a main loop that no longer takes its events). The pending status becomes `Unavailable(failed)`, and
  every job in `handed` is answered with `map::answer(.., NOT_RUNNING)`;
- every job the shell cannot hand over is answered at once with `NOT_RUNNING`: after the loss, on a closed classifier
  channel, and in the slots `None` and `Waiting`. The core turns it into `undecided` with the cause `prover_not_running`
  (2.4), or `no_prover` for the classifier, whose kind is not Isabelle (W4D 5.10). Only the Isabelle worker's loss
  changes the status;
- after the loss a status still queued from the worker is fed as `Input::Tick`, and the overrun poll still runs, so that
  its latch is set and the prover's due time clears, but its status is dropped: neither replaces `failed`;
- the overrun poll (2.3) sets the pending status when `poll` returns `Some`.

`Shell::next_due_ms` returns `Some(0)` while a status or a result is pending, so `next_event` returns at once and
main feeds `Input::Tick`. `Shell::turn` first (`notice`) takes out of `handed` the job whose result is its own input,
so a loss noticed in this turn never answers a result in hand with `NOT_RUNNING`, then asks the worker whether it is
lost and runs the overrun poll (`watch_prover`), then takes `pending_health` and, if there was one, calls
`broker.handle(Input::ProverHealth(s), now_ms)`, tells the agent and emits, then feeds each pending result as
`Input::ProofDone` and emits, and only then handles its own input. A job leaves `handed` when its result passes
through `turn` (sh04); a result that names another request or purpose than its job leaves the job there (K21).

A panic of the worker sends no event, so nothing wakes the main loop when the worker ends. `notice` keeps the start
of the last turn (`last_turn_ms`). While a job is out with the Isabelle worker (`handed` is not empty) and its loss
is not known, `Shell::next_due_ms` is no later than `last_turn_ms` plus `LOSS_LOOK_MS` (250 ms), so the main loop
sleeps at most that long. A worker that ended is noticed within that time, and its jobs are answered then (sh10,
sh11). Every due time is a raw wall clock time, as `next_event` reads it. With no job out, a lost worker is noticed
at the next event (K27).

### 2.3 One job in `Ready`

```
if a stop was sent                        -> no_report(job, NOT_RUNNING)    // `Shared.stop_by` is set (2.5)
left = job.deadline_ms - now_ms()         // one wall clock read, saturating; nothing subtracted
if left == 0                              -> no_report(job, PAST_DEADLINE)
limit = min(left, check_timeout)          // the adapter's own cap, `CHECK_TIMEOUT`
(context, action) = context_of(&job)      // Err(JobRefused(t)) -> no_report(job, t); gate kept
if a stop was sent                        -> no_report(job, NOT_RUNNING)    // read again where the mark is set
mark the check running until job.deadline_ms (shared record; the same critical section as the read above)
began = Instant::now(); result = gate.check_within(&context, &action, limit); took = began.elapsed()
lateness = PastDeadline if now_ms() >= job.deadline_ms, else AtLimit if took >= limit, else InTime
broken = gate.is_broken()
clear the mark
(done, fate) = done_of(&job, &context, built_theory_sha256(), result, lateness)
if broken: fate = Restart; emit Done(done)
fate == Restart -> failure (2.2), then the gate's stop (2.2)
else, if the overrun poll had reported `unavailable`, emit Health(Ready)
```

The limit. The adapter passes `limit`, the smaller of `left` and its `check_timeout` (`CHECK_TIMEOUT`, 30 s), so the
gate's own cap at the same value changes nothing, and Isabelle ends the check at that limit (an Undecided check). The
gate's answer, a stuck gate's error included, comes by the limit plus `CHECK_GRACE` (5 s). A fresh job has 35 s left, so
it gets the plan's 30 s and the answer comes by its deadline. A job that waited behind other checks (K8) gets a shorter
limit, which ends at its deadline; the gate's answer then comes a moment after the deadline (at most `CHECK_GRACE`
after), when the core has already ended the job as undecided and ignores the result by job id, so no decided result is
lost that would have been used (X11). A check that returned `Ok` and left the gate broken (its `use_theories` answer was
read in full, then its purge failed) keeps its report: `done_of` still checks its theory hash and both digests; the gate
is restarted with the category `connection`.

Lateness. The adapter times the call with its own monotonic clock and reads no text of the gate for it. When the check
returns an error or an undecided verdict, its cause is `DeadlinePassed` when the wall clock is then at or past the job's
deadline, else `CheckLimitReached` when the call took at least `limit`, else the cause of its row in 2.4; both of the
first two read `out_of_time` in the details. The gate ends a check at or after its limit (it rounds the limit up to
Isabelle's 50 ms look), so a check that ran out its limit always reads `CheckLimitReached` or `DeadlinePassed`, and a
fresh job's check that runs out the 30 s reads `out_of_time` although it answers about 5 s before its deadline. Lateness
never changes the gate's fate, and another theory, an unbound check and a report that does not fit keep their cause.

Overrun. While a check runs, `IsabelleProver::next_due_ms()` returns the job's `deadline_ms`; the shell includes it in
the main loop's due time and calls `IsabelleProver::poll(now_ms)` at the start of every `turn`. `poll` returns
`Some(Unavailable(None))` once when a check is still running at or past its job's deadline, and records it as the last
status sent: the cause is not known while the check still runs, so the problem is null. The shell puts it in
`pending_health` (2.2), which the same `turn` feeds to the broker and the agent before its own input. No thread is
added: the worker is blocked in `check_within`, so the main loop does the timing. The synthetic agent proposes nothing
while health is not `ready`. `Health(Ready)` after an overrun comes only with fate `Keep`, that is after `Ok(..)` or
`Err(Render)` with the gate not broken; after `Err(Io | Protocol | Isabelle | Json)`, or any result after which
`is_broken()` is true (an `Ok` included), the gate restarts and health goes `unavailable` with the failure's problem,
`starting`, `ready` through 2.2. A `Keep` sends `Ready`, and a `Restart` sends the failure's cause; both differ from
`Unavailable(None)`, so each is sent.

The poll is a backstop, for three reasons: a hung gate is reported at the job's deadline, up to `CHECK_GRACE` before the
gate's own error for a job that waited; the gate does not bound its write and removal of the check theory; and the limit
is computed from the wall clock but enforced by the gate on a monotonic clock, so a step of the wall clock can move the
deadline while the gate still has time. With a healthy gate even a fresh job's check that reaches its limit answers
about 5 s before its deadline and changes no health. For a check of a job that waited and that ends at its limit, health
is `unavailable` from the deadline until the gate answers (normally well under a second), then `ready` again; before
this change it lasted longer (until 30 s after the check's start).

| Bound | Value | Ends as |
|---|---|---|
| core watchdog | 35 s from the facts commit (W4D) | `undecided`; a late `ProofDone` is ignored by job id |
| gate check limit | the time left before the job's deadline, at most `CHECK_TIMEOUT` (30 s, PLAN 5), passed by the adapter; Isabelle ends the check at it (T1) | report with outcome Undecided and the cause `CheckLimitReached` (or `DeadlinePassed` at the deadline), gate kept |
| gate's answer | the limit plus `CHECK_GRACE` (5 s) for the whole check (`use_theories` and `purge_theories`), so by the job's deadline for a fresh job | `Err`, the gate broken: no report, restart |
| gate start | 120 s for the server, 600 s for the session | health `starting`; jobs are answered at once |
| gate stop | `GATE_STOP_BOUND` (1.2 s), or what is left of the quit's instant, then the kill (about 0.06 s; at most `KILL_WAIT`, 2 s, more) | 2.2, 2.5 |
| quit | 1.5 s (`QUIT_BOUND`), of which the gate's stop gets at most 1.2 s | 2.5 |

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
`#[non_exhaustive]`) is UNKNOWN_ACTION. Identifiers outside the theory's charset are not pre-checked: `check_within`
returns `Err(Render)`, so no report, and the gate is kept (`Fault::Request`).

`done_of(job, context, expected_theory, result, lateness) -> (ProofDone, GateFate)`; the first matching row decides.
`late(R)` is `R` for `InTime`, `CHECK_LIMIT` for `AtLimit` and `CHECK_LATE` for `PastDeadline` (2.3):

| `result` | `ProofDone` | Fate |
|---|---|---|
| `Err(e)`, `Fault::of(&e) == Request` (`Render`) | `no_report(job, late(RENDER))` | Keep |
| `Err(e)`, `Fault::of(&e) == Gate` (every other kind: `Io`, `Protocol`, `Isabelle`, `Json`) | `no_report(job, late(FAILED))` | Restart |
| `Ok(c)`, `c.theory_sha256 != expected_theory` | `no_report(job, OTHER_THEORY)` | Restart (the next start meets the pin) |
| `Ok(c)`, `c.action_digest != context.digest` or `c.context_digest != fabric_broker_policy::context_digest(context)` | `no_report(job, UNBOUND)` | Keep |
| `Ok(c)`, `proof_report` or `ruling` fails, or an `Id::new` fails | `no_report(job, NOT_FIT)` | Keep |
| `Ok(c)`, otherwise | `report: Some(proof_report(&c, &job.request_id, job.purpose, &format_timestamp(job.started_at_ms)))`, `ruling: ruling(&c)`, `authorizing_grants`: `Id::new` each, `undecided_reason: Some(r)` exactly when the verdict is `Undecided { reason: r }`, `check_theory_text: Some(c.theory_text)`, `cause`: for an Undecided verdict `CheckNotDecided`, `CheckLimitReached` or `DeadlinePassed` by lateness, else `None` | Keep |

The worker restarts the gate whenever `is_broken()` is true after the check, whatever the row; an `Ok` row keeps its
result (2.3).

`no_report(job, refusal)` is `report: None, ruling: None, authorizing_grants: [], undecided_reason: Some(refusal.text),
check_theory_text: None, cause: Some(refusal.cause)`, with `job_id`, `request_id` and `purpose` copied; `answer(job_id,
request_id, purpose, refusal)` is the same for a job the shell knows only by those three (2.2). `proof_report` restores
`sha256:` on the action digest, so the core's binding (W4D D39) compares equal strings; the `UNBOUND` row makes the
binding hold for values the adapter did not copy itself.

Fixed refusals (`pub const` `Refusal { text, cause }` in `map.rs`), each with the cause it states: STARTING "the rule
prover is starting", `Starting`; NOT_RUNNING "the rule prover is not running", `NotRunning`; PAST_DEADLINE "the proof's
deadline passed before its check started" and CHECK_LATE "the check returned after the proof's deadline",
`DeadlinePassed`; CHECK_LIMIT "the check reached its time limit", `CheckLimitReached`; OTHER_CALENDAR "the observation
names another calendar", BAD_ATTENDEE "an observed attendee is not an address", BAD_DIGEST "the action digest is not
sha256", BAD_WINDOW "a grant window is not a time", NO_ACTION "no broker action for this tool call", UNKNOWN_ACTION
"this adapter does not know the action" and RENDER "a fact could not be written for Isabelle", `JobRefused`;
OTHER_THEORY "the check ran on another rule theory", UNBOUND "the check is bound to another action or context" and
NOT_FIT "the proof did not fit the contract", `ReportUnusable`; FAILED "the rule prover failed", `CheckNotDecided`.
Their texts are never stored or shown; their causes are (X12). A check that returns at or after its job's deadline, with
an error or an undecided verdict, states `DeadlinePassed`, and one that took its whole limit before the deadline
`CheckLimitReached`; the adapter decides this from the deadline, the limit and its clocks only (2.3). The owner's view
of an absent prover is the health and, on each request it left `undecided`, its reason.

What the core stores and the details show (W4D 5.7 lists every stored cause, the core's own included). The core converts
each `ProverCause` to its stored cause in one function (`stored_cause`), and `reason_of` maps the stored cause to the
contract's `undecided_reason`. The core ignores the cause of a prover whose kind is not Isabelle: the kind decides
(`no_prover`, W4D 5.10).

| `ProverCause` | Stated by | Stored cause | `undecided_reason` |
|---|---|---|---|
| `Starting` | STARTING | `prover_starting` | `prover_unavailable` |
| `NotRunning` | NOT_RUNNING: down, a stop sent, or a job the shell could not hand over (2.2) | `prover_not_running` | `prover_unavailable` |
| `DeadlinePassed` | PAST_DEADLINE, CHECK_LATE; an Undecided verdict at or past the deadline | `deadline_passed` | `out_of_time` |
| `CheckLimitReached` | CHECK_LIMIT; an Undecided verdict that took its whole limit | `check_limit_reached` | `out_of_time` |
| `CheckNotDecided` | FAILED; an Undecided verdict in time | `check_not_decided` | `failed` |
| `ReportUnusable` | OTHER_THEORY, UNBOUND, NOT_FIT | `report_unusable` | `failed` |
| `JobRefused` | OTHER_CALENDAR, BAD_ATTENDEE, BAD_DIGEST, BAD_WINDOW, NO_ACTION, UNKNOWN_ACTION, RENDER | `job_refused` | `failed` |

### 2.5 Stop

Every exit path of main after the shell exists (end of input, read failure, halt, stdout failure) ends with
`shell.stop(&events)`, after the last frames are written. Configuration and argument errors happen before any thread.

`Shell::stop` drops the provider's channel and its own sender of the event channel, then calls
`IsabelleProver::stop(QUIT_BOUND, drain)`, where `drain` empties the main event channel, so a worker blocked on the
full channel can read its `Stop`. `stop` first sets `Shared.stop_by`, the instant by which a gate's stop must end: now
plus `QUIT_BOUND` less `KILL_ALLOWANCE` (0.3 s), at most `GATE_STOP_BOUND`, that is 1.2 s from then. It is set once
(`get_or_insert`), so the handle's later `Drop` does not move it. Then `stop` sends `Stop(ack)` (the message carries no
instant) and, until the bound, waits for the acknowledgement in 20 ms slices and calls `drain` between them. Without
an Isabelle worker it returns at once (sh03).

Main's kill of a running check (Unix). The first time a slice ends at or after `stop_by` without an acknowledgement, so
up to one slice after it, `stop` looks once at the shared record: if a check is running (`Shared.running`) and a gate
start gave a kill handle (`Shared.kill`), it clones the handle, releases the lock and kills it on a short-lived thread
(`fabric-prover-kill`), inline only if that thread cannot be spawned; the result is ignored. It then keeps waiting and
draining until the bound as before. Why a thread: `KillHandle::kill` waits for its helper process up to
`KILL_HELPER_WAIT` (2 s) when the helper hangs, and the relay kills the broker 2 s after quit, so main keeps its slices,
its drain and its bound. Why at `stop_by` and not at once: a check that ends before `stop_by` still gets an orderly stop
in what is left, and one look at one instant is simpler than two rules. The look is exact: `check_job` reads `stop_by`
in the same critical section that marks a check running and refuses the job with `NOT_RUNNING` when it is set (2.3,
w34), so once `stop_by` has passed, `Shared.running` is set if and only if a check that began before the stop still
runs. With no check running at `stop_by` main kills nothing: the worker is then in its gate's stop, where the gate kills
at that same instant itself (w32), or in its own kill of a start (below). After main's kill the check fails with an
error of the gate's (`Fault::Gate`: an I/O or a protocol error, as the killed server resets or closes the connection)
and the worker handles it as the check of any broken gate (2.2, 2.3): the job's result without a report, one stderr
failure line that names a retry, and health `unavailable`; no retry follows, because a stop was sent, and the drain
discards the two events. The broken gate's `stop_within` then returns at once: with the killed handle it reaps the
server (a second helper run on the dead group, then its waits for the groups and the port). The worker removes the work
directory and acknowledges. On 2026-11-17 on one Mac this stop (L22) was acknowledged 1.31 s to 1.38 s after it began,
in four runs, against `QUIT_BOUND` of 1.5 s (K15). A check on a server found already running is not reached by the kill
(K1 case 4).

Once `stop_by` is set, the worker checks no more jobs and starts no gate: a job still queued in `Ready` ahead of the
`Stop` is answered `NOT_RUNNING` without a check (2.3), and neither the first start nor a retry that is due begins (the
worker waits for the `Stop`). Every stop of a gate on the worker gets `stop_bound`: what is left until `stop_by` (zero:
the gate kills its server at once; the kill's signals go out about 0.06 s after), at most `GATE_STOP_BOUND`. For a check
that ends `r` after the stop was sent, whether it keeps or breaks its gate, the gate's stop gets 1.2 s less `r`, and the
kill's signals go out by about the larger of 1.2 s and `r`, plus 0.06 s, after the send: inside `QUIT_BOUND` for any `r`
up to about 1.4 s. A check still running 1.2 s after the send is not left to end by itself: main kills its server then
(above), so on Unix the acknowledgement no longer depends on `r`.

Worker on `Stop` (P1 d): `Ready` stops its gate on the worker within `stop_bound` (errors ignored); on Unix `Starting`
kills the handle in `Shared.kill` (cloned out first: no lock is held across the kill); `Down` kills nothing; then every
state removes `work_dir`; then it acknowledges and exits. The same happens when the channel is closed or the main loop
is gone (no stop was sent, so the gate gets `GATE_STOP_BOUND`), or the handle is dropped without `stop` (its `Drop` sets
`stop_by` to now plus `GATE_STOP_BOUND` unless it is set, and sends `Stop` without waiting). A starter still inside
`BrokerGate::start_with_kill` keeps its thread. The killed handle refuses a start that has not spawned its server yet,
and ends a server being started, so the start returns an error; a gate already started whose `Started` is in the
channel, or that its starter cannot send any more, has the killed handle, so its stop, by the channel's drop or by the
starter within `stop_bound`, returns at once. The kill does not reach a server `isabelle server` found already running:
its `shutdown` is sent by whoever stops its gate, after a quit during its start only after the acknowledgement (by the
starter thread, or by the worker thread as it drops the channel), so it goes out only if the process is still alive
(K1 case 4). A server this broker spawned was ended before the acknowledgement. On 2026-11-17 on one Mac the built
broker's quit during a gate start (L21) took 55 ms to 57 ms in four runs. The worker's kill stays on the worker: if it
takes longer than the bound (its helper may take up to `KILL_HELPER_WAIT`, 2 s), main returns false at its bound and
removes the work directory itself, as for any unacknowledged stop (below). Outside Unix neither the worker nor main
kills through the handle: it would reach nothing, and its flag alone would only make the gate's later stop skip
`session_stop` and `shutdown` and kill the wrapper alone (K1 case 8).

No acknowledgement within the bound (P1 d): the calling thread removes `work_dir` itself with `remove_dir_all`, which
removes a `Check_*.thy` that a running check still holds (Unix allows removing an open file), and `stop` returns false.
This finishes inside the relay's 2 s drain. A server may still outlive the broker in the cases K1 lists: the next start
with the prover finds a running one by its stable name and uses it (L4). A SIGKILL from the relay leaves files; the next
start removes them once the store is open (X10).

### 2.6 Gate configuration

`gate_config(s, data_dir)` = `GateConfig { isabelle_home, user_home, rules_dir` from `s`, `work_dir:
data_dir.join("prover")`, `server_name: server_name(data_dir)`, `check_timeout: CHECK_TIMEOUT` (30 s, the cap on each
check's own limit, 2.3), `consolidate_delay_seconds: 0.1 }`. `server_name` is `fabric-broker-` and the first 16
characters of `sha256_hex(data_dir.as_os_str().as_encoded_bytes())`. `ProverOptions::new` uses it, `RESTART_WAITS`, and
the real starter (`BrokerGate::start_with_kill(config.clone(), kill)` with the start's kill handle, boxed as `dyn
Gate`). Its `gate` and `restart_waits` fields are
public, so a live test can shorten `check_timeout`; any `GateConfig` still runs the real `BrokerGate` and still meets
the theory pin. Since 2026-11-22 it is `gate_config(isabelle, installation, data_dir)`, whose folders for the installed
layout are those of `GateConfig::for_installation`; `ProverOptions::new` means the developer mode and `Plan::of` uses
`ProverOptions::with_installation` (10.3).

---------------------------------------------------------------------------------------------------------------------

## 3. Configuration

### 3.1 Broker: exit 78, one stderr line naming variable and path, before stdin or threads

| Variable | Refused when |
|---|---|
| `FABRIC_BROKER_ISABELLE_HOME` | not absolute; `<it>/bin/isabelle` is not a file (`metadata`, which follows a symbolic link); under `cfg(unix)` also when no execute bit is set |
| `FABRIC_BROKER_ISABELLE_USER_HOME` | not absolute; not an existing directory (the broker never creates it) |
| `FABRIC_BROKER_RULES_DIR` | not absolute; `Fabric_Broker/ROOT` or `Fabric_Broker/Fabric_Broker.thy` is not a file |
| the three | one or two set: "fabric-broker: the rule prover needs FABRIC_BROKER_ISABELLE_HOME, FABRIC_BROKER_ISABELLE_USER_HOME and FABRIC_BROKER_RULES_DIR together, or none" |
| `FABRIC_BROKER_APP_RESOURCES_DIR`, `FABRIC_BROKER_APP_SUPPORT_DIR` (since 2026-11-22) | one without the other; either one not absolute; either one with any of the three above (10.2) |

On Unix the gate writes one settings line and a private `tmp` directory into the Isabelle user home before each start
(T9).

Read in every build (X17). Nothing heavier than `metadata` runs here, except that since 2026-11-22 the installed layout
reads `etc/ISABELLE_IDENTIFIER` once (10.3); the theory is checked by the worker (2.2), so a
wrong theory is a prover failure, not exit 78 (P1). An unusable distribution or an unwritable user home shows as a
failed start (`unavailable`). The refusal of an unknown `FABRIC_BROKER_*` name and the usage line list six names (W4D
D27 named three); eight since 2026-11-22 (10.2). `Config` has `isabelle: Option<IsabelleSettings>` and `practice()`, and
since 2026-11-22 `installation` (10.3). `EXIT_IO` (74) lives in `config.rs`, so the shell and main share it.

### 3.2 Who sets them

Development and tests: the caller passes the paths explicitly; the broker never discovers `.tooling` and never
searches `PATH`. Live tests read the three variables and default to the repository's layout when all three are absent;
a partial set fails the test (P6). Host, debug: `BrokerStore::from_environment` reads the three names and forwards them
in `variables()` when all three are set and absolute, with any store (X16); one or two of them, or a relative path, is
a host configuration error ("nothing was started"); since 2026-11-22, with none of them set, it passes the installed
layout (10.2). Host, release: ignores them (no prover until PLAN Q09); since 2026-11-22 it passes the installed layout
(10.2). The host's trace line gains ` prover` when it passes them, and ` installed` when it passes the installed layout.
Relay: `ENVIRONMENT_ALLOWLIST` holds the three names beside the three W04 names (and since 2026-11-22 the two of the
installed layout last, `BROKER_APP_RESOURCES_DIR` and `BROKER_APP_SUPPORT_DIR`, 10.2), with the constants
`BROKER_ISABELLE_HOME`, `BROKER_ISABELLE_USER_HOME`, `BROKER_RULES_DIR`, which `relay/src/lib.rs` re-exports and the
host imports, so the names are spelt once on the desktop side; the relay does not depend on the broker crate, so its
tests pin the text. The relay clears the environment, then sets exactly the allowlisted names and, on Unix, the fixed
`PATH=/usr/bin:/bin:/usr/sbin:/sbin` (X18); `PATH` itself stays out of the allowlist, so the host's value never passes.
The harness (`harness/serve.ts`) passes the three names when all three are set in its own environment (never the two of
the installed layout), exits 64 with one or two of them, and gives the broker the same fixed `PATH`. The packaged check
(`verify-desktop.sh --packaged`) unsets the three names in its launches, so values exported in a terminal never turn its
run into a practice run; since 2026-11-22 the host then passes the installed layout with the scratch folder as the
support folder, finds no Isabelle there, and the run stays synthetic (10.5).

### 3.3 Attachments and health (`Plan::of`, 8)

| Configuration | `Attachments.prover` | `provider_present` | Requests |
|---|---|---|---|
| no Isabelle, no synthetic agent | `None`: the core's status is `unavailable` with `not_configured` | false | `undecided` at once, no report (W4D D14), reason `prover_unavailable` |
| no Isabelle, synthetic agent (debug) | `SyntheticClassifier` (O5), status `unavailable` with `not_configured` | false | card, then `undecided` (W04), reason `prover_unavailable` |
| Isabelle, no synthetic agent | `{Isabelle, Starting}` | false | proved; a fresh Permit ends `undecided` (no provider) |
| Isabelle and the synthetic agent (debug): practice | `{Isabelle, Starting}` | true | the whole path |

Since 2026-11-22 (10.3), "Isabelle" in this table also means an installed Isabelle that the broker found, and "no
Isabelle" also an installed layout whose Isabelle is missing, another version or unreadable: that run shows
`not_configured` and is not a practice run.

Contract schema version 2 added the agent error `prover_unavailable`, which the broker may send while the prover is not
ready but need not (CR 8; CR 14 item 40). This step does not send it; whether to refuse tool calls at entry is CD-W06's
decision. Until then each call ends `undecided` within one turn of the main loop, and its details give the reason
`prover_unavailable`. The build label in `hello` is `<version> CD-W04`, with ` synthetic` or
` synthetic practice` after it.

### 3.4 Data directory

| Path | Written by | Removed |
|---|---|---|
| `<data_dir>/prover/`, mode 0700 on Unix | the worker, non-recursively, only after the store is open | whole: once by the shell when the store opens (every configuration), before every gate start, and at every stop (2.5) |
| `prover/Check_NNNNNN.thy` (plain calendar data) | the gate, per check | by the gate's guard after each check; with the directory, by the worker or the stopping thread; and by the gate at each start (T3) |
| `prover/isabelle-server-std{out,err}.log` | the gate at start (created even when the child exits at once) | with the directory: at the next start or stop |
| `<user_home>/.isabelle/...` heaps, registry | Isabelle | never by the broker |
| `<user_home>/.isabelle/<identifier>/etc/settings` | the gate appends its `ISABELLE_TMP_PREFIX` line once (Unix) | never |
| `<user_home>/.isabelle/<identifier>/tmp`, mode 0700 | Isabelle's temporary files (Unix) | never by the broker (K13, K18) |

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
`#[cfg(test)]`, in `prover/mod.rs` and one in `shell.rs`. Those of `prover/mod.rs` return from `check_within` only
`Err`, or block and then return `Err` (one given its start's kill handle blocks until the handle is killed, then
breaks); they record each limit and each stop bound, and report `is_broken` as set by a check that breaks them; their
`theory_sha256()` returns `built_theory_sha256()` unless the test says otherwise. A double starter may return `Ok` with
such a double (w12, w14, w15, w22, w24 to w26, w31, w32, w35, w36, w38, w40; w34 builds a worker already `Ready` with
one): a gate that cannot prove, not a proof. `shell.rs`'s tests add one double, `PanickingGate`, whose check waits until
a deadline the test sets has passed and then panics (sh04, sh09 to sh11). No double ever returns a `Check`. The map
tests u09 to u12, u15,
u16 and u19 build `Check` values by hand (u10 with a Permit verdict) and pass them only to `done_of`, a pure function;
they never reach a worker, a `Shell` or a `Broker`. Unix-only assertions (file modes, execute bits, a shell script
standing in for Isabelle) are `#[cfg(unix)]`; CI compiles and tests the workspace on macOS, Linux and Windows without
Isabelle.

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
| u13 | `only_a_render_error_keeps_the_gate` | the five error kinds' fate, as `Fault::of` | S1 |
| u14 | `no_policy_error_text_reaches_the_reason` | X12 | R13 |
| u15 | `a_check_on_another_theory_is_no_report_and_restarts` | P1 a, on each check | R06 |
| u16 | `a_check_bound_to_another_action_or_context_is_no_report` | UNBOUND | R06 |
| u17 | `no_report_copies_the_job_and_names_the_reason` | `no_report`, and `answer` for a job known by its id: the job's ids, the refusal's text and its cause | R06 |
| u18 | `every_refusal_names_its_cause` | the cause of each of the 16 fixed refusals of 2.4; no two share a text | R06 |
| u19 | `done_of_states_the_cause_and_lateness` | lateness (2.3) in `done_of`: it decides the refusal of an error and the cause of an Undecided verdict, never the fate or the cause of a report that cannot be used; a decided verdict states no cause | S1 |

`apps/fabric-broker/src/prover/mod.rs` (`Gate` doubles; tests whose worker reaches a start read the repository's
`isabelle/`, read only):

| Id | Test | Shows | Plan |
|---|---|---|---|
| w01 | `health_is_starting_until_the_gate_starts` | 2.2: `starting`, then `ready`, neither with a problem | R06 |
| w02 | `the_nth_failure_waits_the_nth_wait_and_the_third_leaves_the_prover_unavailable` | X8; the statuses `failed_will_retry`, `starting`, `failed_will_retry`, `starting`, `failed` | S1 |
| w03 | `jobs_while_not_ready_are_answered_at_once_with_no_report` | STARTING, NOT_RUNNING | R06 |
| w04 | `a_job_past_its_deadline_is_not_checked` | X11 | S1 |
| w05 | `a_broken_gate_is_stopped_on_the_worker_and_restarted` | 2.2, the stop on the worker within `GATE_STOP_BOUND` | S1 |
| w06 | `a_render_error_keeps_the_gate` | 2.4 | S1 |
| w07 | `stop_returns_within_its_bound_while_starting_or_blocked` | X9 | - |
| w08 | `stop_removes_the_work_dir_in_every_state` | P1 d in `Starting`, `Ready`, `Down` | R13 |
| w09 | `the_server_name_is_stable_per_data_dir_and_valid` | 2.6 | - |
| w10 | `a_real_gate_with_no_isabelle_is_unavailable` | the real starter on an empty Isabelle home fails at once on every platform; with no wait left the problem is `failed` | R06 |
| w11 | `a_rules_dir_with_another_theory_leaves_the_prover_unavailable_and_starts_nothing` | P1 a on the file (`THEORY_TEXT` changed by one byte, `SESSION_ROOT`); a counting starter is never called; the problem is `wrong_rules` | R06 |
| w12 | `a_gate_reporting_another_theory_is_dropped_before_any_check` | P1 a on the gate; the gate is stopped; the problem is `wrong_rules` | R06 |
| w13 | `an_unacknowledged_stop_removes_the_check_file_from_the_calling_thread` | P1 d without an acknowledgement; the gate that then breaks is stopped with a zero bound (2.5, X9) | R13 |
| w14 | `a_restart_waits_until_the_broken_gate_is_dropped` | X8, a stop that holds the worker | S1 |
| w15 | `health_leaves_ready_while_a_check_runs_past_its_deadline` | the overrun poll of 2.3, a backstop: `unavailable` with no problem, then `ready` | S1 |
| w16 | `the_work_dir_is_emptied_and_made_private_before_each_start` | X10, mode 0700 on Unix | R13 |
| w17 | `the_built_theory_is_the_repository_theory` | the repository's rules meet the pin; `THEORY_TEXT` is the repository's theory | R06 |
| w18 | `session_files_that_point_elsewhere_leave_the_prover_unavailable_and_start_nothing` | P1 a on `Fabric_Broker/ROOT` and a `Fabric_Broker/ROOTS` beside it | R06 |
| w19 | `a_dropped_handle_stops_the_worker` | 2.5 | - |
| w20 | `a_gone_main_loop_stops_the_worker` | 2.5 | - |
| w21 | `a_rules_dir_without_roots_reaches_the_starter` | 2.1, T4: without `<rules_dir>/ROOTS`, or with one naming another directory, the pin passes and the starter is called | R06 |
| w22 | `a_gate_broken_by_a_check_is_restarted_even_when_its_error_would_keep_it` | 2.3: `is_broken` is read after the check; a gate broken by a check is restarted although its error (`Render`) would keep it | S1 |
| w23 | `each_check_gets_the_time_left_before_its_deadline` | 2.3, X11: a job due in 20 s gets one limit in (19 s, 20 s] | S1 |
| w24 | `a_quit_gives_the_gate_what_is_left_of_the_quit_bound` | 2.5, X9: an idle gate's stop gets at most `GATE_STOP_BOUND`; a gate in a check that ends at least 800 ms after the stop gets at most `GATE_STOP_BOUND` less 800 ms; a stop of `KILL_ALLOWANCE` gives the gate's stop nothing | - |
| w25 | `queued_jobs_are_not_checked_once_a_stop_is_sent` | 2.5, X9: two jobs queued behind a running check at the stop get `NOT_RUNNING`; one check; the same when the stop comes from a dropped handle | - |
| w26 | `a_gate_started_after_the_stop_is_stopped_by_its_starter` | 2.2 step 3, K1: once the worker is gone, the starter stops its gate within `GATE_STOP_BOUND`, or with what is left of a stop's instant (zero) when one was sent | - |
| w27 | `the_bounds_fit_the_core_and_the_relay` | X9, X11: `QUIT_BOUND` is under the relay's `QUIT_DRAIN`, `KILL_ALLOWANCE` under `QUIT_BOUND`, and `CHECK_TIMEOUT` plus `CHECK_GRACE` at most the core's `PROOF_WATCHDOG_MS` | - |
| w28 | `no_start_begins_once_a_stop_was_sent` | 2.5, X8: a retry due when the stop is sent starts no gate; a sent stop makes a due retry neither waited for nor due | - |
| w29 | `a_stop_sent_before_the_first_start_starts_nothing` | 2.5, X8: a worker whose stop was sent before it ran acknowledges it, never calls the starter, spawns no starter thread and makes no work directory | - |
| w30 | `a_stop_while_starting_kills_the_start` | 2.5, K1 case 2: a stop while the starter is inside its start is acknowledged, and by then the kill handle that start was given is killed on Unix and not killed elsewhere | - |
| w31 | `a_check_still_running_at_the_stop_instant_is_killed` (`cfg(unix)`) | 2.5, K1 case 3: a check still running at the stop's instant has its handle killed; the stop is acknowledged at least `GATE_STOP_BOUND` and less than `GATE_STOP_BOUND` plus 700 ms after it began, and the broken gate's stop runs once; with a restart wait left, the last status is `failed` and no `starting` follows (2.2) | - |
| w32 | `a_stop_kills_no_server_whose_check_is_not_running` | 2.5: an idle gate whose stop returns 200 ms after its bound: the stop is acknowledged, not before `GATE_STOP_BOUND`, and the handle is not killed | - |
| w33 | `each_start_gets_its_own_kill_handle` | 2.2 step 3, X7: after a start whose handle was killed, the retry's starter is given a handle that is not killed | - |
| w34 | `no_check_starts_once_a_stop_was_sent` | 2.3, 2.5: `check_job` with `stop_by` set refuses the job with `NOT_RUNNING`, does not call the gate's check and marks no check running | - |
| w35 | `a_check_that_returns_after_its_deadline_is_late` | 2.3: a check held until the clock is past its job's deadline, then failing, gives CHECK_LATE (`DeadlinePassed`); the same failure in time gives FAILED (`CheckNotDecided`) | S1 |
| w36 | `a_check_that_takes_its_whole_limit_reached_it` | 2.3: the gate is given the smaller of the time left and `check_timeout`; a check that fails after its whole limit, before the deadline, gives CHECK_LIMIT (`CheckLimitReached`) | S1 |
| w37 | `a_pin_that_cannot_read_its_files_will_retry` | 2.2: a `Fabric_Broker.thy` that cannot be read is a failed start, `failed_will_retry` with a wait left, not `wrong_rules`; since 2026-11-22 a directory stands in its place, because a missing one is `wrong_rules` (10.3) | R06 |
| w38 | `overrun_then_a_broken_gate_says_why` | 2.3: the poll's `unavailable` with no problem, then `failed_will_retry` when the check breaks the gate, although the health is the same | S1 |
| w39 | `a_start_that_panics_is_a_failed_start` | 2.2 step 3: a starter that panics is a failed start (`failed` with no wait); the worker is not lost and answers jobs `NOT_RUNNING` | S1 |
| w40 | `a_worker_is_lost_only_when_it_ended_without_a_stop` | 2.2: `lost()` is true for a worker that ended because the main loop took no more events and for one never spawned, false while it runs and after a stop | - |

`apps/fabric-broker/src/config.rs`:

| Id | Test | Shows | Plan |
|---|---|---|---|
| c40 | `isabelle_variables_are_all_or_none` | 3.1, the three together; a rules directory without `ROOTS` is accepted | DEF-2 |
| c41 | `each_isabelle_path_is_checked_before_anything_starts` | each row of 3.1 (the execute-bit row under `cfg(unix)`) | DEF-2 |
| c42 | `practice_is_debug_synthetic_and_isabelle_only` | X2, debug builds | - |
| c42r | `practice_is_never_on_in_release` | X2 in a release build on macOS (`verify-desktop.sh` runs the release library tests there) | - |
| c43 | `the_refusal_lists_eight_names` (`the_refusal_lists_six_names` until 2026-11-22) | 3.1, the unknown-name refusal | - |

`apps/fabric-broker/src/shell.rs`:

| Id | Test | Shows | Plan |
|---|---|---|---|
| sh01 | `the_plan_of_each_configuration_is_the_table_of_plan_of` | the table of 3.3, which is `Plan::of`'s (debug rows under `cfg(debug_assertions)`); the classifier's status is `unavailable` with `not_configured`, Isabelle's identity `starting` | - |
| sh02 | `the_isabelle_worker_does_not_start_while_the_store_is_in_use` | P1 c: with the store held, the broker runs in the unavailable mode and no `<data_dir>/prover` exists | - |
| sh03 | `stop_without_an_isabelle_worker_returns_at_once` | 2.5 | - |
| sh04 | `a_lost_worker_feeds_unavailable_once` | pending status and results of 2.2: a worker lost inside a check past its deadline, noticed through `lost()` at the start of a turn, sets `failed` once; the job whose result is that turn's input leaves `handed` first, and the other handed jobs are answered `NOT_RUNNING`; the deadline is set once the worker is ready; one turn feeds them, answers its owner line and leaves no due time in the past; neither the poll nor a `ready` queued before the loss replaces `failed`; later jobs are answered at once; a second loss sets nothing | S1 |
| sh05 | `a_left_check_theory_goes_once_the_store_is_open_in_every_configuration` | X10, and kept while another holder has the store | R13 |
| sh06 | `a_request_while_the_isabelle_prover_is_down_ends_undecided` | a down prover: undecided, no proof, no authorization, no attempt; the problem `failed` (no wait) and the reason `prover_unavailable` | R06 |
| sh07 | `a_request_while_the_isabelle_worker_is_lost_ends_undecided` | 2.2 end to end: the synthetic agent's request on a lost worker ends `undecided` within a few turns, its redacted details read `prover_unavailable`, the health `unavailable` with `failed` | S1 |
| sh08 | `a_job_no_prover_can_take_is_answered_not_running` | 2.2: a closed classifier channel and the slots `None` and `Waiting` answer a job at once with `NOT_RUNNING` and leave the status as it is | R06 |
| sh09 | `a_failed_prove_send_is_a_sign_of_loss` | 2.2: with the worker ended and its loss not yet noticed, a failed `prove` send sets `failed` once and answers the jobs handed earlier, then its own job, with `NOT_RUNNING` | S1 |
| sh10 | `a_job_out_bounds_the_next_due_time` | 2.2: with a job out and the loss not known, `next_due_ms` is the minimum of the other due times and the last turn's time plus `LOSS_LOOK_MS`; with no job out, or once the loss is known, it is the minimum of the other due times | S1 |
| sh11 | `a_worker_lost_with_a_job_out_is_noticed_within_a_look` | 2.2 end to end: a worker that panics inside a check whose deadline is 30 s ahead, with no other event; the loop of `main.rs` (bounded only by the test's 5 s) answers the job `NOT_RUNNING` (cause `NotRunning`) and sets `failed` in less than 5 s | S1 |

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
(`relay/src/config.rs`, six names, eight since 2026-11-22; Isabelle's own `ISABELLE_HOME` and `USER_HOME` are never
allowlisted), and `environment_holds_only_allowlisted_names` (`relay/tests/relay.rs`, which on Unix now requires exactly
the fixed `PATH` and still refuses every other name). No page test (P3). The tests added on 2026-11-22 for the installed
layout are in 10.7.

### 7.2 Process, CI (`apps/fabric-broker/tests/process.rs`, `tests/relay_round_trip.rs`)

| Id | Test | Shows | Plan |
|---|---|---|---|
| p40 | rows of `configuration_errors_exit_78_before_any_frame` | each refusal of 3.1 before any frame (the execute-bit row under `cfg(unix)`) | DEF-2 |
| p41 | `an_isabelle_that_cannot_start_leaves_the_prover_unavailable_and_proposes_nothing` (`cfg(unix)`) | with `bin/isabelle` a script exiting 1, the repository's `isabelle/` and a temporary user home, the broker reports `prover: unavailable` (snapshot or notice), the first such snapshot with `prover_problem: failed_will_retry`, the practice agent proposes nothing, and after quit no `<data_dir>/prover` is left | S1 |
| p42 | `the_synthetic_agent_waits_for_subscribe_asks_and_an_approval_ends_undecided` (W4D p07, unchanged) | a classifier run without Isabelle is unchanged | - |
| p43 | `the_relay_passes_the_three_isabelle_names` (`tests/relay_round_trip.rs`) | through the relay, the broker reads all three (its build label says `synthetic practice`) | - |
| p44 | `a_second_broker_on_a_locked_directory_leaves_the_first_brokers_prover_alone` (`cfg(unix)`) | P1 c across processes: only after the first broker reports `prover: unavailable` does the test put a marker in `<data_dir>/prover`; a second broker answers `store_unavailable`; the marker is still there. It concludes only within the first restart wait less a second and fails with "too slow to conclude" otherwise, because the first broker's next start would remove the marker | - |
| p45 | `a_start_without_isabelle_removes_a_check_theory_a_killed_broker_left` | X10 across processes | R13 |
| p46 | `the_first_run_snapshot` (W4D, changed) | 3.3: a run with no prover reads `unavailable` with `not_configured`, the health of the contract's fixture `accept-owner-notice-health-prover-not-configured.json` | - |
| p47 | `an_approval_of_a_synthetic_request_creates_a_grant_and_ends_undecided` (W4D, extended) | 3.3: the classifier's permit is not acted on; the redacted details of the `undecided` request read `prover_unavailable`, with `private` null and no proof id | R06 |
| p48 | `decode_line`, on every owner line of every test in the file | CR 7: a health names a problem only while the prover is unavailable, and request details name a reason only while the request is `undecided` | - |

### 7.3 Live (real Isabelle; never in CI; P6)

`apps/fabric-broker/tests/live.rs`, `#![cfg(all(unix, debug_assertions))]`. Each test is `#[ignore = "live: needs
Isabelle; run sh scripts/verify-desktop.sh --live-proofs"]`, takes one static `LIVE` lock, uses the test key and a
temporary data directory of its own (hence a server name of its own), and never reaches the Keychain. Isabelle paths:
the three variables when all are set, else the repository's layout; a partial set, or no `bin/isabelle`, fails the
test with "run sh scripts/setup-demo.sh by hand". The in-process tests build `Broker::start(shell::opener(..), ..,
plan.attachments(), ..)` and a `Shell` from a `Plan` holding `ProverChoice::Isabelle(ProverOptions::new(..))`, a
`PracticeCalendar` they keep a clone of and, where the script runs, `SyntheticAgent::practice`; they drive owner frames
and worker events through `shell::next_event`, `shell::input_of` and `Shell::turn`, the same code as `main.rs` (P1 b).
The process tests among them run the built broker. Every owner
line a live test reads is checked against the two rules of p48. Every test is a top-level function, so its name is its
full test path and `--exact <name>` matches it.

How to run them: all of them, after the first test alone and the rule theory's corpus, with `sh
scripts/verify-desktop.sh --live-proofs` (7.4); one of them with `sh scripts/run.sh cargo test --locked -p fabric-broker
--test live -- --ignored --exact <name> --nocapture`. Run them on an otherwise idle Mac: the in-process stop must be
acknowledged within `QUIT_BOUND`; the gate's stop inside it is bounded at 1.2 s and then killed (K15). On 2026-11-16 the
whole mode took about seven minutes on one Mac: the corpus about 210 s, the live binary, with L20, about 180 s. On
2026-11-17, with L21 and L22, the corpus took 213 s and the live binary 190 s.

| Id | Test | Shows | Plan |
|---|---|---|---|
| L1 | `the_practice_script_is_asked_approved_permitted_applied_and_blocked` | the practice script: three `applied`, one `blocked` on `fabric_created`; `get_proof` reports with Isabelle `Isabelle2025-2`, the theory hash `sha256:` and `built_theory_sha256()`, 18 verdicts; the practice event created, at revision 2, deleted; the foreign event untouched; after quit no file under `<data_dir>/prover`; no calendar text (titles; account, calendar, agent and event ids, plain and as the check theory's byte lists) in a file changed during the test under the work directory, the user home or Isabelle's temporary prefix | S7, DEF-7, R06, R13 |
| L2 | `killing_isabelle_mid_check_is_undecided_then_ready_again` | the server killed while A's fresh-permit check theory is on disk: A ends `undecided` with no dispatch and no authorization, no check theory is left, A's reason `failed`, health `unavailable` with `failed_will_retry`, then `starting` and `ready` with no problem, the next request is decided | S1 |
| L3 | `a_check_over_budget_is_undecided` | a 1 ms check limit: Isabelle ends the check at its first look; outcome Undecided within 500 ms (the test prints the time; a full check takes 1.0 s to 1.8 s), no failing rule, no grant, no authorization, no dispatch, the reason `out_of_time` (the check took its whole limit, 2.3), the gate kept | S1 |
| L4 | `a_killed_broker_restarts_with_an_empty_work_dir_and_one_server` | the built broker killed with SIGKILL mid-run; the next start on the same directory empties `prover/`, finds the orphan server by its name or starts one, never a second, reports ready, and leaves nothing at quit; it is the test of K1 case 4 (a reused server): it prints the second broker's quit time and a drop guard declared before both brokers kills any server still listed when the test ends, so a failure leaves nothing running | R12, K1 |
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
| L16 | `killing_isabelle_ends_the_open_request_undecided_then_ready_again` | the server killed between checks while a request is open: `undecided` with the reason `failed`, nothing dispatched, health `unavailable` with `failed_will_retry`, then `starting` and `ready` with no problem, the next request decided | S1 |
| L17 | `revoke_before_a_live_retry_blocks_the_retry` | `Faults { transient: 1 }`, revoke while the request waits for its retry: the retry's fresh proof is blocked on `relied_grant_live`; one dispatch, one attempt, one permit | S3, R08 |
| L18 | `the_theory_pin_refuses_a_changed_rule_file` | P1 a: a theory one byte different is refused before any gate starts; no server, no proposal, nothing proved; the problem `wrong_rules` | R06 |
| L19 | `the_theory_pin_refuses_session_files_that_load_another_theory` | P1 a: the session's `ROOT` points the session at a copied, changed theory, or a `ROOTS` beside it adds a directory that defines the session with it; each is refused before any gate starts, with the problem `wrong_rules` | R06 |
| L20 | `quit_kills_a_server_that_does_not_answer` | the built broker, its server's processes stopped with SIGSTOP once it is ready; end of input: the broker exits within the relay's drain, no server is left 500 ms later, and `prover/` is gone; it prints the quit time; a drop guard made right after the SIGSTOP kills the stopped processes, so a failure leaves nothing stopped | X9, K1 |
| L21 | `quit_during_a_gate_start_leaves_no_server` | the built broker; once a process names its server and a snapshot still reports the prover `starting` with no problem, end of input: the broker exits within the relay's drain, no server is left 500 ms later, and `prover/` is gone; it prints the quit time; a prover already `ready` at the snapshot fails the test, since no quit during a start was tried; a drop guard declared before the broker kills any server still listed when the test ends | X9, K1 |
| L22 | `quit_during_a_check_kills_its_server` | in process, as L2: once ready and A's classification check has ended, the server's processes are stopped with SIGSTOP and A is approved; a check theory in the work directory, still there just before the stop, shows the fresh permit's check running; the stop is acknowledged within `QUIT_BOUND` and not before 1.2 s (`GATE_STOP_BOUND`), no server is left 500 ms later, and the work directory is gone; it prints the stop time; a drop guard made right after the SIGSTOP kills the stopped processes | X9, K1 |

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

Since 2026-11-22 `verify-desktop.sh` has a fourth mode, `--installed-proofs`, run by hand and never in CI (10.7);
the sources mode begins with the check of the archive pin, and `--packaged` checks the bundle's rule files (10.5).

---------------------------------------------------------------------------------------------------------------------

## 8. Modules and interfaces

The rustdoc is the reference: `sh scripts/run.sh cargo doc -p fabric-broker --no-deps --document-private-items` (the
crate-private gate seam needs `--document-private-items`), and `-p fabric-desktop-relay` for the relay. In short:

- `config` (`apps/fabric-broker/src/config.rs`): `config_from` reads the six variables (eight since 2026-11-22:
  `APP_RESOURCES_DIR`, `APP_SUPPORT_DIR`, with `Installation` and `Config.installation`, 10.3); `ISABELLE_HOME`,
  `ISABELLE_USER_HOME`, `RULES_DIR`; `IsabelleSettings { isabelle_home, user_home, rules_dir }`; `Config::isabelle` and
  `Config::practice()`; the exit codes `EXIT_IO` (74), `EXIT_HALT` (75), `EXIT_CONFIG` (78).
- `prover` (`prover/mod.rs`): `CHECK_TIMEOUT`, `RESTART_WAITS`, `QUIT_BOUND`; `built_theory_sha256()` (the policy
  crate's, re-exported); `gate_config`,
  `server_name`; `ProverOptions { gate, restart_waits }` and `ProverOptions::new`; `ProverEvent { Done,
  Health(ProverStatus) }` and `Emit`; `IsabelleProver` with `identity`, `spawn`, `prove`, `lost`, `next_due_ms`, `poll`
  and `stop`. Crate-private: `Gate`
  (`check_within`, `is_broken`, `stop_within`, `theory_sha256`), `Starter` (given the `GateConfig` and the start's
  `KillHandle`), `remove_work_dir`.
- `prover::map` (`prover/map.rs`, pure): `Refusal { text, cause }`, `JobRefused(Refusal)`, the fixed refusals (2.4),
  `Lateness { InTime, AtLimit, PastDeadline }`, `context_of`, `GateFate`, `done_of`, `no_report`, `answer`.
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
  re-exported from the crate root; `ENVIRONMENT_ALLOWLIST` with six names (and `SystemRoot` on Windows; eight since
  2026-11-22, with `BROKER_APP_RESOURCES_DIR` and `BROKER_APP_SUPPORT_DIR`, 10.2); `BrokerConfig::command` adds the
  fixed `PATH` on Unix.
- Host (`apps/fabric-desktop/src-tauri/src/host.rs`): `BrokerProver { isabelle_home, isabelle_user_home, rules_dir }`
  and `BrokerStore::prover`, filled by `BrokerStore::from_environment` in a debug build; since 2026-11-22 `BrokerProver`
  is `Developer { .. }` or `Installed { resources_dir, support_dir }`, filled in every build (10.2).

---------------------------------------------------------------------------------------------------------------------

## 9. Risks, requests, questions, not checked

### 9.1 Risks

- K1 An Isabelle server can outlive its broker in the open cases below, and only these (`s` is when `Shell::stop`
  sends the stop; the process ends at about `s + 1.5 s` when the worker does not acknowledge sooner). Cases 2, 3 and 5
  are closed; they keep their numbers, because other text cites them.
  1. The broker is killed (the relay's SIGKILL after its 2 s drain, a crash, a force quit).
  2. Closed on Unix by the kill handle follow-up (L21, w30): a quit while a starter is inside
     `BrokerGate::start_with_kill` (about 5 s with a built heap, minutes while the heap builds). The worker kills the
     start's handle before it acknowledges (2.5): a start that has not spawned its server is refused, a server being
     started is ended and the start returns an error, and a gate whose start returns in the milliseconds between the
     stop and the worker's exit stops at once when it is dropped with the channel or by its starter. What is left of
     it, a start that found a server already running, is case 4.
  3. Closed on Unix by the kill handle follow-up (L22, w31, w34): a quit while the worker is inside a check that ends
     more than about 1.4 s after `s`. Main kills the check's server through the kill handle at `stop_by`, 1.2 s after
     `s` (2.5); the check fails, the broken gate's stop returns at once and the worker acknowledges.
  4. A server `isabelle server -n` found already running under the name (an orphan of case 1, or of a case that was
     open before, reused): neither the gate's kill at its bound nor the kill handle reaches it. It stops if
     `session_stop` and then `shutdown` are acknowledged within the bound. Since pull request 30 (T12), when that
     exchange did not get `shutdown` acknowledged, `stop_within` still sends `shutdown` on a fresh connection, up to
     `FOUND_SHUTDOWN_GRACE` (0.3 s) past its bound and without confirmation, also after a kill through the handle. Its
     `shutdown` is sent by whoever stops its gate: by the worker in `Ready`; after a quit during its start only after
     the acknowledgement, by the starter thread or by the worker thread as it drops the channel, so it goes out only if
     the process is still alive. Left: a quit during a check on a found server, which no kill ends, when the check
     ends after the process; a quit during a start that found a server, when the process ends before the start
     returns; and a `shutdown` that is sent but not confirmed. The server and its session's ML process then stay, and
     the next run reuses it and has the same chance again. L4 is its test.
  5. Closed by pull request 30: the kill under way when the process ends. The kill now runs in a helper process
     (`/bin/sh` in a process group of its own) that stops the server's group, lists the groups below it with `ps`, and
     kills them and the server's group; once started it finishes without its caller, so the server is not left
     stopped. Pull request 30 says the case itself was not provoked in a test.
  6. A panic of the main thread (the handle's `Drop` sends the stop without waiting).
  7. The kill fails: `/bin/sh`, `ps`, `awk` or `/bin/kill` cannot run, or the helper does not finish within
     `KILL_HELPER_WAIT` (2 s). The server's own group is then killed directly, and a `poly` process below it can be
     left.
  8. Outside Unix: no process group; the kill reaches only the wrapper (the broker runs Isabelle only on macOS). The
     adapter makes no kill through the handle there, which would only set its flag.
  9. A timing case beside case 5: the process ends before a kill has started: the worker's own kill at its gate's bound,
     when the worker is more than `KILL_ALLOWANCE` (0.3 s) late for the stop's instant (an idle gate, or a check that
     ended shortly before `stop_by`, which main does not kill); or main's kill of a check still running at that instant
     (2.5), when its kill thread has not started its helper process by then.

  The next start with the prover configured and the same data directory finds a running orphan by its stable name and
  uses it (L4 showed both server processes surviving a kill and being reused); a start without the prover configured
  cannot stop it, having no `isabelle`; a check theory left behind is still removed by every start once the store is
  open. Closed by the adapter follow-up: an idle or ready gate at an orderly quit (killed at the bound); a quit during a
  check that ends within about 1.4 s, whether the check keeps or breaks its gate; jobs queued at the quit; a start that
  would begin after the quit; a gate being stopped shortly before the quit; a gate whose start returns after the worker
  is gone while the process lives on. Closed by the kill handle follow-up, on Unix, with the kill handle of pull request
  30 (T10): a quit during a start (case 2) and a quit during a check still running at the stop's instant (case 3); by
  pull request 30 itself: a kill under way when the process ends (case 5).
- K2 A hung check holds the worker up to its limit plus `CHECK_GRACE`: until its job's deadline for a fresh job, up to
  `CHECK_GRACE` after it for a job that waited (X11). The gate is then broken and restarted; health turns `unavailable`
  at the job's deadline (the overrun poll, 2.3), and the request ends `undecided` by the core's watchdog.
- K3 The broker's `PATH`: Isabelle does not start under an empty environment (L11 failed before X18). The relay now
  gives the broker a fixed `PATH=/usr/bin:/bin:/usr/sbin:/sbin` on Unix, so "the broker's environment holds only its
  own variables" is no longer true on Unix; the value is fixed, never the host's, and the relay's tests pin it. The live
  test L11 still says "empty environment" in its name; the second step of `--live-proofs` says "the relay's
  environment (its variables and a fixed PATH)".
- K4 The first start may build Isabelle's session heap for minutes (health `starting`). Since 2026-11-22 (10): in the
  installed layout the first start after an installation, and after the application moved to another folder, builds it
  in `<support>/isabelle-user-home`; the policy crate's live test says about half a minute on one Mac, and no
  measurement is recorded.
- K5 The observation of an update or delete is filled in by the shell, not in the core's facts transaction (X6); debug
  builds only.
- K6 `applied` never reconciles before CD-W05: `busy` after about eight runs per scratch directory, and a restart shows
  done for events the practice calendar lost (the README says so).
- K7 Isabelle's server logs are not read; they go with `prover/` at every start and stop. L1 scans the work directory,
  the user home and Isabelle's temporary prefix (files changed during the test) for the calendar data a check theory
  carries: the account, calendar and agent ids and the event ids, plain and as byte lists, and the titles. It does not
  cover `$TMPDIR` (the JVM's own files), nor what a real Google account's ids would look like, and it was never shown to
  fail on a real leak.
- K8 One worker, one check at a time: a burst pushes later jobs past their deadline (`undecided`). A job behind others
  gets a shorter limit, which ends at its deadline; its answer can come up to `CHECK_GRACE` after the deadline, when the
  core has ended the job, so no decided result is lost that would have been used (X11); a job with no time left is not
  checked.
- K9 W4D D13 is contradicted by code until the owner accepts the amendment (O1); the merge waits for it.
- K10 Texts on the page that name Google can show for practice requests (9.4); the practice line says what a practice
  run does.
- K11 A stop while a starter is still inside `BrokerGate::start_with_kill` kills its handle (2.5), which refuses only
  the server's spawn. A start whose thread had not reached the gate's own preparation of the work directory when the
  worker removed it creates the directory again, empty, just before the process ends; the next start, or the next run
  once the store is open, removes it. Before the kill handle follow-up such a starter could also write its logs there
  (it was part of K1 case 2). L13 waits for `ready` before it quits; L21 quits after a server exists, so it does not
  cover this.
- K12 Other sessions running live Isabelle tests with the same user home at the same time share Isabelle's server
  registry; the policy crate serializes server starts only within one process.
- K13 Isabelle's temporary prefix is `/tmp/isabelle-<user>` by default, which another account on the Mac could create
  first; Isabelle creates it without an owner check. The relay cannot change it (Isabelle's settings assign it
  unconditionally). Answered by the gate on Unix (T9): before each start it appends
  `ISABELLE_TMP_PREFIX="$ISABELLE_HOME_USER/tmp"` once to `<user_home>/.isabelle/<identifier>/etc/settings`, makes that
  `tmp` with mode 0700, and does not start unless Isabelle names it as its prefix. A server found running under the
  name since before keeps its old prefix until it restarts; Windows keeps Isabelle's default.
- K14 Under the Careful profile a grant until revoked does not serve a create (the theory's `standing_serves`), so
  approving a create with "Until I take it back" sends the request back to `pending_approval`; the page offers that
  duration. The theory's owner answered on 2026-11-16 that the rule is deliberate, as that owner reads the frozen matrix
  (the product owner had not confirmed the reading then): under a row that asks, a standing grant serves only reading
  event details (CD-M02) and editing a Fabric-created event in Agent Drafts (CD-M05). For every other row that asks, the
  card should offer only "this request only"; the contract has no field for the durations a card may offer. The policy
  crate's owner reports that the product owner confirmed this reading on 2026-11-16 ('Yes, as it is'): under a row that
  asks, only an approval of exactly one request takes effect, except reading event details (CD-M02) and editing a
  Fabric-created event in Agent Drafts (CD-M05).
- K15 The gate's stop is bounded at 1.2 s (`GATE_STOP_BOUND`) and the gate kills the server after it, so an idle stop is
  acknowledged within about 1.3 s (the bound and the kill) against `QUIT_BOUND` of 1.5 s; a quit whose stop takes longer
  than the bound now ends in a kill, where before it ran on. With the gate before pull request 26, whose stop had no
  such bound (2.2; `BrokerGate::stop`, which the adapter called until this change, has been bounded at 20 s since then),
  the in-process stop took 0.8 s to 1.3 s on an idle Mac and the built broker's quit 0.9 s to 1.2 s against the relay's
  2 s drain. With this change, on 2026-11-16 on one Mac (the practice app and its own Isabelle server ran beside the
  tests), thirteen in-process stops of a ready gate took 0.88 s to 1.26 s, nine of them between 1.05 s and 1.20 s. One,
  at 1.26 s, ran to the bound and, by its time, ended in a kill; the tests do not print which way a stop ended. The
  built broker's quit (L13) took 1.01 s to 1.02 s in three runs, its quit with a server that does not answer (L20) 1.30
  s to 1.32 s in four runs, and the quit of L4's second broker, which reuses the orphan, 0.25 s to 0.90 s in four runs
  with no server left in any. So an orderly stop of an idle server ends close to the bound, and in this sample one quit
  in thirteen ended in a kill. The bound cannot grow unless `QUIT_BOUND` grows towards the relay's 2 s drain or
  `KILL_ALLOWANCE` shrinks (X9). With the kill handle follow-up, on 2026-11-17 on one Mac (the practice app and its own
  Isabelle server ran beside the tests): the built broker's quit during a gate start (L21) took 55 ms to 57 ms in four
  runs; the in-process stop during a check whose server does not answer (L22), which main kills at the stop's instant
  (2.5), took 1.31 s to 1.38 s in four runs, against `QUIT_BOUND` of 1.5 s, so it needs an otherwise idle Mac too; L20
  took 1.32 s in two runs, L13 1.06 s and 1.14 s, and the quit of L4's second broker 0.14 s in one run; ten idle
  in-process stops in one run of the whole live mode took 0.90 s to 1.24 s. A stop of a gate on a server found already
  running may take up to `FOUND_SHUTDOWN_GRACE` (0.3 s) past the gate's bound (T12, K1 case 4); with the gate's bound at
  the stop's instant that is `QUIT_BOUND`. An unacknowledged stop means the worker was late: a kill (the gate's at its
  bound, or the kill handle's of a check still running at the stop's instant) or what follows it took longer than
  `QUIT_BOUND`, as on a busy Mac, or the kill failed (K1 case 7), or the check runs on a found server, which no kill
  ends (K1 case 4); past the bound the calling thread removes the work directory. A killed Isabelle leaves its temporary
  files under the private prefix in the user home, and nothing removes them (K18). The live tests print the time and
  fail when the stop is not acknowledged.
- K16 Live proofs never run in CI (PLAN RD-7): CI compiles `tests/live.rs` on Unix and runs none of it, so a change
  that breaks the prover is found only when someone runs `--live-proofs` by hand. Windows and Linux builds were checked
  only by emulating their `cfg` on a Mac, not on those targets.
- K17 The live test L14 `checks_are_timed` fails the live mode when the mean check exceeds 1.5 s, and its margin is
  thin too: idle means were 1.22 s to 1.29 s, and one run on a loaded Mac (load average 6 to 10) failed at 1,529 ms
  and passed when repeated. Like the stop bound (K15), it needs an otherwise idle Mac.
- K18 Only the rule theory and `Fabric_Broker/ROOT` are pinned, and no `ROOTS` may stand beside them (2.1): by the
  worker before each start; since pull request 30 the gate checks `ROOT` and `ROOTS` too, before and after
  `session_start` (T11). The Isabelle distribution (`bin/isabelle` and what it loads) and the Isabelle user home (its
  `etc/settings` and `etc/components`, which Isabelle sources) are trusted as given, a second definition of
  `Fabric_Broker` in their component directories being refused by Isabelle as a duplicate session (9.5), and a debug
  host forwards the three paths with any store (X16). On Unix the gate writes one settings line into the user home (T9).
  A killed Isabelle leaves its scripts and temporary files under the private prefix in the user home, and nothing
  removes them; the live tests L2, L20, L21 and L22 kill one on every run, and a quit now ends in a kill whenever the
  gate's stop takes longer than the bound, a gate start is in progress, or a check still runs at the stop's instant
  (K15, 2.5). Whether an idle killed Isabelle leaves calendar text there is not known; the live test L1, which scans
  that directory after its stop, is the check that exists.
- K19 A forward step of the wall clock, or a sleep of the computer longer than the watchdog, ends every proof in flight
  as out of time: the core's watchdog and the adapter's lateness both read the wall clock (W4D D34, 2.3).
- K20 Results the shell still holds at quit (`pending_results`, 2.2) are dropped; their requests stay in their proving
  state and are proved again at the next start (W4D 5.8).
- K21 A result that names another request or purpose than its job is ignored by the core, and the job's watchdog ends it
  `undecided` with `deadline_passed`; the shell keeps the job in `handed`, so a later loss of the worker answers it too,
  and the core ignores that answer as well.
- K22 A gate start that never returns leaves the prover `starting` for the run: no bound of the adapter ends it, only
  the gate's own start bounds (2.3). A start that panics is a failed start (2.2).
- K23 In a synthetic run the core stores `no_prover` for every result inside `accepted()`, whatever the classifier
  states; the paths outside it (spent re-proofs, a result outside the facts, an unbound report, the watchdog) keep their
  own causes, as in an Isabelle run (W4D 5.10).
- K24 Requests made, or proved again after a restart, while Isabelle starts end `undecided` at once with
  `prover_starting`, read `prover_unavailable`: the worker answers STARTING (2.2). The page's sentence for a starting
  prover, "Requests wait until it is ready.", is not true until proofs are held while the prover starts. Since
  2026-11-22 every owner meets it on the first start after an installation (10.8).
- K25 A proof that decided but was not acted on (no provider, a paused agent, a tool that changes no calendar, no scope)
  stores its cause but reads `undecided_reason` null, since the contract has no value for it (W4D 5.7); the page then
  says that the broker did not record why. A paused agent is defensive here: a correct gate already blocks it.
- K26 A result the worker sent before its loss but that is still queued in the event channel when the shell notices the
  loss is superseded: the shell answers its job `NOT_RUNNING` first, the request ends `undecided` with
  `prover_not_running`, and the core ignores the real result by job id. Only the result that is the turn's own input is
  kept (2.2). The outcome is fail-closed.
- K27 A worker that ends while no job is out is noticed only at the next event the broker handles (an owner's
  command, a new request, a due time): only a job out bounds the main loop's sleep (2.2). Until then health keeps the
  last status it had (`ready`, or `failed` with a retry that will not come), and the first request after it is
  answered at once with not running.

### 9.2 Requests to other owners

To the owner of the rule theory and the policy crate (the design works without them):
- T1 a health probe, or a cancel, for a running check; in part answered by pull request 26: `check_within` bounds a
  whole check by its limit and `CHECK_GRACE` (2.3); a cancel from another thread by pull request 30: the kill handle
  (T10), which ends a running check's server, used at quit (2.5);
- T2 a typed "gate broken" error; done by pull request 26: `Fault` and `is_broken` (2.3, 2.4);
- T3 `BrokerGate::start` removes stale `Check_*.thy` files itself; done by pull request 26; the adapter keeps its own
  emptying (X10);
- T4 a `GateConfig` for a packaged layout, and a `ROOTS` that names only `Fabric_Broker`; in part answered by pull
  request 26: `<rules_dir>/ROOTS` is no longer read (2.1); the `GateConfig` for the packaged layout came with commit
  76bc41e (`GateConfig::for_installation`, `gate::isabelle_installation`); done 2026-11-22: the broker, the host, the
  bundle and an installer script use it (10); what remains is CD-W11's (10.8);
- T5 a bounded `stop` and `Drop`; done by pull request 26: `stop_within` (2.2, 2.5);
- T6 identifiers that admit `@` (CD-W05 calendar ids); answered on 2026-11-16: not needed, because a `calendar_id` is
  an opaque id issued by the broker, so CD-W05 issues a broker id for each Google calendar and keeps Google's id in its
  store;
- T7 export the expected theory hash, or make `BrokerGate::start` take one and refuse another theory; done by pull
  request 26: `theory::THEORY_TEXT`, `theory::SESSION_ROOT` and `built_theory_sha256` (2.1);
- T8 hash and load the theory from the same bytes (the window between the hash and `session_start`, 2.1); narrowed by
  pull request 26: the theory is hashed before and after `session_start`; the session's `ROOT` and `ROOTS` are checked
  the same way since pull request 30 (T11);
- T9 a private temporary prefix for Isabelle (K13); done by pull request 26 on Unix (3.4);
- T10 a `Send + Sync` kill handle on `BrokerGate`, available as soon as `start` has spawned the server, that ends the
  server's process groups from another thread at once and without leaving them stopped, so a quit can end a server whose
  gate is still in `start` or blocked in `check_within` (K1 cases 2, 3 and 5); done by pull request 30 on Unix:
  `KillHandle` and `BrokerGate::start_with_kill`; the kill runs in a helper process that finishes without its caller;
  the adapter gives every start a new handle and kills it at quit when that start is in progress or a check still runs
  at the stop's instant (2.2, 2.5);
- T11 compare `<rules_dir>/Fabric_Broker/ROOT` with `theory::SESSION_ROOT`, and refuse a `ROOTS` beside it, before and
  after `session_start`, as the gate does for the theory's hash; done by pull request 30: `gate::check_session_dir`
  before and after `session_start`; the adapter keeps its own pin (2.1);
- T12 for a server found already running under the name: stop and replace it at `start`; or, in `stop_within`, still
  write `shutdown` (on a fresh connection, without waiting for the answer) when `session_stop` has used the bound or
  the bound is zero (K1 case 4); done by pull request 30 the second way: `stop_within` still sends `shutdown` on a
  fresh connection, up to `FOUND_SHUTDOWN_GRACE` (0.3 s) past its bound, also after a kill through the handle, and the
  stop is then an error that says `shutdown` was sent without confirmation (K1 case 4, K15).

Nothing of pull request 28 is used.

To the owner of the contract:
- E1 a reason and a remedy beside `health.prover` (not configured, failed to start, wrong theory);
- E2 an owner-visible reason on `undecided`;
- E3 a marker for practice accounts, attempts and reports;
- E4 an agent error "rule prover unavailable".

Contract schema version 2 (pull request 28) answered E1 with `health.prover_problem`, E2 with
`get_request_details.undecided_reason` and E4 with the agent error `prover_unavailable`. The broker fills the first two
since the update of 2026-11-17 (2.2, 2.4); it does not send the third (3.3).

### 9.3 Questions that are the product owner's

Each with the default the code runs on today:
- O1 Amend W4D D13 for a debug-only practice provider (in memory; only with the synthetic agent, the test key, a scratch
  directory and Isabelle)? Default: built as designed; the amendment of W4D D13 and D26 is proposed with this step's
  pull request; the merge waits for the answer.
- O2 Accept the observation filled in by the shell until CD-W05 (X6)? Default: yes.
- O3 The restart policy: 10 s, 60 s, then down until the application restarts (X8)? Default: yes.
- O4 Does the release broker accept the Isabelle variables now, or refuse them until PLAN Q09 (X17)? Default: it accepts
  them; the host does not pass them in a release build. Since 2026-11-22 PLAN Q09 is answered for now (10.1): a release
  host passes the installed layout, and the release broker still accepts the three.
- O5 Keep the synthetic classifier for synthetic runs without Isabelle? Default: keep.
- O6 Does a debug host forward the Isabelle paths with a Keychain store (X16)? Default: yes; with no provider a fresh
  permit ends `undecided`. Since 2026-11-22 a debug host with none of the three passes the installed layout, with any
  store too (10.2).
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
7. `undecided` showed no reason (E2). Answered: the broker fills `undecided_reason` (2.4), and the page of pull
   request 32 shows it. Open only: a cause the contract maps to null reads "The broker did not record why." (K25).
8. The strip could not tell "not configured" from "failed to start" or "wrong rule theory" (E1). Answered: the broker
   fills `prover_problem` (2.2), and the page of pull request 32 shows it. Open only: the sentence "Requests wait until
   it is ready." for a starting prover (K24).
9. The approval cards of update and delete: the practice script's C and D are the first cards that are not creates.
   Whether every field the card shows has a text for them was not checked here.
10. Page tests for the practice run: every prover health has a text; no state text reachable in a practice run names
    Google; a denial shows its rule and proof; health is read from the snapshot and from health changes.
11. Since 2026-11-22 (10.4): an installed application whose Isabelle is missing shows the sentence for
    `not_configured`, "Nothing can be done about it in this run.", although running `sh scripts/install-isabelle.sh`
    and opening the application again would help; the contract's `not_installed` (schema version 3) is the follow-up.

### 9.5 Not checked

Windows and Linux targets (only this Mac; their `cfg` was emulated for clippy). That L1's scan fails on a real leak
(K7). Which W04 core tests cover S5. The edge of p44's bound and the drop of a failing live process (both timing). The
page's cards for update and delete (9.4 item 9). The `Ok`-and-broken path with a real Isabelle (a purge that fails after
a complete `use_theories`; only w22's double shows the restart). The kill at the bound at quit with a slow but answering
server: no test asserts it, and it was seen once, by its time only (K15); how often a quit ends in a kill beyond that
sample of thirteen. Whether `KILL_ALLOWANCE` (0.3 s) covers the gate's kill at its bound on a loaded Mac (its helper
process since pull request 30, `/bin/sh` with `ps` and `awk`, took 0.03 s idle and a stop with no time 0.05 s, as
measured by pull request 30; the kill before it, `ps` and two `kill`, took 0.06 s idle), and whether it covers main's
kill of a running check and what follows it there (on 2026-11-17, idle, L22's acknowledgement came 0.11 s to 0.18 s
after the stop's instant). The quit's own processing time before `Shell::stop` (the core's quit, its store write, the
last frames). The two cases pull request 30 did not provoke: the process ending while a kill is under way (K1 case 5)
and the kill helper running out of time (K1 case 7). A quit during a start or during a check on a server found already
running (K1 case 4). L21 relies on the owner channel's `starting`, which is the shell's view and can lag the worker by
one event, so a gate that became ready just before the snapshot would make its quit an orderly stop and the test would
still pass; in its runs the quit came well before Isabelle could be ready. The empty work directory of K11. Outside Unix
nothing was built or run here: w30 asserts there that the worker does not kill a start, and CI runs it on Windows; no
test covers that main kills no running check there (w31 is `cfg(unix)`). The new tests on CI or a loaded Mac: w31's 700
ms margin and w32's timing assume a scheduler that is not badly delayed, and L22's stop leaves about 0.1 s of
`QUIT_BOUND`. A quit during a check on a server that is computing: L22's server is stopped, and pull request 30 tried
the kill during a computing check at the gate only. What a server killed during a check leaves in Isabelle's temporary
files (K18): L22 does no scan. That main keeps its bound when the kill's helper hangs (by reading the code only).
For the update of 2026-11-17 (contract schema version 2): Linux and Windows; a loaded machine (the new tests that
wait on a real clock, sh04 and sh11 and the deadline and limit tests, and the live bounds of K15 and K17 were not
tried under load); a store file written by another build, in either direction (the tests forge rows of the old shape
into a store this build made); the packaged application; a worker that ends while no job is out (K27) with a real
Isabelle; the statuses `failed` and unavailable with no cause, and a request made while Isabelle starts, with a real
Isabelle (a stand-in shows them).

Checked in Isabelle2025-2's sources on 2026-11-16: Isabelle reads the `ROOT` and the `ROOTS` of every directory it is
given, the `dirs` of `session_start` included (`src/Pure/Build/sessions.scala` 1284-1325: `load_dir` is `load_root`
followed by `load_roots`), and a second definition of a session is an error (same file 866-873). Checked on 2026-11-16
on one Mac, before the adapter follow-up: `sh scripts/verify-desktop.sh --live-proofs` passed three runs in a row
and once more after the last fixes; a manual start of the debug broker under an emptied environment with the fixed
`PATH` went from approval through a fresh permit to `applied`, left no check file, and left no server after quit.
Checked on 2026-11-16 on the same Mac, with the adapter follow-up: `sh scripts/verify-desktop.sh --live-proofs` passed
once (the first test alone, the corpus's 3 tests, the live binary's 16 tests); then L4 passed twice more, L3 and L13
twice more each and L20 once more, each run alone; a check at a 1 ms limit took 0.08 s to 0.10 s (L3). A process
listing after the full run and after the single runs showed no Isabelle server but the practice app's own. The times
are in K15; they include one run each of L3 and L4 and two of L20 made before the last review fixes, which changed L4's
clean-up and added the start guards of 2.2, not the stop path. The Keychain was not touched: every test uses the test
key.
Checked on 2026-11-17 on one Mac, with the kill handle follow-up: a quit during a start and during a long check with a
real Isabelle (K1 cases 2 and 3), each run alone three times: L21 and L22 passed every time (times in K15); L22's
check could not end by itself, its server's processes being stopped, and its stderr held the one failure line 2.5
names. L20 and L13 passed once each with the changed adapter. After every run a process listing showed no Isabelle
server but the practice app's own and no stopped process. The unit tests of the prover passed ten runs in a row, and
removing each new behaviour in turn made its test fail: the worker's kill (w30), main's kill (w31), the condition that
a check runs (w32), the read of `stop_by` in `check_job` (w34), and a handle reused across starts (w33). Those single
runs were made before the last review fixes, which changed comments and this document only. Then, on the final code,
`sh scripts/verify-desktop.sh --live-proofs` passed once (the first test alone, the corpus's 3 tests, the live binary's
18 tests, L21 and L22 among them), and a process listing after it showed no Isabelle server but the practice app's own
and no stopped process. The Keychain was not touched: every test uses the test key.

Checked on 2026-11-17 on one Mac, with the update for contract schema version 2 and `main` at 2d41210 merged in: `sh
scripts/verify-desktop.sh --live-proofs` passed three times, once before the merge, once after it and once after the
look for a lost worker (2.2) was added: L11 alone, the corpus (3 tests) and the 18 live tests each time. In the last
run the mean check took 1235 ms and 1226 ms, the slowest check 1837 ms, and the slowest stop 1325 ms against its 1500
ms bound. The new asserts held with the real Isabelle: a check with a 1 ms limit reads `out_of_time` (L3); the
request whose check was running when Isabelle was killed reads `failed`, and health goes `failed_will_retry`, then
starting and ready with no problem (L2, L16); a changed rule file and session files that load another theory read
`wrong_rules` (L18, L19); health carries no problem while starting (L21); and the two rules of p48 held on every
owner line of the live harness. The practice application and its own Isabelle server ran on the same Mac. In a
browser through the harness, in a run without Isabelle, the strip's text began with the sentence for
`not_configured`, and an allowed request's row read the sentence for `prover_unavailable`.

For the installed layout of 2026-11-22: what was run is in 10.7, and what is left in 10.8.

---------------------------------------------------------------------------------------------------------------------

## 10. The installed layout (T4)

Added 2026-11-22 (branch `claude/cd-installed-rule-checker`), as built. The owner answered PLAN Q09 on 2026-11-17
(PLAN 15.2, CD-Q09): "Installed once by a script into a fixed folder that the app finds, version 2025-2. Bundling or
download is decided at release". Under that answer the session lead decided I1 to I12 below on 2026-11-22; they are
D1 to D12 of the lead's design of that day, in the same order (code comments of this change cite them as `PW I1` to
`PW I12`).
This section answers T4 (9.2): the policy crate's `GateConfig::for_installation` and `gate::isabelle_installation`
(commit 76bc41e, GATE) now have callers: the broker, the host, the bundle and an installer script. The developer mode,
the three variables of X1, stays as it is.

### 10.1 Decisions

| # | Topic | Chosen | Why |
|---|---|---|---|
| I1 | Two broker variables | `FABRIC_BROKER_APP_RESOURCES_DIR`, the application's resources folder (in a bundle `Contents/Resources`; the rule theory is `<it>/isabelle/Fabric_Broker/{ROOT,Fabric_Broker.thy}`), and `FABRIC_BROKER_APP_SUPPORT_DIR`, the application's own folder for this user (Isabelle is `<it>/Isabelle2025-2`, its user home `<it>/isabelle-user-home`); both or none, each absolute, never with any of the three developer variables; each refusal exit 78 with one stderr line naming the variable; read in every build, as X17 says of the three (10.2) | `for_installation` takes the two folders plus the broker's own work directory and server name, so the broker is its natural caller and the folder names stay in the policy crate. The broker can tell an installed application without Isabelle from a run without a prover, and the developer mode keeps its rules and its pinned tests |
| I2 | The support folder | The host's own folder, `~/Library/Application Support/ai.paracels.fabric.calendar`, which already holds `instance.lock` and `broker/`; in a debug build `FABRIC_DESKTOP_DATA_DIR` moves it | A test's scratch folder is then also where Isabelle is looked for, so no test touches the real folder, and CI's packaged check finds no Isabelle. No administrator rights. Cost: 3.3 GB beside the store |
| I3 | Which runs use it | The host passes the two in every build whenever it does not pass the three: a release host always, a debug host when none of the three is set; a debug host with all three set is the developer mode (X16); it computes the two and never reads them from its own environment | `build-desktop.sh` refuses the release configuration until CD-W11, so a release-only path could not be run before then |
| I4 | One decision before the plan | With the two set, `config_from` calls `isabelle_installation(<support>/Isabelle2025-2)` once (10.3). `Found`: `Config.isabelle` holds the installed paths. `Missing`, `OtherVersion` or an error reading the two files: no rule checker, one stderr line, no exit, and not a practice run. `Config.installation` keeps what was found | The practice calendar and the classifier depend on whether Isabelle is configured (`Config::practice`, `Plan::of`); deciding in the worker would turn CI's packaged check into a practice run that never proposes. An installation finished while the application runs is seen at its next start |
| I5 | The reason the owner sees on main | `not_configured`, as for any run without a rule checker (the core's first health with no prover attached); `not_installed` for `Missing` and `OtherVersion` is a follow-up once contract schema version 3 (pull request 36) is merged; a comment in `Plan::of` marks the place | This change lands on main at schema version 2 and uses no contract value main does not have |
| I6 | Another Isabelle version in the folder | Not installed (I4); it is never started | The rule theory was verified with Isabelle2025-2, whose name is part of every proof id |
| I7 | Isabelle's user home | The gate creates `<support>/isabelle-user-home`; in the installed layout the broker neither requires nor creates it; the developer mode keeps its check (3.1) | It is what `for_installation`'s documentation says and what the gate does; a user home deleted later comes back by itself |
| I8 | The rule files | The configuration check does not look at them (no exit 78 for a broken bundle). The worker's pin (2.1) runs before each gate start as before, with one change in both modes: a rule file that does not exist (`NotFound`) is `wrong_rules`, down for the run, not a failed start that is retried; every other read error stays a failed start (`I/O`) | A missing file in a bundle is a broken installation, for which `wrong_rules` has the fitting remedy ("Install Fabric Calendar again."), and the broker keeps running for the store and the screens |
| I9 | The host | `BrokerProver` is the developer paths or the installed layout; the resources folder is Tauri's `resource_dir()` of the running application; when it cannot be resolved the host reports a configuration error ("nothing was started") and starts no broker; the relay's allowlist gains the two names; the host sets the two itself and drops any it inherited (10.2) | As the host already does for the data directory |
| I10 | The bundle | `bundle.resources` in `tauri.conf.json` is a map that puts the repository's two rule files at `isabelle/Fabric_Broker/ROOT` and `isabelle/Fabric_Broker/Fabric_Broker.thy` under the resources folder; no other file of `isabelle/` is bundled; `build-desktop.sh` and `verify-desktop.sh --packaged` check them (10.5); `isabelle/` itself is not changed | The pin needs `ROOT` byte for byte and no `ROOTS` beside it; the list form of `bundle.resources` would put the files under `_up_` folders |
| I11 | The installer | `scripts/install-isabelle.sh` (10.6); the archive pin is the one of `scripts/setup-demo.sh`, which is not edited, and `verify-desktop.sh` checks that the two are equal | The owner's answer says "by a script"; bundling against download stays open, so a download and a local archive both work |
| I12 | Not in this change | 10.8 | Each is a change of its own or belongs to CD-W11 |

### 10.2 The two variables, and what the host passes

The broker (`apps/fabric-broker/src/config.rs`, constants `APP_RESOURCES_DIR` and `APP_SUPPORT_DIR`) refuses, with
exit 78 and one stderr line before stdin or threads, as for the refusals of 3.1:

| Case | The stderr line |
|---|---|
| either of the two with any of the three developer variables (checked first) | `fabric-broker: <the first of the two set> is set with <the first developer variable set>; the rule prover takes the installed layout or the three Isabelle paths, not both` |
| one of the two without the other | `fabric-broker: the installed rule prover needs FABRIC_BROKER_APP_RESOURCES_DIR and FABRIC_BROKER_APP_SUPPORT_DIR together, or none` |
| either of the two not absolute | `fabric-broker: <variable> is "<path>", which is not an absolute path` |

The refusal of an unknown `FABRIC_BROKER_*` name and the usage line of `main.rs` list eight names, the two new ones
last, `FABRIC_BROKER_APP_RESOURCES_DIR` before `FABRIC_BROKER_APP_SUPPORT_DIR`.

The host (`apps/fabric-desktop/src-tauri/src/host.rs`): `BrokerProver` is `Developer { isabelle_home,
isabelle_user_home, rules_dir }` or `Installed { resources_dir, support_dir }`, chosen by
`BrokerProver::prover_from(values, release, root, resources_dir)`; `BrokerStore::from_environment(app_data_dir,
resources_dir, vars)` gains the resources folder, which `main.rs` takes from `application.path().resource_dir().ok()`.
The support folder is the host's root: `app_data_dir`, or `FABRIC_DESKTOP_DATA_DIR` in a debug build.

| Host | The three developer variables | It passes |
|---|---|---|
| release | any | the installed layout: `FABRIC_BROKER_APP_RESOURCES_DIR` is the resources folder, `FABRIC_BROKER_APP_SUPPORT_DIR` is `app_data_dir`; never the three |
| debug | all three set and absolute | the three (the developer mode, X16), with any store; it needs no resources folder |
| debug | none | the installed layout, with the host's root as the support folder |
| debug | one or two, or a relative one | nothing: the configuration error of 3.2, as before |
| release, or debug with none of the three | the resources folder is not resolved or not absolute | nothing: the configuration error `NO_RESOURCES`, "the application's resources folder, which holds the rule theory, could not be found; nothing was started" |

`BrokerStore::variables()` emits the two after the data directory, the key source and the synthetic agent, and the
broker's environment still drops every inherited `FABRIC_BROKER_*` name first. The trace's `connected` line ends with
` prover` for the developer paths and ` installed` for the installed layout. The relay
(`apps/fabric-desktop/relay/src/config.rs`) has the constants `BROKER_APP_RESOURCES_DIR` and `BROKER_APP_SUPPORT_DIR`,
re-exported from the crate root; `ENVIRONMENT_ALLOWLIST` holds them last, eight names (with `SystemRoot` first on
Windows). The harness (`harness/serve.ts`) passes neither: started without the three it still runs with no rule
checker.

What `resource_dir()` returns (tauri-utils 2.10.2, `platform.rs`): in a bundle `<executable's folder>/../Resources`,
canonicalized, so an error when `Contents/Resources` is missing; from a Cargo `target/<profile>` folder that holds
`.cargo-lock`, the executable's own folder, where tauri-build copies the two files when the host compiles
(`target/debug/isabelle/Fabric_Broker/`); and an error on macOS when the executable's path goes through a symbolic
link (Tauri's `STARTING_BINARY` refuses it). So a copy started by a path below `/var/folders` (`/var` is a link)
reports `NO_RESOURCES`; `verify-desktop.sh` starts its copies from the scratch folder's physical path (`pwd -P`), in
`--packaged` and `--installed-proofs`. An application under `/Applications` has no link in its path.

### 10.3 The broker's one decision before its plan

With the two folders set, `config_from` calls
`fabric_broker_policy::gate::isabelle_installation(<support>/Isabelle2025-2)` once, before `Plan::of`. `main.rs` prints
`Installation::line()`, when there is one, right after `config_from`, once.

| What it finds | `Config.installation` | `Config.isabelle` | The stderr line |
|---|---|---|---|
| the two folders not set | `NotAsked` | from the three variables, or `None` | none |
| `Found` | `Found { resources_dir, support_dir }` | `<support>/Isabelle2025-2`, `<support>/isabelle-user-home`, `<resources>/isabelle` | none |
| `Missing` | `Missing { isabelle_home, unreadable: None }` | `None` | `fabric-broker: Isabelle2025-2 is not installed: "<support>/Isabelle2025-2" holds no file bin/isabelle; the rule prover does not run in this run` |
| `OtherVersion(name)` | `OtherVersion { isabelle_home, name }` | `None` | `fabric-broker: Isabelle2025-2 is not installed: "<support>/Isabelle2025-2" holds "<name>", another version, which is not started; the rule prover does not run in this run` |
| an error reading the two files | `Missing { isabelle_home, unreadable: Some(error) }` | `None` | `fabric-broker: Isabelle2025-2 is taken as not installed: "<support>/Isabelle2025-2" could not be read (<error>); the rule prover does not run in this run` |

The paths and the name are printed with `{:?}` and the error's line breaks become spaces, so the line is one line.
An Isabelle with no identifier file is another version with the name `""`. With `Found` the broker does not require
the user home (I7), does not look at the rule files (I8) and does not check the execute bit of `bin/isabelle`
(`isabelle_installation` does not): an Isabelle that does not run shows as a failed start (2.2). A run without a rule
checker is not refused and is not a practice run (`Config::practice` needs `isabelle`), so a synthetic run keeps its
classifier (3.3, O5) and CI's packaged check is unchanged.

`prover::gate_config(isabelle, installation, data_dir)` is the one function that builds the `GateConfig` in both
modes: `GateConfig::for_installation(resources, support, <data_dir>/prover, server_name(data_dir))` for `Found`, the
three paths otherwise, then `CHECK_TIMEOUT` and the consolidate delay of 0.1 s (`CONSOLIDATE_DELAY_SECONDS`) in both.
`Plan::of` uses `ProverOptions::with_installation(isabelle, installation, data_dir)`; `ProverOptions::new(isabelle,
data_dir)` keeps its signature and means the developer mode (`Installation::NotAsked`).

The pin (I8, `Worker::enter_starting`, 2.2 step 2): a `NotFound` from `pin` calls `other_theory()`: line T, the
problem `wrong_rules`, down for the run, no gate start and no retry. Every other read error is still a failed start
(`I/O`) that is tried again. A missing `ROOTS` still passes. This holds in both modes; in the developer mode the
configuration check already refuses a missing rule file at start (3.1), so there it arises only when a file goes
after the start. Line T names the rules folder the pin read, not a variable, so it is also right in the installed
layout, which does not set `FABRIC_BROKER_RULES_DIR` (2.2).

### 10.4 What the owner sees

| The installed layout | Health | The strip on the page today |
|---|---|---|
| Isabelle `Found` | `starting`, then `ready` | "The rule checker is starting." with "Requests wait until it is ready." until it is ready; the first start after an installation builds the session heap (K4) and requests made meanwhile end not done (K24) |
| `Missing`, `OtherVersion` or unreadable | `unavailable`, `not_configured` | "The rule checker is not running." with the sentence for `not_configured`, "This run has no rule checker. Nothing can be done about it in this run." |
| a rule file missing from the bundle | `unavailable`, `wrong_rules` | the sentence for `wrong_rules`, which ends "Install Fabric Calendar again." |
| the resources folder not found | no broker | "Fabric Calendar is not installed correctly.", with `NO_RESOURCES` as the host's reason |

The sentence for `not_configured` is the wrong advice here, since running the installer helps; that is the follow-up
with `not_installed` (I5). With Isabelle found and the synthetic agent (a debug build with the test key and a scratch
folder) the run is a practice run (X2); without it, as in the product, requests are proved and a fresh permit ends
`undecided`, since no provider is attached before CD-W05 (3.3).

### 10.5 The bundle's rule files

`apps/fabric-desktop/src-tauri/tauri.conf.json`: `bundle.resources` maps `../../../isabelle/Fabric_Broker/ROOT` to
`isabelle/Fabric_Broker/ROOT` and `../../../isabelle/Fabric_Broker/Fabric_Broker.thy` to
`isabelle/Fabric_Broker/Fabric_Broker.thy` (sources relative to `src-tauri`), so they land at
`Contents/Resources/isabelle/Fabric_Broker/` and nothing else is under `Contents/Resources`. The host's build fails
when either source file is missing. Checks on them:
- `scripts/build-desktop.sh`, step 4, after the signature checks: both files are there and equal to the repository's
  (`cmp -s`); there is no `ROOTS` in `isabelle/` or `isabelle/Fabric_Broker/`; the listing of
  `Contents/Resources/isabelle` is exactly `Fabric_Broker`, `Fabric_Broker/Fabric_Broker.thy` and
  `Fabric_Broker/ROOT`. Each failure stops the build with its own line, for example "the bundle has a ROOTS file
  beside the rule files: <path>".
- `scripts/verify-desktop.sh --packaged`: the same check (`rule_files`) on its copy, right after the copy's signature
  check and before its launches; it prints "ok: the bundle holds the repository's two rule files and no ROOTS". Its
  launches still unset the three developer variables; the host then passes the installed layout with the scratch
  folder as the support folder, finds no Isabelle there, and each run stays a synthetic run without the rule checker.

### 10.6 The installer script

`scripts/install-isabelle.sh` (POSIX sh, `set -eu`, macOS only):

```sh
sh scripts/install-isabelle.sh [--support-dir <dir>] [--archive <file> | --url <url>]
```

- The default target is `$HOME/Library/Application Support/ai.paracels.fabric.calendar`; `--support-dir <dir>` names
  another, which must be absolute (tests and stand-ins). `--archive <file>` uses a local archive and downloads
  nothing; `--url <url>` downloads from another place; by default it downloads with curl from the URL of
  `scripts/setup-demo.sh`. `--help` (or `-h`) prints its header.
- The pin: the four lines `ISABELLE_VERSION`, `ARCHIVE_BYTES`, `ARCHIVE_SHA256` and `ARCHIVE_URL` are textually those
  of `scripts/setup-demo.sh`. The size, then the SHA-256 are checked before the support folder is created or anything
  is unpacked; a mismatch is refused ("... does not match the pinned Isabelle2025-2 archive: expected 1730395791
  bytes, received <n>. Nothing was unpacked.") and nothing is left behind. A download goes to a temporary file in
  the system's temporary folder, is not resumed, and is removed at exit.
- Three outcomes for an existing `<support>/Isabelle2025-2`. Present with `bin/isabelle` and `etc/ISABELLE_IDENTIFIER`
  reading `Isabelle2025-2`: "Isabelle2025-2 is already installed in <support>/Isabelle2025-2; nothing to do.", exit
  0, untouched. Present otherwise: refused, "<support>/Isabelle2025-2 exists but is not an Isabelle2025-2 installation
  (...). ... Move it away and run this again; this script does not delete it.", exit 1. Absent: it installs.
- It creates the support folder only when it is missing and its parent exists, with mode 0700, and never its parents,
  `broker/` or the store. It unpacks into a new `<support>/.install-isabelle.XXXXXX`, checks the two files on the
  unpacked `Isabelle2025-2.app`, checks the target once more, and renames it to `<support>/Isabelle2025-2` with one
  `mv`; then it checks the two files again. If a folder appeared between the last check and the `mv`, the `mv` moves
  the copy into it; the script then moves that copy back into its unpack folder and stops with "appeared while this
  script ran", so a folder it did not create is left as it was. It runs nothing from the archive, needs no sudo, and
  does not build the session heap: the first start of the application builds it.
- On a failure or an interrupt (INT exits 130, TERM 143, HUP 129) it removes its unpack folder and its download, and
  removes the support folder only if this run created it and it is empty. Every error is one stderr line starting
  "error: ", exit 1. On another system: "error: this script installs Isabelle for the macOS application Fabric
  Calendar; on this system there is nothing to install."
- Its last line: "Isabelle2025-2 is installed in <support>/Isabelle2025-2. Quit Fabric Calendar and open it again to
  use it." The broker decides once per run (I4), so a running application sees the installation only after that.

### 10.7 How it is checked

CI, without Isabelle (`sh scripts/run.sh cargo test`, and the host's tests in `verify-desktop.sh`):

| Id | File | Test | Shows |
|---|---|---|---|
| c43 | `apps/fabric-broker/src/config.rs` | `the_refusal_lists_eight_names` (was `the_refusal_lists_six_names`) | 10.2, the unknown-name refusal |
| c44 | `apps/fabric-broker/src/config.rs` | `installed_folders_are_both_or_none` | I1 |
| c45 | `apps/fabric-broker/src/config.rs` | `installed_folders_and_isabelle_paths_are_exclusive` | I1 |
| c46 | `apps/fabric-broker/src/config.rs` | `a_relative_installed_folder_is_refused` | I1 |
| c47 | `apps/fabric-broker/src/config.rs` | `an_installed_isabelle_gives_the_installed_layout` | I4, I7, I8: `Found` gives the paths of `for_installation`; the user home and the rule files need not exist; an identifier with and without a line feed |
| c48 | `apps/fabric-broker/src/config.rs` | `a_missing_installation_is_a_run_without_a_rule_prover` | I4, the exact `Missing` line |
| c49 | `apps/fabric-broker/src/config.rs` | `another_isabelle_version_is_not_installed` | I6, the name in the line, and `""` without an identifier file |
| c50 | `apps/fabric-broker/src/config.rs` | `an_unreadable_installation_is_taken_as_missing` | I4 |
| c51 | `apps/fabric-broker/src/config.rs` | `a_synthetic_run_with_a_missing_installation_is_not_practice` | I4 |
| w37 | `apps/fabric-broker/src/prover/mod.rs` | `a_pin_that_cannot_read_its_files_will_retry` (changed) | I8: a `Fabric_Broker.thy` that is a directory is still a failed start that is retried |
| w41 | `apps/fabric-broker/src/prover/mod.rs` | `the_installed_gate_is_the_installed_layout` | I4: `gate_config` of `Found` equals `for_installation`'s folders, not the three paths it is given, with `CHECK_TIMEOUT` and the consolidate delay; `NotAsked` takes the three paths |
| w42 | `apps/fabric-broker/src/prover/mod.rs` | `a_missing_rule_file_leaves_the_prover_unavailable_with_wrong_rules` | I8: a missing `.thy`, a missing `ROOT`, a missing rules folder; no start, no retry |
| p40 | `apps/fabric-broker/tests/process.rs` | `configuration_errors_exit_78_before_any_frame`, seven new rows | I1: each folder alone, each folder relative, a folder with `FABRIC_BROKER_ISABELLE_HOME`, a folder with `FABRIC_BROKER_RULES_DIR`, both with all three |
| p49 | `apps/fabric-broker/tests/process.rs` | `an_installed_layout_without_isabelle_runs_without_a_rule_prover` | I4, I5: an empty support folder: exit 0, `unavailable` with `not_configured`, build `CD-W04 synthetic`, the stderr line exactly once, nothing created in the support folder |
| p50 | `apps/fabric-broker/tests/process.rs` | `an_installed_layout_without_its_rule_theory_is_wrong_rules` | I8: a stand-in Isabelle2025-2 and an empty resources folder: `unavailable` with `wrong_rules`, line T exactly once naming `<resources>/isabelle` and not `FABRIC_BROKER_RULES_DIR`, no user home created |
| h01 | `apps/fabric-desktop/src-tauri/src/host.rs` | `a_debug_host_passes_the_prover_paths_only_all_three_and_absolute` (changed) | I3: all three give the developer paths, with or without a resources folder; none gives the installed layout |
| h02 | `apps/fabric-desktop/src-tauri/src/host.rs` | `a_release_host_ignores_the_prover_variables` (changed) | I3: a release host gives the installed layout for any of the three, never the developer paths |
| h05 | `apps/fabric-desktop/src-tauri/src/host.rs` | `a_host_without_its_resources_folder_passes_no_installed_layout` | I9: no or a relative resources folder is `NO_RESOURCES`, a configuration error with nothing started |
| h04 | `apps/fabric-desktop/relay/tests/relay.rs` | `prover_names_are_exported_and_allowlisted` (extended) | I9: all five prover names |

Also extended: `the_variable_names_are_those_the_host_and_the_relay_spell` and `an_argument_is_a_usage_error`
(the broker); `allowlist_names_the_broker_variables` and `broker_variables_are_named_by_the_prefix` (the relay);
`a_debug_host_takes_the_test_key_only_with_a_scratch_directory`,
`every_broker_variable_the_host_sets_passes_the_relay_allowlist` and
`the_broker_environment_drops_inherited_broker_variables` (the host). Changed so they keep their meaning under I8:
w37 above, and sh06 `a_request_while_the_isabelle_prover_is_down_ends_undecided`, which now uses the repository's rules.
The scripts: the sources mode of `verify-desktop.sh` has a new first step, "[1/6] The pinned Isabelle archive"
(`archive_pin`), which fails with "scripts/install-isabelle.sh and scripts/setup-demo.sh pin different Isabelle
archives (version, URL, size or SHA-256)."; the mode still needs Node, whose check comes first, and CI runs it on
macOS. `build-desktop.sh` and `--packaged` check the rule files (10.5). The repository has no harness
for shell scripts, so the installer has no CI test.

By hand, on a Mac, never in CI: `sh scripts/verify-desktop.sh --installed-proofs`. It needs the debug bundle, the
pinned archive in `.tooling/isabelle/download/` (it never downloads) and about 4 GB free. In a new scratch folder
(`mktemp -d`, its physical path):
1. "[1/6] The installer": `install-isabelle.sh --support-dir "<scratch>/a path with spaces/Application Support"
   --archive <the archive>`; its last line, the folder's mode 700, and a folder that holds only `Isabelle2025-2`;
2. "[2/6] The installer again": "already installed";
3. "[3/6] The installer with a wrong archive": a small wrong file into a second scratch folder is refused by its size,
   then a sparse wrong file of exactly the pinned size by its SHA-256, and that folder stays empty;
4. "[4/6] The bundle": the copy's signature and its rule files; a listing of `Contents/Resources` is taken;
5. "[5/6] The application with the installed Isabelle": the copy started with `FABRIC_DESKTOP_DATA_DIR` set to the
   support folder, the test key, the synthetic agent, and the three developer variables and the two folders unset;
   it waits up to `FIRST_START_SECONDS` (900 s) for the first `change=request` notice, whose request the practice agent
   makes only once the rule checker is ready. A run that found no Isabelle is a synthetic run whose classifier also
   proposes, so the step then also requires the host's `connected` line to end in ` installed` and fails if the trace
   holds the broker's "Isabelle2025-2 is not installed" or "is taken as not installed" line; together they show a live
   Isabelle from the installed layout, and step 6's heap check backs them;
6. "[6/6] Quit, then the bundle, the heap and the resources": the host this run started is killed, every process of
   this run must end within 30 s, the signature must verify again, the listing of `Contents/Resources` must be
   unchanged, and a `Fabric_Broker` heap must be below `<support>/isabelle-user-home`.
It removes its scratch folders, kills only processes whose command line names its scratch folder, never uses the
Keychain and never touches the real support folder.

Run on 2026-11-22 on one Mac, by the builders of this change: `cargo test -p fabric-broker` (library 124, process
28, relay round trip 9 passed; live 18 ignored), the release library tests (92), clippy with `-D warnings`;
`cargo test --locked -p fabric-desktop-relay` (26 and 20); `sh scripts/verify-desktop.sh` (host 19 tests);
`sh scripts/build-desktop.sh --configuration debug` with its rule-file check; and `sh scripts/verify-desktop.sh
--packaged`, which passed after the change to the physical path (10.2). The installer was run once from the cached
archive into a scratch folder whose path has spaces (16 s, then "already installed"), and once stopped with TERM
during the unpack (exit 143, nothing left).

Run the same day by the lead on the final tree, after the review's fixes (process tests now 29): `sh
scripts/verify.sh`, `sh scripts/build-desktop.sh --configuration debug` and `sh scripts/verify-desktop.sh
--packaged` passed. `sh scripts/verify-desktop.sh --installed-proofs` passed twice, before and after the fixes;
the first request notice came 22.2 s and 21.5 s after the application started, which includes the first build of
the heap. `sh scripts/verify-desktop.sh --live-proofs` failed its first run in one test, `checks_are_timed` (mean
check 1726 ms against 1.5 s), while another session's builds ran on the same Mac (load average 15); the one rerun
passed (1, 3 and 18 tests; mean 1286 ms). That the first failure was the load is likely and not proven. The
download path of the installer was not run, and neither was a release build.

### 10.8 Not in this change

- Holding proofs while Isabelle starts (K24), and the page's sentence for a starting prover; every owner meets that
  state on the first start after an installation.
- `not_installed` (contract schema version 3, pull request 36): until the follow-up the owner sees `not_configured`,
  whose sentence says nothing can be done (10.4, 9.4 item 11).
- Signing and notarization, Gatekeeper and quarantine for the installed Isabelle, and how a released application
  ships the installer (CD-W11). A release build of the bundle: `build-desktop.sh` still refuses it, so the release
  host's path is tested only through the `release: bool` parameter (h02).
- The fallback URL of PLAN RD-8: the installer has the one URL of `setup-demo.sh`; `--url` names another.
- A heap rebuilt when the application moves to another folder (GATE's documentation of `for_installation`; no test
  asserts it), and the folder of an older version left behind by a later one; the broker removes nothing in the user
  home (3.4).
- The execute bit of `bin/isabelle` and whether the installed Isabelle runs: `Found` reads two files only (10.3).
- A by-hand run of the release application against the real support folder and the Keychain (CD-W11).
