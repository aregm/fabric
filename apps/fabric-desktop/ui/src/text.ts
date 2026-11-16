// The words on the page: request states, operations, the facts of an approval card,
// activity lines and dates. Pure functions; nothing here touches the page.
import type {
  Account, Action, Agent, CalendarRef, Change, DurationKind, EventField, EventTime, RequestDetails, RequestState,
  SendUpdates, ToolName, Window as TimeWindow,
} from "../../../../contracts/desktop/v1/typescript/src/contract.ts";
import type { HostState } from "./host.ts";

// Every one of the 23 states has a text; the Record type makes the compiler check that.
export const STATE_TEXT: Record<RequestState, string> = {
  received: "Received, checking",
  validating: "Received, checking",
  proving_classification: "Received, checking",
  allowed: "Allowed by your settings",
  pending_approval: "Waiting for your decision",
  blocked: "Blocked by your settings",
  undecided: "Not done: Fabric could not prove that it is allowed",
  approved: "You allowed it; Fabric checks it once more before it is done",
  denied: "You denied it",
  expired: "Not done: nobody decided in time",
  cancelled: "Cancelled",
  stale: "Not done: the calendar changed after the request was made",
  proving_fresh_permit: "Fabric checks it once more before it is done",
  queued: "Waiting to be sent to Google",
  dispatching: "Being sent to Google",
  applied: "Done in Google Calendar; Fabric is confirming it",
  reconciled: "Done and confirmed",
  retry_wait: "Waiting to try again",
  outcome_unknown: "Not known whether Google did it",
  terminal_failure: "Failed",
  reconciling: "Fabric is asking Google whether it was done",
  absence_confirmed: "Not done: Google confirms that nothing changed",
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

function accountName(names: Names, accountId: string): string {
  return names.accounts.get(accountId)?.email ?? accountId;
}

function calendarName(names: Names, ref: CalendarRef): string {
  const calendar = names.accounts.get(ref.account_id)?.calendars.find((c) => c.calendar_id === ref.calendar_id);
  return calendar === undefined ? ref.calendar_id : `${calendar.name} (${accountName(names, ref.account_id)})`;
}

// One line of the card's facts: a term and one or more values.
export interface Fact {
  readonly term: string;
  readonly values: readonly string[];
  readonly untrusted?: boolean;
}

function actionFacts(action: Action, names: Names): Fact[] {
  const a = action.arguments;
  const z = names.zone;
  const facts: Fact[] = [];
  const add = (term: string, ...values: string[]): void => void facts.push({ term, values });
  if ("account_id" in a) add("Account", accountName(names, a.account_id));
  if ("calendar_id" in a) add("Calendar", calendarName(names, { account_id: a.account_id, calendar_id: a.calendar_id }));
  if ("calendar_ids" in a) add("Calendars", ...a.calendar_ids.map((id) => calendarName(names, { account_id: a.account_id, calendar_id: id })));
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

// The facts of an approval card, in reading order up to "who is notified".
export function cardFacts(details: RequestDetails, names: Names): Fact[] {
  const agent = names.agents.get(details.request.agent_id);
  const where = agent === undefined ? "Not known"
    : agent.destination.kind === "local" ? `Stays on this computer: ${agent.destination.endpoint}`
    : `Leaves this computer: ${agent.destination.processor}`;
  const facts: Fact[] = [
    { term: "Who asks", values: [agentName(names, details.request.agent_id)] },
    { term: "Where its data goes", values: [where] },
  ];
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
  facts.push(...actionFacts(action, names));
  for (const w of details.private?.warnings ?? []) facts.push({ term: "Warning", values: [w.explanation] });
  facts.push({ term: "Who is notified", values: [notified(action)] });
  return facts;
}

// One line of the Activity list for a change.
export function changeText(change: Change, names: Names): string {
  switch (change.kind) {
    case "account": return `Account ${change.account.email ?? change.account.account_id}: ${change.account.state.replaceAll("_", " ")}`;
    case "agent": return `Agent ${change.agent.registered_name}${change.agent.paused ? " (paused)" : ""}`;
    case "profiles": return "Your permission profiles changed";
    case "recipient_set": return `Group ${change.recipient_set.definition.name} changed`;
    case "recipient_set_deleted": return "A group was deleted";
    case "preferences": return "Your calendar preferences changed";
    case "request": {
      const r = change.request;
      return `${OPERATION[r.tool]}, from ${agentName(names, r.agent_id)}: ${STATE_TEXT[r.status.state]}`;
    }
    case "grant": return change.grant.ended === null
      ? `Permission given to ${agentName(names, change.grant.agent_id)}`
      : `Permission of ${agentName(names, change.grant.agent_id)} ended`;
    case "health": return `Broker health: rule checker ${change.health.prover}, storage ${change.health.store}`;
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
