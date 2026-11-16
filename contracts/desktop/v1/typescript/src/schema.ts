// A validator for exactly the schema subset of README section 6.
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { canonical } from "./canonical.ts";
import type { Json } from "./strict-json.ts";

export const SCHEMA_DIR = fileURLToPath(new URL("../../schemas/", import.meta.url));

export const KEYWORDS: readonly string[] = [
  "$schema", "$defs", "$ref", "title", "description", "type", "const", "enum", "properties", "required",
  "additionalProperties", "items", "minItems", "maxItems", "uniqueItems", "minLength", "maxLength", "pattern",
  "minimum", "maximum", "oneOf",
];
const TYPES: readonly string[] = ["object", "array", "string", "integer", "boolean", "null"];
const REF = /^([a-z-]+\.schema\.json)?#\/\$defs\/([A-Za-z0-9_]+)$/;

type Schema = { readonly [keyword: string]: unknown };
export type SchemaFiles = { readonly [file: string]: Schema };
export type Validate = (ref: string, value: Json) => boolean;

export function loadSchemaFiles(dir: string = SCHEMA_DIR): SchemaFiles {
  const files: Record<string, Schema> = {};
  for (const name of readdirSync(dir).sort()) {
    if (name.endsWith(".schema.json")) files[name] = JSON.parse(readFileSync(join(dir, name), "utf8")) as Schema;
  }
  return files;
}

function isSchema(x: unknown): x is Schema {
  return typeof x === "object" && x !== null && !Array.isArray(x);
}

// Throws unless every schema node uses only the subset. `where` names the node.
function checkNode(s: unknown, where: string): void {
  if (!isSchema(s)) throw new Error(`${where}: a schema must be an object`);
  for (const k of Object.keys(s)) {
    if (!KEYWORDS.includes(k)) throw new Error(`${where}: keyword ${k} is outside the subset`);
  }
  if ("$ref" in s) {
    if (typeof s.$ref !== "string" || !REF.test(s.$ref)) throw new Error(`${where}: bad $ref ${String(s.$ref)}`);
    if (Object.keys(s).some((k) => k !== "$ref" && k !== "description")) throw new Error(`${where}: $ref has siblings`);
  }
  if ("type" in s && !TYPES.includes(s.type as string)) throw new Error(`${where}: bad type ${String(s.type)}`);
  if ("pattern" in s && !/^\^.*\$$/s.test(String(s.pattern))) throw new Error(`${where}: pattern is not anchored`);
  for (const k of ["$defs", "properties"]) {
    if (k in s) for (const [n, sub] of Object.entries(s[k] as Schema)) checkNode(sub, `${where}/${k}/${n}`);
  }
  if ("items" in s) checkNode(s.items, `${where}/items`);
  if ("oneOf" in s) (s.oneOf as unknown[]).forEach((sub, i) => checkNode(sub, `${where}/oneOf/${i}`));
  if ("additionalProperties" in s && typeof s.additionalProperties !== "boolean") {
    checkNode(s.additionalProperties, `${where}/additionalProperties`);
  }
}

export function checkSubset(files: SchemaFiles): void {
  for (const [name, schema] of Object.entries(files)) checkNode(schema, name);
}

// Returns a validator for references like `common.schema.json#/$defs/id`.
export function createValidator(files: SchemaFiles = loadSchemaFiles()): Validate {
  checkSubset(files);
  const regexps = new Map<string, RegExp>();

  function resolve(file: string, ref: string): [string, Schema] {
    const m = REF.exec(ref);
    if (m === null) throw new Error(`bad reference ${ref}`);
    const target = m[1] ?? file;
    const defs = files[target]?.$defs as SchemaFiles | undefined;
    const node = defs?.[m[2] as string];
    if (node === undefined) throw new Error(`unresolved reference ${ref} from ${file}`);
    return [target, node];
  }

  function typeOk(type: unknown, v: Json): boolean {
    switch (type) {
      case "object": return typeof v === "object" && v !== null && !Array.isArray(v);
      case "array": return Array.isArray(v);
      case "string": return typeof v === "string";
      case "integer": return typeof v === "number" && Number.isInteger(v);
      case "boolean": return typeof v === "boolean";
      case "null": return v === null;
    }
    throw new Error(`bad type ${String(type)}`);
  }

  function ok(file: string, s: Schema, v: Json): boolean {
    if (typeof s.$ref === "string") {
      const [f, node] = resolve(file, s.$ref);
      return ok(f, node, v);
    }
    if ("type" in s && !typeOk(s.type, v)) return false;
    if ("const" in s && canonical(s.const as Json) !== canonical(v)) return false;
    if ("enum" in s && !(s.enum as Json[]).some((e) => canonical(e) === canonical(v))) return false;
    if (typeof v === "string") {
      const length = [...v].length;
      if (typeof s.minLength === "number" && length < s.minLength) return false;
      if (typeof s.maxLength === "number" && length > s.maxLength) return false;
      if (typeof s.pattern === "string") {
        let re = regexps.get(s.pattern);
        if (re === undefined) regexps.set(s.pattern, (re = new RegExp(s.pattern, "u")));
        if (!re.test(v)) return false;
      }
    }
    if (typeof v === "number") {
      if (typeof s.minimum === "number" && v < s.minimum) return false;
      if (typeof s.maximum === "number" && v > s.maximum) return false;
    }
    if (Array.isArray(v)) {
      if (typeof s.minItems === "number" && v.length < s.minItems) return false;
      if (typeof s.maxItems === "number" && v.length > s.maxItems) return false;
      if (s.uniqueItems === true && new Set(v.map(canonical)).size !== v.length) return false;
      if (isSchema(s.items) && !v.every((x) => ok(file, s.items as Schema, x))) return false;
    } else if (typeof v === "object" && v !== null) {
      const object = v as { readonly [key: string]: Json };
      const properties = (s.properties ?? {}) as SchemaFiles;
      const required = (s.required ?? []) as string[];
      if (!required.every((n) => Object.hasOwn(object, n))) return false;
      for (const [n, x] of Object.entries(object)) {
        const sub = Object.hasOwn(properties, n) ? properties[n] : s.additionalProperties;
        if (sub === false) return false;
        if (isSchema(sub) && !ok(file, sub, x)) return false;
      }
    }
    if (Array.isArray(s.oneOf) && s.oneOf.filter((b: Schema) => ok(file, b, v)).length !== 1) return false;
    return true;
  }

  return (ref, value) => {
    const [file, node] = resolve("", ref);
    return ok(file, node, value);
  };
}
