// All state and logic of the page, with no DOM and no globals: the clock, the id maker and the
// host come in through `Deps`. The views read the fields below after every `changed()` call.
// The page keeps no state the broker owns: everything here is what the broker last reported.
import type {
  Account, Agent, Approval, DurationKind, Enrollment, ErrorCode, Grant, Health, OwnerCommand, OwnerError, OwnerMethod, OwnerMethods,
  OwnerNotice, OwnerRequest, OwnerResponse, ParamsDecideRequest, Party, Preferences, Profiles, Protocol, RecipientSet,
  Request, RequestDetails, ResultHello, SCHEMA_VERSION as SchemaVersion, Snapshot, UndecidedReason,
} from "../../../../contracts/desktop/v1/typescript/src/contract.ts";
import type { Host, HostMessage, HostState, SendRefusal } from "./host.ts";
import { ALLOWED_TEXT, CHECKING_STATES, OPERATION, STATE_TEXT, STORE_PROBLEM, TERMINAL, agentName, changeText, proverProblemText } from "./text.ts";
import type { Names } from "./text.ts";

const PROTOCOL: Protocol = "fabric-desktop/1";
const SCHEMA_VERSION: typeof SchemaVersion = 3;
export const CLIENT: Party = { name: "Fabric Calendar", build: "CD-W08 webview" };
export const ACTIVITY_LIMIT = 200;
export const ENDED_GRANT_LIMIT = 200; // grants that ended while this window was open
export const ENDED_REQUEST_LIMIT = 200; // requests in a terminal state

export interface Deps {
  readonly host: Host;
  readonly now: () => number; // Unix time in milliseconds
  readonly newId: () => string; // a frame id that the contract's Id pattern accepts
  readonly zone: string; // the time zone the page shows times in
  readonly changed: () => void; // called after every change
  readonly announce: (text: string) => void; // said by the polite live region
}

// Why a query or command did not succeed.
export interface Failure {
  readonly text: string; // the broker's message, or words for a send that failed
  readonly code: ErrorCode | null; // null: no answer came (not sent, or the broker stopped first)
  readonly error: OwnerError | null; // the whole error when the broker answered one
}

// Methods that change nothing; commands go through `command`.
export type OwnerQuery = Exclude<OwnerMethod, OwnerCommand>;

// Where the page is in "hello, snapshot, subscribe".
export type Phase = "waiting" | "hello" | "snapshot" | "subscribe" | "live" | "failed";

export interface Entry {
  request: Request;
  requestRun: number; // the run whose broker last described this request
  available: boolean; // false when the broker no longer lists this open request
  checkingDelayMs: number | null; // ms after received_at that "Received, checking" was first shown
  details: RequestDetails | null; // the last get_request_details answer, the content of the card
  detailsRun: number;
  reading: boolean;
  undecided: { readonly reason: UndecidedReason | null } | null; // why no proof decided it, as the details said; null until read
  reasonReading: boolean; // a redacted read for the reason alone is in flight
  detailsOpen: boolean; // the owner has the details open: they are read, and read again when the request changes
  changed: boolean; // the digest differs from the card read before; the owner must acknowledge
  duration: DurationKind; // the owner's choice, kept across a disconnect
  durationChosen: boolean;
  command: { readonly id: string; inFlight: boolean } | null;
  approval: Approval | null;
  note: string | null; // the last problem with this request, in words
}

// What the card offers. "waiting" and "changed" keep the buttons on the page but disabled.
export type CardStatus =
  | { readonly kind: "none" }
  | { readonly kind: "ready" }
  | { readonly kind: "waiting"; readonly text: string }
  | { readonly kind: "changed"; readonly text: string }
  | { readonly kind: "closed"; readonly text: string };

interface Pending {
  readonly ok: (result: unknown) => void;
  readonly fail: (f: Failure) => void;
}

function refusalText(r: unknown): string {
  const s = r as SendRefusal;
  if (s?.kind === "refused") return `this app refused to send it (${s.reason})`;
  if (s?.kind === "not_connected") return "the broker is not running";
  if (s?.kind === "io") return `it could not be sent (${s.detail})`;
  return String(r);
}

const noAnswer = (text: string): Failure => ({ text, code: null, error: null });

function proverAnnouncement(h: Health): string {
  if (h.prover === "ready") return "The rule checker is ready.";
  if (h.prover === "starting") return "The rule checker is starting.";
  return `The rule checker is not running. ${proverProblemText(h.prover_problem)}`;
}

export class Model {
  run = 0;
  connection: HostState | null = null;
  phase: Phase = "waiting";
  problem: string | null = null; // a problem of the connection as a whole
  syncFailure: Failure | null = null; // why hello, snapshot or subscribe failed (phase "failed")
  broker: ResultHello | null = null;
  checking = false;
  lastCheck: { readonly ms: number; readonly at: number } | null = null;
  restarting = false;
  cursor = 0;
  health: Health | null = null;
  healthRun = -1; // the run whose snapshot last set `health`
  profiles: Profiles | null = null;
  preferences: Preferences | null = null;
  agents = new Map<string, Agent>();
  enrollments = new Map<string, Enrollment>(); // the enrollments that wait for the owner
  accounts = new Map<string, Account>();
  recipientSets = new Map<string, RecipientSet>();
  readonly grants = new Map<string, Grant>(); // live ones, and those that ended while this window was open
  readonly requests = new Map<string, Entry>();
  readonly activity: { readonly sequence: number; readonly text: string }[] = [];
  private readonly pending = new Map<string, Pending>();
  private readonly commandIds = new Map<string, string>(); // method + params of an unanswered command -> its id
  private readonly deps: Deps;

  constructor(deps: Deps) {
    this.deps = deps;
  }

  get names(): Names {
    return { agents: this.agents, accounts: this.accounts, zone: this.deps.zone };
  }

  get zone(): string {
    return this.deps.zone;
  }

  get ready(): boolean {
    return this.connection?.name === "connected" && this.phase === "live";
  }

  // Why a command cannot be sent now, in words; null when it can.
  get notReadyText(): string | null {
    if (this.ready) return null;
    const c = this.connection;
    if (c === null) return "Not available until this window has reached the broker.";
    if (c.name !== "connected") return "Not available while the broker is not running.";
    if (this.phase === "failed") {
      return this.syncFailure?.code === "store_unavailable"
        ? "Not available while the broker cannot open its store."
        : "Not available: the broker's current state could not be read.";
    }
    return "Not available until the broker's current state has been read.";
  }

  start(): void {
    this.deps.host.attach((m) => this.receive(m)).catch((e: unknown) => {
      this.problem = `This window could not reach the app: ${String(e)}`;
      this.deps.changed();
    });
  }

  receive(m: HostMessage): void {
    if (m.kind === "state") this.onState(m.run, m.state);
    else if (m.run === this.run) this.onFrame(m.frame); // a frame of an earlier run is dropped
    this.deps.changed();
  }

  // ---- For the screens ----

  changed(): void {
    this.deps.changed();
  }

  announce(text: string): void {
    this.deps.announce(text);
  }

  newId(): string {
    return this.deps.newId();
  }

  // A method that changes nothing. `fail` gets code null when no answer came.
  query<M extends OwnerQuery>(
    method: M,
    params: OwnerMethods[M]["params"],
    ok: (result: OwnerMethods[M]["result"]) => void,
    fail: (f: Failure) => void,
  ): void {
    this.call(method, params, ok, fail);
  }

  // A command. The same method and params, sent again after a send that got no answer, reuse that
  // send's command id, so that the broker's deduplication makes the retry safe; every other send
  // gets a new id. Returns the id used.
  command<M extends OwnerCommand>(
    method: M,
    params: OwnerMethods[M]["params"],
    ok: (result: OwnerMethods[M]["result"]) => void,
    fail: (f: Failure) => void,
  ): string {
    const key = `${method}\n${JSON.stringify(params)}`;
    const id = this.commandIds.get(key) ?? this.deps.newId();
    this.commandIds.set(key, id);
    const answered = (): void => {
      if (this.commandIds.get(key) === id) this.commandIds.delete(key);
    };
    this.call(method, params, (result) => {
      answered();
      ok(result);
    }, (f) => {
      if (f.code !== null) answered();
      fail(f);
    }, id);
    return id;
  }

  // ---- What the owner does ----

  checkConnection(): boolean {
    if (this.connection?.name !== "connected" || this.checking) return false;
    this.hello(null);
    this.deps.changed();
    return true;
  }

  restart(): boolean {
    if (this.connection?.name !== "disconnected" || this.restarting) return false;
    this.restarting = true;
    this.problem = null;
    this.deps.host.restart().catch((e: unknown) => {
      this.restarting = false;
      this.problem = `The broker could not be restarted: ${String(e)}`;
      this.deps.announce(this.problem);
      this.deps.changed();
    });
    this.deps.changed();
    return true;
  }

  // After a failed sync: hello, snapshot and subscribe again.
  retrySync(): boolean {
    if (this.connection?.name !== "connected" || this.phase !== "failed") return false;
    this.startSync();
    this.deps.changed();
    return true;
  }

  // While live: a new snapshot and subscribe (after a `conflict`, to read the current state).
  refresh(): boolean {
    if (!this.ready) return false;
    this.snapshot();
    this.deps.changed();
    return true;
  }

  // "session" is not taken while the request's agent has no live session (the broker would refuse
  // it); the views render again, so that a radio the owner pressed shows the choice that holds.
  chooseDuration(requestId: string, kind: DurationKind): void {
    const e = this.requests.get(requestId);
    if (e === undefined || this.cardStatus(e).kind === "closed") return;
    if (kind !== "session" || this.sessionAvailable(e)) {
      e.duration = kind;
      e.durationChosen = true;
    }
    this.deps.changed();
  }

  // Opens or closes a request's details. Open details are read when missing, of an earlier run or of
  // an older revision, also for a request that is no longer waiting (the broker keeps it).
  openDetails(requestId: string, open: boolean): void {
    const e = this.requests.get(requestId);
    if (e === undefined || e.detailsOpen === open) return;
    e.detailsOpen = open;
    if (this.ready) this.readWanted(e);
    this.deps.changed();
  }

  acknowledgeChange(requestId: string): void {
    const e = this.requests.get(requestId);
    if (e === undefined || !e.changed) return;
    e.changed = false;
    this.deps.changed();
  }

  // Escape or leaving the card. It never decides and sends nothing.
  dismiss(requestId: string): void {
    if (this.requests.get(requestId)?.request.status.state === "pending_approval") {
      this.deps.announce("Nothing was decided. The request is still waiting for you.");
    }
  }

  // One decision is one command. The same params, after no answer, reuse the same command id.
  decide(requestId: string, decision: "approve" | "deny"): boolean {
    const e = this.requests.get(requestId);
    if (e === undefined || e.details === null || this.cardStatus(e).kind !== "ready") return false;
    const r = e.details.request;
    const base = { request_id: r.request_id, request_digest: r.request_digest, expected_revision: r.revision };
    const params: ParamsDecideRequest = decision === "approve"
      ? { ...base, decision, selected: null, duration: { kind: this.duration(e) } }
      : { ...base, decision, selected: null, duration: null };
    const command = { id: "", inFlight: true };
    e.command = command;
    e.note = null;
    command.id = this.command("decide_request", params, (result) => {
      command.inFlight = false;
      e.approval = result.approval;
      if (result.request.revision >= e.request.revision) e.request = result.request;
      e.changed = false;
      this.deps.announce(this.decisionText(e) ?? "Your decision was recorded.");
    }, (f) => {
      command.inFlight = false;
      if (f.code === "stale_request") {
        e.note = "The request changed after you saw it. It is read again before you can decide.";
        if (e.request.status.state === "pending_approval") this.readDetails(e);
      } else if (f.code === null) {
        e.note = `Your decision was not answered: ${f.text}. It is not known whether it was recorded. Deciding the same way again is safe.`;
      } else {
        e.note = `The broker did not accept your decision: ${f.text}`;
      }
      this.deps.announce(e.note);
    });
    this.deps.changed();
    return true;
  }

  // ---- What the views ask ----

  cardStatus(e: Entry): CardStatus {
    if (e.details === null || e.details.card === null) return { kind: "none" };
    const decided = this.decisionText(e);
    if (decided !== null) return { kind: "closed", text: decided };
    if (!e.available) return { kind: "closed", text: "The broker no longer lists this request, so it cannot be decided here." };
    if (e.request.status.state !== "pending_approval") {
      return { kind: "closed", text: `This request is no longer waiting for you: ${STATE_TEXT[e.request.status.state]}.` };
    }
    if (this.connection?.name !== "connected") {
      return { kind: "waiting", text: "The broker is not running. When it runs again, this request is read again before you can decide." };
    }
    if (e.command?.inFlight === true) return { kind: "waiting", text: "Sending your decision…" };
    if (this.phase === "failed") {
      return { kind: "waiting", text: "The broker's current state could not be read. This request is read again when it can be." };
    }
    const d = e.details.request;
    if (e.reading || e.detailsRun !== this.run || d.revision !== e.request.revision || d.request_digest !== e.request.request_digest) {
      return { kind: "waiting", text: "Reading this request again before you can decide…" };
    }
    if (e.changed) {
      return { kind: "changed", text: "This request changed since you last saw it. The card now shows the new version. Read it before you decide." };
    }
    return { kind: "ready" };
  }

  // Whether the request's agent has a live session to tie a "session" permission to.
  sessionAvailable(e: Entry): boolean {
    return (this.agents.get(e.request.agent_id)?.session ?? null) !== null;
  }

  // The duration a decision sends: the owner's choice, but "session" falls back to "one_request"
  // while the agent has no live session. The choice itself is kept.
  duration(e: Entry): DurationKind {
    return e.duration === "session" && !this.sessionAvailable(e) ? "one_request" : e.duration;
  }

  decisionText(e: Entry): string | null {
    const a = e.approval;
    if (a === null) return null;
    return a.decision === "approve" ? ALLOWED_TEXT[a.duration.kind] : "You denied this.";
  }

  // ---- The host and the broker ----

  private onState(run: number, state: HostState): void {
    if (run < this.run) return;
    const before = this.connection;
    const newRun = run !== this.run;
    this.run = run;
    this.connection = state;
    this.restarting = false;
    if (state.name === "connected") {
      if (newRun || before?.name !== "connected") {
        this.problem = null;
        if (before !== null) this.deps.announce("The broker is running.");
        this.startSync();
      }
      return;
    }
    this.phase = "waiting";
    this.failInFlight();
    if (before?.name !== state.name) {
      this.deps.announce(state.name === "disconnected"
        ? "The broker stopped. Nothing can be decided until you restart it."
        : "The broker was not started. See the Status screen.");
    }
  }

  // Every request still waiting for an answer fails here, in this window; nothing is resent.
  private failInFlight(): void {
    const pending = [...this.pending.values()];
    this.pending.clear();
    for (const p of pending) p.fail(noAnswer("the broker stopped before it answered"));
  }

  private onFrame(frame: OwnerResponse | OwnerNotice): void {
    if ("sequence" in frame) return this.onNotice(frame);
    if (frame.id === null) {
      this.problem = `The broker could not read something this app sent: ${"error" in frame ? frame.error.message : ""}`;
      return;
    }
    const p = this.pending.get(frame.id);
    if (p === undefined) return; // not ours, or already failed
    this.pending.delete(frame.id);
    if ("error" in frame) p.fail({ text: frame.error.message, code: frame.error.code, error: frame.error });
    else p.ok(frame.result);
  }

  private onNotice(n: OwnerNotice): void {
    if (this.phase !== "live" || n.sequence <= this.cursor) return;
    if (n.change.kind === "snapshot_required" || n.sequence !== this.cursor + 1) {
      this.log(n.sequence, n.change.kind === "snapshot_required" ? changeText(n.change, this.names)
        : `Notices ${this.cursor + 1} to ${n.sequence - 1} were missed; this app reads everything again`);
      this.snapshot();
      return;
    }
    this.cursor = n.sequence;
    const c = n.change;
    switch (c.kind) {
      case "account": this.accounts.set(c.account.account_id, c.account); break;
      case "agent": this.agents.set(c.agent.agent_id, c.agent); break;
      case "agent_removed": this.agents.delete(c.agent_id); break;
      case "enrollment": this.enrollments.set(c.enrollment.enrollment_id, c.enrollment); break;
      case "enrollment_closed": this.enrollments.delete(c.enrollment_id); break;
      case "profiles": this.profiles = c.profiles; break;
      case "preferences": this.preferences = c.preferences; break;
      case "recipient_set": this.recipientSets.set(c.recipient_set.recipient_set_id, c.recipient_set); break;
      case "recipient_set_deleted": this.recipientSets.delete(c.recipient_set_id); break;
      case "grant": this.applyGrant(c.grant); break;
      case "health": this.applyHealth(c.health); break;
      case "request": this.applyRequest(c.request, true); break;
    }
    this.log(n.sequence, changeText(c, this.names));
  }

  private log(sequence: number, text: string): void {
    this.activity.push({ sequence, text });
    if (this.activity.length > ACTIVITY_LIMIT) this.activity.shift();
  }

  private call<M extends OwnerMethod>(
    method: M,
    params: OwnerMethods[M]["params"],
    ok: (result: OwnerMethods[M]["result"]) => void,
    fail: (f: Failure) => void,
    id: string = this.deps.newId(),
  ): void {
    const frame = { protocol: PROTOCOL, id, method, params } as OwnerRequest;
    const entry: Pending = { ok: ok as (result: unknown) => void, fail };
    this.pending.set(id, entry);
    this.deps.host.send(JSON.stringify(frame)).catch((refusal: unknown) => {
      if (this.pending.get(id) !== entry) return;
      this.pending.delete(id);
      fail(noAnswer(refusalText(refusal)));
      this.deps.changed();
    });
  }

  private hello(then: (() => void) | null): void {
    const sent = this.deps.now();
    this.checking = true;
    this.call("hello", { client: CLIENT, schema_version: SCHEMA_VERSION }, (result) => {
      const at = this.deps.now();
      this.checking = false;
      this.broker = result;
      this.lastCheck = { ms: at - sent, at };
      then?.();
    }, (f) => {
      this.checking = false;
      if (then !== null) this.syncFailed(f);
      else this.problem = `The broker did not answer the connection check: ${f.text}`;
    });
  }

  private startSync(): void {
    this.syncFailure = null;
    this.phase = "hello";
    this.hello(() => this.snapshot());
  }

  private snapshot(): void {
    this.phase = "snapshot";
    this.query("get_snapshot", {}, (s) => {
      this.applySnapshot(s);
      this.subscribe();
    }, (f) => this.syncFailed(f));
  }

  private subscribe(): void {
    this.phase = "subscribe";
    this.query("subscribe", { cursor: this.cursor }, () => {
      this.phase = "live";
      this.syncFailure = null;
    }, (f) => (f.code === "snapshot_required" ? this.snapshot() : this.syncFailed(f)));
  }

  // A failure without an answer happens only when the broker stopped, which the state already says.
  private syncFailed(f: Failure): void {
    if (this.connection?.name !== "connected") return;
    this.phase = "failed";
    this.syncFailure = f;
    if (f.error?.code === "store_unavailable") {
      this.deps.announce(`${STORE_PROBLEM[f.error.detail.reason]} Nothing is read or written until the store opens.`);
    }
  }

  private applySnapshot(s: Snapshot): void {
    this.cursor = s.cursor;
    this.agents = new Map(s.agents.map((a) => [a.agent_id, a]));
    this.enrollments = new Map(s.enrollments.map((e) => [e.enrollment_id, e]));
    this.accounts = new Map(s.accounts.map((a) => [a.account_id, a]));
    this.recipientSets = new Map(s.recipient_sets.map((r) => [r.recipient_set_id, r]));
    this.profiles = s.profiles;
    this.preferences = s.preferences;
    if (this.healthRun === this.run) this.applyHealth(s.health);
    else this.health = s.health; // the first snapshot of a run: the strip shows it, nothing is said
    this.healthRun = this.run;
    // The snapshot lists the live grants. A grant held as live that it does not list ended unseen.
    const live = new Set(s.grants.map((g) => g.grant_id));
    for (const [id, g] of this.grants) if (g.ended === null && !live.has(id)) this.grants.delete(id);
    for (const g of s.grants) this.applyGrant(g);
    const listed = new Set(s.requests.map((r) => r.request_id));
    for (const e of this.requests.values()) {
      if (!listed.has(e.request.request_id) && !TERMINAL[e.request.status.state]) e.available = false;
    }
    for (const r of s.requests) this.applyRequest(r, false);
    // Open details of a request the snapshot does not list (one that ended) are read in this run too.
    // A reason read that failed is tried again here.
    for (const e of this.requests.values()) this.readWanted(e);
  }

  private applyHealth(h: Health): void {
    const before = this.health;
    if (before !== null && (before.prover !== h.prover || before.prover_problem !== h.prover_problem)) this.deps.announce(proverAnnouncement(h));
    this.health = h;
  }

  private applyGrant(g: Grant): void {
    const held = this.grants.get(g.grant_id);
    if (held !== undefined && held.revision > g.revision) return;
    this.grants.set(g.grant_id, g);
    if (g.ended === null) return;
    const ended = [...this.grants.values()].filter((x) => x.ended !== null);
    if (ended.length <= ENDED_GRANT_LIMIT) return;
    ended.sort((a, b) => Date.parse(a.ended?.at ?? "") - Date.parse(b.ended?.at ?? ""));
    for (const x of ended.slice(0, ended.length - ENDED_GRANT_LIMIT)) this.grants.delete(x.grant_id);
  }

  // `live`: the request came in a notice while the page is live, not from a snapshot.
  private applyRequest(r: Request, live: boolean): void {
    let e = this.requests.get(r.request_id);
    const checking = CHECKING_STATES.includes(r.status.state);
    // "Received, checking" is first shown now, unless the same reception was already shown as checking.
    const firstShown = checking && !(e !== undefined && e.request.received_at === r.received_at
      && CHECKING_STATES.includes(e.request.status.state));
    if (e === undefined) {
      e = {
        request: r, requestRun: this.run, available: true, checkingDelayMs: null, details: null, detailsRun: -1,
        reading: false, undecided: null, reasonReading: false, detailsOpen: false, changed: false, duration: "one_request", durationChosen: false, command: null, approval: null,
        note: null,
      };
      this.requests.set(r.request_id, e);
    } else {
      if (e.requestRun === this.run && r.revision < e.request.revision) return; // older than what is shown
      if (r.received_at !== e.request.received_at) e.checkingDelayMs = null;
      e.request = r;
      e.requestRun = this.run;
      e.available = true;
    }
    if (r.status.state === "pending_approval") e.approval = null; // pending again: an earlier decision no longer holds
    // Measured only when the text first appears for a request that came in a notice; a request first
    // seen in a snapshot was received earlier, and a negative difference means the clocks disagree.
    if (firstShown && e.checkingDelayMs === null && live) {
      const delay = this.deps.now() - Date.parse(r.received_at);
      if (delay >= 0) e.checkingDelayMs = delay;
    }
    this.readWanted(e);
    if (TERMINAL[r.status.state]) this.dropEndedRequests();
  }

  // At most ENDED_REQUEST_LIMIT requests in a terminal state are kept, the oldest change dropped first.
  private dropEndedRequests(): void {
    const ended = [...this.requests.values()].filter((e) => TERMINAL[e.request.status.state]);
    if (ended.length <= ENDED_REQUEST_LIMIT) return;
    ended.sort((a, b) => Date.parse(a.request.updated_at) - Date.parse(b.request.updated_at));
    for (const e of ended.slice(0, ended.length - ENDED_REQUEST_LIMIT)) this.requests.delete(e.request.request_id);
  }

  // A request is read (again) when this run has no details of its current revision and digest, and it
  // is pending (for its card) or the owner has its details open.
  private needsRead(e: Entry): boolean {
    const d = e.details?.request;
    const wanted = (e.request.status.state === "pending_approval" && e.available) || e.detailsOpen;
    return wanted && !e.reading
      && (e.detailsRun !== this.run || d?.revision !== e.request.revision || d?.request_digest !== e.request.request_digest);
  }

  // An undecided request's recorded reason is read when it is not known yet and no read is in flight.
  private needsReason(e: Entry): boolean {
    return e.request.status.state === "undecided" && e.available && e.undecided === null && !e.reading && !e.reasonReading;
  }

  // A full read when one is wanted (its answer carries the reason too), otherwise the reason alone when that is wanted.
  private readWanted(e: Entry): void {
    if (this.needsRead(e)) this.readDetails(e);
    else if (this.needsReason(e)) this.readReason(e);
  }

  private readDetails(e: Entry): void {
    e.reading = true;
    this.query("get_request_details", { request_id: e.request.request_id, redaction: "full" }, (d) => {
      e.reading = false;
      const before = e.details;
      if (before !== null && before.card !== null && before.request.request_digest !== d.request.request_digest) e.changed = true;
      if (!e.durationChosen && d.card !== null) e.duration = d.card.proposed_duration.kind;
      e.details = d;
      e.detailsRun = this.run;
      if (d.request.revision >= e.request.revision) e.request = d.request;
      if (TERMINAL[e.request.status.state]) e.available = true; // it ended; the broker lists only open ones
      e.approval = d.approval; // the broker is the authority on whether it is decided
      if (d.request.status.state === "undecided") e.undecided = { reason: d.undecided_reason };
      if (this.needsRead(e)) return this.readDetails(e); // a newer revision arrived meanwhile
      if (this.needsReason(e)) this.readReason(e); // it became undecided meanwhile
      if (d.card === null || d.request.status.state !== "pending_approval") return;
      const what = `${OPERATION[d.request.tool]}, from ${agentName(this.names, d.request.agent_id)}`;
      if (before === null || before.card === null) this.deps.announce(`A request needs your decision: ${what}.`);
      else if (e.changed) this.deps.announce(`A request changed and needs your decision again: ${what}.`);
      else if (this.cardStatus(e).kind === "ready") this.deps.announce(`You can decide again: ${what}.`);
    }, (f) => {
      e.reading = false;
      if (f.code === "not_found") e.available = false;
      else e.note = `This request could not be read: ${f.text}`;
    });
  }

  // The recorded reason alone. A redacted read: the full one writes an audit row in the broker and
  // brings event content into the page. It touches nothing else of the entry, so the card and the open
  // details keep what the full read gave. A failure is tried again at the next snapshot only.
  private readReason(e: Entry): void {
    e.reasonReading = true;
    this.query("get_request_details", { request_id: e.request.request_id, redaction: "redacted" }, (d) => {
      e.reasonReading = false;
      if (d.request.status.state === "undecided") e.undecided = { reason: d.undecided_reason };
    }, () => {
      e.reasonReading = false;
    });
  }
}
