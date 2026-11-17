// Every fixture entry, computed by this side and compared with its expectation.
import assert from "node:assert/strict";
import { createPublicKey, verify } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import { canonical, digest } from "../src/canonical.ts";
import { handshakeText, pairingCode } from "../src/handshake.ts";
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

// An Ed25519 public key as a DER SubjectPublicKeyInfo: the fixed prefix of RFC 8410 and the 32 bytes.
function ed25519Key(publicKey: string) {
  const raw = Buffer.from(publicKey.replace(/^ed25519:/, ""), "hex");
  assert.equal(raw.length, 32, "an Ed25519 public key has 32 bytes");
  const der = Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), raw]);
  return createPublicKey({ key: der, format: "der", type: "spki" });
}

test("handshake.json", async (t) => {
  const entries = read("handshake.json");
  assert.ok(entries.length >= 3, "handshake.json holds fewer than three entries");
  for (const e of entries) {
    await t.test(e.name, () => {
      assert.equal(pairingCode(e.public_key), e.pairing_code, `handshake ${e.name}: pairing code`);
      const text = handshakeText(e.schema_version, e.enrollment_id, e.public_key, e.broker_nonce);
      assert.equal(text, e.text, `handshake ${e.name}: text`);
      const signature = Buffer.from(e.signature, "hex");
      assert.ok(verify(null, Buffer.from(text, "utf8"), ed25519Key(e.public_key), signature), `handshake ${e.name}: signature`);
      const other = Buffer.from(text.replace(e.broker_nonce, "0".repeat(64)), "utf8");
      assert.ok(!verify(null, other, ed25519Key(e.public_key), signature), `handshake ${e.name}: another nonce`);
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
