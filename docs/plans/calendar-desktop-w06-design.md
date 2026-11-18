# CD-W06 design: the agent channel (MCP bridge and enrollment)

**What this is.** The design of work package `CD-W06` of the Calendar Desktop plan: the MCP bridge that lets
Claude Desktop, Claude Code and Codex desktop reach the running broker, and the enrollment of an agent. Phase 1
builds the bridge as a new crate with an MCP test client and a stand-in broker socket. Phase 2 is the integration
with the real broker, the store, the contract and the screens; section 7 lists what each owner is asked to build.

**How code comments cite it.** A comment in `apps/fabric-mcp-bridge` such as `(W06 3.4)` or `(A12)` names a
section or a decision of this document. Test ids such as `m03`, `b05` or `p11` name the tests of section 10.
`LR-n` are the requests to the lead session, `V3-n` the needs for schema version 3, `SR-n` the needs of the Agents
screen, `Tn` the threats of section 8, `Rn` the risks of section 11.1 and `Qn` the questions of section 11.2.

**Who decides.** The plan and the contract (`contracts/desktop/v1`) win wherever they differ from this document.
The lead session owns the store, the core and the broker binary; the contract session owns the contract and the
rule theory; the screens session owns the page. Sections 7.1 to 7.4 are requests to them, not decisions. Every
default of section 11.2 is the owner's to change.

Date 2026-11-17. Status: Proposed. Three independent readers critiqued the first draft before anything was built
(section 12). Gate `CD-G2c` is not closed by this document or by phase 1.

Abbreviations (paths relative to the repository root):

| Short | Means |
|---|---|
| PLAN | `docs/plans/calendar-desktop-plan.md` |
| W04 | `docs/plans/calendar-desktop-w04-design.md` (store, core, owner channel) |
| ADR108, ADR-115 | `docs/adr/ADR-108-agent-and-mcp-permission-model.md`, `docs/adr/ADR-115-broker-ledger-envelope.md` |
| CR | `contracts/desktop/v1/README.md` (the contract) |
| CORE | `crates/fabric-broker-core/src/` |
| MCP | the Model Context Protocol specification, https://modelcontextprotocol.io/specification |
| modern, legacy | MCP's own words: revision `2026-07-28` and later (no handshake, every request carries its version), and `2025-11-25` and earlier (an `initialize` handshake) |
| the bridge | the executable `fabric-mcp-bridge` |

---------------------------------------------------------------------------------------------------------------------

## 0. Decisions

### 0.1 Design decisions

| # | Topic | Chosen | Why |
|---|---|---|---|
| A1 | Crate | `apps/fabric-mcp-bridge`, package and binary `fabric-mcp-bridge`, a library and a `main.rs` like `apps/fabric-broker` | PLAN 4.3 "a small Rust MCP bridge"; `apps/` holds the executables. |
| A2 | MCP SDK or own code | Own small implementation over `serde_json`, blocking I/O, no async runtime. No MCP SDK | The bridge serves seven methods. The official Rust SDK (`rmcp` 3.x) supports the current revision but brings an async runtime and a large dependency tree into a binary that holds a signing key. Section 3.1. |
| A3 | MCP revisions | Two, pinned: `2026-07-28` (modern, the current revision) and `2025-11-25` (legacy, through `initialize`). The bridge is what MCP calls a dual-era server | All three installed clients know both (appendix A). Which one each client uses is seen only in a live run; the gate's report then pins the bridge's two revisions and each client's negotiated one (PLAN 9.2). Section 3.2. |
| A4 | Era rule | Decided per request: a request whose `params._meta` holds `io.modelcontextprotocol/protocolVersion` is modern; any other is legacy and needs an answered `initialize` first. No other MCP state | MCP 2026-07-28 "Backward Compatibility": a dual-era server selects its behaviour from how the client opens, and may serve both. |
| A5 | The MCP side never needs the broker | `server/discover`, `initialize` and `tools/list` are answered from the binary alone. Only `tools/call` talks to the broker. The bridge never exits because the broker is absent | Claude Code may stop asking a stdio server about the modern revision for 7 days "after the server exits when asked" (appendix A). PLAN 12: "Clients receive a clear Fabric-not-running response". |
| A6 | Tools | The ten tools of `data/tool-catalog.json`, in its order, with its names and descriptions. `inputSchema` is made from the contract's argument schema by four fixed rules (3.5): references inlined, a member that may be null written as an optional member, a union told apart by `kind` written as one object, examples added to the time formats. No `outputSchema` | One to one with the contract (PLAN 9.2). Claude Desktop is reported to strip a null-or-value union from a tool schema and to pass a null as the text "null" (appendix A); a schema with no union and no null has nothing to strip. |
| A7 | Arguments | The bridge writes `null` for every absent member that the contract lets be null, builds the contract's agent request frame and reads it with `fabric_desktop_contracts::decode`. A refusal is a tool error for the model and nothing reaches the broker. Filling in the nulls is the bridge's only change to an argument | CR 5: an adapter applies the frame, protocol and schema layers "and nothing more". The emitted JSON Schema is guidance; the contract types decide. |
| A8 | Tool results | Two text blocks: fixed guidance for the model, then the contract's value as canonical JSON (`tool_outcome`, or `{"error": ...}`). `isError` is true for every error and for every state in which the request did not and will not do what was asked. No `structuredContent` | A model reads text. One relay of the contract's value, no second copy that a client might feed to the model twice. Section 3.7. |
| A9 | Contract errors | Every contract error becomes a tool result with `isError: true`. JSON-RPC errors are only for MCP's own structure: unknown tool or malformed `tools/call` (`-32602`), unknown method (`-32601`), bad JSON (`-32700`) | MCP "Tools, Error Handling": input and business errors are tool errors so the model can correct itself. |
| A10 | Waiting is the broker's | The bridge sends a call and relays its one answer. It does not poll. The broker is asked to answer an action when its request is in a state the agent can act on, or after about 20 s (LR-C4) | A status may never carry a result (CR 8), and `fabric_request_result` is itself an action, so only the answer to an action can deliver a result. The broker knows when a request settles; a poll from the bridge could never bring the result. Section 3.8. |
| A11 | No hidden actions | The bridge sends the broker only the call the client made. It never calls `fabric_request_result` or `fabric_request_status` by itself, never repeats a proposal, never retries after an error | A result release is an action with its own proof (CR 8). Authority stays with the broker. |
| A12 | No cache | The bridge keeps no result, no status and no calendar content beyond the call in flight | PLAN 9.2 and 14: a completed read polled after revocation discloses nothing. The bridge's half of that rule is to hold nothing. |
| A13 | Socket | `<broker data directory>/run/agent.sock`; the directory `run` has mode 0700 and the socket mode 0600. Without `--socket` the bridge takes the installed application's data directory under `HOME` | PLAN 4.3. One path that the broker and the bridge both derive; the broker knows only its data directory (W04 D28). Section 4.1. |
| A14 | Checks before connecting | The bridge makes `run` its working directory, then requires that it is a directory of mode exactly 0700 owned by the bridge's own user, and that `agent.sock` in it is a socket of mode exactly 0600 owned by the same user, and connects by the relative name | Only that user can then have put the socket there, whatever the path above it looks like; the working directory holds the checked directory fast between the check and the connection. Section 4.2. |
| A15 | Connection | Opened at the first tool call, not at start. Kept until the process ends or the broker closes it. The next tool call opens a new one | A session then starts when the agent first acts and not when a client merely loads its servers. One connection is one session (W04 5.5). |
| A16 | Handshake | Two round trips at the start of every connection: `agent_hello` (the agent's id, its public key, the schema version) answered with a 256-bit broker nonce; `agent_prove` with an Ed25519 signature over a domain-separated canonical text of all four. Every decision is made after the signature verifies. No signature on later frames | The owner's decision of 2026-11-17: the key is checked once per connection, no request is signed in version 1 (ADR-115 open question). Section 4.4. |
| A17 | Key | Ed25519 (`ed25519-dalek`, already in the tree). Made by the bridge at the first tool call for an agent id, or by its `code` command. Release builds keep the 32-byte seed in the macOS Keychain. Debug builds with `--key-dir` keep it in a file | PLAN 4.3: the bridge generates the key in its own Keychain namespace. PLAN 12: a distinct Keychain service. Tests and sessions never touch the Keychain. Section 5.4. |
| A18 | Enrollment starts in the application | The owner adds an agent on the Agents screen and gets its id and the configuration text. The bridge makes the key later and proves it in its first handshake; the broker then holds that key for the pending agent | PLAN 4.3: configuration holds "only the public enrollment ID and executable location", so the id must exist before the bridge first runs. The enrollment id is the agent's id; there is one id. Section 5. |
| A19 | Pairing code | Twelve hexadecimal digits from the SHA-256 of the public key, shown as `AB12-CD34-EF56` by the bridge (its `code` command, and the agent's answer while it waits) and on the Agents screen. The owner's approval names the code | The owner approves one exact key and can see that it is the one the bridge holds. Section 5.3. |
| A20 | States | `pending`, `approved`, `removed`. `removed` is final; an id and a key are never used again | The task's list. Section 5.1. |
| A21 | List permission | Approving an enrollment gives, in the same commit, a standing permission for `fabric_calendar_list`. Preferred form: a grant until revoked | The owner's decision of 2026-11-17. Section 5.5 gives the two forms and what each costs. |
| A22 | Pause | Nothing in the bridge. A paused agent's call is decided by the broker (the rule `agent_not_paused` blocks it) and the bridge relays the denial | ADR108 D3. The bridge makes no policy decision. |
| A23 | Developer seams | `cfg(debug_assertions)` only, as W04 D11: `--socket <path>` and `--key-dir <dir>`. A release build refuses both with exit 64 | Tests and the practice run use scratch directories and test keys; a release build can reach only the installed broker and the Keychain. |
| A24 | Configuration text | Made by one function from the bridge's real path and the agent's id: JSON for Claude Desktop and Claude Code, TOML for Codex, each string escaped for its format. The bridge prints it with `config`; nothing writes into a client's file | PLAN S07 and 12. The owner pastes it himself. Section 6. |
| A25 | Logging | stderr only: fixed words, ids, tool names, state names and error codes. Never an argument, a result, a task text, a broker message or a key. A panic prints one fixed line | Claude Desktop keeps a server's stderr in a log file. PLAN 12: no secret and no content in logs. |
| A26 | No process is started | The bridge has no code that starts or restarts any process | PLAN 4.3: "MCP bridges cannot restart it into an implicit owner session". |
| A27 | The guidance is fixed | The first text block of a tool result is the bridge's own wording plus values that pass the contract's grammar: an id, a time, a rule's name, a number. Free text from the broker (an explanation, an error message) appears only inside the second block, as data | Whatever answers on the socket cannot put words of its own into the part that reads as instruction (T4, T7). |

### 0.2 Settled before this design

| # | Settled | Source |
|---|---|---|
| S1 | A stdio bridge joins the running broker over a Unix socket, mode 0600 in a directory of mode 0700 | PLAN 4.3 |
| S2 | No secret in configuration or arguments; the bridge sits in the application bundle beside the broker | PLAN 4.3, 12, S07 |
| S3 | The three external clients are Claude Desktop, Claude Code and Codex desktop | PLAN C04, 9.3 |
| S4 | Approving an enrollment also gives the permission to list calendars | The owner, 2026-11-17 |
| S5 | A real agent may use the practice calendar before Google exists, in debug builds only | The owner, 2026-11-17 |
| S6 | No signature per request in version 1; the key is checked once per connection | The owner, 2026-11-17 |
| S7 | The contract session drafts schema version 3 with the missing owner commands; the owner's approval of it is the merge | The owner, 2026-11-17 |
| S8 | "Polling a completed read after revocation discloses nothing" is proven in CD-W05 and CD-W06 | W04 Q10, the owner's default of 2026-11-17 |

---------------------------------------------------------------------------------------------------------------------

## 1. Scope

### 1.1 Phase 1 delivers

1. This design.
2. `apps/fabric-mcp-bridge`: the MCP side (both revisions), the ten tools, the mapping of contract answers to
   tool results, the socket client with its checks and the handshake, the key in a file (debug builds), the
   pairing code, the configuration text.
3. Tests: an MCP test client and a stand-in broker socket under `apps/fabric-mcp-bridge/tests/support`. Test keys
   live in scratch directories. No test touches the Keychain or starts a real client.

No file of the core, the store, the broker binary, the contract, the rule theory or the page is edited.

### 1.2 Phase 2: the integration

| Owner | Builds | Section |
|---|---|---|
| Lead | Store: the agent's public key and enrollment state. Core: enrollment, removal, the handshake's decisions, the list permission, held answers and results. Binary: the socket listener, a practice mode for a real agent | 7.1, 7.2 |
| Contract session | Schema version 3 in the contract and its crate: the handshake, the enrollment commands, the visible pending agent, the list permission | 7.3 |
| Screens session | The Agents screen S07 | 7.4 |
| This session | The Keychain key store, the bridge's switch to the contract crate's handshake types, the wording changes of section 13, the tests against the real broker, a review of the bundle (the lead builds it) | 5.4, 7.5, 13 |

Order: the contract's handshake (V3-1) is merged before the lead's listener and core work, which read its types.

### 1.3 Not in CD-W06

The embedded Ollama runtime (CD-W07). Google (CD-W05). A desktop extension package (`.mcpb`) for Claude Desktop.
Signing and notarization of the bridge (CD-W11). A message that cancels a request (CR 15).

### 1.4 What the owner sees after phase 1

Nothing in the application. Phase 1 is a binary that speaks MCP and answers every tool call with "Fabric Calendar
is not running", because no broker listens yet. Its tests show the rest against a stand-in.

---------------------------------------------------------------------------------------------------------------------

## 2. The bridge executable

### 2.1 Modules

```text
apps/fabric-mcp-bridge/
  src/main.rs          arguments, the threads, the loop
  src/lib.rs
  src/cli.rs           the command line (2.2)
  src/mcp/jsonrpc.rs   one line to one message; responses and errors
  src/mcp/server.rs    the era rule and the methods (3.3, 3.4)
  src/mcp/tools.rs     the ten tools from the contract's catalog and schemas (3.5), the null filling (3.6)
  src/mcp/render.rs    a contract answer to a tool result (3.7)
  src/bridge.rs        the state machine: calls in flight, their limits, cancellation (2.3)
  src/broker/frames.rs agent frames in and out, the 1 MiB bound
  src/broker/handshake.rs  the provisional handshake of 4.4 (replaced by the contract crate's types with V3-1)
  src/broker/socket.rs the path, the checks, the connection (Unix only)
  src/broker/key.rs    the key, its file store (debug builds) and the pairing code
  src/config_text.rs   the configuration text of section 6
  tests/support/       the MCP test client and the stand-in broker
```

Dependencies: `fabric-desktop-contracts`, `serde_json`, `ed25519-dalek` (with its default `zeroize`),
`getrandom`, `sha2`, `zeroize`, and on Unix `rustix` for the process's own user id. All are in `Cargo.lock`
already; the lock file still changes, because it lists the new member. `security-framework` joins on macOS with
the Keychain store in phase 2.

### 2.2 Command line

```text
fabric-mcp-bridge --enrollment <id> [--socket <path> --key-dir <dir>]      serve MCP on stdin and stdout
fabric-mcp-bridge code --enrollment <id> [--key-dir <dir>]                 print the pairing code (5.3)
fabric-mcp-bridge config --client <claude-desktop|claude-code|codex> --enrollment <id>
                  [--socket <path> --key-dir <dir>]                        print the configuration text (6)
fabric-mcp-bridge --version
```

- `<id>` is a contract `Id` (`^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$`), so it is safe in a file name and in every
  configuration format. Reading it as an `Id` is the first thing the bridge does with it: `..`, `a/b` and
  `.hidden` are a wrong command line, before any file is touched, and the key store takes only the typed `Id`.
- `--socket` and `--key-dir` exist only in debug builds (A23), take absolute paths, and when serving come together
  or not at all. A debug build without them behaves as a release build does.
- A wrong command line prints one line to stderr and exits 64, before anything is read from stdin. Arguments
  must be UTF-8, because they end up in JSON and TOML.
- No argument and no environment variable carries a secret. The bridge reads one variable, `HOME`, to find the
  installed socket (4.1).

### 2.3 Process model

Threads and one loop, the shape of the broker's own `main.rs` (W04 10.6):

- a reader of stdin, which sends each line (or "too long", or end of input) to the loop;
- a reader of the socket, started per connection, which sends each frame (or "closed");
- the loop, which owns all state and does all writing.

The state is one value, `Bridge`, with one entry point: `handle(event, now_ms) -> effects`. Events are an MCP
line, a broker frame, "the broker closed", the outcome of a connection attempt, and a tick. Effects are a line for
stdout, a frame for the socket, "connect" and "close". The loop waits on its channel until the earliest deadline
of the calls in flight. `Bridge` does no I/O, so its tests run without a process or a socket.

Connecting and the handshake run on the loop with a 5 s limit in total. A local socket answers at once; the limit
bounds the worst case.

End of input on stdin ends the process with exit 0 (MCP stdio "Shutdown"). The socket closes with it, and the
broker ends the session (W04 5.5).

### 2.4 Output

- stdout: one JSON-RPC message per line and nothing else, written through `serde_json`, so no line holds a line
  feed.
- stderr: A25. Example: `fabric-mcp-bridge: call call-9f.. tool fabric_calendar_propose_create state pending_approval`.
- Exit codes: 0 at end of input (a read error on stdin or a closed stdout counts as the end); 64 for a wrong
  command line; 1 when `code` or `config` cannot answer; 70 after a panic, whose hook prints one fixed line and
  nothing of the panic's message. Nothing else ends the serving process (A5).

### 2.5 Limits

| Limit | Value | Past it |
|---|---|---|
| One line from the client | 2 MiB | JSON-RPC error `-32600` without an id; the rest of the line is skipped |
| One frame to or from the broker | 1,048,576 bytes (CR 1) | To the broker: the call is a tool error and nothing is sent. From the broker: the connection is dropped |
| Calls in flight | 16 | A tool error, "too many calls at once" |
| Open `subscriptions/listen` | 8 | JSON-RPC error `-32603` |
| Connect and handshake | 5 s | A tool error, `broker_not_running` |
| One write to the broker | 5 s | The connection is dropped: a frame may be half written |
| The answer to one call | 30 s | That call alone is a tool error (3.7); the connection stays |

One tool call therefore ends within 35 s. Codex ends a tool call after 60 s by default and Claude Code after far
longer (appendix A).

Dropping a connection ends the agent's session and with it every grant "for this session" (W04 5.5). The bridge
drops one only when the frames on it can no longer be trusted: a frame over the bound, or one that is not a
contract frame, or a write that did not finish. A slow answer does not drop it.

---------------------------------------------------------------------------------------------------------------------

## 3. The MCP side

### 3.1 An SDK or own code

| | Own code (chosen) | `rmcp` 3.x, the official Rust SDK |
|---|---|---|
| Supports `2026-07-28` and `initialize` | To be written and tested here, seven methods | Yes, since 3.0 (release notes; the exact server API was not read) |
| Dependencies | None new to the lock file | An async runtime and its tree; `tokio` is in `Cargo.lock` only through the demo (`apps/fabric-demo`), not through any crate of the desktop product |
| Fit to the contract | Strict contract JSON on the socket, plain JSON on stdio, one loop | Its own types and handler traits; a fresh handler per request since 3.0 |
| Conformance risk | Ours. Met by a test client written from the specification text by another hand than the server's (10.2), and by the live runs | The SDK's |
| Audit surface of a key-holding binary | Small | Large |

The handshake's text, signature and pairing code are checked against vectors that the contract session computed
on its own (k04). The MCP test client is also own code. A second opinion from the SDK is cheap to add later as a test dependency
if a live run shows a misreading; it is not added now.

### 3.2 Revisions

- `2026-07-28` is the current revision (MCP "Versioning", read 2026-11-17). It has no `initialize`. Every request
  carries `io.modelcontextprotocol/protocolVersion` and `io.modelcontextprotocol/clientCapabilities` in
  `params._meta`. A server must implement `server/discover`. Every result carries `resultType`. `ping` and
  `logging/setLevel` are gone. A server never sends a request.
- `2025-11-25` is the last legacy revision: `initialize`, then `notifications/initialized`, then requests.
- On stdio a client that knows both asks `server/discover` first and falls back to `initialize` on any error it
  does not recognize, or on silence (MCP stdio "Backward Compatibility").

The bridge answers both. In `server/discover` and in the error `-32022` it lists `["2026-07-28"]` only. MCP's own
example of the error lists a legacy version beside the modern one; the bridge lists only versions that a request's
`_meta` may carry, so that the list and rule 3 of 3.4 agree. An `initialize` that names any version is answered
with `2025-11-25`, as the legacy rule says a server answers with a version it supports. Older legacy revisions are
not accepted: each would be a claim to test, and no installed client needs one (R4).

### 3.3 Methods

| Method | Modern | Legacy |
|---|---|---|
| `server/discover` | `resultType: "complete"`, `supportedVersions`, `capabilities: {"tools": {}}`, `instructions`, `ttlMs: 300000`, `cacheScope: "public"`, `_meta` with `serverInfo` | Has no modern `_meta`, so rule 4 of 3.4 answers `-32602`; a client that knows both then falls back |
| `initialize` | - | `protocolVersion: "2025-11-25"`, `capabilities: {"tools": {}}`, `serverInfo`, `instructions` |
| `notifications/initialized` | ignored | ignored |
| `ping` | `-32601` | `{}`, also before `initialize` |
| `tools/list` | the ten tools, `resultType`, `ttlMs: 300000`, `cacheScope: "public"`, `_meta` with `serverInfo` | `{"tools": [...]}` and nothing more |
| `tools/call` | 3.6; the result is `resultType: "complete"`, `content`, `isError`, `_meta` with `serverInfo` | 3.6; the result is `content` and `isError` and nothing more |
| `subscriptions/listen` | below | `-32601` |
| `notifications/cancelled` | 3.9 | 3.9 |
| anything else | `-32601`; a notification is ignored | the same |

- `tools/list` with a `cursor` gets `-32602`: the list is one page and the bridge issues no cursor.
- `subscriptions/listen` must be served even by a server with nothing to announce (MCP "Overview": every
  implementation supports the message patterns). The bridge answers with the notification
  `{"jsonrpc": "2.0", "method": "notifications/subscriptions/acknowledged", "params": {"_meta":
  {"io.modelcontextprotocol/subscriptionId": <the request's id>}, "notifications": {}}}` and then nothing: no
  response while the subscription is open, and none after the client cancels it. A request without
  `params.notifications` gets `-32602`.
- `serverInfo` is `{"name": "fabric-mcp-bridge", "title": "Fabric Calendar", "version": <crate version>}`. The
  tool list never changes while the process lives, so `tools` has no `listChanged`.
- `instructions` is one fixed paragraph of at most 512 characters (Codex reads only so many reliably), the two
  rules first: an answer of `pending_approval` means the owner must approve in Fabric Calendar, and a proposal is
  never repeated; then that `fabric_request_status` with the `request_id` shows how a request stands.

### 3.4 The era rule, exactly

1. A line that is not one JSON object gets `-32700` (not JSON) or `-32600` (an array, a wrong `jsonrpc`, an id that
   is not a string or an integer). The error has no `id` member when the id could not be read: MCP's schema has no
   null id. A response from the client is ignored.
2. `initialize`: answered as legacy; the process remembers that it was answered.
3. `params._meta` holds `io.modelcontextprotocol/protocolVersion`: the request is modern. A version other than
   `2026-07-28` gets `-32022` with `data: {"supported": ["2026-07-28"], "requested": ...}`. A missing or
   non-object `io.modelcontextprotocol/clientCapabilities` gets `-32602`.
4. Otherwise the request is legacy. `ping` is answered. Any other request before an answered `initialize` gets
   `-32602`, "the request carries no protocol version", which is what MCP asks of a modern server.
5. `clientInfo` is never used for identity. The agent is the enrolled key (PLAN 9.2).

### 3.5 `tools/list`

Each tool is `name`, `title`, `description`, `inputSchema` and `annotations`.

- `name` and `description`: `data/tool-catalog.json`, word for word, in its order. The file and the two schema
  files are compiled in with `include_str!`.
- `title`: a short name for a person, such as "Propose a new event". It is the bridge's own text.
- `inputSchema`: the catalog's `arguments` definition, changed by four rules and no other:
  1. Every `$ref` is replaced by its definition. A reference resolves against the file that contains it
     (`common.schema.json` refers to itself). The ten definitions reach nothing recursive.
  2. A member whose schema is "null or X" (`oneOf` of `null` and X) is written with the schema X and taken out of
     `required`. Its description gains "Leave this member out for none." The bridge writes the `null` back (3.6).
  3. A union of objects told apart by a `kind` constant (`response_target`) is written as one object: `kind` with
     an `enum` of the constants, every other member of every branch optional, `required: ["kind"]`. The contract's
     reader still enforces which members go with which kind.
  4. The descriptions of the three time formats gain an example: `2026-11-20T16:00:00.000Z` for a UTC instant,
     `2026-11-20T17:00:00+01:00` for a local time with its offset, `Europe/Berlin` for a zone. Codex drops
     `pattern` before its model sees a schema (appendix A), and the contract's descriptions point at the pattern.
  After the rules a schema holds no `$ref`, `oneOf`, `anyOf`, `const` or `null`. Patterns, bounds and the other
  descriptions stay. The ten schemas are about 16 KB together.
- `annotations`. `readOnlyHint: true` for `fabric_calendar_list`, `fabric_calendar_availability`,
  `fabric_calendar_read` and `fabric_request_status`: they change nothing in a calendar, though the first three
  make a request in Fabric Calendar and may raise an approval card. `fabric_request_result` is `readOnlyHint:
  false`, `destructiveHint: false`: it uses up a permission given for one request. The create proposal is
  `readOnlyHint: false`, `destructiveHint: false`; the update, invitation, response and delete proposals are
  `destructiveHint: true`. A client may skip its own confirmation for a read-only tool; the hints carry no
  authority in Fabric Calendar.

A test compares the names with `ToolName::ALL` and the catalog, so a tool added to the contract fails the
bridge's tests until the bridge lists it.

### 3.6 `tools/call`

1. `params.name` must be one of the ten names; otherwise `-32602`, "Unknown tool", and nothing is sent. This is
   also the answer to any attempt to call an owner method through MCP: the bridge knows no such name.
2. `params.arguments` (an absent one counts as `{}`) must be an object; otherwise `-32602`.
3. The bridge walks the arguments beside the contract's schema and writes `null` for every absent member that the
   contract lets be null (rule 2 of 3.5 backwards). A `null` the client sent stays.
4. The bridge builds `{"protocol": "fabric-desktop/1", "id": <call id>, "method": <name>, "params": <arguments>}`
   and reads it with `decode(Entry::AgentRequest, ...)`. Any refusal is the tool error `invalid_params` with the
   pointer `/params` and the contract's reason, cut to the contract's 1000 characters; the reason goes into the
   tool result only, never to stderr, because it can repeat an argument (A7, A25). The call id is `call-` and 32
   hexadecimal digits from 128 random bits, new for every call. `serde_json` has by then resolved a member written
   twice to its last value; the strict reader sees the result.
5. With no connection, the bridge connects (4.2 to 4.4). A failure is a tool error (3.7).
6. The frame is sent, and the answer with the same id is rendered (3.7).

Several calls may be in flight; answers are matched by frame id in any order.

### 3.7 What the agent sees

A tool result has two text blocks. The first is fixed wording (A27) with the request id and, where the status has
them, the card's expiry, the rule's name or the failure code. The second is the contract's value as canonical
JSON: the `tool_outcome`, or `{"error": <the contract's agent error>}`. A rule's explanation and an error's
message are in the second block.

| The request's state | `isError` | The first block says |
|---|---|---|
| a result is present (any state) | false | The result follows. |
| `pending_approval` | false | The owner must approve in Fabric Calendar. Do not repeat the call. Ask the owner, then call `fabric_request_status` with this `request_id`. The card expires at the given time. |
| `applied`, `reconciled` | false | The change was applied. `fabric_request_result` with this `request_id` gives the event's id and revision. |
| `received`, `validating`, `proving_classification`, `allowed`, `approved`, `proving_fresh_permit`, `queued`, `dispatching`, `retry_wait`, `reconciling` | false | Fabric Calendar is still working. Call `fabric_request_status` with this `request_id` in a few seconds. Do not repeat the call. |
| `outcome_unknown` | false | Fabric Calendar does not know yet whether the change reached the calendar. Do not repeat it. Call `fabric_request_status` later; the owner can also look in Fabric Calendar. |
| `blocked` | true | A rule blocked the request: the rule's name. Nothing was changed. |
| `denied` | true | The owner denied the request. Nothing was changed. |
| `expired`, `cancelled`, `stale` | true | The request ended without a change; for `stale`: read again and propose again if it is still wanted. |
| `undecided` | true | Fabric Calendar could not check the request against its rules, so nothing was done. The owner can see why in Fabric Calendar. A new call may be made later. |
| `terminal_failure` | true | The request failed, with the failure code. |
| `needs_owner_resolution` | true | Fabric Calendar does not know whether the change reached the calendar. Do not repeat it. The owner must look in Fabric Calendar. |
| `absence_confirmed` | true | The change did not reach the calendar. |

`undecided` carries no reason: why a request is undecided is for the owner only (CR 8, choice 38). The bridge
adds none.

The draft of schema version 3 (pull request 36, the contract session's notice of 2026-11-17) adds three states
and reopens one. The contract crate on `main` does not have them, so the bridge cannot name them yet. When the
draft is on `main` the table gains these lines:

| The request's state | `isError` | The first block says |
|---|---|---|
| `completed`, terminal: a request the broker serves itself, such as a calendar list or the release of a result | false | The request is complete. With a result, the first row of the table applies. |
| `resolved_as_applied`, terminal | false | The owner states that the change is in the calendar. |
| `resolved_as_absent`, terminal | true | The owner states that the change is not in the calendar. |
| `needs_owner_resolution`, no longer terminal | false | As today, and: call `fabric_request_status` later. |

The bridge's table is one match over every state of the contract crate, with no default. A contract with a state
the table lacks does not compile with the bridge, so no state can pass unnoticed (R13).

| Error | From | The first block says |
|---|---|---|
| `broker_not_running` | the bridge: no socket, a refused connection, a failed check of 4.2, a handshake out of time, a connection that closed with the call in flight, no key store in this build | Fabric Calendar is not running, or its connection ended. Ask the owner to open Fabric Calendar, then call again. If a proposal was in flight it may have been recorded: do not repeat it before the owner has looked. |
| `prover_unavailable` | the broker | Fabric Calendar's rule checker is not ready. Nothing was recorded. Try again in a little while. |
| `busy` | the broker | Too many requests: the limit, its maximum and the wait in seconds. Nothing was recorded. |
| `not_permitted`, the agent is pending | the handshake | This agent waits for the owner's approval in Fabric Calendar, Agents. Its code is `AB12-CD34-EF56`. After the approval, call again. |
| `not_permitted` otherwise | the handshake, or the broker | Fabric Calendar does not accept this agent. Its code is `AB12-CD34-EF56`. The owner can look in Fabric Calendar, Agents. |
| `invalid_params` | the bridge's own check (A7), or the broker | The arguments are not valid; the reason is in the error below. Correct them and call again. |
| `not_found` | the broker | No request of this agent has that id. |
| `unsupported_schema_version` | the handshake | Fabric Calendar and its bridge are different versions. The owner must install Fabric Calendar again. |
| `internal` | the broker (also the handshake while the store cannot be opened), or the bridge after 30 s without an answer | Fabric Calendar had a fault or did not answer. If a proposal was in flight it may have been recorded: do not repeat it. The owner must look in Fabric Calendar. |
| `internal`, made by the bridge for its own faults | too many calls at once (2.5); a key that cannot be read or stored (5.4); a handshake answer the bridge cannot read (4.4) | One fixed sentence for each, saying which. Nothing was sent to Fabric Calendar. |
| `malformed_frame`, `unsupported_protocol`, `unknown_method` | the broker | The same wording as `internal`: the bridge and the broker disagree, which the agent cannot mend. |

An error the bridge makes itself is written as the contract's agent error with that code, so the second block has
one shape.

`prover_unavailable` is relayed as it comes. The bridge does not retry it: a retry loop would hide the prover's
state from the agent and the owner. A request the broker accepted all the same ends `undecided` (CR 8) and is
shown as such.

### 3.8 Waiting

The bridge does not wait for a request and does not poll (A10). When the answer comes, the bridge shows what it
says.

For an agent to get the plan's first answer ("an initial Ask returns `pending_approval`", "Allow returns the
proven filtered read result", PLAN 9.2), the broker must hold its answer to an action until the request is
`pending_approval`, is `applied`, is in a terminal state or has a released result, or until about 20 s have
passed (LR-C4). Today the core answers every action at once with `received` and no result (CORE
`service/agent.rs`), and a status may never carry a result, so no result can reach an agent yet. Until LR-C4 is
built an agent sees "still working" and calls `fabric_request_status` itself.

`fabric_request_status` is answered at once, as today. `pending_approval` never holds an answer back: no call
stays open while a person decides (PLAN 9.2).

### 3.9 Cancellation

`notifications/cancelled` with a `requestId` in flight removes the call. No response is sent for it, and a later
broker answer for it is dropped. The broker is told nothing, because the contract has no cancel message (CR 15):
a request that was recorded stays recorded and its card stays open. The same notification for a
`subscriptions/listen` id ends that subscription, also without a response. An unknown id is ignored.

---------------------------------------------------------------------------------------------------------------------

## 4. The agent socket

### 4.1 Path

`<broker data directory>/run/agent.sock`. The installed application's data directory is
`$HOME/Library/Application Support/ai.paracels.fabric.calendar/broker` (the bundle identifier of
`tauri.conf.json`; W04 D28). Without `--socket` the bridge builds the path from `HOME`, in a release and in a
debug build alike. Without `HOME` every tool call answers `broker_not_running`, with the reason on stderr.

A Unix socket's name is at most 103 bytes on macOS. The installed path is 85 bytes plus the user's short name.
The bridge is not bound by this, because it connects by the relative name (4.2). The broker binds the name and
must either do the same or check the length and say so in `health` (LR-B2).

Debug builds: `--socket <path>` names the socket. The practice run and the tests use it (A23).

### 4.2 Checks before connecting

In this order; the first failure is `broker_not_running` and one stderr line naming the check:

1. The bridge makes the socket's directory its working directory.
2. That directory (`.`, read without following a link) is a directory, its permission bits are exactly `0700`,
   and its owner is the bridge's own effective user id.
3. `agent.sock` in it (read without following a link) is a socket, its permission bits are exactly `0600`, and
   its owner is the same user id.
4. The bridge connects to the relative name `agent.sock` within the limit.

What this gives: the directory the bridge stands in belongs to its user and nobody else can write into it, so the
socket in it was put there by that user (or by root), whatever links or loose permissions the path above may
have. Standing in the directory keeps it the same directory between the check and the connection. The bridge
uses no other relative path.

What it does not give: the bridge does not know which program of that user listens (T4). The standard library has
no stable call for the peer's identity, and the workspace forbids `unsafe`. The broker, for the other direction,
is asked to read the connecting peer's user id (LR-B3).

### 4.3 Frames

The contract's frames and nothing else (CR 1, 2): one strict JSON object per line, at most 1,048,576 bytes,
`"protocol": "fabric-desktop/1"` on every frame. After the handshake the bridge sends `agent_request` frames and
reads `agent_response` frames. The bridge never sends an owner method; the agent socket has none.

### 4.4 The handshake (provisional until schema version 3)

```text
bridge -> broker   {"protocol": "fabric-desktop/1", "id": "hello-<32 hex>", "method": "agent_hello",
                    "params": {"schema_version": <SCHEMA_VERSION>, "enrollment_id": "<id>",
                               "public_key": "ed25519:<64 hex>"}}
broker -> bridge   result {"schema_version": <n>, "broker_nonce": "<64 hex>"}
                   or error unsupported_schema_version | internal
bridge -> broker   {"protocol": "fabric-desktop/1", "id": "prove-<32 hex>", "method": "agent_prove",
                    "params": {"signature": "<128 hex>"}}
broker -> bridge   result {"enrollment": "approved"}  or  result {"enrollment": "pending"}
                   or error not_permitted | internal
```

The signature is Ed25519 over the UTF-8 bytes of

```text
"fabric-desktop/1\nagent_handshake\n" + canonical({"broker_nonce": ..., "enrollment_id": ...,
                                                   "public_key": ..., "schema_version": ...})
```

with the contract's canonical form (CR 3). The two answers are response frames as the contract writes them (CR
1): `protocol`, the request's `id` and `method`, and either `result` or `error`, the error being the contract's
agent error. The two leading lines of the signed text are the contract's digest prefix with a new
domain, so the signed bytes can be mistaken for nothing else the product hashes or signs. The only part a broker
chooses is the nonce, a string inside the canonical object, so nothing useful elsewhere can be had from the
bridge's signature.

- The broker answers every well-formed `agent_hello` of its own schema version with a nonce, whatever the id. It
  decides only at `agent_prove`, after the signature verifies: an unknown id, a removed agent, a key that is not
  the approved one, and a bad signature all get `not_permitted`. So a hello tells nobody which ids exist, and a
  key that was not proven is never recorded.
- The nonce is 256 random bits, made by the broker for this connection and used once. A recorded handshake cannot
  be played again.
- `approved` means the broker has started the session (`AgentInput::Connect`). `pending` means the proven key is
  now the pending agent's key (5.1) and the owner has not approved yet; the bridge closes the connection and
  answers the tool call with the pairing code (3.7). The next tool call tries again.
- The bridge reads the answers strictly: the nonce is exactly 64 lowercase hexadecimal digits, the answer's
  `schema_version` equals the bridge's own (another one is `unsupported_schema_version`), the frame ids match,
  and any other frame before the handshake ends closes the connection. There is no negotiation, so there is
  nothing to downgrade.
- `internal` at either step is the broker saying that it cannot open its store (today it closes such a
  connection, CORE `broker.rs`).
- The bridge sends its key and proves it once. Later frames are not signed (S6). ADR-115 action item 4 is
  answered by this: the agent channel signs no request in version 1.

The four frames are not contract frames yet: `decode` refuses their methods. Their types live in
`src/broker/handshake.rs` until the contract crate has them (V3-1). The stand-in broker of the tests reads these
four with its own code, written from this section, and every other frame with the contract's `decode`.

### 4.5 Sessions

- A successful handshake of an approved agent starts a session (W04 5.5). The bridge connects at the first tool
  call (A15), so the session starts when the agent first acts.
- The session ends when the bridge's process ends (the client closes it, or the client quits), when the broker
  quits or restarts, when the owner removes the agent, when another connection of the same agent replaces it
  (W04 5.5, `Replaced`), or after 30 minutes without a call.
- The bridge learns of an end by the closed connection: the core already closes a connection whose session ended
  by idle time, replacement or quit (CORE `service/core.rs`, `service/recovery.rs`). The next tool call connects
  again and gets a new session.
- A session is the life of one connection of one client process. It is not a conversation: MCP says a stdio
  process is not one ("Statelessness"), and Claude Desktop serves all its chats from one server process. A grant
  "for this session" therefore covers every chat of that client until the session ends. This design recommended
  the words "until this client disconnects" for the Agents screen and the approval card (SR-3). The lead ruled
  otherwise: the screen keeps "until this agent's current session ends" and adds a fixed note that lists the ends
  above (13).
- One agent has one live session (the store's `sessions_one_live`). Two bridge processes with the same id, such
  as two Claude Code windows in the same folder, replace each other's session at every call, and a grant "for
  this session" ends each time (R6).

### 4.6 Pause

The owner pauses an agent on the owner channel. The bridge holds no pause state. A call of a paused agent reaches
the broker, the rule theory blocks it (`agent_not_paused`), and the bridge shows `blocked` with that rule.
Resuming needs nothing from the bridge.

---------------------------------------------------------------------------------------------------------------------

## 5. Enrollment

### 5.1 States

```text
(none)   -> pending    the owner adds an agent: a name and the kind of client
pending  -> pending    a handshake proves a key: it becomes the pending agent's key, in place of an earlier one
pending  -> approved   the owner approves, naming the pairing code he saw; the code must be the current key's
pending  -> removed    the owner removes it
approved -> removed    the owner removes it
```

- While an agent is pending, the last proven key is its key. So a key that was lost before the approval, or one
  that another program proved first, does not block the agent for good: the bridge's next call puts its own key
  in place. After the approval the key is fixed.
- The approval names the code and the agent's revision. If the key changed after the owner looked, the code no
  longer matches and nothing is approved.
- A public key belongs to at most one agent.
- `removed` is final. The id and the key are never accepted again; a new enrollment gets a new id and the bridge
  makes a new key for it. There is no way to change the key of an approved agent: a lost key (a reset Keychain)
  means removing the agent and adding it again.

### 5.2 The flow

1. On the Agents screen the owner adds an agent: he picks Claude Desktop, Claude Code or Codex desktop and a name.
   The broker records a pending agent with a new id and where its data goes (the cloud service of that client,
   ADR108 D11), and returns the configuration text (section 6).
2. The owner pastes the text into the client and starts it. The client starts the bridge with the id.
3. At the first tool call the bridge makes its key for that id, connects and proves the key. The broker holds the
   key for the pending agent and answers `pending`. The tool call's answer shows the pairing code.
4. The Agents screen now shows the pending agent with the same code. The owner compares and approves.
5. The broker approves in one commit: the agent becomes usable and gets its list permission (5.5).
6. The agent's next tool call connects, proves the key and gets a session.

### 5.3 The pairing code

`code = the first 12 hexadecimal digits, in capitals, of SHA-256("fabric-desktop/1\nagent_key\n" + "ed25519:<64
hex>")`, shown in three groups of four.

The owner reads it in two places besides the screen: in the agent's answer while it waits, and from
`fabric-mcp-bridge code --enrollment <id>` in a terminal, which does not pass through a model. The approval
command carries the code, so the broker approves exactly the key the owner saw.

What the code shows: that the approved key is one the bundled bridge holds for this id. A program that proved a
key of its own shows a different code on the screen than the bridge prints, and the owner does not approve it.
What it does not show: which program started the bridge. Software of the same user that runs the same bridge
with the same id has the same key and the same code (T3).

### 5.4 The key

- Ed25519. The 32-byte seed comes from the system's random source (`getrandom`) and is wiped from memory after
  use (`zeroize`); copies inside the Keychain's own framework cannot be wiped.
- Made at the first tool call or by `code`, whichever comes first, so an id that is never used gets no key.
- Two processes may start for a new id at once. A key is stored without replacing: the process that loses the
  race discards its key and reads the stored one. Only "there is no key" leads to a new key. Any other failure of
  the store (a locked Keychain, a refused dialog, an unreadable file) is a tool error and never a new key.
- Debug builds with `--key-dir <dir>`: the file `<dir>/<id>.key`, 64 hexadecimal digits and a line feed. `<dir>`
  must be a directory of mode 0700 owned by the bridge's user. The seed is written to a temporary file of mode
  0600 in `<dir>` and then linked to its name, which fails if the name exists, so a key file is whole or absent
  and is never overwritten. A key file must be a plain file, not a link, owned by the bridge's user, with no
  permission for anyone else. An error about a key file never repeats its content.
- Release builds, and debug builds without `--key-dir`: the macOS Keychain. Service
  `ai.paracels.fabric.calendar.bridge` (`...bridge.debug` in a debug build, so that a debug bridge never touches
  the installed one's keys), account the agent's id, not synchronized, this device only. The bridge's items and
  the broker's (the store key, later Google's tokens) must not be readable by each other's program. Built in
  phase 2 with an `#[ignore]` test for the owner. Until then such a build has no key store and answers every tool
  call `broker_not_running`, with the reason on stderr. In phase 2 the bridge also checks the socket before it
  opens the key store, so that no Keychain item is made or read while Fabric Calendar is not running; today the
  key file is made at the first tool call whether or not a broker listens.
- The key never leaves the bridge. The public key and signatures are the only things derived from it that are
  sent.

Which program may read the Keychain item, and what the owner sees when a client first starts the bridge, depend
on how the bridge is signed. That is checked with the owner in phase 2 and settled at CD-W11 (Q3).

### 5.5 The list permission

Approving an enrollment gives the agent the standing permission to list calendars. Two forms can carry it. The
choice touches the store and the rule theory, so it is the lead's and the contract session's (LR-S3, V3-4).

| | (a) A grant until revoked, preferred | (b) A fact about the agent |
|---|---|---|
| What it is | A grant whose `tools` is `[fabric_calendar_list]`, made in the approval's commit | The agent's record says it may list; the rule theory permits a list for such an agent |
| The owner | Sees it among the permissions and can revoke it alone; removal ends it (`agent_removed`) | Sees it on the agent; it ends only with the agent |
| The rule theory | Unchanged: a list is already permitted under a grant that names the tool (`scope_covers ... List_calendars`) | A new context fact and a changed rule, with their proofs |
| The contract | `grant` must allow a scope without a calendar and an origin that is an approval of an agent, not of a request. Both only widen: every grant written under version 2 stays valid with the same canonical form | The agent record gains a member |
| The store | `grants.origin_request_id` is `NOT NULL REFERENCES requests`. Lifting that is a changed constraint, which ADR-115 "Open" forbids until replay knows versions; a form that only adds (a second table for grants without a request, or a nullable column beside) avoids it. The core reads `grant.origin.request_id` for its facts and validity (`service/facts.rs`, `service/grants.rs`) | An added column on `agents` |

Pull request 24, item 8 ("the list tool can never get an approval card") is answered by either: a list no longer
needs a card.

### 5.6 `Broker::seed`

`Broker::seed` stays what it is, a debug seam for the synthetic agent and the practice account (W04 5.10, D11).
A real agent never comes from it. In a release build agents exist only through the approval of 5.2.

For the practice run of S5 a real client enrolls the same way, through the same commands. Today's binary cannot
host it: the practice calendar exists only together with the synthetic agent, which then runs its own script
(`apps/fabric-broker/src/shell.rs`), and a practice read has no path to a result. Both are asked of the lead
(LR-B6, LR-C4).

### 5.7 Removal

The owner removes an agent with one command. In one commit the broker ends its session and closes its connection,
ends its grants (`agent_removed`), ends its open requests in the way the lead's design chooses, and marks the
agent `removed`. The agent stays readable for the history. The bridge's next call gets `not_permitted`.

---------------------------------------------------------------------------------------------------------------------

## 6. The configuration text

One function, `config_text`, makes the text from the client, the bridge's path, the id, the debug extras and the
path rule of the build. Nothing but the path and the id is in the text.

- `bridge_path` is the path of the bridge that will run: in the bundle,
  `<Fabric Calendar.app>/Contents/MacOS/fabric-mcp-bridge`. The `config` command uses its own resolved path; the
  broker (phase 2) uses the bridge beside its own executable and makes no text when no bridge is there.
- A release build makes a text only for a bridge under `/Applications/` or `$HOME/Applications/`. A path under
  `/Volumes/` or one that holds `AppTranslocation` is where macOS runs an application that was not moved into
  place; a text naming it would later start whatever is mounted there. A debug build accepts any absolute path and
  starts the text with a line saying it is a test configuration.
- A path that holds a control character or a character that changes the direction of text is refused.
- Escaping: JSON strings through `serde_json`; TOML basic strings with `\"`, `\\` and `\uXXXX`; the shell line
  with single quotes, a single quote inside written `'\''`. A path with spaces, quotes, `$` or a backslash stays
  one argument in all three.
- The server's name is `fabric-calendar` in all three.

**Claude Desktop.** The text is the entry and one sentence: add the `fabric-calendar` entry to `mcpServers` in
`~/Library/Application Support/Claude/claude_desktop_config.json` (Settings, Developer, Edit Config), beside the
servers already there; then quit Claude Desktop and open it again.

```json
"fabric-calendar": {
  "command": "/Applications/Fabric Calendar.app/Contents/MacOS/fabric-mcp-bridge",
  "args": ["--enrollment", "agent-0123456789abcdef0123456789abcdef"]
}
```

**Claude Code.** A whole `.mcp.json` for a folder of its own, used only for calendar work:

```json
{
  "mcpServers": {
    "fabric-calendar": {
      "command": "/Applications/Fabric Calendar.app/Contents/MacOS/fabric-mcp-bridge",
      "args": ["--enrollment", "agent-0123456789abcdef0123456789abcdef"]
    }
  }
}
```

The text says:

- Claude Code asks once before it uses a server from `.mcp.json`.
- Do not add the entry for the user (`--scope user`), and do not put it into the Fabric repository; to be safe,
  not into a folder above it either. A user-wide entry reaches every Claude Code session on this Mac, and so the
  sessions that build Fabric. Sessions that other programs start load a folder's `.mcp.json` without asking.
- The same entry can be added from that folder with
  `claude mcp add --scope project fabric-calendar -- '<path>' --enrollment <id>`; the text gives the line with the
  path quoted for the shell. Leaving out `--scope project` keeps the entry out of the folder and private to the
  user, for that folder only.

**Codex desktop.** The entry goes into `.codex/config.toml` of the same kind of folder, which Codex reads for a
trusted project, or into `~/.codex/config.toml`, which the ChatGPT desktop application, the Codex command line and
the editor extension share. The text recommends the folder for the same reason as above.

```toml
[mcp_servers.fabric-calendar]
command = "/Applications/Fabric Calendar.app/Contents/MacOS/fabric-mcp-bridge"
args = ["--enrollment", "agent-0123456789abcdef0123456789abcdef"]
```

In a debug build with `--socket` and `--key-dir` the `args` also hold those two pairs, so that a practice run
reaches its scratch broker.

Each client gets its own enrollment and so its own id, key, name, destination notice and pause. That separates
the record and the pause of clients that behave; it is not a wall between them (T3).

---------------------------------------------------------------------------------------------------------------------

## 7. Integration (phase 2)

### 7.1 Request to the lead: what the store must keep

- LR-S1 Per agent: the public key (32 bytes, null until one is proven, unique among agents), the enrollment state
  (`pending`, `approved`, `removed`), and when the agent was added, its key last proven while pending, approved
  and removed. A session end reason for removal: `SessionEndReason` has none today
  (`crates/fabric-storage/src/mutation.rs`), and the grant end reason `agent_removed` should follow from it.
- LR-S2 A migration for it, and new fixed ledger operation types (ADR-115 D2): three owner commands (add, approve,
  remove) and one agent-side operation, a key proven for a pending agent. ADR-115 "Open" says replay is not
  version-aware and must be settled before CD-W05's migration 2; CD-W06 needs a migration too, so the order of
  the two is the lead's.
- LR-S3 The list permission of 5.5: which form, and for form (a) how the store holds a grant without a request.
  This needs the lead's store design before the contract session writes V3-4.
- LR-S4 Audit of failed handshakes (unknown id, removed, wrong key, bad signature, out of time), counted into one
  row per minute as `agent_busy` is, so that a program that fails in a loop cannot grow the table.

### 7.2 Request to the lead: what the core and the broker binary must do

Core:

- LR-C1 Enrollment: the owner commands add (pending), approve (with the code and the revision, giving the list
  permission in the same commit) and remove (5.7); and the agent-side effect "a proven key becomes the pending
  agent's key". For form (a) of 5.5 also the readers of a grant's origin in `service/facts.rs` and
  `service/grants.rs`.
- LR-C2 The handshake's decisions, all after the signature verifies (4.4): the agent's state, the key equal to the
  approved key, the nonce used once. `AgentInput::Connect` only after them.
- LR-C3 The answer `prover_unavailable` while the prover is not ready (CR 8 allows it), if the broker part of
  schema version 2 does not already bring it.
- LR-C4 Held answers and results. (1) The answer to an action is held until the request is `pending_approval`,
  `applied`, in a terminal state or has a released result, or until about 20 s have passed; then the status of
  that moment is the answer. The bridge allows 30 s. (2) Results: today a read ends `undecided` and
  `fabric_request_result` ends `terminal_failure` (W04 D33). For gate G2c on the practice calendar the agent
  needs at least the list (from the broker's own records), the effect of an applied change (the event's id and
  revision, without which it can neither update nor delete what it created), and `fabric_request_result` with its
  own proof. Availability and event reads need the provider's read path: the practice calendar in debug builds,
  CD-W05 for Google.
- LR-C5 The rule of S8 on the broker's side: a status never carries content; a result is released only by a fresh
  proof under current authority; a test polls a completed read after revocation.
- LR-C6 A connection is closed when its agent is removed. Closing at idle end, replacement and quit exists
  already.
- LR-C7 The ports for the handshake. Today `AgentInput` is `Connect`, `Call` and `Disconnect`, and `AgentReply`
  carries a contract `Frame` with no handshake variant (CORE `ports.rs`). The decisions of LR-C2 read the store,
  so the hello and the proof must reach the core, and their answers must come back out.

Binary:

- LR-B1 The socket listener: bind, accept, one reader per connection, the handshake before any call, frames into
  `Input::Agent`, `Output.agent` and `Output.close` back out. A connection that does not finish the handshake in
  5 s is closed. At most 8 connections may be in a handshake at once.
- LR-B2 The path of 4.1: `run` created 0700, the socket 0600; binding from inside `run` by the relative name, or
  a check of the length with a failure visible in `health`. A stale socket file may be removed only by a broker
  that knows no other broker serves this data directory. The store's `broker.lock` says so while the store is
  open; what says so while the store is unavailable is the lead's to design.
- LR-B3 The peer's user id read from the system and compared with the broker's own, on every accepted connection.
  The standard library has no stable call for it, so it needs a crate; none is chosen here.
- LR-B4 The relay's environment allowlist needs no new name for this (`apps/fabric-desktop/relay/src/config.rs`):
  the socket is under `FABRIC_BROKER_DATA_DIR`.
- LR-B5 The configuration text: the broker calls `fabric_mcp_bridge::config_text` with the bridge beside its own
  executable, for the owner method of V3-5.
- LR-B6 A practice mode for a real agent (S5): the practice account and calendar in a debug build with the test
  key, without the synthetic agent's own proposals.

### 7.3 Needs for schema version 3 (to the contract session)

- V3-1 The handshake of 4.4 on the agent channel, in the schemas and in the contract crate: the two methods,
  their params, results and errors; scalars for a public key (`ed25519:` and 64 hexadecimal digits), a nonce
  (64), a signature (128) and a pairing code; the signing rule with the domain `agent_handshake` and the code
  rule with the domain `agent_key`; fixtures with a fixed seed, its public key, its code and its signature
  (Ed25519 signatures are deterministic). The bridge then drops its own `handshake.rs`.
- V3-2 Owner commands: add an agent (name, kind of client), approve an agent (id, expected revision, the code),
  remove an agent (id, expected revision).
- V3-3 A pending agent the owner can see: the agent record carries its enrollment state, its pairing code (null
  until a key is proven) and the times of LR-S1, in the snapshot and in notices. A removed agent stays readable.
- V3-4 The list permission of 5.5, in the form the lead's store design accepts (LR-S3).
- V3-5 An owner method that returns the three configuration texts for an agent. The lead ruled later that it
  returns one text, for the client stored with the agent (13).
- V3-6 README section 8: when the answer to an action comes (LR-C4), that answers on one connection may come in
  any order and are matched by id, and that an adapter connects again after its connection was closed.
- V3-7 README section 15: the handshake is covered; `broker_not_running` is made by the adapter, never sent by
  the broker; the adapter writes `null` for an absent member that may be null (A7).

### 7.4 What the Agents screen S07 needs (to the screens session, later)

- SR-1 Add an agent: the kind of client, a name; then the configuration text with a copy control, and the words
  of section 6 for that client.
- SR-2 A pending agent: "waiting for the client" until a key is proven, then the pairing code and Approve and
  Remove. The text says to compare the code with the one the agent shows, or with
  `fabric-mcp-bridge code --enrollment <id>`.
- SR-3 An approved agent: its name, where its data goes (the destination notice, PLAN R14), connected or not,
  the last tool call, Pause and Resume, Remove, and its permissions with the list permission among them. "For
  this session" is worded "until this client disconnects" (4.5). The lead ruled other words (13).
- SR-4 That Google's credentials stay with Fabric Calendar (PLAN S07).

### 7.5 The bundle and the checks (later, with the lead)

`scripts/build-desktop.sh` builds `fabric-mcp-bridge` with the broker and puts it in `Contents/MacOS/` beside
`fabric-broker`. `scripts/verify-desktop.sh` checks and tests the bridge in release too, as it does the broker
(the release half of test a01 runs only there), and `--packaged` checks that the bridge is in the bundle, that
`--version` answers, and that the configuration text names the bundle's path. A debug bundle holds a debug
bridge, so the practice run can use `--socket` and `--key-dir`. Signing is CD-W11's.

---------------------------------------------------------------------------------------------------------------------

## 8. Threats and residuals

PLAN 4.3 states the residual that bounds this table: "Same-user hostile software is a stated residual threat; a
socket mode is not an OS sandbox."

| # | Threat | Answer |
|---|---|---|
| T1 | Another user of the Mac connects, or puts his own socket in the path | The bridge connects only inside a directory of its own user, mode 0700 (4.2); the broker checks the peer's user id (LR-B3). |
| T2 | Software of the same user connects without a key | The handshake fails; the broker counts it (LR-S4). |
| T3 | Software of the same user starts the bundled bridge and speaks MCP to it | Residual. Any process of this user can start the bridge with any agent's id from a configuration file and act as that agent. An enrolled agent that has a shell (Claude Code, Codex) is such a process, and text in an event may steer it to do so (T7). Enrollments separate the record and the pause of clients that behave; they are not a wall between agents. The broker's rules, the owner's approvals and pause bound what any of them can do. A changed `HOME` in a client's entry is the same case. |
| T4 | Software of the same user puts itself between the bridge and the broker | Residual. It can do so at any time by renaming the socket and binding its own. It then acts as the agent (T3), and also reads and changes that agent's traffic and what the model is told. It learns no key. It cannot put free text into the bridge's guidance (A27); what it can place there is a request id or a rule's name, at most 128 or 64 characters of letters, digits, `.`, `_` and `-`. |
| T5 | A pending agent's key is proven by another program first | The bridge's next call puts its own key in place (5.1); the owner approves by the code (5.3). A program that proves keys in a loop can keep the owner from approving: a denial of service by same-user software, residual. |
| T6 | A recorded handshake is replayed | The broker's nonce (4.4). |
| T7 | Text in an event steers the model | ADR108 D11, accepted. The bridge's guidance is fixed wording (A27); content is data in the second block. |
| T8 | A secret in configuration, arguments, stdout or logs | None is put there (A24, A25). The seed exists in the Keychain, in the bridge's memory, and in a debug build's key file. |
| T9 | Oversized or malformed input, from the client or the broker | The bounds of 2.5; strict contract JSON on the socket; a broken connection is dropped. |
| T10 | The agent reaches an owner method | The agent socket has none, and the bridge has no name for one (3.6). |
| T11 | A stale answer is shown after authority ended | The bridge keeps nothing (A12). |
| T12 | A flood of connections or failed handshakes at the broker | A cap on handshakes in progress (LR-B1) and counted audit rows (LR-S4). |
| T13 | Two processes replace each other's session on purpose, ending "this session" grants each time | Residual, same-user software (T3). The owner sees the sessions change on the Agents screen. |
| T14 | The configuration text names a bridge on a mounted volume or a translocated copy | A release build makes no text for such a path (section 6). |
| T15 | A lost or doubled key | A key is stored without replacing, and only "no key" makes a new one (5.4). A lost key of an approved agent means remove and add again. |

---------------------------------------------------------------------------------------------------------------------

## 9. Platforms and CI

The crate builds on macOS, Linux and Windows. The MCP side, the tools, the rendering, the state machine, the
handshake's text and signature, the pairing code and the configuration text have no platform code and are public
in the library, so nothing is dead code on Windows. The socket, the key file and the process tests are
`cfg(unix)`; on Windows a tool call answers `broker_not_running`, "not supported on this system". Linux runs
every test. The Keychain store of phase 2 is `cfg(target_os = "macos")`.

The process tests need the debug seams, so each file under `tests/` is `#![cfg(all(unix, debug_assertions))]`.
The stand-in broker binds its socket by its full name, which must stay under 103 bytes on macOS: the tests keep
their directory names short and assert the length.

Lessons of earlier Windows runs apply (paths print with doubled backslashes under `{:?}`; a lint about type size
can fire only there).

---------------------------------------------------------------------------------------------------------------------

## 10. Test plan

No test touches the Keychain, starts a real client, starts a broker or reads a file outside its scratch
directory.

### 10.1 Unit tests (in the source files)

| Id | Module | Shows |
|---|---|---|
| m01 | `mcp/server` | `server/discover` answers `resultType`, `supportedVersions: ["2026-07-28"]`, the tools capability, `ttlMs`, `cacheScope`, `serverInfo` and instructions of at most 512 characters |
| m02 | `mcp/server` | a request with no protocol version and no answered `initialize` gets `-32602` |
| m03 | `mcp/server` | another version gets `-32022` with the supported list and the requested value; a missing `clientCapabilities` gets `-32602` |
| m04 | `mcp/server` | `initialize` answers `2025-11-25` for that version and for any other |
| m05 | `mcp/server` | legacy `tools/list` and `tools/call` results have no modern member; modern ones have all of them |
| m06 | `mcp/server` | `ping` is answered in legacy, also before `initialize`, and unknown in modern |
| m07 | `mcp/jsonrpc` | bad JSON `-32700` and an array `-32600`, both without an `id` member; a bad id `-32600`; an unknown method `-32601`; ids come back as sent, string or integer |
| m08 | `mcp/server` | `subscriptions/listen` gets the acknowledgement as a notification under its id and no response; a later `notifications/cancelled` for it produces nothing; a ninth open one gets `-32603`; one without `notifications` gets `-32602` |
| m09 | `mcp/server` | no notification is ever answered; `tools/list` with a cursor gets `-32602` |
| t01 | `mcp/tools` | the ten names are `ToolName::ALL` and the catalog, in order, with the catalog's descriptions |
| t02 | `mcp/tools` | no `inputSchema` holds `$ref`, `oneOf`, `anyOf`, `const` or `null`; each root is an object schema; each member that may be null is absent from `required` |
| t03 | `mcp/tools` | the hints of 3.5, tool by tool |
| t04 | `mcp/tools` | each example of rule 4 is accepted by the contract's own type for that format |
| t05 | `mcp/tools` | filling in: an absent member that may be null becomes `null` at every depth; a present one, null or not, is untouched; nothing else is added |
| r01 | `mcp/render` | each of the 23 states gives the `isError` and the wording class of 3.7; the second block is the canonical `tool_outcome` |
| r02 | `mcp/render` | each of the agent error codes, named one by one, gives `isError: true`, its wording and the canonical error |
| r03 | `mcp/render` | the first block holds none of a result's text, a rule's explanation or an error's message (A27) |
| b01 | `bridge` | arguments the contract refuses never reach the broker; the answer is `invalid_params` with the pointer `/params` |
| b02 | `bridge` | an unknown tool name, an owner method's name among them, gets `-32602` and sends nothing |
| b03 | `bridge` | a call is answered by the one broker answer with its id; no other frame is sent for it (A10, A11) |
| b04 | `bridge` | after 30 s without an answer that call alone is the tool error of 3.7; the connection and the other calls stay |
| b05 | `bridge` | a cancelled call gets no response, and its late answer is dropped |
| b06 | `bridge` | a closed connection answers the calls in flight; the next call connects again |
| b07 | `bridge` | answers that come out of order reach their own calls |
| b08 | `bridge` | the seventeenth call in flight is a tool error |
| b09 | `bridge` | a result shown in one call's answer appears in no later answer (A12) |
| k01 | `broker/handshake` | the signed text and the signature for a fixed seed and nonce; a changed nonce changes it; an answer with a wrong nonce form, another schema version or another id is refused |
| k02 | `broker/key` | the pairing code of a fixed key, twelve digits in three groups |
| k04 | `broker/handshake` | the public key, the pairing code, the signed text and the signature equal two vectors of the contract draft's own fixture (schema version 3, pull request 36), the first on the seed of RFC 8032 test 1 |
| k03 | `broker/key` | a key file is made 0600 in a 0700 directory and read back; a link, a file others can read and a loose directory are refused; two makers at once end with one key; an error never repeats the file's content |
| s01 | `broker/socket` | the path from `HOME`; no `HOME` is a refusal |
| s02 | `broker/socket` | a directory not 0700, a socket not 0600, a link in place of the socket and a plain file in its place are refused |
| c01 | `config_text` | the three texts for a path with a space, a double quote, a single quote, `$`, a backtick and a backslash parse back to the same path, the shell line included |
| c02 | `config_text` | a text holds the path and the id and nothing else; the debug extras appear only when given; a path with a line feed, a tab or a direction mark is refused; a release build refuses a path outside the two application folders |
| a01 | `cli` | a wrong command line is exit 64; a release build refuses `--socket` and `--key-dir`; the ids `..`, `a/b`, `.hidden` and the empty one are refused before any file is touched |

### 10.2 Process tests (`tests/`, the real binary, Unix, debug builds)

The MCP test client starts the built bridge, writes JSON-RPC lines and reads answers with a time limit. It is
written from the specification's text, by another hand than the server's, and knows both revisions. The stand-in
broker listens on a socket in a scratch directory (0700 and 0600), verifies the handshake with its own reading of
4.4, reads every other frame with the contract's `decode`, and answers from a script of contract values, at once
or after a pause.

| Id | Shows | Stands in for |
|---|---|---|
| p01 | Modern: `server/discover`, `tools/list`, a list call, its result | G2c: a client connects and reads; PLAN 14 "a real availability read" (shape only) |
| p02 | Legacy: `initialize`, `notifications/initialized`, `tools/list`, a call | the same for a client on the older revision |
| p03 | A create is answered `pending_approval`, the stand-in holding its answer for a moment first; a later status call answers `applied` | G2c pending and allow paths; PLAN 14 "a pending approval, an approved create" |
| p04 | A blocked request shows its rule | G2c block path; PLAN 14 "a denied request" |
| p05 | A paused agent's call shows `blocked` by `agent_not_paused` | G2c pause path |
| p06 | `undecided`, `prover_unavailable` and `busy` are relayed as 3.7 says | PLAN 14 "Kill Isabelle or exceed proof budget" (the agent's view) |
| p07 | With no socket, `tools/list` works and a call answers `broker_not_running`; after the stand-in starts, the next call works with the same bridge process | PLAN 12 "a clear Fabric-not-running response"; A5, A26 |
| p08 | A key the stand-in refuses gets `not_permitted` with the bridge's code; nothing but the handshake was sent | PLAN 14 "Unregistered agent ... through MCP: denied" |
| p09 | A pending agent answers with the pairing code, the same as `code` prints; after the stand-in approves, the next call gets a session | 5.2, 5.3 |
| p10 | One connection per run; end of input closes it; a connection the stand-in closes is replaced at the next call | 4.5, session start and end |
| p11 | The socket's directory, the key directory and the bridge's own path hold spaces | PLAN 14 "a path containing spaces" |
| p12 | stdout holds only JSON-RPC lines; stderr never holds a marker placed in an argument, in a refused argument, in a result and in a broker message | A25, PLAN 12 |
| p13 | A read's result is shown once; a later status call, which the stand-in answers without a result, shows none of it | PLAN 14 "Poll a completed read after ... revocation" (the bridge's half) |
| p14 | A line over the bound from the client, and a malformed frame from the stand-in | T9 |
| p15 | A socket of mode 0666 and a directory of mode 0755 are refused | 4.2 |
| p16 | Two calls in flight on one connection are both answered, the second answer first | 3.6 |
| p17 | A cancelled call gets no response | 3.9 |
| p18 | An omitted description reaches the stand-in as `null`; a response target written as one object reaches it as the contract's union | 3.5, 3.6 |

### 10.3 What only the real broker shows (phase 2)

Every row of 10.2 again against the broker's own listener, in a debug build with the test key
(`FABRIC_BROKER_KEY_SOURCE=test`) and a scratch data directory: enrollment through the owner commands, the list
permission, held answers, revocation while polling (S8), removal, restart, idle end. The allow path needs LR-C4
and LR-B6. The block and pause paths need the rule prover: the synthetic classifier answers only "ask the owner"
(W04 5.10).

### 10.4 What only a live run with a real client shows

None of this is checked by phase 1 or phase 2:

- which revision Claude Desktop, Claude Code and Codex desktop each choose, and that each completes
  `server/discover` or `initialize` with the bridge;
- that each client accepts the ten tool schemas and that its model fills them; in Claude Desktop in particular,
  that an omitted description and an omitted title change reach the broker as `null`, and what a model's explicit
  `null` becomes (R2);
- how each client shows the two text blocks, an `isError` result and a long read result (R5);
- that each client passes `HOME` to the bridge (R3);
- the Keychain: the dialog at first use, and which program may read the key (Q3);
- the client's own confirmation prompts beside Fabric Calendar's approval;
- the destination notice for a cloud agent's approved read (PLAN 14, R14);
- the gate itself: for each client a real availability read, a denied request, a pending approval, an approved
  create, an edit, an invitation to an owned test recipient, pause and revocation, with the provider's outcome
  and the proof (PLAN 14).

---------------------------------------------------------------------------------------------------------------------

## 11. Risks and questions

### 11.1 Risks

- R1 The handshake is built before the contract has it. The contract session may change names or shapes; the
  types are in one file for that reason (4.4).
- R2 A client may change a tool schema before its model sees it. Claude Desktop is reported to strip a
  null-or-value union and to pass a null as the text "null"; Codex drops `pattern` and bounds (appendix A). The
  emitted schemas avoid unions and nulls and carry examples for that reason. What remains: a model that sends an
  explicit `null` for an optional text member through Claude Desktop could have it arrive as the word "null",
  which the contract accepts as text. Only a live run shows whether that happens (10.4).
- R3 A client that starts the bridge without `HOME` gets `broker_not_running`. Codex passes `HOME` by default
  (appendix A). The remedy, if a client does not, would be the socket path in the configuration text.
- R4 A client that knows only a legacy revision older than `2025-11-25` is answered with `2025-11-25` and may
  give up. None of the three installed clients is one.
- R5 A long read result (up to 1 MiB) may be cut or put aside by the client: Claude Code saves a text result of
  more than 50,000 characters to a file. The bridge does not shorten a result; an agent should ask for narrow
  windows and few fields.
- R6 Two processes of one agent replace each other's session, and a session is a client process, not a
  conversation (4.5).
- R7 The broker's socket name is too long for a user name over 18 bytes unless the broker binds as LR-B2 says.
- R8 A retried tool call is a new call: MCP has no key to tie a retry to its first try. A client that repeats a
  proposal makes a second request and a second card. The guidance says not to repeat; the request limits bound
  it.
- R9 Until the broker holds its answers (LR-C4), every action is answered "still working" and no result reaches
  an agent.
- R10 The own MCP code and the own test client could share a misreading of the specification. The two are written
  apart (10.2); the live runs decide.
- R11 Under CD-W05's design a read also ends `reconciled` (the lead's notice of 2026-11-17). The wording of 3.7
  for `applied` and `reconciled` speaks of a change and must then tell a read from a change. Not changed here; it
  follows the lead's design for phase 2.
- R14 A call that the client cancels still gets its result released by the broker, into an answer the bridge
  drops (section 13). The lead asked for a cancel message in schema version 3 to close it (request P2-30 of the
  phase 2 design); until the contract has one it is a residual.
- R13 The bridge does not compile against a contract crate that has a request state its table lacks (3.7), and
  schema version 3 adds three. Whichever of the two pull requests reaches `main` second must add the three lines
  and their wording to `apps/fabric-mcp-bridge/src/mcp/render.rs` and to test r01. This session does it as soon
  as it is told.
- R12 The contract draft for schema version 3 makes a waiting enrollment a record of its own, with no agent until
  the approval, where 5.1 puts `pending` on the agent (the lead's notice of 2026-11-17). The lead settles which
  in the store design. The bridge sees only `pending` or `approved` in the handshake's answer, so either fits it.

### 11.2 Questions

None stops phase 1. The code runs on the default after each.

- Q1 Enrollment starts in the application and the bridge supplies the key later (A18). PLAN 4.3 words it as "the
  bridge generating a private key ... and a public enrollment ID/key". Default: as A18, because the configuration
  must hold the id before the bridge first runs.
- Q2 The agent's answer shows the pairing code while it waits (5.3). Default: yes, beside the `code` command.
- Q3 How the bridge is signed decides what the Keychain asks when a client first starts it, and whether a debug
  rebuild asks again. Default: accept one dialog per rebuilt debug bridge, as W04 Q3 does for the store; settle
  the rest at CD-W11.
- Q4 A pending agent never expires; the owner removes it. Default: no expiry.
- Q5 One live session per agent stays (R6). Default: yes; two windows of one client are not a case of version 1.
- Q6 The list permission as a grant the owner can revoke alone, or as a fact of the agent (5.5). Default: the
  grant, if the lead's store design can hold it.

---------------------------------------------------------------------------------------------------------------------

## 12. The critique of the first draft

Three independent readers read the first draft on 2026-11-17, before any code: one against the MCP specification
and the clients' documentation, one for security, one against the repository. What they changed:

- Waiting moved from the bridge to the broker (A10, 3.8, LR-C4): a status can never carry a result, so the
  draft's polling could not deliver one.
- The tool schemas lost their unions and nulls (A6, A7, 3.5, 3.6), after the reports about Claude Desktop.
- The socket checks compare owners with the bridge's own user id and hold the directory by standing in it (4.2).
- A key is recorded only after its signature verifies, the last proven key holds while pending, and the `code`
  command was added (4.4, 5.1, 5.3).
- The guidance block lost every free text of the broker (A27).
- The threat table states T3 and T4 as the residuals they are, and gained T12 to T15.
- `subscriptions/listen` is acknowledged by a notification; errors without a readable id carry no id; a cursor is
  refused (3.3, 3.4).
- The requests of section 7 gained the ports for the handshake, the held answers, the practice mode, the session
  end reason for removal, and the store's constraint on a grant's origin; one request that already exists was cut
  down (LR-C6).
- `outcome_unknown` is no longer shown as final (3.7).

---------------------------------------------------------------------------------------------------------------------

## 13. The lead's design for phase 2 and what the bridge does about it

The lead answered section 7 in `docs/plans/calendar-desktop-phase2-design.md` (pull request 43, Proposed, not
merged on 2026-11-17): its section F has a verdict for every `LR-n`, and its section H.3 asks nine things of this
session (W6-1 to W6-9). Nothing below is built in phase 1. It is the bridge's part of phase 2, after schema
version 3 is on `main`.

| Request | What the bridge does |
|---|---|
| W6-1 | A connection closed with no answer during the handshake is already `broker_not_running` for that call, and the next call connects again. The wording for `not_permitted` at the handshake changes: the broker also answers it when an agent connected more than six times in a minute or changed its key within a minute, so the sentence must say "or wait a minute and call again" beside "the owner can look". `config_text` stays public, without platform code, with its present signature |
| W6-2 | The answer to `agent_prove` gains `practice`. While it is true, the first block of every tool result of that connection begins with the fixed sentence "Practice run: changes go to a practice calendar in memory; nothing real changes." |
| W6-3 | `completed` with a result is the result row. A status call can also show `completed` without a result (a status never carries one); its sentence is "The request is complete." (3.7) |
| W6-4 | A read ends `reconciled` and its result comes only through `fabric_request_result`. On a status call the bridge does not know which tool made the request (A12), so `applied` and `reconciled` get one sentence that fits a change and a read: Fabric Calendar has carried out the request; call `fabric_request_result` with this `request_id` for its result. This settles R11 |
| W6-5 | Unchanged: `prover_unavailable` is relayed and not retried |
| W6-6 | The lead puts the bridge into the debug application in its step; this session reviews it (7.5) |
| W6-7 | The bridge never sends a tool frame before an approved handshake, so the close after `not_permitted` needs nothing new. The bridge's own `handshake.rs` gives way to the contract crate's types |
| W6-8 | Accepted: the debug key directory `<data_dir>/bridge-keys` (0700, made by the broker, the same user) and `--socket <data_dir>/run/agent.sock` in the debug configuration text. The key store's checks of 5.4 hold for that directory as they are |
| W6-9 | The broker binary depends on the bridge's library for `config_text`. Accepted as it is; a smaller crate only if the weight ever matters |

Three things this design said that the lead's design changes:

- While the store cannot be opened there is no listener, so the bridge answers `broker_not_running`; the handshake
  answers `internal` only for a fault (4.4 expected `internal` for a locked store).
- A pending agent's key may be replaced by a newly proven one at most once a minute and twenty times (5.1 set no
  limit).
- The broker binds the socket by its full name and checks the length, so R7 stays for a user name over 18 bytes.

One point the lead's design did not cover, recorded here as R14: when a client cancels a call (3.9), the broker
is not told, because the contract has no cancel message. The broker still holds the answer and releases the
result into an answer that the bridge drops. For a result given under a permission for one request, that
permission is then used up and the agent must ask again.

The lead ruled on these points on 2026-11-17. The three wordings above (W6-1, W6-3, W6-4) stand. For R14 the lead
asks the contract session for a cancel message on the agent channel in schema version 3: the adapter names the id
of a call it no longer waits for, the broker drops that held answer without a reply, and a release that finds no
waiting call releases nothing and uses up no permission (request P2-30 of the phase 2 design). The bridge will
send it on `notifications/cancelled` in its phase 2, once the contract has it. Until then, and for a cancel that arrives after the release was committed, R14 is a
residual of the same kind as a lost answer, which is the owner's to decide. The socket path limit (R7) is
accepted for version 1 and goes to the owner for CD-W11. Whichever of pull requests 36 and 38 merges second
carries the bridge's lines for the three new states (R13). This session wrote them as a patch against the
contract draft and checked it there, last at the draft's head bf9e6db: the crate's 64 tests pass with the draft's
contract crate. The patch is in the sessions' shared notes folder, `cd-w06-bridge-for-schema-v3.patch`.

The contract draft at bf9e6db already carries what the bridge's phase 2 will use:

- `agent_cancel`, a method that is not a tool: params `{call_id}`, the frame id of a call whose answer the adapter
  no longer waits for; result `{held}`. The call it names never gets an answer. This is the message R14 waits
  for. The bridge sends it for a call in flight when the client sends `notifications/cancelled`.
- `practice` in the answer to `agent_prove`, a required member (W6-2). The phase 1 bridge reads its provisional
  answer strictly and would refuse the member, so the switch to the contract's types comes before any broker
  listens.
- The busy limit `sessions_per_minute_per_agent`, as an answer to `agent_prove`. The bridge writes a limit's
  name from the contract's value, so it shows this one without a change; the handshake's `busy` gets the same
  sentence as a call's.
- Only the answer to `fabric_calendar_list` and the answer to `fabric_request_result` ever carry a result; a
  read's own answer never does. The first row of 3.7 already says "a result is present", whatever the tool.

Two later rulings of the lead touch the edges of this design (the phase 2 design at b443c1f). Neither changes the
bridge.

- One configuration text, not three. The client the owner names at `add_agent` is stored with the enrollment, and
  `get_agent_configuration` answers the text for that client only (request P2-35 of the phase 2 design, its
  section C.10). This replaces V3-5. The broker calls `config_text` once with the stored client. The function
  already takes one client and returns one text, and its four errors come from the paths and the build, never
  from the client, so its signature stays. The contract's names for the clients (`claude_desktop`, `claude_code`,
  `codex_desktop`) are not the bridge's command line names (`claude-desktop`, `claude-code`, `codex`), so the
  broker maps the three values with a `match` and does not go through `Client::from_name`. The bridge's `config`
  command still prints the text for any of the three.
- The words for a session permission. The screen keeps "until this agent's current session ends" and adds a fixed
  note that lists the ends of a session: the client closes or quits, 30 minutes pass without a call, Fabric
  Calendar quits or restarts, another copy of the client connects with the same agent, or the owner removes the
  agent. This replaces the recommendation "until this client disconnects" of 4.5 and SR-3. The facts of 4.5 stand
  and are the ones the note uses.

---------------------------------------------------------------------------------------------------------------------

## Appendix A. Evidence for the protocol revisions and the clients (2026-11-17)

Read from the specification:

- "The current protocol version is 2026-07-28" (MCP "Versioning").
- No `initialize`; per-request `_meta`; `server/discover` mandatory; `resultType` on every result; `ping` removed;
  `ttlMs` and `cacheScope` on `server/discover` and `tools/list`; error codes `-32021` and `-32022`; the
  acknowledgement of a subscription as a notification (MCP 2026-07-28 "Key Changes", "Overview", "Discovery",
  "Tools", "Caching", "stdio", "Cancellation", "Subscriptions").
- The compatibility matrix and the stdio probe (MCP 2026-07-28 "Versioning and Compatibility", "stdio").

Read from the clients' documentation:

- Claude Code: its v2 runtime "adds MCP protocol revision 2026-07-28" and "asks HTTP, stdio, and claude.ai
  connector servers whether they support the newer revision"; it is the default from version 2.1.232 or 2.1.274
  depending on the session; "Claude Code can skip asking a stdio server for 7 days, for example after the server
  exits when asked"; a project's `.mcp.json` is approved once in an interactive session and loaded without asking
  by `claude -p` and Agent SDK sessions; a stdio call with no answer ends after 30 minutes; a text result over
  50,000 characters is saved to a file (https://code.claude.com/docs/en/mcp).
- Codex: `~/.codex/config.toml` with `[mcp_servers.<name>]`, `command`, `args`; `.codex/config.toml` for trusted
  projects; "The ChatGPT desktop app, Codex CLI, and IDE extension share this configuration"; a tool call's
  default limit is 60 s (https://learn.chatgpt.com/docs/extend/mcp).
- Claude Desktop: `mcpServers` in `claude_desktop_config.json`; a server's stderr goes to
  `~/Library/Logs/Claude/mcp-server-<name>.log`
  (https://modelcontextprotocol.io/docs/develop/connect-local-servers).

Reports about tool schemas:

- Claude Desktop: the open issue anthropics/claude-code#56263 (opened 2026-05-05, last updated 2026-10-22) says
  that a property-level `anyOf` of a type and `null` "is silently stripped before reaching the model" in Claude
  Desktop, and that Claude Code's command line is not affected. The issue was read; the behaviour was not
  reproduced here. One of the readers reports from the issue's comments that an explicit null then reaches the
  server as the text "null", and from #78237 (closed) the same loss for a property-level `oneOf`.
- Codex: one of the readers read Codex's source (`codex-rs/tools/src/json_schema`) and reports that its schema
  type keeps `type`, `description`, `enum`, `items`, `properties`, `required`, `additionalProperties` and the
  combinators, drops `pattern` and the length bounds, and that its default environment for a stdio server
  includes `HOME`. This was not re-read for this document.

Read on this Mac, from files only; no client was started:

| Client | Version | Found in its program file |
|---|---|---|
| Claude Code | 2.1.325 (`~/.local/share/claude/versions`) | the strings `2026-07-28` and the list `2025-11-25`, `2025-06-18`, `2025-03-26` |
| Claude Desktop | 5.11492.5 (`/Applications/Claude.app`) | the strings `2026-07-28`, `2025-11-25`, `2025-06-18`, `2025-03-26`, `2024-11-05` |
| Codex desktop | ChatGPT.app 26.1111.21308 with Codex 0.169.0-alpha.11.1 | `server/discover`, `2026-07-28`, `2025-11-25`, and the text "MCP discovery requires legacy initialization" |

That a client's file holds a revision's name does not show which one it uses with this bridge. Only the live runs
of 10.4 show that.

The Rust SDK: `rmcp` 3.0 added `2026-07-28` (its release notes, found by search). Which 3.x release is the newest
was not pinned down. Its API and its dependency tree were not read.
