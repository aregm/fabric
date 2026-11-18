// The attention strip's items: what needs the owner now, in a fixed order.
import assert from "node:assert/strict";
import { test } from "node:test";
import type { Snapshot } from "../../../contracts/desktop/v1/typescript/src/contract.ts";
import { attention, buildLine, calendarLine, connectionText, proverCauseText, proverText, storeText } from "../ui/src/status.ts";
import { PROVER_PROBLEM } from "../ui/src/text.ts";
import { HELLO_RESULT, connect, details, fixture, request, setup, snapshot } from "./support.ts";

const keys = (s: ReturnType<typeof setup>): string[] => attention(s.model).map((i) => i.key);

test("a healthy, empty broker needs nothing", () => {
  const s = setup();
  connect(s.host, 1, snapshot(0));
  assert.deepEqual(attention(s.model), []);
  assert.equal(connectionText(s.model), "Running and up to date");
  assert.equal(storeText(s.model), "Open");
  assert.equal(proverText(s.model), "Running");
  assert.equal(buildLine(s.model), "Development build. Broker: Fabric broker, build " + HELLO_RESULT.broker.build + ".");
});

test("the broker not running: restart only when it is disconnected", () => {
  const s = setup();
  s.host.state({ name: "disconnected", reason: "exited", detail: "exit code 1" }, 1);
  const [item] = attention(s.model);
  assert.deepEqual(item, {
    key: "broker", kind: "problem", title: "The broker stopped.", text: item?.text, detail: "Details: exit code 1", action: "restart",
  });
  s.host.state({ name: "conflict", detail: "another instance holds the lock" }, 2);
  assert.equal(attention(s.model)[0]?.action, null);
  assert.equal(attention(s.model)[0]?.title, "Fabric Calendar is already open.");
});

test("a failed sync: by the store's reason, or the broker's message; action retry", () => {
  for (const [reason, title] of [
    ["locked", "The broker cannot open its store."],
    ["keychain-refused", "macOS did not let the broker use the store key in the Keychain."],
  ] as const) {
    const s = setup();
    s.host.state({ name: "connected" }, 1);
    s.host.reply("hello", HELLO_RESULT);
    const error = fixture(`accept-owner-response-error-store-unavailable-${reason}.json`).error;
    s.host.fail("get_snapshot", error);
    assert.deepEqual(attention(s.model), [{
      key: "sync", kind: "problem", title, text: `${error.message} Nothing is read or written until the store opens.`, detail: null, action: "retry",
    }]);
  }
  const s = setup();
  s.host.state({ name: "connected" }, 1);
  s.host.reply("hello", HELLO_RESULT);
  s.host.error("get_snapshot", "internal");
  assert.deepEqual(attention(s.model), [{
    key: "sync", kind: "problem", title: "The current state could not be read from the broker.", text: "Refused by the test.", detail: null, action: "retry",
  }]);
});

test("the store and the rule checker by the health the broker of this run reported", () => {
  const s = setup();
  const locked: Snapshot = fixture("accept-owner-response-get-snapshot-store-locked-prover-unavailable.json").result;
  connect(s.host, 1, { ...locked, accounts: [] });
  const items = attention(s.model);
  assert.deepEqual(items.map((i) => [i.key, i.kind, i.title, i.action]), [
    ["store", "problem", "The broker cannot open its store.", null],
    ["prover", "notice", "The rule checker is not running.", null],
  ]);
  assert.match(items[1]?.text ?? "", /^The broker did not say why\. Fabric allows nothing it cannot prove/);
  s.host.notice(1, { kind: "health", health: { prover: "starting", prover_problem: null, store: "keychain_refused", agent_channel: "listening", agent_channel_problem: null } });
  assert.deepEqual(attention(s.model).map((i) => i.title), [
    "macOS did not let the broker use the store key in the Keychain.", "The rule checker is starting.",
  ]);
  s.host.state({ name: "disconnected", reason: "exited", detail: "exit code 1" });
  assert.deepEqual(keys(s), ["broker"], "the health of a stopped broker is not shown");
  assert.equal(storeText(s.model), "Not known while the broker's current state has not been read");
});

test("the rule checker not running: the strip and the Status screen name the cause the broker gives", () => {
  const s = setup();
  connect(s.host, 1, snapshot(0));
  assert.equal(proverCauseText(s.model), null, "ready: no cause");
  const causes = [
    ["unavailable", "The broker did not say why."],
    ["not-configured", PROVER_PROBLEM.not_configured],
    ["failed-will-retry", PROVER_PROBLEM.failed_will_retry],
    ["failed", PROVER_PROBLEM.failed],
    ["wrong-rules", PROVER_PROBLEM.wrong_rules],
  ] as const;
  causes.forEach(([name, cause], i) => {
    s.host.notice(i + 1, fixture(`accept-owner-notice-health-prover-${name}.json`).change);
    const item = attention(s.model).find((x) => x.key === "prover");
    assert.equal(item?.title, "The rule checker is not running.", name);
    assert.ok(item?.text.startsWith(`${cause} Fabric allows nothing it cannot prove`), `${name}: ${item?.text}`);
    assert.equal(item?.detail, null);
    assert.equal(proverText(s.model), "Not running");
    assert.equal(proverCauseText(s.model), cause, name);
  });
  assert.equal(PROVER_PROBLEM.wrong_rules, "The rules on this computer are not the ones this broker was built with. Install Fabric Calendar again.");
  s.host.notice(causes.length + 1, { kind: "health", health: { prover: "starting", prover_problem: null, store: "ready", agent_channel: "listening", agent_channel_problem: null } });
  assert.equal(proverCauseText(s.model), null, "starting: no cause");
  s.host.notice(causes.length + 2, fixture("accept-owner-notice-health-prover-failed.json").change);
  assert.equal(proverCauseText(s.model), PROVER_PROBLEM.failed);
  s.host.state({ name: "disconnected", reason: "exited", detail: "exit code 1" });
  assert.equal(proverCauseText(s.model), null, "the health of a stopped broker is not shown");
  s.host.state({ name: "connected" }, 2);
  s.host.reply("hello", HELLO_RESULT);
  assert.equal(proverCauseText(s.model), null, "a new run before its snapshot: no cause of the earlier run");
});

test("one item per account that needs the owner", () => {
  const s = setup();
  const locked: Snapshot = fixture("accept-owner-response-get-snapshot-store-locked-prover-unavailable.json").result;
  connect(s.host, 1, { ...locked, health: { prover: "ready", prover_problem: null, store: "ready", agent_channel: "listening", agent_channel_problem: null } });
  // awaiting_consent and disconnected need nothing; the other four do.
  assert.deepEqual(keys(s), ["account-acct-4", "account-acct-5", "account-acct-6", "account-acct-7"]);
  const [denied, partial] = attention(s.model);
  assert.equal(denied?.title, "An account: Not connected: access was refused while connecting it.");
  assert.equal(denied?.detail, "Account acct-4");
  assert.equal(partial?.title, "third@example.net: Connected, but with less access than Fabric needs.");
  assert.equal(partial?.text, "This build cannot connect or reconnect a Google account yet.");
  s.host.notice(1, fixture("accept-owner-notice-account-reauthentication-required.json").change);
  assert.equal(attention(s.model).at(-1)?.title, "owner.work@example.org: Needs you to sign in again.");
});

test("requests waiting for a decision: one item, action review", () => {
  const s = setup();
  connect(s.host, 1, snapshot(0));
  s.host.notice(1, { kind: "request", request: request("pending_approval", 4) });
  s.host.reply("get_request_details", details(request("pending_approval", 4)));
  assert.deepEqual(attention(s.model), [{ key: "waiting", kind: "notice", title: "1 request waits for your decision.", text: "", detail: null, action: "review" }]);
  s.host.notice(2, { kind: "request", request: request("pending_approval", 4, undefined, "spike-request-2") });
  assert.equal(attention(s.model)[0]?.title, "2 requests wait for your decision.");
  s.host.notice(3, { kind: "request", request: request("denied", 5) });
  assert.equal(attention(s.model)[0]?.title, "1 request waits for your decision.");
});

test("the items come in the order of the design: broker, sync, store, rule checker, accounts, requests", () => {
  const s = setup();
  const rich: Snapshot = fixture("accept-owner-response-get-snapshot-rich.json").result;
  connect(s.host, 1, { ...rich, cursor: 0, health: { prover: "unavailable", prover_problem: null, store: "locked", agent_channel: "listening", agent_channel_problem: null } });
  assert.deepEqual(keys(s), ["store", "prover", "account-acct-2", "waiting"]);
  s.model.refresh();
  s.host.error("get_snapshot", "internal");
  assert.deepEqual(keys(s), ["sync", "store", "prover", "account-acct-2", "waiting"]);
  s.host.state({ name: "disconnected", reason: "exited", detail: "exit code 1" });
  assert.deepEqual(keys(s), ["broker", "account-acct-2", "waiting"]);
});

test("a calendar line names its access and its flags", () => {
  assert.equal(calendarLine({ calendar_id: "c", name: "Agent Drafts", access: "owner", selected: true, agent_drafts: true }),
    "Agent Drafts: you own it; agents may use it; this is the Agent Drafts calendar");
  assert.equal(calendarLine({ calendar_id: "c", name: "Holidays", access: "reader", selected: false, agent_drafts: false }),
    "Holidays: you can see its events");
});
