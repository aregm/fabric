# ADR-014: Mobile client and messaging relay

**Status:** Proposed
**Date:** 2026-10-26
**Deciders:** Areg Melik-Adamyan
**Owner:** Areg Melik-Adamyan
**Related:** [Technical architecture](../architecture.md) §9.1, §9.3, §9.6, §14.3 · [PRD](../PRD.md) §2.1, §2.2 items 4 and 13, §8.1, §8.3 · [Paracels chat prototype](../paracels-chat.md) §8, §11, §13, §15 · [ADR-007](ADR-007-operation-log-and-sync-evolution.md) (envelope, stages, relay as cursor) · [ADR-009](ADR-009-sharing-crypto-and-metadata-model.md) (content format, relay message, leakage table) · ADR-012 (Rendezvous, open)

## Context

The paracels chat prototype is complete: an Expo (React Native) iOS app, Expo Router API routes for history and attachments, a Node WebSocket server that appends messages to SQLite, and an on-device outbox. Every gate from G0 to G6 is closed, 111 automated tests pass, and the acceptance criteria A1 to A9 were met on the iPhone 18 Pro simulator and an iPhone 16 Pro. Architecture §9.6 says the prototype exists "to exercise the message lifecycle and to fix the seams where Fabric's encryption, policy and local-first storage will attach", and that whether it graduates is this record's decision, together with whether the relay moves into the Rust core.

The prototype did not meet invariant 2 (local interactive path): reading history needs the server. It has no TLS, no end-to-end encryption, self-asserted identities and a shared room key (paracels §13). Those were accepted limits of a dummy, not findings. What it did establish:

- The message lifecycle (paracels §8): four statuses, exactly-once effect through a device-created ID kept across retries, oldest-first sending per device, a 10-second acknowledgement timeout, retry with the same ID, catch-up from a server sequence after every reconnect. Gates G4 and G5 proved it through airplane mode, force-quit and a server restart.
- A code shape that separates protocol, client logic, server logic and UI, with a lint rule that keeps server code out of the app (paracels §5, D30).
- Which Expo pieces are worth keeping: the pickers, the documents-directory copy of an attachment before upload, Quick Look, `expo-sqlite` for the outbox, and a list that is not inverted on iOS 26.
- Which pieces exist only because of the dev server: Expo Router API routes (they cannot accept WebSockets and run only under `npx expo start`), the Node socket process, `node:sqlite`, and the whole `src/chat/server/` tree.

Since the prototype was built, ADR-007 fixed the operation envelope and the three sync stages, with "the relay is a cursor, never an authority" taken directly from the prototype's lesson. ADR-009 fixed the content format (`e2ee/1`), the relay message (`{feed, relay_message_id, size_bucket, expires_at, ciphertext}`), the leakage table, and the rule that the paracels payload tag and room key are the seams where encryption and membership attach. A Rust relay already exists: `fabric-demo`'s relay (axum) stores frames in random 128-bit mailboxes with a per-mailbox receive sequence, a retention window, long-polling, loopback-only listening and an audit log that records only mailbox, size and time. It was built for the calendar demo's Noise sessions, but its shape is the ADR-009 feed.

Quality attributes served: the local interactive path on mobile (invariant 2); one implementation of domain, policy, crypto and transactions, in the trusted Rust core (§9.1, §9.3: "keep domain/core outside UI framework"); the UI never holding vault keys (§9.1); and continuity, so the validated lifecycle and the gates are kept as the regression contract rather than rebuilt.

Constraints: macOS and iOS first (PRD §2.1); no startup dependency on network or sync (PRD §8.1); localhost services bind loopback only unless authenticated (PRD §8.3); whether a home server or hosted relay is acceptable is still open (PRD §2.2 items 4 and 13) and must not change the client; Android, web, push delivery and release builds were out of the prototype's scope (paracels §18) and remain so here.

## Decision

**The prototype graduates in parts. Its Expo app stays as the mobile UI shell, its lifecycle and acceptance criteria become the contract, and its TypeScript domain logic, servers and protocol do not graduate: the domain moves into the Rust core and runs on the device through a native module, the relay is a Rust binary grown from the calendar demo's relay, and a chat message becomes an operation in a shared space.**

### D1. Mobile client: Expo UI over the Rust core, in process

- `apps/paracels` remains the iOS client and keeps Expo Router, the screen components, the pickers, Quick Look and the theme. It is renamed when it stops being a dummy; the bundle ID and signing team (paracels D28) are kept until then.
- The Rust core runs inside the app as a native module generated with UniFFI's React Native bindings and wrapped as an Expo module, so a rebuild is `npx expo run:ios` as today. It owns everything that `src/chat/client/` owns now except presentation: the outbox and its worker, the device replica of the space, the ADR-007 envelope and chain verification, the ADR-009 content format and space keys, and the relay client. Keys live in the iOS Keychain through the core, never in JavaScript.
- The TypeScript that remains is the reducer that merges what the core reports into screen state, the hook that drives the screen, and the components. The rule "app code never imports `src/chat/server/`" becomes "app code never touches a key, a signature or a socket": only the core does.
- Tauri 2 mobile is not chosen now (Option C), because the Expo app already passes the gates and iOS integration (pickers, Keychain, Quick Look and later push) is what Expo modules are for. It is the alternative if the native-module spike fails (Executable manifestations, item 1).

### D2. Relay: a Rust binary, grown from the demo relay, speaking the ADR-009 message

- A `fabric-relay` crate replaces `server/chat-socket.ts` and the Expo API routes. Its starting point is `apps/fabric-demo/src/relay.rs`: random mailboxes become ADR-009 feeds, the per-mailbox sequence becomes the per-feed receive sequence that ADR-007 D4 uses as the catch-up cursor, the retention window becomes the space's retention setting, and the audit log keeps recording only feed, size and time.
- It adds what ADR-009 D4 requires and the demo did not need: deduplication by `relay_message_id`, size buckets, `expires_at`, deletion once every enrolled device has fetched a message, blob chunk storage, TLS, and push of new messages over a WebSocket in addition to long-polling. It stores ciphertext only; it has no policy module, because content policy moves to the client (ADR-009, assumptions table).
- It listens on loopback by default and on other interfaces only with TLS and a configured feed list, which satisfies PRD §8.3 and keeps the prototype's "servers on this Mac" setup working for development. Where it runs beyond this Mac is PRD §2.2 item 13 and does not change its code.
- The relay is not the Rust core and never links `vault.db`, SQLCipher or the policy engine. It is the "cloud/relay" row of the threat model (§14.1): compromised, it yields ciphertext and the leakage table's "may learn" column.

### D3. Protocol: a chat message is an operation; `paracels-chat/1` is retired

- A room becomes a shared space (ADR-009 D3); the room key is replaced by MLS membership; `main` becomes a space ID (paracels §11, invariant 8). A message is an operation of type `message.post` in the ADR-007 envelope, encrypted as `e2ee/1`. An attachment is a blob with ADR-009 chunks, referenced from the operation by its identifier.
- The device holds a replica of the space's operations, so history reads are local and invariant 2 is met. Catch-up uses the relay cursor and is verified by the device chains; a relay reset is a new relay identity, not a reason to drop local history (ADR-007 D4).
- The prototype's lifecycle (paracels §8) is kept whole as the stage S1 outbox: the four statuses, exactly-once by operation ID, oldest-first per device, the 10-second acknowledgement timeout, retry with the same ID, and the late-acknowledgement rule. The "ack" is now the relay's receipt with the receive sequence.
- Frames of `paracels-chat/1` and the `plain/1` payload are not versioned forward. They are deleted with the TypeScript server once step 1 of D4 is passed.

### D4. Graduation runs in three steps, each gated by the prototype's own criteria

| Step | What changes | What stays | Gate |
|---|---|---|---|
| 1. Relay swap | `fabric-relay` serves the existing `paracels-chat/1` frames, history pages and attachment routes behind the room key. The Node server and API routes are removed. | The app, unchanged | Paracels A1 to A9 pass against the Rust relay |
| 2. Core on device | The native module ships the outbox, envelope, chain and `e2ee/1` under an interim pre-shared epoch key (the room key, re-purposed); the relay stores ADR-009 messages. `src/chat/client/` shrinks to the reducer and hook. | The screens; the relay binary | A1 to A9 pass; a relay capture scan finds no plaintext (the demo's A5); the UI holds no key |
| 3. Membership | OpenMLS in the core; device enrollment; the pre-shared epoch is deleted; TLS on. The space is a real shared space. | Everything from step 2 | A1 to A9 pass with two accounts; a removed device cannot read the next epoch |

The pre-shared epoch in step 2 is an interim for one gate and is named as such in the code; it is not a fourth kind of space in ADR-009.

### D5. What is out of scope and stays out

Android and web builds, push notifications and background delivery, release builds and TestFlight, more than one attachment per message, and editing or deleting messages (paracels §18). Each is a later decision once the three steps are closed. Push delivery, when it comes, is a generic notification from the relay with no content (ADR-009, calendar §23.6).

## Options Considered

### Option A: Status quo, the prototype stays a throwaway

| Dimension | Assessment |
|---|---|
| Complexity | None |
| Cost | None now; the lifecycle and gates are rebuilt later |
| Scalability | Not applicable |
| Team familiarity | High |

**Pros:** honest; no dummy code reaches the product.
**Cons:** the validated lifecycle, the acceptance suite and the Expo integration work would be redone from memory. §9.6 built the prototype to fix seams; discarding it discards the seams. Rejected.

### Option B: Graduate as built, TypeScript end to end

| Dimension | Assessment |
|---|---|
| Complexity | Low at first |
| Cost | A second implementation of envelope, chain, AEAD, HPKE and MLS in TypeScript |
| Scalability | Node relay fine; Expo API routes are development-only |
| Team familiarity | High |

**Pros:** no FFI; the fastest path to a TLS-and-encrypted chat.
**Cons:** two implementations of the ADR-007 and ADR-009 crypto, one of them in the UI language where §9.1 says the UI never holds keys. Expo Router API routes cannot accept WebSockets and run only under the dev server, so the server half must be rewritten anyway. Rejected.

### Option C: Replace the client with Tauri 2 mobile

| Dimension | Assessment |
|---|---|
| Complexity | Medium to high |
| Cost | A new client; the Expo integration work is lost |
| Scalability | One shell for desktop and mobile |
| Team familiarity | Low on mobile |

**Pros:** the Rust core is in process by construction; one UI codebase with the desktop (§9.3).
**Cons:** the Expo app passes the gates today and the WebView route for pickers, Keychain and Quick Look is less direct than Expo modules. Tauri mobile is young relative to Expo. Kept as the fallback if the native-module spike fails or when the desktop shell exists and one UI codebase is measured to be worth it.

### Option D: Expo UI, Rust core in process, Rust relay, operations as protocol (chosen)

| Dimension | Assessment |
|---|---|
| Complexity | Medium: one FFI boundary, one relay binary, one protocol migration |
| Cost | A UniFFI React Native spike; growing the demo relay |
| Scalability | Content and crypto cost is the core's, the same on desktop and mobile |
| Team familiarity | Medium; Expo and the Rust crates are both in hand, the binding is new |

**Pros:** one implementation of the domain; keys never in JavaScript; the lifecycle and gates kept; the relay reuses tested code; invariant 2 met on mobile.
**Cons:** a native-module build step in the app; two languages in one app; the binding generator is a dependency to pin and watch.

## Trade-off Analysis

The decision is really about where the domain lives. Option B keeps everything in one language by putting crypto and the outbox in the UI, which §9.1 and §9.3 forbid for the desktop and which there is no reason to allow on mobile. Option C puts the core in process by changing shells, at the cost of the only client that passes the gates. Option D pays for one FFI boundary and gets both: the shell that works and the core that is supposed to own the domain.

The relay decision is easier. The TypeScript relay existed because Expo API routes were convenient during the prototype; it has no production path (no WebSockets in routes, dev server only). The Rust relay exists, is tested, and already has the ADR-009 shape. Moving the relay into Rust is therefore not a rewrite but a merge of two relays into one.

The protocol migration is the only part that cannot be done gradually in place, which is why D4 swaps the relay first, under the unchanged app and protocol, so that the first step is pure substitution with the gates as proof.

What this ADR leaves open, on purpose: how two accounts first meet and pin keys (ADR-012), where the relay runs beyond this Mac and whether it is paid (PRD §2.2 items 4 and 13), and whether the desktop later shares the mobile UI (Option C, revisited with measurements).

## Consequences

- **Easier:** the mobile client gets every future core feature (spaces, policy, replicas) without a TypeScript port; the leakage table applies to one relay; the acceptance suite from the prototype becomes the client's regression suite; the demo relay stops being demo-only code.
- **Harder:** every Expo rebuild compiles Rust for the simulator and device; the binding's generated API is a contract to version; debugging crosses a language boundary; the app's tests split into `node --test` for the reducer and `cargo test` for everything else.
- **Operational burden:** pinning UniFFI, its React Native bindings and OpenMLS; a relay to run (loopback for development, TLS beyond it); the relay's deletion-after-fetch rule needs every enrolled device to check in or messages expire.
- **Revisit when:** the native-module spike cannot meet the budget (item 1 below) or blocks an Expo SDK upgrade; Android is wanted before step 3 closes; the desktop shell exists and one UI codebase is measured to pay off (Option C); PRD §2.2 chooses a hosted relay with retention rules that conflict with deletion-after-fetch; push delivery is required, which adds an APNs path to the relay.

## Assumptions and evidence

| Assumption | Evidence | Residual uncertainty |
|---|---|---|
| The prototype's lifecycle holds under a verified-chain, encrypted feed | Paracels gates G4, G5 and G6: exactly-once, ordering and catch-up through airplane mode, force-quit and a relay restart | Tested with one room and two devices, plaintext, no chain verification |
| The demo relay is the right seed for `fabric-relay` | `apps/fabric-demo/src/relay.rs`: random mailboxes, per-mailbox sequence, retention, long-poll, audit log of mailbox, size and time; demo acceptance A5 | In-memory only, loopback only, no dedup, no TLS, built for Noise frames not feeds |
| A Rust core can run inside an Expo app with an acceptable build and call overhead | UniFFI React Native bindings exist as a project; Expo modules wrap native code | No spike has been run; build time and binding maturity on Expo SDK 58 are unknown |
| A chat message maps onto the ADR-007 envelope without a new operation kind | Paracels §6.1 fields (ID, room, sender, device time, server time, payload format, attachment) are all present in the envelope or the blob manifest | The 10-second acknowledgement timeout was tuned for a local WebSocket, not an internet relay |
| The Expo pieces kept are stable across SDKs | Paracels AGENTS.md and R10: every phase re-read the SDK 58 docs; pickers and `File.preview` worked on the device | Expo changes per SDK; the native module adds one more thing an upgrade can break |

## Executable manifestations

1. **Spike, before step 2:** a UniFFI React Native binding of one `fabric-core` function inside `apps/paracels`, built with `npx expo run:ios` for the simulator and the iPhone. Budget: rebuild under five minutes on this Mac, a call round trip under one millisecond, no change to the JavaScript debugging workflow. A failed budget reopens Option C.
2. **`fabric-relay` crate** in the workspace, with the demo relay's tests moved over and new ones for dedup by `relay_message_id`, expiry, deletion-after-fetch, size buckets and the capture scan.
3. **Conformance suite:** paracels A1 to A9 rewritten as integration tests that run against `fabric-relay` (step 1) and then against the core-on-device build (steps 2 and 3), so the gate is automated where a device is not needed.
4. **Fitness functions in the app:** the existing ESLint rule extended so `src/` may not import any crypto, WebSocket or SQLite package except through the generated binding; a test that the JavaScript bundle contains no key material markers.
5. **Workspace rule:** `fabric-relay` may depend on `fabric-wire` and `fabric-token` but never on the vault, policy or space-crypto crates, enforced by a dependency check in CI.

## Action Items

1. [ ] Approve or amend D1 to D5; record the outcome and bump this ADR to Accepted.
2. [ ] Run the native-module spike (item 1) and record the measurements here.
3. [ ] Create `fabric-relay` from the demo relay and pass step 1 of D4 with the unchanged app.
4. [ ] Implement the stage S1 outbox, envelope and `e2ee/1` in the core and pass step 2 with the interim epoch.
5. [ ] Add OpenMLS and device enrollment and pass step 3; delete the pre-shared epoch and the TypeScript server tree.
6. [ ] Update architecture §9.6 to describe the graduated client and relay, and the paracels spec's status line to point here.
