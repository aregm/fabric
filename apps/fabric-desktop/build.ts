// Builds dist/: copies the static files and strips the types of each ui/src/*.ts with Node.
// Nothing is bundled. A run-time import from outside ui/src fails the build; type-only imports
// (the contract types) are erased by the stripping and never reach dist/.
import { copyFileSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { stripTypeScriptTypes } from "node:module";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = fileURLToPath(new URL(".", import.meta.url));
const SOURCE = join(ROOT, "ui", "src");
const DIST = join(ROOT, "dist");

// stripTypeScriptTypes warns once that it is experimental. That one warning is silenced; every
// other warning still goes to the default handler.
const emit = process.emitWarning.bind(process);
process.emitWarning = ((warning: string | Error, ...rest: unknown[]) => {
  const message = typeof warning === "string" ? warning : warning.message;
  if (/stripTypeScriptTypes/.test(message)) return;
  (emit as (w: string | Error, ...r: unknown[]) => void)(warning, ...rest);
}) as typeof process.emitWarning;

// Static `import … from "x"`, `export … from "x"`, side-effect `import "x"` and `import("x")`.
const SPECIFIER = /(\bfrom\s*|\bimport\s*\(?\s*)(["'])([^"']+)\2/g;

rmSync(DIST, { recursive: true, force: true });
mkdirSync(DIST);
for (const name of ["index.html", "styles.css"]) copyFileSync(join(ROOT, "ui", name), join(DIST, name));

const problems: string[] = [];
for (const name of readdirSync(SOURCE).filter((n) => n.endsWith(".ts")).sort()) {
  const js = stripTypeScriptTypes(readFileSync(join(SOURCE, name), "utf8"), { mode: "strip" });
  const rewritten = js.replace(SPECIFIER, (_all, lead: string, quote: string, spec: string) => {
    if (!/^\.\/[A-Za-z0-9_-]+\.ts$/.test(spec)) {
      problems.push(`ui/src/${name} imports ${spec} at run time; only ./<file>.ts inside ui/src is allowed`);
      return _all;
    }
    return `${lead}${quote}${spec.slice(0, -3)}.js${quote}`;
  });
  writeFileSync(join(DIST, name.replace(/\.ts$/, ".js")), rewritten);
}
if (problems.length > 0) {
  rmSync(DIST, { recursive: true, force: true });
  for (const p of problems) console.error(`build: ${p}`);
  process.exit(1);
}
console.log(`build: wrote ${readdirSync(DIST).sort().join(", ")} to dist/`);
