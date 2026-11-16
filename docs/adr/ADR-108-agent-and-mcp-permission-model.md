# ADR-108: Agent and MCP permission model

**Status:** Proposed
**Date:** 2026-11-15
**Deciders:** the owner
**Owner:** the owner
**Related:** [Calendar Desktop plan](../plans/calendar-desktop-plan.md) §6 (S01 to S04), §7, §8, §9.2, §10.3, §13, §15 · [Roadmap](../roadmap.md) §4.3, §8.1 · [Isabelle calendar demo](../isabelle-calendar-demo.md) Phase 9, §15 · [Technical architecture](../architecture.md) §19 item 8 · [PRD](../PRD.md) AIA-006, AIA-009 to AIA-013 · [Calendar specification](../calendar.md) CAL-010 · [Agent sandbox](../agent-sandbox.md) (historical input)

## Context

Architecture §19 reserves this record for the agent and MCP permission model: the resource and tool surface, grants, consent, injection controls, permission profiles and modes, incremental authorization and duration, and recipient sets. The Calendar Desktop plan builds that model as a permission broker: a Rust process that holds the Google credentials, decides every agent request with a live Isabelle proof, and asks the owner when a grant is insufficient. Claude Desktop, Claude Code, Codex desktop and an embedded Ollama runtime reach it only through bounded MCP tools (plan §9.2).

The plan proposed a policy matrix, duration semantics and quality targets "to freeze at G0". Gate `CD-G0` (work package `CD-W00`) is that freeze. The owner gave his decisions on 2026-11-15, group by group. This ADR records the part of them that defines the permission model; the plan records the rest. Under `RD-18` the owner chose to renumber Fabric's ADRs by adding 100, so this record is ADR-108, and the plan and this ADR together record the gate.

The evidence comes from the demo's gate `DEMO-G3`, run on 2026-11-15 on this Mac ([demo spec](../isabelle-calendar-demo.md), Phase 9 results):

- Every executed action carried a token whose proof id matched a live Isabelle permit. In the fault run Isabelle denied each injected `add_meeting` on the `consent` rule, and no token was issued.
- The measured cost of a proof: 19 and 23 Isabelle checks in the two runs, 0.71 s and 0.62 s per check on average, 1.39 s at most. The broker proves a read at least three times and a write at least twice, so a read takes about 1.9 to 2.1 s at those averages. A verdict cannot meet the plan's 500 ms.
- The dashboard showed a denial when authorizing, but not the failing rule and no proof link (`DEF-7`, [roadmap](../roadmap.md) §8.1). The owner closed the demo with that gap recorded.

Constraints: the plan's settled `CD-C01` to `CD-C13` are not reopened. `CD-C03` and `CD-C04` send approved data to cloud agents, and text inside events can steer an agent, so the broker, not the agent, must bound what a steered agent can do. Google scopes and ACLs are necessary but never sufficient authority (plan §7).

## Decision

**The broker decides every agent request against one of two frozen built-in profiles or a named custom profile, by a live proof in a separate Isabelle theory, and asks the owner for an exact, scoped, durable grant whenever the current authority is insufficient.**

### D1. Profiles

- The built-in profiles are named **Careful** and **Flexible**, and the names are frozen. Agents are shown by their registered names. Careful is preselected at setup; selecting a profile creates no grant (plan S01).
- Built-in profiles are immutable templates. A named custom profile may set any row up to Allow, wider than Flexible, except the rows that stay fixed in every profile: "Delete event" stays Ask, and "Delete calendar or change sharing" stays Block. An invitation to an address outside every approved recipient set also asks in every profile (D5). Saving a custom profile that widens authority shows the expansion first.
- "Edit a pre-existing event" and "Respond to an invitation" are Ask in both built-in profiles and are **not locked**: a custom profile may set them to Allow.

### D2. Policy matrix

| Operation | Careful | Flexible | Custom profile may set |
|---|---|---|---|
| List calendars, read availability (`CD-M01`) | Allow in grant | Allow in grant | Any mode up to Allow |
| Read event details (`CD-M02`) | Ask for a grant bound to the requested fields and date window; reads inside it run for the grant's duration | Allow inside an accepted grant | Any mode up to Allow |
| Create owner-only event in Agent Drafts (`CD-M03`) | Allow in grant | Allow in grant | Any mode up to Allow |
| Create event in a selected existing calendar (`CD-M04`) | Ask | Allow in a user-created grant naming the calendars | Any mode up to Allow |
| Edit an event Fabric created (`CD-M05`) | Ask outside Drafts | Allow in an accepted scoped grant if no new recipients | Any mode up to Allow |
| Edit a pre-existing event (`CD-M06`) | Ask | Ask | Any mode up to Allow, after the expansion is shown |
| Send or change invitations (`CD-M07`) | Ask | Ask by default; Allow only for an approved recipient set | Any mode up to Allow |
| Respond to an invitation (`CD-M08`) | Ask | Ask | Any mode up to Allow, after the expansion is shown |
| Delete event, Fabric-created events only (`CD-Q05a`) | Ask | Ask | Ask only |
| Delete calendar or change sharing (`CD-M10`) | Block | Block | Block only |

The hard constraints of plan §7 apply to every row. "Delete event" asks in every profile, custom ones included; this is interpretation 1 below.

### D3. Decision order

The plan's priority rule (plan §7) is the order in which the broker decides:

1. Paused, revoked, expired or unsupported: Block.
2. Explicit prohibition: Block.
3. Insufficient capability or required owner approval: Ask.
4. Otherwise prove permission live and Allow.

A read profile alone never implies sending rights. A warning cannot turn Block into Allow or Allow into Block.

### D4. Grants, durations and expiry

- Every grant kind offers the plan's three durations: this request only, this session, and until revoked (`expires_at = null`, labelled "until revoked", never described as a finite expiry).
- A session ends on disconnect, app quit, broker restart, or after a **30-minute idle timeout**.
- An exact-action authorization lives **60 s**; every retry gets a fresh proof.
- An unanswered approval card expires after **24 hours**.

### D5. Recipient sets and contacts

- Invitations use recipient sets (plan S04): every recipient is evaluated, and one recipient needing approval makes the whole request Ask.
- Strangers always ask, as the owner ruled on 2026-11-15: in every profile, custom ones included, an invitation to an address outside every approved recipient set is held for the owner's approval.
- **Version 1 requires Google People import.** It needs an extra Google scope and its verification, a new adapter and a later Google work package. The plan's rule that the manual contact UI needs no People permission (plan §10.2) changes.
- Interpretation 2, confirmed by the owner on 2026-11-15: local groups and exact or domain recipient sets remain, and People import adds a source for them.

### D6. Invitations are a compatibility mode

Google invitations are an explicit compatibility mode outside `CAL-010`'s private scheduling. Google and the recipients see the approved details; `sendUpdates`, attendees and notification behaviour are bound to the approval (plan §10.3). Nothing claims private scheduling. The calendar specification's wording follows in `BASE-1`.

### D7. Action surface in version 1

- **Event deletion:** an agent may ask to delete an event Fabric created; the owner approves each one. Rule theory: a `DeleteEvent` action. Google integration: `events.delete` with If-Match and `sendUpdates` bound to the approval. Reconciliation reads the event back; this is to be confirmed in the W05 spike.
- **Recurring events:** no creating series, no series or instance edits. An agent may ask to accept or decline one occurrence of a recurring invitation, bound to its instance identity (instance id, `originalStartTime`, revision). This amends the exclusion list's "recurring-event mutations".
- Timed events only, each with an explicit IANA zone.
- The narrow list of editable fields.
- Default event type only.
- The plan's exclusion list is frozen, except as amended above.
- The owner creates the Agent Drafts calendar in Google and selects it.

### D8. What an agent learns about warnings

Time and buffer warnings are owner-only in version 1: no warning tool and no warning field reach an agent. Workday, protected time and buffers ship empty ("Not configured"); the owner sets his own hours.

### D9. Proof targets and request limits

- Within 500 ms a "received, checking" state is visible. The verdict follows when the proofs finish.
- The 30 s timeout is per Isabelle check, counted from the check's start. W03 re-measures and reopens these targets if a mean check exceeds 1.5 s.
- Owner actions are acknowledged within 100 ms, excluding proof and network.
- Limits: 100 open requests in total and 25 per agent; 15 requests per minute and 5 open approval cards per agent. Past a limit the agent gets a retryable busy answer. The limits are re-set after W03 measures the theory.
- A Google call that is retried gets a fresh proof per retry, and a notification is never resent blindly.

### D10. A denial stays explained

A denial keeps its failing rule and its proof link on screen (`DEF-7`). This and the lessons of `DEF-1` (one fail-closed audit writer), `DEF-2` (configuration checked before any process starts) and `DEF-4` (used authorizations stored durably) become broker acceptance tests.

### D11. Accepted residual risk

The owner accepts that approved calendar data goes to the Claude and Codex cloud services, and that text inside events can steer an agent. Each agent shows a destination notice saying where its data goes (plan R14). Only proven fields leave the broker, and the Ollama endpoint stays local.

### D12. The rule theory is separate

The broker's rules live in a separate Isabelle theory and session, `isabelle/Fabric_Broker`. The demo's `Fabric_Policy` stays untouched, so the frozen demo record (tag `demo-g3-2026-11-15`) keeps its theory, policy hash and corpus.

The owner interface, the broker process boundary, CI, release operations, distribution identity, the reviewer and the authorization of work packages were decided at the same gate and are recorded in the [plan](../plans/calendar-desktop-plan.md). For this model it is enough that the Tauri host and its webview hold no Google credentials, no database and make no policy decision.

## Options Considered

One line per decision with real alternatives. The options are those of the `CD-G0` decision brief.

- **Profiles and custom ceiling (`G0-CUSTOM`, lock answer):** not chosen: "Flexible is the ceiling" with edit-pre-existing and respond locked at Ask; no custom profiles in version 1. The owner chose against the brief's recommended default, which was "Flexible is the ceiling".
- **Profile names (`G0-NAMES`):** not chosen: another pair of names.
- **Default profile (`CD-M-PROFILE`):** not chosen: Flexible preselected; no preselection.
- **Read event details (`CD-M02`):** not chosen: Careful asks on each read; both profiles allow in a grant. The owner chose against the brief's recommended default, which was "Careful asks on each read".
- **Edit a Fabric-created event (`CD-M05`):** not chosen: Flexible allows only with no attendees and a matching etag; Ask in both. The owner chose against the brief's recommended narrowing and kept the plan's wording.
- **Create in an existing calendar (`CD-M04`):** not chosen: Ask in both.
- **Invitations (`CD-M07`):** not chosen: Ask every time in both profiles.
- **Respond (`CD-M08`):** not chosen: Flexible may allow for a named organizer set.
- **Delete calendar or sharing (`CD-M10`):** not chosen: a custom profile may set Ask.
- **Durations (`CD-M-DUR`):** not chosen: a finite maximum instead of "until revoked"; "until revoked" only for reads and Agent Drafts.
- **Authorization and approval expiry (`CD-Q11f`):** not chosen: one authorization covering the whole retry budget; no approval expiry.
- **Contacts (`CD-Q06`):** not chosen: manual groups only, with People import later. The owner chose against the brief's recommended default, which was manual groups only.
- **Invitations as compatibility mode (`G0-INVITE`):** not chosen: keep `CAL-010` unchanged and treat the plan as a separate product.
- **Event deletion (`CD-Q05a`):** not chosen: off in version 1; any non-recurring event, always Ask. The owner chose against the brief's recommended default, which was "off in v1".
- **Recurring events (`CD-Q05b`):** not chosen: reject all recurring mutations; also allow "this event only" edits. The owner chose against the brief's recommended default, which was "none".
- **Warnings (`G0-WARNDISC`):** not chosen: the agent learns that a warning exists; full detail within a disclosure grant.
- **Proof targets (`RD-10`, `CD-Q11b`, `CD-Q11c`):** not chosen: the targets as written (500 ms read as a verdict); fewer proofs per read; 30 s per action; 10 s per check.
- **Request limits (`CD-Q11d`, `CD-Q11g`):** not chosen: one total bound only; 25 in total; the Google quota only.
- **Denial display (`G0-DEF`):** not chosen: only `DEF-1` and `DEF-4`; defer to `CD-G2` or `CD-G3`.
- **Residual risk (`G0-RISK`):** not chosen: accept for Ollama only at first (reopens `CD-C04`); do not accept (the release cannot ship).
- **Rule theory (`G0-THY`):** not chosen: extend `Fabric_Policy`, which would change the demo's theory, policy hash and proof ids.

## Consequences

What the work packages must now contain:

- **Rule theory (W03, `isabelle/Fabric_Broker`):** Permit, NeedApproval, Block and Undecided for every row of D2; the custom-profile ceiling and the rows fixed in every profile; Careful's read grant bound to fields and date window; a `DeleteEvent` action with ownership, revision and notification facts; instance identity for a reply to one occurrence; the three duration kinds. It must state how the `CD-M05` row combines with the invitation row, so that a change of time for existing attendees is caught, with negative cases. W03 re-measures the proof cost and re-sets the D9 targets and limits.
- **Contracts (W01):** a profile schema that carries custom profiles and the fixed rows; duration kinds with a null expiry for "until revoked"; the 60 s authorization in the token profile; instance identity fields; recipient sets with People import as a source; an error and status grammar whose denial names the failing rule and the proof link; the retryable busy answer. The golden fixtures are Rust and TypeScript.
- **Store (W04):** grants with purpose, scope, duration and revocation; the 30-minute session idle timeout; the 24-hour approval-card expiry; used authorizations stored durably (`DEF-4`); one fail-closed audit writer (`DEF-1`).
- **Screens (W08):** S02 shows "Delete event" as Ask and the Block row locked, and shows the expansion before a custom profile is saved; S03 shows the "received, checking" state and a denial with its failing rule and proof link; S04 adds People import; S07 shows each agent's destination notice; warnings show "Not configured" until the owner sets his hours.

Costly to change later, per the brief:

- High: the custom-profile ceiling once W03's theory and W01's profile schema encode it; the separate theory once W03 writes rules.
- Medium: durations, once grant records and the theory carry duration kinds; invitations and the compatibility mode, since copy and tests depend on them; replies to one occurrence, since instance identity is a schema version bump; the editable-field list, which fixes W01 schemas and W03 arguments.
- Cheap now, costly later: the denial's rule field, once the W01 error schema is frozen; the busy answer's shape, frozen in W01.
- Asymmetric: event deletion and warning disclosure. Enabling deletion and then withdrawing it breaks agents. Widening warning disclosure is cheap, while narrowing it after release breaks agents.
- Low: profile names before W01 (medium after); the numeric targets and limits; the `CD-M02` and `CD-M05` cells before W03.

Baseline wording that must follow in `BASE-1` (named only, not drafted here): PRD AIA-006, AIA-009 to AIA-013, and in particular AIA-010 (expiry and "until revoked"); the calendar specification's CAL-010.

## Open

Interpretations of the answers. The owner confirmed all three on 2026-11-15:

1. Event deletion asks in every profile, custom ones included, because the answer chosen was "Fabric-created only, always ask". So a custom profile can raise every row to Allow except "Delete event" (Ask) and "Delete calendar or change sharing" (Block).
2. With People import required, local groups and exact or domain recipient sets remain, and import adds a source for them.
3. "Everything to CD-G5" authorizes all work packages, but the plan's dependency order, the single contract owner for schemas, token format and rule theory, the independent reviewer, and the owner's approval at each gate in order all still apply.

The owner also ruled on 2026-11-15, after the first `BASE-1` draft: strangers always ask (D5).

Still open:

- Reconciling a deletion by reading the event back is to be confirmed in the W05 spike.
- The extra Google scope for People import and its verification are not yet chosen; they belong to a later Google work package.
- The D9 targets and limits are re-set after W03 measures the theory.

## Action Items

1. [x] The owner confirmed interpretations 1 to 3 on 2026-11-15. This ADR stays Proposed until he accepts it.
2. [ ] W03 writes `isabelle/Fabric_Broker` with the rows of D2, the `CD-M05` and invitation combination with negative cases, and re-measures the proof cost.
3. [ ] W01 freezes the profile schema, duration kinds, instance identity, recipient sets and the denial's rule and proof-link fields.
4. [ ] `BASE-1` shows the owner the wording for AIA-006, AIA-009 to AIA-013 and CAL-010.
