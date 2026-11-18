// The Review screen's own state and words, with no DOM (plan S03, S05): the proofs read so far, the
// state of each revoke and pause command, the counts, and what each request, permission and agent
// shows. The page decides nothing about permissions: it shows what the broker reports and sends
// the owner's commands through `model.command`.
import type { Agent, Grant, ProofReport } from "../../../../contracts/desktop/v1/typescript/src/contract.ts";
import type { Entry, Failure, Model } from "./model.ts";
import {
  NO_PROOF, OPERATION, OWNER_CAN_DO, TERMINAL, UNDECIDED_REASON, UNDECIDED_REASON_UNKNOWN, agentFacts, agentName, approvalFacts,
  askedFacts, attemptFacts, authorizationFacts, formatInstant, grantFacts, proofFacts,
} from "./text.ts";
import type { Fact } from "./text.ts";

// A proof read with get_proof, or being read.
export interface ProofState {
  reading: boolean;
  report: ProofReport | null;
  note: string | null; // why it could not be read
}

// A revoke or pause command of the screen.
export interface CommandState {
  inFlight: boolean;
  note: string | null; // the last problem, in words
}

// What a request always shows besides its state (the ruling stays as long as the request is listed: DEF-7).
export interface RequestLines {
  readonly ruling: string | null; // "Rule <rule>: <explanation>"
  readonly proofId: string | null; // the proof of the ruling
  readonly reason: string | null; // why it failed, or why no proof decided it as the broker recorded it (once read)
  readonly help: string | null; // what the owner can do for an unknown outcome
}

// The details of a request, in the parts the view shows.
export interface DetailsParts {
  readonly general: readonly Fact[]; // received, last change, why it was not decided (undecided only), account, request digest
  readonly asked: readonly Fact[]; // empty while the approval card shows them
  readonly decision: readonly Fact[]; // empty when nobody decided
  readonly proofIds: readonly string[]; // in the broker's order, newest first
  readonly authorization: readonly Fact[]; // empty when there is none
  readonly attempts: readonly { readonly number: number; readonly facts: readonly Fact[]; readonly proofId: string }[];
  readonly event: readonly Fact[]; // empty when the calendar has no event for it
}

export interface Counts {
  readonly waiting: number;
  readonly inProgress: number;
  readonly ended: number;
  readonly unavailable: number;
}

const ALL = "all";
const agentKey = (id: string): string => `agent:${id}`;
const grantKey = (id: string): string => `grant:${id}`;

export function requestTitle(m: Model, e: Entry): string {
  return `${OPERATION[e.request.tool]}, from ${agentName(m.names, e.request.agent_id)}`;
}

export class ReviewModel {
  readonly proofs = new Map<string, ProofState>(); // per proof id
  private readonly shownProofs = new Set<string>(); // per place: "<where>\n<proof id>"
  private readonly commands = new Map<string, CommandState>();
  private readonly model: Model;

  constructor(model: Model) {
    this.model = model;
  }

  // ---- Requests ----

  get counts(): Counts {
    let waiting = 0;
    let inProgress = 0;
    let ended = 0;
    let unavailable = 0;
    for (const e of this.model.requests.values()) {
      const s = e.request.status.state;
      if (TERMINAL[s]) ended += 1;
      else if (!e.available) unavailable += 1;
      else if (s === "pending_approval") waiting += 1;
      else inProgress += 1;
    }
    return { waiting, inProgress, ended, unavailable };
  }

  countsText(): string {
    const c = this.counts;
    const parts = [
      `${c.waiting} ${c.waiting === 1 ? "waits" : "wait"} for your decision`,
      `${c.inProgress} in progress`,
      `${c.ended} ended while this window was open`,
    ];
    if (c.unavailable > 0) parts.push(`${c.unavailable} no longer listed by the broker`);
    return `${parts.join(", ")}.`;
  }

  lines(e: Entry): RequestLines {
    const s = e.request.status;
    return {
      ruling: "denial" in s ? `Rule ${s.denial.rule}: ${s.denial.explanation}` : null,
      proofId: "denial" in s ? s.denial.proof_id : null,
      reason: "failure" in s ? `Reason: ${s.failure.explanation}` : s.state === "undecided" ? this.undecidedReason(e) : null,
      help: OWNER_CAN_DO[s.state] ?? null,
    };
  }

  // Why no proof decided the request, as the broker recorded it; null until read, or when it recorded none.
  private undecidedReason(e: Entry): string | null {
    const reason = e.undecided?.reason ?? null;
    return reason === null ? null : `Reason: ${UNDECIDED_REASON[reason]}`;
  }

  // Why the card asks, and while it is open when it expires; a closed card says why the owner was asked.
  whyFacts(e: Entry): Fact[] {
    const card = e.details?.card ?? null;
    if (card === null) return [];
    const closed = this.model.cardStatus(e).kind === "closed";
    const why: Fact = { term: closed ? "Why you were asked" : "Why you are asked", values: [card.insufficiency.explanation, `Rule: ${card.insufficiency.rule}`] };
    if (closed) return [why];
    return [why, {
      term: "This card expires",
      values: [`${formatInstant(card.expires_at, this.model.zone)}. If nobody decides by then, nothing is done.`],
    }];
  }

  toggleDetails(requestId: string): void {
    const e = this.model.requests.get(requestId);
    if (e !== undefined) this.model.openDetails(requestId, !e.detailsOpen);
  }

  // What the open details region says above the details, or null.
  detailsStatus(e: Entry): string | null {
    if (e.reading) return "Reading the details…";
    if (e.details === null) return this.model.notReadyText ?? "The details have not been read.";
    if (e.details.request.revision < e.request.revision) {
      return `These details are of an earlier version of the request. ${this.model.notReadyText ?? ""}`.trim();
    }
    return null;
  }

  details(e: Entry): DetailsParts | null {
    const d = e.details;
    if (d === null) return null;
    const m = this.model;
    const names = m.names;
    const z = names.zone;
    const r = d.request;
    const why: Fact[] = r.status.state !== "undecided" ? []
      : [{ term: "Why it was not decided", values: [d.undecided_reason === null ? UNDECIDED_REASON_UNKNOWN : UNDECIDED_REASON[d.undecided_reason]] }];
    return {
      general: [
        { term: "Received", values: [formatInstant(r.received_at, z)] },
        { term: "Last change", values: [formatInstant(r.updated_at, z)] },
        ...why,
        { term: "Account", values: [r.account_id === null ? "None" : names.accounts.get(r.account_id)?.email ?? r.account_id] },
        { term: "Request digest", values: [r.request_digest] },
      ],
      asked: d.card === null ? askedFacts(d, names) : [], // the card shows them
      decision: d.approval === null ? [] : approvalFacts(d.approval, names, m.recipientSets),
      proofIds: d.proof_ids,
      authorization: d.authorization === null ? [] : authorizationFacts(d.authorization, z),
      attempts: d.attempts.map((a) => ({ number: a.number, facts: attemptFacts(a, z), proofId: a.proof_id })),
      event: d.provider_event === null ? [] : [
        { term: "Event in the calendar", values: [d.provider_event.event_id] },
        { term: "Its version in the calendar", values: [d.provider_event.revision] },
      ],
    };
  }

  // ---- Proofs, shown in place under what asked for them ----

  proofShown(place: string, proofId: string): boolean {
    return this.shownProofs.has(`${place}\n${proofId}`);
  }

  // Shows or hides a proof at one place. Showing one not read yet reads it (get_proof); a proof
  // read once is kept, since a proof report never changes.
  toggleProof(place: string, proofId: string): void {
    const key = `${place}\n${proofId}`;
    if (this.shownProofs.delete(key)) return this.model.changed();
    this.shownProofs.add(key);
    const p = this.proofs.get(proofId) ?? { reading: false, report: null, note: null };
    this.proofs.set(proofId, p);
    if (p.report === null && !p.reading) {
      if (this.model.ready) this.readProof(proofId, p);
      else p.note = this.model.notReadyText;
    }
    this.model.changed();
  }

  // The words above a shown proof's facts, or null when its report is shown.
  proofStatus(proofId: string): string | null {
    const p = this.proofs.get(proofId);
    if (p === undefined || p.reading) return "Reading the proof…";
    return p.note;
  }

  proofFacts(proofId: string): Fact[] {
    const report = this.proofs.get(proofId)?.report ?? null;
    return report === null ? [] : proofFacts(report, this.model.zone);
  }

  private readProof(proofId: string, p: ProofState): void {
    p.reading = true;
    p.note = null;
    this.model.query("get_proof", { proof_id: proofId }, (report) => {
      p.reading = false;
      p.report = report;
    }, (f) => {
      p.reading = false;
      p.note = f.code === "not_found" ? NO_PROOF
        : f.code === null ? `The proof was not read: ${f.text}. Show it again to read it again.`
        : `The broker did not give the proof: ${f.text}`;
    });
  }

  // ---- Permissions you gave ----

  // Live ones first, then those that ended while this window was open.
  grants(): Grant[] {
    const all = [...this.model.grants.values()];
    return [...all.filter((g) => g.ended === null), ...all.filter((g) => g.ended !== null)];
  }

  grantFacts(g: Grant): Fact[] {
    const p = this.model.profiles?.items.find((x) => x.profile_id === g.policy.profile_id);
    const profile = p === undefined ? `The profile ${g.policy.profile_id}` : `The profile ${p.name}`;
    const o = g.origin;
    let origin: string;
    if ("enrollment_id" in o) {
      origin = `enrollment ${o.enrollment_id}`;
    } else {
      const e = this.model.requests.get(o.request_id);
      origin = e === undefined ? o.request_id : `${requestTitle(this.model, e)} (request ${o.request_id})`;
    }
    return grantFacts(g, this.model.names, this.model.recipientSets, profile, origin);
  }

  revokeState(grantId: string): CommandState | null {
    return this.commands.get(grantKey(grantId)) ?? null;
  }

  // Why "Take it back" does nothing now, or null when it can be pressed.
  revokeBlocked(grantId: string): string | null {
    return this.revokeState(grantId)?.inFlight === true ? "Taking it back…" : this.model.notReadyText;
  }

  revoke(grantId: string): boolean {
    const m = this.model;
    const g = m.grants.get(grantId);
    if (g === undefined || g.ended !== null || this.revokeBlocked(grantId) !== null) return false;
    const agent = agentName(m.names, g.agent_id);
    this.run(grantKey(grantId), (ok, fail) => m.command("revoke_grant", { grant_id: grantId, expected_revision: g.revision }, ok, fail), {
      done: `You took back the permission of ${agent}.`,
      conflict: "This permission changed just now, so nothing was taken back. Look at it again, then try again.",
      unanswered: "It is not known whether the permission was taken back. Taking it back again is safe.",
      refused: "The broker did not take the permission back:",
      held: () => m.grants.get(grantId)?.revision ?? 0,
    });
    return true;
  }

  // ---- Agents ----

  agentFacts(a: Agent): Fact[] {
    return agentFacts(a, this.model.zone);
  }

  agentState(agentId: string | null): CommandState | null {
    return this.commands.get(agentId === null ? ALL : agentKey(agentId)) ?? null;
  }

  // Why the pause or resume button of one agent (or of all, for null) does nothing now, or null.
  pauseBlocked(agentId: string | null): string | null {
    const s = this.agentState(agentId);
    if (s?.inFlight === true) return "Sending…";
    if (this.model.notReadyText !== null) return this.model.notReadyText;
    if (agentId === null && this.model.agents.size === 0) return "No agent is registered.";
    return null;
  }

  // Pauses or resumes one agent, with its revision.
  pause(agentId: string, paused: boolean): boolean {
    const m = this.model;
    const a = m.agents.get(agentId);
    if (a === undefined || this.pauseBlocked(agentId) !== null) return false;
    const name = a.registered_name;
    const target = { kind: "agent", agent_id: agentId, expected_revision: a.revision } as const;
    this.run(agentKey(agentId), (ok, fail) => m.command("pause_agent", { target, paused }, ok, fail), {
      done: paused ? `${name} is paused.` : `${name} can work again.`,
      conflict: `This agent's record changed just now, so nothing was ${paused ? "paused" : "resumed"}. Try again.`,
      unanswered: `It is not known whether ${name} was ${paused ? "paused" : "resumed"}. Doing it again is safe.`,
      refused: `The broker did not ${paused ? "pause" : "resume"} ${name}:`,
      held: () => m.agents.get(agentId)?.revision ?? 0,
    });
    return true;
  }

  pauseAll(paused: boolean): boolean {
    if (this.pauseBlocked(null) !== null) return false;
    this.run(ALL, (ok, fail) => this.model.command("pause_agent", { target: { kind: "all_agents" }, paused }, ok, fail), {
      done: paused ? "All agents are paused." : "All agents can work again.",
      conflict: `Something changed just now, so nothing was ${paused ? "paused" : "resumed"}. Try again.`,
      unanswered: `It is not known whether the agents were ${paused ? "paused" : "resumed"}. Doing it again is safe.`,
      refused: `The broker did not ${paused ? "pause" : "resume"} the agents:`,
      held: () => Number.MAX_SAFE_INTEGER,
    });
    return true;
  }

  // One command of the screen: in flight until answered; the result is announced. A conflict changed
  // nothing; the page reads the current state unless it already holds the newer record.
  private run(
    key: string,
    send: (ok: () => void, fail: (f: Failure) => void) => string,
    words: {
      readonly done: string; readonly conflict: string; readonly unanswered: string; readonly refused: string;
      readonly held: () => number;
    },
  ): void {
    const state: CommandState = { inFlight: true, note: null };
    this.commands.set(key, state);
    send(() => {
      state.inFlight = false;
      this.model.announce(words.done);
    }, (f) => {
      state.inFlight = false;
      if (f.error?.code === "conflict") {
        state.note = words.conflict;
        if (words.held() < f.error.detail.current_revision) this.model.refresh();
      } else if (f.code === null) {
        state.note = `${words.unanswered} (${f.text})`;
      } else {
        state.note = `${words.refused} ${f.text}`;
      }
      this.model.announce(state.note);
    });
    this.model.changed();
  }
}
