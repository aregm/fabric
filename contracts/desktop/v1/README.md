# The `fabric-desktop/1` contract

This directory is the contract between the Fabric Calendar broker and everything that talks to it: the Tauri host and its webview on the owner channel, and the agent adapters (the MCP bridge and the embedded Ollama runtime) on the agent channel. It is work package CD-W01 of the [Calendar Desktop plan](../../../docs/plans/calendar-desktop-plan.md) and records the permission model of [ADR-108](../../../docs/adr/ADR-108-agent-and-mcp-permission-model.md).

Schema version 3, written on 2026-11-17. Version 1 was written on 2026-11-15 and version 2 on 2026-11-16. One session writes this directory; other work packages ask for changes.

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

- **Request:** `protocol`, `id`, `method`, `params`. The owner channel's methods are in `data/owner-methods.json`; the agent channel's methods are the tools of `data/tool-catalog.json` the two handshake methods and `agent_cancel` (section 8).
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

Two more texts begin with the same first line and have domains of their own, `agent_handshake` and `agent_key`. They are not digests of this form; section 8 defines them.

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

`data/owner-methods.json` lists the methods with their parameter and result schemas: the thirteen of plan section 9.1, and eight that the plan's text needs and its table does not list (`select_calendars`, `refresh_calendars`, `resolve_outcome`, `add_agent`, `approve_agent`, `remove_agent`, `get_agent_configuration`, `list_ended_requests`).

- **hello** reports the broker's build, `schema_version`, capabilities and limits. Under `fabric-desktop/1` the two sides must have the same `schema_version`; otherwise the broker answers `unsupported_schema_version`. One capability name is defined: `practice_calendar`, which the broker reports exactly when it runs with the practice calendar.
- **get_snapshot** returns the state at a cursor: health, profiles, accounts, what the broker knows about each account's connection to Google, agents, recipient sets, preferences, the open requests and the grants that have not ended.
- **subscribe** asks for every change after a cursor. Notices then arrive with sequences rising by one. If the cursor is older than the history the broker kept, the answer is `snapshot_required`; if a subscriber falls more than `subscriber_queue_notices` behind, it receives the change `snapshot_required` and must take a snapshot and subscribe again.
- **Commands** name the revision they were written against (`expected_revision`). A revision that is no longer current gets `conflict` and nothing is overwritten. A successful command returns `commit_sequence`, the sequence of the notice that carries its effect; the broker commits before it answers.
- **decide_request** names the request, its revision and the `request_digest` the owner saw. If the request has changed, the answer is `stale_request`. `selected` is null to approve the scope as asked, or a scope that narrows it. `duration` is the card's `proposed_duration` or a shorter one (section 10); a longer one gets `invalid_params`. A recipient rule may name recipient sets in place of exact addresses.
- **save_profile** either puts a profile in force (`activate`) or saves a custom profile (`save_custom`). When a save makes a row more permissive, the broker answers `expansion_not_acknowledged` with the expansion and its digest; the interface shows it, and the owner's confirmation is a new command that carries that digest. `delete_custom` deletes a custom profile; the broker refuses to delete the profile in force with `invalid_params`. A grant made under a deleted profile keeps naming its id and version.
- **pause_agent** pauses or resumes one agent or all of them.
- **select_calendars** sets, for one account, which calendars the owner has selected and which of them is Agent Drafts (CD-Q05f). `selected` is the whole selection: a calendar of the account that it does not name is no longer selected. `agent_drafts` names the Agent Drafts calendar, or is null for none; an account has at most one. The broker answers `invalid_params` when that calendar is not in `selected` or the account cannot write to it, and `not_found` when it does not know the account or a calendar. Nothing changes at Google: the owner creates the calendar there.
- **refresh_calendars** asks the broker to list one account's calendars again. The answer says only that the command is committed; the list arrives later, as changes of the account.
- **add_agent**, **approve_agent** and **remove_agent** enroll and remove an agent, and **get_agent_configuration** returns the texts that start its bridge. They are described below.
- **resolve_outcome** is the owner's own statement about a request in `needs_owner_resolution`: the change is at Google (`applied`) or it is not (`absent`). The request ends in `resolved_as_applied` or `resolved_as_absent` (section 9). The broker sends nothing to Google and starts no new attempt. If the request has changed or is in another state, the answer is `stale_request`.
- **list_ended_requests** returns requests that have ended, newest first and at most `limit` at a time, without private event details. `before` is null for the first page and otherwise the id of the last request of the page before; `more` says whether older ones remain.
- **get_request_details** returns the request, its approval card, the approval, the proofs, the authorization and the attempts. `approval` is the owner's decision on the card now shown. It is null while the request waits for a decision, also when an earlier approval was asked again, and it stays null when such a request expired while it waited. With `redaction` `redacted`, `private` is null and no event content leaves the broker's store. For a request that no proof decided, the details say why in `undecided_reason`: `prover_unavailable` when the rule prover was starting or not running, `out_of_time` when the proof did not finish within its time limit, `provider_unreachable` when Google could not be reached to look at the event, and `failed` when the check ran and did not decide the request, or its report could not be used. It is null when the broker did not record why, and null for a request in any other state. The schema cannot state the tie to the state; the broker must uphold it. The details also carry what happened around the request: `notification` says, for an applied change, whether its notifications are known to be sent (`sent`, `not_requested`, `unknown`), and is null otherwise; `system_proofs` are the proofs of what the broker did itself for the request (a read to look at the event, a reconciliation, a calendar list); `releases` are the releases of a result to the agent, each under its own proof. None of them holds calendar content.

`health` says whether the rule prover and the store are ready. While `prover` is `unavailable`, `prover_problem` names the cause, or is null when the broker does not know it; while `prover` is `ready` or `starting`, it is null. The schema cannot state this rule; the broker must uphold it. What the owner can do follows from the value and is not sent:

| `prover_problem` | Meaning | What the owner can do |
|---|---|---|
| `not_configured` | This run has no rule prover. | Nothing in this run. |
| `not_installed` | The rule prover is not installed on this machine. | Run Fabric Calendar's installer for the rule prover, then quit Fabric Calendar and open it again. |
| `failed_will_retry` | The prover could not start, or stopped; the broker starts it again by itself. | Wait. |
| `failed` | The prover could not start, or stopped, and the broker will not try again in this run. | Quit Fabric Calendar and open it again. |
| `wrong_rules` | The rule theory on this machine is not the one this broker was built with. | Install Fabric Calendar again. |

`health.agent_channel` says whether agents can connect. While it is `not_listening`, `agent_channel_problem` names the cause, or is null when there is none to name, as while the store is not ready:

| `agent_channel_problem` | Meaning |
|---|---|
| `path_too_long` | The path of the agent socket is longer than the system allows. |
| `run_dir_unsafe` | The directory of the socket is not a directory of the broker's own with access for its user only. |
| `run_dir_owner` | That directory belongs to another user. |
| `stale_not_socket` | Something that is not a socket is where the socket belongs, and the broker does not remove it. |
| `socket_mode` | The socket could not be given access for its user only. |
| `bind_failed` | The system refused the socket. |

Beside each account the snapshot holds a `provider_account`, announced by the change `provider_account`. The account's own record is unchanged. It is for the owner only:

- `last_connect_problem` says why the last attempt to connect ended without a connection, and is null when it did not:

| `last_connect_problem` | Meaning |
|---|---|
| `timed_out` | The owner did not finish in the browser in time. |
| `superseded` | A newer attempt to connect replaced this one. |
| `browser_not_opened` | The browser could not be opened. |
| `google_error` | Google answered the attempt with an error. |
| `client_rejected` | Google did not accept Fabric Calendar itself. |
| `network` | Google could not be reached. |
| `identity_check_failed` | The identity of the account could not be verified. |
| `unexpected_scope` | Google granted permissions other than the ones asked for. |
| `no_refresh_token` | Google gave no lasting access. |
| `token_store` | The access could not be kept in the Keychain. |
| `already_an_account` | This Google account is connected already. |

- `missing_scope` names, for an account in the state `partial_grant`, the permission the owner did not grant, by Google's name of the scope.
- `refresh_token_ends_at` is when the connection is due to end, or null when no end is known.
- `revocation` says whether Google confirmed that Fabric's access was revoked at a disconnect: `not_requested`, `pending`, `revoked`, `not_confirmed`. It is not in the answer to `disconnect_google`, because the broker answers after its commit and asks Google afterwards.

An agent is enrolled in the application. Its record carries `enrollment`:

| `enrollment` | Meaning |
|---|---|
| `pending` | The owner added the agent and has not approved it. It has no session, makes no request and holds no grant. |
| `approved` | The owner approved one key of the agent. Only an approved agent gets a session. |
| `removed` | The owner removed the agent. Its id and its key are never accepted again. The record stays, so that the history can name the agent. |

- **add_agent** takes a name and the kind of client: Claude Desktop, Claude Code or Codex desktop. The broker creates a pending agent with a new id and sets `destination` from the kind of client, as `data/agent-clients.json` lists it. That destination is a label for the client the owner named, not a checked fact: the broker cannot check which program holds the key or where it sends data. The id is what the handshake calls the enrollment id: an agent has one id.
- While an agent is pending, the key it proved last in a handshake (section 8) is its key. `pairing_code` is the code of that key and `key_proven_at` the time it was proved; both are null until a key is proved. A proved key that differs from the agent's current key raises the agent's revision; proving the current key again changes nothing.
- **approve_agent** names the agent, its revision and the pairing code the owner saw, so the broker approves exactly the key with that code. If the revision is no longer current the answer is `conflict`. If the code is not the current key's, or no key was proved, the answer is `invalid_params`. In the same commit the agent gets a grant until revoked for the calendar list (section 10), and the answer carries that grant. Approving an agent that is not pending gets `conflict`. After the approval the key is fixed.
- **remove_agent** removes a pending or an approved agent. Removing an agent that is removed already gets `conflict`. It ends the agent's session and ends its grants with the reason `agent_removed`. Its requests that were not yet handed to Google are cancelled. A request that was handed over runs to its end, because cancelling after dispatch would be a false rollback (plan section 9.2).
- Each of these steps is announced by the change `agent`, which carries the whole record.
- **get_agent_configuration** returns three texts, one for each kind of client, that start this agent's bridge with its id. They hold the bridge's path and the id, and nothing secret. `configuration` is null when the broker cannot name an installed bridge. For a removed agent the answer is `not_found`.

The schema cannot state how the members of an agent go together; the broker must uphold it. `approved_at` is set exactly when the agent is or was approved, and `removed_at` exactly when it is removed. Only an approved agent has a session. Pausing a pending or a removed agent gets `conflict`.

The broker bounds the size of the owner's records. A command that would take them past the bound gets `invalid_params` and stores nothing: at `/params/registered_name` for `add_agent`, at `/params` for `approve_agent`, and at `/params/account_id` for `connect_google`.

`added_at` is null in one case only. A broker of schema version 2 stored its answers to `pause_agent`, which hold agent records of that version. When such an answer is given again, its agents carry `enrollment` `approved` and null for the five other members that version 3 added, because the stored answer does not hold them. The contract crate has a reader for stored answers that does this (`decode_stored_owner_response`); on the channel an agent record without those members stays refused.

Errors have a code, a message for the person, `retryable`, a recovery action, a correlation id and a typed detail. For each code the schema fixes `retryable`, the recovery action and the shape of the detail.

## 8. The agent channel

The methods are the ten tools of `data/tool-catalog.json`, the two handshake methods, `agent_hello` and `agent_prove`, and `agent_cancel`. An action is a tool and its exact arguments; `fabric_request_status`, the handshake methods and `agent_cancel` are not actions.

### The handshake

A connection begins with two round trips, before any tool frame.

1. `agent_hello` carries the adapter's `schema_version`, the agent's id (`enrollment_id`) and its public key. The broker answers with its own `schema_version` and with `broker_nonce`: 256 random bits that it makes for this connection and uses once. If the versions differ, the answer is `unsupported_schema_version`.
2. `agent_prove` carries the signature of the handshake text. The broker answers `approved` when the session has started, or `pending` when the proved key is now the pending agent's key and the owner has not approved yet (section 7). With either answer comes `practice`, which is true exactly when the broker runs with the practice calendar. Every other outcome is the error `not_permitted`, or `busy` naming `sessions_per_minute_per_agent` when the agent starts sessions too fast.

```
handshake text = "fabric-desktop/1\nagent_handshake\n"
                 + canonical({"broker_nonce": …, "enrollment_id": …, "public_key": …, "schema_version": …})
signature      = Ed25519 (RFC 8032) over the UTF-8 bytes of the handshake text,
                 its 64 bytes written as 128 lowercase hexadecimal digits
pairing code   = the first 12 hexadecimal digits, in capitals, of
                 SHA-256( UTF-8("fabric-desktop/1\nagent_key\n" + public key) ),
                 written in three groups of four: AB12-CD34-EF56
```

The four members of the handshake text are those of the hello and its answer; `schema_version` is the adapter's. The public key is written as in the hello: `ed25519:` and its 32 bytes as 64 lowercase hexadecimal digits.

- The broker answers every well-formed hello of its own schema version with a nonce, whatever the id. It decides only at `agent_prove`, after the signature verifies: an unknown id, a removed agent, a key that is not the approved one and a bad signature all get `not_permitted`. So a hello tells nobody which ids exist, and a key that was not proved is never recorded.
- A public key belongs to at most one agent.
- The key is proved once per connection. No later frame is signed (plan section 15.2, NOV17-SIGN).
- A tool frame on a connection whose handshake did not end with `approved` gets `not_permitted`.
- The pairing code lets the owner see that the key the broker holds for a pending agent is the key the agent's bridge holds: the bridge shows the same code.
- The limits of the handshake are the broker's own and not part of this contract. Today a handshake may take 5 seconds, 8 may run at once, an agent may start 6 sessions a minute, and a pending agent may change its key once a minute and 20 times in all. Of failed handshakes the broker records one audit row a minute for each cause and known id.

### Tool calls

Every tool answers a `tool_outcome`: the request's status and, only when a fresh proof released it, a result.

- When the owner must decide, the state is `pending_approval`, with the time the card expires, and the result is null. The agent polls `fabric_request_status` with the `request_id`.
- When the request is blocked, the status carries `denial`: the failing rule with a bounded explanation.
- A status never carries calendar content. Reading a result (`fabric_request_result`) is itself an action with its own proof.
- In an event view, `occurrence` is disclosed together with the field `time`. It is null for an occurrence of an all-day series, whose original start is a date and not a time.
- When no proof decided the request, the state is `undecided` and the status carries nothing more. Why it is undecided is for the owner only (section 7).
- Past a request limit the broker answers the error `busy`, which is retryable and names the limit, its maximum and a wait in milliseconds. No request is created.
- While the rule prover is not ready the broker may refuse a tool call with the error `prover_unavailable`, which is retryable and carries no detail. No request is created. A request the broker accepted all the same ends `undecided`.

When the answer comes:

- The broker answers an action when its request waits for the owner, was applied, has an unknown outcome, has ended or has a released result. If none of these happens within a wait that the broker bounds, it answers with the status of that moment. So an Ask returns `pending_approval`; an Allow of a calendar list returns the list; and an allowed read is answered when its result is held, after which the agent calls `fabric_request_result`. No answer is held while a person decides.
- `fabric_request_status` is answered at once.
- Answers on one connection may come in any order. An adapter matches them by `id`.
- The broker closes a connection when its session ends, when its agent is removed and when the broker quits. It also closes it after `not_permitted` to a frame before an approved handshake, after every `agent_prove` that does not answer `approved`, `pending` included, and after every refused `agent_hello`. An adapter then connects and proves its key again.

Which answers carry a result:

- Only two answers ever carry a result. The answer to `fabric_calendar_list` carries the list that its own fresh proof released. The answer to `fabric_request_result` carries the result that its own proof of the release permitted.
- A read's own answer never carries its result. The agent asks for it with `fabric_request_result`, and the rules are proved again at that moment. So a read takes two calls, and every result leaves the broker by one path. This is the owner's decision of 2026-11-18 (plan section 15.3, NOV18-READ-CALLS). For version 1 of Fabric Calendar it departs from the plan's sentence in section 9.2 that an Allow returns the proven filtered read result.
- Only a call that still waits receives a result. If the call has ended, the request ends `cancelled` and nothing is released.
- `fabric_request_result` for a calendar list, for another request for a result, or for a request that has no result ends `terminal_failure` with the code `invalid_arguments`.
- A release that needs the owner's approval ends `undecided`, and nothing is released. No card is opened for a release in version 1 of Fabric Calendar (NOV18-RELEASE-CARD): the agent reads again, and that read can ask the owner with an ordinary card.

When an adapter stops waiting:

- `agent_cancel` tells the broker that the adapter no longer waits for the answer to one call on this connection, named by that call's frame id. The broker drops the waiting call and never answers it. This is the one case in which a request frame gets no response.
- The request itself goes on as it does when a connection ends. A calendar list or a release whose call is gone ends `cancelled` and releases nothing. A proposal stays recorded, and its card stays open.
- The cancel is not a request of its own and creates none. Its answer says whether a call with that id was waiting (`held`). A cancel for an id that is not waiting changes nothing.

Two things an adapter does are outside this contract. The error `broker_not_running` is made by an adapter that cannot reach the broker; the broker never sends it. And an adapter may show its own client a member that can be null as optional, and write the `null` itself: on the channel every member is present.

No tool carries a warning: warnings are for the owner only in version 1 of Fabric Calendar. No tool approves, grants, edits a profile, deletes a calendar, changes sharing or reaches an arbitrary Google URL.

## 9. The request lifecycle

`data/request-lifecycle.json` lists the 26 states and the legal transitions. A request starts in `received`. It is open while its state is not terminal; the open-request limits count open requests, and an open approval card is a request in `pending_approval`.

The 24 transitions marked `plan` are the state grammar of plan section 11. The 18 marked `added` close it, and each cites the plan text it rests on.

A request's `status` is its state and what that state always carries, and the schema will not accept one without the other:

- `blocked` carries `denial`: the failing rule, an explanation and the id of the proof. A denial without its rule or its proof is not a valid message.
- `terminal_failure` carries `failure` with a code.
- `pending_approval` carries `approval_expires_at`. An unanswered card expires after `approval_expiry_seconds`.
- Every other state carries nothing more.

The reason a request is `undecided` is in its details (section 7), not in its status.

A request in `needs_owner_resolution` is open: it stays in the snapshot until the owner resolves it (`resolve_outcome`, section 7), and it counts against the open-request limits. `resolved_as_applied` and `resolved_as_absent` record the owner's statement, not an observation of the broker. The interface must not show them as confirmed by Google.

`completed` ends a request that the broker serves itself, without a call to Google: a calendar list, and the release of a result (`fabric_request_result`). It is reached from `proving_fresh_permit`.

## 10. Profiles, grants and durations

A profile carries one mode per row of the policy matrix (`data/policy-matrix.json`). The built-in profiles Careful and Flexible are constants of the schema. A custom profile may set any row to any mode, except that `delete_event` is `ask` and `delete_calendar_or_change_sharing` is `block` in every profile.

A scope names calendars, each with its account, and where they apply a date window, fields, recipients and who is notified. A member that applies and is null covers nothing. No value of a scope stands for "everything".

A recipient rule names exact addresses or recipient sets. No rule covers everyone, so an invitation to an address outside every approved recipient set always asks. A recipient set is a selector with a source: curated in Fabric, or imported from Google People.

An approval has one of three durations: `one_request`, `session`, `until_revoked`. A grant's `term` binds it: to one action digest, to one agent session, or to nothing but revocation, in which case it carries `"expires_at": null`. A session ends on disconnect, on app quit, on broker restart, or after `session_idle_timeout_seconds` without activity.

A grant's `origin` names what created it: the owner's approval of a request (`request_id` and `approval_digest`), or the owner's approval of the agent (`enrollment_id`, which is the agent's id).

A one-request grant for a read ends with the reason `result_expired` when the result the broker held for it passed its life without being released.

The calendar list (`fabric_calendar_list`) names no calendar. Its permission comes with the enrollment: when the owner approves an agent, the broker creates a grant until revoked whose only tool is `fabric_calendar_list` and whose scope names no calendar (plan section 15.2, NOV17-LIST). Its `purpose` is the empty text, the other four members of its scope are null, and it ends only when the owner revokes it (`revoked_by_owner`) or removes the agent (`agent_removed`). To the rules it is a grant like any other. Under a row whose mode is `allow` it permits the list; revoking it, or removing the agent, ends it. A scope that names no calendar covers no calendar, so such a grant serves no tool that names one. A calendar list request that needs the owner's approval, for example after the owner revoked that grant, gets a card whose scope names no calendar. An approval of that card for this one request delivers nothing in version 1 of Fabric Calendar: by the time the owner decides, the agent's call has ended, so the list is cancelled and the approval is used up, and the agent's next list asks again (plan section 15.3, NOV18-LIST-CARD). Under a row whose mode is `ask` no list completes, then, and the rule below that the broker proposes no duration whose grant would not serve the request does not hold for a list. The card must say so.

The durations are ordered from shortest to longest: `one_request`, `session`, `until_revoked`. A card's `proposed_duration` is the longest one the owner may choose for that request. The interface offers it and the shorter ones: the owner can narrow a duration, like a scope, and cannot widen it. When a card opens, the interface selects the shortest duration, `one_request`; a longer one is the owner's own choice (plan section 15.2, NOV17-DURATION).

The broker does not propose a duration whose grant would not serve the request. Under a row whose mode is `ask`, every request needs the owner's own approval, so a grant for a session or until revoked does not serve it and the card proposes `one_request`. The matrix states two exceptions, where such a grant does serve: a read of event details (CD-M02), and an edit of an event Fabric created in Agent Drafts (CD-M05). This is the rule theory's reading of the matrix; the owner confirmed it on 2026-11-16.

## 11. The authorization

The broker's exact-action authorization is the record `authorization` in the owner schema, profile `fabric-broker-authorization/1`. It names the request, the agent, the action digest, the grant and the live proof that permitted the action, and it is used once. `lifetime_seconds` is the constant 60, and `expires_at` is `issued_at` plus that.

It is the internal authorization type that plan section 8 allows. The CBOR token of `crates/fabric-token` is not changed and is not used by this protocol.

## 12. Versions

`data/version.json` holds the protocol name, the schema version and the authorization profile. A change that an older peer could not read needs a new protocol major. Every other change raises `schema_version`.

Version 2 added why the rule prover is unavailable (`prover_problem` in `health`), why a request is undecided (`undecided_reason` in the request's details) and the agent error `prover_unavailable`. The protocol name stays `fabric-desktop/1`: `hello`, the frame format and the error envelope did not change, so a version 1 peer can still read the refusal `unsupported_schema_version`.

Version 3 added the commands that the plan's text needs and its method table does not list: selecting calendars and naming Agent Drafts, adding, approving and removing an agent, and resolving an unknown outcome by hand. With them came the agent's enrollment state and pairing code, the configuration texts, the agent channel's handshake, three request states, a second origin of a grant, and a scope that names no calendar. It also added the list of ended requests, the deletion of a custom profile, the prover problem `not_installed`, the state of the agent channel in `health`, what the broker knows about an account's connection to Google, a command to list an account's calendars again, and in a request's details the notifications, the broker's own proofs and the releases. Every request, card, grant and account that was valid under version 2 is valid under version 3, so what a broker stored stays readable.

## 13. Fixtures and the contract suite

Both languages read the same files in `fixtures/`, compute every result themselves, and compare it with the expectation in the file. The expectations were computed by `reference/reference.py`, a third reading of this README in Python that shares no code with either side.

| File | Form | What is checked |
|---|---|---|
| `strict-json.json` | `{"accept": [{"name", source, "canonical"}], "reject": [{"name", source}]}` | Section 2 and section 3. |
| `scalars.json` | `{"<schema reference>": {"accept": [...], "reject": [...]}}` | Each pattern and bound, value by value. |
| `digests.json` | `[{"name", "domain", "subject", "canonical", "digest"}]` | Section 4. |
| `lifecycle.json` | `[{"name", "states", "valid", "first_invalid"}]` | A sequence is valid when it starts at `received` and every step is a listed transition. `first_invalid` is the index of the first state that breaks this. |
| `messages/*.json` | `{"description", "schema", source, "expect"}` | Section 5 against the named schema. |
| `handshake.json` | `[{"name", "seed", "schema_version", "enrollment_id", "broker_nonce", "public_key", "pairing_code", "text", "signature"}]` | Section 8: the pairing code of the public key, the handshake text of the four values, and that the signature verifies under the public key. The Python reference also derives the public key and the signature from the seed. |

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
handshake  <name>         <pairing code> <sha256 of the handshake text>
mutant     <id>           accept | reject <layer> <error>
```

To add a fixture, write it with `"canonical"` and the digests null, or with the layer and error it must be refused with, and run `python3 contracts/desktop/v1/reference/reference.py bless`. It fills in what an accepted fixture expects and reports any fixture that is not judged as written. For a handshake fixture, write the seed, the version, the id and the nonce; `bless` fills in the rest.

`scripts/verify-contracts.sh` runs both sides, compares the two reports byte for byte, then does the same for the mutants that the Rust side derives from the accepted messages: at every node of every accepted message, a member removed, an unknown member added, the value replaced by values of other types and by its neighbours, an object written as an array. The Rust verdict comes from the serde types and the TypeScript verdict from the schemas, so any place where the two describe different languages fails the suite. When Python 3 is installed the suite also runs the reference, whose report and verdicts must equal the Rust ones.

The Rust crate also keeps `tests/data/stored-version-2.json`: values of the types a broker stores, and its stored answers to commands, as schema version 2 wrote them. A test checks that each still decodes and encodes to the same bytes, and that the reader for stored answers gives a version 2 answer to `pause_agent` the members version 3 added.

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

31. The 15 transitions added in version 1, and the terminal states. `undecided` and `absence_confirmed` are terminal. `needs_owner_resolution` was terminal until version 3 (choice 54).
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
45. The broker does not propose a duration whose grant would not serve the request. Which of the durations that would serve it the broker proposes is not fixed (section 15).

Version 3:

46. `select_calendars` sets the whole selection of one account. The Agent Drafts calendar is named per account, and an account has at most one.
47. Whether a change of selection ends a grant is left to the broker. A grant that ends is announced by its own notice.
48. An agent's enrollment is a state of its record (`pending`, `approved`, `removed`), and it starts in the application: the owner adds the agent, so its id exists before its bridge first runs. This is the bridge design's reading (`docs/plans/calendar-desktop-w06-design.md`, decision A18) of the plan's sentence that the configuration holds only the public enrollment id and the executable's location. The plan also says that enrollment starts with the bridge generating its key; under this reading the key comes second.
49. Approving an agent gives the calendar list as a grant until revoked. The plan binds an "allowed tool scope" to the agent at enrollment (its section 4.3), and the owner's answer NOV17-LIST says what that scope holds. As a grant it is shown with the others and can be revoked alone, and the rule theory needs no change.
50. That grant's scope names no calendar, so a scope's list of calendars may be empty. Such a scope covers no calendar. A calendar list request that needs approval gets a card with such a scope.
51. A grant's origin has two forms that are told apart by their members, with no `kind` member, so that a grant stored under version 2 keeps its form.
52. Removing an agent cancels its requests that were not handed to Google and lets the others run to their end. The record of a removed agent stays in the snapshot.
53. The owner can resolve an outcome by hand only in `needs_owner_resolution`, after the broker's own reconciliation gave up. The request ends in one of two new states, which say that the owner settled the outcome and the broker did not observe it.
54. `needs_owner_resolution` is no longer terminal, so that the request stays in the snapshot until it is resolved. It then counts against the open-request limits.
55. `resolve_outcome` answers `stale_request`, as `decide_request` does, when the request changed or is in another state.
56. `list_ended_requests`: newest first, at most 100 at a time, the next page named by the id of the last request seen. The owner's go for version 3 (NOV17-SCHEMA3) does not cover it; plan section S05 needs it to show ended requests.
57. `delete_custom` in `save_profile`, and that the profile in force cannot be deleted. The owner's go for version 3 does not cover it.
58. `not_installed` is a prover problem of its own, because the owner can act on it and cannot act on `not_configured`. It follows from the plan's answer that Isabelle is installed by a script (CD-Q09).
59. The interface selects `one_request` first (NOV17-DURATION).
60. The owner cannot change an agent's name or destination after adding it. The record does not keep the kind of client; the destination follows from it.
61. `approve_agent` carries the pairing code, and a code that is not the current key's gets `invalid_params`.
62. A pairing code is written as the owner reads it, with its two hyphens.
63. `get_agent_configuration` returns all three texts, or null in their place when the broker cannot name an installed bridge. It does not say why.
64. The handshake is two methods of the agent channel that are not tools. The signed text and the pairing code begin with the first line of the digest formula and have domains of their own. Keys, nonces and signatures are lowercase hexadecimal. These are the bridge design's shapes (its section 4.4).
65. A tool frame before an approved handshake gets `not_permitted`.
66. When the answer to an action comes (section 8). How long the broker waits at most is the broker's to set.
67. In a request's details, `approval` is the decision on the card now shown, so it is null while a request waits again after an earlier approval. A grant that the earlier approval created still names that approval in its origin.
68. A request the broker serves itself ends in `completed`, a state of its own reached from `proving_fresh_permit`. Such a request has no call to Google, so it cannot end `reconciled`. The broker's design asked for the state and proposed its name.
69. A version 2 answer to `pause_agent` that a broker stored is read by a reader of its own, which fills in the agent members version 3 added. For it `added_at` may be null. The channel stays strict: every member is required there.
70. What the broker knows about an account's connection to Google is a record beside the account, not new members of the account, because the broker stores accounts as written. `missing_scope` is Google's name of the scope, as text. The meanings of the connection problems follow their names and the Google design (CD-W05).
71. `refresh_calendars` answers only that it is committed; the list arrives as changes.
72. The agent channel's state in `health` is two members, in the pattern of the prover's. A null problem while not listening means there is none to name.
73. A system proof is shown with the outcomes of a proof report and with the kind of prover. A release record names its request, its proof and its grants.
74. The handshake's limits are the broker's and are stated in section 8 as what they are today, not as part of the contract.
75. The busy limit `sessions_per_minute_per_agent` has no member in `limits`.
76. The capability `practice_calendar`, and `practice` in the answer to `agent_prove`.
77. The destination of each kind of client is contract data (`data/agent-clients.json`), and it is a label, not a checked fact.
78. The grant made by an agent's approval has the empty text as its purpose: no agent wrote one.
79. `select_calendars` takes at most 250 calendars, the most the Google design lists for one account.
80. Three rules follow the owner's decisions of 2026-11-18, recorded in plan section 15.3: a read takes two calls (NOV18-READ-CALLS), a release never gets a card (NOV18-RELEASE-CARD), and an approval of a calendar list for one request delivers nothing (NOV18-LIST-CARD). The first departs, for version 1, from the plan's sentence that an Allow returns the proven filtered read result.
81. `agent_cancel` ends the wait for one answer and nothing more. It is answered, with `held`; the call it names is not. The broker's design asked for the message and left its name to the contract.

## 15. Not covered

- Commands that the plan's text needs and its method table does not list: starting a People import, the task composer for the embedded runtime, and a message with which an agent withdraws a proposal (`agent_cancel` only ends the wait for an answer).
- How long the broker keeps ended requests.
- Actions the broker starts itself, such as refresh and reconciliation reads, and the formal context with its digest (CD-W03, CD-W04).
- A marker for practice accounts, attempts and reports.
- Which of the durations that would serve a request the broker proposes on its card.
- The names of the rules. The contract fixes their form; the rule theory names them. The rule names in the fixtures are examples.
- Facts that tie several members together, which a schema cannot state and the broker must uphold: that a snapshot's requests are open and its grants have not ended, that an expansion names each row once, that the active profile exists, that an approval card expires 24 hours after it opens, that a card proposes no duration whose grant would not serve its request, that a scope for a tool that names calendars names at least one, that the Agent Drafts calendar of an account is selected and writable, that a grant made by an agent's approval covers the calendar list alone, how the members of an agent go together (section 7), and that an authorization expires 60 seconds after it is issued (the Rust crate offers a check for the last).
