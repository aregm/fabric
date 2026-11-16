// The words on the page: dates name their zone, and the card's facts come in reading order.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { CONTRACT_ROOT } from "../../../contracts/desktop/v1/typescript/src/judge.ts";
import { cardFacts, formatInstant } from "../ui/src/text.ts";

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
