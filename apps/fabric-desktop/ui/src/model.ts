// All state and logic of the page, with no DOM and no globals: the clock, the id maker and the
// host come in through `Deps`. The view reads the fields below after every `changed()` call.
import type {
  Account, Agent, Approval, DurationKind, ErrorCode, OwnerMethod, OwnerMethods, OwnerNotice, OwnerRequest,
  OwnerResponse, ParamsDecideRequest, Party, Protocol, Request, RequestDetails, ResultHello, SCHEMA_VERSION as SchemaVersion,
  Snapshot,
} from "../../../../contracts/desktop/v1/typescript/src/contract.ts";
import type { Host, HostMessage, HostState, SendRefusal } from "./host.ts";
import { CHECKING_STATES, DURATION_TEXT, OPERATION, STATE_TEXT, TERMINAL, agentName, changeText } from "./text.ts";
import type { Names } from "./text.ts";

const PROTOCOL: Protocol = "fabric-desktop/1";
const SCHEMA_VERSION: typeof SchemaVersion = 1;
export const CLIENT: Party = { name: "Fabric Calendar", build: "CD-W02 webview" };
export const ACTIVITY_LIMIT = 200;

export interface Deps {
  readonly host: Host;
  readonly now: () => number; // Unix time in milliseconds
  readonly newId: () => string; // a frame id that the contract's Id pattern accepts
  readonly zone: string; // the time zone the page shows times in
  readonly changed: () => void; // called after every change
  readonly announce: (text: string) => void; // said by the polite live region
}

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
  changed: boolean; // the digest differs from the card read before; the owner must acknowledge
  duration: DurationKind; // the owner's choice, kept across a disconnect
  durationChosen: boolean;
  readonly commandIds: Map<string, string>; // exact params of a decision -> its command id
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
  readonly fail: (text: string, code: ErrorCode | null) => void;
}

function refusalText(r: unknown): string {
  const s = r as SendRefusal;
  if (s?.kind === "refused") return `this app refused to send it (${s.reason})`;
  if (s?.kind === "not_connected") return "the broker is not running";
  if (s?.kind === "io") return `it could not be sent (${s.detail})`;
  return String(r);
}

export class Model {
  run = 0;
  connection: HostState | null = null;
  phase: Phase = "waiting";
  problem: string | null = null; // a problem of the connection as a whole
  broker: ResultHello | null = null;
  checking = false;
  lastCheck: { readonly ms: number; readonly at: number } | null = null;
  restarting = false;
  cursor = 0;
  agents = new Map<string, Agent>();
  accounts = new Map<string, Account>();
  readonly requests = new Map<string, Entry>();
  readonly activity: { readonly sequence: number; readonly text: string }[] = [];
  private readonly pending = new Map<string, Pending>();
  private readonly deps: Deps;

  constructor(deps: Deps) {
    this.deps = deps;
  }

  get names(): Names {
    return { agents: this.agents, accounts: this.accounts, zone: this.deps.zone };
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

  chooseDuration(requestId: string, kind: DurationKind): void {
    const e = this.requests.get(requestId);
    if (e === undefined || this.cardStatus(e).kind === "closed") return;
    e.duration = kind;
    e.durationChosen = true;
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

  // One decision is one command. The same params reuse the same command id.
  decide(requestId: string, decision: "approve" | "deny"): boolean {
    const e = this.requests.get(requestId);
    if (e === undefined || e.details === null || this.cardStatus(e).kind !== "ready") return false;
    const r = e.details.request;
    const base = { request_id: r.request_id, request_digest: r.request_digest, expected_revision: r.revision };
    const params: ParamsDecideRequest = decision === "approve"
      ? { ...base, decision, selected: null, duration: { kind: e.duration } }
      : { ...base, decision, selected: null, duration: null };
    const key = JSON.stringify(params);
    const id = e.commandIds.get(key) ?? this.deps.newId();
    e.commandIds.set(key, id);
    const command = { id, inFlight: true };
    e.command = command;
    e.note = null;
    this.call("decide_request", params, (result) => {
      command.inFlight = false;
      e.approval = result.approval;
      if (result.request.revision >= e.request.revision) e.request = result.request;
      e.changed = false;
      this.deps.announce(this.decisionText(e) ?? "Your decision was recorded.");
    }, (text, code) => {
      command.inFlight = false;
      if (code === "stale_request") {
        e.note = "The request changed after you saw it. It is read again before you can decide.";
        if (e.request.status.state === "pending_approval") this.readDetails(e);
      } else if (code === null) {
        e.note = `Your decision was not answered: ${text}. It is not known whether it was recorded. Deciding the same way again is safe.`;
      } else {
        e.note = `The broker did not accept your decision: ${text}`;
      }
      this.deps.announce(e.note);
    }, id);
    this.deps.changed();
    return true;
  }

  // ---- What the view asks ----

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
    const d = e.details.request;
    if (e.reading || e.detailsRun !== this.run || d.revision !== e.request.revision || d.request_digest !== e.request.request_digest) {
      return { kind: "waiting", text: "Reading this request again before you can decide…" };
    }
    if (e.changed) {
      return { kind: "changed", text: "This request changed since you last saw it. The card now shows the new version. Read it before you decide." };
    }
    return { kind: "ready" };
  }

  decisionText(e: Entry): string | null {
    const a = e.approval;
    if (a === null) return null;
    return a.decision === "approve" ? `You allowed this. It lasts: ${DURATION_TEXT[a.duration.kind].toLowerCase()}.` : "You denied this.";
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
        : "The broker was not started. See the Broker section.");
    }
  }

  // Every request still waiting for an answer fails here, in this window; nothing is resent.
  private failInFlight(): void {
    const pending = [...this.pending.values()];
    this.pending.clear();
    for (const p of pending) p.fail("the broker stopped before it answered", null);
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
    if ("error" in frame) p.fail(frame.error.message, frame.error.code);
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
    if (c.kind === "account") this.accounts.set(c.account.account_id, c.account);
    else if (c.kind === "agent") this.agents.set(c.agent.agent_id, c.agent);
    else if (c.kind === "request") this.applyRequest(c.request);
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
    fail: (text: string, code: ErrorCode | null) => void,
    id: string = this.deps.newId(),
  ): void {
    const frame = { protocol: PROTOCOL, id, method, params } as OwnerRequest;
    const entry: Pending = { ok: ok as (result: unknown) => void, fail };
    this.pending.set(id, entry);
    this.deps.host.send(JSON.stringify(frame)).catch((refusal: unknown) => {
      if (this.pending.get(id) !== entry) return;
      this.pending.delete(id);
      fail(refusalText(refusal), null);
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
    }, (text) => {
      this.checking = false;
      this.problem = `The broker did not answer the connection check: ${text}`;
      if (then !== null) this.phase = "failed";
    });
  }

  private startSync(): void {
    this.phase = "hello";
    this.hello(() => this.snapshot());
  }

  private snapshot(): void {
    this.phase = "snapshot";
    this.call("get_snapshot", {}, (s) => {
      this.applySnapshot(s);
      this.subscribe();
    }, (text) => this.syncFailed(text));
  }

  private subscribe(): void {
    this.phase = "subscribe";
    this.call("subscribe", { cursor: this.cursor }, () => {
      this.phase = "live";
    }, (text, code) => (code === "snapshot_required" ? this.snapshot() : this.syncFailed(text)));
  }

  private syncFailed(text: string): void {
    this.phase = "failed";
    this.problem = `The current state could not be read from the broker: ${text}`;
  }

  private applySnapshot(s: Snapshot): void {
    this.cursor = s.cursor;
    this.agents = new Map(s.agents.map((a) => [a.agent_id, a]));
    this.accounts = new Map(s.accounts.map((a) => [a.account_id, a]));
    const listed = new Set(s.requests.map((r) => r.request_id));
    for (const e of this.requests.values()) {
      if (!listed.has(e.request.request_id) && !TERMINAL[e.request.status.state]) e.available = false;
    }
    for (const r of s.requests) this.applyRequest(r);
  }

  private applyRequest(r: Request): void {
    let e = this.requests.get(r.request_id);
    if (e === undefined) {
      e = {
        request: r, requestRun: this.run, available: true, checkingDelayMs: null, details: null, detailsRun: -1,
        reading: false, changed: false, duration: "one_request", durationChosen: false, commandIds: new Map(),
        command: null, approval: null, note: null,
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
    if (e.checkingDelayMs === null && CHECKING_STATES.includes(r.status.state)) {
      e.checkingDelayMs = this.deps.now() - Date.parse(r.received_at);
    }
    if (this.needsRead(e)) this.readDetails(e);
  }

  // A pending request is read (again) when this run has no card for its current revision and digest.
  private needsRead(e: Entry): boolean {
    const d = e.details?.request;
    return e.request.status.state === "pending_approval" && !e.reading && e.available
      && (e.detailsRun !== this.run || d?.revision !== e.request.revision || d?.request_digest !== e.request.request_digest);
  }

  private readDetails(e: Entry): void {
    e.reading = true;
    this.call("get_request_details", { request_id: e.request.request_id, redaction: "full" }, (d) => {
      e.reading = false;
      const before = e.details;
      if (before !== null && before.card !== null && before.request.request_digest !== d.request.request_digest) e.changed = true;
      if (!e.durationChosen && d.card !== null) e.duration = d.card.proposed_duration.kind;
      e.details = d;
      e.detailsRun = this.run;
      if (d.request.revision >= e.request.revision) e.request = d.request;
      e.approval = d.approval; // the broker is the authority on whether it is decided
      if (this.needsRead(e)) return this.readDetails(e); // a newer revision arrived meanwhile
      if (d.card === null || d.request.status.state !== "pending_approval") return;
      const what = `${OPERATION[d.request.tool]}, from ${agentName(this.names, d.request.agent_id)}`;
      if (before === null || before.card === null) this.deps.announce(`A request needs your decision: ${what}.`);
      else if (e.changed) this.deps.announce(`A request changed and needs your decision again: ${what}.`);
      else if (this.cardStatus(e).kind === "ready") this.deps.announce(`You can decide again: ${what}.`);
    }, (text, code) => {
      e.reading = false;
      if (code === "not_found") e.available = false;
      else e.note = `This request could not be read: ${text}`;
    });
  }
}
