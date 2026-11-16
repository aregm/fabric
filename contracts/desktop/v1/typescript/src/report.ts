// The report of README section 13, and the mutant judge.
//   node src/report.ts report [contract dir]
//   node src/report.ts judge <mutants.jsonl> [contract dir]
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { canonical, digest, sha256Hex } from "./canonical.ts";
import { CONTRACT_ROOT, checkLifecycle, judge, loadContract } from "./judge.ts";
import type { Contract } from "./judge.ts";
import { parseStrict } from "./strict-json.ts";
import type { Json } from "./strict-json.ts";

// One fixture source: exactly one of value, text, hex, generate.
export interface Source {
  readonly value?: Json;
  readonly text?: string;
  readonly hex?: string;
  readonly generate?: { readonly prefix: string; readonly fill: string; readonly count: number; readonly suffix: string };
}

// UTF-8 that keeps a lone surrogate as its three-byte form, so that the strict
// reader sees it and refuses it (TextEncoder would replace it with U+FFFD).
export function encodeText(text: string): Uint8Array {
  const out: number[] = [];
  for (const ch of text) {
    const c = ch.codePointAt(0) ?? 0;
    if (c < 0x80) out.push(c);
    else if (c < 0x800) out.push(0xc0 | (c >> 6), 0x80 | (c & 63));
    else if (c < 0x10000) out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
    else out.push(0xf0 | (c >> 18), 0x80 | ((c >> 12) & 63), 0x80 | ((c >> 6) & 63), 0x80 | (c & 63));
  }
  return Uint8Array.from(out);
}

export function sourceBytes(e: Source): Uint8Array {
  const present = (["value", "text", "hex", "generate"] as const).filter((k) => k in e);
  if (present.length !== 1) throw new Error(`a fixture needs exactly one source, found ${present.join(", ") || "none"}`);
  if (e.text !== undefined) return encodeText(e.text);
  if (e.hex !== undefined) {
    if (!/^([0-9a-f]{2})*$/.test(e.hex)) throw new Error("bad hex source");
    return Uint8Array.from(Buffer.from(e.hex, "hex"));
  }
  if (e.generate !== undefined) {
    const g = e.generate;
    return encodeText(g.prefix + g.fill.repeat(g.count) + g.suffix);
  }
  return encodeText(canonical(e.value as Json));
}

export function sortBytewise(lines: string[]): string[] {
  return lines.sort((a, b) => Buffer.compare(Buffer.from(a, "utf8"), Buffer.from(b, "utf8")));
}

function readJson(path: string): any {
  return JSON.parse(readFileSync(path, "utf8"));
}

// Digests of an accepted frame: `null` when the frame is not a command or not an action.
export function frameDigests(contract: Contract, entry: string, value: Json): [string | null, string | null] {
  const v = value as { readonly method: string; readonly params: Json };
  const command =
    entry.endsWith("#/$defs/owner_request") && contract.commands.includes(v.method)
      ? digest("command", { method: v.method, params: v.params })
      : null;
  const action =
    entry.endsWith("#/$defs/agent_request") && contract.actions.includes(v.method)
      ? digest("action", { tool: v.method, arguments: v.params })
      : null;
  return [command, action];
}

export function report(root: string): { lines: string[]; problems: string[] } {
  const contract = loadContract(root);
  const fixtures = join(root, "fixtures");
  const lines: string[] = [];
  const problems: string[] = [];

  const strict = readJson(join(fixtures, "strict-json.json"));
  for (const [list, expected] of [["accept", true], ["reject", false]] as const) {
    for (const e of strict[list] as (Source & { name: string; canonical?: string })[]) {
      const verdict = judgeText(e);
      if (verdict === null) {
        lines.push(`strict\t${e.name}\treject`);
        if (expected) problems.push(`strict ${e.name}: refused, but it is in the accept list`);
      } else {
        lines.push(`strict\t${e.name}\taccept\t${sha256Hex(verdict)}`);
        if (!expected) problems.push(`strict ${e.name}: accepted, but it is in the reject list`);
        else if (verdict !== e.canonical) problems.push(`strict ${e.name}: canonical form differs`);
      }
    }
  }

  const scalars = readJson(join(fixtures, "scalars.json"));
  for (const [ref, lists] of Object.entries(scalars as Record<string, Record<string, Json[]>>)) {
    for (const list of ["accept", "reject"] as const) {
      (lists[list] ?? []).forEach((v, i) => {
        const ok = contract.validate(ref, v);
        lines.push(`scalar\t${ref}\t${list}\t${i}\t${ok ? "accept" : "reject"}`);
        if (ok !== (list === "accept")) problems.push(`scalar ${ref} ${list}[${i}]: computed ${ok ? "accept" : "reject"}`);
      });
    }
  }

  for (const e of readJson(join(fixtures, "digests.json"))) {
    const d = digest(e.domain, e.subject);
    lines.push(`digest\t${e.name}\t${d}`);
    if (canonical(e.subject) !== e.canonical) problems.push(`digest ${e.name}: canonical form differs`);
    if (d !== e.digest) problems.push(`digest ${e.name}: digest differs`);
  }

  for (const e of readJson(join(fixtures, "lifecycle.json"))) {
    const r = checkLifecycle(contract, e.states);
    lines.push(`lifecycle\t${e.name}\t${r.valid ? "valid" : "invalid"}\t${r.firstInvalid ?? "-"}`);
    if (r.valid !== e.valid || r.firstInvalid !== e.first_invalid) problems.push(`lifecycle ${e.name}: differs`);
  }

  const messages = join(fixtures, "messages");
  const files = existsSync(messages) ? readdirSync(messages).filter((f) => f.endsWith(".json")).sort() : [];
  for (const f of files) {
    const e = readJson(join(messages, f));
    const x = e.expect;
    const r = judge(contract, e.schema, sourceBytes(e));
    if (r.verdict === "accept") {
      const c = canonical(r.value);
      const [cd, ad] = frameDigests(contract, e.schema, r.value);
      lines.push(`message\t${f}\taccept\t${sha256Hex(c)}\t${cd ?? "-"}\t${ad ?? "-"}`);
      if (x.verdict !== "accept") problems.push(`message ${f}: accepted, expected ${x.verdict} ${x.layer} ${x.error}`);
      else {
        if (x.canonical !== c) problems.push(`message ${f}: canonical form differs`);
        if (x.command_digest !== cd) problems.push(`message ${f}: command digest ${cd} differs from ${x.command_digest}`);
        if (x.action_digest !== ad) problems.push(`message ${f}: action digest ${ad} differs from ${x.action_digest}`);
      }
    } else {
      lines.push(`message\t${f}\treject\t${r.layer}\t${r.error}`);
      if (x.verdict !== "reject" || x.layer !== r.layer || x.error !== r.error) {
        problems.push(`message ${f}: computed reject ${r.layer} ${r.error}, expected ${x.verdict} ${x.layer ?? ""} ${x.error ?? ""}`);
      }
    }
  }
  return { lines: sortBytewise(lines), problems };

  // The canonical text of a strict-json fixture, or null when it is refused.
  function judgeText(e: Source): string | null {
    try {
      // The strict reader alone: these fixtures are not frames.
      return canonical(parseStrict(sourceBytes(e)));
    } catch (err) {
      if (err instanceof SyntaxError) return null;
      throw err;
    }
  }
}

export function judgeMutants(root: string, path: string): string[] {
  const contract = loadContract(root);
  const out: string[] = [];
  for (const line of readFileSync(path, "utf8").split("\n")) {
    if (line.trim() === "") continue;
    const m = JSON.parse(line) as { id: string; schema: string; text: string };
    const r = judge(contract, m.schema, encodeText(m.text));
    out.push(r.verdict === "accept" ? `mutant\t${m.id}\taccept` : `mutant\t${m.id}\treject\t${r.layer}\t${r.error}`);
  }
  return sortBytewise(out);
}

function main(argv: readonly string[]): number {
  const [command, a, b] = argv;
  if (command === "report") {
    const { lines, problems } = report(a ?? CONTRACT_ROOT);
    process.stdout.write(lines.map((l) => l + "\n").join(""));
    for (const p of problems) process.stderr.write(`disagreement: ${p}\n`);
    return problems.length === 0 ? 0 : 1;
  }
  if (command === "judge" && a !== undefined) {
    process.stdout.write(judgeMutants(b ?? CONTRACT_ROOT, a).map((l) => l + "\n").join(""));
    return 0;
  }
  process.stderr.write("usage: node src/report.ts report [contract dir]\n       node src/report.ts judge <mutants.jsonl> [contract dir]\n");
  return 2;
}

if (import.meta.main) process.exitCode = main(process.argv.slice(2));
