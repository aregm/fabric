// The Review screen (plan S03, S05): requests, the permissions the owner gave, and the agents. Every
// node is made once and updated in place, so focus and a screen reader's place survive updates; a
// request's article never moves. Broker and agent text goes in with textContent only. What is
// disabled and which words are shown is decided in review.ts and model.ts; this file only shows it.
import type { Agent, DurationKind, Grant } from "../../../../contracts/desktop/v1/typescript/src/contract.ts";
import { button, byId, keyed, make, renderFacts, setDisabled, setText, show, textLine, uid } from "./dom.ts";
import type { Entry, Model } from "./model.ts";
import { ReviewModel, requestTitle } from "./review.ts";
import type { DetailsParts } from "./review.ts";
import {
  DURATION_NOTE, DURATION_ONLY, DURATION_TEXT, ENDED_REQUESTS_NOTE, PAUSE_NOTE, REVOKE_NOTE, SESSION_UNAVAILABLE, STATE_TEXT,
  cardFacts, grantTitle,
} from "./text.ts";
import type { Fact } from "./text.ts";

const CHECKING_TARGET_MS = 500;

function part(section: HTMLElement, title: string): { readonly part: HTMLDivElement; readonly heading: HTMLHeadingElement } {
  const node = make("div", "part");
  const heading = make("h3", "", title);
  heading.id = uid("part");
  heading.tabIndex = -1;
  node.append(heading);
  section.append(node);
  return { part: node, heading };
}

// A button whose name is a short visible text plus words that say what it acts on.
function namedButton(text: string, onPress: () => void, className = ""): { readonly node: HTMLButtonElement; readonly visible: HTMLSpanElement; readonly hidden: HTMLSpanElement } {
  const node = button("", onPress, className);
  const visible = make("span", "", text);
  const hidden = make("span", "visually-hidden");
  node.append(visible, hidden);
  return { node, visible, hidden };
}

// A list of facts that is written again only when its content changed.
interface FactsDom {
  readonly list: HTMLDListElement;
  key: string;
}

function facts(): FactsDom {
  return { list: make("dl", "facts"), key: "" };
}

function setFacts(dom: FactsDom, list: readonly Fact[]): void {
  const key = JSON.stringify(list);
  if (key === dom.key) return;
  dom.key = key;
  renderFacts(dom.list, list);
}

// A titled block of facts in the details, hidden while it has none.
interface BlockDom {
  readonly node: HTMLDivElement;
  readonly facts: FactsDom;
}

function block(title: string): BlockDom {
  const node = make("div");
  const f = facts();
  node.append(make("h5", "", title), f.list);
  return { node, facts: f };
}

// "Proof <id>" with "Show the proof", and the proof shown in place under it.
interface ProofDom {
  readonly node: HTMLDivElement;
  readonly line: HTMLParagraphElement;
  readonly button: HTMLButtonElement;
  readonly hidden: HTMLSpanElement;
  readonly region: HTMLDivElement;
  readonly status: HTMLParagraphElement;
  readonly facts: FactsDom;
  proofId: string;
}

interface DetailsDom {
  readonly button: HTMLButtonElement;
  readonly hidden: HTMLSpanElement;
  readonly region: HTMLDivElement;
  readonly status: HTMLParagraphElement;
  readonly content: HTMLDivElement;
  readonly general: FactsDom;
  readonly asked: BlockDom;
  readonly decision: BlockDom;
  readonly proofs: HTMLDivElement;
  readonly proofsEmpty: HTMLParagraphElement;
  readonly proofList: HTMLDivElement;
  readonly authorization: BlockDom;
  readonly attempts: HTMLDivElement;
  readonly attemptList: HTMLDivElement;
  readonly event: BlockDom;
}

interface RequestDom {
  readonly article: HTMLElement;
  readonly heading: HTMLHeadingElement;
  readonly state: HTMLParagraphElement;
  readonly ruling: HTMLParagraphElement;
  readonly rulingProof: ProofDom;
  readonly reason: HTMLParagraphElement;
  readonly help: HTMLParagraphElement;
  readonly delay: HTMLParagraphElement;
  readonly note: HTMLParagraphElement;
  readonly card: HTMLDivElement;
  readonly facts: FactsDom;
  readonly fieldset: HTMLFieldSetElement;
  readonly durations: HTMLDivElement; // a label per offered duration, stacked
  readonly radios: Map<DurationKind, HTMLInputElement>; // the radios made so far, in the page or not
  readonly radioName: string;
  readonly only: HTMLParagraphElement; // the single line of a card that offers one duration
  readonly sessionReason: HTMLParagraphElement;
  readonly covers: HTMLParagraphElement; // what a lasting choice covers
  readonly why: FactsDom;
  readonly whyProof: ProofDom;
  readonly changed: HTMLDivElement;
  readonly changedText: HTMLParagraphElement;
  readonly controls: HTMLDivElement;
  readonly deny: HTMLButtonElement;
  readonly allow: HTMLButtonElement;
  readonly status: HTMLParagraphElement;
  readonly outcome: HTMLParagraphElement;
  readonly details: DetailsDom;
}

interface ItemDom {
  readonly heading: HTMLHeadingElement;
  readonly state: HTMLParagraphElement;
  readonly facts: FactsDom;
  readonly actions: HTMLDivElement;
  readonly button: { readonly node: HTMLButtonElement; readonly visible: HTMLSpanElement; readonly hidden: HTMLSpanElement };
  readonly reason: HTMLParagraphElement;
  readonly note: HTMLParagraphElement;
}

export class ReviewView {
  private readonly cards = new Map<string, RequestDom>();
  private readonly proofNodes = new WeakMap<HTMLElement, ProofDom>();
  private readonly attemptNodes = new WeakMap<HTMLElement, { readonly facts: FactsDom; readonly title: HTMLHeadingElement; readonly proof: ProofDom }>();
  private readonly itemNodes = new WeakMap<HTMLElement, ItemDom>();
  private count = 0;
  private readonly model: Model;
  private readonly review: ReviewModel;
  private readonly heading: HTMLHeadingElement;
  private readonly counts: HTMLParagraphElement;
  private readonly empty: HTMLParagraphElement;
  private readonly list: HTMLDivElement;
  private readonly grantsHeading: HTMLHeadingElement;
  private readonly grantsEmpty: HTMLParagraphElement;
  private readonly grantList: HTMLDivElement;
  private readonly agentsHeading: HTMLHeadingElement;
  private readonly agentsEmpty: HTMLParagraphElement;
  private readonly agentList: HTMLDivElement;
  private readonly pauseAll: HTMLButtonElement;
  private readonly resumeAll: HTMLButtonElement;
  private readonly allReason: HTMLParagraphElement;
  private readonly allNote: HTMLParagraphElement;

  constructor(model: Model) {
    this.model = model;
    this.review = new ReviewModel(model);
    const section = byId("review");

    const requests = part(section, "Requests");
    this.heading = requests.heading;
    this.counts = make("p");
    this.empty = make("p", "", "No agent has asked for anything yet.");
    this.list = make("div", "list");
    requests.part.append(this.counts, make("p", "note", ENDED_REQUESTS_NOTE), this.empty, this.list);

    const grants = part(section, "Permissions you gave");
    this.grantsHeading = grants.heading;
    this.grantsEmpty = make("p", "", "No permission is in force.");
    this.grantList = make("div", "list");
    grants.part.append(make("p", "note", REVOKE_NOTE), this.grantsEmpty, this.grantList);

    const agents = part(section, "Agents");
    this.agentsHeading = agents.heading;
    this.allReason = make("p", "note");
    this.allReason.id = uid("all-agents-reason");
    this.allNote = make("p", "problem");
    this.pauseAll = button("Pause all agents", () => this.review.pauseAll(true));
    this.resumeAll = button("Resume all agents", () => this.review.pauseAll(false));
    const all = make("div", "actions");
    for (const b of [this.pauseAll, this.resumeAll]) b.setAttribute("aria-describedby", this.allReason.id);
    all.append(this.pauseAll, this.resumeAll, this.allReason);
    this.agentsEmpty = make("p", "", "No agent is registered. Connecting an agent is not in this build.");
    this.agentList = make("div", "list");
    agents.part.append(make("p", "note", PAUSE_NOTE), all, this.allNote, this.agentsEmpty, this.agentList);
  }

  render(): void {
    for (const e of this.model.requests.values()) this.renderRequest(e);
    // A request the model no longer keeps leaves the page.
    for (const [id, dom] of this.cards) {
      if (this.model.requests.has(id)) continue;
      if (dom.article.contains(document.activeElement)) this.heading.focus();
      dom.article.remove();
      this.cards.delete(id);
    }
    setText(this.counts, this.review.countsText());
    show(this.empty, this.model.requests.size === 0, this.heading);
    this.renderGrants();
    this.renderAgents();
  }

  // Moves focus to the heading of the first request that waits for a decision.
  focusFirstWaiting(): boolean {
    for (const e of this.model.requests.values()) {
      if (e.request.status.state !== "pending_approval") continue;
      const dom = this.cards.get(e.request.request_id);
      if (dom === undefined) continue;
      dom.heading.focus();
      return true;
    }
    return false;
  }

  // ---- Proofs ----

  private createProof(place: string): ProofDom {
    const node = make("div");
    const line = make("p");
    const region = make("div");
    region.id = uid("proof");
    region.hidden = true;
    const b = namedButton("Show the proof", () => this.review.toggleProof(place, dom.proofId));
    const dom: ProofDom = { node, line, button: b.node, hidden: b.hidden, region, status: make("p", "note"), facts: facts(), proofId: "" };
    dom.button.setAttribute("aria-expanded", "false");
    dom.button.setAttribute("aria-controls", region.id);
    const actions = make("div", "actions");
    actions.append(dom.button);
    region.append(dom.status, dom.facts.list);
    node.append(line, actions, region);
    this.proofNodes.set(node, dom);
    return dom;
  }

  private updateProof(dom: ProofDom, place: string, proofId: string | null, fallback: HTMLElement): void {
    show(dom.node, proofId !== null, fallback);
    if (proofId === null) return;
    dom.proofId = proofId;
    setText(dom.line, `Proof ${proofId}`);
    setText(dom.hidden, ` ${proofId}`);
    const shown = this.review.proofShown(place, proofId);
    if (dom.button.getAttribute("aria-expanded") !== String(shown)) dom.button.setAttribute("aria-expanded", String(shown));
    show(dom.region, shown, dom.button);
    if (!shown) return;
    textLine(dom.status, this.review.proofStatus(proofId), dom.button);
    setFacts(dom.facts, this.review.proofFacts(proofId));
  }

  // ---- Requests ----

  private renderRequest(e: Entry): void {
    const m = this.model;
    const id = e.request.request_id;
    const dom = this.cards.get(id) ?? this.createRequest(id);
    const r = e.request;
    const title = requestTitle(m, e);
    setText(dom.heading, title);
    dom.article.classList.toggle("item-waiting", e.available && r.status.state === "pending_approval");
    setText(dom.state, e.available ? `State: ${STATE_TEXT[r.status.state]}.` : "Not available: the broker no longer lists this request.");
    const lines = this.review.lines(e);
    textLine(dom.ruling, lines.ruling, dom.heading);
    this.updateProof(dom.rulingProof, `${id}:ruling`, lines.proofId, dom.heading);
    textLine(dom.reason, lines.reason, dom.heading);
    textLine(dom.help, lines.help, dom.heading);
    textLine(dom.delay, e.checkingDelayMs === null ? null
      : `“Received, checking” was on this page ${e.checkingDelayMs} ms after the broker received the request `
        + `(target: at most ${CHECKING_TARGET_MS} ms${e.checkingDelayMs > CHECKING_TARGET_MS ? "; this was slower" : ""}).`, dom.heading);
    textLine(dom.note, e.note, dom.heading);
    this.renderCard(e, dom);
    this.renderDetails(e, dom, title);
  }

  private renderCard(e: Entry, dom: RequestDom): void {
    const m = this.model;
    const status = m.cardStatus(e);
    show(dom.card, status.kind !== "none", dom.heading);
    if (status.kind === "none" || e.details === null || e.details.card === null) return;
    const card = e.details.card;
    setFacts(dom.facts, cardFacts(e.details, m.names));
    setFacts(dom.why, this.review.whyFacts(e));
    this.updateProof(dom.whyProof, `${e.request.request_id}:why`, card.insufficiency.proof_id, dom.heading);
    const open = status.kind !== "closed";
    show(dom.fieldset, open, dom.heading);
    // Only the offered durations are radios. A card that offers one says so in a line and offers no choice.
    const offered = m.offeredDurations(e);
    const single = offered.length === 1;
    keyed(dom.durations, single ? [] : offered, (kind) => kind, (kind) => this.durationChoice(e.request.request_id, dom, kind),
      () => undefined, dom.heading);
    textLine(dom.only, single ? DURATION_ONLY : null, dom.heading);
    const chosen = m.duration(e);
    for (const [kind, radio] of dom.radios) if (radio.checked !== (kind === chosen)) radio.checked = kind === chosen;
    // The session radio cannot be chosen while the agent has no live session. `disabled` is set only
    // while it does not hold focus; while it does, the model ignores the choice and the render puts
    // the selection back.
    const session = dom.radios.get("session");
    const sessionOk = m.sessionAvailable(e);
    if (session !== undefined) {
      const disabled = !sessionOk && document.activeElement !== session;
      if (session.disabled !== disabled) session.disabled = disabled;
    }
    textLine(dom.sessionReason, offered.includes("session") && !sessionOk ? SESSION_UNAVAILABLE : null, dom.heading);
    textLine(dom.covers, open ? m.lastingCovers(e) : null, dom.heading);
    show(dom.changed, status.kind === "changed", dom.heading);
    if (status.kind === "changed") setText(dom.changedText, status.text);
    show(dom.controls, open, dom.heading);
    setDisabled(dom.deny, status.kind !== "ready");
    setDisabled(dom.allow, status.kind !== "ready");
    textLine(dom.status, status.kind === "waiting" ? status.text : status.kind === "changed" ? "Decide after you have read the new version." : null, dom.heading);
    textLine(dom.outcome, status.kind === "closed" ? status.text : null, dom.heading);
  }

  private renderDetails(e: Entry, dom: RequestDom, title: string): void {
    const d = dom.details;
    const id = e.request.request_id;
    setText(d.hidden, ` of ${title}`);
    if (d.button.getAttribute("aria-expanded") !== String(e.detailsOpen)) d.button.setAttribute("aria-expanded", String(e.detailsOpen));
    show(d.region, e.detailsOpen, d.button);
    if (!e.detailsOpen) return;
    textLine(d.status, this.review.detailsStatus(e), d.button);
    const parts: DetailsParts | null = this.review.details(e);
    show(d.content, parts !== null, d.button);
    if (parts === null) return;
    setFacts(d.general, parts.general);
    const blockOf = (b: BlockDom, list: readonly Fact[]): void => {
      show(b.node, list.length > 0, d.button);
      setFacts(b.facts, list);
    };
    blockOf(d.asked, parts.asked);
    blockOf(d.decision, parts.decision);
    show(d.proofsEmpty, parts.proofIds.length === 0, d.button);
    const place = `${id}:proofs`;
    keyed(d.proofList, parts.proofIds, (p) => p, () => this.createProof(place).node, (node, p) => {
      const proof = this.proofNodes.get(node);
      if (proof !== undefined) this.updateProof(proof, place, p, d.button);
    }, d.button);
    blockOf(d.authorization, parts.authorization);
    show(d.attempts, parts.attempts.length > 0, d.button);
    keyed(d.attemptList, parts.attempts, (a) => String(a.number), (a) => this.createAttempt(`${id}:attempt:${a.number}`), (node, a) => {
      const attempt = this.attemptNodes.get(node);
      if (attempt === undefined) return;
      setText(attempt.title, `Attempt ${a.number}`);
      setFacts(attempt.facts, a.facts);
      this.updateProof(attempt.proof, `${id}:attempt:${a.number}`, a.proofId, d.button);
    }, d.button);
    blockOf(d.event, parts.event);
  }

  private createAttempt(place: string): HTMLDivElement {
    const node = make("div");
    const title = make("h6");
    const f = facts();
    const proof = this.createProof(place);
    node.append(title, f.list, proof.node);
    this.attemptNodes.set(node, { facts: f, title, proof });
    return node;
  }

  private createDetails(id: string): DetailsDom {
    const b = namedButton("Details", () => this.review.toggleDetails(id));
    const region = make("div");
    region.id = uid("details");
    region.hidden = true;
    b.node.setAttribute("aria-expanded", "false");
    b.node.setAttribute("aria-controls", region.id);
    const status = make("p", "note");
    const content = make("div");
    const general = facts();
    const asked = block("What was asked");
    const decision = block("Decision");
    const proofs = make("div");
    const proofsEmpty = make("p", "", "No proof is recorded for this request.");
    const proofList = make("div");
    proofs.append(make("h5", "", "Proofs, newest first"), proofsEmpty, proofList);
    const authorization = block("Authorization");
    const attempts = make("div");
    const attemptList = make("div");
    attempts.append(make("h5", "", "Attempts"), attemptList);
    const event = block("In the calendar");
    content.append(general.list, asked.node, decision.node, proofs, authorization.node, attempts, event.node);
    region.append(status, content);
    return {
      button: b.node, hidden: b.hidden, region, status, content, general, asked, decision, proofs, proofsEmpty, proofList,
      authorization, attempts, attemptList, event,
    };
  }

  private createRequest(id: string): RequestDom {
    const n = ++this.count;
    const article = make("article", "item");
    const heading = make("h4");
    heading.id = `request-${n}-heading`;
    heading.tabIndex = -1;
    article.setAttribute("aria-labelledby", heading.id);
    const state = make("p", "state");
    const ruling = make("p");
    const rulingProof = this.createProof(`${id}:ruling`);
    const reason = make("p");
    const help = make("p");
    const delay = make("p", "detail");
    const note = make("p", "problem");

    const card = make("div", "card");
    const cardFactsDom = facts();
    const fieldset = make("fieldset");
    fieldset.append(make("legend", "", "How long this approval lasts"));
    const durations = make("div");
    const only = make("p");
    const sessionReason = make("p", "note");
    sessionReason.id = uid("session-reason");
    // Part of the choice, not small print.
    const covers = make("p");
    covers.id = `request-${n}-covers`;
    fieldset.append(durations, only, sessionReason, covers, make("p", "note", DURATION_NOTE));
    const why = facts();
    const whyProof = this.createProof(`${id}:why`);
    const changed = make("div", "changed");
    const changedText = make("p");
    const ack = button("I have read the new version", () => this.model.acknowledgeChange(id));
    changed.append(changedText, ack);
    const controls = make("div", "actions");
    const status = make("p", "note");
    status.id = `request-${n}-status`;
    const deny = button("Deny", () => this.model.decide(id, "deny"));
    const allow = button("Allow", () => this.model.decide(id, "approve"), "primary");
    deny.setAttribute("aria-describedby", status.id);
    allow.setAttribute("aria-describedby", `${status.id} ${covers.id}`);
    controls.append(deny, allow, status);
    const outcome = make("p", "outcome");
    card.append(cardFactsDom.list, fieldset, why.list, whyProof.node, changed, controls, outcome);

    const details = this.createDetails(id);
    const detailsActions = make("div", "actions");
    detailsActions.append(details.button);
    article.append(heading, state, ruling, rulingProof.node, reason, help, delay, note, card, detailsActions, details.region);
    // Escape never decides: the request stays pending and nothing is sent.
    article.addEventListener("keydown", (event) => {
      if (event.key === "Escape") this.model.dismiss(id);
    });
    this.list.append(article);
    const dom: RequestDom = {
      article, heading, state, ruling, rulingProof, reason, help, delay, note, card, facts: cardFactsDom, fieldset, durations,
      radios: new Map(), radioName: `request-${n}-duration`, only, sessionReason, covers, why, whyProof, changed, changedText, controls,
      deny, allow, status, outcome, details,
    };
    this.cards.set(id, dom);
    return dom;
  }

  // The label of one duration's radio, made once per kind and kept in `dom.radios` while it is out
  // of the page.
  private durationChoice(id: string, dom: RequestDom, kind: DurationKind): HTMLLabelElement {
    const held = dom.radios.get(kind)?.parentElement;
    if (held instanceof HTMLLabelElement) return held;
    const label = make("label", "choice");
    const radio = make("input");
    radio.type = "radio";
    radio.name = dom.radioName;
    radio.value = kind;
    radio.addEventListener("change", () => this.model.chooseDuration(id, kind));
    if (kind === "session") radio.setAttribute("aria-describedby", dom.sessionReason.id);
    label.append(radio, ` ${DURATION_TEXT[kind]}`);
    dom.radios.set(kind, radio);
    return label;
  }

  // ---- Permissions and agents ----

  private createItem(onPress: (node: HTMLElement) => void, text: string): HTMLElement {
    const node = make("article", "item");
    const heading = make("h4");
    heading.id = uid("item");
    heading.tabIndex = -1;
    node.setAttribute("aria-labelledby", heading.id);
    const state = make("p", "state");
    const f = facts();
    const reason = make("p", "note");
    reason.id = uid("item-reason");
    const b = namedButton(text, () => onPress(node));
    b.node.setAttribute("aria-describedby", reason.id);
    const actions = make("div", "actions");
    actions.append(b.node, reason);
    const note = make("p", "problem");
    node.append(heading, state, f.list, actions, note);
    this.itemNodes.set(node, { heading, state, facts: f, actions, button: b, reason, note });
    return node;
  }

  private renderGrants(): void {
    const grants = this.review.grants();
    const focused = [...this.grantList.children].find((c) => c.contains(document.activeElement)) as HTMLElement | undefined;
    keyed(this.grantList, grants, (g) => g.grant_id, () => this.createItem((node) => {
      const grantId = node.dataset["grant"];
      if (grantId !== undefined) this.review.revoke(grantId);
    }, "Take back the permission"), (node, g) => this.updateGrant(node, g), this.grantsHeading);
    // A grant that ended moves below the live ones; focus stays in it or goes to its heading.
    if (focused !== undefined && focused.isConnected && !focused.contains(document.activeElement)) {
      this.itemNodes.get(focused)?.heading.focus();
    }
    show(this.grantsEmpty, !grants.some((g) => g.ended === null), this.grantsHeading);
  }

  private updateGrant(node: HTMLElement, g: Grant): void {
    const dom = this.itemNodes.get(node);
    if (dom === undefined) return;
    node.dataset["grant"] = g.grant_id;
    const title = grantTitle(g, this.model.names);
    setText(dom.heading, title);
    setText(dom.state, g.ended === null ? "In force." : "Ended while this window was open.");
    setFacts(dom.facts, this.review.grantFacts(g));
    show(dom.actions, g.ended === null, dom.heading);
    setText(dom.button.hidden, ` of ${title}`);
    const blocked = this.review.revokeBlocked(g.grant_id);
    setDisabled(dom.button.node, blocked !== null);
    setText(dom.reason, blocked ?? "");
    textLine(dom.note, this.review.revokeState(g.grant_id)?.note ?? null, dom.heading);
  }

  private renderAgents(): void {
    const agents = [...this.model.agents.values()];
    keyed(this.agentList, agents, (a) => a.agent_id, () => this.createItem((node) => {
      const agent = this.model.agents.get(node.dataset["agent"] ?? "");
      if (agent !== undefined) this.review.pause(agent.agent_id, !agent.paused);
    }, "Pause"), (node, a) => this.updateAgent(node, a), this.agentsHeading);
    show(this.agentsEmpty, agents.length === 0, this.agentsHeading);
    const blocked = this.review.pauseBlocked(null);
    setDisabled(this.pauseAll, blocked !== null);
    setDisabled(this.resumeAll, blocked !== null);
    setText(this.allReason, blocked ?? "");
    textLine(this.allNote, this.review.agentState(null)?.note ?? null, this.agentsHeading);
  }

  private updateAgent(node: HTMLElement, a: Agent): void {
    const dom = this.itemNodes.get(node);
    if (dom === undefined) return;
    node.dataset["agent"] = a.agent_id;
    setText(dom.heading, a.registered_name);
    setText(dom.state, a.paused ? "Paused." : a.session === null ? "Not connected." : "Connected.");
    setFacts(dom.facts, this.review.agentFacts(a));
    setText(dom.button.visible, a.paused ? "Resume" : "Pause");
    setText(dom.button.hidden, ` ${a.registered_name}`);
    const blocked = this.review.pauseBlocked(a.agent_id);
    setDisabled(dom.button.node, blocked !== null);
    setText(dom.reason, blocked ?? "");
    textLine(dom.note, this.review.agentState(a.agent_id)?.note ?? null, dom.heading);
  }
}
