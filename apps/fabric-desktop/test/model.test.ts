// The model against a fake host: no browser, no DOM. Every frame the model sends, and every
// frame the tests feed it, is judged by the contract's own TypeScript checker.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { REQUEST_STATES } from "../../../contracts/desktop/v1/typescript/src/contract.ts";
import type {
  Agent, ErrorCode, OwnerMethod, OwnerNotice, OwnerResponse, Request, RequestDetails, RequestState, Snapshot,
} from "../../../contracts/desktop/v1/typescript/src/contract.ts";
import { CONTRACT_ROOT, judge, loadContract } from "../../../contracts/desktop/v1/typescript/src/judge.ts";
import type { Host, HostMessage, HostState, SendRefusal } from "../ui/src/host.ts";
import { Model } from "../ui/src/model.ts";
import { STATE_TEXT, TERMINAL, bannerText } from "../ui/src/text.ts";

const contract = loadContract();
const ENTRY = "owner-channel.schema.json#/$defs/";
const encoder = new TextEncoder();

function assertValid(entry: string, frame: unknown): void {
  const verdict = judge(contract, ENTRY + entry, encoder.encode(JSON.stringify(frame)));
  assert.equal(verdict.verdict, "accept", `${entry} must be valid: ${JSON.stringify(frame)} ${JSON.stringify(verdict)}`);
}

const fixture = (name: string): any =>
  JSON.parse(readFileSync(`${CONTRACT_ROOT}fixtures/messages/${name}`, "utf8")).value;

// ---- Test data, all valid by the contract (the helpers check it) ----

const RECEIVED_AT = "2026-11-20T16:00:00.000Z";
const RECEIVED_MS = Date.parse(RECEIVED_AT);
const DIGEST_A = `sha256:${"a".repeat(64)}` as const;
const DIGEST_B = `sha256:${"b".repeat(64)}` as const;

const AGENT: Agent = {
  agent_id: "spike-agent", revision: 1, registered_name: "Synthetic agent (spike)",
  destination: { kind: "local", endpoint: "inside the broker spike" }, paused: false,
  session: { session_id: "spike-session", started_at: RECEIVED_AT, last_active_at: RECEIVED_AT }, last_tool_call_at: null,
};

function request(state: RequestState, revision: number, digest: `sha256:${string}` = DIGEST_A, id = "spike-request-1"): Request {
  const status = state === "pending_approval" ? { state, approval_expires_at: "2026-11-21T16:00:00.000Z" } : { state } as Request["status"];
  return {
    request_id: id, revision, agent_id: "spike-agent", tool: "fabric_calendar_propose_create", account_id: "spike-account",
    received_at: RECEIVED_AT, updated_at: RECEIVED_AT, request_digest: digest, status,
  };
}

function details(r: Request): RequestDetails {
  return {
    request: r, redaction: "full",
    private: {
      action: {
        tool: "fabric_calendar_propose_create",
        arguments: {
          account_id: "spike-account", calendar_id: "spike-calendar",
          event: {
            title: "Planning", description: null, location: null,
            start: { date_time: "2026-11-20T10:00:00-06:00", zone: "America/Chicago" },
            end: { date_time: "2026-11-20T11:00:00-06:00", zone: "America/Chicago" },
          },
          task: "Block time to plan next week.",
        },
      },
      before: null, warnings: [],
    },
    card: {
      scope: { calendars: [{ account_id: "spike-account", calendar_id: "spike-calendar" }], window: null, fields: null, recipients: null, send_updates: null },
      proposed_duration: { kind: "one_request" },
      insufficiency: { rule: "spike_no_rule_theory", explanation: "Synthetic: no rule theory.", proof_id: "spike-no-proof" },
      expires_at: "2026-11-21T16:00:00.000Z",
    },
    approval: null, proof_ids: [], authorization: null, attempts: [], provider_event: null,
  };
}

function snapshot(cursor: number, requests: Request[] = []): Snapshot {
  const first: Snapshot = fixture("accept-owner-response-get-snapshot-first-run.json").result;
  return { ...first, cursor, agents: [AGENT], requests };
}

// ---- The fake host ----

class FakeHost implements Host {
  listener: ((m: HostMessage) => void) | null = null;
  readonly raw: string[] = [];
  refusal: SendRefusal | null = null;
  restarts = 0;
  run = 0;

  async attach(listener: (m: HostMessage) => void): Promise<void> {
    this.listener = listener;
  }
  async send(frame: string): Promise<void> {
    this.raw.push(frame);
    if (this.refusal !== null) throw this.refusal;
  }
  async restart(): Promise<void> {
    this.restarts += 1;
  }

  get sent(): any[] {
    return this.raw.map((f) => JSON.parse(f));
  }
  methods(): string[] {
    return this.sent.map((f) => f.method);
  }
  last(method: OwnerMethod): any {
    const frame = this.sent.filter((f) => f.method === method).at(-1);
    assert.ok(frame !== undefined, `the model sent no ${method}`);
    return frame;
  }
  state(state: HostState, run = this.run): void {
    this.run = run;
    this.listener?.({ kind: "state", run, state });
  }
  frame(frame: OwnerResponse | OwnerNotice, run = this.run): void {
    assertValid("sequence" in frame ? "owner_notice" : "owner_response", frame);
    this.listener?.({ kind: "frame", run, frame });
  }
  reply(method: OwnerMethod, result: unknown, run = this.run): void {
    this.frame({ protocol: "fabric-desktop/1", id: this.last(method).id, method, result } as OwnerResponse, run);
  }
  error(method: OwnerMethod, code: ErrorCode, run = this.run): void {
    const errors: Partial<Record<ErrorCode, any>> = {
      snapshot_required: { retryable: false, recovery: "refresh_snapshot", detail: null },
      stale_request: { retryable: false, recovery: "review_request_again", detail: null },
      not_found: { retryable: false, recovery: "none", detail: null },
    };
    const error = { code, message: "Refused by the test.", correlation_id: null, ...errors[code] };
    this.frame({ protocol: "fabric-desktop/1", id: this.last(method).id, method, error } as OwnerResponse, run);
  }
  notice(sequence: number, change: OwnerNotice["change"]): void {
    this.frame({ protocol: "fabric-desktop/1", sequence, change });
  }
}

const HELLO_RESULT = fixture("accept-owner-response-hello.json").result;
const flush = (): Promise<void> => new Promise((resolve) => setImmediate(resolve));

function setup() {
  const host = new FakeHost();
  const clock = { now: RECEIVED_MS };
  let ids = 0;
  const announced: string[] = [];
  const model = new Model({
    host, now: () => clock.now, newId: () => `id-${++ids}`, zone: "America/Chicago",
    changed: () => {}, announce: (text) => void announced.push(text),
  });
  model.start();
  return { host, model, clock, announced };
}

// Runs hello, snapshot and subscribe for a new run.
function connect(host: FakeHost, run: number, snap: Snapshot): void {
  host.state({ name: "connected" }, run);
  host.reply("hello", HELLO_RESULT);
  host.reply("get_snapshot", snap);
  host.reply("subscribe", { cursor: snap.cursor });
}

// A request waiting with its card read; returns the model ready to decide.
function pendingCard() {
  const s = setup();
  connect(s.host, 1, snapshot(0));
  s.host.notice(1, { kind: "request", request: request("pending_approval", 4) });
  s.host.reply("get_request_details", details(request("pending_approval", 4)));
  return s;
}

const entry = (model: Model, id = "spike-request-1") => {
  const e = model.requests.get(id);
  assert.ok(e !== undefined);
  return e;
};

// ---- Tests ----

test("on connected: hello, then get_snapshot, then subscribe with the snapshot's cursor", () => {
  const { host, model } = setup();
  host.state({ name: "connected" }, 1);
  assert.deepEqual(host.methods(), ["hello"]);
  assert.deepEqual(host.last("hello").params, { client: { name: "Fabric Calendar", build: "CD-W02 webview" }, schema_version: 1 });
  host.reply("hello", HELLO_RESULT);
  assert.deepEqual(host.methods(), ["hello", "get_snapshot"]);
  host.reply("get_snapshot", snapshot(7));
  assert.deepEqual(host.methods(), ["hello", "get_snapshot", "subscribe"]);
  assert.deepEqual(host.last("subscribe").params, { cursor: 7 });
  assert.equal(model.phase, "subscribe");
  host.reply("subscribe", { cursor: 7 });
  assert.equal(model.phase, "live");
  assert.equal(model.broker?.broker.name, "Fabric broker");
});

test("notices are applied in order", () => {
  const { host, model } = setup();
  connect(host, 1, snapshot(0));
  host.notice(1, { kind: "agent", agent: AGENT });
  host.notice(2, { kind: "account", account: fixture("accept-owner-notice-account-connected.json").change.account });
  host.notice(3, { kind: "request", request: request("received", 1) });
  host.notice(4, { kind: "request", request: request("validating", 2) });
  assert.equal(model.cursor, 4);
  assert.deepEqual(model.activity.map((a) => a.sequence), [1, 2, 3, 4]);
  assert.equal(entry(model).request.status.state, "validating");
  assert.equal(model.agents.get("spike-agent")?.registered_name, "Synthetic agent (spike)");
  host.notice(4, { kind: "request", request: request("received", 1) }); // a repeat is ignored
  assert.equal(entry(model).request.status.state, "validating");
});

test("a sequence gap leads to a new snapshot and subscribe", () => {
  const { host, model } = setup();
  connect(host, 1, snapshot(0));
  host.notice(1, { kind: "agent", agent: AGENT });
  host.notice(3, { kind: "request", request: request("validating", 2) });
  assert.deepEqual(host.methods().slice(-1), ["get_snapshot"]);
  assert.equal(model.requests.size, 0, "the notice after the gap is not applied");
  host.reply("get_snapshot", snapshot(3, [request("validating", 2)]));
  assert.deepEqual(host.last("subscribe").params, { cursor: 3 });
  host.reply("subscribe", { cursor: 3 });
  assert.equal(model.phase, "live");
  assert.equal(entry(model).request.status.state, "validating");
});

test("a snapshot_required change and a snapshot_required error each lead to a new snapshot", () => {
  const { host } = setup();
  connect(host, 1, snapshot(0));
  host.notice(1, { kind: "snapshot_required" });
  assert.deepEqual(host.methods().slice(-1), ["get_snapshot"]);
  host.reply("get_snapshot", snapshot(5));
  host.error("subscribe", "snapshot_required");
  assert.deepEqual(host.methods().slice(-2), ["subscribe", "get_snapshot"]);
  host.reply("get_snapshot", snapshot(6));
  assert.deepEqual(host.last("subscribe").params, { cursor: 6 });
});

test("a frame from an older run is dropped", () => {
  const { host, model } = setup();
  host.state({ name: "connected" }, 1);
  host.state({ name: "disconnected", reason: "exited", detail: "exit code 0" }, 1);
  host.state({ name: "connected" }, 2);
  const hello2 = host.last("hello");
  host.frame({ protocol: "fabric-desktop/1", id: hello2.id, method: "hello", result: HELLO_RESULT }, 1);
  assert.equal(model.broker, null);
  assert.equal(model.phase, "hello");
  host.reply("hello", HELLO_RESULT, 2);
  assert.equal(model.phase, "snapshot");
});

test("'Received, checking' and how long after received_at it was first shown", () => {
  const { host, model, clock } = setup();
  connect(host, 1, snapshot(0));
  clock.now = RECEIVED_MS + 137;
  host.notice(1, { kind: "request", request: request("received", 1) });
  clock.now = RECEIVED_MS + 400;
  host.notice(2, { kind: "request", request: request("validating", 2) });
  assert.equal(STATE_TEXT.received, "Received, checking");
  assert.equal(STATE_TEXT.validating, "Received, checking");
  assert.equal(STATE_TEXT.proving_classification, "Received, checking");
  assert.equal(entry(model).checkingDelayMs, 137, "measured once, when the text was first shown");
});

test("all 23 states have a text, and TERMINAL equals the lifecycle data", () => {
  assert.equal(REQUEST_STATES.length, 23);
  for (const s of REQUEST_STATES) assert.ok(STATE_TEXT[s].length > 0, s);
  assert.deepEqual(Object.keys(STATE_TEXT).sort(), [...REQUEST_STATES].sort());
  const lifecycle = JSON.parse(readFileSync(`${CONTRACT_ROOT}data/request-lifecycle.json`, "utf8"));
  const terminal = Object.fromEntries(lifecycle.states.map((s: any) => [s.state, s.terminal]));
  assert.deepEqual(TERMINAL, terminal);
});

test("a card appears only after get_request_details answered", () => {
  const { host, model, announced } = setup();
  connect(host, 1, snapshot(0));
  host.notice(1, { kind: "agent", agent: AGENT });
  host.notice(2, { kind: "request", request: request("pending_approval", 4) });
  assert.deepEqual(host.last("get_request_details").params, { request_id: "spike-request-1", redaction: "full" });
  assert.equal(model.cardStatus(entry(model)).kind, "none");
  host.reply("get_request_details", details(request("pending_approval", 4)));
  assert.equal(model.cardStatus(entry(model)).kind, "ready");
  assert.equal(entry(model).duration, "one_request", "the proposed duration is preselected");
  assert.match(announced.at(-1) ?? "", /needs your decision: Create an event, from Synthetic agent \(spike\)/);
});

test("a decision is one command; while it is awaited the buttons are disabled; twice sends once", () => {
  const { host, model, announced } = pendingCard();
  const before = host.raw.length;
  assert.equal(model.decide("spike-request-1", "approve"), true);
  assert.equal(model.decide("spike-request-1", "approve"), false);
  assert.equal(model.decide("spike-request-1", "deny"), false);
  assert.equal(host.raw.length, before + 1);
  const frame = host.last("decide_request");
  assert.deepEqual(frame.params, {
    request_id: "spike-request-1", request_digest: DIGEST_A, expected_revision: 4, decision: "approve", selected: null,
    duration: { kind: "one_request" },
  });
  const status = model.cardStatus(entry(model));
  assert.equal(status.kind, "waiting");
  assert.ok(status.kind === "waiting" && status.text.length > 0, "the disabled buttons are explained");
  const approved = request("approved", 5);
  const approval = {
    decision: "approve", command_id: frame.id, decided_at: RECEIVED_AT, approval_digest: DIGEST_B,
    scope: details(approved).card?.scope, duration: { kind: "one_request" }, effective_request_digest: DIGEST_A, grant_id: "spike-grant-1",
  };
  host.reply("decide_request", { commit_sequence: 3, request: approved, approval });
  assert.equal(model.cardStatus(entry(model)).kind, "closed");
  assert.equal(model.decide("spike-request-1", "approve"), false);
  assert.match(announced.at(-1) ?? "", /You allowed this/);
});

test("Escape (dismiss) sends nothing and leaves the request pending", () => {
  const { host, model } = pendingCard();
  const before = host.raw.length;
  model.dismiss("spike-request-1");
  assert.equal(host.raw.length, before);
  assert.equal(entry(model).request.status.state, "pending_approval");
  assert.equal(model.cardStatus(entry(model)).kind, "ready");
});

test("disconnect fails in-flight requests, disables cards and keeps the chosen duration", async () => {
  const { host, model } = pendingCard();
  model.chooseDuration("spike-request-1", "session");
  model.decide("spike-request-1", "approve");
  model.checkConnection();
  host.state({ name: "disconnected", reason: "exited", detail: "killed by signal 9" });
  const e = entry(model);
  assert.equal(e.command?.inFlight, false);
  assert.match(e.note ?? "", /not answered/);
  assert.equal(model.checking, false);
  assert.equal(model.cardStatus(e).kind, "waiting");
  assert.equal(e.duration, "session");
  assert.equal(model.decide("spike-request-1", "approve"), false);
  await flush();
});

test("after a restart a card is enabled only after the re-read in the new run with the same digest", () => {
  const { host, model } = pendingCard();
  model.chooseDuration("spike-request-1", "session");
  model.decide("spike-request-1", "approve");
  const firstId = host.last("decide_request").id;
  host.state({ name: "disconnected", reason: "exited", detail: "killed by signal 9" });
  assert.equal(model.restart(), true);
  assert.equal(model.restart(), false, "one restart at a time");
  assert.equal(host.restarts, 1);
  connect(host, 2, snapshot(1, [request("pending_approval", 4)]));
  const e = entry(model);
  assert.equal(model.cardStatus(e).kind, "waiting", "not before the re-read");
  host.reply("get_request_details", details(request("pending_approval", 4)));
  assert.equal(model.cardStatus(e).kind, "ready");
  assert.equal(e.duration, "session");
  // The same decision reuses the command id; another duration gets a new one.
  model.decide("spike-request-1", "approve");
  assert.equal(host.last("decide_request").id, firstId);
  host.state({ name: "disconnected", reason: "exited", detail: "exit code 1" });
  model.restart();
  connect(host, 3, snapshot(1, [request("pending_approval", 4)]));
  host.reply("get_request_details", details(request("pending_approval", 4)));
  model.chooseDuration("spike-request-1", "until_revoked");
  model.decide("spike-request-1", "approve");
  assert.notEqual(host.last("decide_request").id, firstId);
});

test("a changed digest after a restart shows the new card and asks the owner to read it", () => {
  const { host, model } = pendingCard();
  host.state({ name: "disconnected", reason: "exited", detail: "exit code 1" });
  model.restart();
  connect(host, 2, snapshot(1, [request("pending_approval", 4, DIGEST_B)]));
  host.reply("get_request_details", details(request("pending_approval", 4, DIGEST_B)));
  const e = entry(model);
  assert.equal(model.cardStatus(e).kind, "changed");
  assert.equal(model.decide("spike-request-1", "deny"), false);
  model.acknowledgeChange("spike-request-1");
  assert.equal(model.cardStatus(e).kind, "ready");
  model.decide("spike-request-1", "deny");
  assert.equal(host.last("decide_request").params.request_digest, DIGEST_B);
});

test("a request the new broker does not list is not available", () => {
  const { host, model } = pendingCard();
  host.state({ name: "disconnected", reason: "exited", detail: "exit code 1" });
  model.restart();
  connect(host, 2, snapshot(0));
  const e = entry(model);
  assert.equal(e.available, false);
  assert.equal(model.cardStatus(e).kind, "closed");
});

test("an approval of an earlier run does not close a card the broker lists as pending again", () => {
  const { host, model } = pendingCard();
  model.chooseDuration("spike-request-1", "session");
  model.decide("spike-request-1", "approve");
  const frame = host.last("decide_request");
  const approved = request("approved", 5);
  host.reply("decide_request", {
    commit_sequence: 3, request: approved,
    approval: {
      decision: "approve", command_id: frame.id, decided_at: RECEIVED_AT, approval_digest: DIGEST_B,
      scope: details(approved).card?.scope, duration: { kind: "session" }, effective_request_digest: DIGEST_A, grant_id: "spike-grant-1",
    },
  });
  assert.equal(model.cardStatus(entry(model)).kind, "closed");
  host.state({ name: "disconnected", reason: "exited", detail: "killed by signal 9" });
  model.restart();
  connect(host, 2, snapshot(0));
  host.notice(1, { kind: "request", request: request("received", 1) });
  host.notice(2, { kind: "request", request: request("pending_approval", 4) });
  const e = entry(model);
  assert.equal(e.approval, null);
  assert.equal(model.cardStatus(e).kind, "waiting", "not before the re-read");
  host.reply("get_request_details", details(request("pending_approval", 4)));
  assert.equal(model.cardStatus(e).kind, "ready");
  assert.equal(e.duration, "session");
});

test("stale_request reads the request again", () => {
  const { host, model } = pendingCard();
  model.decide("spike-request-1", "deny");
  const reads = host.methods().filter((m) => m === "get_request_details").length;
  host.error("decide_request", "stale_request");
  assert.equal(host.methods().filter((m) => m === "get_request_details").length, reads + 1);
  assert.match(entry(model).note ?? "", /changed after you saw it/);
});

test("a send the host refuses fails that request in words", async () => {
  const { host, model } = pendingCard();
  host.refusal = { kind: "io", detail: "broken pipe" };
  model.decide("spike-request-1", "deny");
  await flush();
  const e = entry(model);
  assert.equal(e.command?.inFlight, false);
  assert.match(e.note ?? "", /broken pipe/);
});

test("conflict and configuration_error offer no restart", () => {
  for (const state of [
    { name: "conflict", detail: "another instance holds the lock" },
    { name: "configuration_error", detail: "the broker is missing" },
  ] as const) {
    const { host, model } = setup();
    host.state(state, 0);
    assert.equal(model.restart(), false);
    assert.equal(host.restarts, 0);
    assert.equal(host.raw.length, 0, "nothing is sent");
    assert.ok(bannerText(state)?.title);
  }
  const { host, model } = setup();
  host.state({ name: "disconnected", reason: "start_failed", detail: "no such file" }, 1);
  assert.equal(model.restart(), true);
  assert.equal(host.restarts, 1);
});

test("every frame the model builds is a valid owner request", async () => {
  const { host, model } = pendingCard();
  model.checkConnection();
  host.reply("hello", HELLO_RESULT);
  model.chooseDuration("spike-request-1", "until_revoked");
  model.decide("spike-request-1", "approve");
  host.state({ name: "disconnected", reason: "exited", detail: "exit code 0" });
  model.restart();
  connect(host, 2, snapshot(0, [request("pending_approval", 4)]));
  host.reply("get_request_details", details(request("pending_approval", 4)));
  model.decide("spike-request-1", "deny");
  assert.deepEqual(new Set(host.methods()), new Set(["hello", "get_snapshot", "subscribe", "get_request_details", "decide_request"]));
  for (const raw of host.raw) {
    const verdict = judge(contract, `${ENTRY}owner_request`, encoder.encode(raw));
    assert.equal(verdict.verdict, "accept", `${raw} -> ${JSON.stringify(verdict)}`);
  }
  await flush();
});
