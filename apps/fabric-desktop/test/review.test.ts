// The Review screen's model (review.ts) against the fake host: requests and their details, proofs,
// taking a permission back, pausing and resuming agents. No browser, no DOM.
import assert from "node:assert/strict";
import { test } from "node:test";
import type { OwnerError, Request, RequestDetails, Snapshot } from "../../../contracts/desktop/v1/typescript/src/contract.ts";
import { ReviewModel } from "../ui/src/review.ts";
import { NO_PROOF, UNDECIDED_REASON } from "../ui/src/text.ts";
import { assertRequestsValid, connect, details, entry, fixture, flush, request, setup } from "./support.ts";

const RICH: Snapshot = fixture("accept-owner-response-get-snapshot-rich.json").result;
const result = (name: string): any => fixture(name).result;
const BLOCKED: Request = fixture("accept-owner-notice-request-blocked.json").change.request;
const APPLIED: RequestDetails = result("accept-owner-response-get-request-details-applied-create.json");
const FAILED: RequestDetails = result("accept-owner-response-get-request-details-terminal-failure.json");
const CONFLICT: OwnerError = fixture("accept-owner-response-error-conflict.json").error; // current_revision 8
const NOT_FOUND: OwnerError = fixture("accept-owner-response-error-not-found.json").error;
// request-30 of agent-claude-desktop, revision 3, undecided, redacted; one per recorded reason.
const UNDECIDED = (reason: string): RequestDetails => result(`accept-owner-response-get-request-details-undecided-${reason}.json`);

// A live model on the rich snapshot (cursor 1287), its request-17 card read, nothing announced.
function live() {
  const s = setup();
  connect(s.host, 1, RICH);
  const pending = RICH.requests[0];
  assert.ok(pending !== undefined);
  s.host.reply("get_request_details", details(pending));
  s.announced.length = 0;
  let sequence = RICH.cursor;
  const notice = (change: Parameters<typeof s.host.notice>[1]): void => s.host.notice(++sequence, change);
  return { ...s, review: new ReviewModel(s.model), notice };
}

const count = (host: { methods(): string[] }, method: string): number => host.methods().filter((m) => m === method).length;

test("a blocked request keeps its rule, the broker's explanation and its proof id (DEF-7)", () => {
  const { host, model, review, notice } = live();
  notice({ kind: "request", request: BLOCKED });
  const e = entry(model, "request-22");
  assert.deepEqual(review.lines(e), {
    ruling: "Rule recurring_mutation_unsupported: Version 1 does not change recurring events; only a reply to one occurrence is supported.",
    proofId: "proof-22-classification", reason: null, help: null,
  });
  assert.equal(count(host, "get_request_details"), 1, "a blocked request is not read unless its details are opened");
  // A request without a card: the details say what was asked.
  review.toggleDetails("request-22");
  host.reply("get_request_details", result("accept-owner-response-get-request-details-blocked.json"));
  const asked = review.details(e)?.asked ?? [];
  assert.equal(asked.find((f) => f.untrusted === true)?.values[0], "Rename the event.");
  assert.deepEqual(asked.find((f) => f.term === "New title")?.values, ["Shared with everyone"]);
});

test("Show the proof sends get_proof and gives the report's facts, failing rule first; a proof is read once", () => {
  const { host, model, review, notice } = live();
  notice({ kind: "request", request: BLOCKED });
  review.toggleProof("request-22:ruling", "proof-22-classification");
  assert.deepEqual(host.last("get_proof").params, { proof_id: "proof-22-classification" });
  assert.equal(review.proofShown("request-22:ruling", "proof-22-classification"), true);
  assert.equal(review.proofStatus("proof-22-classification"), "Reading the proof…");
  host.reply("get_proof", result("accept-owner-response-get-proof-block.json"));
  assert.equal(review.proofStatus("proof-22-classification"), null);
  const facts = review.proofFacts("proof-22-classification");
  const value = (term: string) => facts.find((f) => f.term === term)?.values;
  assert.deepEqual(value("What it was for"), ["Deciding whether the request is allowed, needs your approval or is blocked"]);
  assert.deepEqual(value("Outcome"), ["Blocked"]);
  assert.deepEqual(value("Failing rule"), ["recurring_mutation_unsupported"]);
  assert.deepEqual(value("Rules"), ["recurring_mutation_unsupported: fails", "agent_paused: holds"]);
  assert.deepEqual(value("Isabelle version"), ["Isabelle2025"]);
  assert.deepEqual(value("Took"), ["287 ms"]);
  assert.match(value("Started")?.[0] ?? "", /Central Standard Time/);
  // Hidden and shown again, or shown at another place: no second read.
  review.toggleProof("request-22:ruling", "proof-22-classification");
  assert.equal(review.proofShown("request-22:ruling", "proof-22-classification"), false);
  review.toggleProof("request-22:ruling", "proof-22-classification");
  review.toggleProof("request-22:proofs", "proof-22-classification");
  assert.equal(count(host, "get_proof"), 1);
  assert.equal(model.ready, true);
});

test("a proof the broker does not have: not_found is said in words", () => {
  const { host, review } = live();
  review.toggleProof("request-17:why", "spike-no-proof");
  host.fail("get_proof", NOT_FOUND);
  assert.equal(review.proofStatus("spike-no-proof"), NO_PROOF);
  assert.deepEqual(review.proofFacts("spike-no-proof"), []);
});

test("an applied request's details: the decision, the proofs in the broker's order, the authorization, both attempts, the event", () => {
  const { host, model, review, notice } = live();
  notice({ kind: "request", request: APPLIED.request });
  const e = entry(model, "request-25");
  assert.equal(count(host, "get_request_details"), 1, "not read while its details are closed");
  review.toggleDetails("request-25");
  assert.deepEqual(host.last("get_request_details").params, { request_id: "request-25", redaction: "full" });
  assert.equal(review.detailsStatus(e), "Reading the details…");
  host.reply("get_request_details", APPLIED);
  assert.equal(review.detailsStatus(e), null);
  const d = review.details(e);
  assert.ok(d !== null);
  const terms = (facts: readonly { term: string }[]) => facts.map((f) => f.term);
  assert.deepEqual(terms(d.general), ["Received", "Last change", "Account", "Request digest"]);
  assert.deepEqual(d.general[2]?.values, ["owner@example.com"]);
  assert.deepEqual(d.asked, [], "the approval card on the page shows what was asked");
  assert.deepEqual(d.decision.find((f) => f.term === "Decision")?.values, ["You allowed it"]);
  assert.deepEqual(d.decision.find((f) => f.term === "How long it lasts")?.values, ["Only this request"]);
  assert.deepEqual(d.decision.find((f) => f.term === "Approved: calendars")?.values, ["Owner (owner@example.com)"]);
  assert.deepEqual(d.decision.find((f) => f.term === "Permission it created")?.values, ["grant-8"]);
  assert.deepEqual(d.proofIds, APPLIED.proof_ids);
  assert.deepEqual(terms(d.authorization), ["Issued", "Expires", "Used"]);
  assert.deepEqual(d.attempts.map((a) => [a.number, a.proofId, a.facts.find((f) => f.term === "Result")?.values[0]]), [
    [1, "proof-25-permit-1", "The calendar did not take it; Fabric tried again"],
    [2, "proof-25-permit-2", "The calendar accepted it"],
  ]);
  assert.deepEqual(d.event.map((f) => f.values[0]), ["evt-fabric-0051", "3181162000000000"]);
});

test("the details of a request that waits again show no decision of an earlier round; a decided request's do", () => {
  const { host, model, review, notice } = live();
  const pending = request("pending_approval", 4);
  notice({ kind: "request", request: pending });
  host.reply("get_request_details", { ...details(pending), approval: APPLIED.approval });
  review.toggleDetails("spike-request-1");
  host.reply("get_request_details", { ...details(pending), approval: APPLIED.approval });
  assert.deepEqual(review.details(entry(model))?.decision, []);
  const approved = request("approved", 5);
  notice({ kind: "request", request: approved });
  host.reply("get_request_details", { ...details(approved), approval: APPLIED.approval });
  assert.deepEqual(review.details(entry(model))?.decision.find((f) => f.term === "Decision")?.values, ["You allowed it"]);
});

test("a failed request shows its reason; redacted details say that the broker did not send them", () => {
  const { host, model, review, notice } = live();
  notice({ kind: "request", request: FAILED.request });
  const e = entry(model, "request-26");
  assert.equal(review.lines(e).reason, "Reason: The event changed at Google after the agent read it, so the update was not applied.");
  review.toggleDetails("request-26");
  host.reply("get_request_details", FAILED);
  const d = review.details(e);
  assert.deepEqual(d?.asked.find((f) => f.term === "Details")?.values, ["The broker did not send the details of this request."]);
  assert.deepEqual(d?.event, []);
});

// The get_request_details frames the model sent for one request.
const reads = (host: { sent: any[] }, id: string): any[] =>
  host.sent.filter((f) => f.method === "get_request_details" && f.params.request_id === id).map((f) => f.params);

test("an undecided request is read redacted for its recorded reason, and its row says the reason", () => {
  for (const [name, reason] of [["prover-unavailable", "prover_unavailable"], ["out-of-time", "out_of_time"], ["failed", "failed"]] as const) {
    const { host, model, review, notice, announced } = live();
    const d = UNDECIDED(name);
    notice({ kind: "request", request: d.request });
    assert.deepEqual(reads(host, "request-30"), [{ request_id: "request-30", redaction: "redacted" }], name);
    const e = entry(model, "request-30");
    assert.equal(review.lines(e).reason, null, "not read yet");
    host.reply("get_request_details", d);
    assert.equal(review.lines(e).reason, `Reason: ${UNDECIDED_REASON[reason]}`);
    assert.equal(e.details, null, "the redacted answer is not kept as the details");
    assert.equal(e.note, null);
    assert.deepEqual(announced, []);
    assertRequestsValid(host);
  }
  assert.equal(UNDECIDED_REASON.prover_unavailable, "The rule checker was starting or not running.");
});

test("an undecided request without a recorded reason has no reason line, also while the rule checker is not running, and is not asked again", () => {
  const { host, model, review, notice } = live();
  notice({ kind: "health", health: { prover: "unavailable", prover_problem: null, store: "ready" } });
  const d = UNDECIDED("reason-unknown");
  notice({ kind: "request", request: d.request });
  host.reply("get_request_details", d);
  const e = entry(model, "request-30");
  assert.equal(review.lines(e).reason, null);
  assert.deepEqual(e.undecided, { reason: null });
  notice({ kind: "request", request: d.request });
  model.refresh();
  host.reply("get_snapshot", { ...RICH, cursor: model.cursor });
  host.reply("subscribe", { cursor: model.cursor });
  assert.equal(reads(host, "request-30").length, 1, "a reason read once is not asked again");
});

test("the redacted reason read does not replace the details the card was read with", () => {
  const { host, model, review, notice } = live();
  const e = entry(model, "request-17");
  assert.notEqual(e.details?.card, null);
  const pending = e.request;
  notice({ kind: "request", request: { ...pending, revision: pending.revision + 1, status: { state: "approved" } } });
  const undecided: Request = { ...pending, revision: pending.revision + 2, status: { state: "undecided" } };
  notice({ kind: "request", request: undecided });
  assert.deepEqual(reads(host, "request-17").at(-1), { request_id: "request-17", redaction: "redacted" });
  host.reply("get_request_details", { ...UNDECIDED("out-of-time"), request: undecided });
  assert.equal(review.lines(e).reason, `Reason: ${UNDECIDED_REASON.out_of_time}`);
  assert.notEqual(e.details?.card ?? null, null, "the card's details are kept");
  assert.equal(e.details?.redaction, "full");
  assert.equal(model.cardStatus(e).kind, "closed");
  assert.equal(e.request.status.state, "undecided");
});

test("a failed reason read leaves no note and is tried again after the next snapshot", () => {
  const { host, model, review, notice, announced } = live();
  const d = UNDECIDED("failed");
  notice({ kind: "request", request: d.request });
  host.error("get_request_details", "internal");
  const e = entry(model, "request-30");
  assert.equal(e.note, null);
  assert.equal(review.lines(e).reason, null);
  assert.deepEqual(announced, []);
  assert.equal(reads(host, "request-30").length, 1, "not tried again from the failure");
  model.refresh();
  host.reply("get_snapshot", { ...RICH, cursor: model.cursor });
  assert.deepEqual(reads(host, "request-30"), [
    { request_id: "request-30", redaction: "redacted" }, { request_id: "request-30", redaction: "redacted" },
  ]);
  host.reply("get_request_details", d);
  assert.equal(review.lines(e).reason, `Reason: ${UNDECIDED_REASON.failed}`);
});

test("with the details open, an undecided request is read in full only, and that read gives the reason and the fact", () => {
  const { host, model, review, notice } = live();
  const d = UNDECIDED("prover-unavailable");
  const checking: Request = { ...d.request, revision: 2, status: { state: "proving_classification" } };
  notice({ kind: "request", request: checking });
  review.toggleDetails("request-30");
  host.reply("get_request_details", { ...d, request: checking, undecided_reason: null });
  notice({ kind: "request", request: d.request });
  assert.deepEqual(reads(host, "request-30"), [
    { request_id: "request-30", redaction: "full" }, { request_id: "request-30", redaction: "full" },
  ]);
  host.reply("get_request_details", d);
  const e = entry(model, "request-30");
  assert.equal(review.lines(e).reason, `Reason: ${UNDECIDED_REASON.prover_unavailable}`);
  assert.deepEqual(review.details(e)?.general.map((f) => f.term), ["Received", "Last change", "Why it was not decided", "Account", "Request digest"]);
  assert.equal(reads(host, "request-30").length, 2, "no reason read besides");
});

test("the details of an undecided request say why it was not decided; another state has no such fact", () => {
  const { model, review } = live();
  const base = entry(model, "request-17");
  const why = (d: RequestDetails) => review.details({ ...base, details: d })?.general.find((f) => f.term === "Why it was not decided")?.values;
  assert.deepEqual(why(UNDECIDED("prover-unavailable")), ["The rule checker was starting or not running."]);
  assert.deepEqual(why(UNDECIDED("out-of-time")), ["The rule checker did not finish within its time limit."]);
  assert.deepEqual(why(UNDECIDED("failed")), ["The rule checker ran and did not decide the request, or its report could not be used."]);
  assert.deepEqual(why(UNDECIDED("reason-unknown")), ["The broker did not record why."]);
  assert.equal(why(APPLIED), undefined);
  assert.equal(why(FAILED), undefined);
});

test("an unknown outcome says what the owner can do today", () => {
  const { model, review, notice } = live();
  notice({ kind: "request", request: { ...APPLIED.request, revision: 10, status: { state: "outcome_unknown" } } });
  assert.match(review.lines(entry(model, "request-25")).help ?? "", /Look in your calendar/);
  notice({ kind: "request", request: { ...APPLIED.request, revision: 11, status: { state: "needs_owner_resolution" } } });
  assert.match(review.lines(entry(model, "request-25")).help ?? "", /no way to mark it as resolved/);
});

test("open details are read again when a newer revision arrives; closed ones are not", () => {
  const { host, review, notice } = live();
  notice({ kind: "request", request: { ...APPLIED.request, revision: 8, status: { state: "dispatching" } } });
  notice({ kind: "request", request: APPLIED.request });
  assert.equal(count(host, "get_request_details"), 1, "closed: not read");
  review.toggleDetails("request-25");
  assert.equal(count(host, "get_request_details"), 2);
  host.reply("get_request_details", APPLIED);
  notice({ kind: "request", request: { ...APPLIED.request, revision: 10, status: { state: "reconciled" } } });
  assert.equal(count(host, "get_request_details"), 3, "open: read again for the newer revision");
  host.reply("get_request_details", { ...APPLIED, request: { ...APPLIED.request, revision: 10, status: { state: "reconciled" } } });
  review.toggleDetails("request-25");
  notice({ kind: "request", request: { ...APPLIED.request, revision: 11, status: { state: "reconciled" } } });
  assert.equal(count(host, "get_request_details"), 3, "closed again: not read");
});

test("the counts of the requests", () => {
  const { review, notice } = live();
  assert.deepEqual(review.counts, { waiting: 1, inProgress: 1, ended: 0, unavailable: 0 });
  notice({ kind: "request", request: BLOCKED });
  notice({ kind: "request", request: FAILED.request });
  assert.equal(review.countsText(), "1 waits for your decision, 1 in progress, 2 ended while this window was open.");
});

test("revoke: the frame, in flight once, success announced and the grant kept as ended", () => {
  const { host, model, review, announced, notice } = live();
  assert.equal(review.revokeBlocked("grant-6"), null);
  assert.equal(review.revoke("grant-6"), true);
  assert.deepEqual(host.last("revoke_grant").params, { grant_id: "grant-6", expected_revision: 2 });
  assert.equal(review.revokeBlocked("grant-6"), "Taking it back…");
  assert.equal(review.revoke("grant-6"), false, "not sent twice");
  assert.equal(count(host, "revoke_grant"), 1);
  const revoked = result("accept-owner-response-revoke-grant.json");
  notice({ kind: "grant", grant: revoked.grant });
  host.reply("revoke_grant", revoked);
  assert.deepEqual(announced, ["You took back the permission of Claude Desktop."]);
  assert.equal(review.revokeState("grant-6")?.inFlight, false);
  assert.deepEqual(review.grants().map((g) => g.grant_id), ["grant-4", "grant-5", "grant-6"], "the ended one stays, after the live ones");
  assert.equal(model.grants.get("grant-6")?.ended?.reason, "revoked_by_owner");
  assert.equal(review.revoke("grant-6"), false, "an ended grant has no button");
  const facts = review.grantFacts(revoked.grant);
  assert.deepEqual(facts.map((f) => f.term).filter((t) => /^(What it|How long)/.test(t)), ["What it allowed", "How long it was given for"]);
  assert.deepEqual(facts.find((f) => f.term === "Given for request")?.values, ["request-9"], "a request the page does not hold: its id");
  const live6 = review.grantFacts({ ...revoked.grant, ended: null, origin: { ...revoked.grant.origin, request_id: "request-17" } });
  assert.deepEqual(live6.map((f) => f.term).filter((t) => /^(What it|How long)/.test(t)), ["What it may do", "How long it lasts"]);
  assert.deepEqual(live6.find((f) => f.term === "Given for request")?.values,
    ["Change who is invited to an event, from Claude Desktop (request request-17)"]);
  assert.match(facts.find((f) => f.term === "Ended")?.values[0] ?? "", /You took it back\.$/);
  assert.deepEqual(facts.find((f) => f.term === "Given under")?.values, ["The profile Assistant hours, version 2"]);
  assert.deepEqual(facts.find((f) => f.term === "How long it was given for")?.values, ["Until you take it back. It has no end date."]);
  assert.deepEqual(facts.find((f) => f.term === "Groups")?.values, ["Team", "Family (Google contacts)"]);
  assert.equal(facts[0]?.untrusted, true);
});

test("revoke: a conflict changed nothing and the page reads the current state", () => {
  const { host, review, announced } = live();
  review.revoke("grant-5");
  host.fail("revoke_grant", CONFLICT);
  assert.equal(review.revokeState("grant-5")?.note, "This permission changed just now, so nothing was taken back. Look at it again, then try again.");
  assert.equal(announced.at(-1), review.revokeState("grant-5")?.note);
  assert.equal(host.methods().at(-1), "get_snapshot");
});

test("revoke: no answer says it is not known, and the retry is the same command", async () => {
  const { host, review } = live();
  host.refusal = { kind: "io", detail: "broken pipe" };
  review.revoke("grant-4");
  const first = host.last("revoke_grant").id;
  await flush();
  assert.match(review.revokeState("grant-4")?.note ?? "", /^It is not known whether the permission was taken back\. Taking it back again is safe\./);
  host.refusal = null;
  review.revoke("grant-4");
  assert.equal(host.last("revoke_grant").id, first);
  host.error("revoke_grant", "internal");
  assert.equal(review.revokeState("grant-4")?.note, "The broker did not take the permission back: Refused by the test.");
});

test("revoke and pause are not sent while the broker is not ready", () => {
  const { host, model, review } = live();
  host.state({ name: "disconnected", reason: "exited", detail: "exit code 1" });
  const sent = host.raw.length;
  assert.equal(review.revokeBlocked("grant-6"), model.notReadyText);
  assert.equal(review.pauseBlocked("agent-claude-code"), "Not available while the broker is not running.");
  assert.equal(review.revoke("grant-6"), false);
  assert.equal(review.pause("agent-claude-code", true), false);
  assert.equal(review.pauseAll(true), false);
  assert.equal(host.raw.length, sent);
});

test("pause and resume one agent, pause and resume all: the frames and the announcements", () => {
  const { host, review, announced, notice } = live();
  const one = result("accept-owner-response-pause-agent-one.json");
  assert.equal(review.pause("agent-claude-code", true), true);
  assert.deepEqual(host.last("pause_agent").params, { target: { kind: "agent", agent_id: "agent-claude-code", expected_revision: 5 }, paused: true });
  assert.equal(review.pauseBlocked("agent-claude-code"), "Sending…");
  assert.equal(review.pause("agent-claude-code", true), false);
  notice({ kind: "agent", agent: one.agents[0] });
  host.reply("pause_agent", one);
  assert.equal(announced.at(-1), "Claude Code is paused.");
  review.pause("agent-claude-code", false);
  assert.deepEqual(host.last("pause_agent").params, { target: { kind: "agent", agent_id: "agent-claude-code", expected_revision: 6 }, paused: false });
  host.reply("pause_agent", { commit_sequence: 1, agents: [{ ...one.agents[0], revision: 7, paused: false }] });
  assert.equal(announced.at(-1), "Claude Code can work again.");
  review.pauseAll(true);
  assert.deepEqual(host.last("pause_agent").params, { target: { kind: "all_agents" }, paused: true });
  assert.equal(review.pauseBlocked(null), "Sending…");
  host.reply("pause_agent", result("accept-owner-response-pause-all-agents.json"));
  assert.equal(announced.at(-1), "All agents are paused.");
  review.pauseAll(false);
  assert.deepEqual(host.last("pause_agent").params, { target: { kind: "all_agents" }, paused: false });
  host.reply("pause_agent", result("accept-owner-response-pause-all-agents.json"));
  assert.equal(announced.at(-1), "All agents can work again.");
});

test("pause: a conflict changed nothing; the page refreshes only when it does not hold the newer record", () => {
  const { host, review, notice, model } = live();
  review.pause("agent-claude-code", true);
  host.fail("pause_agent", CONFLICT);
  assert.equal(review.agentState("agent-claude-code")?.note, "This agent's record changed just now, so nothing was paused. Try again.");
  assert.equal(host.methods().at(-1), "get_snapshot");
  host.reply("get_snapshot", RICH);
  host.reply("subscribe", { cursor: RICH.cursor });
  assert.equal(model.ready, true);
  // The newer record arrived as a notice before the answer: no refresh.
  review.pause("agent-ollama", false);
  const agent = RICH.agents.find((a) => a.agent_id === "agent-ollama");
  assert.ok(agent !== undefined);
  notice({ kind: "agent", agent: { ...agent, revision: 8, paused: true } });
  const sent = host.raw.length;
  host.fail("pause_agent", CONFLICT);
  assert.equal(host.raw.length, sent);
  assert.equal(review.agentState("agent-ollama")?.note, "This agent's record changed just now, so nothing was resumed. Try again.");
});

test("an agent's facts in words", () => {
  const { model, review } = live();
  const facts = (id: string) => {
    const a = model.agents.get(id);
    assert.ok(a !== undefined);
    return Object.fromEntries(review.agentFacts(a).map((f) => [f.term, f.values[0]]));
  };
  assert.deepEqual(facts("agent-codex-desktop"), {
    "Where its data goes": "Leaves this computer: OpenAI", Connected: "Not connected", "Last call": "None since the broker started",
    Paused: "Yes: it can do nothing until you resume it",
  });
  assert.match(facts("agent-claude-code").Connected ?? "", /^Since Friday, November 20, 2026/);
});

test("every command and query of the screen sends a valid owner request", async () => {
  const { host, model, review, notice } = live();
  notice({ kind: "request", request: BLOCKED });
  review.toggleProof("request-22:ruling", "proof-22-classification");
  review.toggleDetails("request-22");
  model.chooseDuration("request-17", "until_revoked");
  model.decide("request-17", "approve");
  review.revoke("grant-6");
  review.pause("agent-claude-code", true);
  review.pauseAll(false);
  host.refusal = { kind: "io", detail: "broken pipe" };
  review.revoke("grant-5");
  await flush();
  host.refusal = null;
  review.revoke("grant-5");
  assert.deepEqual(new Set(host.methods()), new Set([
    "hello", "get_snapshot", "subscribe", "get_request_details", "get_proof", "decide_request", "revoke_grant", "pause_agent",
  ]));
  assertRequestsValid(host);
});

test("an open card says why you are asked and when it expires; a closed one why you were asked, without the expiry", () => {
  const { host, model, review } = live();
  const e = entry(model, "request-17");
  assert.deepEqual(review.whyFacts(e).map((f) => f.term), ["Why you are asked", "This card expires"]);
  model.decide("request-17", "deny");
  const denied = { ...e.request, revision: e.request.revision + 1, status: { state: "denied" } };
  host.reply("decide_request", {
    commit_sequence: 1, request: denied,
    approval: {
      decision: "deny", command_id: host.last("decide_request").id, decided_at: e.request.updated_at,
      approval_digest: e.request.request_digest, scope: null, duration: null, effective_request_digest: null, grant_id: null,
    },
  });
  assert.equal(model.cardStatus(e).kind, "closed");
  const why = review.whyFacts(e);
  assert.deepEqual(why.map((f) => f.term), ["Why you were asked"]);
  assert.equal(why[0]?.values[1], "Rule: spike_no_rule_theory");
});
