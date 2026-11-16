// The words on the page: dates name their zone, and the card's facts come in reading order.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { ACCOUNT_STATES, CALENDAR_ACCESS, GRANT_END_REASONS } from "../../../contracts/desktop/v1/typescript/src/contract.ts";
import type { Grant, ProofReport, RecipientSet, Snapshot } from "../../../contracts/desktop/v1/typescript/src/contract.ts";
import { CONTRACT_ROOT } from "../../../contracts/desktop/v1/typescript/src/judge.ts";
import {
  ACCESS_TEXT, ACCOUNT_TEXT, ATTEMPT_TEXT, GRANT_END_TEXT, PROOF_OUTCOME, PROOF_PURPOSE, PROVER_TEXT, STORE_TEXT, VERDICT_TEXT, cardFacts,
  formatInstant, proofFacts, scopeFacts, termText,
} from "../ui/src/text.ts";

test("an instant is shown in the given zone, with the zone named", () => {
  assert.equal(formatInstant("2026-11-20T16:00:00.000Z", "America/Chicago"), "Friday, November 20, 2026 at 10:00 AM Central Standard Time");
});

test("the card's facts: who asks, the agent's words marked untrusted, the operation, then who is notified", () => {
  const details = JSON.parse(readFileSync(`${CONTRACT_ROOT}fixtures/messages/accept-owner-response-get-request-details-full-invitation.json`, "utf8")).value.result;
  const facts = cardFacts(details, { agents: new Map(), accounts: new Map(), zone: "America/Chicago" });
  const terms = facts.map((f) => f.term);
  assert.equal(terms[0], "Who asks");
  assert.equal(terms[1], "Where its data goes");
  assert.equal(facts[2]?.untrusted, true);
  assert.deepEqual(facts[2]?.values, ["Invite the team to the planning meeting."]);
  assert.equal(terms[3], "What it wants to do");
  assert.deepEqual(facts.find((f) => f.term === "Guests after the change")?.values, ["ally@example.net", "stranger@example.org"]);
  assert.equal(terms.at(-1), "Who is notified");
  assert.deepEqual(facts.at(-1)?.values, ["Everyone invited gets an email from Google."]);
});

test("every account state, calendar access and health value has words", () => {
  const all: [Record<string, string>, readonly string[]][] = [
    [ACCOUNT_TEXT, ACCOUNT_STATES], [ACCESS_TEXT, CALENDAR_ACCESS],
    [PROVER_TEXT, ["ready", "starting", "unavailable"]], [STORE_TEXT, ["ready", "locked", "keychain_refused"]],
  ];
  for (const [words, values] of all) {
    assert.deepEqual(Object.keys(words).sort(), [...values].sort());
    for (const v of values) assert.ok((words[v] ?? "").length > 0, v);
  }
});

const message = (name: string): any => JSON.parse(readFileSync(`${CONTRACT_ROOT}fixtures/messages/${name}`, "utf8")).value;

test("every grant end reason, proof outcome and purpose, rule verdict and attempt outcome has words", () => {
  const all: [Record<string, string>, readonly string[]][] = [
    [GRANT_END_TEXT, GRANT_END_REASONS],
    [PROOF_OUTCOME, ["permit", "need_approval", "block", "undecided"] satisfies ProofReport["outcome"][]],
    [PROOF_PURPOSE, ["classification", "fresh_permit"] satisfies ProofReport["purpose"][]],
    [VERDICT_TEXT, ["holds", "fails", "undecided"]],
    [ATTEMPT_TEXT, ["applied", "transient_failure", "terminal_failure", "outcome_unknown"]],
  ];
  for (const [words, values] of all) {
    assert.deepEqual(Object.keys(words).sort(), [...values].sort());
    for (const v of values) assert.ok((words[v] ?? "").length > 0, v);
  }
  assert.deepEqual([PROOF_OUTCOME.permit, PROOF_OUTCOME.need_approval, PROOF_OUTCOME.block, PROOF_OUTCOME.undecided],
    ["Allowed", "Needs your approval", "Blocked", "Not decided"]);
});

test("each grant term in words", () => {
  const digest = `sha256:${"a".repeat(64)}` as const;
  assert.equal(termText({ kind: "one_request", action_digest: digest }, "Claude Code"), "Only the one request it was given for");
  assert.equal(termText({ kind: "session", session_id: "s" }, "Claude Code"), "Until Claude Code's session ends");
  assert.equal(termText({ kind: "until_revoked", expires_at: null }, "Claude Code"), "Until you take it back. It has no end date.");
});

test("a scope in words: calendars by name, dates, fields, groups by name, who is notified", () => {
  const rich: Snapshot = message("accept-owner-response-get-snapshot-rich.json").result;
  const names = { agents: new Map(), accounts: new Map(rich.accounts.map((a) => [a.account_id, a])), zone: "America/Chicago" };
  const groups = new Map<string, RecipientSet>(rich.recipient_sets.map((r) => [r.recipient_set_id, r]));
  const grant = (id: string): Grant => {
    const g = rich.grants.find((x) => x.grant_id === id);
    assert.ok(g !== undefined);
    return g;
  };
  const words = (g: Grant) => Object.fromEntries(scopeFacts(g.scope, names, groups).map((f) => [f.term, f.values]));
  assert.deepEqual(words(grant("grant-6")), {
    Calendars: ["Team schedule (owner@example.com)"], Groups: ["Team", "Family (Google contacts)"],
    "Who is notified": ["Everyone invited gets an email from Google."],
  });
  assert.deepEqual(words(grant("grant-5")), {
    Calendars: ["Owner (owner@example.com)"],
    Dates: ["From Friday, November 20, 2026 at 12:00 AM Central Standard Time until Saturday, November 21, 2026 at 12:00 AM Central Standard Time"],
    "What it may read": ["time, title"],
  });
  assert.deepEqual(words({ ...grant("grant-6"), scope: { ...grant("grant-6").scope, recipients: { kind: "recipient_sets", recipient_set_ids: ["gone"] } } }).Groups,
    ["A group this app does not know (gone)"]);
});

test("the card shows the event as it is now before the change", () => {
  const details = message("accept-owner-response-get-request-details-full-invitation.json").result;
  const facts = cardFacts(details, { agents: new Map(), accounts: new Map(), zone: "America/Chicago" });
  const terms = facts.map((f) => f.term);
  assert.ok(terms.indexOf("Now: title") < terms.indexOf("Guests after the change"));
  assert.deepEqual(facts.find((f) => f.term === "Now: title")?.values, ["Q1 planning"]);
  assert.deepEqual(facts.find((f) => f.term === "Now: place")?.values, ["Room 4"]);
  assert.deepEqual(facts.find((f) => f.term === "Now: description")?.values, ["None"]);
  assert.deepEqual(facts.find((f) => f.term === "Guests now")?.values, ["owner@example.com (accepted)"]);
  assert.match(facts.find((f) => f.term === "Now: starts")?.values[0] ?? "", /2:00 PM Central Standard Time$/);
});

test("a proof without verdicts says so", () => {
  const p: ProofReport = message("accept-owner-response-get-proof-undecided-no-verdicts.json").result;
  const facts = proofFacts(p, "America/Chicago");
  assert.deepEqual(facts.find((f) => f.term === "Outcome")?.values, ["Not decided"]);
  assert.deepEqual(facts.find((f) => f.term === "Failing rule")?.values, ["None"]);
  assert.deepEqual(facts.find((f) => f.term === "Rules")?.values, ["The check gave no verdict on any rule."]);
  const permit = proofFacts(message("accept-owner-response-get-proof-permit.json").result, "America/Chicago");
  assert.deepEqual(permit.find((f) => f.term === "What it was for")?.values, ["The check just before carrying it out"]);
});
