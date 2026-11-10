# Requirements Traceability — Implementation Against the Specifications

**Status:** snapshot taken 2026-10-26 on branch `claude/agent-sandbox` (which includes `cursor/paracels-chat`). Regenerate when a phase closes.

**Scope:** every requirement with an ID in the [PRD](PRD.md) §7 and the [calendar vertical](calendar.md) §7.6, mapped to what the repository implements, with the evidence and the next step
**Related:** [Technical architecture](architecture.md) · [Isabelle-gated calendar demo](isabelle-calendar-demo.md) · [Paracels chat prototype](paracels-chat.md) · [Agent Sandbox proposal](agent-sandbox.md) · [ADRs](adr/)

---

## 1. How to read the status column

| Status | Meaning |
|---|---|
| **Implemented** | Production-shaped code in a workspace crate, covered by tests that run in CI, and the spec names it as the implemented slice |
| **Demo** | Exists in `apps/fabric-demo` or `apps/paracels` as evidence of the semantics, with tests, but the artifact says of itself that it is not production (synthetic data, one platform, or no security claim) |
| **Partial** | Some element of the requirement exists in code; the acceptance criterion is not met |
| **Specified** | A design, ADR or proposal fixes the shape; no code |
| **Not started** | Neither code nor a design beyond the requirement text itself |

Evidence names a crate module, a test file, or a spec section. "Demo, Windows" means the component builds and was verified only on Windows 11 (demo §16); on macOS its live tests are ignored.

## 2. Summary

| Area | IDs | Implemented | Demo | Partial | Specified | Not started |
|---|---:|---:|---:|---:|---:|---:|
| Capture (CAP) | 10 | 0 | 0 | 0 | 0 | 10 |
| Knowledge (KNO) | 11 | 0 | 0 | 0 | 1 | 10 |
| Retrieval (RET) | 7 | 0 | 0 | 0 | 0 | 7 |
| Automation and AI (AIA) | 13 | 0 | 1 | 5 | 5 | 2 |
| Sharing (SHR) | 5 | 0 | 0 | 0 | 3 | 2 |
| Calendar (CAL) | 12 | 2 | 4 | 0 | 0 | 6 |
| **Total** | **58** | **2** | **5** | **5** | **9** | **37** |

What the numbers say: the repository has executed the two hardest *semantic* claims of the first Fabric application (bounded candidate negotiation and deterministic local availability) and demonstrated the authority model around them (per-action tokens from a policy proof, signed consent, an opaque relay). It has not started the kernel that every P0 capture, knowledge and retrieval requirement depends on. Phase 1 of the roadmap (PRD §16, "kernel MVP") is the gate for 28 of the 37 unstarted rows.

## 3. Roadmap milestones

| Milestone (PRD §16, calendar §23.16) | State | Evidence |
|---|---|---|
| M0 executable two-agent agreement semantics | **Done** | `fabric-core::rendezvous` and `::calendar`; `fabric demo-meeting` reaches `Agreed` on `c3`; exhaustive Boolean selection test (`apps/fabric-cli/tests/demo_meeting.rs`) |
| Isabelle-gated calendar demo (between M0 and C2: policy proofs, tokens, Noise relay, consent, agents, dashboard) | **Built, phases 0 to 9 done; gate G3 (your own run) open** | demo §19; `fabric-policy`, `fabric-token`, `fabric-wire`, `apps/fabric-demo`; Windows 11 only |
| Paracels chat prototype (message lifecycle, outbox, relay as cursor, attachments) | **Done, gates G0 to G6 closed 2026-10-26** | paracels §17; `apps/paracels`, 111 tests |
| ADR-007, ADR-009, ADR-014 | **Drafted, Proposed** | `docs/adr/` |
| Agent Sandbox proposal | **Drafted; D1 to D13 await gate G0** | `docs/agent-sandbox.md` |
| Phase 0 one-week reality test | Not started | |
| Phase 1 kernel MVP (encrypted schema, blob store, operation log, capture, search, editor, export) | Not started; ADR-007 specifies the operation log | |
| Phase 2 trusted personal automation | Not started | |
| Phase 3 first application plus encrypted sharing | Not started; ADR-009 specifies the crypto; C0 to C5 not started | |
| Calendar C0 standards and Google spike | Not started | |

## 4. Capture and ingestion (PRD §7.1)

| ID | P | Requirement | Status | Evidence and gap |
|---|---|---|---|---|
| CAP-001 | P0 | Universal capture with a durable receipt in ≤100 ms | Not started | No capture broker exists |
| CAP-002 | P0 | Immutable raw source plus normalized representation | Not started | Blob store specified in architecture §9.3 and §14.2 |
| CAP-003 | P0 | Idempotent ingestion by source ID and fingerprint | Not started | The idempotency pattern (client-chosen ID, one writer, at most one row) is proven in paracels and fixed as scoped idempotency keys in ADR-007 D2; no ingestion path uses it yet |
| CAP-004 | P0 | Resumable background processing with visible state | Not started | Job DAG described in architecture §13 |
| CAP-005 | P1 | Email connectors (Gmail, Graph, IMAP, JMAP) | Not started | The Agent Sandbox proposal would add a Gmail connector for the broker, read-only first; it is not an ingestion connector |
| CAP-006 | P1 | Web preservation modes | Not started | |
| CAP-007 | P1 | Video intake with captions or transcription | Not started | |
| CAP-008 | P1 | Message connector framework | Not started | Paracels is Fabric's own messaging, not a connector to others |
| CAP-009 | P1 | PDF, office, OCR, audio transcription | Not started | |
| CAP-010 | P2 | Cloud drive, bookmark, RSS, task connectors | Not started | |

## 5. Knowledge and composition (PRD §7.2)

| ID | P | Requirement | Status | Evidence and gap |
|---|---|---|---|---|
| KNO-001 | P0 | Stable items, immutable revisions, source artifacts, typed relations, evidence spans | Specified | Schema sketch in architecture §10.4; operation envelope in ADR-007. No `vault.db` exists |
| KNO-002 | P0 | Markdown-compatible notes with block structure | Not started | Editor decision deferred to ADR-004 |
| KNO-003 | P0 | Human and AI-derived content visibly distinct | Not started | Epistemic model in PRD §4.2 |
| KNO-004 | P0 | Accept, reject, edit suggestions without overwriting them | Not started | |
| KNO-005 | P1 | First-class people, projects, tasks, events, claims, decisions, sources | Not started | |
| KNO-006 | P1 | Contradiction, support, refinement relations | Not started | |
| KNO-007 | P1 | Templates and saved views as queries | Not started | |
| KNO-008 | P1 | History, diff, restore, merge, tombstones | Not started | ADR-007 D5 fixes the conflict classes that merge and fork will use |
| KNO-009 | P0 | Independent intent field | Not started | |
| KNO-010 | P1 | Learning mode | Not started | |
| KNO-011 | P1 | Development mode with claims and belief revision | Not started | |

## 6. Retrieval and exploration (PRD §7.3)

| ID | P | Requirement | Status | Evidence and gap |
|---|---|---|---|---|
| RET-001 | P0 | Instant lexical search, p95 ≤50 ms | Not started | FTS5 chosen in architecture §9.3 |
| RET-002 | P0 | Hybrid retrieval with explained signals | Not started | |
| RET-003 | P0 | Search never requires an LLM | Not started | Follows from RET-001 once built |
| RET-004 | P1 | Bounded graph explorer | Not started | |
| RET-005 | P1 | Timeline, conversation, map, project and person projections | Not started | |
| RET-006 | P1 | Grounded answers citing exact spans | Not started | |
| RET-007 | P1 | Duplicate and contradiction discovery | Not started | |

## 7. Automation and AI (PRD §7.4)

| ID | P | Requirement | Status | Evidence and gap |
|---|---|---|---|---|
| AIA-001 | P0 | Local model provider abstraction (Ollama, OpenAI-compatible) | Partial | `apps/fabric-demo/src/agent/planner.rs`: Claude and Ollama backends chosen per run (demo §10). It abstracts an agent planner, not the vault's processing pipeline, and Claude is a cloud provider |
| AIA-002 | P0 | Schema-constrained extraction; invalid output never partially committed | Partial | The agent's plan is a JSON schema with ordered properties and is validated before any action (demo §10.1, G1). No extraction pipeline |
| AIA-003 | P0 | Processing provenance (model digest, prompt version, inputs, outputs, evaluator) | Partial | Audit records carry proof ID, rule, token and action per executed step (`audit.rs`, demo §13); model digest and prompt version are not recorded; nothing for derived objects |
| AIA-004 | P0 | Rules deterministic and separate from inference; inspect, test, disable, roll back | Demo, Windows | `isabelle/Fabric_Policy/Fabric_Policy.thy` and `fabric-policy`: every action proven against the theory; a 39-case corpus (A7); the agent cannot alter its policy. No user-facing inspect or disable |
| AIA-005 | P1 | Infer candidate rules from repeated actions | Not started | |
| AIA-006 | P1 | Three tiers auto, suggest, confirm; destructive never auto | Partial | The demo has permit or deny plus owner consent for the commit (demo §12); the Agent Sandbox proposal fixes allow, ask, block per action (D4). No tier assignment UI |
| AIA-007 | P1 | Local evaluation set gating model upgrades | Not started | Phase 1 of the demo measured local models choosing the right plan in 2 to 4 of 10 turns (Q27); a measurement, not a gate |
| AIA-008 | P1 | Agent access through MCP plus typed API; scopes, purpose, expiry, audit outside the model | Partial | The demo enforces scopes, expiry (60 s), single use and audit outside the model through per-action tokens (`fabric-token`, A6), over loopback HTTP, not MCP. No MCP server exists |
| AIA-009 | P1 | Permission profiles with per-action modes | Specified | `docs/agent-sandbox.md` D3, D4 |
| AIA-010 | P1 | Incremental authorization with scope narrowing and duration | Specified | `docs/agent-sandbox.md` §7 item 3, D5 |
| AIA-011 | P1 | Recipient and attendee rules by set | Specified | `docs/agent-sandbox.md` D6 |
| AIA-012 | P1 | Every agent tool call recorded in OpenTelemetry GenAI attributes before the external call | Specified | `docs/agent-sandbox.md` D7, §6. The demo's audit log is the nearest code (one record per action, no OTel names) |
| AIA-013 | P1 | Grants revocable at any time, effective on the next call | Specified | `docs/agent-sandbox.md` D5, A3. Demo tokens expire in 60 s and are single-use (`replay.rs`), which bounds but does not revoke |

## 8. Sharing and communication (PRD §7.5)

| ID | P | Requirement | Status | Evidence and gap |
|---|---|---|---|---|
| SHR-001 | P1 | Export as Markdown plus JSON-LD with pseudonymous IDs | Not started | |
| SHR-002 | P2 | E2EE spaces with per-space keys and device membership; relay cannot decrypt | Specified | ADR-009 D1 to D4; the demo relay and paracels relay are the two seeds (ADR-014 D2). The demo's relay capture test A5 is the evidence that a ciphertext-only relay is achievable |
| SHR-003 | P2 | Share a note, collection or subgraph with a preview of what leaves | Not started | Calendar disclosure receipts (§23.5) are the pattern |
| SHR-004 | P2 | Revocation rotates future keys and explains its limits | Specified | ADR-009 D5 |
| SHR-005 | P2 | Comments, mentions, tasks, collaborative editing | Specified | ADR-007 D3 stage S2; Automerge per document, pending ADR-004 |

## 9. Calendar and agentic scheduling (calendar §7.6)

| ID | P | Requirement | Status | Evidence and gap |
|---|---|---|---|---|
| CAL-001 | P0 | Encrypted local mirror of provider calendars with cursors and tombstones | Demo, Windows | `apps/fabric-demo/src/calendar/store.rs`: one encrypted CBOR file per person (XChaCha20-Poly1305, key in Windows Credential Manager), seeded with fixtures. No provider, no sync, no cursors |
| CAL-002 | P0 | Normalize events without losing recurrence, zones, DST semantics | Not started | Synthetic UTC intervals only; the fixture suite is C0 |
| CAL-003 | P0 | Deterministic local availability and preferences | Implemented (M0 profile) | `fabric-core::calendar`: exact-candidate availability over half-open UTC intervals; the demo's free/busy endpoint clips and merges (`calendar/freebusy.rs`). No preferences, no versioned policy engine |
| CAL-004 | P1 | Negotiate by exchanging a small candidate set, no free/busy grid | Implemented (M0 profile) plus Demo | `fabric-core::rendezvous`: one round, at most three candidates, eligibility bits only; demo protocol `propose`, `respond`, `decide` over the relay; A4 (no titles in agent responses) and A5 (no plaintext in the relay capture) |
| CAL-005 | P1 | Human decisions as signed, expiring grants bound to negotiation, template, candidate, roster, projection | Demo, Windows | `fabric-wire::consent`: COSE_Sign1 consent records bound to negotiation, start, length, participants and expiry (demo §12). Not bound to an event-template digest or projection profile |
| CAL-006 | P1 | Bound candidate count, horizon, granularity, rounds, rate, expiry before any send | Demo, Windows | Isabelle rules: disclosure budget, candidate count, `read_window`, 14-day horizon, note limits (demo §8.2, Q17, Q28). No per-contact rate or session limits |
| CAL-007 | P1 | Revalidation and encrypted soft holds before commit | Not started | C3 |
| CAL-008 | P1 | Idempotent per-owner provider writes and a recoverable saga | Not started | The demo writes one meeting per owner to its synthetic store under a single-use token; no provider, no saga, no reconciliation |
| CAL-009 | P2 | Cancel or reschedule through new consented transitions | Not started | |
| CAL-010 | P1 | Provider invitations out of private mode by default | Not started | No provider adapter; projection profiles are §23.11 |
| CAL-011 | P1 | Link meetings to people, projects, notes | Not started | Needs the kernel |
| CAL-012 | P1 | Narrow scheduling capabilities and a complete audit | Demo, Windows | Distinct tokened actions `read_free_busy`, `handshake`, `send`, `add_meeting` (demo §7, §9); per-component audit log and dashboard (§13); A2, A3, A6 |

## 10. Work without a requirement ID

| Work | Why it exists | Where it lands in the specs |
|---|---|---|
| Paracels chat prototype | Exercise the message lifecycle and fix the seams for encryption, policy and local-first storage (architecture §9.6) | Evidence for ADR-007 D4 (relay as cursor) and ADR-009's seams; graduation is ADR-014. Feeds PRD §6.5 stages, not a numbered requirement |
| Isabelle policy theory and gate | Show that policy outside the model can be a proof per action (invariant 7) | AIA-004, AIA-008, CAL-006, CAL-012 above; the Agent Sandbox proposal reuses it (D2) |
| Per-action tokens, Noise relay, signed messages | The authority and transport layer of the demo | CAL-005, CAL-012, SHR-002 above; `fabric-wire` frames are the seed of the ADR-009 relay message |
| ADR-007, ADR-009, ADR-014 | Fix the operation envelope, the crypto and the mobile client before Phase 1 and Phase 3 code | KNO-001, SHR-002, SHR-004, SHR-005 above |

## 11. Verification state on this Mac

- `apps/paracels`: 111 tests pass; typecheck and lint clean (2026-10-26).
- Rust workspace: see the line below, recorded when the matrix was written.
- The demo's live tests (`live_negotiation`, `calendar_command`, `messaging_commands`) are `#[ignore]` and need Isabelle and Windows Credential Manager from `scripts/setup-demo.ps1`; they were verified on Windows 11 on 2026-10-19 (demo §19, Phase 9).

- Rust workspace, run 2026-10-26 through `scripts/run.sh cargo test --workspace --all-targets --all-features --locked`: 93 tests passed, 0 failed, 8 ignored across 23 test binaries. The ignored tests are the ones that need Isabelle, Ollama, the Claude API or Windows Credential Manager. So the demo's unit, protocol, relay, token and CBOR tests pass on macOS; only its live Isabelle and credential-store paths are unverified here.

## 12. Gaps that block the most rows

1. **No kernel.** `vault.db`, the blob store, the operation log and capture do not exist; 28 unstarted rows across CAP, KNO and RET wait on PRD Phase 1. ADR-007 is the design for its log; ADR-001 to ADR-006 are still unwritten.
2. **No provider.** Every calendar row past CAL-006 and every email row needs a Google adapter (calendar C0). The Agent Sandbox proposal's Gmail and Calendar connector would be the first provider code, scoped to the broker.
3. **One platform.** The demo's keys and Isabelle setup are Windows-only (`keyring` stores, `setup-demo.ps1`). Nothing in the demo has been run on this Mac.
4. **No MCP surface.** AIA-008 and the Agent Sandbox D1 both need an MCP server in Rust; none exists.
5. **No evaluation set** (AIA-007) and no learning (AIA-005), by design until Phase 2.
