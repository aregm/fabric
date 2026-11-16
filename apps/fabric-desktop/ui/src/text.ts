// The words on the page: request states, operations, the facts of an approval card,
// activity lines and dates. Pure functions; nothing here touches the page.
import type {
  Account, AccountState, Action, Agent, Approval, Attempt, Authorization, CalendarAccess, CalendarRef, Change, DurationKind,
  EventField, EventTime, EventView, Grant, GrantEndReason, GrantTerm, Health, ProofReport, RecipientSet, RequestDetails,
  RequestState, ResponseStatus, Scope, SendUpdates, ToolName, Window as TimeWindow,
} from "../../../../contracts/desktop/v1/typescript/src/contract.ts";
import type { HostState } from "./host.ts";

// Every one of the 23 states has a text; the Record type makes the compiler check that.
export const STATE_TEXT: Record<RequestState, string> = {
  received: "Received, checking",
  validating: "Received, checking",
  proving_classification: "Received, checking",
  allowed: "Allowed by your settings; Fabric checks it once more before it is done",
  pending_approval: "Waiting for your decision",
  blocked: "Blocked by your settings",
  undecided: "Not done: Fabric could not prove that it is allowed",
  approved: "You allowed it; Fabric checks it once more before it is done",
  denied: "You denied it",
  expired: "Not done: nobody decided in time",
  cancelled: "Cancelled",
  stale: "Not done: something changed after the request was made, so it is out of date",
  proving_fresh_permit: "Fabric checks it once more before it is done",
  queued: "Allowed; waiting to be sent to the calendar",
  dispatching: "Being sent to the calendar",
  applied: "Done: the calendar accepted it. Fabric has not confirmed it by reading it back yet",
  reconciled: "Done and confirmed",
  retry_wait: "The calendar did not take it; waiting to try again",
  outcome_unknown: "Not known whether the calendar took it",
  terminal_failure: "Failed",
  reconciling: "Fabric is asking the calendar whether it was done",
  absence_confirmed: "Not done: the calendar confirms that nothing changed",
  needs_owner_resolution: "Fabric cannot tell what happened and needs you to check",
};

// The states after which a request never changes (data/request-lifecycle.json; a test compares them).
export const TERMINAL: Record<RequestState, boolean> = {
  received: false, validating: false, proving_classification: false, allowed: false, pending_approval: false,
  blocked: true, undecided: true, approved: false, denied: true, expired: true, cancelled: true, stale: true,
  proving_fresh_permit: false, queued: false, dispatching: false, applied: false, reconciled: true,
  retry_wait: false, outcome_unknown: false, terminal_failure: true, reconciling: false, absence_confirmed: true,
  needs_owner_resolution: true,
};

export const CHECKING_STATES: readonly RequestState[] = ["received", "validating", "proving_classification"];

export const OPERATION: Record<ToolName, string> = {
  fabric_calendar_list: "See the names of your calendars",
  fabric_calendar_availability: "See when you are free or busy",
  fabric_calendar_read: "Read the details of events",
  fabric_calendar_propose_create: "Create an event",
  fabric_calendar_propose_update: "Change an event",
  fabric_calendar_propose_invitation: "Change who is invited to an event",
  fabric_calendar_propose_response: "Answer an invitation",
  fabric_calendar_propose_delete: "Delete an event",
  fabric_request_status: "Check how an earlier request went",
  fabric_request_result: "Read the result of an earlier request",
};

// What the owner is told after allowing a request, by the duration chosen.
export const ALLOWED_TEXT: Record<DurationKind, string> = {
  one_request: "You allowed this, for this one request.",
  session: "You allowed this. The permission lasts until this agent's current session ends.",
  until_revoked: "You allowed this. The permission lasts until you take it back.",
};

export const DURATION_TEXT: Record<DurationKind, string> = {
  one_request: "Only this request",
  session: "Until this agent's current session ends",
  until_revoked: "Until I take it back",
};

const NOTIFIED: Record<SendUpdates, string> = {
  all: "Everyone invited gets an email from Google.",
  external_only: "Only guests outside your organization get an email from Google.",
  none: "Nobody gets an email.",
};

const FIELD_TEXT: Record<EventField, string> = {
  time: "time", title: "title", location: "place", description: "description", attendees: "guests",
  organizer: "organizer", own_response: "your answer",
};

export const ACCOUNT_TEXT: Record<AccountState, string> = {
  awaiting_consent: "Waiting for you to finish connecting it",
  connected: "Connected",
  consent_denied: "Not connected: access was refused while connecting it",
  partial_grant: "Connected, but with less access than Fabric needs",
  cancelled: "Not connected: connecting it was cancelled",
  account_mismatch: "Not connected: another account was signed in than the one expected",
  reauthentication_required: "Needs you to sign in again",
  disconnected: "Disconnected",
};

export const ACCESS_TEXT: Record<CalendarAccess, string> = {
  owner: "you own it",
  writer: "you can change its events",
  reader: "you can see its events",
  free_busy_reader: "you can see only when it is free or busy",
};

export const PROVER_TEXT: Record<Health["prover"], string> = {
  ready: "Running",
  starting: "Starting",
  unavailable: "Not running",
};

export const STORE_TEXT: Record<Health["store"], string> = {
  ready: "Open",
  locked: "Cannot be opened: another broker process holds it",
  keychain_refused: "Cannot be opened: macOS did not let the broker use the store key in the Keychain",
};

// What the owner is told when the broker cannot open its store, by the reason it gives.
export const STORE_PROBLEM: Record<Exclude<Health["store"], "ready">, string> = {
  locked: "The broker cannot open its store.",
  keychain_refused: "macOS did not let the broker use the store key in the Keychain.",
};

// The zone the page shows times in. The caller may fix it (tests do).
export function localZone(): string {
  return new Intl.DateTimeFormat().resolvedOptions().timeZone;
}

// A date and time in the local zone, with the zone named: "Friday, November 20, 2026 at 10:00 AM Central Standard Time".
export function formatInstant(text: string, zone: string = localZone()): string {
  const ms = Date.parse(text);
  if (Number.isNaN(ms)) return text;
  return new Intl.DateTimeFormat("en", {
    weekday: "long", year: "numeric", month: "long", day: "numeric", hour: "numeric", minute: "2-digit",
    timeZone: zone, timeZoneName: "long",
  }).format(ms);
}

// A time of day with seconds, in the local zone, with the zone named.
export function formatTimeOfDay(ms: number, zone: string = localZone()): string {
  return new Intl.DateTimeFormat("en", {
    hour: "numeric", minute: "2-digit", second: "2-digit", timeZone: zone, timeZoneName: "long",
  }).format(ms);
}

function formatEventTime(t: EventTime, zone: string): string {
  const shown = formatInstant(t.date_time, zone);
  return t.zone === zone ? shown : `${shown} (the event's own time zone is ${t.zone})`;
}

function formatWindow(w: TimeWindow, zone: string): string {
  return `From ${formatInstant(w.start, zone)} until ${formatInstant(w.end, zone)}`;
}

// What the page knows about agents and accounts, to name them in words.
export interface Names {
  readonly agents: ReadonlyMap<string, Agent>;
  readonly accounts: ReadonlyMap<string, Account>;
  readonly zone: string;
}

export function agentName(names: Names, agentId: string): string {
  return names.agents.get(agentId)?.registered_name ?? `An agent this app does not know (${agentId})`;
}

export function accountName(names: Names, accountId: string): string {
  return names.accounts.get(accountId)?.email ?? accountId;
}

export function calendarName(names: Names, ref: CalendarRef): string {
  const calendar = names.accounts.get(ref.account_id)?.calendars.find((c) => c.calendar_id === ref.calendar_id);
  return calendar === undefined ? ref.calendar_id : `${calendar.name} (${accountName(names, ref.account_id)})`;
}

// One line of the card's facts: a term and one or more values.
export interface Fact {
  readonly term: string;
  readonly values: readonly string[];
  readonly untrusted?: boolean;
}

const RESPONSE_TEXT: Record<ResponseStatus, string> = {
  needs_action: "has not answered", accepted: "accepted", declined: "declined", tentative: "maybe",
};

// The event as it is now in the calendar, before the change ("Now: title", …, "Guests now").
// A field the broker did not read is null and left out.
function beforeFacts(e: EventView, zone: string): Fact[] {
  const facts: Fact[] = [];
  const add = (term: string, ...values: string[]): void => void facts.push({ term, values });
  if (e.title !== null) add("Now: title", e.title === "" ? "No title" : e.title);
  if (e.time?.kind === "timed") {
    add("Now: starts", formatEventTime(e.time.start, zone));
    add("Now: ends", formatEventTime(e.time.end, zone));
  } else if (e.time?.kind === "all_day") {
    add("Now: dates", `All day, from ${e.time.start_date} until ${e.time.end_date}`);
  }
  if (e.location !== null) add("Now: place", e.location === "" ? "None" : e.location);
  if (e.description !== null) add("Now: description", e.description === "" ? "None" : e.description);
  if (e.attendees !== null) {
    add("Guests now", ...(e.attendees.length === 0 ? ["Nobody"] : e.attendees.map((a) => `${a.email} (${RESPONSE_TEXT[a.response]})`)));
  }
  return facts;
}

function actionFacts(action: Action, before: EventView | null, names: Names): Fact[] {
  const a = action.arguments;
  const z = names.zone;
  const facts: Fact[] = [];
  const add = (term: string, ...values: string[]): void => void facts.push({ term, values });
  if ("account_id" in a) add("Account", accountName(names, a.account_id));
  if ("calendar_id" in a) add("Calendar", calendarName(names, { account_id: a.account_id, calendar_id: a.calendar_id }));
  if ("calendar_ids" in a) add("Calendars", ...a.calendar_ids.map((id) => calendarName(names, { account_id: a.account_id, calendar_id: id })));
  if (before !== null) facts.push(...beforeFacts(before, z));
  switch (action.tool) {
    case "fabric_calendar_availability":
      add("Time span", formatWindow(action.arguments.window, z));
      break;
    case "fabric_calendar_read":
      add("Time span", formatWindow(action.arguments.window, z));
      add("What it reads", action.arguments.fields.map((f) => FIELD_TEXT[f]).join(", "));
      break;
    case "fabric_calendar_propose_create": {
      const e = action.arguments.event;
      add("Title", e.title);
      add("Starts", formatEventTime(e.start, z));
      add("Ends", formatEventTime(e.end, z));
      add("Place", e.location ?? "None");
      add("Description", e.description ?? "None");
      break;
    }
    case "fabric_calendar_propose_update": {
      const c = action.arguments.changes;
      add("Event", action.arguments.event.event_id);
      if (c.title !== null) add("New title", c.title);
      if (c.start !== null) add("New start", formatEventTime(c.start, z));
      if (c.end !== null) add("New end", formatEventTime(c.end, z));
      if (c.location !== null) add("New place", c.location === "" ? "Removed" : c.location);
      if (c.description !== null) add("New description", c.description === "" ? "Removed" : c.description);
      break;
    }
    case "fabric_calendar_propose_invitation":
      add("Event", action.arguments.event.event_id);
      add("Guests after the change", ...(action.arguments.attendees.length === 0 ? ["Nobody"] : action.arguments.attendees));
      break;
    case "fabric_calendar_propose_response": {
      const t = action.arguments.target;
      add("Event", t.kind === "event" ? t.event.event_id : `One occurrence of ${t.occurrence.instance_id}`);
      add("Answer", action.arguments.response === "accepted" ? "Accept" : "Decline");
      break;
    }
    case "fabric_calendar_propose_delete":
      add("Event", action.arguments.event.event_id);
      break;
    case "fabric_request_result":
      add("Earlier request", action.arguments.request_id);
      break;
    case "fabric_calendar_list":
      break;
  }
  return facts;
}

function notified(action: Action): string {
  if ("send_updates" in action.arguments) return NOTIFIED[action.arguments.send_updates];
  return action.tool === "fabric_calendar_propose_create" ? "Nobody: a new event has no guests." : "Nobody: this only reads.";
}

// Where an agent's data goes, in words.
export function destinationText(agent: Agent | undefined): string {
  if (agent === undefined) return "Not known";
  return agent.destination.kind === "local" ? `Stays on this computer: ${agent.destination.endpoint}`
    : `Leaves this computer: ${agent.destination.processor}`;
}

// The facts of an approval card, in reading order up to "who is notified".
export function cardFacts(details: RequestDetails, names: Names): Fact[] {
  return [
    { term: "Who asks", values: [agentName(names, details.request.agent_id)] },
    { term: "Where its data goes", values: [destinationText(names.agents.get(details.request.agent_id))] },
    ...askedFacts(details, names),
  ];
}

// What was asked: the agent's words (untrusted), the operation, the event now and the change, the
// warnings and who is notified.
export function askedFacts(details: RequestDetails, names: Names): Fact[] {
  const facts: Fact[] = [];
  const action = details.private?.action;
  if (action === undefined) {
    facts.push({ term: "What it wants to do", values: [OPERATION[details.request.tool]] });
    facts.push({ term: "Details", values: ["The broker did not send the details of this request."] });
    return facts;
  }
  if ("task" in action.arguments) {
    facts.push({ term: "In the agent's own words (not checked by Fabric)", values: [action.arguments.task], untrusted: true });
  }
  facts.push({ term: "What it wants to do", values: [OPERATION[action.tool]] });
  facts.push(...actionFacts(action, details.private?.before ?? null, names));
  for (const w of details.private?.warnings ?? []) facts.push({ term: "Warning", values: [w.explanation] });
  facts.push({ term: "Who is notified", values: [notified(action)] });
  return facts;
}

// ---- Review: request states, details, proofs, permissions and agents ----

// What the owner can do today for a request whose outcome Fabric does not know (plan S05).
export const OWNER_CAN_DO: Partial<Record<RequestState, string>> = {
  outcome_unknown: "Look in your calendar to see whether the change is there. Fabric keeps asking the calendar. This build has no way to mark it as resolved.",
  needs_owner_resolution: "Look in your calendar to see whether the change is there. This build has no way to mark it as resolved.",
};

export const SESSION_UNAVAILABLE = "This agent is not connected now, so there is no session to tie the permission to.";
export const DURATION_NOTE = "A permission that lasts longer is used for later requests only where your profile allows it; where the profile says Ask, the agent still asks each time.";
export const ENDED_REQUESTS_NOTE = "Requests that ended before this window opened are not listed; this build has no way to ask the broker for them.";
export const REVOKE_NOTE = "Taking a permission back stops calls that have not been sent yet. It cannot undo what the calendar already accepted, and a call already on its way may still arrive.";
export const PAUSE_NOTE = "A paused agent can do nothing. Resuming does not bring back a permission you took back or one that ended with an earlier session.";
export const NO_PROOF = "The broker has no proof report under this id.";

export const GRANT_END_TEXT: Record<GrantEndReason, string> = {
  consumed: "It was used for the one request it was given for.",
  revoked_by_owner: "You took it back.",
  account_disconnected: "The Google account was disconnected.",
  authority_reduced: "Something it depended on changed, for example a group it names gained someone, so it no longer holds.",
  agent_removed: "The agent was removed.",
  session_disconnected: "The agent's session ended because the agent disconnected.",
  session_idle_timeout: "The agent's session ended after a long time without activity.",
  app_quit: "The agent's session ended because the app was quit.",
  broker_restart: "The agent's session ended because the broker restarted.",
};

export function termText(term: GrantTerm, agent: string): string {
  const words: { readonly [K in GrantTerm["kind"]]: string } = {
    one_request: "Only the one request it was given for",
    session: `Until ${agent}'s session ends`,
    until_revoked: "Until you take it back. It has no end date.",
  };
  return words[term.kind];
}

export const PROOF_PURPOSE: Record<ProofReport["purpose"], string> = {
  classification: "Deciding whether the request is allowed, needs your approval or is blocked",
  fresh_permit: "The check just before carrying it out",
};

export const PROOF_OUTCOME: Record<ProofReport["outcome"], string> = {
  permit: "Allowed",
  need_approval: "Needs your approval",
  block: "Blocked",
  undecided: "Not decided",
};

export const VERDICT_TEXT: Record<ProofReport["rule_verdicts"][number]["verdict"], string> = {
  holds: "holds",
  fails: "fails",
  undecided: "not decided",
};

export const ATTEMPT_TEXT: Record<NonNullable<Attempt["outcome"]>, string> = {
  applied: "The calendar accepted it",
  transient_failure: "The calendar did not take it; Fabric tried again",
  terminal_failure: "Failed; Fabric does not try again",
  outcome_unknown: "Not known whether the calendar took it",
};

// A proof report: what it was for, the outcome, the failing rule, every verdict (failing first), and
// what it was checked with. A rule's name is shown as the broker gives it.
export function proofFacts(p: ProofReport, zone: string): Fact[] {
  const order = { fails: 0, undecided: 1, holds: 1 } as const;
  const verdicts = [...p.rule_verdicts].sort((a, b) => order[a.verdict] - order[b.verdict]);
  return [
    { term: "What it was for", values: [PROOF_PURPOSE[p.purpose]] },
    { term: "Outcome", values: [PROOF_OUTCOME[p.outcome]] },
    { term: "Failing rule", values: [p.failing_rule ?? "None"] },
    { term: "Rules", values: verdicts.length === 0 ? ["The check gave no verdict on any rule."] : verdicts.map((v) => `${v.rule}: ${VERDICT_TEXT[v.verdict]}`) },
    { term: "Isabelle version", values: [p.isabelle_version] },
    { term: "Theory hash", values: [p.theory_hash] },
    { term: "Context digest", values: [p.context_digest] },
    { term: "Action digest", values: [p.action_digest] },
    { term: "Started", values: [formatInstant(p.started_at, zone)] },
    { term: "Took", values: [`${p.elapsed_ms} ms`] },
  ];
}

// A scope in words: calendars by name, the dates, the fields, the guests or the groups by name, who
// is notified. A member that is null is left out: it covers nothing.
export function scopeFacts(scope: Scope, names: Names, groups: ReadonlyMap<string, RecipientSet>): Fact[] {
  const facts: Fact[] = [{ term: "Calendars", values: scope.calendars.map((c) => calendarName(names, c)) }];
  if (scope.window !== null) facts.push({ term: "Dates", values: [formatWindow(scope.window, names.zone)] });
  if (scope.fields !== null) facts.push({ term: "What it may read", values: [scope.fields.map((f) => FIELD_TEXT[f]).join(", ")] });
  const r = scope.recipients;
  if (r?.kind === "exact") facts.push({ term: "Guests", values: r.addresses.length === 0 ? ["Nobody"] : [...r.addresses] });
  if (r?.kind === "recipient_sets") {
    facts.push({
      term: "Groups",
      values: r.recipient_set_ids.map((id) => groups.get(id)?.definition.name ?? `A group this app does not know (${id})`),
    });
  }
  if (scope.send_updates !== null) facts.push({ term: "Who is notified", values: [NOTIFIED[scope.send_updates]] });
  return facts;
}

export function approvalFacts(a: Approval, names: Names, groups: ReadonlyMap<string, RecipientSet>): Fact[] {
  const facts: Fact[] = [
    { term: "Decision", values: [a.decision === "approve" ? "You allowed it" : "You denied it"] },
    { term: "Decided", values: [formatInstant(a.decided_at, names.zone)] },
  ];
  if (a.decision === "approve") {
    facts.push({ term: "How long it lasts", values: [DURATION_TEXT[a.duration.kind]] });
    facts.push(...scopeFacts(a.scope, names, groups).map((f) => ({ ...f, term: `Approved: ${f.term.toLowerCase()}` })));
    facts.push({ term: "Permission it created", values: [a.grant_id] });
  }
  facts.push({ term: "Approval digest", values: [a.approval_digest] });
  return facts;
}

export function authorizationFacts(a: Authorization, zone: string): Fact[] {
  return [
    { term: "Issued", values: [formatInstant(a.issued_at, zone)] },
    { term: "Expires", values: [formatInstant(a.expires_at, zone)] },
    { term: "Used", values: [a.consumed_at === null ? "Not used" : formatInstant(a.consumed_at, zone)] },
  ];
}

export function attemptFacts(a: Attempt, zone: string): Fact[] {
  return [
    { term: "Started", values: [formatInstant(a.started_at, zone)] },
    { term: "Finished", values: [a.finished_at === null ? "Not finished" : formatInstant(a.finished_at, zone)] },
    { term: "Result", values: [a.outcome === null ? "Not known yet" : ATTEMPT_TEXT[a.outcome]] },
  ];
}

// The heading of a permission: "<agent>: <operations>".
export function grantTitle(g: Grant, names: Names): string {
  return `${agentName(names, g.agent_id)}: ${g.tools.map((t) => OPERATION[t]).join("; ")}`;
}

// A permission: the agent's purpose (untrusted), the operations, the scope, how long it lasts, when
// and under which profile it was given, the request it came from, and when and why it ended.
// `origin` names the request it was given for.
export function grantFacts(g: Grant, names: Names, groups: ReadonlyMap<string, RecipientSet>, profile: string, origin: string): Fact[] {
  const over = g.ended !== null;
  const facts: Fact[] = [
    { term: "Its purpose, in the agent's own words (not checked by Fabric)", values: [g.purpose], untrusted: true },
    { term: over ? "What it allowed" : "What it may do", values: g.tools.map((t) => OPERATION[t]) },
    ...scopeFacts(g.scope, names, groups),
    { term: over ? "How long it was given for" : "How long it lasts", values: [termText(g.term, agentName(names, g.agent_id))] },
    { term: "Given", values: [formatInstant(g.created_at, names.zone)] },
    { term: "Given under", values: [`${profile}, version ${g.policy.version}`] },
    { term: "Given for request", values: [origin] },
  ];
  if (g.ended !== null) facts.push({ term: "Ended", values: [`${formatInstant(g.ended.at, names.zone)}. ${GRANT_END_TEXT[g.ended.reason]}`] });
  return facts;
}

// What an agent is: where its data goes, connected or not, its last call, paused or not.
export function agentFacts(a: Agent, zone: string): Fact[] {
  return [
    { term: "Where its data goes", values: [destinationText(a)] },
    { term: "Connected", values: [a.session === null ? "Not connected" : `Since ${formatInstant(a.session.started_at, zone)}`] },
    { term: "Last call", values: [a.last_tool_call_at === null ? "None since the broker started" : formatInstant(a.last_tool_call_at, zone)] },
    { term: "Paused", values: [a.paused ? "Yes: it can do nothing until you resume it" : "No"] },
  ];
}

// One line of the Activity list for a change.
export function changeText(change: Change, names: Names): string {
  switch (change.kind) {
    case "account": return `Account ${change.account.email ?? change.account.account_id}: ${ACCOUNT_TEXT[change.account.state]}`;
    case "agent": return `Agent ${change.agent.registered_name}${change.agent.paused ? " (paused)" : ""}`;
    case "profiles": return "Your permission profiles changed";
    case "recipient_set": return `Group ${change.recipient_set.definition.name} changed`;
    case "recipient_set_deleted": return "A group was deleted";
    case "preferences": return "Your calendar preferences changed";
    case "request": {
      const r = change.request;
      const rule = r.status.state === "blocked" ? ` (rule ${r.status.denial.rule})` : "";
      return `${OPERATION[r.tool]}, from ${agentName(names, r.agent_id)}: ${STATE_TEXT[r.status.state]}${rule}`;
    }
    case "grant": return change.grant.ended === null
      ? `Permission given to ${agentName(names, change.grant.agent_id)}`
      : `Permission of ${agentName(names, change.grant.agent_id)} ended`;
    case "health": return `Rule checker: ${PROVER_TEXT[change.health.prover]}. Store: ${STORE_TEXT[change.health.store]}`;
    case "snapshot_required": return "The broker asked this app to read everything again";
  }
}

// The banner for a state in which the broker is not running. Null when there is no banner.
export function bannerText(state: HostState | null): { readonly title: string; readonly text: string } | null {
  if (state === null || state.name === "connected") return null;
  if (state.name === "conflict") {
    return {
      title: "Fabric Calendar is already open.",
      text: "Another copy of the app is running and uses the broker. Use that window, or quit it and open this one again.",
    };
  }
  if (state.name === "configuration_error") {
    return {
      title: "Fabric Calendar is not installed correctly.",
      text: "The app found a problem before it started the broker, so nothing was started. Installing the app again may fix it.",
    };
  }
  const titles = {
    exited: "The broker stopped.",
    protocol_violation: "The broker sent something this app could not read, so the app stopped it.",
    stopped: "The broker was stopped.",
    start_failed: "The broker could not be started.",
  } as const satisfies Record<typeof state.reason, string>;
  return {
    title: titles[state.reason],
    text: "Nothing can be decided while the broker is stopped. Your choices on open requests are kept. Requests are read again after the restart.",
  };
}
