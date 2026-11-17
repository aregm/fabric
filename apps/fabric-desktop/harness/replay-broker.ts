#!/usr/bin/env node
// A development tool, not shipped: a stand-in for the broker that speaks the owner channel of
// `fabric-desktop/1` on stdin and stdout by replaying the contract's fixtures, so the page can be
// looked at with what the real broker cannot show yet (it has no rule prover): a blocked request
// with its proof, a request carried out, a failure, requests that no proof decided with the reason
// the broker recorded, an account that needs a new sign-in, several agents, a custom profile, and
// the causes of a rule checker that is not running. It is for looking at the page only, never
// evidence for a gate.
//
// No fixture proposes a duration longer than one request. So that a card with three durations can
// be looked at, one more pending request is made here from the create fixture: its card proposes
// "until_revoked". Its agent has no live session in the snapshot, so the session choice is there
// and cannot be taken.
//
// It answers hello and get_snapshot with the rich fixtures, get_request_details and get_proof with
// the fixture whose id matches (otherwise not_found), and every command with `internal`: it changes
// nothing. The four undecided fixtures share one request id; here each gets its own, from its file
// name. The first subscribe of the process is followed by notices built from the fixtures: eight
// requests and the one made here, three grant changes, four seconds later the prover becoming
// unavailable with no cause named, and then, six seconds apart, each cause the broker can name,
// the wrong rules last. Every
// frame is judged by the contract's checker before it is written; a frame the checker does not
// accept is not written, and one line about it goes to stderr. stdout carries frames only.
//
// It ignores the environment serve.ts passes for the real broker and writes nothing to disk. End
// of input ends it.
//
//   node harness/serve.ts --broker harness/replay-broker.ts --port 4319
import { readdirSync, readFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { CONTRACT_ROOT, judge, loadContract } from "../../../contracts/desktop/v1/typescript/src/judge.ts";

const PROTOCOL = "fabric-desktop/1";
const ENTRY = "owner-channel.schema.json#/$defs/";
const DIR = `${CONTRACT_ROOT}fixtures/messages/`;
const contract = loadContract();
const encoder = new TextEncoder();

// The `value` of a message fixture (null for a fixture written as text or bytes).
const value = (name: string): any => JSON.parse(readFileSync(DIR + name, "utf8")).value ?? null;
// The values of the fixtures whose names start with `prefix`, but not with `except`.
const valuesOf = (prefix: string, except: string | null = null): any[] =>
  readdirSync(DIR).filter((f) => f.startsWith(prefix) && (except === null || !f.startsWith(except)) && f.endsWith(".json"))
    .map(value).filter((v) => v !== null);

const HELLO = value("accept-owner-response-hello.json").result;
const SNAPSHOT = value("accept-owner-response-get-snapshot-rich.json").result;
// The undecided requests, one per recorded reason, each with an id of its own.
const UNDECIDED_PREFIX = "accept-owner-response-get-request-details-undecided-";
const UNDECIDED: any[] = ["prover-unavailable", "out-of-time", "failed", "reason-unknown"].map((name) => {
  const d = value(`${UNDECIDED_PREFIX}${name}.json`).result;
  return { ...d, request: { ...d.request, request_id: `${d.request.request_id}-${name}` } };
});
// The create fixture as a pending request whose card proposes "until_revoked", with only what a
// pending request has: no approval, authorization, attempt or event, and the one proof that led to
// its card.
const LONGER: any = (() => {
  const d = value("accept-owner-response-get-request-details-applied-create.json").result;
  return {
    ...d,
    request: {
      ...d.request, request_id: "request-25-until-revoked",
      status: { state: "pending_approval", approval_expires_at: d.card.expires_at },
    },
    card: { ...d.card, proposed_duration: { kind: "until_revoked" } },
    approval: null, proof_ids: [d.card.insufficiency.proof_id], authorization: null, attempts: [], provider_event: null,
  };
})();
const DETAILS: any[] = [
  ...valuesOf("accept-owner-response-get-request-details-", UNDECIDED_PREFIX).map((v) => v.result), ...UNDECIDED, LONGER,
];
const PROOFS: any[] = valuesOf("accept-owner-response-get-proof-").map((v) => v.result);

// The changes announced after the first subscribe, in order; the health changes come later.
const CHANGES: object[] = [
  ...["blocked", "applied-create", "terminal-failure", "full-invitation"].map((name) => ({
    kind: "request",
    request: value(`accept-owner-response-get-request-details-${name}.json`).result.request,
  })),
  ...UNDECIDED.map((d) => ({ kind: "request", request: d.request })),
  { kind: "request", request: LONGER.request },
  ...["grant-created", "grant-ended-consumed", "grant-ended-session-idle-timeout"].map(
    (name) => value(`accept-owner-notice-${name}.json`).change,
  ),
];
// The prover unavailable with no cause named, then each cause, the wrong rules last.
const LATE_CHANGES: object[] = ["unavailable", "not-configured", "failed-will-retry", "failed", "wrong-rules"].map(
  (name) => value(`accept-owner-notice-health-prover-${name}.json`).change,
);

function write(entry: "owner_response" | "owner_notice", frame: object): void {
  const line = JSON.stringify(frame);
  const verdict = judge(contract, ENTRY + entry, encoder.encode(line));
  if (verdict.verdict !== "accept") {
    console.error(`replay-broker: not written, the checker rejects this ${entry} (${verdict.layer}, ${verdict.error}): ${line.slice(0, 200)}`);
    return;
  }
  process.stdout.write(`${line}\n`);
}

const respond = (id: string, method: string, result: unknown): void =>
  write("owner_response", { protocol: PROTOCOL, id, method, result });

const fail = (id: string, method: string, code: "not_found" | "internal", message: string): void =>
  write("owner_response", {
    protocol: PROTOCOL, id, method,
    error: {
      code, message, retryable: false, recovery: code === "not_found" ? "none" : "report_problem",
      correlation_id: null, detail: null,
    },
  });

let sequence: number = SNAPSHOT.cursor;
let subscribed = false;
const timers = new Set<NodeJS.Timeout>();

function later(ms: number, change: object): void {
  const t = setTimeout(() => {
    timers.delete(t);
    sequence += 1;
    write("owner_notice", { protocol: PROTOCOL, sequence, change });
  }, ms);
  timers.add(t);
}

function startNotices(): void {
  CHANGES.forEach((change, i) => later(300 * (i + 1), change));
  LATE_CHANGES.forEach((change, i) => later(300 * CHANGES.length + 4000 + 6000 * i, change));
}

function handle(frame: any): void {
  const { id, method } = frame;
  const params = frame.params ?? {};
  switch (method) {
    case "hello":
      return respond(id, method, HELLO);
    case "get_snapshot":
      return respond(id, method, SNAPSHOT);
    case "subscribe":
      respond(id, method, { cursor: params.cursor });
      if (!subscribed) {
        subscribed = true;
        startNotices();
      }
      return;
    case "get_request_details": {
      const matches = DETAILS.filter((d) => d.request.request_id === params.request_id);
      const found = matches.find((d) => d.redaction === params.redaction) ?? matches[0];
      return found !== undefined ? respond(id, method, found)
        : fail(id, method, "not_found", `There is no request ${params.request_id} among the fixtures.`);
    }
    case "get_proof": {
      const found = PROOFS.find((p) => p.proof_id === params.proof_id);
      return found !== undefined ? respond(id, method, found)
        : fail(id, method, "not_found", `There is no proof ${params.proof_id} among the fixtures.`);
    }
    default:
      return fail(id, method, "internal", "The replay stand-in changes nothing.");
  }
}

const input = createInterface({ input: process.stdin });
input.on("line", (line) => {
  let frame: any;
  try {
    frame = JSON.parse(line);
  } catch {
    return;
  }
  if (frame === null || typeof frame !== "object" || typeof frame.id !== "string" || typeof frame.method !== "string") return;
  handle(frame);
});
input.on("close", () => {
  // Nothing else keeps the process alive: it ends with code 0 once stdout has drained.
  for (const t of timers) clearTimeout(t);
});
