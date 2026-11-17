// The model against the fake host of support.ts: no browser, no DOM. Every frame the model
// sends, and every frame the tests feed it, is judged by the contract's own TypeScript checker.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { REQUEST_STATES } from "../../../contracts/desktop/v1/typescript/src/contract.ts";
import type { Grant, Snapshot } from "../../../contracts/desktop/v1/typescript/src/contract.ts";
import { CONTRACT_ROOT } from "../../../contracts/desktop/v1/typescript/src/judge.ts";
import { ENDED_GRANT_LIMIT, ENDED_REQUEST_LIMIT } from "../ui/src/model.ts";
import { STATE_TEXT, TERMINAL, bannerText } from "../ui/src/text.ts";
import {
  AGENT, DIGEST_A, DIGEST_B, HELLO_RESULT, RECEIVED_AT, RECEIVED_MS, assertRequestsValid, connect, details, entry, fixture,
  flush, pendingCard, request, setup, snapshot,
} from "./support.ts";

// ---- Tests ----

test("on connected: hello, then get_snapshot, then subscribe with the snapshot's cursor", () => {
  const { host, model } = setup();
  host.state({ name: "connected" }, 1);
  assert.deepEqual(host.methods(), ["hello"]);
  assert.deepEqual(host.last("hello").params, { client: { name: "Fabric Calendar", build: "CD-W08 webview" }, schema_version: 3 });
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

test("all 26 states have a text, and TERMINAL equals the lifecycle data", () => {
  assert.equal(REQUEST_STATES.length, 26);
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
  assertRequestsValid(host);
  await flush();
});

// ---- The state of the whole broker (CD-W08) ----

const RICH: Snapshot = fixture("accept-owner-response-get-snapshot-rich.json").result;
const change = (name: string): any => fixture(name).change;
const ENDED: Grant = change("accept-owner-notice-grant-ended-consumed.json").grant;
const PROVER_STOPPED = "The rule checker is not running. The broker did not say why.";

// A live model whose last snapshot was `snap`, with nothing announced yet.
function live(snap: Snapshot = snapshot(0)) {
  const s = setup();
  connect(s.host, 1, snap);
  s.announced.length = 0;
  return s;
}

test("a snapshot replaces accounts, agents, profiles, preferences, groups, health and the live grants", () => {
  const { model } = live(RICH);
  assert.equal(model.phase, "live");
  assert.deepEqual(model.health, { prover: "ready", prover_problem: null, store: "ready" });
  assert.equal(model.profiles?.revision, 7);
  assert.equal(model.preferences?.revision, RICH.preferences.revision);
  assert.deepEqual([...model.accounts.keys()], ["acct-1", "acct-2"]);
  assert.deepEqual([...model.agents.keys()], RICH.agents.map((a) => a.agent_id));
  assert.deepEqual([...model.recipientSets.keys()], ["rset-team", "rset-people-family"]);
  assert.deepEqual([...model.grants.keys()], ["grant-4", "grant-5", "grant-6"]);
  assert.deepEqual([...model.requests.keys()], ["request-17", "request-18"]);
});

test("each kind of notice updates its state", () => {
  const { host, model, announced } = live({ ...RICH, cursor: 0 });
  const kinds = [
    "profiles", "preferences", "recipient-set", "recipient-set-deleted", "grant-created", "grant-ended-consumed",
    "grant-ended-session-idle-timeout", "account-reauthentication-required", "account-consent-denied", "agent",
    "request-blocked", "health-prover-unavailable",
  ];
  kinds.forEach((kind, i) => host.notice(i + 1, change(`accept-owner-notice-${kind}.json`)));
  assert.equal(model.cursor, kinds.length);
  assert.deepEqual(model.profiles, change("accept-owner-notice-profiles.json").profiles);
  assert.deepEqual(model.preferences, change("accept-owner-notice-preferences.json").preferences);
  assert.deepEqual([...model.recipientSets.keys()], ["rset-people-family"], "rset-team was deleted");
  assert.equal(model.grants.get("grant-4")?.ended?.reason, "consumed");
  assert.equal(model.grants.get("grant-5")?.ended?.reason, "session_idle_timeout");
  assert.equal(model.grants.get("grant-6")?.ended, null);
  assert.equal(model.accounts.get("acct-2")?.state, "reauthentication_required");
  assert.equal(model.accounts.get("acct-3")?.state, "consent_denied");
  assert.equal(model.agents.get("agent-claude-code")?.revision, 5);
  assert.equal(model.requests.get("request-22")?.request.status.state, "blocked");
  assert.deepEqual(model.health, { prover: "unavailable", prover_problem: null, store: "ready" });
  assert.deepEqual(announced, [PROVER_STOPPED]);
  assert.match(model.activity.at(-2)?.text ?? "", /Blocked by a rule \(rule [a-z_]+\)$/);
  assert.equal(model.activity.at(-1)?.text, "Rule checker: Not running. Store: Open");
});

test("a grant that ended stays; a live grant the next snapshot does not list is dropped", () => {
  const { host, model } = live({ ...RICH, cursor: 0 });
  host.notice(1, change("accept-owner-notice-grant-ended-consumed.json")); // grant-4 ended
  host.notice(2, { kind: "snapshot_required" });
  host.reply("get_snapshot", { ...RICH, cursor: 2, grants: RICH.grants.filter((g) => g.grant_id === "grant-6") });
  host.reply("subscribe", { cursor: 2 });
  assert.equal(model.grants.get("grant-4")?.ended?.reason, "consumed", "an ended grant stays");
  assert.equal(model.grants.has("grant-5"), false, "its end was not seen, so it is dropped");
  assert.equal(model.grants.get("grant-6")?.ended, null);
});

test(`at most ${ENDED_GRANT_LIMIT} ended grants are kept, the oldest end dropped first`, () => {
  const { host, model } = live();
  const start = Date.parse(ENDED.ended?.at ?? "");
  for (let i = 0; i <= ENDED_GRANT_LIMIT; i++) {
    // Newest end first, so the oldest arrives last and is the one dropped.
    const at = new Date(start + (ENDED_GRANT_LIMIT - i) * 1000).toISOString();
    host.notice(i + 1, { kind: "grant", grant: { ...ENDED, grant_id: `grant-${i}`, ended: { at, reason: "consumed" } } });
  }
  assert.equal(model.grants.size, ENDED_GRANT_LIMIT);
  assert.equal(model.grants.has(`grant-${ENDED_GRANT_LIMIT}`), false);
  assert.equal(model.grants.has("grant-0"), true);
});

test(`at most ${ENDED_REQUEST_LIMIT} ended requests are kept; open ones are never dropped`, () => {
  const { host, model } = live();
  const at = (s: number): string => new Date(RECEIVED_MS + s * 1000).toISOString();
  host.notice(1, { kind: "request", request: { ...request("received", 1, DIGEST_A, "open-1"), updated_at: RECEIVED_AT } });
  for (let i = 0; i <= ENDED_REQUEST_LIMIT; i++) {
    host.notice(i + 2, { kind: "request", request: { ...request("denied", 2, DIGEST_A, `ended-${i}`), updated_at: at(i + 1) } });
  }
  assert.equal(model.requests.size, ENDED_REQUEST_LIMIT + 1);
  assert.equal(model.requests.has("open-1"), true);
  assert.equal(model.requests.has("ended-0"), false, "the oldest change is dropped");
  assert.equal(model.requests.has(`ended-${ENDED_REQUEST_LIMIT}`), true);
});

test("command: no answer, then the same content reuses the id; an answer, or other content, gets a new id", async () => {
  const { host, model } = live();
  const params = { target: { kind: "agent", agent_id: "spike-agent", expected_revision: 1 }, paused: true } as const;
  const failures: (string | null)[] = [];
  const send = (p: typeof params | { target: { kind: "all_agents" }; paused: boolean } = params): string =>
    model.command("pause_agent", p, () => {}, (f) => void failures.push(f.code));
  host.refusal = { kind: "io", detail: "broken pipe" };
  const first = send();
  await flush();
  assert.deepEqual(failures, [null]);
  host.refusal = null;
  assert.equal(send(), first, "no answer came: the retry is the same command");
  host.reply("pause_agent", { commit_sequence: 1, agents: [{ ...AGENT, revision: 2, paused: true }] });
  const afterResult = send();
  assert.notEqual(afterResult, first, "answered with a result: a new command");
  host.error("pause_agent", "internal");
  assert.deepEqual(failures, [null, "internal"]);
  const afterError = send();
  assert.notEqual(afterError, afterResult, "answered with an error: a new command");
  assert.notEqual(send({ target: { kind: "all_agents" }, paused: true }), afterError, "other content: a new command");
  assertRequestsValid(host);
});

test("a command's failure carries the broker's whole error", () => {
  const { host, model } = live();
  let detail: unknown = null;
  model.command("pause_agent", { target: { kind: "agent", agent_id: "spike-agent", expected_revision: 1 }, paused: true }, () => {}, (f) => {
    detail = f.error?.code === "conflict" ? f.error.detail : null;
  });
  host.fail("pause_agent", fixture("accept-owner-response-error-conflict.json").error);
  assert.deepEqual(detail, { current_revision: 8 });
});

for (const reason of ["locked", "keychain_refused"] as const) {
  test(`store_unavailable (${reason}) fails the sync; retrySync reads everything again`, () => {
    const { host, model, announced } = setup();
    host.state({ name: "connected" }, 1);
    host.reply("hello", HELLO_RESULT);
    host.fail("get_snapshot", fixture(`accept-owner-response-error-store-unavailable-${reason.replace("_", "-")}.json`).error);
    assert.equal(model.phase, "failed");
    assert.equal(model.syncFailure?.code, "store_unavailable");
    assert.equal(model.ready, false);
    assert.equal(model.notReadyText, "Not available while the broker cannot open its store.");
    const title = reason === "locked" ? "The broker cannot open its store." : "macOS did not let the broker use the store key in the Keychain.";
    assert.equal(announced.at(-1), `${title} Nothing is read or written until the store opens.`);
    assert.equal(model.refresh(), false);
    const sent = host.raw.length;
    assert.equal(model.retrySync(), true);
    assert.deepEqual(host.methods().slice(sent), ["hello"]);
    host.reply("hello", HELLO_RESULT);
    host.reply("get_snapshot", snapshot(0));
    host.reply("subscribe", { cursor: 0 });
    assert.equal(model.phase, "live");
    assert.equal(model.syncFailure, null);
    assert.equal(model.retrySync(), false, "only after a failed sync");
  });
}

test("refresh takes a new snapshot and subscribes again, only while live", () => {
  const { host, model } = live(snapshot(3));
  const sent = host.raw.length;
  assert.equal(model.refresh(), true);
  assert.deepEqual(host.methods().slice(sent), ["get_snapshot"]);
  assert.equal(model.ready, false);
  assert.equal(model.refresh(), false);
  host.reply("get_snapshot", snapshot(5));
  assert.deepEqual(host.last("subscribe").params, { cursor: 5 });
  host.reply("subscribe", { cursor: 5 });
  assert.equal(model.ready, true);
});

test("ready and notReadyText in each phase", () => {
  const { host, model } = setup();
  assert.equal(model.ready, false);
  assert.equal(model.notReadyText, "Not available until this window has reached the broker.");
  host.state({ name: "connected" }, 1);
  assert.equal(model.notReadyText, "Not available until the broker's current state has been read.");
  host.reply("hello", HELLO_RESULT);
  assert.equal(model.phase, "snapshot");
  host.error("get_snapshot", "internal");
  assert.equal(model.notReadyText, "Not available: the broker's current state could not be read.");
  model.retrySync();
  host.reply("hello", HELLO_RESULT);
  host.reply("get_snapshot", snapshot(0));
  assert.equal(model.phase, "subscribe");
  assert.equal(model.notReadyText, "Not available until the broker's current state has been read.");
  host.reply("subscribe", { cursor: 0 });
  assert.equal(model.ready, true);
  assert.equal(model.notReadyText, null);
  host.state({ name: "disconnected", reason: "exited", detail: "exit code 1" });
  assert.equal(model.notReadyText, "Not available while the broker is not running.");
  host.state({ name: "conflict", detail: "another instance" }, 2);
  assert.equal(model.ready, false);
  assert.equal(model.notReadyText, "Not available while the broker is not running.");
});

test("the rule checker's state is announced when it changes after the first snapshot of a run", () => {
  const unavailable = { prover: "unavailable", prover_problem: null, store: "ready" } as const;
  const { host, model, announced } = setup();
  connect(host, 1, { ...snapshot(0), health: unavailable });
  assert.deepEqual(announced, [], "the first snapshot of a run says nothing");
  host.notice(1, { kind: "health", health: { prover: "starting", prover_problem: null, store: "ready" } });
  host.notice(2, { kind: "health", health: { prover: "starting", prover_problem: null, store: "locked" } });
  host.notice(3, { kind: "health", health: { prover: "ready", prover_problem: null, store: "ready" } });
  assert.deepEqual(announced, ["The rule checker is starting.", "The rule checker is ready."]);
  model.refresh();
  host.reply("get_snapshot", { ...snapshot(3), health: unavailable });
  assert.deepEqual(announced.at(-1), PROVER_STOPPED, "a later snapshot of the same run");
  host.state({ name: "disconnected", reason: "exited", detail: "exit code 1" });
  model.restart();
  announced.length = 0;
  connect(host, 2, snapshot(0));
  assert.deepEqual(announced, ["The broker is running."], "a new run's first snapshot says nothing about the rule checker");
});

test("the cause of a rule checker that is not running is announced and in the activity line; the same health twice is said once", () => {
  const { host, model, announced } = live({ ...RICH, cursor: 0 });
  host.notice(1, change("accept-owner-notice-health-prover-failed-will-retry.json"));
  assert.deepEqual(announced, [
    "The rule checker is not running. It could not start, or it stopped. The broker starts it again by itself, so you only need to wait.",
  ]);
  assert.equal(model.activity.at(-1)?.text, "Rule checker: Not running, the broker starts it again. Store: Open");
  host.notice(2, change("accept-owner-notice-health-prover-failed-will-retry.json"));
  assert.equal(announced.length, 1, "the same health again is not announced again");
  host.notice(3, change("accept-owner-notice-health-prover-failed.json"));
  assert.deepEqual(announced.at(-1),
    "The rule checker is not running. It could not start, or it stopped, and the broker will not try again in this run. Quit Fabric Calendar and open it again.",
    "a change of cause while still not running is announced");
  assert.equal(model.activity.at(-1)?.text, "Rule checker: Not running, the broker will not start it again in this run. Store: Open");
  host.notice(4, change("accept-owner-notice-health-prover-unavailable.json"));
  assert.deepEqual(announced.at(-1), PROVER_STOPPED, "the cause became unknown");
  assert.equal(model.activity.at(-1)?.text, "Rule checker: Not running. Store: Open");
  host.notice(5, change("accept-owner-notice-health-prover-not-configured.json"));
  assert.equal(model.activity.at(-1)?.text, "Rule checker: Not running, this run has none. Store: Open");
  host.notice(6, change("accept-owner-notice-health-prover-wrong-rules.json"));
  assert.equal(model.activity.at(-1)?.text, "Rule checker: Not running, the rules on this computer are not this broker's. Store: Open");
  assert.deepEqual(announced.at(-1),
    "The rule checker is not running. The rules on this computer are not the ones this broker was built with. Install Fabric Calendar again.");
  assert.equal(announced.length, 5);
});

// ---- Details and the session duration (CD-W08 Review) ----

test("openDetails reads a request that is not pending, also one the next snapshot does not list; not while not ready", () => {
  const { host, model } = live();
  const fixed = fixture("accept-owner-response-get-request-details-applied-create.json").result;
  const applied = { ...fixed, request: { ...fixed.request, revision: 10, status: { state: "reconciled" } } }; // it ended
  host.notice(1, { kind: "request", request: applied.request });
  const reads = () => host.methods().filter((m) => m === "get_request_details").length;
  assert.equal(reads(), 0, "a request that ended is not read for a card");
  model.openDetails("request-25", true);
  assert.deepEqual(host.last("get_request_details").params, { request_id: "request-25", redaction: "full" });
  assert.equal(entry(model, "request-25").detailsOpen, true);
  host.reply("get_request_details", applied);
  assert.equal(entry(model, "request-25").details?.attempts.length, 2);
  model.openDetails("request-25", true);
  assert.equal(reads(), 1, "the details of this run and revision are not read again");
  // A new run: the snapshot does not list the request; its open details are read in the new run.
  host.state({ name: "disconnected", reason: "exited", detail: "exit code 1" });
  model.openDetails("request-25", false);
  model.openDetails("request-25", true);
  assert.equal(reads(), 1, "not while the broker is not running");
  model.restart();
  connect(host, 2, snapshot(0));
  assert.equal(reads(), 2);
  host.reply("get_request_details", applied);
  assert.equal(entry(model, "request-25").detailsRun, 2);
  assert.equal(entry(model, "request-25").available, true);
});

test("without a live session of the agent, the session duration is not taken and a chosen one falls back", () => {
  const { host, model, changes } = live();
  const away = { ...AGENT, revision: 2, session: null };
  host.notice(1, { kind: "agent", agent: away });
  const pending = request("pending_approval", 4);
  host.notice(2, { kind: "request", request: pending });
  const d = details(pending);
  host.reply("get_request_details", { ...d, card: d.card === null ? null : { ...d.card, proposed_duration: { kind: "session" } } });
  const e = entry(model);
  assert.equal(model.sessionAvailable(e), false);
  assert.equal(e.duration, "session", "the proposed duration is kept");
  assert.equal(model.duration(e), "one_request", "but what is shown and sent falls back");
  const before = changes();
  model.chooseDuration("spike-request-1", "session");
  assert.equal(e.durationChosen, false, "the choice is ignored");
  assert.ok(changes() > before, "the views render again, so the radio is put back");
  model.decide("spike-request-1", "approve");
  assert.deepEqual(host.last("decide_request").params.duration, { kind: "one_request" });
  host.error("decide_request", "internal");
  host.notice(3, { kind: "agent", agent: { ...AGENT, revision: 3 } });
  assert.equal(model.duration(e), "session", "with a live session again, the proposed duration holds");
  model.chooseDuration("spike-request-1", "until_revoked");
  assert.equal(model.duration(e), "until_revoked");
});

test("a card says so when the broker's state could not be read, and is ready again after a new sync", () => {
  const { host, model } = pendingCard();
  model.refresh();
  host.error("get_snapshot", "internal");
  assert.equal(model.phase, "failed");
  assert.deepEqual(model.cardStatus(entry(model)), {
    kind: "waiting", text: "The broker's current state could not be read. This request is read again when it can be.",
  });
  assert.equal(model.decide("spike-request-1", "approve"), false);
  model.retrySync();
  host.reply("hello", HELLO_RESULT);
  host.reply("get_snapshot", snapshot(0, [request("pending_approval", 4)]));
  host.reply("subscribe", { cursor: 0 });
  assert.equal(model.cardStatus(entry(model)).kind, "ready");
});

test("the decision is said in one whole sentence per duration", () => {
  const sentences = {
    one_request: "You allowed this, for this one request.",
    session: "You allowed this. The permission lasts until this agent's current session ends.",
    until_revoked: "You allowed this. The permission lasts until you take it back.",
  } as const;
  for (const kind of ["one_request", "session", "until_revoked"] as const) {
    const { host, model, announced } = pendingCard();
    model.chooseDuration("spike-request-1", kind);
    model.decide("spike-request-1", "approve");
    const approved = request("approved", 5);
    host.reply("decide_request", {
      commit_sequence: 3, request: approved,
      approval: {
        decision: "approve", command_id: host.last("decide_request").id, decided_at: RECEIVED_AT, approval_digest: DIGEST_B,
        scope: details(approved).card?.scope, duration: { kind }, effective_request_digest: DIGEST_A, grant_id: "spike-grant-1",
      },
    });
    assert.equal(announced.at(-1), sentences[kind]);
    const status = model.cardStatus(entry(model));
    assert.equal(status.kind === "closed" ? status.text : null, sentences[kind]);
  }
});

test("'Received, checking' is not measured for a request first seen in a snapshot, nor when the clocks disagree", () => {
  const { host, model, clock } = setup();
  clock.now = RECEIVED_MS + 60_000;
  connect(host, 1, snapshot(0, [request("validating", 2)]));
  assert.equal(entry(model).checkingDelayMs, null, "from a snapshot: received long before this page saw it");
  host.notice(1, { kind: "request", request: request("proving_classification", 3) });
  assert.equal(entry(model).checkingDelayMs, null, "the text was first shown by the snapshot, not by this notice");
  clock.now = RECEIVED_MS - 250; // this computer's clock is behind the broker's
  host.notice(2, { kind: "request", request: request("received", 1, DIGEST_A, "ahead") });
  assert.equal(entry(model, "ahead").checkingDelayMs, null, "a negative difference records nothing");
});
