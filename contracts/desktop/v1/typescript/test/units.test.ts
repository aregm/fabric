// Unit tests of the strict reader, the canonical form and the judge.
import assert from "node:assert/strict";
import { test } from "node:test";
import { canonical, compareCodePoints, digest, sha256Hex } from "../src/canonical.ts";
import { checkLifecycle, judge, loadContract } from "../src/judge.ts";
import { encodeText } from "../src/report.ts";
import { parseStrict } from "../src/strict-json.ts";

const parse = (text: string) => parseStrict(encodeText(text));
const refused = (text: string) => assert.throws(() => parse(text), SyntaxError, JSON.stringify(text));

test("the strict reader reads every kind of value", () => {
  assert.deepEqual(parse(' {"a" : [1, -2, true, false, null, "x"]}\r\n'), { a: [1, -2, true, false, null, "x"] });
  assert.equal(parse('"\\u00e9\\ud83d\\ude00\\/"'), "é😀/");
  assert.equal(parse("9007199254740991"), 9007199254740991);
  assert.equal(parse("-9007199254740991"), -9007199254740991);
});

test("the strict reader keeps __proto__ an ordinary member", () => {
  const v = parse('{"__proto__":{"x":1}}') as Record<string, unknown>;
  assert.deepEqual(Object.keys(v), ["__proto__"]);
  assert.equal(Object.getPrototypeOf(v), Object.prototype);
  refused('{"__proto__":1,"__proto__":2}');
});

test("the strict reader refuses what section 2 refuses", () => {
  for (const t of ["", " ", "{", "}", "[1,]", "01", "-", "-0", "1.0", "1e2", "9007199254740992", "-9007199254740992",
    '"\\ud800"', '"\\udc00"', '"\\ud800\\u0041"', '"\\u12"', '"\\U0041"', '"\t"', "\u000b1", "1 2", "tru", "nul",
    '{"a":1,"\\u0061":2}', "[".repeat(33) + "]".repeat(33)]) {
    refused(t);
  }
  assert.doesNotThrow(() => parse("[".repeat(32) + "]".repeat(32)));
  assert.doesNotThrow(() => parse('{"a":'.repeat(31) + "1" + "}".repeat(31)));
  refused('{"a":'.repeat(33) + "1" + "}".repeat(33));
  assert.throws(() => parseStrict(Uint8Array.from([0xef, 0xbb, 0xbf, 0x7b, 0x7d])), SyntaxError);
  assert.throws(() => parseStrict(Uint8Array.from([0x22, 0xed, 0xa0, 0x80, 0x22])), SyntaxError);
  assert.throws(() => parseStrict(Uint8Array.from([0x22, 0xff, 0x22])), SyntaxError);
});

test("the canonical form", () => {
  assert.equal(canonical({ b: 1, a: [true, null, "x"] }), '{"a":[true,null,"x"],"b":1}');
  assert.equal(canonical("\u0000\u001f\b\t\n\f\r\"\\/\u007f "), '"\\u0000\\u001f\\b\\t\\n\\f\\r\\"\\\\/\u007f "');
  assert.equal(canonical({ "😀": 1, "￿": 2, "": 3 }), '{"":3,"￿":2,"😀":1}');
  assert.ok(compareCodePoints("￿", "😀") < 0);
  assert.ok(compareCodePoints("a", "ab") < 0);
  assert.equal(compareCodePoints("ab", "ab"), 0);
  assert.throws(() => canonical(1.5), TypeError);
});

test("the digest", () => {
  assert.equal(sha256Hex(""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
  assert.equal(digest("command", {}), "sha256:" + sha256Hex("fabric-desktop/1\ncommand\n{}"));
});

test("the judge applies the layers in order", () => {
  const c = loadContract();
  const req = "owner-channel.schema.json#/$defs/owner_request";
  const j = (entry: string, text: string) => {
    const r = judge(c, entry, encodeText(text));
    return r.verdict === "accept" ? "accept" : `${r.layer} ${r.error}`;
  };
  assert.equal(j(req, '{"protocol":"fabric-desktop/1","id":"a","method":"get_snapshot","params":{}}'), "accept");
  assert.equal(j(req, "[]"), "frame malformed_frame");
  assert.equal(j(req, '{"protocol":"fabric-desktop/2","method":1}'), "protocol unsupported_protocol");
  assert.equal(j(req, '{"protocol":"fabric-desktop/1","id":"a","method":"fabric_calendar_list","params":{}}'), "schema unknown_method");
  assert.equal(j(req, '{"protocol":"fabric-desktop/1","id":"a","method":"get_snapshot","params":[]}'), "schema invalid_params");
  // The protocol layer and the size bound are for the five frame entries only.
  assert.equal(j("owner-channel.schema.json#/$defs/health", '{"prover":"ready","store":"ready"}'), "accept");
  assert.equal(j("owner-channel.schema.json#/$defs/health", '{"protocol":"x"}'), "schema invalid_params");
  assert.equal(j("owner-channel.schema.json#/$defs/health", '"ready"'), "frame malformed_frame");
  const big = '{"protocol":"fabric-desktop/1","id":"a","method":"subscribe","params":{"cursor":1}}';
  assert.equal(j(req, big + " ".repeat(1048576 - big.length)), "accept");
  assert.equal(j(req, big + " ".repeat(1048577 - big.length)), "frame malformed_frame");
});

test("the lifecycle check", () => {
  const c = loadContract();
  assert.deepEqual(checkLifecycle(c, []), { valid: false, firstInvalid: 0 });
  assert.deepEqual(checkLifecycle(c, ["received", "validating"]), { valid: true, firstInvalid: null });
  assert.deepEqual(checkLifecycle(c, ["received", "queued"]), { valid: false, firstInvalid: 1 });
});
