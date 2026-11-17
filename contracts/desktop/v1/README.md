# The `fabric-desktop/1` contract

This directory is the contract between the Fabric Calendar broker and everything that talks to it: the Tauri host and its webview on the owner channel, and the agent adapters (the MCP bridge and the embedded Ollama runtime) on the agent channel. It is work package CD-W01 of the [Calendar Desktop plan](../../../docs/plans/calendar-desktop-plan.md) and records the permission model of [ADR-108](../../../docs/adr/ADR-108-agent-and-mcp-permission-model.md).

Schema version 2, written on 2026-11-16. Version 1 was written on 2026-11-15. One session writes this directory; other work packages ask for changes.

| Path | What it holds |
|---|---|
| `schemas/` | JSON Schemas: shared types and the error grammar, the owner channel, the agent channel |
| `data/` | Contract data: versions, the limits frozen at CD-G0, the request lifecycle, the policy matrix, the tool catalog, the owner methods |
| `fixtures/` | Golden and negative fixtures that the Rust and TypeScript sides both check |
| `typescript/` | The TypeScript types and the fixture check |
| `reference/` | A Python reference reading of the contract, which computed the fixtures' expectations |
| `../../../crates/fabric-desktop-contracts/` | The Rust transport types, the canonical encoding and the digests |

The schemas and data files are the contract. This README states the rules that a schema cannot.

## 1. Frames

A frame is one JSON object written as UTF-8 on one line and ended by a line feed (U+000A). A frame is at most 1,048,576 bytes without the line feed, and holds no line feed itself: inside a string a line feed is written `\n`, and between tokens it is not allowed. Nothing else is written to the channel: no logs, no byte-order mark, no blank lines.

There are three kinds of frame.

- **Request:** `protocol`, `id`, `method`, `params`. The owner channel's methods are in `data/owner-methods.json`; the agent channel's methods are the tools of `data/tool-catalog.json`.
- **Response:** `protocol`, the request's `id` and `method`, and either `result` or `error`, never both. When the broker could not read a frame far enough to know its id or method, the error response carries `null` for them.
- **Notice:** `protocol`, `sequence`, `change`. Only the broker sends notices, and only on the owner channel, after `subscribe`.

Every frame carries `"protocol": "fabric-desktop/1"`. The entry schemas are `owner_request`, `owner_response` and `owner_notice` in `schemas/owner-channel.schema.json`, and `agent_request` and `agent_response` in `schemas/agent-channel.schema.json`.

## 2. Strict JSON

A receiver reads a frame as JSON (RFC 8259) and refuses it unless all of these hold:

1. The bytes are valid UTF-8 and do not begin with a byte-order mark.
2. The text is one JSON value with nothing after it but whitespace. For a frame the value is an object.
3. No object has two members with the same name.
4. Every number is an integer written as `-?(0|[1-9][0-9]*)`, with no fraction and no exponent, between −9007199254740991 and 9007199254740991. `-0` is refused.
5. Every string is well-formed Unicode: an escaped surrogate must be half of a pair.
6. Arrays and objects nest at most 32 deep, counting the outermost value as depth 1.

Whitespace between tokens (space, tab, line feed, carriage return) is allowed and ignored. A frame is one line, so in a frame a line feed is refused (section 1).

## 3. Canonical form

The canonical form of a JSON value is the text made by these rules. Digests are taken over its UTF-8 bytes.

- An object is `{`, its members separated by `,`, and `}`. Members are ordered by name, comparing names by Unicode code point. A member is its name as a string, `:`, and its value.
- An array is `[`, its elements in order separated by `,`, and `]`.
- A string is `"`, its characters, and `"`. `"` is written `\"` and `\` is written `\\`. U+0008, U+0009, U+000A, U+000C and U+000D are written `\b`, `\t`, `\n`, `\f` and `\r`. Every other character below U+0020 is written `\u00` and two lowercase hexadecimal digits. Every other character is written as itself.
- An integer is written in decimal with no leading zero and no plus sign.
- `true`, `false` and `null` are written as such.
- There is no whitespace, and no Unicode normalization.
- A string that is not well-formed Unicode has no canonical form.

Two rules of the schemas make the canonical form of a contract value unique. Every property of every object is required, so a missing value is written `null`, never left out. And a list is ordered: where the schema says its items are unique, the order still counts.

## 4. Digests

```
digest(domain, value) = "sha256:" + lowercase hex of
    SHA-256( UTF-8("fabric-desktop/1\n" + domain + "\n") + UTF-8(canonical(value)) )
```

| Domain | Subject | Used for |
|---|---|---|
| `action` | The `action` object: `{"tool": …, "arguments": …}`. For an agent request frame it is `{"tool": method, "arguments": params}`. | A one-request grant and an authorization bind exactly this action. |
| `request` | `{"agent_id": …, "action": …, "scope": …}` | `request_digest`: what the owner saw on the approval card. The same subject with the scope the owner approved gives `effective_request_digest`. |
| `approval` | `{"request_id": …, "request_digest": …, "decision": …, "scope": …, "duration": …}`. For an approval `scope` is the scope the owner approved: `selected` when the decision gave one, otherwise the scope of the approval card. For a denial `scope` and `duration` are null. | `approval_digest`: the owner's exact decision. |
| `command` | `{"method": …, "params": …}` of an owner command frame | Command deduplication. |
| `expansion` | The `expansion` array | The owner's confirmation that a custom profile widens authority. |

`theory_hash` and `context_digest` in a proof report have the same text form. Their subjects belong to the rule theory (CD-W03).

A command is an owner method marked `command` in `data/owner-methods.json`. Its frame `id` is the command id. A command id sent again with the same command digest gets the first answer again and changes nothing; sent with a different digest it gets the error `command_reused`.

## 5. Layers of refusal

A receiver checks a frame in this order and stops at the first failure. The error code is what the broker answers on a request.

| Layer | Check | Error code |
|---|---|---|
| `frame` | The size bound, no line feed, and strict JSON (sections 1 and 2) | `malformed_frame` |
| `protocol` | `protocol` is the string `fabric-desktop/1` | `unsupported_protocol` |
| `schema` | The frame validates against its entry schema | `unknown_method` when a request's `method` is not a string naming a method of that channel; `invalid_params` otherwise |

The webview and the Tauri host apply these layers and nothing more. They make no policy decision. Checks that need knowledge of the world belong to the broker: that a zone exists and an offset agrees with it, that a start is before an end, that an id resolves, that an authorization's expiry is its issue time plus its lifetime.

## 6. The schema subset

The schemas are JSON Schema 2020-12 restricted to these keywords: `$schema`, `$defs`, `$ref`, `title`, `description`, `type`, `const`, `enum`, `properties`, `required`, `additionalProperties`, `items`, `minItems`, `maxItems`, `uniqueItems`, `minLength`, `maxLength`, `pattern`, `minimum`, `maximum`, `oneOf`.

- `$ref` is `#/$defs/name` or `file.schema.json#/$defs/name`, resolved against `schemas/`. A schema with `$ref` has no other keyword but `description`.
- Every object schema lists `properties`, requires all of them, and sets `additionalProperties` to `false`.
- `type` is one of `object`, `array`, `string`, `integer`, `boolean`, `null`.
- `minLength` and `maxLength` count Unicode code points.
- `pattern` is an anchored ECMAScript regular expression, matched with the `u` flag.
- `const`, `enum` and `uniqueItems` compare values by their canonical form.
- `oneOf` holds when exactly one branch holds.

## 7. The owner channel

`data/owner-methods.json` lists the thirteen methods of plan section 9.1 with their parameter and result schemas.

- **hello** reports the broker's build, `schema_version`, capabilities and limits. Under `fabric-desktop/1` the two sides must have the same `schema_version`; otherwise the broker answers `unsupported_schema_version`. No capability name is defined yet.
- **get_snapshot** returns the state at a cursor: health, profiles, accounts, agents, recipient sets, preferences, the open requests and the grants that have not ended.
- **subscribe** asks for every change after a cursor. Notices then arrive with sequences rising by one. If the cursor is older than the history the broker kept, the answer is `snapshot_required`; if a subscriber falls more than `subscriber_queue_notices` behind, it receives the change `snapshot_required` and must take a snapshot and subscribe again.
- **Commands** name the revision they were written against (`expected_revision`). A revision that is no longer current gets `conflict` and nothing is overwritten. A successful command returns `commit_sequence`, the sequence of the notice that carries its effect; the broker commits before it answers.
- **decide_request** names the request, its revision and the `request_digest` the owner saw. If the request has changed, the answer is `stale_request`. `selected` is null to approve the scope as asked, or a scope that narrows it. `duration` is the card's `proposed_duration` or a shorter one (section 10); a longer one gets `invalid_params`. A recipient rule may name recipient sets in place of exact addresses.
- **save_profile** either puts a profile in force (`activate`) or saves a custom profile (`save_custom`). When a save makes a row more permissive, the broker answers `expansion_not_acknowledged` with the expansion and its digest; the interface shows it, and the owner's confirmation is a new command that carries that digest.
- **pause_agent** pauses or resumes one agent or all of them.
- **get_request_details** returns the request, its approval card, the approval, the proofs, the authorization and the attempts. With `redaction` `redacted`, `private` is null and no event content leaves the broker's store. For a request that no proof decided, the details say why in `undecided_reason`: `prover_unavailable` when the rule prover was starting or not running, `out_of_time` when the proof did not finish within its time limit, and `failed` when the check ran and did not decide the request, or its report could not be used. It is null when the broker did not record why, and null for a request in any other state. The schema cannot state the tie to the state; the broker must uphold it.

`health` says whether the rule prover and the store are ready. While `prover` is `unavailable`, `prover_problem` names the cause, or is null when the broker does not know it; while `prover` is `ready` or `starting`, it is null. The schema cannot state this rule; the broker must uphold it. What the owner can do follows from the value and is not sent:

| `prover_problem` | Meaning | What the owner can do |
|---|---|---|
| `not_configured` | This run has no rule prover. | Nothing in this run. |
| `failed_will_retry` | The prover could not start, or stopped; the broker starts it again by itself. | Wait. |
| `failed` | The prover could not start, or stopped, and the broker will not try again in this run. | Quit Fabric Calendar and open it again. |
| `wrong_rules` | The rule theory on this machine is not the one this broker was built with. | Install Fabric Calendar again. |

Errors have a code, a message for the person, `retryable`, a recovery action, a correlation id and a typed detail. For each code the schema fixes `retryable`, the recovery action and the shape of the detail.

## 8. The agent channel

The methods are the ten tools of `data/tool-catalog.json`. An action is a tool and its exact arguments; `fabric_request_status` is not an action.

Every tool answers a `tool_outcome`: the request's status and, only when a fresh proof released it, a result.

- When the owner must decide, the state is `pending_approval`, with the time the card expires, and the result is null. The agent polls `fabric_request_status` with the `request_id`.
- When the request is blocked, the status carries `denial`: the failing rule with a bounded explanation.
- A status never carries calendar content. Reading a result (`fabric_request_result`) is itself an action with its own proof.
- When no proof decided the request, the state is `undecided` and the status carries nothing more. Why it is undecided is for the owner only (section 7).
- Past a request limit the broker answers the error `busy`, which is retryable and names the limit, its maximum and a wait in milliseconds. No request is created.
- While the rule prover is not ready the broker may refuse a tool call with the error `prover_unavailable`, which is retryable and carries no detail. No request is created. A request the broker accepted all the same ends `undecided`.

No tool carries a warning: warnings are for the owner only in version 1 of Fabric Calendar. No tool approves, grants, edits a profile, deletes a calendar, changes sharing or reaches an arbitrary Google URL.

## 9. The request lifecycle

`data/request-lifecycle.json` lists the 23 states and the legal transitions. A request starts in `received`. It is open while its state is not terminal; the open-request limits count open requests, and an open approval card is a request in `pending_approval`.

The 24 transitions marked `plan` are the state grammar of plan section 11. The 15 marked `added` close it, and each cites the plan text it rests on.

A request's `status` is its state and what that state always carries, and the schema will not accept one without the other:

- `blocked` carries `denial`: the failing rule, an explanation and the id of the proof. A denial without its rule or its proof is not a valid message.
- `terminal_failure` carries `failure` with a code.
- `pending_approval` carries `approval_expires_at`. An unanswered card expires after `approval_expiry_seconds`.
- Every other state carries nothing more.

The reason a request is `undecided` is in its details (section 7), not in its status.

## 10. Profiles, grants and durations

A profile carries one mode per row of the policy matrix (`data/policy-matrix.json`). The built-in profiles Careful and Flexible are constants of the schema. A custom profile may set any row to any mode, except that `delete_event` is `ask` and `delete_calendar_or_change_sharing` is `block` in every profile.

A scope names calendars, each with its account, and where they apply a date window, fields, recipients and who is notified. A member that applies and is null covers nothing. No value of a scope stands for "everything".

A recipient rule names exact addresses or recipient sets. No rule covers everyone, so an invitation to an address outside every approved recipient set always asks. A recipient set is a selector with a source: curated in Fabric, or imported from Google People.

An approval has one of three durations: `one_request`, `session`, `until_revoked`. A grant's `term` binds it: to one action digest, to one agent session, or to nothing but revocation, in which case it carries `"expires_at": null`. A session ends on disconnect, on app quit, on broker restart, or after `session_idle_timeout_seconds` without activity.

The durations are ordered from shortest to longest: `one_request`, `session`, `until_revoked`. A card's `proposed_duration` is the longest one the owner may choose for that request. The interface offers it and the shorter ones: the owner can narrow a duration, like a scope, and cannot widen it.

The broker does not propose a duration whose grant would not serve the request. Under a row whose mode is `ask`, every request needs the owner's own approval, so a grant for a session or until revoked does not serve it and the card proposes `one_request`. The matrix states two exceptions, where such a grant does serve: a read of event details (CD-M02), and an edit of an event Fabric created in Agent Drafts (CD-M05). This is the rule theory's reading of the matrix; the owner confirmed it on 2026-11-16.

## 11. The authorization

The broker's exact-action authorization is the record `authorization` in the owner schema, profile `fabric-broker-authorization/1`. It names the request, the agent, the action digest, the grant and the live proof that permitted the action, and it is used once. `lifetime_seconds` is the constant 60, and `expires_at` is `issued_at` plus that.

It is the internal authorization type that plan section 8 allows. The CBOR token of `crates/fabric-token` is not changed and is not used by this protocol.

## 12. Versions

`data/version.json` holds the protocol name, the schema version and the authorization profile. A change that an older peer could not read needs a new protocol major. Every other change raises `schema_version`.

Version 2 added why the rule prover is unavailable (`prover_problem` in `health`), why a request is undecided (`undecided_reason` in the request's details) and the agent error `prover_unavailable`. The protocol name stays `fabric-desktop/1`: `hello`, the frame format and the error envelope did not change, so a version 1 peer can still read the refusal `unsupported_schema_version`.

## 13. Fixtures and the contract suite

Both languages read the same files in `fixtures/`, compute every result themselves, and compare it with the expectation in the file. The expectations were computed by `reference/reference.py`, a third reading of this README in Python that shares no code with either side.

| File | Form | What is checked |
|---|---|---|
| `strict-json.json` | `{"accept": [{"name", source, "canonical"}], "reject": [{"name", source}]}` | Section 2 and section 3. |
| `scalars.json` | `{"<schema reference>": {"accept": [...], "reject": [...]}}` | Each pattern and bound, value by value. |
| `digests.json` | `[{"name", "domain", "subject", "canonical", "digest"}]` | Section 4. |
| `lifecycle.json` | `[{"name", "states", "valid", "first_invalid"}]` | A sequence is valid when it starts at `received` and every step is a listed transition. `first_invalid` is the index of the first state that breaks this. |
| `messages/*.json` | `{"description", "schema", source, "expect"}` | Section 5 against the named schema. |

A source is exactly one of four members. `text` is the text itself. `hex` gives its bytes in hexadecimal, for bytes that are not UTF-8. `generate` is `{"prefix", "fill", "count", "suffix"}` and stands for the prefix, then `fill` repeated `count` times, then the suffix. `value`, in a message fixture only, is a JSON value and stands for its canonical text. The fixture files themselves are ordinary JSON.

`expect` in a message fixture is `{"verdict": "accept", "canonical", "command_digest", "action_digest"}` or `{"verdict": "reject", "layer", "error"}`. `command_digest` is set for an owner command frame and `action_digest` for an agent request frame that is an action; otherwise they are null. The protocol layer, the frame size bound and the line-feed rule apply to the five frame entry schemas only; for every entry the value must be an object, or it is refused at the frame layer.

Each side also prints a report of what it computed, one line per check, tab-separated and sorted:

```
strict     <name>         accept <sha256 of canonical>     | reject
scalar     <reference>    accept|reject <index>  accept|reject
digest     <name>         <digest>
lifecycle  <name>         valid|invalid  <first_invalid or ->
message    <file name>    accept <sha256 of canonical> <command_digest or -> <action_digest or ->
message    <file name>    reject <layer> <error>
mutant     <id>           accept | reject <layer> <error>
```

To add a fixture, write it with `"canonical"` and the digests null, or with the layer and error it must be refused with, and run `python3 contracts/desktop/v1/reference/reference.py bless`. It fills in what an accepted fixture expects and reports any fixture that is not judged as written.

`scripts/verify-contracts.sh` runs both sides, compares the two reports byte for byte, then does the same for the mutants that the Rust side derives from the accepted messages: at every node of every accepted message, a member removed, an unknown member added, the value replaced by values of other types and by its neighbours, an object written as an array. The Rust verdict comes from the serde types and the TypeScript verdict from the schemas, so any place where the two describe different languages fails the suite. When Python 3 is installed the suite also runs the reference, whose report and verdicts must equal the Rust ones.

## 14. Choices made where the plan was open

Each of these is the narrowest reading that makes the contract complete. The owner may change any of them.

Encoding:

1. The frame bound (1 MiB) and the subscriber queue (256 notices) are the plan's proposed values.
2. Strict JSON, with every property required and `null` for a missing value, so that a contract value has one canonical form.
3. The canonical form and the digest formula of sections 3 and 4.
4. Instants are UTC with milliseconds (`2026-11-20T16:00:00.000Z`). An event time is a local date-time with its offset plus an IANA zone, as Google stores it.
5. Ids are at most 128 ASCII characters. An address has an ASCII local part and a lowercase ASCII domain. The schemas bound every string and most lists; accounts and agents are not capped.
6. A response repeats the request's `method`, so that a frame can be checked on its own.
7. `schema_version` must match exactly under `fabric-desktop/1`.
8. The error codes, the recovery actions and the shape of the busy answer.
9. A frame holds no line feed, so that one line is always one frame.

Owner channel:

10. The frame id is the command id, as in the plan's example.
11. The snapshot also carries recipient sets and preferences. Its `requests` are the open ones, which the limits bound at 100.
12. One profile is in force for the whole broker, not one per agent.
13. The expansion of a custom profile is confirmed by digest (section 7). Which earlier state the expansion is measured against is left to the broker.
14. A scope names each calendar together with its account, so one approval can cover calendars of more than one account; the list tool names no account.
15. `pause_agent` carries `paused`, so the same method resumes, and may target all agents.
16. `get_proof` takes no redaction parameter, because a proof report holds no calendar content.
17. The connection states of an account, and the shape of the preferences, follow plan sections 10.1 and S06.

Agent channel:

18. The method is the tool. Each tool takes `task`, the agent's own words for the owner to read; it is part of the action.
19. The delete tool is named `fabric_calendar_propose_delete`.
20. The editable fields are title, start, end, description and location. CD-Q05d says "narrow list" and the repository does not enumerate it.
21. A created event has no attendees; an invitation carries the complete attendee list.
22. A response is `accepted` or `declined`. `tentative` can be read but not proposed.
23. `send_updates` is `all`, `external_only` or `none`, on every change that can notify.
24. A read names fields from: time, title, location, description, attendees, organizer, own_response.
25. An agent sees the lifecycle state by name, and for a blocked request the failing rule and an explanation, not the proof id.
26. The schema does not tie the kind of a result to the tool that was called; the broker does.

Policy:

27. A profile carries a mode per row and nothing else. Flexible's cells for invitations and for editing a Fabric-created event are encoded `allow`, and Careful's "Ask outside Drafts" is encoded `ask`; the frozen wording is kept beside each mode in `data/policy-matrix.json`.
28. A custom profile's id begins with `custom-`. The schema does not stop a custom profile from being named Careful or Flexible; the broker should.
29. A recipient set has one source. The Google People source records the account and the import time.
30. The reasons a grant ends.

Lifecycle:

31. The 15 added transitions, and the terminal states. `undecided`, `absence_confirmed` and `needs_owner_resolution` are terminal in this version.
32. The failure codes of `terminal_failure`.

Authorization:

33. An internal authorization record with a versioned profile, not a new token format (section 11).

Version 2:

34. The cause of an unavailable prover is one closed list (`prover_problem`). What the owner can do follows from the value and is not sent.
35. `prover_problem` may be null while the prover is unavailable, for a cause the broker does not know.
36. The reason of an undecided request is in the request's details and not in its status. The broker's store keeps statuses as they were written, and a status that changed shape could not be read back from an existing store.
37. The reason of an undecided request may be null, for a request whose reason the broker did not record.
38. Agents do not see why a request is undecided; they see the state only.
39. There are three reasons. A check that ran and did not decide, and a report that could not be used, share the value `failed`.
40. `prover_unavailable` carries no detail. The broker may answer it while the prover is not ready, but need not: it may accept the request, which then ends `undecided`.
41. A change of this kind raises `schema_version` and not the protocol major (section 12).
42. A card's `proposed_duration` is the longest duration the owner may choose, not only the one shown first. The plan says the owner can narrow the duration and the interface cannot widen a request (its section 6, S03). No member is added for it.
43. The durations are ordered `one_request`, `session`, `until_revoked`.
44. A decision with a longer duration than the card proposes gets `invalid_params`.
45. The broker does not propose a duration whose grant would not serve the request. Which of the durations that would serve it the broker proposes, and which one the interface selects at first, are not fixed (section 15).

## 15. Not covered

- Commands that the plan's text needs and its method table does not list: selecting calendars and naming the Agent Drafts calendar, approving or removing an agent's enrollment, resolving an unknown outcome by hand, starting a People import, listing ended requests, the task composer for the embedded runtime, and the message that cancels a request. The snapshot can already show the state these would change.
- The agent channel's connection handshake, which proves possession of the enrolled key (CD-W06).
- Actions the broker starts itself, such as refresh and reconciliation reads, and the formal context with its digest (CD-W03, CD-W04).
- A marker for practice accounts, attempts and reports.
- Which of the durations that would serve a request the broker proposes on its card, and which duration the interface selects at first.
- The names of the rules. The contract fixes their form; the rule theory names them. The rule names in the fixtures are examples.
- Facts that tie several members together, which a schema cannot state and the broker must uphold: that a snapshot's requests are open and its grants have not ended, that an expansion names each row once, that the active profile exists, that an approval card expires 24 hours after it opens, that a card proposes no duration whose grant would not serve its request, and that an authorization expires 60 seconds after it is issued (the Rust crate offers a check for the last).
