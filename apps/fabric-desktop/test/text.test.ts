// The words on the page: dates name their zone, and the card's facts come in reading order.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { ACCOUNT_STATES, CALENDAR_ACCESS, GRANT_END_REASONS, PROVER_PROBLEMS, UNDECIDED_REASONS } from "../../../contracts/desktop/v1/typescript/src/contract.ts";
import type { Action, Grant, ProofReport, RecipientSet, RequestDetails, SendUpdates, Snapshot } from "../../../contracts/desktop/v1/typescript/src/contract.ts";
import { CONTRACT_ROOT } from "../../../contracts/desktop/v1/typescript/src/judge.ts";
import {
  ACCESS_TEXT, ACCOUNT_TEXT, ALLOWED_TEXT, ATTEMPT_TEXT, DURATION_TEXT, GRANT_END_TEXT, OWNER_CAN_DO, PROOF_OUTCOME, PROOF_PURPOSE, PROVER_PROBLEM,
  PROVER_PROBLEM_UNKNOWN, PROVER_TEXT, STATE_TEXT, STORE_TEXT, UNDECIDED_REASON, UNDECIDED_REASON_UNKNOWN, VERDICT_TEXT, bannerText, cardFacts,
  formatInstant, proofFacts, scopeFacts, termText,
} from "../ui/src/text.ts";
import { assertValid, details, request } from "./support.ts";

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
  assert.deepEqual(facts.at(-1)?.values, ["Everyone invited gets an email."]);
});

test("every account state, calendar access, health value, rule checker cause and undecided reason has words", () => {
  const all: [Record<string, string>, readonly string[]][] = [
    [ACCOUNT_TEXT, ACCOUNT_STATES], [ACCESS_TEXT, CALENDAR_ACCESS],
    [PROVER_TEXT, ["ready", "starting", "unavailable"]], [STORE_TEXT, ["ready", "locked", "keychain_refused"]],
    [PROVER_PROBLEM, PROVER_PROBLEMS], [UNDECIDED_REASON, UNDECIDED_REASONS],
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
    "Who is notified": ["Everyone invited gets an email."],
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

// ---- Cards of a practice run: the broker sends no event as it is now ----

const NAMES = { agents: new Map(), accounts: new Map(), zone: "America/Chicago" };
const EVENT = { event_id: "evt-practice-1", revision: "3181161784712000" };
const NO_CHANGES = { title: null, start: null, end: null, description: null, location: null };

// Waiting details of this action, with `before` null; checked against the contract.
function waiting(action: Action): RequestDetails {
  const base = details({ ...request("pending_approval", 4), tool: action.tool });
  const d: RequestDetails = { ...base, private: { action, before: null, warnings: [] } };
  assertValid("owner_response", { protocol: "fabric-desktop/1", id: "details-1", method: "get_request_details", result: d });
  return d;
}

const moveTime = (send_updates: SendUpdates): Action => ({
  tool: "fabric_calendar_propose_update",
  arguments: {
    account_id: "spike-account", calendar_id: "spike-calendar", event: EVENT, send_updates, task: "Move the meeting.",
    changes: {
      ...NO_CHANGES,
      start: { date_time: "2026-11-20T14:00:00-06:00", zone: "America/Chicago" },
      end: { date_time: "2026-11-20T15:00:00-06:00", zone: "America/Chicago" },
    },
  },
});

const remove = (send_updates: SendUpdates): Action => ({
  tool: "fabric_calendar_propose_delete",
  arguments: { account_id: "spike-account", calendar_id: "spike-calendar", event: EVENT, send_updates, task: "Remove the meeting." },
});

// The terms from "What it wants to do" on.
function asked(d: RequestDetails) {
  const facts = cardFacts(d, NAMES);
  return facts.slice(facts.findIndex((f) => f.term === "What it wants to do"));
}

test("the card of a change that moves an event's time, without the event as it is now", () => {
  const facts = asked(waiting(moveTime("none")));
  assert.deepEqual(facts.map((f) => f.term), [
    "What it wants to do", "Account", "Calendar", "The event as it is now", "Event", "New start", "New end", "Who is notified",
  ]);
  const value = (term: string) => facts.find((f) => f.term === term)?.values;
  assert.deepEqual(value("What it wants to do"), ["Change an event"]);
  assert.deepEqual(value("The event as it is now"), ["The broker did not send it. Only the event's id is shown."]);
  assert.deepEqual(value("Event"), ["evt-practice-1"]);
  assert.deepEqual(value("New start"), ["Friday, November 20, 2026 at 2:00 PM Central Standard Time"]);
  assert.deepEqual(value("Who is notified"), ["Nobody gets an email."]);
  for (const term of ["New title", "New place", "New description"]) assert.equal(value(term), undefined, term);
});

test("the card of a delete, without the event as it is now", () => {
  const facts = asked(waiting(remove("none")));
  assert.deepEqual(facts.map((f) => f.term), ["What it wants to do", "Account", "Calendar", "The event as it is now", "Event", "Who is notified"]);
  assert.deepEqual(facts[0]?.values, ["Delete an event"]);
});

test("an update that changes nothing says so; an answer to one occurrence names it; an event as it is now replaces the note", () => {
  const empty = asked(waiting({ ...moveTime("none"), arguments: { ...moveTime("none").arguments, changes: NO_CHANGES } } as Action));
  const terms = empty.map((f) => f.term);
  assert.equal(terms.indexOf("What changes"), terms.indexOf("Event") + 1);
  assert.deepEqual(empty.find((f) => f.term === "What changes")?.values, ["Nothing: the request names no new value."]);
  const answer = asked(waiting({
    tool: "fabric_calendar_propose_response",
    arguments: {
      account_id: "spike-account", calendar_id: "spike-calendar", response: "accepted", send_updates: "none", task: "Accept the next one.",
      target: {
        kind: "occurrence",
        occurrence: { instance_id: "evt-weekly_20261127", revision: "3181161784712000", original_start_time: { date_time: "2026-11-27T10:00:00-06:00", zone: "America/Chicago" } },
      },
    },
  }));
  const answerTerms = answer.map((f) => f.term);
  assert.equal(answerTerms.indexOf("Which occurrence"), answerTerms.indexOf("Event") + 1);
  assert.ok(answerTerms.indexOf("The event as it is now") < answerTerms.indexOf("Event"));
  assert.deepEqual(answer.find((f) => f.term === "Which occurrence")?.values,
    ["The one planned for Friday, November 27, 2026 at 10:00 AM Central Standard Time"]);
  const invitation = cardFacts(message("accept-owner-response-get-request-details-full-invitation.json").result, NAMES);
  assert.equal(invitation.find((f) => f.term === "The event as it is now"), undefined);
});

test("no text that a practice run can show names Google", () => {
  const { account_disconnected: _, ...ends } = GRANT_END_TEXT;
  const texts: string[] = [
    ...[
      STATE_TEXT, ALLOWED_TEXT, DURATION_TEXT, OWNER_CAN_DO, PROOF_PURPOSE, PROOF_OUTCOME, VERDICT_TEXT, ATTEMPT_TEXT, PROVER_TEXT, STORE_TEXT, ends,
      PROVER_PROBLEM, UNDECIDED_REASON,
    ].flatMap((words) => Object.values(words)),
    PROVER_PROBLEM_UNKNOWN, UNDECIDED_REASON_UNKNOWN,
  ];
  const create = details(request("pending_approval", 4));
  const cards = [create, ...(["all", "external_only", "none"] as const).flatMap((u) => [waiting(moveTime(u)), waiting(remove(u))])];
  for (const d of cards) {
    for (const f of cardFacts(d, NAMES)) if (f.untrusted !== true) texts.push(f.term, ...f.values);
  }
  assert.ok(texts.length > 100);
  for (const t of texts) assert.doesNotMatch(t, /google/i);
});

test("a configuration error: the app's settings or the installation", () => {
  assert.deepEqual(bannerText({ name: "configuration_error", detail: "the broker is missing" }), {
    title: "Fabric Calendar is not installed correctly.",
    text: "The app found a problem before it started the broker, so nothing was started. If you started the app with settings of your own, as for a test run, correct them and start it again. Otherwise installing the app again may fix it.",
  });
});
