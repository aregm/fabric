#!/usr/bin/env node
// A development tool, not shipped: a stand-in for the broker that speaks the owner channel of
// `fabric-desktop/1` on stdin and stdout by replaying the contract's fixtures, so the page can be
// looked at with what the real broker cannot show yet (it has no rule prover): a blocked request
// with its proof, a request carried out, a failure, an account that needs a new sign-in, several
// agents, a custom profile. It is for looking at the page only, never evidence for a gate.
//
// It answers hello and get_snapshot with the rich fixtures, get_request_details and get_proof with
// the fixture whose id matches (otherwise not_found), and every command with `internal`: it changes
// nothing. The first subscribe of the process is followed by notices built from the fixtures: four
// requests, three grant changes, and four seconds later the prover becoming unavailable. Every
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
const valuesOf = (prefix: string): any[] =>
  readdirSync(DIR).filter((f) => f.startsWith(prefix) && f.endsWith(".json")).map(value).filter((v) => v !== null);

const HELLO = value("accept-owner-response-hello.json").result;
const SNAPSHOT = value("accept-owner-response-get-snapshot-rich.json").result;
const DETAILS: any[] = valuesOf("accept-owner-response-get-request-details-").map((v) => v.result);
const PROOFS: any[] = valuesOf("accept-owner-response-get-proof-").map((v) => v.result);

// The changes announced after the first subscribe, in order; the last one comes later.
const CHANGES: object[] = [
  ...["blocked", "applied-create", "terminal-failure", "full-invitation"].map((name) => ({
    kind: "request",
    request: value(`accept-owner-response-get-request-details-${name}.json`).result.request,
  })),
  ...["grant-created", "grant-ended-consumed", "grant-ended-session-idle-timeout"].map(
    (name) => value(`accept-owner-notice-${name}.json`).change,
  ),
];
const LATE_CHANGE = value("accept-owner-notice-health-prover-unavailable.json").change;

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
  later(300 * CHANGES.length + 4000, LATE_CHANGE);
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
