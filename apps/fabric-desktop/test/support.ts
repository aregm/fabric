// What every test of the page shares: the fake host, the contract's fixtures and checker, valid
// test data, and a model connected to the fake host. No browser, no DOM. Every frame the model
// sends, and every frame a test feeds it, is judged by the contract's own TypeScript checker.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import type {
  Agent, ErrorCode, OwnerError, OwnerMethod, OwnerNotice, OwnerResponse, Request, RequestDetails, RequestState, Snapshot,
} from "../../../contracts/desktop/v1/typescript/src/contract.ts";
import { CONTRACT_ROOT, judge, loadContract } from "../../../contracts/desktop/v1/typescript/src/judge.ts";
import type { Host, HostMessage, HostState, SendRefusal } from "../ui/src/host.ts";
import { Model } from "../ui/src/model.ts";

export const contract = loadContract();
const ENTRY = "owner-channel.schema.json#/$defs/";
const encoder = new TextEncoder();

export function assertValid(entry: string, frame: unknown): void {
  const verdict = judge(contract, ENTRY + entry, encoder.encode(JSON.stringify(frame)));
  assert.equal(verdict.verdict, "accept", `${entry} must be valid: ${JSON.stringify(frame)} ${JSON.stringify(verdict)}`);
}

// The `value` of a message fixture: a whole frame.
export const fixture = (name: string): any =>
  JSON.parse(readFileSync(`${CONTRACT_ROOT}fixtures/messages/${name}`, "utf8")).value;

// ---- Test data, all valid by the contract (the helpers check it) ----

export const RECEIVED_AT = "2026-11-20T16:00:00.000Z";
export const RECEIVED_MS = Date.parse(RECEIVED_AT);
export const DIGEST_A = `sha256:${"a".repeat(64)}` as const;
export const DIGEST_B = `sha256:${"b".repeat(64)}` as const;

export const AGENT: Agent = {
  agent_id: "spike-agent", revision: 1, registered_name: "Synthetic agent (spike)",
  destination: { kind: "local", endpoint: "inside the broker spike" }, client: null, enrollment: "approved", pairing_code: null,
  added_at: RECEIVED_AT, key_proven_at: null, approved_at: RECEIVED_AT, removed_at: null, paused: false,
  session: { session_id: "spike-session", started_at: RECEIVED_AT, last_active_at: RECEIVED_AT }, last_tool_call_at: null,
};

export function request(state: RequestState, revision: number, digest: `sha256:${string}` = DIGEST_A, id = "spike-request-1"): Request {
  const status = state === "pending_approval" ? { state, approval_expires_at: "2026-11-21T16:00:00.000Z" }
    : state === "blocked" ? { state, denial: { rule: "spike_rule", explanation: "Synthetic: blocked.", proof_id: "spike-proof" } }
    : state === "terminal_failure" ? { state, failure: { code: "provider_rejected", explanation: "Synthetic: failed." } }
    : { state } as Request["status"];
  return {
    request_id: id, revision, agent_id: "spike-agent", tool: "fabric_calendar_propose_create", account_id: "spike-account",
    received_at: RECEIVED_AT, updated_at: RECEIVED_AT, request_digest: digest, status: status as Request["status"],
  };
}

export function details(r: Request): RequestDetails {
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
      before: null, after: null, warnings: [],
    },
    card: {
      scope: { calendars: [{ account_id: "spike-account", calendar_id: "spike-calendar" }], window: null, fields: null, recipients: null, send_updates: null },
      proposed_duration: { kind: "one_request" },
      insufficiency: { rule: "spike_no_rule_theory", explanation: "Synthetic: no rule theory.", proof_id: "spike-no-proof" },
      expires_at: "2026-11-21T16:00:00.000Z",
    },
    approval: null, proof_ids: [], authorization: null, attempts: [], provider_event: null, undecided_reason: null,
    owner_resolution_reason: null, notification: null, system_proofs: [], releases: [],
  };
}

// The first run's snapshot with a healthy broker, the test agent and these requests.
export function snapshot(cursor: number, requests: Request[] = []): Snapshot {
  const first: Snapshot = fixture("accept-owner-response-get-snapshot-first-run.json").result;
  return { ...first, cursor, health: { prover: "ready", prover_problem: null, store: "ready", agent_channel: "listening", agent_channel_problem: null }, agents: [AGENT], requests };
}

// The error each code carries by the contract, with the test's message.
const ERRORS: Partial<Record<ErrorCode, Omit<OwnerError, "code" | "message" | "correlation_id">>> = {
  snapshot_required: { retryable: false, recovery: "refresh_snapshot", detail: null },
  stale_request: { retryable: false, recovery: "review_request_again", detail: null },
  not_found: { retryable: false, recovery: "none", detail: null },
  internal: { retryable: false, recovery: "report_problem", detail: null },
};

// ---- The fake host ----

export class FakeHost implements Host {
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
  // Answers the last `method` frame with a result.
  reply(method: OwnerMethod, result: unknown, run = this.run): void {
    this.frame({ protocol: "fabric-desktop/1", id: this.last(method).id, method, result } as OwnerResponse, run);
  }
  // Answers the last `method` frame with this whole error (any OwnerError, from a fixture or written out).
  fail(method: OwnerMethod, error: OwnerError, run = this.run): void {
    this.frame({ protocol: "fabric-desktop/1", id: this.last(method).id, method, error }, run);
  }
  // Answers with an error of a code that carries no detail.
  error(method: OwnerMethod, code: ErrorCode, run = this.run): void {
    this.fail(method, { code, message: "Refused by the test.", correlation_id: null, ...ERRORS[code] } as OwnerError, run);
  }
  notice(sequence: number, change: OwnerNotice["change"]): void {
    this.frame({ protocol: "fabric-desktop/1", sequence, change });
  }
}

// Every frame the model sent is a valid owner request.
export function assertRequestsValid(host: FakeHost): void {
  for (const raw of host.raw) {
    const verdict = judge(contract, `${ENTRY}owner_request`, encoder.encode(raw));
    assert.equal(verdict.verdict, "accept", `${raw} -> ${JSON.stringify(verdict)}`);
  }
}

export const HELLO_RESULT = fixture("accept-owner-response-hello.json").result;
export const flush = (): Promise<void> => new Promise((resolve) => setImmediate(resolve));

export function setup() {
  const host = new FakeHost();
  const clock = { now: RECEIVED_MS };
  let ids = 0;
  let changes = 0;
  const announced: string[] = [];
  const model = new Model({
    host, now: () => clock.now, newId: () => `id-${++ids}`, zone: "America/Chicago",
    changed: () => void (changes += 1), announce: (text) => void announced.push(text),
  });
  model.start();
  return { host, model, clock, announced, changes: () => changes };
}

// Runs hello, snapshot and subscribe for a new run.
export function connect(host: FakeHost, run: number, snap: Snapshot): void {
  host.state({ name: "connected" }, run);
  host.reply("hello", HELLO_RESULT);
  host.reply("get_snapshot", snap);
  host.reply("subscribe", { cursor: snap.cursor });
}

// A request waiting with its card read; returns the model ready to decide.
export function pendingCard() {
  const s = setup();
  connect(s.host, 1, snapshot(0));
  s.host.notice(1, { kind: "request", request: request("pending_approval", 4) });
  s.host.reply("get_request_details", details(request("pending_approval", 4)));
  return s;
}

export const entry = (model: Model, id = "spike-request-1") => {
  const e = model.requests.get(id);
  assert.ok(e !== undefined);
  return e;
};
