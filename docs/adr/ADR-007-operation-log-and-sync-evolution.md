# ADR-007: Operation log and sync evolution

**Status:** Proposed
**Date:** 2026-10-26
**Deciders:** Areg Melik-Adamyan
**Owner:** Areg Melik-Adamyan
**Related:** [Technical architecture](../architecture.md) §9.3, §10.4, §14.3 · [PRD](../PRD.md) §2.1, §8.2, §16 · [Paracels chat prototype](../paracels-chat.md) §8, §11 · ADR-009 (sharing crypto, open) · ADR-014 (mobile client and relay, open)

## Context

Fabric is local-first, encrypted at rest and end-to-end encrypted between Fabric endpoints (PRD §2.1). It starts single-user on one Mac, then adds the user's other devices, then shared spaces (PRD §16, Phases 1 to 3). Every one of those steps changes how a mutation travels, but none of them may change what a mutation *is*, or the kernel built in Phase 1 gets rewritten in Phase 3.

The architecture document already sketches the answer in three places: an `operation_log` table in `vault.db` (§10.4), a sync model in three stages (§14.3), and a stack row that says "signed, encrypted operation feeds; Automerge per mutable document, never one CRDT for the whole vault" (§9.3). It does not say which of these is binding, in what order they are built, or what the kernel must record on day one so that later stages need no migration of history. This ADR makes those choices.

Two pieces of executable evidence exist:

- The paracels chat prototype (`apps/paracels`, complete on 2026-10-26) ran a server-first message log with device-created IDs, an on-device outbox, idempotent delivery and a server sequence used as a catch-up cursor. Its 111 tests and the manual gates showed that the combination of a client-chosen ID, one writer per log, and a monotonic cursor gives exactly-once effect and gapless catch-up across airplane mode, force-quit and a server restart. It also showed what a server-authoritative log costs: invariant 2 (local interactive path) is not met, because reading history needs the server.
- `fabric-token` and `fabric-wire` already implement deterministic CBOR, COSE_Sign1 over Ed25519 with domain-separating external data, a replay cache, and Noise IK sessions. The operation envelope can reuse them instead of inventing a second signing profile.

Quality attributes served, in priority order: durability and attribution of every canonical change (invariant 5), a local interactive path that never waits on a network (invariant 2), an upgrade path from one device to many without re-encoding history, and a bounded relay that stores ciphertext it cannot read (§14.1).

Constraints:

- PRD §8.2: no claim of one ACID transaction across SQLite, the filesystem and a remote service; no live SQLite database in a file-sync folder.
- Space is a security invariant (invariant 8): every operation belongs to exactly one space.
- Policy and HLC never grant authority or silently resolve a semantic conflict (§10.4).
- Decisions this ADR must leave open: the ciphertext wrapping of an operation and the relay's metadata posture (ADR-009), whether a paid relay is acceptable (PRD §2.2 items 5 and 13), and whether the paracels relay graduates (ADR-014).

## Decision

**Fabric records every canonical mutation as a signed operation in a per-device, hash-chained log from the first single-device release, and evolves synchronization in three stages that add transport and merge rules without changing the operation envelope.**

### D1. The operation is the unit of canonical change, from P0

- A canonical mutation appends one row to `operation_log` and updates the current-state projection in the same SQLite transaction. There is no other write path to canonical tables. Derived state (`index.db`, thumbnails, embeddings) is rebuilt from operations and is never logged.
- Event sourcing is selective. Large immutable blobs are referenced by operations (`payload_blob_id`, content identifiers) and are not replayed through the log. The access and security audit log stays a separate, higher-volume table.
- The log exists and is verified even when there is one device and no transport. Backup snapshots record the operation watermark they were taken at (§14.4).

### D2. The envelope is fixed now and reused by every later stage

An operation envelope carries, in deterministic CBOR using the `fabric-token` profile (sorted map keys, normalized integers, timestamps as integer milliseconds):

| Field | Rule |
|---|---|
| `operation_id` | UUIDv7, created on the device that performs the mutation, kept across every retry |
| `space_id`, `key_epoch` | The one space the operation belongs to, and the epoch it will be encrypted under (epoch semantics come from ADR-009) |
| `device_id`, `device_sequence` | Per-device monotonic sequence, no gaps, assigned at append time |
| `previous_device_hash` | BLAKE3 of this device's previous envelope; null only for a device's first operation |
| `hybrid_logical_clock` | For ordering across devices; carries no authority |
| `operation_type`, `schema_version` | A new field or meaning is a new version; readers refuse unknown versions rather than guessing |
| `idempotency_scope`, `idempotency_key` | Scoped so that connector replays and user retries dedupe without colliding across spaces |
| `causation_id`, `correlation_id` | Optional provenance links to the operation or job that caused this one |
| `payload_hash` or ciphertext | The plaintext hash is inside the envelope; what the relay sees is decided by ADR-009 |
| `signature` | COSE_Sign1 with the device's Ed25519 signing key over the envelope's CBOR, external data `fabric-operation-v1` |

The signature covers every field above, so an operation cannot pass for a token, a consent record or a wire message (which use `fabric-message-v1` and their own tags), and an envelope cannot be re-sequenced, moved between spaces or re-attributed without breaking the chain.

### D3. Synchronization evolves in three stages on top of D1 and D2

| Stage | Roadmap | What is added | What stays fixed |
|---|---|---|---|
| S0 single device | PRD Phase 1 | The log, chain verification at startup, snapshot at a watermark, restore verifier | Envelope, append transaction |
| S1 personal devices | PRD Phase 3 | Device enrollment, per-device feeds sent through a relay as opaque ciphertext, per-device cursors, witnessed signed feed heads, snapshot compaction, the field-level merge matrix | Envelope, chain, local-first reads |
| S2 shared spaces | PRD Phase 3 | Space membership and epochs (ADR-009), one Automerge document per collaboratively mutable note or document, carried as operation payloads in the same feeds | Everything above; no whole-vault CRDT |

Each stage's log is readable by the next without rewriting history. A device enrolled in S1 replays an S0 log unchanged; an S2 space is an S1 feed whose payloads include CRDT changes.

### D4. The relay is a cursor, never an authority

The paracels prototype's lesson is kept and its limitation removed. A relay assigns each space a monotonic receive sequence that devices use as a catch-up cursor, exactly as the chat used `seq`, and it stores at most one copy of an operation per relay-visible identifier (ADR-009 D4 makes that a keyed hash of `operation_id`, so the relay deduplicates without seeing the ID). But authority stays with the device chain: a device accepts an operation only if its signature verifies, its `device_sequence` continues the sender's chain, and its hash links to the previous envelope it holds. A relay that reorders, drops, replays, or serves an old head is detected by the chain and the witnessed heads, and the client reports incomplete synchronization instead of silently trusting the cursor. A relay reset (the chat's `databaseId` change) is a new relay identity, not a reason to discard local operations.

### D5. Conflicts are classified, never guessed

Every mutable field in the canonical schema is assigned one class in a merge matrix maintained next to the schema: immutable, multi-value register, observed-remove set, CRDT text, presentation-only last-writer-wins, or explicit manual conflict. Replay order and HLC decide only inside the presentation-only class. A semantic disagreement in any other class surfaces as a visible fork for the user to resolve, and the resolution is itself an operation.

### D6. Compaction is a signed, acknowledged operation

A snapshot that prunes history records a signed snapshot root, the operation watermark, the reducer and schema version, the late-join history policy of the space, and the acknowledgements of every enrolled device. Operations before the watermark may be pruned only after those acknowledgements.

## Options Considered

### Option A: Status quo, no operation log; sync the canonical database as a file

| Dimension | Assessment |
|---|---|
| Complexity | Low to build, unbounded to recover from |
| Cost | None up front |
| Scalability | Fails at two devices |
| Team familiarity | High |

**Pros:** nothing to design in Phase 1.
**Cons:** PRD §8.2 forbids a live SQLite database in a synced folder, and WAL is unsafe on network filesystems. No attribution per change (invariant 5). Every later stage becomes a migration of history. Rejected.

### Option B: One CRDT for the whole vault

| Dimension | Assessment |
|---|---|
| Complexity | Medium to start, high in steady state |
| Cost | A CRDT document holding every canonical object |
| Scalability | Pathological merge state and document size at personal-vault scale |
| Team familiarity | Medium |

**Pros:** merge is "free" for every field.
**Cons:** §9.3 rules it out explicitly. Every field gets CRDT semantics whether or not that is the right conflict class, which violates D5. Blobs, policy, audit and provenance do not belong in a CRDT. Rejected.

### Option C: Server-authoritative log, the paracels model generalized

| Dimension | Assessment |
|---|---|
| Complexity | Low; proven by the prototype |
| Cost | A server that must be reachable to read |
| Scalability | Fine for a chat room, wrong for a vault |
| Team familiarity | High, it was just built |

**Pros:** simplest exactly-once story; one writer assigns the order.
**Cons:** invariant 2 is not met: reading needs the server. The relay would hold the only order and the only history, so a relay compromise or withholding is undetectable. Keeps its cursor idea (D4), rejected as the authority model.

### Option D: Per-device signed hash-chained feeds with selective CRDT per document (chosen)

| Dimension | Assessment |
|---|---|
| Complexity | Medium: an envelope, a chain verifier, a merge matrix |
| Cost | One signing per mutation, one chain check per received operation |
| Scalability | Linear in operations per device; compaction bounds history |
| Team familiarity | Medium; CBOR, COSE and replay handling already exist in `fabric-token` |

**Pros:** local reads never wait; every change attributed and tamper-evident; the relay stores ciphertext and can be replaced; S0 to S2 reuse one envelope; CRDTs are used only where the field class calls for them.
**Cons:** the merge matrix must be maintained with the schema; witnessed heads and compaction acknowledgements are protocol work with no user-visible feature until S1; two sources of order (device chain and relay cursor) must be explained in the UI.

## Trade-off Analysis

The decision trades Phase 1 effort for the absence of a Phase 3 rewrite. Option C would ship a working multi-device chat fastest, and the prototype proves it, but it cannot become Fabric's vault without breaking invariant 2 and handing the relay authority over history. Option D costs a signature and a hash per mutation, which is negligible against the write budget in PRD §8.1, and a merge matrix, which is work the schema needs anyway to state what a conflict means.

The real risk in Option D is scope creep in S1: witnessed heads, compaction and device revocation are each a protocol. D3 keeps them out of Phase 1 by making S0 nothing more than "the log exists, is chained, and is verified at startup". Everything S1 adds is transport and agreement on top of operations that are already correct.

What this ADR does not decide, on purpose: how an envelope is encrypted and what a relay learns (ADR-009), how two parties pair and pin keys (ADR-012), and whether the Node relay from paracels graduates or is rewritten in Rust (ADR-014). Those decisions consume this envelope; they do not change it.

## Consequences

- **Easier:** attribution and audit of every canonical change from the first release; backups at an exact watermark; a device replica for the chat (paracels §11, local-first seam) becomes an S1 feed instead of a new design; ADR-014 can be written against a fixed envelope.
- **Harder:** every canonical write goes through one append path, so bulk imports and connector replays must batch operations rather than bypass them; schema changes must update the merge matrix; the UI must show "synchronization incomplete" states instead of hiding them.
- **Operational burden:** a chain verifier runs at startup and after every sync; a corrupt or forked chain is a user-visible repair flow, not a log line. Compaction needs every enrolled device to acknowledge, so a lost device blocks pruning until it is revoked.
- **Revisit when:** ADR-009 chooses MLS over envelope encryption, which changes what `key_epoch` means and whether the relay sees `operation_type`; ADR-004 finds the editor's CRDT cannot be carried as an operation payload; measured operation volume from connectors (email, messages) makes one signature per mutation a bottleneck; PRD §2.2 items 5 and 13 pick a hosted relay with retention rules the cursor model must honor.

## Assumptions and evidence

| Assumption | Evidence | Residual uncertainty |
|---|---|---|
| A client-chosen ID plus one writer per log gives exactly-once effect across retries and restarts | Paracels: the server stores at most one row per ID, the outbox resends the same ID, and gates G4 and G5 passed airplane mode, force-quit and a server restart | Only tested with one room and two devices |
| A monotonic receive sequence is a sufficient catch-up cursor | Paracels §7.5: catch-up after every reconnect filled every gap; `AUTOINCREMENT` never reuses a number | Gaps after refused inserts are fine for a cursor, wrong for a chain; hence D4 separates them |
| Deterministic CBOR and COSE_Sign1 are already correct in this codebase | `fabric-token` `cbor` and `cose` modules, used by the calendar demo's tokens and `fabric-wire` messages | Neither has been profiled at thousands of signatures per second |
| Automerge per document is viable for the editor | Architecture §9.3 names it; ADR-004 has not confirmed a ProseMirror binding | Open until ADR-004 |

## Executable manifestations

1. A `fabric_core::oplog` module: the envelope type, CBOR encoding through `fabric_token::cbor`, signing and verification through `fabric_token::cose` with external data `fabric-operation-v1`, and a chain verifier that reports the first gap, fork or bad signature.
2. The `operation_log` migration in `vault.db` as in §10.4, with `UNIQUE(device_id, device_sequence)` and `UNIQUE(space_id, idempotency_scope, idempotency_key)` enforced by the database, not the application.
3. A merge-matrix file next to the schema, with a test that fails when a mutable column has no class.
4. Property tests: any permutation of a device's operations replays to the same projection; any single-bit change to an envelope fails verification; a replayed `operation_id` is a no-op.
5. A fitness function in CI: no module outside the append path may write a canonical table (a dependency rule, in the spirit of the paracels lint rule that keeps server code out of the app).

## Action Items

1. [ ] Approve or amend D1 to D6; record the outcome and bump this ADR to Accepted.
2. [ ] Add the `operation_log` migration and `fabric_core::oplog` with the chain verifier (S0), with the property tests in item 4.
3. [ ] Write the first merge matrix for the Phase 1 schema (item, revision, relation_assertion, artifact).
4. [ ] Draft ADR-009 against this envelope: what is encrypted, what the relay sees, how `key_epoch` is assigned.
5. [x] Draft ADR-014: map the paracels outbox and cursor onto S1, and decide the relay's language and home. Drafted 2026-10-26.
6. [ ] Link this ADR from §9.3 (Sync row), §10.4 and §14.3 of the architecture document once accepted.
