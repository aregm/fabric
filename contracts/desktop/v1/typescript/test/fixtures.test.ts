// Every fixture entry, computed by this side and compared with its expectation.
import assert from "node:assert/strict";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import { canonical, digest } from "../src/canonical.ts";
import { CONTRACT_ROOT, checkLifecycle, judge, loadContract } from "../src/judge.ts";
import { frameDigests, sourceBytes } from "../src/report.ts";
import { parseStrict } from "../src/strict-json.ts";

const FIXTURES = join(CONTRACT_ROOT, "fixtures");
const contract = loadContract();
const read = (name: string): any => JSON.parse(readFileSync(join(FIXTURES, name), "utf8"));

test("strict-json.json", async (t) => {
  const strict = read("strict-json.json");
  for (const e of strict.accept) {
    await t.test(`accept ${e.name}`, () => {
      assert.equal(canonical(parseStrict(sourceBytes(e))), e.canonical, `strict-json ${e.name}: canonical form`);
    });
  }
  for (const e of strict.reject) {
    await t.test(`reject ${e.name}`, () => {
      assert.throws(() => parseStrict(sourceBytes(e)), SyntaxError, `strict-json ${e.name}: must be refused`);
    });
  }
});

test("scalars.json", async (t) => {
  for (const [ref, lists] of Object.entries(read("scalars.json") as Record<string, Record<string, any[]>>)) {
    await t.test(ref, () => {
      for (const list of ["accept", "reject"]) {
        (lists[list] ?? []).forEach((v, i) => {
          assert.equal(contract.validate(ref, v), list === "accept", `scalars ${ref} ${list}[${i}] = ${JSON.stringify(v)}`);
        });
      }
    });
  }
});

test("digests.json", async (t) => {
  for (const e of read("digests.json")) {
    await t.test(e.name, () => {
      assert.equal(canonical(e.subject), e.canonical, `digests ${e.name}: canonical form`);
      assert.equal(digest(e.domain, e.subject), e.digest, `digests ${e.name}: digest`);
    });
  }
});

test("lifecycle.json", async (t) => {
  for (const e of read("lifecycle.json")) {
    await t.test(e.name, () => {
      const r = checkLifecycle(contract, e.states);
      assert.deepEqual([r.valid, r.firstInvalid], [e.valid, e.first_invalid], `lifecycle ${e.name}`);
    });
  }
});

test("messages/", async (t) => {
  const dir = join(FIXTURES, "messages");
  assert.ok(existsSync(dir), "fixtures/messages/ is missing");
  const files = readdirSync(dir).filter((f) => f.endsWith(".json")).sort();
  assert.ok(files.length >= 100, `fixtures/messages/ holds ${files.length} files, fewer than 100`);
  for (const f of files) {
    await t.test(f, () => {
      const e = JSON.parse(readFileSync(join(dir, f), "utf8"));
      const sources = ["value", "text", "hex", "generate"].filter((k) => k in e);
      assert.deepEqual(Object.keys(e).sort(), ["description", "expect", "schema", ...sources].sort(), `message ${f}: members`);
      assert.equal(sources.length, 1, `message ${f}: exactly one source`);
      const r = judge(contract, e.schema, sourceBytes(e));
      if (r.verdict === "accept") {
        const [cd, ad] = frameDigests(contract, e.schema, r.value);
        assert.deepEqual(
          { verdict: "accept", canonical: canonical(r.value), command_digest: cd, action_digest: ad },
          e.expect,
          `message ${f}`,
        );
      } else {
        assert.deepEqual({ verdict: "reject", layer: r.layer, error: r.error }, e.expect, `message ${f}`);
      }
    });
  }
});
