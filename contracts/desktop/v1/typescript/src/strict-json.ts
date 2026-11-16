// The strict JSON reader of README section 2. It turns bytes into a value or
// throws a SyntaxError. JSON.parse is not used: it cannot see duplicate
// members, number tokens or escaped lone surrogates.

export type Json = null | boolean | number | string | readonly Json[] | { readonly [key: string]: Json };

const MAX_SAFE = 9007199254740991n;
const MAX_DEPTH = 32;
const INTEGER = /-?(0|[1-9][0-9]*)/y;
const HEX4 = /^[0-9A-Fa-f]{4}$/;
const SIMPLE_ESCAPES: Readonly<Record<string, string>> = {
  '"': '"', "\\": "\\", "/": "/", b: "\b", f: "\f", n: "\n", r: "\r", t: "\t",
};

function fail(message: string): never {
  throw new SyntaxError(message);
}

export function parseStrict(bytes: Uint8Array): Json {
  let text: string;
  try {
    text = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes);
  } catch {
    return fail("the bytes are not valid UTF-8");
  }
  if (text.charCodeAt(0) === 0xfeff) fail("the text begins with a byte-order mark");
  return parseText(text);
}

function parseText(text: string): Json {
  let i = 0;

  function skipWhitespace(): void {
    while (i < text.length) {
      const c = text[i];
      if (c !== " " && c !== "\t" && c !== "\n" && c !== "\r") return;
      i++;
    }
  }

  function expect(c: string): void {
    if (text[i] !== c) fail(`expected '${c}' at offset ${i}`);
    i++;
  }

  // `depth` is the depth of the value being read; the outermost value is 1.
  function value(depth: number): Json {
    const c = text[i];
    if (c === "{") return object(depth);
    if (c === "[") return array(depth);
    if (c === '"') return string();
    if (text.startsWith("true", i)) return (i += 4), true;
    if (text.startsWith("false", i)) return (i += 5), false;
    if (text.startsWith("null", i)) return (i += 4), null;
    return integer();
  }

  function object(depth: number): Json {
    if (depth > MAX_DEPTH) fail("nesting deeper than 32");
    i++;
    const out: Record<string, Json> = {};
    skipWhitespace();
    if (text[i] === "}") return i++, out;
    for (;;) {
      if (text[i] !== '"') fail(`expected a member name at offset ${i}`);
      const name = string();
      if (Object.hasOwn(out, name)) fail(`duplicate member ${JSON.stringify(name)}`);
      skipWhitespace();
      expect(":");
      skipWhitespace();
      // defineProperty, so that a member named __proto__ is an ordinary member.
      Object.defineProperty(out, name, { value: value(depth + 1), enumerable: true, writable: true, configurable: true });
      skipWhitespace();
      if (text[i] === "}") return i++, out;
      expect(",");
      skipWhitespace();
    }
  }

  function array(depth: number): Json {
    if (depth > MAX_DEPTH) fail("nesting deeper than 32");
    i++;
    const out: Json[] = [];
    skipWhitespace();
    if (text[i] === "]") return i++, out;
    for (;;) {
      out.push(value(depth + 1));
      skipWhitespace();
      if (text[i] === "]") return i++, out;
      expect(",");
      skipWhitespace();
    }
  }

  function hex4(): number {
    const digits = text.slice(i, i + 4);
    if (!HEX4.test(digits)) fail(`bad \\u escape at offset ${i}`);
    i += 4;
    return parseInt(digits, 16);
  }

  function string(): string {
    i++;
    let out = "";
    let start = i;
    for (;;) {
      if (i >= text.length) fail("unterminated string");
      const c = text.charCodeAt(i);
      if (c === 0x22) {
        out += text.slice(start, i);
        i++;
        return out;
      }
      if (c < 0x20) fail(`raw control character in a string at offset ${i}`);
      if (c !== 0x5c) {
        i++;
        continue;
      }
      out += text.slice(start, i);
      i++;
      const e = text[i++] ?? "";
      if (e === "u") {
        const unit = hex4();
        if (unit >= 0xdc00 && unit <= 0xdfff) fail("lone low surrogate");
        if (unit >= 0xd800 && unit <= 0xdbff) {
          if (text[i] !== "\\" || text[i + 1] !== "u") fail("lone high surrogate");
          i += 2;
          const low = hex4();
          if (low < 0xdc00 || low > 0xdfff) fail("lone high surrogate");
          out += String.fromCharCode(unit, low);
        } else {
          out += String.fromCharCode(unit);
        }
      } else {
        const s = SIMPLE_ESCAPES[e];
        if (s === undefined) fail(`invalid escape at offset ${i - 1}`);
        out += s;
      }
      start = i;
    }
  }

  function integer(): number {
    INTEGER.lastIndex = i;
    const m = INTEGER.exec(text);
    if (m === null) fail(`unexpected character at offset ${i}`);
    const token = m[0];
    if (token === "-0") fail("-0 is refused");
    const n = BigInt(token);
    if (n > MAX_SAFE || n < -MAX_SAFE) fail("integer outside ±9007199254740991");
    i += token.length;
    // A fraction or an exponent leaves '.', 'e' or 'E' here, which no rule accepts.
    return Number(n);
  }

  skipWhitespace();
  if (i >= text.length) fail("no value");
  const result = value(1);
  skipWhitespace();
  if (i !== text.length) fail(`unexpected text after the value at offset ${i}`);
  return result;
}
