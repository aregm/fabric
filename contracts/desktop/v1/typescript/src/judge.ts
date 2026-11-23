// The layers of refusal (README sections 5 and 13) and the lifecycle check (section 9).
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { NON_TOOL_METHODS } from "./contract.ts";
import { createValidator, loadSchemaFiles } from "./schema.ts";
import type { Validate } from "./schema.ts";
import { parseStrict } from "./strict-json.ts";
import type { Json } from "./strict-json.ts";

export const CONTRACT_ROOT = fileURLToPath(new URL("../../", import.meta.url));

export const FRAME_ENTRIES: readonly string[] = [
  "owner-channel.schema.json#/$defs/owner_request",
  "owner-channel.schema.json#/$defs/owner_response",
  "owner-channel.schema.json#/$defs/owner_notice",
  "agent-channel.schema.json#/$defs/agent_request",
  "agent-channel.schema.json#/$defs/agent_response",
];
const OWNER_REQUEST = "owner-channel.schema.json#/$defs/owner_request";
const AGENT_REQUEST = "agent-channel.schema.json#/$defs/agent_request";

export interface Contract {
  readonly validate: Validate;
  readonly maxFrameBytes: number;
  readonly ownerMethods: readonly string[];
  readonly commands: readonly string[];
  readonly tools: readonly string[];
  // The methods of the agent channel: the tools and the two handshake methods (README section 8).
  readonly agentMethods: readonly string[];
  readonly actions: readonly string[];
  readonly initialState: string;
  readonly transitions: ReadonlySet<string>;
}

export type Verdict =
  | { readonly verdict: "accept"; readonly value: Json }
  | {
      readonly verdict: "reject";
      readonly layer: "frame" | "protocol" | "schema";
      readonly error: "malformed_frame" | "unsupported_protocol" | "unknown_method" | "invalid_params";
    };

function readData(root: string, name: string): any {
  return JSON.parse(readFileSync(join(root, "data", name), "utf8"));
}

export function loadContract(root: string = CONTRACT_ROOT): Contract {
  const methods: { method: string; command: boolean }[] = readData(root, "owner-methods.json").methods;
  const tools: { name: string; creates_request: boolean }[] = readData(root, "tool-catalog.json").tools;
  const lifecycle = readData(root, "request-lifecycle.json");
  return {
    validate: createValidator(loadSchemaFiles(join(root, "schemas"))),
    maxFrameBytes: readData(root, "limits.json").limits.max_frame_bytes,
    ownerMethods: methods.map((m) => m.method),
    commands: methods.filter((m) => m.command).map((m) => m.method),
    tools: tools.map((t) => t.name),
    agentMethods: [...NON_TOOL_METHODS, ...tools.map((t) => t.name)],
    actions: tools.filter((t) => t.creates_request).map((t) => t.name),
    initialState: lifecycle.initial,
    transitions: new Set(lifecycle.transitions.map((t: { from: string; to: string }) => `${t.from} ${t.to}`)),
  };
}

export function judge(contract: Contract, entry: string, bytes: Uint8Array): Verdict {
  const isFrame = FRAME_ENTRIES.includes(entry);
  // A frame is one line: it is bounded and holds no line feed.
  if (isFrame && (bytes.length > contract.maxFrameBytes || bytes.includes(0x0a))) {
    return { verdict: "reject", layer: "frame", error: "malformed_frame" };
  }
  let value: Json;
  try {
    value = parseStrict(bytes);
  } catch {
    return { verdict: "reject", layer: "frame", error: "malformed_frame" };
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return { verdict: "reject", layer: "frame", error: "malformed_frame" };
  }
  const object = value as { readonly [key: string]: Json };
  if (isFrame && object.protocol !== "fabric-desktop/1") {
    return { verdict: "reject", layer: "protocol", error: "unsupported_protocol" };
  }
  if (!contract.validate(entry, value)) {
    const method = object.method;
    const names = entry === OWNER_REQUEST ? contract.ownerMethods : entry === AGENT_REQUEST ? contract.agentMethods : null;
    const known = typeof method === "string" && names !== null && names.includes(method);
    return { verdict: "reject", layer: "schema", error: names !== null && !known ? "unknown_method" : "invalid_params" };
  }
  return { verdict: "accept", value };
}

// A sequence is valid when it is non-empty, starts at the initial state and
// every step is a listed transition. `firstInvalid` is null when it is valid.
export function checkLifecycle(
  contract: Contract,
  states: readonly string[],
): { readonly valid: boolean; readonly firstInvalid: number | null } {
  if (states.length === 0 || states[0] !== contract.initialState) return { valid: false, firstInvalid: 0 };
  for (let i = 1; i < states.length; i++) {
    if (!contract.transitions.has(`${states[i - 1]} ${states[i]}`)) return { valid: false, firstInvalid: i };
  }
  return { valid: true, firstInvalid: null };
}
