# Fabric Policy Demo — Isabelle-Gated Calendar Agents

**Status:** approved on 2026-10-18; version 1.8 records gates G1 and G2, the choices of Phases 3–8, and the Phase 9 results. Phases 0–9 are done; gate G3, your own run, is next.

**Prepared:** 2026-10-18

**Scope:** a working, single-machine demo in which LLM agents negotiate and book a meeting while Isabelle/HOL proves every action against explicit rules and per-action tokens gate every effect
**Related:** [Product requirements](PRD.md) · [Technical architecture](architecture.md) · [Calendar vertical](calendar.md)

---

## 1. What the demo must show

Two people, Alice and Bob, each have a private calendar and an agent that plans with an LLM. Both agents use Claude by default; either can run on a local model through Ollama instead, chosen per run (Q27). The agents agree on a 30-minute meeting and add it to both calendars. Along the way the demo shows that:

1. Calendar data sits in a separate, encrypted service per person. Agents reach it only through a loopback API that demands a valid token on every call, and they only ever receive free/busy intervals, never event details.
2. On each turn an agent's LLM writes 2–3 alternative plans. Each plan is an ordered list of typed actions.
3. Isabelle/HOL checks every action in every plan against a rule theory. The dashboard shows each verdict, the rule that decided it, and the proof.
4. An action runs only with a token minted from an Isabelle theorem about that exact action. Tokens are single-use, short-lived, and bound to the action's arguments and to the agent that asked.
5. The agents handshake and negotiate over an encrypted relay with typed, signed messages and an optional short note.
6. Each agent adds the meeting only after both owners clicked yes for that exact meeting.
7. Dashboard switches trigger four denials on demand: an over-broad calendar read, too many candidates, a write without consent, and token misuse.

The demo shows the semantics and how they are enforced. It is not production security; section 15 lists what it does not protect.

## 2. Decisions recorded from the Q&A

| # | Topic | Your decision |
|---|---|---|
| Q1 | How Isabelle checks actions | A live proof per action through a running `isabelle server`. A token is issued only after the proof succeeds. |
| Q2 | Language and location | Rust, as new crates in this workspace. |
| Q3 | LLM backends | Claude API (cloud) and Ollama (local). |
| Q4 | Calendar | A synthetic local store seeded with fixture events. |
| Q5 | Participants | Two people. Alice's agent on Ollama and Bob's on Claude by default; either can be switched per run in config. Backends superseded by Q27. |
| Q6 | Agent connection | Signed and encrypted messages through a relay process. |
| Q7 | Message content | Typed protocol messages plus a short free-text note. |
| Q8 | Cloud model's view | Free/busy within the negotiation window. No titles or other event details. |
| Q9 | Calendar isolation | A separate calendar service process per person. Its loopback API answers only requests that carry a valid per-action token. The store is encrypted at rest with a per-user key. |
| Q10 | Owner consent | Each owner says yes or no to each mutual option. Add-to-calendar only for a meeting both owners accepted. |
| Q11 | Encryption | The Noise protocol framework. |
| Q12 | Local model's view | Same as the cloud model: free/busy only. |
| Q13 | Token format | COSE_Sign1 over CBOR, Ed25519. |
| Q14 | UI | A local web dashboard, plain HTML and JS served by Rust. |
| Q15 | Platform | Windows, native. |
| Q16 | Note limits | A length cap and no digits. |
| Q17 | Rule groups | The disclosure budget, plus the calendar-view and note rules. |
| Q18 | Denials to show | All four: over-broad read, too many candidates, write without consent, token misuse. |
| Q19 | On denial | The agent receives the rule name and the failed condition, and may re-plan a bounded number of times. |
| Q20 | Where this spec lives | `docs/` on a new branch. |
| Q21 | Who may be handshaked | Only the participant named in the owner's request, identified by a pre-configured public key. No contact list and no TOFU. |
| Q22 | Consent rule | A minimal rule: add-to-calendar only for the exact meeting both owners said yes to. No once-only or own-calendar checks. |
| Q23 | Alternatives | Alternative plans: 2–3 action lists per turn; the first plan whose actions are all permitted runs. |
| Q24 | Triggering denials | Fault-injection switches in the dashboard. |
| Q25 | Anthropic API key | Read from a git-ignored `.env` file at the repository root, which you create. An `ANTHROPIC_API_KEY` already set in the environment takes precedence. |
| Q26 | API workspace | Your key is not scoped to a workspace, so the Claude backend sends the `anthropic-workspace-id` header with the ID from `ANTHROPIC_WORKSPACE_ID` in the same `.env` file. |
| Q27 | Agent backends (gate G1) | Both agents use Claude by default; Ollama stays available as a per-run option. Phase 1 measured 3–4B local models picking the correct plan in only 2–4 of 10 turns. |
| Q28 | Rules and test corpus (gate G2) | `Fabric_Policy.thy` and the 39-case corpus are approved as written. `read_window` bounds a read by the request's window only: the 14-day `horizon` limits what an agent sends, not what it reads. A decision must match the owner's signed record, checked under `well_formed`. Each side's `horizon` and `consent_lifetime` use its own owner's request. The consent rule stays minimal (D12), and notes may not contain digits (D13). |

## 3. Decisions that need your confirmation

Your answers left the gaps below. You approved every proposed default on 2026-10-18; the alternatives stay here for the record.

### 3.1 Flow and planning

| # | Proposed default | Alternative |
|---|---|---|
| D1 | **Plans cover one turn.** A turn ends at the next outside event: an inbound message, a result the agent asked for, or an owner's click. An agent cannot know Bob's answer or the owners' clicks in advance, so plans for the whole negotiation would be mostly guesses. | Whole-negotiation plans with placeholders that are filled in later. |
| D2 | **Every action is checked twice.** Once when plans are screened (no token), and again right before it runs. Only the second check mints a token, against the state at that moment, because consent can arrive and rounds can advance in between. | Mint tokens at screening time. |
| D3 | **Actions that return information end the plan.** `ReadFreeBusy` is always the last action of a plan; the next turn sees its result. | Let plans continue after a read, with later actions fixed before the result is known. |
| D4 | **The LLM is a planner, not a tool caller.** Each turn is one LLM request that returns plans as JSON matching a schema. The agent runtime, not the model, submits actions to Isabelle and presents tokens. The prompt states the rules in plain words, and Isabelle still checks everything. | Model-driven tool calls. Harder to show alternatives, and harder for a small local model. |
| D5 | **Re-planning limit:** 3 attempts per turn after denials; then the agent stops and the dashboard says so. | Another limit. |

### 3.2 Semantic layer (Isabelle)

| # | Proposed default | Alternative |
|---|---|---|
| D6 | **Isabelle proves denials too.** For each rule, Isabelle must prove either the rule or its negation for the concrete action. Permit means every rule is proven. Deny means at least one negation is proven, and that rule is reported. If neither is proven within the timeout, the action is denied as `undecided`. A failed proof attempt alone never counts as a violation. | Treat any failed proof as a denial. Simpler, but it cannot tell "violates the rule" from "could not decide". |
| D7 | **Kernel-checked proofs.** Verdicts are proven with Isabelle's simplifier over the policy definitions, so each verdict is a theorem checked by the kernel. `value` is used only to compute the numbers shown in denial messages. | `by eval`, which is faster but trusts the code generator instead of the kernel. |
| D8 | **Batched screening.** One generated theory per screening pass holds every action of every alternative, each proven separately inside it. Each execution-time check is its own theory. | One theory per action at screening time too, at the cost of more round trips. |
| D9 | **One shared Isabelle server** on the machine, used by both policy gates as a stateless prover. Each check is its own theory and is purged afterwards. | One server per person: more memory, cleaner separation. |
| D10 | **Policy sanity lemmas**, proven when the policy session builds. For example: event details are never readable; `AddMeeting` is never permitted without both owners' yes for that exact meeting; a permitted proposal never has more than 3 candidates. | Per-action checks only. |
| D11 | **Protocol well-formedness rules.** A message must refer to the active negotiation and round, a response must cover exactly the received candidates in order, and a decision must refer to a mutual option. These go beyond the rule groups you picked. | Leave them out. The disclosure budget still bounds what each side reveals. |
| D12 | **Gaps in the minimal consent rule.** Your minimal rule permits (a) a second `AddMeeting` for the same meeting with a fresh token, and (b) adding several meetings if both owners said yes to several options. Replaying the same token is still rejected. Keep the minimal rule as is? | Add "once per negotiation" and "only the first option both accepted, in proposal order", which is the rule `fabric-core` already implements. |
| D13 | **Notes are printable ASCII** (character codes 32–126), at most 200 characters, with no `0`–`9`. Restricting to ASCII closes the loophole of non-ASCII digits. | Allow any Unicode text and ban only ASCII digits. |

### 3.3 Components and security

| # | Proposed default | Alternative |
|---|---|---|
| D14 | **One trusted host process per person**, beside the calendar service. It holds that person's keys and runs the policy gate (talks to Isabelle, mints tokens), the messaging gateway (the Noise endpoint) and the consent signer. The agent process holds no keys except its own token-binding key and its LLM credential. **The agent supplies only the proposed action; the host supplies every fact Isabelle reasons about.** | Separate processes for the gate, gateway and signer: more processes, same boundaries. |
| D15 | **Where tokens are verified.** Calendar actions at the calendar service; handshake and send actions at the messaging gateway. The relay needs no tokens and sees only ciphertext. This refines your "calendar service or relay" answer so the relay learns nothing about actions. | The relay also checks a minimal mailbox token. |
| D16 | **Signed inside encrypted.** Every protocol message is a COSE_Sign1 signed by the sender's Ed25519 identity key, then encrypted in the Noise session. Consent records are signed the same way, so Alice's gate can verify Bob's yes without trusting either agent. Isabelle receives the results of these signature checks as facts. | Rely on Noise's authentication alone. It is not transferable, so Alice's gate could not verify Bob's consent on its own. |
| D17 | **Noise pattern IK** (`Noise_IK_25519_ChaChaPoly_BLAKE2s`). The caller is authenticated in the first message, so the responder can check it is the configured participant before answering. Each side needs its own `Handshake` token. The XX pattern mentioned in the earlier question would reveal the responder's key before it knows who is calling. | XX with a key check after the handshake, or KK. |
| D18 | **Both owners file a request** naming the other person: duration, window, and a free-text preference. Bob's request is his pre-authorization to negotiate with Alice. Alice coordinates. | Only Alice files a request; Bob's side uses a standing configuration. |
| D19 | **Inbound messages need no token.** The gateway delivers decrypted inbound messages to its own agent; only actions the agent starts need tokens. | Make "receive" a tokenized action too. |
| D20 | **Calendar store:** one CBOR file per person, encrypted with XChaCha20-Poly1305, with the key in Windows Credential Manager. This avoids building SQLCipher and OpenSSL on Windows. | SQLite with SQLCipher. |
| D21 | **Same-user limit.** Every process runs under your Windows user. The demo's isolation is the API and token boundary, not an operating-system sandbox. | Run agents under a separate Windows account or in an AppContainer. Out of scope unless you want it. |

### 3.4 Runtime and engineering

| # | Proposed default | Alternative |
|---|---|---|
| D22 | **Fixed demo clock per run**, configurable, with all times in UTC, so fixtures and denials are reproducible. | The real clock. |
| D23 | **Owner request form** in the dashboard, pre-filled with the scenario: the other person, duration, window, and a free-text preference that only the owner's own LLM reads. | The LLM drafts the typed request from a sentence and the owner confirms it. |
| D24 | **The dashboard is a demo observer** that shows both owners' views and both agents' traces. In a real deployment each owner would see only their own console. | Two separate owner consoles. |
| D25 | **Models.** Both agents: `claude-opus-5-5` with adaptive thinking, `effort: "medium"` set explicitly, and server-side refusal fallbacks enabled (`fallbacks: "default"`), which re-run a declined request on another Claude model; confirmed at G1. A local run names its Ollama model in the run config; Phase 1 measured `granite4.1:3b`, `qwen3:4b` and `qwen3.5:4b`. | Name other models, or turn fallbacks off. |
| D26 | **Parameters** as listed in section 8.3. | Change any value. |
| D27 | **New dependencies.** The workspace has no third-party crates today. The demo adds them only to new crates; `fabric-core` stays dependency-free. Every version is pinned in `Cargo.lock`. | — |
| D28 | **Tests and CI.** Tests that need Isabelle or an LLM run locally and are skipped in CI. Everything else, including the existing six-host CI, stays green. The demo itself is supported on Windows only. No stand-in prover is used anywhere. | Add Isabelle to CI, which means a large download on six runners. |
| D29 | **Isabelle install** into `.tooling/isabelle/` by a new setup script, like the Rust toolchain. If the Isabelle site publishes no checksum, the script pins the SHA-256 of the first download you approve. | You install Isabelle anywhere and the demo reads its path from config. |

## 4. System overview

```mermaid
flowchart LR
    subgraph AliceSide["Alice"]
        AAgent["Agent runtime<br/>untrusted"]
        AHost["Trusted host<br/>policy gate, gateway, consent signer"]
        ACal["Calendar service<br/>encrypted store"]
    end
    subgraph Shared["Shared on this machine"]
        Isa["Isabelle server<br/>Fabric_Policy session"]
        Relay["Relay<br/>ciphertext mailboxes"]
        Dash["Dashboard<br/>demo observer"]
    end
    subgraph BobSide["Bob"]
        BAgent["Agent runtime<br/>untrusted"]
        BHost["Trusted host<br/>policy gate, gateway, consent signer"]
        BCal["Calendar service<br/>encrypted store"]
    end
    Ollama["Ollama<br/>local LLM, optional"]
    Claude["Claude API<br/>cloud LLM"]

    AAgent -->|"turn input, returns plans"| Claude
    BAgent -->|"turn input, returns plans"| Claude
    AAgent -.->|"or, per run"| Ollama
    AAgent -->|"screen, authorize, send"| AHost
    BAgent -->|"screen, authorize, send"| BHost
    AHost -->|"one theory per check"| Isa
    BHost -->|"one theory per check"| Isa
    AAgent -->|"token + request"| ACal
    BAgent -->|"token + request"| BCal
    AHost <-->|"Noise frames"| Relay
    Relay <-->|"Noise frames"| BHost
```

Every component also sends audit events to the dashboard; those arrows are left out for clarity.

Trust levels:

- **Untrusted:** both agent runtimes (their plans come from LLMs, and peer notes can carry prompt injection), the LLM providers, and the relay.
- **Trusted, one per person:** the trusted host and the calendar service.
- **Trusted, shared:** the Isabelle server (the prover and its kernel) and the dashboard, which observes everything for the demo.

## 5. Components

### 5.1 Calendar service (one per person)

- Owns the encrypted store (D20) and loads the fixtures.
- Agent API, loopback only, token required on every call:
  - `POST /v1/freebusy` runs `ReadFreeBusy(window)`. It returns this person's busy intervals clipped to the window and merged, with no titles, IDs or other fields.
  - `POST /v1/meetings` runs `AddMeeting(meeting)` and writes one event to this person's store only.
  - No endpoint returns event details to an agent. `ReadEventDetails` exists only in the action vocabulary, so an agent can ask and Isabelle can deny it.
- Owner API, used only by the dashboard and authenticated by a per-run owner secret: list events with titles, reset fixtures.
- Before acting, it checks the token: signed by this person's policy gate key, addressed to this service, argument digest matches the request, not expired, nonce unused, and a valid proof-of-possession signature by the agent key named in the token. The nonce is consumed only when the action executes.

Phase 4 settled the details. `fabric-demo calendar --config <file>` runs the service.

| Request | Body | Answer |
|---|---|---|
| `POST /v1/freebusy` | the `read_free_busy` action, as JSON | 200 `{"busy": [{"start_ms", "end_ms"}, ...]}` |
| `POST /v1/meetings` | the `add_meeting` action, as JSON | 201 `{"start_ms", "length_ms"}` |
| `GET /v1/owner/events` | none | 200 `{"person", "events": [...]}`, with titles |
| `POST /v1/owner/reset` | none | 200 `{"events": <count>}`; the fixtures are restored |
| `GET /v1/health` | none | 200 `{"service": "calendar:<person>"}` |

- **Token.** The agent sends `Authorization: FabricToken <lowercase hex of the presentation>` (section 9). The body is exactly the action the token was minted for, so the service checks the token against the action it parses from the body.
- **Refusals.** A refusal is `{"error", "detail"}`:
  - 401 when the token cannot be trusted: `bad_signature` (also for a missing or unreadable token), `expired` or `pop_invalid`.
  - 403 when the token is genuine but does not allow this request: `wrong_audience`, `token_replayed` or `token_argument_mismatch`.
  - 400 `malformed_request` when the body is not an action, and 400 `wrong_endpoint` when the action belongs to the other endpoint (or to none, as `read_event_details` does). Both come before the token check, so they never use up a token.
  - 500 `store_failed` when the meeting cannot be written. The token is released and can be retried.
  - Bodies are limited to 16 KiB.
- **Owner API.** It needs `Authorization: Bearer <owner secret>`, the per-run secret of at least 32 characters. Anything else gets 401 `owner_unauthorized`.
- **Meetings.** A meeting gets the title "Meeting with *peer* (Fabric)", where *peer* is the configured name of the other person. It also records the negotiation, proof ID and token nonce, which only the owner sees.
- **Store file.** The magic `FABCAL01`, a random 24-byte nonce, then the XChaCha20-Poly1305 ciphertext of the deterministic CBOR of the events. The associated data binds the file to its person. The 32-byte key is the generic credential `fabric-demo/<person>/store`, which the service creates on first start. If the key cannot open an existing file, the service refuses to start instead of overwriting the calendar.
- **Audit.** Every agent and owner request appends a JSON line to the service's audit log: the endpoint, the action kind, the outcome, the refusal reason, and for executed actions the token's claims. Records never contain titles or event IDs.
- **Addresses.** The service binds only loopback addresses. Port 0 picks a free port, which the service prints.

### 5.2 Trusted host (one per person)

- **Policy gate.** Accepts `screen(plans)` and `authorize(action)` from its own agent. Builds the context from its own records: the owner's request, the negotiation state reported by the gateway, verified consent records, round counts, the demo clock and the parameters. Renders the context and the actions as HOL terms, runs the check on Isabelle (section 8.4), and returns verdicts; for `authorize` it also mints a token. It records the check theory, Isabelle's messages, timings and the verdict in the audit log.
- **Messaging gateway.** Holds the person's X25519 Noise static key and Ed25519 identity key. It opens a Noise session through the relay only with a valid `Handshake` token, and sends a message only with a valid `Send…` token whose argument digest matches that exact message. It signs outgoing messages, decrypts and verifies incoming ones, tracks the negotiation state, and delivers inbound messages to its agent.
  - **Agent API** (Phase 5; `fabric-demo host --config <file>` runs the gateway for now):
    - `POST /v1/gateway/handshake` takes a `handshake` action and answers `{"link": "calling" | "connected"}`.
    - `POST /v1/gateway/send` takes a send action and answers `{"sent": "<type>"}`.
    - Both need a token, as at the calendar service.
    - `GET /v1/gateway/inbox?after=n&wait_ms=m` returns the deliveries after `n`. `GET /v1/gateway/status` returns the link (`idle`, `calling`, `ringing` or `connected`) and the negotiation. Both need `Authorization: Bearer <agent secret>`, a per-run secret. Inbound messages need no token (D19).
  - **Refusals beyond the token reasons:**
    - 400: `malformed_request`, `wrong_endpoint`, and `unknown_peer` for a handshake with anyone but the configured peer.
    - 409: `no_session`, `already_connected`, `call_pending`, and `out_of_turn`.
    - 502: `relay_failed`.
    - None of them uses up the token.
  - **`out_of_turn`.** When an action runs, the gateway checks the protocol rules again: the active negotiation, not closed, the role, the round, and answers or decisions in order. A message may have arrived since Isabelle checked the action.
  - **Deliveries** are numbered events:
    - `peer_calling`, `session_established` and `peer_key_mismatch`.
    - `message`, carrying the verified message.
    - `protocol_error`, for a frame or message that broke the rules. It changes nothing.
  - **Checks on incoming messages.** The gateway checks each one before it changes the negotiation: the sender and recipient, the active negotiation (the invitee adopts the ID from the first proposal), the round, distinct candidate IDs, and answers that cover the candidates in order.
  - **Negotiation state.** The gateway keeps the policy gate's negotiation record: ID, role, round, candidates and mutual options. Before the invitee learns the ID, the record uses the placeholder `none`. The coordinator's host opens its negotiation with a fresh ID such as `n-3f2a9c1d`.
  - **Audit.** Executed handshakes and sends are logged with the token's claims, and refused ones with the refusal reason, as at the calendar service.
- **Consent signer.** When the owner clicks yes or no in the dashboard, it signs a consent record with the identity key and stores it. When the agent sends its decision, the gateway attaches the signed records.
- **Owner API** (Phase 8), used only by the dashboard and authenticated by the per-run owner secret (`Authorization: Bearer`):
  - `POST /v1/owner/request` files the owner's request: `peer`, `duration_minutes`, `window_start`, `window_end` and `expires_at` (UTC), and `preference`. The negotiation opens then.
  - `POST /v1/owner/consent {"candidate_id", "yes"}` records the owner's answer (section 12).
  - `POST /v1/owner/faults` arms fault switches, and `GET /v1/owner/status` returns what the agent sees.
  - Each of these actions also ends the agent's wait for deliveries, so the agent looks at once rather than within 20 seconds.
- Keys live in Windows Credential Manager. Each person has three keys: `gate` (Ed25519, signs tokens), `identity` (Ed25519, signs messages and consent records) and `noise` (X25519, the Noise static key). Each is a generic credential named `fabric-demo/<person>/<key>`, persisted for this computer only, so it never roams to other machines. The launcher creates missing keys on its first run.
- The launcher writes both people's public keys to the key directory `.local/demo/run/keys.json` (format `fabric-demo-keys-v1`, lowercase hex), and every process reads its peers' keys from there. Readers refuse unknown fields, malformed or weak Ed25519 keys, and person IDs that break the identifier rule.

### 5.3 Agent runtime (one per person, untrusted)

- Holds its LLM backend configuration and credential (an Anthropic API key, or none when it runs on Ollama), and an Ed25519 token-binding key generated at start. The launcher registers the public half with the trusted host.
- Runs the turn loop in section 10.1.
- Phase 6 details (`fabric-demo agent --config <file>`):
  - At start the agent makes its binding key and registers it with `POST /v1/agent/register`, authenticated by the per-run agent secret that only the agent and its host know. Tokens are bound to the key registered last.
  - **Host endpoints.** The agent reads `GET /v1/agent/state` (the owner's request with its preference, the link, the negotiation, and the owner's decisions). It checks actions with `POST /v1/gate/screen` and `POST /v1/gate/authorize`, which answer with verdicts, the proof ID and, for a permit, the token. The host builds the context itself (D14), including the consent facts.
  - **When it takes a turn.** A turn starts when the owner files a request, decides or arms fault switches, when a delivery arrives, or right after a read (D3, at most 3 such turns in a row). The agent's own actions never start a turn.
  - **What a turn does.**
    - The agent screens every action of every plan in one check (D8).
    - It runs the first plan whose actions are all permitted, authorizing and tokening one action at a time.
    - A denial at either point goes back to the planner, with the rule and condition, for at most 3 re-plans (D5).
    - Then the agent gives up until the next outside event, and logs `gave_up`.
  - **Fault switches** (Q24) are hooks in the runtime. Each fires once, on the next turn where it can act, and the agent logs a `fault` record when it does. Phase 8 changed one thing: a switch that cannot act yet stays armed instead of being used up. It rides along on later turns but starts none of its own, so a switch with nothing to act on does not make the agent plan again and again.
    - `over_broad_read` widens every read 30 days past the window and adds a `read_event_details` plan.
    - `too_many_candidates` pads the next proposal to 5 candidates. Only the coordinator proposes.
    - `write_without_consent` puts an `add_meeting` first in the next chosen plan that has actions.
    - `replay_token` sends the next executed request again with its used token.
    - `token_for_different_slot` presents the next calendar action's token for that action moved by 15 minutes, then gets a fresh token.
  - **Audit.** The agent logs each turn's input, plans, planner usage, screening verdicts, authorizations and results.
- Sees: its owner's request, the free/busy results it fetched with tokens, inbound messages (the peer's candidates, answers, notes and decisions), its own past actions, and denial reasons.
- Never sees: the calendar store, event titles, any key other than its own token-binding key, or the other person's data beyond the messages it receives.

### 5.4 Relay (shared)

- FIFO mailboxes keyed by random 128-bit IDs, one inbox per person, set in the per-run configuration. `POST /v1/mailboxes/{id}` appends a frame; `GET /v1/mailboxes/{id}?after=n` long-polls.
- Keeps frames in memory for 10 minutes and logs only mailbox ID, size and time.
- Phase 5 details (`fabric-demo relay --config <file>`):
  - A post answers `{"seq": n}`.
  - A poll answers `{"frames": [{"seq", "at_ms", "frame": "<hex>"}]}`, oldest first. With `wait_ms=m` it waits up to `m` milliseconds for a frame, at most 25 seconds.
  - Reading removes nothing; each reader keeps its own position.
  - Unknown mailboxes get 404. Frames are at most about 64 KiB.
  - Each audit line holds exactly `at_ms`, `component`, `mailbox`, `seq` and `bytes`.

### 5.5 Isabelle server (shared)

- Isabelle2025-2 for Windows, the current release (January 2026). On Windows, Isabelle's command-line tools run through its bundled Cygwin.
- `scripts/setup-demo.ps1` installs it into `.tooling/isabelle/` (D29), and `scripts/isabelle.ps1` runs its tools. Both set `USER_HOME`, so Isabelle's per-user directory, including session heaps, stays in `.tooling/isabelle/home` instead of your Windows profile.
- Sessions start with `headless_consolidate_delay=0.1`; the 2.0 s default adds about a second to every check (G1). On Windows, `session_stop` always reports return code 130, so the launcher treats that as normal and always sends `shutdown` afterwards; otherwise the server keeps running.
- The launcher starts `isabelle server -n fabric-demo`, reads the address and password from the server's first output line, and passes them privately to the two policy gates.
- Phase 6 details. Each host's gate connects to that server with `Gate::connect` and opens its own session. Stopping a gate stops only its session; the launcher alone shuts the server down.
- One session, `Fabric_Policy`, built from `isabelle/Fabric_Policy` with HOL as its parent. The gates use the server's `session_start`, `use_theories` and `purge_theories` commands over its TCP protocol.

### 5.6 Dashboard (shared demo observer)

See section 13. Phase 8 details (`fabric-demo dashboard --config <file>`):

- **Audit intake.** Every component appends its audit records to a JSON-lines file in the run directory. The dashboard tails those files, so no component needs the dashboard's address or a secret for it. It pushes each record to the page over Server-Sent Events, and keeps the last 5,000 for pages that connect later.
  - It reads whole lines only, so a record still being written is never split.
  - It leaves out its own polling of the owners' calendars. The calendar service still audits those reads (section 5.1).
- **Endpoints**, on loopback only:

  | Request | Answer |
  |---|---|
  | `GET /` | the page, which reads its key from the URL fragment `#key=…` |
  | `GET /v1/overview` | both owners' status and calendars, and the relay frames' metadata |
  | `GET /v1/stream?after=n` | the audit records after `n`, as Server-Sent Events |
  | `GET /v1/proof?id=<proof ID>` | the check report behind that proof ID |
  | `POST /v1/people/{name}/{request, consent, faults, reset}` | the owner action, passed to that person's host or calendar service with the owner's secret |

- **Access.** Every API call needs the per-run dashboard key, in `x-dashboard-key` or the `key` query parameter (Server-Sent Events cannot send headers). The `Host` header must name the dashboard's own loopback address, which stops DNS-rebinding pages.

### 5.7 Launcher

- `fabric-demo up [--alice <backend>] [--bob <backend>] [--start <UTC time>] [--repository <dir>]`:
  - Checks that it runs from the repository root.
  - Creates both people's keys in Credential Manager on the first run.
  - Writes the per-run configuration and secrets to `.local/demo/run/` (git-ignored).
  - Starts the Isabelle server, the relay, both calendar services, both trusted hosts, the dashboard and both agents. Each runs as a `fabric-demo` child process on a free loopback port. Its first output line names its address; the rest of its output goes to `<name>.log` in the run directory.
  - Prints the dashboard URL with its key.
- A backend is `claude` (the default for both), `ollama:<model>` or `scripted` (tests only).
- Every run starts from the fixtures, so the scenario is the same each time (D22). `up` deletes the previous run's calendar stores.
- `fabric-demo down` asks the running launcher to stop, through a control endpoint whose address and per-run key are in `run.json`. The launcher stops the agents first, then the services, then Isabelle, and removes `run.json`. Ctrl+C does the same.
- `fabric-demo reset-keys` deletes both people's keys and store keys from Credential Manager, and their calendar files.
- `scripts/demo.ps1 [-Command up|down|reset-keys] [-Alice <backend>] [-Bob <backend>]` runs these from the repository root. For `up` it builds the release binary first.

## 6. Data model

Times are UTC milliseconds since the Unix epoch, and durations are milliseconds too, as in the policy theory. (`fabric-core` keeps durations in seconds; the demo does not use its types.) Intervals are half-open, `[start, end)`. The JSON the LLM reads and writes uses ISO 8601 UTC strings; the runtime converts them.

| Record | Fields | Notes |
|---|---|---|
| Calendar event | ID, start, duration, title, busy flag | Stored only in the calendar service. Events that are not busy do not block time. |
| Free/busy result | busy intervals | Clipped to the requested window and merged. |
| Owner request | peer (configured key IDs), duration, window `[opened_at, opened_at + length)`, negotiation expiry, preference text | The typed fields go into the policy context. The preference text goes only to the owner's own LLM. |
| Candidate | random ID, start, duration | |
| `Propose` message | negotiation ID, round, candidates, note | Sent by the coordinator. |
| `Respond` message | negotiation ID, round, (candidate ID, available) pairs, note | |
| `Decide` message | negotiation ID, round, signed consent records, note | One record per mutual option. |
| `Close` message | negotiation ID, reason (`agreed`, `no_match` or `withdrawn`), note | |
| Consent record | signer, negotiation ID, meeting (start, duration, both participants), yes or no, issued at, expires at | COSE_Sign1 by the owner's identity key, made only by the consent signer. |
| Meeting | start, duration, participants | Written with the title "Meeting with *peer* (Fabric)". |

Two people need no extra message to find mutual options: Alice knows which of her candidates Bob marked available, and Bob knows which he marked.

## 7. Action vocabulary

| Action | Arguments | Executed by | Rules (section 8.2) |
|---|---|---|---|
| `ReadFreeBusy` | window | own calendar service | `read_window` |
| `ReadEventDetails` | window | nobody | `view_details_never` (always denied) |
| `Handshake` | peer key ID | own gateway | `handshake_target` |
| `SendPropose` | negotiation, round, candidates, note | own gateway | disclosure budget, `note`, `well_formed` |
| `SendRespond` | negotiation, round, answers, note | own gateway | disclosure budget, `note`, `well_formed` |
| `SendDecide` | negotiation, round, decisions, note | own gateway | `consent_lifetime`, `note`, `well_formed` |
| `SendClose` | negotiation, reason, note | own gateway | `consent_lifetime`, `note`, `well_formed` |
| `AddMeeting` | meeting | own calendar service | `consent` |

## 8. Semantic layer: Isabelle/HOL

### 8.1 Theory layout

- `isabelle/Fabric_Policy/ROOT` declares `session Fabric_Policy = HOL` with the theory `Fabric_Policy`.
- `Fabric_Policy.thy` defines datatypes for intervals, candidates, actions, requests, negotiation state, consent facts, parameters and the context; one predicate per rule; `applies r a`, true when rule `r` governs action `a`; `holds r ctx a`, true when `r` does not apply or is satisfied; and `permitted ctx a`, true when every rule holds. It also contains the sanity lemmas (D10).
- This theory is the contract. You review it at gate G2 before anything else is built on it.

### 8.2 Rules

`P` is the parameter set, `req` the owner's request, `now` the demo clock.

| Rule | Applies to | Holds when | Group |
|---|---|---|---|
| `view_details_never` | `ReadEventDetails` | never | calendar view |
| `read_window` | `ReadFreeBusy w` | `w` lies inside `req`'s window | calendar view |
| `cand_count` | `SendPropose` | at least 1 and at most `P.max_candidates` candidates | disclosure budget |
| `answer_count` | `SendRespond` | at most `P.max_candidates` answers | disclosure budget |
| `round_limit` | `SendPropose`, `SendRespond` | round is at most `P.max_rounds` | disclosure budget |
| `horizon` | `SendPropose`, `SendRespond` | `req`'s window is at most `P.horizon` long, and every candidate lies inside it and starts after `now` | disclosure budget |
| `grid` | `SendPropose` | every candidate starts on the `P.grid` and lasts exactly `req`'s duration | disclosure budget |
| `consent_lifetime` | every `Send…` | the negotiation expires at most `P.consent_ttl` after it opened, and `now` is before expiry | disclosure budget |
| `note` | every `Send…` with a note | printable ASCII (D13), at most `P.note_max` characters, no `0`–`9` | note |
| `handshake_target` | `Handshake k` | `k` is the configured key of `req`'s peer | handshake (Q21) |
| `consent` | `AddMeeting m` | the context holds a valid, unexpired yes for exactly `m` signed by the owner, and one signed by the peer | consent (Q22) |
| `well_formed` | every `Send…` | refers to the active negotiation and round; answers cover exactly the received candidates, in order; decisions refer to mutual options | well-formedness (D11) |

### 8.3 Parameters (D26)

| Parameter | Value | Source |
|---|---|---|
| `max_candidates` | 3 | calendar.md §23.7 default |
| `max_rounds` | 3 | calendar.md §23.7 default |
| `horizon` | 14 days | calendar.md §23.7 default |
| `grid` | 15 minutes | calendar.md §23.7 default |
| `consent_ttl` | 24 hours | calendar.md §23.7 default |
| `note_max` | 200 characters | your example value |
| Token lifetime | 60 seconds | proposed |
| Proof-of-possession window | from 10 seconds old to 2 seconds ahead | chosen in Phase 3 |
| Token size limit | 4 KiB | chosen in Phase 3 |
| Isabelle check timeout | 30 seconds | proposed |
| Re-plan attempts per turn | 3 | proposed (D5) |
| Plans per turn | 2 to 3 | your answer (Q23) |

### 8.4 One check, step by step

1. The gate renders the context and each action as HOL definitions in a generated theory, `Check_<n>`, that imports `Fabric_Policy`.
2. For each rule and each action the theory states two lemmas, `holds r ctx a` and `¬ holds r ctx a`, each proven with the simplifier over the policy definitions. For concrete terms only one of the two can succeed.
3. It adds `value` commands for the quantities that denial messages quote, such as the number of candidates.
4. The gate submits the theory with `use_theories`, reads which lemmas failed from the reported errors and their line positions, computes the verdict (D6), and calls `purge_theories`.
5. The proof ID is the SHA-256 of the check theory text, the hash of `Fabric_Policy.thy`, and the Isabelle version. It goes into the token and the audit log.

Spike T1.1 confirmed that a failed proof does not stop Isabelle from checking the later lemmas of the same theory, so each check is a single pass. Measured on this machine: about 1.0 s for one action and 1.5 s for a 9-action screening batch (`spikes/phase1/RESULTS.md`).

An abbreviated check theory. The renderer test's `crates/fabric-policy/tests/fixtures/Check_000001.thy` shows the complete form the gate generates:

```isabelle
theory Check_000042
  imports "Fabric_Policy.Fabric_Policy"
begin

definition ctx :: gate_context where "ctx = ..."   (* rendered from the gate's records *)
definition a0 :: action where "a0 = SendPropose ..."   (* five candidates *)

lemma a0_cand_count_holds: "holds Cand_count ctx a0" by (simp add: a0_def ctx_def policy_simps)
lemma a0_cand_count_fails: "\<not> holds Cand_count ctx a0" by (simp add: a0_def ctx_def policy_simps)
(* ... two lemmas for each of the other rules ... *)
value "payload_count a0"
value "length (note_of a0)"

end
```

Here the first lemma fails, the second is proven, and `value` returns 5, so the gate reports `cand_count: 5 candidates; 1 to 3 allowed`.

### 8.5 What Isabelle does not check

- Signatures, digests and token cryptography are checked in Rust. Isabelle reasons over their results, which the gate supplies as facts.
- The Rust code that renders facts into HOL is not verified. A rendering bug could make Isabelle prove the wrong statement, so T2.4 tests the renderer against hand-written theories.

## 9. Access tokens

A token is a CBOR Web Token (CWT, RFC 8392) in a COSE_Sign1 envelope (RFC 9052), signed with the person's policy-gate Ed25519 key. The payload uses deterministic CBOR (RFC 8949 §4.2). `crates/fabric-token` implements this section.

| Claim | Key | Meaning |
|---|---|---|
| `iss` | 1 | the policy gate that minted it: `gate:<person>` |
| `aud` | 3 | the one service that may accept it: `calendar:<person>` or `gateway:<person>` |
| `exp`, `iat` | 4, 6 | expiry and issue time in whole seconds; expiry is 60 seconds after issue |
| `cti` | 7 | a random 128-bit nonce; the token works once |
| `cnf` | 8 | the agent's token-binding public key (proof of possession, RFC 8747), as `{1: COSE_Key}` with `kty` OKP, `crv` Ed25519 and `x` |
| private: action | -65537 | the action kind, for example `add_meeting` |
| private: args | -65538 | SHA-256 of the deterministic CBOR of the action: a map of `kind` and the exact arguments, under the same names as in the action's JSON |
| private: negotiation | -65539 | the negotiation ID, when there is one |
| private: proof | -65540 | the Isabelle proof ID (section 8.4), as 32 bytes |

The private claims use keys below -65536, the range RFC 8392 reserves for private use.

**Envelope.** `18([h'a10132', {}, payload, signature])`: the protected header is exactly `{1: -19}`, the fully specified Ed25519 algorithm of RFC 9864 (it deprecates the older `-8`, EdDSA). The unprotected header is empty, and the signature covers no external data. A test checks the bytes against a hand-assembled token, and another reads tokens with coset, an independent COSE library.

**Deterministic CBOR.** Integers and lengths take their shortest form, lengths are definite, and map keys are sorted by the bytes of their encodings. The decoder accepts no other encoding, so every value has exactly one byte form for signatures and digests. The profile has no floating-point numbers, bignums, or simple values other than `false`, `true` and `null`. Test vectors come from RFC 8949 appendix A and RFC 8392 appendix A.1.

**Minting.** The gate mints only from an authorize check (D2) that covered exactly one action and permitted it. It takes `aud` from the action kind: `ReadFreeBusy` and `AddMeeting` go to the calendar service, `Handshake` and `Send…` to the gateway. Nothing executes `ReadEventDetails`, so no token can name it.

**Presentation.** With each request, the agent sends the deterministic CBOR `[token, timestamp_ms, signature]`. The signature is made with the token-binding key over `["fabric-pop-v1", cti, SHA-256(request body), timestamp_ms]`. A service accepts it from 10 seconds old to 2 seconds ahead of its own clock. Tokens longer than 4 KiB are refused unread.

**Checks.** The service runs these checks in order and names the first that fails:

| Reason | Fails when |
|---|---|
| `bad_signature` | the token is malformed, not deterministic CBOR, has another header or algorithm, is not signed by this person's gate key, or names another issuer |
| `wrong_audience` | `aud` is not this service |
| `expired` | the service's clock is at or past `exp` |
| `token_replayed` | the nonce was used, or another request is using it |
| `pop_invalid` | the proof of possession is too old, dated ahead, or not signed with the `cnf` key over this body and timestamp |
| `token_argument_mismatch` | the requested action's kind, negotiation or argument digest differs from the token's |

A token that passes is reserved for its request. The service commits the nonce once the action has executed (section 5.1). If the action cannot execute, the service drops the reservation and the nonce is released. A refused attempt never uses up a token, so a misdirected request cannot burn it. The service keeps each nonce until its token expires. It also tracks the latest time any check has used, so a clock that runs behind cannot revive a forgotten nonce.

## 10. Agents and LLM backends

### 10.1 Turn loop

1. A turn starts on an outside event: the owner's request, a result the agent asked for, an inbound message, an owner's click, or a denial.
2. The runtime builds the turn input: the agent's role and the rules in plain words, the owner's request, the negotiation so far, the results it fetched, recent denials with their reasons, the action vocabulary, and an instruction to return 2–3 alternative plans.
3. One LLM request returns the plans as JSON matching the plan schema. Invalid JSON counts as a failed attempt.
4. The runtime sends all plans to its gate for screening. The first plan, in the LLM's order, whose actions are all permitted runs. If none is permitted, the runtime re-plans with every denial reason, at most 3 times (D5).
5. For each action of the chosen plan, the runtime asks for authorization (the execution-time check and token), calls the service, and records the result. A denial at this point stops the plan and starts a re-plan with the reason.

The plan schema shapes the output but enforces no limits; limits are Isabelle's job. Claude's structured outputs do not support numeric or string-length constraints in any case.

The schema lists properties in a fixed order: `kind` first in every action, and `label` and `rationale` before `actions` in every plan. Constrained decoding writes properties in schema order, and in Phase 1 an alphabetically sorted schema made Claude emit placeholder plans (G1). A test checks the order. Example of a valid plan list:

```json
{
  "plans": [
    {
      "label": "Afternoons early in the week",
      "rationale": "Tuesday and Wednesday afternoons are free.",
      "actions": [
        {"kind": "handshake", "peer": "bob"},
        {"kind": "send_propose", "round": 1,
         "candidates": [{"start": "2027-01-12T14:00:00Z", "minutes": 30}],
         "note": "afternoons suit me"}
      ]
    }
  ]
}
```

### 10.2 Claude backend (both agents by default)

- Raw HTTPS to `POST https://api.anthropic.com/v1/messages`, since there is no official Anthropic SDK for Rust. Headers: `x-api-key`, `anthropic-version: 2023-06-01`, `content-type: application/json`, `anthropic-workspace-id` (Q26), and `anthropic-beta: server-side-fallback-2026-07-01` for fallbacks.
- Body: `"model": "claude-opus-5-5"`, `"max_tokens": 16000`, `"thinking": {"type": "adaptive"}`, `"output_config": {"effort": "medium", "format": {"type": "json_schema", "schema": <plan schema>}}`, `"fallbacks": "default"`, the system prompt, and the turn messages.
- The runtime checks `stop_reason` before reading the content (`refusal` and `max_tokens` fail the attempt), and records which model served the turn and the token usage in the audit log.
- Credential: `ANTHROPIC_API_KEY` and `ANTHROPIC_WORKSPACE_ID`, read from the git-ignored `.env` file at the repository root (Q25, Q26). The key is never written to the repository, logs or audit trail.
- Cost: Phase 1 measured 3,000–3,900 input and 400–1,000 output tokens per turn, about 3 cents. With both agents on Claude, a happy-path run is roughly 8–10 turns, about 30 cents.
- Phase 6:
  - The backend sends the turn input as pretty-printed JSON in one user message.
  - HTTPS is Windows' Schannel, through reqwest's `native-tls` feature, on Windows only. rustls would need aws-lc-rs or ring, both C and assembly builds, which risk the Windows ARM64 CI runner. Schannel is part of Windows.
  - A first-turn smoke test returned three valid plans in 5.7 s, with 3,420 input and 435 output tokens.
- **Plans to actions.** The runtime fills in the active negotiation ID and gives each proposed candidate the ID `c<round>-<position>`. A plan with no actions means "wait". A plan whose time is unreadable is skipped, and the error goes back to the planner.

### 10.3 Ollama backend (optional, per run)

- `POST http://localhost:11434/api/chat` with `"stream": false`, `"format": <plan schema>` and temperature 0. The schema is also included in the prompt, as Ollama's documentation recommends.
- Ollama 0.40.5 is installed. Phase 1 measured `granite4.1:3b`, `qwen3:4b` and `qwen3.5:4b`: valid JSON every time, but the correct plan in only 2–4 of 10 turns, at 20–136 s per turn. A run names its model in the run config.

### 10.4 Scripted planner (tests only)

Deterministic plans read from fixture scripts, for automated tests. It is never used in the live demo.

Phase 6 built it in two forms:

- **Fixed:** plan sets returned in order, for the runtime's unit tests.
- **Rules:** a fixed procedure that follows the section 14 scenario, for the end-to-end tests:
  - Read the window, then call.
  - Propose the first three free half hours in weekday working hours.
  - Answer from free/busy.
  - Send the owner's decisions as given, add the meeting both owners accepted, and, for the coordinator, close the negotiation as agreed.

### 10.5 What leaves the machine

- Each agent on Claude sends Anthropic its system prompt, its owner's request and preference text, the free/busy windows it read, the other person's candidates, notes and decisions, and its denial reasons.
- An agent on Ollama sends nothing off the machine. The relay, Isabelle and both calendar services are always local.

## 11. Negotiation protocol

```mermaid
sequenceDiagram
    actor OA as Alice
    participant AA as "Alice agent"
    participant AH as "Alice host"
    participant AC as "Alice calendar"
    participant I as "Isabelle"
    participant R as "Relay"
    participant BH as "Bob host"
    participant BA as "Bob agent"
    actor OB as Bob

    OA->>AH: Request with Bob, 30 min, next 7 days
    OB->>BH: Request with Alice, 30 min, next 7 days
    AA->>AH: Screen 2-3 alternative plans
    AH->>I: One theory with every action of every plan
    I-->>AH: Per-action verdicts
    AA->>AH: Authorize ReadFreeBusy
    AH->>I: Prove the rules for this action
    AH-->>AA: Single-use token
    AA->>AC: ReadFreeBusy with token
    AC-->>AA: Busy intervals, no titles
    Note over AA,AH: Every later action repeats screen, authorize and token
    AA->>AH: Handshake Bob, then SendPropose with 3 candidates
    AH->>R: Noise IK handshake and encrypted proposal
    R->>BH: Ciphertext
    BH-->>BA: Alice is calling, proposal received
    BA->>BH: Handshake Alice, ReadFreeBusy, SendRespond
    BH->>R: Encrypted answers
    R->>AH: Ciphertext
    AH-->>OA: Mutual options, yes or no
    BH-->>OB: Mutual options, yes or no
    OA->>AH: Yes to one option
    OB->>BH: Yes to the same option
    AA->>AH: SendDecide with signed consent
    BA->>BH: SendDecide with signed consent
    AA->>AH: Authorize AddMeeting
    AH->>I: Prove consent, both owners said yes
    AH-->>AA: Single-use token
    AA->>AC: AddMeeting with token
```

Bob's side mirrors Alice's: it reads free/busy and adds the meeting through Bob's own calendar service with Bob's own tokens.

Handshake details: Alice's agent obtains a `Handshake(Bob)` token, and her gateway sends the first IK message to Bob's inbox. Bob's gateway authenticates the caller's static key, tells its agent that Alice is calling, and continues only after Bob's agent obtains its own `Handshake(Alice)` token. If the caller's key is not the configured peer's, the handshake ends with `peer_key_mismatch`.

Messages travel as COSE_Sign1-signed CBOR inside Noise transport frames, in order, through FIFO mailboxes.

Phase 5 settled the wire format (`crates/fabric-wire`):

- **Frames** are deterministic CBOR and carry no names, keys or times:
  - `[1, session, noise]` is the caller's first IK message.
  - `[2, session, noise]` is the answer.
  - `[3, session, nonce, noise]` is a transport message.

  The session is 8 random bytes that the caller picks.
- **Noise.** `Noise_IK_25519_ChaChaPoly_BLAKE2s` with the prologue `fabric-demo-noise-v1` and empty handshake payloads. Transport messages use explicit nonces (Noise's stateless transport mode). A receiver accepts only nonces above the last one, so a replayed or reordered frame is refused, and a frame that never arrived does not break the session.
- **Messages** are `{from, to, negotiation, sent_at_ms, body}`. The body is `propose`, `respond`, `decide` (with a list of signed consent records, filled in Phase 7) or `close`. Each is signed as a COSE_Sign1 with the external data `fabric-message-v1`. Tokens sign no external data, and consent records will use `fabric-consent-v1`, so a signature for one use can never pass for another.
- **Simultaneous calls.** If both agents call at once, the side with the lower Noise public key keeps its call, and the other gateway answers it, because its agent has already asked for a session.
- **Stale and replayed calls.** A call that goes unanswered for 30 seconds may be replaced. A replayed call, with a session ID already seen, is ignored. A new call from the peer replaces an open session, so the agent must answer again.

## 12. Consent

- When mutual options exist, the dashboard shows each owner the options with Yes and No buttons and says what yes allows, for example: "Yes lets your agent add this exact meeting: Tue 12 Jan 2027, 14:00–14:30 UTC with Bob. This consent expires at 08:00 UTC tomorrow."
- A click makes the consent signer sign a record (section 6). The agent learns the decision but never holds the signature key. `SendDecide` carries the signed records, which the gateway attaches.
- The peer's gateway verifies each signature against the configured identity key. The peer's gate then holds both owners' records as facts, and the `consent` rule can be proven for `AddMeeting`.
- Phase 7 details (`crates/fabric-wire/src/consent.rs`):
  - **Record.** `{signer, negotiation, start_ms, length_ms, participants, yes, issued_at_ms, expires_ms}`, with both participants in alphabetical order. It is signed as a COSE_Sign1 with the external data `fabric-consent-v1`. A consent expires with the owner's own request.
  - **Recording.** `POST /v1/owner/consent {"candidate_id", "yes"}` signs a record only for a mutual option of the current round and before this round's decisions were sent. A new answer replaces the old one until then.
  - **Sending.** The gateway attaches the owner's records in decision order. A decision that differs from the owner's record is refused with `no_consent_record`; Isabelle's `well_formed` also checks this.
  - **Receiving.** The peer's gateway accepts a decision only if it carries one record per decision and each record:
    - verifies with the peer's identity key;
    - names the peer and the active negotiation;
    - matches that candidate's start and length;
    - says the same as the decision;
    - lists both participants.

    Otherwise the decision is a `protocol_error`.
  - **Facts.** The policy gate's context holds the owner's records and the peer's verified records. Records are cleared when a new round starts.

## 13. Dashboard

- **Run bar:** start the scenario, reset fixtures, the demo clock, and the backends in use.
- **One column per person (Alice and Bob):**
  - Owner: the request form; the calendar with titles (the owner's view); yes/no cards for mutual options.
  - Agent: the current turn; the alternatives with a verdict per action (permitted, denied with rule and condition, or undecided); "View proof", which opens the check theory with Isabelle's messages and timing; executed actions with their decoded tokens; and the free/busy data the agent received.
  - Fault switches (Q24): over-broad read; too many candidates; write without consent; replay a used token; use a token for a different slot.
- **Relay:** frames with mailbox, size and time. Ciphertext only.
- **Isabelle:** checks run, verdicts and times.
- **Audit log:** every event, filterable by person and kind.

The page is plain HTML, CSS and JS with no build step, updated over Server-Sent Events, and makes no external network requests.

Phase 8 details:

- Values from the services are always inserted as text, never as HTML, because peer notes are untrusted.
- A section redraws only when its data changes. A form being filled in, an open list or a button under the pointer survive the 1.5-second poll.
- **Start scenario** files every request not yet filed, with what each form shows. A refusal shows under the form.
- Each fault switch shows whether it is armed or has fired, and with what result. Hovering over a switch says what it does.
- The proof dialog shows the Isabelle version, the time, the proof ID, each rule's outcome per action, and the check theory.

## 14. Scenario and expected runs

- Demo clock: Monday 11 January 2027, 08:00 UTC (configurable, D22). The clock starts there when the run starts and then advances in real time, so tokens still expire. The launcher records the start and the wall-clock instant of the run in the per-run configuration, and every process derives the demo time from them, so all processes agree.
- Requests: Alice and Bob each ask for 30 minutes with the other within 7 days; the negotiation expires after 24 hours.
- Fixtures: chosen so that Alice's free/busy suggests three afternoon candidates, Bob is busy for one of them, and round 1 yields two mutual options. Exact times live in `demo/fixtures/` and are reviewed at gate G3.
  - The Phase 4 draft: in weekday working hours (09:00–17:00 UTC) within the window, Alice is free for exactly three half hours: Tuesday 14:00, Wednesday 15:00 and Thursday 14:30.
  - Bob is busy on Wednesday 15:00–16:00, so the mutual options are Tuesday 14:00 and Thursday 14:30.
  - Both calendars include events outside working hours and events that are not busy. Some titles are sensitive ("Dentist appointment", "Doctor appointment"), so a leak would be obvious.
  - A test checks these properties.

Expected happy path:

1. Both owners submit the pre-filled requests.
2. Alice's agent reads her free/busy, then plans `Handshake(Bob)` and `SendPropose` with three candidates and a note.
3. Bob's agent gets the handshake, obtains `Handshake(Alice)`, reads his free/busy around the candidates, and sends `SendRespond`.
4. Both owners see two mutual options. In the rehearsal script Bob says no to the first, and both say yes to the second.
5. Both agents send `SendDecide`, add the meeting with `AddMeeting`, and send `SendClose(agreed)`.
6. Both calendars show the meeting. The audit log shows a proof and a token for every action, and the relay log shows only ciphertext.

Denial runs:

| Switch | What it does | Expected result |
|---|---|---|
| Over-broad read | The next turn's plans get a `ReadFreeBusy` window that runs past the request's window, and a `ReadEventDetails` | Screening denies them (`read_window`, `view_details_never`); another alternative runs, or the agent re-plans |
| Too many candidates | The next proposal gets 5 candidates | Screening denies every plan (`cand_count: 5 > 3`); the agent re-plans with 3 |
| Write without consent | Puts `AddMeeting` first in the next plan that acts, before both owners agree | The execution-time check denies it (`consent`); the agent re-plans |
| Replay a token | Sends the next executed request again with its used token | The service rejects it (`token_replayed`); Isabelle is not involved |
| Token for a different slot | Moves the next calendar action by 15 minutes after its token was issued | The calendar service rejects it (`token_argument_mismatch`); the agent gets a fresh token and continues |

## 15. Security model and limits

What the demo enforces:

- Agents receive free/busy only for windows a token allows, and never event details.
- Every agent action is proven permissible by Isabelle before it can run, and runs only with a single-use token bound to its exact arguments and to the agent's key.
- Agents cannot forge consent: consent records are signed with keys agents never hold.
- The relay sees only ciphertext and traffic metadata.

What it does not protect (the dashboard says so too):

- All processes run under one Windows user, so there is no OS sandbox (D21).
- Isabelle checks the policy over facts the trusted host supplies. The host, its signature checks and its HOL rendering are trusted, not verified.
- Nothing checks that an agent's availability answers match its owner's calendar.
- With the default backends, both owners' free/busy windows, and everything the agents receive, go to Anthropic.
- Peer notes can carry prompt injection. They can steer a plan but cannot get an action past Isabelle.
- The cryptography has not been independently reviewed and is not production code.

## 16. Platform, setup and dependencies

- Windows 11 on this machine. Rust 1.97.1 through `scripts/setup.ps1`, which downloads rustup and the toolchain into `.tooling/`.
- Isabelle2025-2 for Windows (`Isabelle2025-2.exe`, a self-extracting archive) into `.tooling/isabelle/` (D29). The Isabelle site notes that the app has no developer signature, so Windows may warn on first run, and that Windows Defender can block external provers; this demo uses none. The site publishes no checksum, so `scripts/setup-demo.ps1` pins the size and SHA-256 of the archive approved on 2026-10-18. It downloads over HTTPS from the TUM mirror, because the site's own link redirects to plain HTTP.
- Ollama for Windows and the three Phase 1 models (about 8 GB), for optional local runs.
- Your Anthropic API key and workspace ID in a git-ignored `.env` file at the repository root (Q25, Q26).
- New crates, with versions pinned when added: an async runtime (tokio), an HTTP server and client (axum, and reqwest with rustls), serde, CBOR (ciborium), COSE (coset), Ed25519 (ed25519-dalek), Noise (snow), XChaCha20-Poly1305 (chacha20poly1305), SHA-256 (sha2), Windows Credential Manager (keyring), and tracing.
- Phase 3 choices:
  - `ed25519-dalek` 2.2 and `x25519-dalek` 2.0, not the new 3.0 releases. The 2.x releases share `curve25519-dalek` 4 and `sha2` 0.10 with `snow` 0.10 and `fabric-policy`.
  - `ciborium` 0.2 parses CBOR. `fabric-token` has its own deterministic encoder of about 75 lines. ciborium writes map entries in the order given, and its optional canonical ordering is the older length-first rule (RFC 8949 §4.2.3), not the §4.2.1 bytewise rule.
  - COSE_Sign1 is built directly on that encoder, since the header is fixed. `coset` 0.4 is a test-only dependency that checks the tokens read as standard CWTs.
  - Credential Manager access uses `keyring-core` 1 and `windows-native-keyring-store` 1, on Windows only. The `keyring` crate's version 4 split the platform stores into these separate crates.
  - Also added: `getrandom` 0.3 for keys and nonces, and `zeroize` to wipe secrets.
- Phase 4 choices:
  - `tokio` 1.53 and `axum` 0.8, with only axum's HTTP/1, JSON and tokio features.
  - `chacha20poly1305` 0.10, the release `snow` 0.10 uses.
  - `reqwest` 0.13 for the calendar client, without TLS for now; loopback needs none.
  - `tower` 0.5 for tests only.
  - Settled in Phase 6: reqwest 0.13's default TLS is rustls on the aws-lc-rs provider, and the Phase 1 spike used rustls on ring. The Claude backend uses Schannel instead (section 10.2).
- Phase 6 choices:
  - The app's `serde_json` has the `preserve_order` feature, so the plan schema keeps its property order (G1). A test checks the order.
  - The agent's calls to its host and calendar go through a `World` trait. The runtime's unit tests replace it with replayed answers, so CI tests the control flow without Isabelle; the verdicts there are test inputs, not policy. The live tests use Isabelle.
- Phase 5 choices:
  - `snow` 0.10 with only the BLAKE2s, ChaCha20-Poly1305, Curve25519 and getrandom resolvers. It runs without its `std` feature, because that feature would also pull in `ring`.
  - The COSE_Sign1 code moved from the token module into `fabric_token::cose`, unchanged, so messages and consent records share it. The token layout test still passes byte for byte.

Proposed layout:

```text
crates/fabric-core/        unchanged, still dependency-free
crates/fabric-policy/      actions, context, HOL rendering, Isabelle client, verdicts
crates/fabric-token/       deterministic CBOR, CWT/COSE tokens, proof of possession, replay cache, keys
crates/fabric-wire/        protocol messages, signatures, Noise sessions
apps/fabric-demo/          one binary: up, down, relay, host, calendar, agent, dashboard (Phase 4 added calendar)
isabelle/Fabric_Policy/    ROOT, Fabric_Policy.thy
demo/                      config, fixtures, prompts, dashboard assets
scripts/                   setup-demo.ps1 (Isabelle), demo.ps1 (start)
```

Per-run state, public keys, logs and audit files go to `.local/demo/`, which is already git-ignored.

## 17. Acceptance criteria

| # | Criterion |
|---|---|
| A1 | `fabric-demo up` starts every component on this machine and prints the dashboard URL; `fabric-demo down` stops them. |
| A2 | The happy path completes with Claude for both agents. The meeting appears in both calendars, and every executed action in the audit log has a permit theorem and a token. A run with Ollama for one agent starts and is checked the same way; its plan quality is not a criterion (Q27). |
| A3 | Each denial switch produces the expected rejection: rule name and condition for Isabelle denials, the token error for misuse. The run then recovers or stops cleanly. |
| A4 | No agent-facing response contains an event title or event ID. An automated test uses fixtures with marker titles. |
| A5 | The relay capture contains no plaintext candidate times, notes or names. An automated test searches the frames for fixture markers. |
| A6 | Every token is single-use and bound to its arguments and agent key (automated tests). |
| A7 | The policy test corpus has at least one permit and one deny case per rule, and live Isabelle decides each as expected (local test). |
| A8 | The sanity lemmas (if D10 is approved) are proven when the session builds. |
| A9 | `scripts/verify.ps1` passes: formatting, check, Clippy with warnings denied, tests, and docs. |
| A10 | Isabelle check times are recorded per check. Target: a screening pass within 5 seconds on this machine, to be confirmed by spike T1.1. |

## 18. Risks

| # | Risk | Mitigation |
|---|---|---|
| R1 | Driving Isabelle on Windows through its bundled Cygwin from Rust: process control, and Windows versus Cygwin paths for `master_dir`. | Resolved in Phase 1: it works, with `/cygdrive/c/...` paths and a `shutdown` after every session. |
| R2 | Checks are too slow for a smooth demo. | Resolved in Phase 1: about 1 s per action and 1.5 s per screening batch. |
| R3 | A small local model on a 4 GB GPU cannot produce valid plans reliably. | Confirmed in Phase 1 (2–4 of 10 turns correct). Both agents use Claude by default (Q27); local runs stay optional. |
| R4 | Schema support differs between Ollama and Claude, for example `anyOf` for the action union, or structured outputs together with fallbacks. | Resolved in Phase 1: both accept the schema; property order matters (section 10.1). |
| R5 | A CBOR library does not encode deterministically as required. | Resolved in Phase 3: ciborium's encoder keeps map entries in the order given, and its optional ordering is the older length-first rule. `fabric-token` has its own encoder, checked against RFC 8949 and RFC 8392 vectors, and its decoder refuses every other encoding. |
| R6 | Scope: this is a large build. | Phase gates G1–G3 let you stop or re-scope early. |

## 19. Task list

You approved this task list on 2026-10-18, including the Phase 0 downloads and installs. "Needs" marks steps that download software, spend money, or need something from you. Sizes are relative to each other: S small, M medium, L large.

### Phase 0 — Setup (done 2026-10-18)

| Task | Work | Size | Needs |
|---|---|---|---|
| T0.1 | Commit this spec on branch `claude/isabelle-calendar-demo`. | S | your approval |
| T0.2 | Install the pinned Rust 1.97.1 with `scripts/setup.ps1` (rustup 1.29.0 and the toolchain, into `.tooling/`). | S | download |
| T0.3 | Download Isabelle2025-2 for Windows into `.tooling/isabelle/` and record its SHA-256. | S | large download |
| T0.4 | Install Ollama for Windows and pull 2–3 candidate models for the spike. | S | install, several GB |
| T0.5 | The git-ignored `.env` file holds `ANTHROPIC_API_KEY` (you paste it) and `ANTHROPIC_WORKSPACE_ID`. | — | you |

### Phase 1 — Spikes, then gate G1 (done 2026-10-18; results in `spikes/phase1/RESULTS.md`)

| Task | Work | Size | Needs |
|---|---|---|---|
| T1.1 | Isabelle spike: from Rust, start `isabelle server` through Cygwin, `session_start` HOL, run a generated theory with `use_theories`, read per-lemma results including failed proofs, `purge_theories`, and measure latency. | M | |
| T1.2 | Ollama spike: run the candidate models on 5 scripted turn inputs with the real plan schema; measure valid-output rate and latency. You pick the model. | M | |
| T1.3 | Claude spike: the same 5 inputs with `claude-opus-5-5` over raw HTTP, with structured output, adaptive thinking, `effort: "medium"` and fallbacks; measure validity, latency, tokens and cost. | S | small API spend |
| G1 | You review the spike results and confirm or adjust D6, D7, D8 and D25. | | your review |

### Phase 2 — Semantic layer, then gate G2 (done 2026-10-18; rules approved as written, Q28)

| Task | Work | Size | Needs |
|---|---|---|---|
| T2.1 | `Fabric_Policy.thy`: datatypes, the rules in section 8.2, `permitted`, and the sanity lemmas; build the session. | M | |
| T2.2 | Rust action and context types, the HOL renderer, and the check-theory generator (screening batches and single actions). | M | |
| T2.3 | Policy gate: the verdict procedure (prove the rule or its negation), denial values, timeouts and `undecided`, proof IDs, and audit records. | M | |
| T2.4 | Policy test corpus with permit and deny cases for every rule, run against live Isabelle, plus renderer tests against hand-written theories. | M | |
| G2 | You review the Isabelle rules and the test corpus. | | your review |

### Phase 3 — Tokens and keys (done 2026-10-18; you approved its choices as made, sections 9 and 16)

| Task | Work | Size | Needs |
|---|---|---|---|
| T3.1 | Deterministic CBOR encoding with test vectors. | S | |
| T3.2 | CWT/COSE_Sign1 tokens: mint and verify, proof of possession, expiry, single-use nonces, argument digests. | M | |
| T3.3 | Keys: generate, store in Windows Credential Manager, and distribute public keys through the launcher's configuration. | S | |

Results:

- `crates/fabric-token` has 27 tests that run everywhere, CI included. They cover the CBOR vectors, the hand-assembled token layout, coset interoperability, and every rejection reason. They also cover A6: single use (including two requests at once), binding to the arguments, and binding to the agent key.
- Two tests run locally only:
  - A Credential Manager round trip under the test prefix `fabric-demo-test/`, which deletes its entries afterwards.
  - A live Isabelle check: a permitted `ReadFreeBusy` (0.7 s) mints a 275-byte token that the calendar verifier accepts once. An over-broad read is denied by `read_window`, and minting from that denial is refused.

### Phase 4 — Calendar service (done 2026-10-18)

| Task | Work | Size | Needs |
|---|---|---|---|
| T4.1 | Encrypted store, fixture loader and owner API. | S | |
| T4.2 | Token-gated `ReadFreeBusy` and `AddMeeting`, with tests for misuse and for "no titles reach agents". | M | |

Results:

- `apps/fabric-demo` has the calendar service, its client, the demo clock, the audit log, and draft fixtures in `demo/fixtures/`. Section 5.1 describes the API.
- Its 13 tests run everywhere, CI included:
  - Each kind of misuse is refused without running the action, and a failed write releases the token.
  - The store opens only with its own key and person.
  - Fixtures are checked, and the demo fixtures set up the section 14 scenario.
  - A round trip over a real loopback socket.
  - A4: across every kind of agent request and refusal, no response body, header or audit record contains a title or event ID. This holds for marker fixtures and for the demo fixtures.
- One test runs locally only: it starts the real `fabric-demo calendar` command twice. The second start decrypts the store with the key from Credential Manager (under the test prefix, deleted afterwards), and the added meeting is still there.

### Phase 5 — Messaging (done 2026-10-19; you asked on 2026-10-19 for the remaining gates to be approved automatically)

| Task | Work | Size | Needs |
|---|---|---|---|
| T5.1 | Relay: FIFO mailboxes, ciphertext-only logging, retention. | S | |
| T5.2 | Messaging gateway: Noise IK through the relay, peer key check, token-gated handshake and send, signed messages, inbound delivery, negotiation state. | L | |

Results:

- New: `crates/fabric-wire` (frames, signed messages, Noise IK), the relay, and the trusted host's gateway with its agent client. The `relay` and `host` commands run them.
- The relay tests check order, waiting reads, retention and metadata-only logs.
- The gateway tests run two gateways through a real relay on loopback:
  - A call answered only after the invitee's own handshake.
  - A full round updates both negotiations.
  - A caller with another key gets `peer_key_mismatch`.
  - Simultaneous calls settle on one session, tested both ways round.
  - Each kind of bad send is refused.
  - Replayed, altered and forged frames change nothing.
  - A5: no frame contains a note, a name, the negotiation ID, a candidate ID, or a candidate time, in CBOR, JSON, decimal or ISO form.
- One test runs locally only: the real `relay` and two `host` processes, with keys in Credential Manager under the test prefix (deleted afterwards), connect and exchange a proposal.

### Phase 6 — Agents (done 2026-10-19)

| Task | Work | Size | Needs |
|---|---|---|---|
| T6.1 | Agent runtime: turn loop, prompt builder, plan schema validation, screening, execution, re-planning, fault-injection hooks. | L | |
| T6.2 | Claude backend over raw HTTP. | S | |
| T6.3 | Ollama backend. | S | |
| T6.4 | Scripted planner for tests. | S | |

Results:

- New: `fabric-demo agent`, with its runtime, plan schema and conversion, turn input, Claude, Ollama and scripted planners, and fault hooks.
- The host gained its policy gate on the shared Isabelle server, the owner's request, agent registration, and screening and authorizing with tokens. The policy crate gained `Gate::connect`.
- CI tests cover the runtime's control flow with replayed answers:
  - The first fully permitted plan runs.
  - A read ends the plan.
  - Re-planning stops at three.
  - A denial while authorizing re-plans.
  - Wait plans, turn triggers, and every fault switch.
- Local tests, both passed:
  - With live Isabelle, scripted agents on both sides reached the scenario's mutual options (Tuesday 14:00 and Thursday 14:30). Isabelle also denied an over-broad read (`view_details_never`, `read_window`), and the agent recovered.
  - One Claude turn returned valid plans.

### Phase 7 — Consent and the end-to-end flow (done 2026-10-19)

| Task | Work | Size | Needs |
|---|---|---|---|
| T7.1 | Consent records: owner click to signed record, attached to `SendDecide`, verified on receipt, and turned into facts for `AddMeeting`. | M | |
| T7.2 | End-to-end negotiation with scripted planners on both sides through live Isabelle. | M | |

Results:

- Consent records live in `fabric-wire`. The gateway is the consent signer, attaches records to decisions and verifies the peer's, and gives the gate both sides' facts.
- The end-to-end test runs the section 14 happy path with scripted planners and live Isabelle, in 47 s:
  - Bob says no to Tuesday, and both say yes to Thursday.
  - Both calendars get the Thursday 14:30 meeting, with the proof ID of the check that permitted it.
  - Both sides close as agreed.
  - Every token in the hosts' logs comes from a permit with the same proof ID (A2).
  - The run took 21 Isabelle checks, 0.95 s each on average and 1.8 s at most.

### Phase 8 — Dashboard and launcher (done 2026-10-19)

| Task | Work | Size | Needs |
|---|---|---|---|
| T8.1 | Audit event intake, Server-Sent Events, and the dashboard page (section 13). | L | |
| T8.2 | Fault-injection switches wired to the agents. | S | |
| T8.3 | Launcher (`up`, `down`), configuration, and `scripts/demo.ps1`. | M | |

Results:

- New: the `up`, `down`, `reset-keys` and `dashboard` commands, the page in `demo/dashboard/`, and `scripts/demo.ps1`. Sections 5.6, 5.7 and 13 describe them.
- Rehearsed in the browser with scripted agents on both sides:
  - **Happy path.** Both owners filed their requests and got the two mutual options. Bob said no to Tuesday, and both said yes to Thursday. Both calendars got the Thursday 14:30 meeting with its proof. The run took 21 Isabelle checks, 0.91 s each on average and 1.6 s at most, and the relay carried 7 frames.
  - **Fault switches.** All five were denied or refused as section 14 expects, and the agents recovered each time:
    - `cand_count`, and `read_window` with `view_details_never`, when screening.
    - `consent` when authorizing.
    - `token_replayed` and `token_argument_mismatch` at the services.
  - **Stopping.** `down` stopped every process, Isabelle included, in under 2 seconds.
- The rehearsal found these problems, now fixed:
  - `up` failed on Windows, because a canonicalized path's `\\?\` prefix breaks Isabelle's Cygwin paths.
  - The page redrew everything on every poll, which wiped a form being filled in.
  - After an owner's click, the agent waited up to 20 seconds before its next look. Owner actions now wake it.
  - A fault switch with nothing to act on was used up silently. It now waits for its action.
  - A finished run's meeting stayed in the next run's calendars. Each run now starts from the fixtures.
  - The gateway did not log refusals. The page showed calendar requests as relay frames, and meeting lengths as dates.
- The demo's keys are now in Credential Manager, as `fabric-demo/alice/…` and `fabric-demo/bob/…`. `fabric-demo reset-keys` deletes them.

### Phase 9 — Verification and docs, then gate G3 (T9.1–T9.3 done 2026-10-19)

| Task | Work | Size | Needs |
|---|---|---|---|
| T9.1 | Automated acceptance tests A3–A7 with scripted planners. | M | |
| T9.2 | Live rehearsal with Claude for both agents, plus one run with Ollama for one agent; record the transcripts and timings, and fix what they find. | M | small API spend |
| T9.3 | README section and docs updates; `scripts/verify.ps1` clean; existing CI still green. | S | |
| G3 | You run the demo. | | your run |

Results:

| # | Evidence | Status |
|---|---|---|
| A1 | `up` and `down`, run directly and through `scripts/demo.ps1`, in every rehearsal. `down` stops every process in about 2 seconds. | Met |
| A2 | The Claude rehearsal below booked the meeting in both calendars. All 11 executed actions were authorized by a permit, and each token carries that permit's proof ID. The Ollama run started, and every action in it was checked the same way. | Met |
| A3 | The new local test `every_fault_switch_is_refused_and_the_run_still_books_the_meeting` arms all five switches against live Isabelle. Each is denied (`view_details_never`, `read_window`, `cand_count`, `consent`) or refused (`token_argument_mismatch`, `token_replayed`), and the meeting is still booked. It takes 55 s. | Met |
| A4 | The calendar tests (Phase 4), which run in CI. | Met |
| A5 | The messaging tests (Phase 5), which run in CI. | Met |
| A6 | The token tests (Phase 3), which run in CI. | Met |
| A7 | The policy corpus against live Isabelle, run locally in 36 s. | Met |
| A8 | The sanity lemmas are proven whenever the session builds (Phase 2). | Met |
| A9 | `scripts/verify.ps1` passes. | Met |
| A10 | Every check's time is in its report. In the Claude rehearsal, screening took 2.2 s on average and 3.8 s at most, under the 5-second target. | Met |

- **Local tests.** All 9 local-only tests pass:
  - the four live negotiations, A3 included;
  - the Isabelle corpus, and the token minted from a permit;
  - both Credential Manager round trips, and the two multi-process runs.
  - The two planner tests (one Claude turn, one Ollama turn) were not rerun, because the rehearsals cover both backends.
- **Live rehearsal with Claude for both agents.** The section 14 happy path, through the dashboard:
  - Alice's agent proposed Tuesday 14:00, Wednesday 15:00 and Thursday 14:30, with a note about her preference. Bob's agent answered yes, no, yes.
  - Both agents sent the owners' decisions exactly as given. They booked Thursday 14:30 in both calendars, and Alice's agent closed as agreed. No plan was denied and none failed.
  - 13 Claude calls (6 for Alice, 7 for Bob) used 63,059 input and 4,918 output tokens, 5.2 s per call on average and 8.3 s at most. That is about $0.35 at the rates Phase 1 used ($4 and $20 per million tokens).
  - 19 Isabelle checks: screening took 2.2 s on average (3.8 s at most), and authorizing 1.0 s (2.0 s at most). The relay carried 7 frames. From the first request to the last action took 88 s, including the owners' clicks.
- **Live run with Ollama (`granite4.1:3b`) for Alice and Claude for Bob.** It started, and every action was checked the same way. But it stalled, as Phase 1 predicted (R3):
  - Alice's agent read free/busy four times, at 15.5 s per turn, and never answered Bob's call.
  - Its rationale named proposals while its only action was another read.
  - Bob's two Claude calls cost about $0.05.
- **Live spend in Phase 9:** about $0.40 in all.
- The transcripts, with each agent's turn inputs, plans and verdicts, are in `.local/rehearsal/`, which git ignores.
- **Not verified: CI.** The branch has no upstream and was not pushed, so GitHub Actions has not run on it.

## 20. Out of scope

- Real calendar providers (Google Calendar, CalDAV, EventKit) and recurrence, time zones or DST beyond UTC.
- Contact lists, TOFU, MLS, and owner time-policy rules (working hours, buffers, notice, daily limits).
- The full consent rule group (once-only, own-calendar), unless you choose it in D12.
- Three or more participants.
- macOS and Linux support for the demo itself.
- Operating-system sandboxing of agents (D21).
- Production key management and an independent security review.
