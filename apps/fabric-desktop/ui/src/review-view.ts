// The Review screen's requests. Each request's article is made once and updated in place, so focus
// and a screen reader's place survive updates. Broker and agent text goes in with textContent only.
import type { DurationKind } from "../../../../contracts/desktop/v1/typescript/src/contract.ts";
import { byId, make, renderFacts, setDisabled, setText, show, textLine } from "./dom.ts";
import type { Entry, Model } from "./model.ts";
import { DURATION_TEXT, OPERATION, STATE_TEXT, agentName, cardFacts, formatInstant } from "./text.ts";
import type { Fact } from "./text.ts";

const DURATIONS: readonly DurationKind[] = ["one_request", "session", "until_revoked"];
const CHECKING_TARGET_MS = 500;

interface RequestDom {
  readonly article: HTMLElement;
  readonly heading: HTMLHeadingElement;
  readonly state: HTMLParagraphElement;
  readonly ruling: HTMLParagraphElement;
  readonly delay: HTMLParagraphElement;
  readonly note: HTMLParagraphElement;
  readonly card: HTMLDivElement;
  readonly facts: HTMLDListElement;
  factsKey: string;
  readonly fieldset: HTMLFieldSetElement;
  readonly radios: ReadonlyMap<DurationKind, HTMLInputElement>;
  readonly why: HTMLDListElement;
  whyKey: string;
  readonly changed: HTMLDivElement;
  readonly changedText: HTMLParagraphElement;
  readonly controls: HTMLDivElement;
  readonly deny: HTMLButtonElement;
  readonly allow: HTMLButtonElement;
  readonly status: HTMLParagraphElement;
  readonly outcome: HTMLParagraphElement;
}

export class ReviewView {
  private readonly cards = new Map<string, RequestDom>();
  private count = 0;
  private readonly model: Model;
  private readonly heading: HTMLHeadingElement;
  private readonly empty: HTMLParagraphElement;
  private readonly list: HTMLDivElement;

  constructor(model: Model) {
    this.model = model;
    const section = byId("review");
    this.heading = make("h3", "", "Requests");
    this.heading.id = "requests-heading";
    this.heading.tabIndex = -1;
    this.empty = make("p", "", "No agent has asked for anything yet.");
    this.list = make("div", "list");
    this.list.id = "requests";
    const part = make("div", "part");
    part.append(this.heading, this.empty, this.list);
    section.append(part);
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
    show(this.empty, this.model.requests.size === 0, this.heading);
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

  private renderRequest(e: Entry): void {
    const m = this.model;
    const id = e.request.request_id;
    const dom = this.cards.get(id) ?? this.createRequest(id);
    const names = m.names;
    const r = e.request;
    setText(dom.heading, `${OPERATION[r.tool]}, from ${agentName(names, r.agent_id)}`);
    setText(dom.state, e.available ? `State: ${STATE_TEXT[r.status.state]}.` : "Not available: the broker no longer lists this request.");
    const s = r.status;
    textLine(dom.ruling, "denial" in s ? `Rule ${s.denial.rule}: ${s.denial.explanation}`
      : "failure" in s ? `Reason: ${s.failure.explanation}` : null, dom.heading);
    textLine(dom.delay, e.checkingDelayMs === null ? null
      : `“Received, checking” was on this page ${e.checkingDelayMs} ms after the broker received the request `
        + `(target: at most ${CHECKING_TARGET_MS} ms${e.checkingDelayMs > CHECKING_TARGET_MS ? "; this was slower" : ""}).`, dom.heading);
    textLine(dom.note, e.note, dom.heading);

    const status = m.cardStatus(e);
    show(dom.card, status.kind !== "none", dom.heading);
    if (status.kind === "none" || e.details === null || e.details.card === null) return;
    const card = e.details.card;
    const facts = cardFacts(e.details, names);
    const factsKey = JSON.stringify(facts);
    if (factsKey !== dom.factsKey) {
      renderFacts(dom.facts, facts);
      dom.factsKey = factsKey;
    }
    const why: Fact[] = [
      { term: "Why you are asked", values: [card.insufficiency.explanation, `Rule: ${card.insufficiency.rule}`] },
      { term: "This card expires", values: [`${formatInstant(card.expires_at, names.zone)}. If nobody decides by then, nothing is done.`] },
    ];
    const whyKey = JSON.stringify(why);
    if (whyKey !== dom.whyKey) {
      renderFacts(dom.why, why);
      dom.whyKey = whyKey;
    }
    const open = status.kind !== "closed";
    show(dom.fieldset, open, dom.heading);
    for (const [kind, radio] of dom.radios) if (radio.checked !== (kind === e.duration)) radio.checked = kind === e.duration;
    show(dom.changed, status.kind === "changed", dom.heading);
    if (status.kind === "changed") setText(dom.changedText, status.text);
    show(dom.controls, open, dom.heading);
    setDisabled(dom.deny, status.kind !== "ready");
    setDisabled(dom.allow, status.kind !== "ready");
    textLine(dom.status, status.kind === "waiting" ? status.text : status.kind === "changed" ? "Decide after you have read the new version." : null, dom.heading);
    textLine(dom.outcome, status.kind === "closed" ? status.text : null, dom.heading);
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
    const delay = make("p", "detail");
    const note = make("p", "problem");

    const card = make("div", "card");
    const facts = make("dl", "facts");
    const fieldset = make("fieldset");
    fieldset.append(make("legend", "", "How long this approval lasts"));
    const radios = new Map<DurationKind, HTMLInputElement>();
    for (const kind of DURATIONS) {
      const label = make("label", "choice");
      const radio = make("input");
      radio.type = "radio";
      radio.name = `request-${n}-duration`;
      radio.value = kind;
      radio.addEventListener("change", () => this.model.chooseDuration(id, kind));
      label.append(radio, ` ${DURATION_TEXT[kind]}`);
      fieldset.append(label);
      radios.set(kind, radio);
    }
    const why = make("dl", "facts");
    const changed = make("div", "changed");
    const changedText = make("p");
    const ack = make("button", "", "I have read the new version");
    ack.type = "button";
    ack.addEventListener("click", () => this.model.acknowledgeChange(id));
    changed.append(changedText, ack);
    const controls = make("div", "actions");
    const status = make("p", "note");
    status.id = `request-${n}-status`;
    const deny = make("button", "", "Deny");
    const allow = make("button", "primary", "Allow");
    for (const [button, decision] of [[deny, "deny"], [allow, "approve"]] as const) {
      button.type = "button";
      button.setAttribute("aria-describedby", status.id);
      button.addEventListener("click", () => this.model.decide(id, decision));
    }
    controls.append(deny, allow, status);
    const outcome = make("p", "outcome");
    card.append(facts, fieldset, why, changed, controls, outcome);
    article.append(heading, state, ruling, delay, note, card);
    // Escape never decides: the request stays pending and nothing is sent.
    article.addEventListener("keydown", (event) => {
      if (event.key === "Escape") this.model.dismiss(id);
    });
    this.list.append(article);
    const dom: RequestDom = {
      article, heading, state, ruling, delay, note, card, facts, factsKey: "", fieldset, radios, why, whyKey: "",
      changed, changedText, controls, deny, allow, status, outcome,
    };
    this.cards.set(id, dom);
    return dom;
  }
}
