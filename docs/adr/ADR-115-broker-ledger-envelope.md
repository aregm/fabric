# ADR-115: Broker ledger envelope

**Status:** Accepted on 2026-11-16
**Date:** 2026-11-16
**Deciders:** the owner
**Owner:** the owner
**Related:** [Roadmap](../roadmap.md) `RD-14`, §4.4, §5, §8.3 · [Calendar Desktop plan](../plans/calendar-desktop-plan.md) §4.3, §11 · [ADR-107](ADR-107-operation-log-and-sync-evolution.md) D1 to D3 (Proposed) · [ADR-108](ADR-108-agent-and-mcp-permission-model.md) (Proposed) · `crates/fabric-storage` (`ledger.rs`, `migrations/0001_initial.sql`) · [CD-W04 design](../plans/calendar-desktop-w04-design.md) section 4

This record decides `RD-14` for the broker. The code of work package `CD-W04` implements it as written. The owner accepted it on 2026-11-16 by merging the pull request that carries this file, pull request 23 of `CD-W04`.

## Context

`RD-14` asks two questions: does Fabric have an operation log at all, now that the vault is PIM's, and does the envelope gain a signed agent field. Its suggested default was to settle the broker's envelope fields in `CD-W01`. That was not done: the contract has no envelope. The roadmap says the envelope is needed before `CD-W04`, and gives `CD-W04` the broker's single-device envelope and its agent field. The rest of `RD-14` waits for multi-device work, after `CD-G2a`, on evidence from the broker's own ledger.

Plan §11 asks the broker to append canonical state changes with "the selected single-device operation-envelope pattern", without making the future multi-device vault a prerequisite. It also asks the broker to keep canonical mutations apart from the higher-volume access audit, as Proposed ADR-107 D1 does.

ADR-107 D2 defines an envelope for many devices and shared spaces: UUIDv7 ids, a space and key epoch, a hybrid logical clock, BLAKE3 links, deterministic CBOR and a COSE_Sign1 signature by the device key. The roadmap (§8.3) lists its gaps. It has no actor or agent field. BLAKE3 and UUIDv7 are not in the tree. CBOR and COSE live in `fabric-token`, which links the demo's policy crate (`RD-9`).

The broker is one process on one Mac, with one writer and one encrypted SQLCipher file whose key is in the Keychain. Plan §4.3 states same-user hostile software as a residual threat.

## Decision

**The broker records every change to its canonical state as one row of a single-device ledger. The envelope is a subset of ADR-107 D2 plus an actor field. A SHA-256 hash chain covers every field. No row is signed.**

### D1. One ledger row per write transaction

- Every change to a projection table is a typed mutation applied inside `Store::write`, with one exception: a migration's SQL, which runs in its own `schema.migrated` transaction, may also seed projection rows (version 1 seeds `counters`, `profile_state` and `preferences`). `verify_replay` runs that SQL again at each `SchemaMigrated`, so the seeds are part of the fold.
- At commit, a transaction that applied at least one mutation appends exactly one ledger row. The ledger head in `meta` is updated in the same transaction.
- The projection is the fold of the mutations. `verify_replay` rebuilds an in-memory database from the ledger and compares every projection table.
- The `ledger` table is append-only. SQL triggers refuse an update or a delete.

### D2. The envelope fields

| Field | Rule | ADR-107 D2 |
|---|---|---|
| `seq` | Gap-free from 1. | `device_sequence`; the store's `meta.store_id` stands for `device_id` |
| `operation_id` | `op-` and 32 lowercase hex digits from 128 random bits. Unique. | `operation_id`, without UUIDv7 |
| `operation_type` | One of a fixed list (below). An unknown type fails verification. | same |
| `payload_version` | 1. An unknown version fails verification. | `schema_version` |
| `committed_at_ms` | The step's time from the broker's clock. Never below the previous row's. Verification checks it. | replaces the hybrid logical clock |
| `run_id` | The broker run that committed the row. Null only on a migration row. | new |
| `actor_kind`, `actor_agent_id` | `owner`, `broker`, or `agent` with the agent id the agent channel authenticated. The agent id is present if and only if the kind is `agent`. | **new: the actor field** |
| `idempotency_scope`, `idempotency_key` | Both present or both null. Unique together. | same |
| `causation_id`, `correlation_id` | The cause: the owner command id, the agent call id, or the proof id of the report being settled; null otherwise. The request id; null on a row not tied to a request (connect, disconnect, seed, timers, start, quit). | same |
| `payload`, `payload_hash` | The canonical JSON `{"mutations":[...]}`, and the SHA-256 of it in hex. | `payload_hash` |
| `previous_hash`, `envelope_hash` | The hash chain, below. | `previous_device_hash`, with SHA-256 for BLAKE3 |

The actor names who caused the row. `owner` is a command from the owner channel. `agent` is a call from an agent; in `CD-W04` only the test agent and the debug-only synthetic agent exist, and `CD-W06` brings the real agent channel. `broker` is everything the broker does on its own: start-up, recovery, timers, the proof pipeline, migrations and quit.

Idempotency keys are used in three places only:

- `owner_command` with the command id, on a command's transaction or on its refusal transaction. Only one of the two ever commits for one id.
- `agent_call` with `<agent_id>/<call_id>`, only on the transaction that inserts a request. A repeated call writes nothing.
- `startup` with the run id, on the recovery transaction.

Every other transaction carries none. A clash on the unique key is therefore a fault, never a normal repeat.

The operation types are fixed: `store.created`, `schema.migrated`, `broker.started`, `broker.quit`, `broker.seed`, `owner.decide_request`, `owner.pause_agent`, `owner.revoke_grant`, `owner.save_profile`, `owner.save_preferences`, `owner.save_group`, `owner.disconnect_google`, `owner.refused`, `agent.connect`, `agent.disconnect`, `agent.call`, `timer.session_idle`, `request.advance`, `request.proof`, `request.authorize`, `request.dispatch`, `request.attempt`. A new store writes one `schema.migrated` row that also carries the store's creation; no separate `store.created` row is written today, but the type stays in the list.

### D3. The hash rule

```text
envelope_hash = sha256_hex("fabric-broker-ledger-v1\n" + canonical({
                  seq, operation_id, operation_type, payload_version, committed_at_ms, run_id,
                  actor: {kind, agent_id},
                  idempotency: {scope, key} | null,
                  causation_id, correlation_id, payload_hash, previous_hash }))
previous_hash = the envelope_hash of row seq - 1; 64 zeros for row 1
```

- `canonical` is the contract's canonical JSON (contract README §3): members ordered by name, no whitespace, fixed string escapes, integers in plain decimal. A null field is written as `null`.
- The domain line `fabric-broker-ledger-v1` keeps a ledger hash apart from every other hash Fabric computes.
- The payload enters through `payload_hash`, so the envelope hash covers the payload too.
- The store verifies the whole chain at every open: gap-free `seq`, known types and versions, payload hashes, links, envelope hashes, time that never goes back, and the head in `meta`. A failure leaves the store unavailable. The backup taken before a migration is also checked by replay.

### D4. No signature today

The envelope is not signed, and the agent field is not signed either.

A signing key would sit beside the store key, readable by the same process under the same user. Anyone who can rewrite the decrypted store can also read that key and sign again. On one device a signature adds nothing against that attacker, and same-user hostile software is already an accepted residual (plan §4.3). A signature would also bring CBOR and COSE from `fabric-token`, which links the demo's policy crate (`RD-9`).

### D5. What the chain detects, and what it does not

Detected:

- A changed field in any row, the payload included.
- A row inserted, deleted or moved.
- Rows cut from the tail, because the head in `meta` no longer matches.
- Time that goes back between rows.
- Page tampering in the encrypted file, by SQLCipher's page HMAC rather than by the chain.

Not detected:

- **A whole-file rollback.** If the whole database is replaced by an older copy, every check passes: the older chain and its head are consistent. Fixing this needs a head kept outside the file, such as a counter in the Keychain or a witness on another device. This record does not add one.
- A rewrite by anyone who holds the store key. They can rebuild a valid chain. The chain is an integrity check against damage, bugs and partial edits, not a defence against the key holder.

### D6. Ledger and access audit are separate

The `audit` table holds access records, not state changes: an owner's full-detail read of a request, a reused command, an unknown agent, a `not_permitted` refusal, and `busy` refusals, counted into one row per agent per one-minute window.

- Audit rows are not chained and never replayed.
- They are written by the same single writer. For a full-detail read, a reused command, an unknown agent and a `not_permitted` refusal, a failed write stops the action (fail-closed). A full-detail read is answered only after its audit row commits.
- `busy` refusals are the exception. Each is counted in memory and answered at once. One `agent_busy` row per agent is written at the first due-work step after the agent's one-minute window ends, or at quit for every open window. A crash loses the open window's count, at most 60 seconds of refusals per agent; this is an accepted residual. A failed write of that row halts the broker, but it cannot recall the `busy` answers already sent.
- An audit-only write appends no ledger row. So a flooding agent cannot turn the request limits into ledger growth.
- The table is append-only, like the ledger.

One logical request is one `requests` row and many ledger rows that share `correlation_id`, the request id. This is ADR-107 D1's split, and plan §11's.

### D7. ADR-107 fields left out, and why

| Left out | Why |
|---|---|
| `space_id`, `key_epoch` | One owner, one store, no sharing. Epoch meaning belongs to ADR-109, which is not decided. |
| `device_id` | One device. `meta.store_id` names the store. |
| Hybrid logical clock | One writer orders the rows by `seq`. `committed_at_ms` records time. |
| `signature` (COSE_Sign1) | No gain on one device (D4). |
| BLAKE3, UUIDv7, deterministic CBOR | None is in the tree for this crate. SHA-256 and canonical JSON are already in the contract crate. CBOR would link `fabric-policy` (`RD-9`). |
| `payload_blob_id` | The broker stores no blobs. |

These are deferred to ADR-107's stage S1, not rejected. If the broker's ledger ever has to travel between devices, it needs a new envelope version, not this one extended in place. ADR-107 promises that an S1 device replays an S0 log unchanged; this ledger does not make that promise.

## Options Considered

- **A signed agent field.** The agent's bridge would sign each request with its enrollment key, and the envelope would keep the signature. This proves which enrolled key sent a call, beyond the broker's own word. Not chosen now: the bridge and its keys come in `CD-W06`, the contract has no place for a request signature, and the broker already authenticates the connection by key possession (plan §4.3). It can be added later as a new payload version. A signature by the broker itself was rejected for the reason in D4.
- **ADR-107's full envelope now.** Not chosen: it needs fields with no meaning on one device, an ADR (109) that is not decided, two libraries not in the tree, and the CBOR codec that links the policy crate. ADR-107 itself is Proposed and has known gaps (roadmap §8.3), including the missing actor field.
- **No chain.** Rows with ids and a replay check, but no hashes. Not chosen: the chain is cheap (two SHA-256 per write), and without it a damaged or partly edited ledger would replay into a wrong projection without notice. Plan §11 asks the start-up to verify durable history.

## Consequences

- Every canonical change is attributed to the owner, an agent or the broker, and tied to a run, a cause and a request.
- Start-up time grows with history, because the whole chain is verified at every open. A verified-head checkpoint can come later.
- Ledger payloads hold calendar content, inside the encrypted file. Deleting audit history (`CD-Q11h`) needs a compaction design later; the append-only triggers forbid it today.
- A whole-file rollback stays undetected (D5). This is a known residual.
- The fixed operation-type list means a new kind of write is a code change and a review, not a free string.
- The tests pin exact hash vectors: the canonical text of a row and its envelope hash.

## If the owner decides otherwise

No store with real data exists yet. The only stores are test stores in temporary directories. A change to any field, to the canonical form, to the domain line or to the hash function changes every hash vector. The tests' expected vectors are regenerated, `migrations/0001_initial.sql` is edited in place (it has not shipped), and `crates/fabric-storage/src/ledger.rs` changes with it. No migration of history is needed. After the first release with real data, the same change needs a new domain line (`fabric-broker-ledger-v2`) and a migration.

## Open

- `RD-14`'s first question for Fabric as a whole: whether Fabric keeps an operation log beyond the broker, now that the vault is PIM's. The roadmap decides it after `CD-G2a`, on evidence from this ledger.
- Whether to anchor the ledger head outside the file, to detect a whole-file rollback.
- Whether `CD-W06` adds a request signature by the agent's bridge (the first option above). Answered by the owner on 2026-11-17: no request signature in version 1; the key is checked once per connection (plan §15.2, `NOV17-SIGN`).
- Replay is not version-aware. `verify_replay` applies this build's apply code to every earlier row, so a migration that changes a table the apply code writes (a renamed or retyped column, a changed constraint) would replay the rows before it wrongly. Until replay chooses its apply and decode rules by the replay database's schema version, and verification accepts every known payload version, migrations may only add tables, nullable or defaulted columns and indexes. This must be settled before `CD-W05`'s migration 2.

## Action Items

1. [x] The owner accepts this record by merging the `CD-W04` pull request, or asks for changes before the merge. Merged as pull request 23 on 2026-11-16.
2. [x] `CD-W04` implements D1 to D7 in `crates/fabric-storage`.
3. [x] On acceptance, mark `RD-14` decided for the broker in the roadmap, and set this record to Accepted.
4. [x] `CD-W06` records whether the agent channel signs requests. It signs none in version 1: the bridge proves
   its enrolled key once per connection, in the handshake
   ([CD-W06 design](../plans/calendar-desktop-w06-design.md) section 4.4; the owner's decision of 2026-11-17).
