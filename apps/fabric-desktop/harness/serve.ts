// A development tool, not shipped: serves dist/ on 127.0.0.1 and stands in for the Tauri host so
// the built interface can be opened in an ordinary browser. It implements the three commands of
// the host bridge (owner_attach, owner_send, broker_restart) over HTTP and server-sent events, by
// running the real broker executable as a child.
//
// Unlike the real host it does NOT validate frames in either direction: it passes the page's
// frames to the broker and the broker's lines to the page as they are. The checks of the
// contract's layers are the Tauri host's job (apps/fabric-desktop/relay); do not test them here.
//
// The broker always runs with the public test key and the synthetic agent, in one new temporary
// data directory per harness run (kept across restarts of the broker, removed when the harness
// stops). It never opens the real store and never touches the Keychain. The broker must be a
// debug build: a release build refuses the test key.
//
// A switch for a manual check of a store that cannot be opened: `POST /harness/hold-store`, allowed
// only while no broker of the harness runs, starts a second broker process on the same data
// directory (test key, no synthetic agent, its stdin kept open and its stdout discarded) that
// holds the store's lock; the next "Restart broker" from the page then starts a broker that
// answers `store_unavailable`. `POST /harness/release-store` ends that process (end of input,
// then a kill after 2 s). Stopping the harness ends it too.
//
//   node harness/serve.ts --broker <path to fabric-broker> [--port 4317]
import { spawn } from "node:child_process";
import type { ChildProcess } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { createServer } from "node:http";
import type { IncomingMessage, ServerResponse } from "node:http";
import { constants, tmpdir } from "node:os";
import { extname, join, resolve, sep } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const { values } = parseArgs({ options: { port: { type: "string", default: "4317" }, broker: { type: "string" } } });
if (values.broker === undefined) {
  console.error("usage: node harness/serve.ts --broker <path to fabric-broker> [--port 4317]");
  process.exit(64);
}
const BROKER = resolve(values.broker);
const PORT = Number(values.port);
const HERE = fileURLToPath(new URL(".", import.meta.url));
const DIST = join(HERE, "..", "dist");
const TYPES: Record<string, string> = { ".html": "text/html", ".css": "text/css", ".js": "text/javascript" };
// One scratch directory per harness run (mode 0700); the broker's store is its child "broker".
const SCRATCH = mkdtempSync(join(tmpdir(), "fabric-harness-"));
const BROKER_ENV = {
  FABRIC_BROKER_DATA_DIR: join(SCRATCH, "broker"),
  FABRIC_BROKER_KEY_SOURCE: "test",
  FABRIC_BROKER_SYNTHETIC_AGENT: "1",
};

type State =
  | { name: "connected" }
  | { name: "disconnected"; reason: "exited" | "protocol_violation" | "stopped" | "start_failed"; detail: string };

let run = 0; // broker starts: 0 before any, 1 for the first
let state: State = { name: "disconnected", reason: "stopped", detail: "not started yet" };
let child: ChildProcess | null = null;
let holder: ChildProcess | null = null; // the broker that holds the store's lock (hold-store)
let page: ServerResponse | null = null; // the one attached event stream; a new attach replaces it

function post(message: object): void {
  page?.write(`data: ${JSON.stringify(message)}\n\n`);
}

function setState(next: State): void {
  state = next;
  post({ kind: "state", run, state });
}

function startBroker(): void {
  run += 1;
  const thisRun = run;
  const c = spawn(BROKER, [], { stdio: ["pipe", "pipe", "inherit"], env: BROKER_ENV });
  child = c;
  c.on("error", (e) => {
    if (child === c) child = null;
    setState({ name: "disconnected", reason: "start_failed", detail: e.message });
  });
  c.on("spawn", () => {
    console.log(`harness: broker run ${thisRun} started, pid ${c.pid}`);
    setState({ name: "connected" });
  });
  c.on("exit", (code, signal) => {
    if (child !== c) return;
    child = null;
    const detail = signal !== null ? `killed by signal ${constants.signals[signal]}` : `exit code ${code}`;
    console.log(`harness: broker run ${thisRun} ended, ${detail}`);
    setState({ name: "disconnected", reason: "exited", detail });
  });
  createInterface({ input: c.stdout! }).on("line", (line) => {
    if (child !== c) return;
    try {
      post({ kind: "frame", run: thisRun, frame: JSON.parse(line) });
    } catch {
      console.error("harness: the broker wrote a line that is not JSON; stopping it");
      child = null;
      c.kill("SIGKILL");
      setState({ name: "disconnected", reason: "protocol_violation", detail: "a line that is not JSON" });
    }
  });
}

function holdStore(): void {
  const { FABRIC_BROKER_DATA_DIR, FABRIC_BROKER_KEY_SOURCE } = BROKER_ENV;
  const h = spawn(BROKER, [], { stdio: ["pipe", "ignore", "inherit"], env: { FABRIC_BROKER_DATA_DIR, FABRIC_BROKER_KEY_SOURCE } });
  holder = h;
  h.on("error", (e) => {
    if (holder === h) holder = null;
    console.error(`harness: the store holder could not be started: ${e.message}`);
  });
  h.on("spawn", () => console.log(`harness: the store is held by a second broker, pid ${h.pid}`));
  h.on("exit", (code, signal) => {
    if (holder === h) holder = null;
    console.log(`harness: the store holder ended, ${signal !== null ? `killed by signal ${signal}` : `exit code ${code}`}`);
  });
}

function releaseStore(): void {
  const h = holder;
  holder = null;
  h?.stdin?.end(); // end of input is the broker's order to stop
  setTimeout(() => {
    if (h !== null && h.exitCode === null && h.signalCode === null) h.kill("SIGKILL");
  }, 2000).unref();
}

async function body(req: IncomingMessage): Promise<any> {
  const chunks: Buffer[] = [];
  for await (const chunk of req) chunks.push(chunk as Buffer);
  return chunks.length === 0 ? null : JSON.parse(Buffer.concat(chunks).toString("utf8"));
}

function reply(res: ServerResponse, status: number, value?: unknown): void {
  res.writeHead(status, { "content-type": "application/json" });
  res.end(value === undefined ? "" : JSON.stringify(value));
}

function serveFile(res: ServerResponse, path: string): void {
  const file = resolve(DIST, `.${path === "/" ? "/index.html" : path}`);
  if (!file.startsWith(DIST + sep)) return reply(res, 404);
  let content: Buffer;
  try {
    content = readFileSync(file);
  } catch {
    return reply(res, 404);
  }
  const type = TYPES[extname(file)] ?? "application/octet-stream";
  // The page gets one extra classic script, which runs before the module scripts.
  if (file === join(DIST, "index.html")) {
    content = Buffer.from(content.toString("utf8").replace("<script type=\"module\"", "<script src=\"/harness/bridge.js\"></script>\n<script type=\"module\""));
  }
  res.writeHead(200, { "content-type": type, "content-security-policy": "default-src 'self'", "cache-control": "no-store" });
  res.end(content);
}

const server = createServer(async (req, res) => {
  const path = new URL(req.url ?? "/", "http://127.0.0.1").pathname;
  try {
    if (req.method === "GET" && path === "/harness/bridge.js") {
      res.writeHead(200, { "content-type": "text/javascript", "cache-control": "no-store" });
      return res.end(readFileSync(join(HERE, "bridge.js")));
    }
    if (req.method === "GET" && path === "/harness/events") { // owner_attach
      page?.end();
      page = res;
      res.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-store" });
      req.on("close", () => {
        if (page === res) page = null;
      });
      return post({ kind: "state", run, state });
    }
    if (req.method === "POST" && path === "/harness/owner_send") {
      const { frame } = await body(req);
      if (child === null || state.name !== "connected") return reply(res, 409, { kind: "not_connected" });
      child.stdin!.write(`${frame}\n`, (e) => e && console.error(`harness: write failed: ${e.message}`));
      return reply(res, 204);
    }
    if (req.method === "POST" && path === "/harness/broker_restart") {
      if (state.name !== "disconnected") return reply(res, 409, "The broker can be restarted only while it is disconnected.");
      startBroker();
      return reply(res, 204);
    }
    if (req.method === "POST" && path === "/harness/kill-broker") {
      if (child === null) return reply(res, 409, "No broker is running.");
      child.kill("SIGKILL");
      return reply(res, 204);
    }
    if (req.method === "POST" && path === "/harness/hold-store") {
      if (child !== null) return reply(res, 409, "Stop the broker first (kill-broker); the store is held only while no broker of the harness runs.");
      if (holder !== null) return reply(res, 409, "The store is held already.");
      holdStore();
      return reply(res, 204);
    }
    if (req.method === "POST" && path === "/harness/release-store") {
      if (holder === null) return reply(res, 409, "The store is not held.");
      releaseStore();
      return reply(res, 204);
    }
    if (req.method === "GET") return serveFile(res, path);
    reply(res, 405);
  } catch (e) {
    reply(res, 400, { kind: "io", detail: String(e) });
  }
});

server.listen(PORT, "127.0.0.1", () => {
  console.log(`harness: serving dist/ at http://127.0.0.1:${PORT}/ with broker ${BROKER}`);
  console.log(`harness: the broker's store is ${BROKER_ENV.FABRIC_BROKER_DATA_DIR} (test key, synthetic agent)`);
  startBroker();
});

function quit(): void {
  child?.stdin?.end(); // end of input is the broker's order to stop
  const h = holder;
  h?.stdin?.end();
  page?.end();
  server.close();
  setTimeout(() => {
    child?.kill("SIGKILL");
    h?.kill("SIGKILL");
    process.exit(0);
  }, 2000).unref();
}
process.on("exit", () => {
  holder?.kill("SIGKILL");
  rmSync(SCRATCH, { recursive: true, force: true });
});
process.on("SIGINT", quit);
process.on("SIGTERM", quit);
