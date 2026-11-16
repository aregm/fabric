// The schema files, and the TypeScript constants that mirror them and the data files.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import * as c from "../src/contract.ts";
import { CONTRACT_ROOT } from "../src/judge.ts";
import { checkSubset, createValidator, loadSchemaFiles } from "../src/schema.ts";

const files = loadSchemaFiles();
const common: any = files["common.schema.json"]?.$defs;
const owner: any = files["owner-channel.schema.json"]?.$defs;
const agent: any = files["agent-channel.schema.json"]?.$defs;
const data = (name: string): any => JSON.parse(readFileSync(join(CONTRACT_ROOT, "data", name), "utf8"));
const sorted = (xs: readonly string[]): string[] => [...xs].sort();

test("the three schema files are there and use only the subset", () => {
  assert.deepEqual(Object.keys(files), ["agent-channel.schema.json", "common.schema.json", "owner-channel.schema.json"]);
  checkSubset(files);
});

test("the validator throws on anything outside the subset", () => {
  const bad = [
    { type: "string", format: "email" },
    { type: "number" },
    { $ref: "#/$defs/a", type: "string" },
    { $ref: "http://example.com/x.json" },
    { anyOf: [{ type: "null" }] },
    { type: "object", properties: { a: { type: "string", default: "x" } }, required: ["a"], additionalProperties: false },
  ];
  for (const s of bad) {
    assert.throws(() => createValidator({ "x.schema.json": { $defs: { a: s } } }), Error, JSON.stringify(s));
  }
});

test("every object schema lists its properties, requires all of them and forbids others", () => {
  const problems: string[] = [];
  function walk(s: any, where: string): void {
    if (typeof s !== "object" || s === null) return;
    if (s.type === "object" || "properties" in s) {
      const names = Object.keys(s.properties ?? {});
      if (!("properties" in s)) problems.push(`${where}: no properties`);
      if (JSON.stringify(sorted(s.required ?? [])) !== JSON.stringify(sorted(names))) problems.push(`${where}: required`);
      if (s.additionalProperties !== false) problems.push(`${where}: additionalProperties`);
    }
    for (const k of ["$defs", "properties"]) for (const [n, x] of Object.entries(s[k] ?? {})) walk(x, `${where}/${k}/${n}`);
    if ("items" in s) walk(s.items, `${where}/items`);
    (s.oneOf ?? []).forEach((x: any, i: number) => walk(x, `${where}/oneOf/${i}`));
  }
  for (const [name, s] of Object.entries(files)) walk(s, name);
  assert.deepEqual(problems, []);
});

test("vocabularies equal the schema enums", () => {
  const pairs: [string, readonly string[], readonly string[]][] = [
    ["weekday", c.WEEKDAYS, common.weekday.enum],
    ["event_field", c.EVENT_FIELDS, common.event_field.enum],
    ["send_updates", c.SEND_UPDATES, common.send_updates.enum],
    ["calendar_access", c.CALENDAR_ACCESS, common.calendar_access.enum],
    ["tool_name", c.TOOL_NAMES, common.tool_name.enum],
    ["request_state", c.REQUEST_STATES, common.request_state.enum],
    ["plain_request_state", c.PLAIN_REQUEST_STATES, common.plain_request_state.enum],
    ["failure_code", c.FAILURE_CODES, common.failure_code.enum],
    ["busy_limit", c.BUSY_LIMITS, common.busy_limit.enum],
    ["undecided_reason", c.UNDECIDED_REASONS, common.undecided_reason.enum],
    ["prover_problem", c.PROVER_PROBLEMS, owner.prover_problem.enum],
    ["mode", c.MODES, owner.mode.enum],
    ["policy_row", c.POLICY_ROWS, owner.policy_row.enum],
    ["profile_rows", c.POLICY_ROWS, Object.keys(owner.profile_rows.properties)],
    ["duration kind", c.DURATION_KINDS, owner.duration.properties.kind.enum],
    ["grant_end reason", c.GRANT_END_REASONS, owner.grant_end.properties.reason.enum],
    ["account state", c.ACCOUNT_STATES, owner.account.properties.state.enum],
    ["response_status", c.RESPONSE_STATUSES, agent.response_status.enum],
    ["owner_request methods", c.OWNER_METHODS, owner.owner_request.oneOf.map((b: any) => b.properties.method.const)],
    ["agent_request methods", c.TOOL_NAMES, agent.agent_request.oneOf.map((b: any) => b.properties.method.const)],
    ["action tools", c.ACTION_TOOLS, agent.action.oneOf.map((b: any) => b.properties.tool.const)],
    ["limits", Object.keys(c.LIMITS), Object.keys(common.limits.properties)],
  ];
  for (const [name, mine, theirs] of pairs) assert.deepEqual([...mine], theirs, name);

  const codes = new Set<string>();
  for (const b of [...common.owner_error.oneOf, ...common.agent_error.oneOf]) {
    codes.add(common[b.$ref.replace("#/$defs/", "")].properties.code.const);
  }
  const defined = Object.keys(common).filter((k) => k.startsWith("error_")).map((k) => common[k].properties.code.const);
  assert.deepEqual([...c.ERROR_CODES], [...codes], "error codes of owner_error and agent_error");
  assert.deepEqual(sorted(c.ERROR_CODES), sorted(defined), "error_* definitions");
});

test("the plain states and the three carrying states are exactly the 23 states", () => {
  const carrying = ["pending_approval", "blocked", "terminal_failure"];
  const all = sorted([...c.PLAIN_REQUEST_STATES, ...carrying]);
  assert.equal(all.length, 23);
  assert.deepEqual(all, sorted(common.request_state.enum));
  assert.deepEqual(all, sorted(data("request-lifecycle.json").states.map((s: any) => s.state)));
  for (const d of [owner.lifecycle_status, agent.agent_lifecycle_status]) {
    const consts = d.oneOf.map((b: any) => b.properties.state.const).filter((x: unknown) => x !== undefined);
    assert.deepEqual(sorted(consts), sorted(carrying));
  }
});

test("vocabularies equal the data files", () => {
  const methods = data("owner-methods.json").methods;
  const tools = data("tool-catalog.json").tools;
  const lifecycle = data("request-lifecycle.json");
  const matrix = data("policy-matrix.json");
  assert.deepEqual([...c.OWNER_METHODS], methods.map((m: any) => m.method));
  assert.deepEqual([...c.OWNER_COMMANDS], methods.filter((m: any) => m.command).map((m: any) => m.method));
  assert.deepEqual([...c.TOOL_NAMES], tools.map((t: any) => t.name));
  assert.deepEqual([...c.ACTION_TOOLS], tools.filter((t: any) => t.creates_request).map((t: any) => t.name));
  assert.deepEqual([...c.REQUEST_STATES], lifecycle.states.map((s: any) => s.state));
  assert.equal(lifecycle.initial, "received");
  // policy-matrix.json lists the modes from least to most permissive; the schema lists them the other way.
  assert.deepEqual(sorted(c.MODES), sorted(matrix.modes));
  assert.deepEqual([...c.POLICY_ROWS], matrix.rows.map((r: any) => r.row));
  for (const m of methods) {
    const name = m.params.replace("owner-channel.schema.json#/$defs/", "");
    const branch = owner.owner_request.oneOf.find((b: any) => b.properties.method.const === m.method);
    assert.equal(branch.properties.params.$ref, `#/$defs/${name}`, `${m.method} params`);
  }
  for (const t of tools) {
    const branch = agent.agent_request.oneOf.find((b: any) => b.properties.method.const === t.name);
    assert.equal(branch.properties.params.$ref, t.arguments.replace("agent-channel.schema.json", ""), `${t.name} arguments`);
  }
});

test("versions", () => {
  const v = data("version.json");
  assert.equal(c.PROTOCOL, v.protocol);
  assert.equal(c.SCHEMA_VERSION, v.schema_version);
  assert.equal(c.AUTHORIZATION_PROFILE, v.authorization_profile);
  assert.equal(common.protocol.const, c.PROTOCOL);
  assert.equal(owner.authorization.properties.profile.const, c.AUTHORIZATION_PROFILE);
});

test("the built-in profiles", () => {
  const validate = createValidator(files);
  const matrix = data("policy-matrix.json");
  for (const [p, def, column] of [
    [c.PROFILE_CAREFUL, owner.profile_careful, "careful"],
    [c.PROFILE_FLEXIBLE, owner.profile_flexible, "flexible"],
  ] as const) {
    const fromSchema = Object.fromEntries(Object.entries(def.properties).map(([k, s]: [string, any]) => [k, s.const]));
    assert.deepEqual(p, fromSchema, `${column} equals the schema constants`);
    assert.ok(validate("owner-channel.schema.json#/$defs/profile", p), `${column} is a profile`);
    const rows = Object.fromEntries(matrix.rows.map((r: any) => [r.row, r[column].mode]));
    assert.deepEqual(p.rows, rows, `${column} equals the policy matrix`);
  }
});

test("the limits", () => {
  assert.deepEqual(c.LIMITS, data("limits.json").limits);
  assert.ok(createValidator(files)("common.schema.json#/$defs/limits", c.LIMITS));
  assert.equal(owner.authorization.properties.lifetime_seconds.const, c.LIMITS.authorization_lifetime_seconds);
  assert.equal(c.LIMITS.max_frame_bytes, 1048576);
});
