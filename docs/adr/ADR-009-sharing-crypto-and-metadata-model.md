# ADR-009: Sharing crypto and metadata model

**Status:** Proposed
**Date:** 2026-10-26
**Deciders:** Areg Melik-Adamyan
**Owner:** Areg Melik-Adamyan
**Related:** [Technical architecture](../architecture.md) §9.3, §14.1 to §14.5 · [PRD](../PRD.md) §2.1, §2.2 items 5 and 13, §7.5, §8.3 · [Calendar vertical](../calendar.md) §23.5 to §23.7 · [ADR-007](ADR-007-operation-log-and-sync-evolution.md) (operation envelope) · ADR-012 (Rendezvous, open) · ADR-014 (mobile client and relay, open)

## Context

ADR-007 fixed the plaintext shape of an operation and left three questions to this record: how an operation and a blob are encrypted, what a relay can learn, and what revocation does and does not achieve. The PRD confirms the direction (§2.1: local-first, encrypted at rest, end-to-end encrypted between Fabric endpoints) and the requirements (§7.5: SHR-002 per-space keys and device membership with a relay that cannot decrypt; SHR-004 revocation rotates future keys and explains its limits). The architecture document already draws a key hierarchy (§14.2) in which a private space has a random epoch key and a shared space takes its epoch from "the MLS group key schedule", and §14.3 says MLS is "a candidate for shared-space group epochs, not a drop-in authorization system". None of that is yet binding.

Three kinds of encrypted traffic exist, and they have different shapes:

| Traffic | Members | Lifetime | History for a late joiner | Decided where |
|---|---|---|---|---|
| Private-space sync | One person's devices | Years | Always full: it is the owner's own vault | This ADR |
| Shared-space sync | Several people, their devices | Months to years | By policy: none, snapshot or full (§14.3) | This ADR |
| Scheduling negotiation | Two to a few people | Minutes to days | None | Calendar §23.6 and ADR-012: one ephemeral MLS group per negotiation |

Evidence in the repository:

- `fabric-wire` runs Noise IK sessions (`Noise_IK_25519_ChaChaPoly_BLAKE2s`) through a relay that stores frames in random mailboxes and logs only mailbox, size and time. The calendar demo's acceptance test A5 searches the relay capture for fixture markers and finds none. That is the working model of "the relay sees ciphertext plus traffic metadata", for a live pairwise session.
- `fabric-token` holds Ed25519 identity keys, X25519 Noise static keys, deterministic CBOR and COSE_Sign1 with domain-separating external data (`fabric-message-v1`, `fabric-consent-v1`). ADR-007 adds `fabric-operation-v1`.
- The paracels prototype stores payloads tagged `plain/1` and attachments as opaque blobs with a size and SHA-256, so an encrypted format can replace them without a schema change (paracels §11).
- The calendar doc's actor-by-actor disclosure contract (§23.5) is the template for the leakage table this ADR must publish (PRD §18: "cryptographic overclaim" risk).

Quality attributes served, in order: confidentiality of content against the relay and any backup store (§14.1); verifiable membership, so that who can read a space is a cryptographic fact, not a server's claim; forward secrecy and post-compromise security where members can change; and honest, measurable metadata disclosure. Not served, and said so: anonymity, protection of an unlocked endpoint, and recall of plaintext already delivered (§14.5).

Constraints: reviewed libraries only, no invented cryptography (PRD §5.3). Keys and nonces are never derived from content hashes, and relay identifiers are keyed or random, never content hashes (§9.3, §14.2). Device keys are generated independently of the vault root. Whether a paid or hosted relay is acceptable (PRD §2.2 items 5 and 13) is still the user's decision and must not change the format.

## Decision

**Content is encrypted under space epoch keys with the operation header as authenticated data. Where the epoch comes from depends on who is in the space: a private space draws a random epoch and wraps it to each enrolled device; a shared space takes its epoch from an MLS group that exists only to agree on membership and keys. Operations and blobs are never sent as MLS application messages. The relay stores opaque, expiring ciphertext in random feeds and learns only what the leakage table says.**

### D1. One content format, keyed by a space epoch

- Every operation payload and every blob chunk is encrypted with XChaCha20-Poly1305 under a content key derived from the space's current epoch key by a context-separated KDF (`fabric/op/v1`, `fabric/blob/v1`). The random nonce is stored with the ciphertext.
- The ADR-007 envelope splits into an authenticated header and an encrypted body. The header is the associated data and holds `operation_id`, `space_id`, `key_epoch` and `schema_version`. The body carries `device_id`, `device_sequence`, `previous_device_hash`, `hybrid_logical_clock`, `operation_type`, the idempotency and causation fields, `payload_hash` and the payload. The plaintext hash is inside on purpose, so the relay cannot match a ciphertext to a known plaintext. The signature covers header and body.
- A blob keeps the §14.2 rule: a random per-blob data key, wrapped under the space epoch key, with immutable manifest fields as associated data and fixed-size independently authenticated chunks.
- The payload format tag that paracels introduced (`plain/1`) becomes `e2ee/1` for this format. A reader refuses a tag it does not know.

### D2. Private spaces: random epochs wrapped to devices

- A private space's epoch key is random (§14.2). It is wrapped to each enrolled device's X25519 key-agreement public key with HPKE (RFC 9180, base mode) and the wrap is signed by the enrolling device, so a device cannot be added silently.
- Enrollment is an operation (ADR-007) signed by an already enrolled device, carrying the new device's signing and key-agreement public keys and the wrapped epoch. Removal is an operation that starts a new epoch and rewraps it only to the remaining devices.
- There is no MLS group for a private space. The members are one person's devices, membership changes are rare and always decided by that person, and MLS's forward secrecy per message is wrong for a vault whose owner must always be able to re-read everything.

### D3. Shared spaces: MLS for membership and epochs, nothing else

- A shared space has one long-lived MLS group (RFC 9420). Every member device is a leaf. Add, remove and update commits are the only way membership changes, so the group's ratchet tree is the authoritative membership list and the relay cannot forge it.
- The space epoch key is the MLS exporter secret of the current MLS epoch, with the label `fabric-space-epoch-v1`. A membership change is therefore automatically a new content epoch.
- Operations and blobs are **not** MLS application messages. MLS deletes its message keys after use for forward secrecy, which would make history unreadable by design; a space's history policy (§14.3) must be able to say "full" and mean it. Only commits, proposals and welcomes travel as MLS messages.
- Late join follows the space's policy. *None*: the joiner receives only the current exporter and reads from the next operation on. *Snapshot* or *full*: an existing member sends the joiner a history key bundle, the earlier epoch keys the policy covers, encrypted to the joiner inside an MLS application message, which is the one legitimate use of application messages in a space.
- MLS credentials are basic credentials whose identity is the thumbprint of the account's Ed25519 identity key; the leaf signature key is the device's key, bound to the account by a device certificate the account key signs (expiry and revocation epoch inside). Pairing, TOFU pinning and key-change warnings stay with ADR-012 and calendar §23.5; this ADR consumes a pinned key, it does not establish one.
- Cipher suite: `MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519`, pinned; no negotiation downward. Implementation: OpenMLS, behind one trait so it can be replaced.

### D4. The relay stores opaque expiring ciphertext in random feeds

- A space has one relay feed identified by a random 256-bit token that members derive from the space epoch key (`fabric/feed/v1`), so a membership change rotates the feed token and a removed member cannot even find the new feed.
- A relay message is `{feed, relay_message_id, size_bucket, expires_at, ciphertext}`. `relay_message_id` is a keyed hash of `operation_id` under a feed-derived key, so honest retries deduplicate on the relay and the relay learns equality within a feed and nothing else. The relay assigns a receive sequence per feed, which clients use as the catch-up cursor that ADR-007 D4 describes.
- Ciphertext is padded to size buckets; the relay keeps a message until every enrolled device has fetched it or `expires_at` passes, whichever is first, and never longer than the space's retention setting.
- Transport to the relay is TLS. The demo's Noise IK sessions remain the model for a live point-to-point session (negotiation, pairing) and are not used for feeds, because a feed must work when the sender is offline.
- Device identity, sequence numbers, hash chain, operation type and HLC are inside the ciphertext. The relay cannot count devices from the format, only from connections.

### D5. Revocation rotates the future and says so

- Removing a device or a member starts a new epoch (D2, D3). Everything after it is unreadable to the removed party.
- Everything before it is not revoked and the interface says so, in the words of SHR-004: a removed device or person may hold plaintext and older epoch keys. "Revoke" in Fabric means "stop future access", and purge (§14.5) means crypto-erasure of local wraps, not recall.
- A device suspected compromised is removed from every space it was enrolled in, in one operation per space; the account identity key is rotated only if the compromised device held it, which a trusted-host design (calendar demo D14) avoids.
- A key change observed for a contact invalidates that contact's active grants and responses until the user accepts it (calendar §23.5). The same rule applies to a member's leaf key in a shared space.

### D6. What each actor learns, published and tested

| Actor | May learn | Must not learn | Enforced by |
|---|---|---|---|
| Relay | Feed token, keyed message ID, size bucket, expiry, receive sequence, timing and frequency, connection metadata (IP, TLS fingerprint), rough device count from connections | Space ID, who the members are, device IDs, operation types, content, whether two feeds belong to one person | D1, D4; a test that scans a relay capture for fixture markers (as the demo's A5) |
| Current member device | Everything in the space from the point its history policy allows | Other spaces of the same members; another member's private space | D2, D3 |
| Removed member or device | Everything up to the epoch it was removed in, if it kept it | Anything after | D5; a property test that a removed leaf cannot derive the next exporter |
| Backup repository | Ciphertext, manifest sizes, snapshot times | Content, keys | §14.4; backups are of the encrypted vault, never of epoch keys in the clear |
| Calendar or mail provider | Whatever its owner projects there | Nothing from this ADR changes that; E2EE between Fabric endpoints does not hide provider-stored fields from the provider | Calendar §23.5 |

The table is part of the product's documentation (PRD §18, "cryptographic overclaim"), not only of this record.

## Options Considered

### Option A: Status quo, Noise IK pairwise sessions for everything

| Dimension | Assessment |
|---|---|
| Complexity | Low; built and tested in `fabric-wire` |
| Cost | A session per pair of devices, both reachable |
| Scalability | Pairs, not groups; no at-rest format |
| Team familiarity | High |

**Pros:** proven in the demo; strong for a live point-to-point exchange.
**Cons:** Noise provides a session, not a stored format: an operation at rest in a feed is not a Noise message. Group membership would be reinvented as N² pairwise sessions, and an offline member cannot complete a handshake. Kept for live sessions, rejected for feeds and spaces.

### Option B: Envelope encryption everywhere, including shared spaces

| Dimension | Assessment |
|---|---|
| Complexity | Low |
| Cost | A rewrap per membership change |
| Scalability | Fine for devices; weak for people |
| Team familiarity | High |

**Pros:** one mechanism; simplest code.
**Cons:** no post-compromise security and no membership proof: whoever holds the epoch key is a member, and the relay or any member can add a member by handing over a wrap. The membership and key-rotation logic MLS already provides, with proofs, would be rebuilt ad hoc. Chosen for private spaces only (D2), where the single owner is the only authority.

### Option C: MLS everywhere, operations as MLS application messages

| Dimension | Assessment |
|---|---|
| Complexity | Medium |
| Cost | MLS state per space including private ones; blobs do not fit messages |
| Scalability | History cannot be re-served after keys are deleted |
| Team familiarity | Low to medium |

**Pros:** forward secrecy and PCS per message; one protocol.
**Cons:** forward secrecy is the problem: a vault must re-read its own history and a space with a "full" history policy must serve it to a late joiner, which MLS message keys forbid by design. Large blobs would need chunking into messages. One's own devices do not need a group protocol. Rejected.

### Option D: Hybrid, MLS for shared membership and epochs, epoch-keyed envelopes for content (chosen)

| Dimension | Assessment |
|---|---|
| Complexity | Medium: two epoch sources behind one content format |
| Cost | One MLS group per shared space; one wrap per device per private epoch |
| Scalability | Content cost is one AEAD per operation and per chunk, whatever the group size |
| Team familiarity | Medium; AEAD, HPKE and COSE are already in use, MLS is new |

**Pros:** membership is cryptographically verifiable in shared spaces; history policy is honoured; content format is the same for both space kinds; the relay sees one shape; removed members lose the future and the feed.
**Cons:** no per-message forward secrecy for content (an epoch key compromise exposes that epoch); two enrollment paths to maintain; OpenMLS is a new dependency to review and pin.

## Trade-off Analysis

The decisive trade is per-message forward secrecy against readable history. Fabric is a vault, not a messenger: the owner must always re-read everything, and a shared space may promise full history to a new member. That rules out content as MLS messages and makes the epoch the unit of secrecy. The cost is stated plainly: compromise of an epoch key exposes that epoch, and the mitigation is to rotate epochs on membership change and on a schedule, not per message.

Between B and D for shared spaces, the question is whether membership needs proofs. SHR-002's acceptance criterion is an independent security review of "per-space keys and device membership". A reviewer can check an MLS ratchet tree; they cannot check an ad hoc list of who was handed a key. That is what D buys, and the reason private spaces, where the only authority is the owner's own devices, do not pay for it.

What this ADR leaves to others, on purpose: how two people first exchange and pin keys (ADR-012); the relay's deployment, retention and whether it is paid (PRD §2.2); and whether the paracels relay becomes the feed relay (ADR-014). Those decisions pick a relay and a ceremony; they do not alter the content format, the feed shape or the leakage table.

## Consequences

- **Easier:** the paracels chat becomes a shared space by replacing `plain/1` with `e2ee/1` and the room key with MLS membership, as its §11 seams intended. Backups and the relay handle one ciphertext shape. Revocation has a one-sentence meaning the interface can show. Security review has a checkable membership structure.
- **Harder:** two enrollment flows (device into a private space, member into a shared space) must be built and kept consistent. A late joiner with a "full" policy depends on an existing member being online to send the history key bundle. Size bucketing and padding cost bandwidth on small operations. OpenMLS state must be persisted transactionally with rollback detection (calendar §23.6).
- **Operational burden:** a scheduled epoch rotation per space; monitoring that every enrolled device has fetched a feed before the relay expires it; a published leakage table that must be re-verified whenever the relay message shape changes.
- **Revisit when:** ADR-012 needs a different credential shape in MLS; OpenMLS cannot be pinned to one reviewed release; measured padding overhead exceeds what the retention and bandwidth budget allows; PRD §2.2 chooses a hosted relay whose retention rules conflict with D4; a metadata-reduction profile (Oblivious HTTP, calendar §23.6) is adopted and changes what the relay sees.

## Assumptions and evidence

| Assumption | Evidence | Residual uncertainty |
|---|---|---|
| A relay that stores ciphertext in random mailboxes leaks only size, timing and routing | `fabric-demo` relay and `fabric-wire` frames; acceptance test A5 finds no plaintext markers in a capture | The demo relay was local and single-user; connection metadata at an internet relay was not measured |
| XChaCha20-Poly1305 with random nonces and AEAD associated data is adequate for operations and chunks | Architecture §14.2 and libsodium guidance; the same AEAD family is inside the Noise suite already used | Chunk-level nonce discipline has no test yet |
| HPKE base mode plus a signature is enough to wrap an epoch to a device | Calendar §23.6 applies the same pattern for responses; RFC 9180 | No implementation in the repository yet |
| The MLS exporter is a sound source for a space epoch key | RFC 9420 §8.5 defines the exporter for exactly this | Label and context strings are proposed here, not reviewed |
| A tagged payload format can replace `plain/1` without a schema change | Paracels §11 and its server, which stores payloads verbatim and reads them only in the policy module | The policy module then checks only sizes; any content policy moves to the client |

## Executable manifestations

1. A `fabric-space-crypto` crate (or `fabric_core::space` module): the `e2ee/1` content format with header as associated data, the context-separated key derivation, blob chunk encryption, and HPKE wrapping of private-space epochs. Property tests: any header change fails authentication; a ciphertext never equals another for the same plaintext; a removed device's wrap cannot open a new epoch.
2. A `SpaceMembership` trait with two implementations: `PrivateDevices` (D2) and `MlsGroup` over OpenMLS (D3). Tests: a removed leaf cannot derive the next exporter; a late joiner under policy *none* cannot read earlier operations and under *full* can.
3. The relay message schema (D4) in `fabric-wire` next to the demo frames, with a capture scanner test inherited from the demo's A5.
4. The leakage table (D6) kept as a document in `docs/` with a test that the relay message schema has no field outside the table's "may learn" column.
5. A fitness function: no module outside the space-crypto crate may call an AEAD or HPKE primitive directly.

## Action Items

1. [ ] Approve or amend D1 to D6; record the outcome and bump this ADR to Accepted.
2. [ ] Review the proposed KDF labels and MLS exporter label with the security reviewer named for SHR-002.
3. [ ] Pin OpenMLS and its cipher suite; record the audit status of the pinned release.
4. [ ] Implement item 1 and item 2's `PrivateDevices` for ADR-007 stage S1; defer `MlsGroup` to stage S2.
5. [ ] Draft ADR-012 against D3's credential shape, and ADR-014 against D4's relay message.
6. [ ] Add the D6 table to the architecture document §14 once accepted, replacing the sentence that the leakage "must be documented".
