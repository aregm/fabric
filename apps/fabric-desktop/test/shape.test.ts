// The shape checks against the accept and reject lists of the contract's scalar fixtures.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { CONTRACT_ROOT } from "../../../contracts/desktop/v1/typescript/src/judge.ts";
import { isLabel, isLocalTime, isZone, normalAddress, normalDomain } from "../ui/src/shape.ts";
import { contract } from "./support.ts";

const SCALARS: Record<string, { accept: unknown[]; reject: unknown[] }> = JSON.parse(readFileSync(`${CONTRACT_ROOT}fixtures/scalars.json`, "utf8"));
const COMMON = "common.schema.json#/$defs/";

function lists(name: string): { accept: string[]; reject: string[] } {
  const l = SCALARS[COMMON + name];
  assert.ok(l !== undefined, name);
  const strings = (xs: unknown[]): string[] => xs.filter((x): x is string => typeof x === "string");
  assert.ok(l.accept.every((x) => typeof x === "string"));
  return { accept: strings(l.accept), reject: strings(l.reject) };
}

for (const [name, check] of [["label", isLabel], ["iana_zone", isZone], ["local_time", isLocalTime]] as const) {
  test(`${name}: accepts what the contract accepts and refuses what it refuses`, () => {
    const { accept, reject } = lists(name);
    for (const v of accept) assert.equal(check(v), true, JSON.stringify(v));
    for (const v of reject) assert.equal(check(v), false, JSON.stringify(v));
  });
}

// A normal form keeps an accepted value as it is. A refused value is either refused, or turned
// into another value that the contract accepts (surrounding spaces dropped, the domain lowered).
for (const [name, normal] of [["email", normalAddress], ["domain", normalDomain]] as const) {
  test(`${name}: the normal form of the contract's values`, () => {
    const { accept, reject } = lists(name);
    for (const v of accept) assert.equal(normal(v), v, JSON.stringify(v));
    for (const v of reject) {
      const n = normal(v);
      assert.notEqual(n, v, JSON.stringify(v));
      if (n !== null) assert.equal(contract.validate(COMMON + name, n), true, `${JSON.stringify(v)} -> ${n}`);
    }
  });
}

test("normal forms: spaces dropped, the domain lowered, the part before the @ kept as typed", () => {
  assert.equal(normalAddress("  Bob.Smith@Example.COM \t"), "Bob.Smith@example.com");
  assert.equal(normalAddress("a@b"), null);
  assert.equal(normalAddress("a@Kelvin.com"), null, "only ASCII letters are lowered");
  assert.equal(normalDomain(" Mail.Example.com "), "mail.example.com");
  assert.equal(normalDomain("*.example.com"), null);
});
