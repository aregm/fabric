// The canonical form (README section 3) and the digest (section 4).
import { createHash } from "node:crypto";
import type { Json } from "./strict-json.ts";

// Orders two strings by Unicode code point. JavaScript's default order compares
// UTF-16 code units, which puts U+10000 and above before U+E000..U+FFFF.
export function compareCodePoints(a: string, b: string): number {
  let i = 0;
  while (i < a.length && i < b.length) {
    const x = a.codePointAt(i) ?? 0;
    const y = b.codePointAt(i) ?? 0;
    if (x !== y) return x - y;
    i += x > 0xffff ? 2 : 1;
  }
  return a.length - b.length;
}

const SHORT_ESCAPES: Readonly<Record<number, string>> = {
  0x08: "\\b", 0x09: "\\t", 0x0a: "\\n", 0x0c: "\\f", 0x0d: "\\r", 0x22: '\\"', 0x5c: "\\\\",
};

function canonicalString(s: string): string {
  if (!s.isWellFormed()) throw new TypeError("a string with a lone surrogate has no canonical form");
  let out = '"';
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    const short = SHORT_ESCAPES[c];
    if (short !== undefined) out += short;
    else if (c < 0x20) out += "\\u00" + c.toString(16).padStart(2, "0");
    else out += s[i];
  }
  return out + '"';
}

export function canonical(value: Json): string {
  if (value === null) return "null";
  if (value === true) return "true";
  if (value === false) return "false";
  if (typeof value === "number") {
    if (!Number.isSafeInteger(value)) throw new TypeError(`not a contract integer: ${value}`);
    return String(value);
  }
  if (typeof value === "string") return canonicalString(value);
  if (Array.isArray(value)) return "[" + value.map(canonical).join(",") + "]";
  const object = value as { readonly [key: string]: Json };
  const names = Object.keys(object).sort(compareCodePoints);
  return "{" + names.map((n) => canonicalString(n) + ":" + canonical(object[n] as Json)).join(",") + "}";
}

export function sha256Hex(text: string): string {
  return createHash("sha256").update(text, "utf8").digest("hex");
}

export function digest(domain: string, value: Json): string {
  return "sha256:" + sha256Hex("fabric-desktop/1\n" + domain + "\n" + canonical(value));
}
