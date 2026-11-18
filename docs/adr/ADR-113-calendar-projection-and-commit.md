# ADR-113: Calendar projection and commit

**Status:** Proposed
**Date:** 2026-11-17
**Deciders:** the owner
**Owner:** the owner
**Related:** [Technical architecture](../architecture.md) §19 item 13 · [Calendar Desktop plan](../plans/calendar-desktop-plan.md) §1, §2.3, §7, §8, §9.2, §10.3, §11, §15.1 (`CD-Q05a`, `CD-Q05b`, `CD-Q11e`, `CD-Q11f`, `G0-INVITE`) · [CD-W05 design](../plans/calendar-desktop-w05-design.md) PD13 to PD20, PD23, PD28, PD29, PD32, PD35, sections 5.3, 6.6 to 6.12, 7, 9 · [ADR-108](ADR-108-agent-and-mcp-permission-model.md) D6, D7, D9 (Proposed) · [ADR-111](ADR-111-calendar-authority-and-time-model.md) (Proposed) · [ADR-115](ADR-115-broker-ledger-envelope.md) (Accepted) · [Calendar specification](../calendar.md) §23.10, §23.11

This record is a proposal. It is not accepted. It settles the broker's part of ADR-113 before `CD-G5`, as architecture §19 asks; the Rendezvous part stays parked. Work package `CD-W05` builds to it as written. Merging the pull request that carries this file is the owner's acceptance. Before the merge it is his chance to change it.

## Context

Architecture §19 reserves ADR-113 for "Fabric-only/private-busy/native-invite modes, idempotency, holds, compensation, and reconciliation". The calendar specification (§23.10, §23.11) describes a multi-party commit: each participant's device prepares, holds the slot locally for five minutes, writes its owner's projection, and compensates or repairs a partial commit.

The broker has one owner, one Mac and no peers. An agent proposes one change; the broker proves it, asks the owner where the rules say so, and makes one Google call.

The G0 decisions fix the frame: invitations are a compatibility mode outside private scheduling (`G0-INVITE`, ADR-108 D6); deletion only of Fabric-created events, with `If-Match` and bound `sendUpdates`, reconciled by reading back (`CD-Q05a`); 30 s per Google call, at most three transient retries, a 120 s budget, a fresh proof per retry, no blind resend of a notification (`CD-Q11e`); a 60 s authorization (`CD-Q11f`). The plan excludes undoing sent invitations (plan §1).

The `CD-W05` design (Proposed; critiqued by three independent readers before anything was built, design 18.4) settles the remaining choices. This record cites its decision ids and sections rather than copying its tables. The plan and the G0 decisions win where they differ.

Nothing here has been checked against the real Google. Phase 1 tests the adapter only against a fake server written from Google's documentation (design 14.3). Live behaviour, `If-Match` on PATCH among it, is checked in the gate run (design 15.3).

## Decision

**The broker writes real Google events and real Google invitations, as approved. Each approved change becomes at most one changing request per order, conditional on the approved revision where the event exists. An unknown outcome is never resent; it is settled by reading. There are no holds and no compensation.**

### D1. Projection mode: native invite only

- The broker writes ordinary Google events in calendars the owner selected, and real Google invitations. This is the calendar specification's `native_invite` profile, used as the compatibility mode of `G0-INVITE` (calendar specification §23.11; plan §10.3).
- Google and every recipient see the approved title, time, place, description and attendees. **This is not private scheduling**, and nothing in the product may call it that (plan §2.3, §10.3).
- The `fabric_only`, `opaque_busy` and `local_details` profiles of the calendar specification are parked with Rendezvous.
- A create carries no attendees (design PD16). Attendees are added only by an approved invitation change.

### D2. Before the call

- Audit and authorization come before every Google call: the request, its proof, the 60 s exact-action authorization and the effect reservation commit locally first; if that fails, nothing is sent (plan §8 steps 1 to 7, §11).
- Every Calendar call names the proof that permits it: an agent's fresh permit, or a system proof for the broker's own reads (observation, calendar list, reconciliation) (design PD29; plan §1).
- The handoff to the provider worker is the dispatch point. Authority is checked again at each handoff, retries and later pages included (plan §8).

### D3. The request

- **One changing request per order.** A dispatch order makes at most one POST, PATCH or DELETE. The adapter never repeats it; every repeat is a new order from the core with a fresh proof (design PD13).
- **A create** uses the event id the core reserved and stored before the first call, and `sendUpdates=none`. A retry uses the same id (design PD16; plan §11).
- **A change of an existing event** (field edit, invitation, answer, delete) always carries `If-Match` with the one revision the approval names, never a wildcard: Google lets `If-Match: *` force a change through, so a revision that is not one quoted entity tag makes no request (design PD14, PD32; plan §10.3, `CD-Q05a`).
- **`sendUpdates` is always written** (`all`, `externalOnly` or `none`) and is the value the owner approved (plan §10.3). The broker promises which value it sends, not what Google then does: `none` is not a guarantee against email (plan §10.3; design RK4).
- **PATCH with only the approved members**, so a field the adapter does not model cannot be dropped (design PD15; plan §10.3 "preserve untouched fields"). If the live run shows that Google ignores `If-Match` on PATCH, the fallback is a read followed by a full update with `If-Match` (design RK2, OQ11).
- An answer to an invitation sends the attendee list exactly as read, with only the account's own answer changed, so no attendee can be dropped. An answer to one occurrence names the instance id and checks the original start and revision together ([ADR-111](ADR-111-calendar-authority-and-time-model.md) D6, D7; design 6.7).

### D4. Outcome classes

| Class | Meaning |
|---|---|
| Applied | Google answered that the change is in effect. |
| Transient | It certainly did not happen: nothing was sent, or Google refused before acting (401, a rate limit). May be retried. |
| Rejected | Google refused it for a reason a retry will not change. Carries a contract failure code. |
| NoAnswer | The request was sent and the answer does not say plainly whether the change was made: no complete readable answer, or a 5xx. |

Only NoAnswer is an unknown outcome (`outcome_unknown`). For a read, an unanswered request and a 5xx are Transient (design PD17, PD18, 6.10).

### D5. Retries

- The retry rule is `CD-Q11e`: at most three retries, waits of 2, 4 and 8 s or Google's wait hint if longer, the 120 s budget wins, a fresh proof per retry (design 6.11).
- **A retry follows only attempts that certainly changed nothing** (design PD17). The core proves each retry afresh on a new observation of the event; a retry after an attempt that was in fact applied would be judged against the changed event and end as a conflict for a change that was made. So a doubtful answer, a 5xx among them, is never retried; it goes to D6.
- A notification cannot be sent twice. A change whose outcome is in doubt is never sent again. And the only requests that can notify are the PATCH and DELETE of an existing event; each is conditional on the approved revision; an applied send changed the revision; so a second send would meet a changed revision and not be applied (design 6.11). The second half holds only if Google honours `If-Match` on PATCH and changes the revision with every change (design FA1, FA16, LV3).

### D6. Recovery from an unknown outcome

- An unknown outcome is never resent. The broker reads and decides (plan §11; design 7).
- The read is the broker's own action under its own proven authority (`System_reconcile`), even when the agent's grant was revoked in the meantime (plan §11; design PD29).
- The first read waits at least 10 s after the attempt ended (design 7).
- Verdicts: `Applied` (the change is there, exactly as approved), `Absent` (it is not), `ReadAgain` (not there yet), `NeedsOwner` (Fabric cannot tell: the event changed in another way, is gone, or exists under a create's id and is not what was approved). They move the request `reconciling -> applied`, `-> absence_confirmed` or `-> needs_owner_resolution` (plan §11 state grammar; design 7).
- A create counts as Fabric's only when Google answered Fabric's own request with it, or when the read finds under the reserved id exactly what was approved.
- **Two reads before absence, for every tool.** A change is absent only when two reads at least 60 s apart both show that: Google may finish a request after the broker stopped waiting, and does not promise to see a new id at once (design PD20).
- `absence_confirmed` ends the request. The agent may ask again; that is a new request with a new proof and, for a create, a new reserved id.
- A read that keeps failing past the retry budget moves the request to `needs_owner_resolution` (design 7).
- **Notification uncertainty is shown, not repaired.** When a change with `sendUpdates` other than `none` is found applied through this path, whether Google sent the messages is unknown. The owner sees that; nothing is sent again (plan §11; design 7).
- No second effect is made to make recovery look successful (plan §11).

### D7. Reconciled

`applied -> reconciled` means the broker holds Google's statement of the event after the change. For a create or a PATCH whose answer matches the approved change, the answer is that statement. For a delete, or an answer that does not match, the broker makes one reconcile read and records what it finds, matching or not. A delete is confirmed by reading the event back (`CD-Q05a`; design 7).

### D8. No holds, no compensation, no undo

- The broker makes no hold, local or at Google. A hold reserves a slot among several participants while they agree; the broker has one owner and writes at once after approval. A conflicting change by someone else is caught by `If-Match` and shown as a conflict, never overwritten (design RK9).
- The broker makes no compensation. A compensating delete or edit would be a second user effect, which plan §11 forbids for recovery, and it could notify the attendees again.
- A sent invitation is not undone (plan §1). An owner who wants it withdrawn makes a new request, with its own approval.

### D9. Held read results

A read's result goes from the adapter to the core and nowhere else. The core holds it in the encrypted store, outside the ledger's payload. It is released only in the outcome of a `fabric_request_result` request whose proof on `Disclose` checks the authority again at that moment. It is purged on release of a one-request read, on disconnect, and after 24 h (design PD28, 9; plan §9.2). Reads are bounded and never cut silently (design PD23).

## Options considered

- **Fabric-only or opaque-busy projection** (calendar specification §23.11; system design §10.3). Not chosen: `CD-C07` requires edits of existing calendars and real invitations; plan §2.3 already sets the busy pilot aside.
- **Unconditional changes with a read before.** Not chosen: a change between the read and the write would be overwritten, and a retried PATCH could notify twice (design PD14).
- **Get-then-PUT for every change.** Not chosen while PATCH honours `If-Match`: PUT replaces the whole event and may drop fields the adapter does not model (design PD15). It stays the fallback.
- **Resend after a timeout.** Not chosen: a create would rely on the reserved id alone, and a notifying change could send twice (plan §11; `CD-Q11e`).
- **Retry after a 5xx, and read before judging the retry's answer.** The design's first draft. Not chosen: Google may have made the change before it failed, and the core's fresh proof for the retry would then see a changed event and refuse (design PD17, 18.4).
- **One read to confirm absence.** Not chosen: Google may not yet show what exists, and may still finish the request (design PD20, RK3).
- **A read after every change.** Possible: it costs one proof and one call more each. The design recommends using the answer when it matches (design 7); the lead may choose the read.

## Consequences

- An approved change either takes effect once, is refused with a reason, or ends in an owner-visible state. It is never reported as done when it is not.
- A change made twice after `absence_confirmed` is made unlikely, not impossible: Google gives no bound on when a change becomes visible (design RK3). `needs_owner_resolution` covers what is left.
- A failure at Google (a 5xx) ends a request in reconciliation, about seventy seconds later, not in a quick retry; under a row that asks, the owner approves again (design RK15).
- Whether `none` and `externalOnly` send mail is Google's behaviour; the owner is told what was sent, not what was received (design RK4).
- An event changed by someone between the observation and the dispatch makes the approval a conflict; the owner approves again (plan §10.3).
- The structural guarantee against double notification depends on `If-Match` for PATCH. Google's performance guide says a PATCH must carry the current ETag in `If-Match`; the 412 answer is stated only for update and delete. Until the live run (design LV3) it is an assumption.
- Conflicts with the baseline, kept as the plan decides:
  - Architecture §14.1 (threat "Calendar-provider disclosure") says "Fabric-only or opaque private projection by default"; calendar specification §23.11 proposes `opaque_busy` as the compatibility default; system design §10.3 writes an opaque `Busy` event to a pilot calendar. The broker writes `native_invite` only (`G0-INVITE`; plan §2.3).
  - Calendar specification §23.10 and architecture §14.1 allow compensation by removing an already-created projection; system design §11 makes compensation an explicit effect. The broker has none (plan §1, §11).
  - System design §11 and §12.1 let an effect whose absence is proven retry the same effect (`Reconciling -> Pending`). In the broker `absence_confirmed` ends the request (plan §11 state grammar; design 7).
  - System design §8.3 retries with jitter; the broker uses `CD-Q11e`'s fixed backoff and budget.

## What is not decided here

- The Rendezvous commit saga: prepare, per-owner projection, partial commits across participants (calendar specification §23.10). Parked with ADR-112.
- Holds and compensation for multi-party scheduling, and the `fabric_only`, `opaque_busy` and `local_details` profiles.
- The busy-block pilot of the older system design (§10.3, slice E) and calendar specification §15.8.
- Whether every change is followed by a reconcile read (D7).
- How long an unfetched read result is held (design OQ5; default 24 h).
- The store's form of effects, observations and held results (design ST3 to ST5), which the lead's store design decides.

## Revisit when

- The live run shows that PATCH ignores `If-Match`, that a reused id is not refused, or that a new event is invisible for long (design LV3, LV4).
- Rendezvous is scheduled: its saga is written then.
- A tool needs a change that can notify without an existing revision to bind to.

## Action items

1. [ ] The owner accepts this record by merging the `CD-W05` pull request, or asks for changes before the merge.
2. [ ] `CD-W05` phase 1 builds D3 to D6 in `crates/fabric-google-calendar` as plain functions, tested against the fake (design 15.1, g28 to g45).
3. [ ] Phase 2: the core runs D2, D6, D7 and D9 (design CO5, CO6, CO8); the theory gains the system actions (design TH1, TH2).
4. [ ] The contract shows notification uncertainty in a request's details (design V3-4).
5. [ ] The live run checks D3 and D6 against the real Google (design LV3, LV4, LV7).
6. [ ] On acceptance, set this record to Accepted and update architecture §19 item 13 and the roadmap row for ADR-113.
