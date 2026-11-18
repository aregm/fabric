# ADR-111: Calendar authority and time model

**Status:** Proposed
**Date:** 2026-11-17
**Deciders:** the owner
**Owner:** the owner
**Related:** [Technical architecture](../architecture.md) §19 item 11 · [Calendar Desktop plan](../plans/calendar-desktop-plan.md) §1, §2.3, §7, §10.1 to §10.3, §13, §15.1 (`CD-Q05b`, `CD-Q05c`, `CD-Q05e`) · [CD-W05 design](../plans/calendar-desktop-w05-design.md) PD10, PD19, PD21, PD22, PD25, PD26, PD31, PD32, PD33, sections 5.3, 6.1, 6.2, 6.4, 6.5, 6.9, 8 · [ADR-108](ADR-108-agent-and-mcp-permission-model.md) D7 (Proposed) · [ADR-113](ADR-113-calendar-projection-and-commit.md) (Proposed) · [Calendar specification](../calendar.md) §10.6, §23.12

This record is a proposal. It is not accepted. It settles the broker's part of ADR-111 before `CD-G5`, as architecture §19 asks; the Rendezvous part stays parked. Work package `CD-W05` builds to it as written. Merging the pull request that carries this file is the owner's acceptance. Before the merge it is his chance to change it.

## Context

Architecture §19 reserves ADR-111 for "Fabric-native versus provider-authoritative events, recurrence/zone semantics, and provider bindings". The calendar specification (§10.6, §23.12) describes these for the whole Fabric Calendar: encrypted local replicas, Fabric-native events, a deterministic recurrence engine, and a `calendar_binding` that links a Fabric meeting to each owner's provider.

The Calendar Desktop plan builds something smaller first: a permission broker between AI agents and Google Calendar on one Mac (plan §1). It has no Fabric-native events and no peer protocol. Its plan maps the earlier "Google observation" slice to `CD-W05` with "sync deferred" (plan §13).

The G0 decisions fix the time rules: timed events only, each with an explicit IANA zone (`CD-Q05c`); recurring changes refused except a reply to one occurrence (`CD-Q05b`); the default event type only (`CD-Q05e`).

`CD-W05` (the Google provider) is the first package that meets real provider data, so its design settles the broker's answers. The design is Proposed; three independent readers critiqued it before anything was built (design 18.4). This record cites its decision ids and sections rather than copying its tables. The plan and the G0 decisions win where they differ from the design.

Nothing here has been checked against the real Google. The adapter of `CD-W05` phase 1 is tested only against a fake Google server written from Google's documentation (design 14.3). Live behaviour is checked in the gate run with the owner (design 15.3).

The baseline specifications say more than the broker does; the conflicts are named under Consequences.

## Decision

**For the broker, Google is the authority for what an event is, and Fabric is the authority for what it permitted and did. The broker keeps no calendar replica, names everything by provider ids it does not reinterpret, reads times as Google states them, and leaves recurrence to Google.**

### D1. Who is the authority for what

- Google is the authority for what an event is: its time, content, attendees, status and revision. Every event the broker knows is in this sense provider-authoritative (calendar specification §23.12, first row).
- Fabric keeps no replica of a calendar. It keeps only observations tied to requests, held read results, and its own records (design 8; plan §11 "provider observations"). An observation stores the event as Google returned it, as evidence (plan §10.3 "raw encrypted evidence").
- Fabric is the authority for grants, authorizations, effects, and the fact "Fabric created this event".
- "Fabric created this event" is the store's record of an applied create, keyed by account, calendar and event id (design PD19; plan §7 "ownership tracked locally"). No marker, private property or extended property is written into Google. A marker on a shared calendar could be forged by anyone who may write there.
- The adapter does not decide ownership. The rule theory decides it from the store's record (design 6.8).

### D2. Accounts

- An account is its Google subject (`sub` of the verified identity token). There is one account record per subject, for good. The verified address is required, so that the owner sees what connected (plan S01), but it is display only and never identity (design PD10; plan §10.1 "stable provider subject as identity").
- Reconnecting an account under another subject is `account_mismatch`. A new connect whose subject some account record already has, in any state, is refused the same way, so one Google account is never two accounts (design 3.7).
- The broker issues the account id (`account-<32 hex>`) at `connect_google` and keeps it with the subject (design 8).

### D3. Calendars

- The broker issues a calendar id `cal-<32 hex>` for each pair (account, Google calendar id), when a calendar list first shows the pair. It keeps the pair for the life of the account record, also while Google does not list the calendar (design PD21, 8).
- An agent and the page never see a Google calendar id. The core resolves the broker's id, under the account the action names, in the transaction that makes the order. An id of another account does not resolve (design 8; plan §10.2, §14).
- The same Google calendar reached through two accounts is two calendars with two ids. Tokens, grants, effects, observations and held results are namespaced per account (plan §10.1).
- No request uses Google's keyword `primary`. Every calendar is named by the id the calendar list returned (design PD22; plan S01: an ambiguous task "never silently uses `primary`").

### D4. Events, occurrences and revisions

- Event ids, instance ids and revisions are Google's, kept verbatim. The revision is the event's `etag` with its quotes, and is sent back verbatim in `If-Match` (design PD21, 6.9).
- For a create the broker reserves the event id before the first call (plan §11); [ADR-113](ADR-113-calendar-projection-and-commit.md) D3 covers its use.
- An occurrence of a recurring event is named by Google's instance id together with its original start (`originalStartTime`) (design 8; plan §10.3).
- The broker uses no iCalendar `UID` and no Fabric-side event id.
- An agent writes event ids and revisions, so both are checked before they reach a URL or a header: an event id is letters, digits and underscores, a revision is one quoted entity tag, never a wildcard. Anything else makes no request (design PD32, 5.3).

### D5. The time model

- **Timed events.** A time is an offset date-time with an explicit IANA zone (`event_time { date_time, zone }`). The broker writes it as given and creates or edits timed events only (`CD-Q05c`).
- **Times are read in the calendar's zone.** Every events read names the calendar's zone; Google then writes each time with that zone's offset, and the view pairs that offset with that zone, so the two agree. An event's own zone, which may differ, is not shown in a view; it stays in the stored evidence. `Z` becomes `+00:00` and a fraction of a second is dropped (design PD33, 6.9).
- **All-day events** are read as dates (`start_date`, `end_date`, the end exclusive as Google has it). They are never converted to an instant and never written (plan §10.3; `CD-Q05c`).
- **Windows** are half-open UTC intervals `[start, end)`. A read asks Google for every event that overlaps the window. Busy periods are converted to UTC with milliseconds and cut to the window (design 6.3, 6.4, 6.9).
- **No zone arithmetic in the adapter.** It parses RFC 3339 to an instant only to compare instants and to cut busy periods. Whether an offset agrees with its zone, and every warning that needs zone rules, belongs to the broker's checks in `CD-W09` (design 6.9, 1.3).

### D6. Recurrence

- The broker has no recurrence engine. It reads recurring events only as the occurrences Google expands (`singleEvents=true`) over a bounded window. A recurring master is never returned and never treated as one event (plan §10.3 "provider-expanded occurrences"; design 6.4).
- Recurring changes are refused: no creating a series, no series edit, no instance edit (plan §1; `CD-Q05b`).
- The one exception is an answer (accept or decline) to one occurrence of a recurring invitation. It is bound to the instance id, the original start and the revision together; if the instance's original start is not the approved instant, the answer is refused as a revision conflict (`CD-Q05b`; design 6.7).
- An occurrence's original start is a time, so it is disclosed to an agent only with the `time` field. An occurrence of an all-day series has a date as its original start, which the contract cannot hold; it is shown without its occurrence identity and cannot be answered one by one (design 6.4).

### D7. What the broker may change

The observation the broker makes before each proof on an action that names an event records whether the event is recurring, whether it is timed, whether it is of the default type, and whether it is a received invitation (design 6.5; plan §10.3 "distinguish owner-organized events, received invitations"). The rule theory refuses a change to an all-day event, a recurring event (except D6's answer), or an event of another type (`CD-Q05c`, `CD-Q05b`, `CD-Q05e`).

A received invitation exists only on the account's primary calendar. Google marks one attendee entry as `self`, and that mark names the calendar the copy is on, not who is signed in. On a calendar another person shared, the marked entry is that person; there the broker sees no invitation to answer and no own response, and counts that person among the attendees (design PD31, 6.1).

### D8. Access roles

| Google `accessRole` | Contract `calendar_access` | Writable |
|---|---|---|
| `owner` | `owner` | yes |
| `writer` | `writer` | yes |
| `writerWithoutPrivateAccess` | `reader`, until the contract names it | no |
| `reader` | `reader` | no |
| `freeBusyReader` | `free_busy_reader` | no |
| anything else | the calendar is left out | |

- The contract has four roles. Google's fifth role may write but sees private events without their details, exactly as `reader` does. Until schema version 3 adds it (design V3-1), the broker shows it as `reader` and does not write to it. An unknown role is never read upward (design PD26, 6.2; plan §2.3: "Never resolve a conflict by silently broadening a permission").
- A role is necessary, never sufficient: the broker's rules limit every call (plan §7). Write access to a shared calendar never implies ownership or a right to change sharing (plan §10.2).
- A calendar that disappears from the list, or whose role falls, is a reduction of authority: the grants that name it end and its waiting approvals go stale (plan §10.2; design CO7).

### D9. Texts the contract cannot hold

A title, place or description longer than the contract's bound is cut at the bound in a read. An attendee address the contract cannot hold is left out of a read, but makes an observation fail, so the check of who is notified never misses a recipient (design PD25).

## Options considered

- **A local replica with incremental sync** (calendar specification §15.8 step 2, §23.11; system design slice C). Not chosen for the broker: the plan defers sync (plan §13), and a replica adds stored calendar content, tombstones and checkpoint recovery that no broker tool needs. Every read is bounded and made for one request.
- **A recurrence engine in the broker** (calendar specification §10.6, §23.12). Not chosen: plan §10.3 allows provider-expanded occurrences, and the broker changes no series. A tested engine stays possible for `CD-W09` or for Rendezvous.
- **A Fabric marker inside the Google event** (a private extended property). Not chosen: it can be forged on a shared calendar and it leaks to anyone who can read the event (design PD19).
- **Google calendar ids as contract ids.** Not chosen: they hold `@` and other characters the contract's `id` does not allow, and one Google calendar seen through two accounts must be two calendars (design PD21).
- **The event's own zone in a view.** Not chosen: Google's offset follows the zone the request names, so the pair would contradict itself, and the broker refuses such a pair in a proposal (design PD33).
- **The fifth role shown as `writer`** (the design's earlier text). Not chosen: it would claim a view of private events the account does not have; the fifth role waits for a contract value of its own (design PD26).

## Consequences

- The broker cannot answer "what is on my calendar" offline. Every answer is a fresh, bounded read under a proof.
- Every change of an existing event costs an observation (a read and a proof) before its approval (design RK13).
- An all-day event can be seen but not changed through Fabric. A recurring series can be seen as occurrences but not changed, except D6's answer.
- Which all-day events overlap a UTC window is Google's decision, made in the calendar's zone. The broker does not recompute it (design 6.4).
- A calendar shared with the fifth role cannot be changed through Fabric until schema version 3 (design PD26). Plan `CD-C13` asks for writable shared calendars; the ordinary "make changes" share gives `writer` and is not affected.
- Long texts are cut in reads, and an agent may act on a shortened description (design RK8).
- Conflicts with the baseline, kept as the plan decides:
  - Calendar specification §23.1 (layer 1), §15.8 step 2 and §23.11 (Google adapter row: `syncToken`, full resync on `410`) describe an encrypted local replica with incremental sync; system design §10.1 and slice C do the same. The broker keeps none (plan §13, "sync deferred").
  - Calendar specification §10.6 and §23.12, and architecture §9.3 (the Calendar row: "deterministic local recurrence/availability core"), keep recurrence masters canonical and expand them locally. The broker uses Google's occurrences only (plan §10.3).
  - Calendar specification §10.6 names four time kinds (instant, zoned-local, floating-local, all-day) and a `calendar_binding` with an iCalendar `UID`. The broker uses two (timed with zone, all-day as dates) and Google's ids only (`CD-Q05c`).

## What is not decided here

- The Rendezvous part: Fabric-native events and their authority, `calendar_binding` across owners, the private scheduling protocol (architecture §19 item 12, ADR-112). Parked.
- A persisted replica and incremental sync.
- A recurrence engine, and zone checks and warnings (`CD-W09`).
- Google People import (`CD-Q06`), a later package.
- The contract's fifth role (design V3-1).

## Revisit when

- Rendezvous is scheduled: its part of this record is written then, as a new record or a revision.
- The live run (design 15.3) shows that Google's ids, etags, occurrences or all-day values behave other than the fake assumes (design FA3, FA8, FA11, FA15, FA17).
- A tool needs data offline or a read that is not bounded by one request.
- Schema version 3 names the fifth role.

## Action items

1. [ ] The owner accepts this record by merging the `CD-W05` pull request, or asks for changes before the merge.
2. [ ] `CD-W05` phase 1 builds D2 to D6 and D8 to D9 in `crates/fabric-google-calendar`, tested against the fake (design 15.1).
3. [ ] The store design for migration 2 keeps the subject and the calendar map (design ST1, ST2) and the creation record (design ST6).
4. [ ] The contract session adds the fifth role (design V3-1); the core's rule for what is writable follows (design CO7).
5. [ ] The live run checks D4 to D8 against the real Google (design LV3, LV6, LV8, LV13, LV14).
6. [ ] On acceptance, set this record to Accepted and update architecture §19 item 11 and the roadmap row for ADR-111.
