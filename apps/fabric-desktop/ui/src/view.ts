// Puts the model on the page. Each request's article is made once and updated in place, so focus
// and a screen reader's place survive updates. Broker and agent text goes in with textContent only.
import type { DurationKind } from "../../../../contracts/desktop/v1/typescript/src/contract.ts";
import type { Entry, Model } from "./model.ts";
import { ACTIVITY_LIMIT } from "./model.ts";
import { DURATION_TEXT, OPERATION, STATE_TEXT, agentName, bannerText, cardFacts, formatInstant, formatTimeOfDay } from "./text.ts";
import type { Fact } from "./text.ts";

const DURATIONS: readonly DurationKind[] = ["one_request", "session", "until_revoked"];
const CHECKING_TARGET_MS = 500;

function byId<T extends HTMLElement = HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (node === null) throw new Error(`index.html has no #${id}`);
  return node as T;
}

function make<K extends keyof HTMLElementTagNameMap>(tag: K, className = "", text = ""): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className !== "") node.className = className;
  if (text !== "") node.textContent = text;
  return node;
}

// Changes the page only when the text differs, so a screen reader is not made to read it again.
function setText(node: HTMLElement, text: string): void {
  if (node.textContent !== text) node.textContent = text;
}

// Hides or shows a node. A node that is hidden while it holds focus hands focus to `fallback`.
function show(node: HTMLElement, visible: boolean, fallback: HTMLElement): void {
  if (!visible && !node.hidden && node.contains(document.activeElement)) fallback.focus();
  node.hidden = !visible;
}

function textLine(node: HTMLElement, text: string | null, fallback: HTMLElement): void {
  setText(node, text ?? "");
  show(node, text !== null && text !== "", fallback);
}

// aria-disabled, not disabled: the button keeps focus and stays in the tab order, and the text
// beside it (aria-describedby) says why it does nothing.
function setDisabled(button: HTMLButtonElement, disabled: boolean): void {
  const value = String(disabled);
  if (button.getAttribute("aria-disabled") !== value) button.setAttribute("aria-disabled", value);
}

function renderFacts(list: HTMLDListElement, facts: readonly Fact[]): void {
  list.replaceChildren();
  for (const f of facts) {
    list.append(make("dt", "", f.term));
    for (const v of f.values) {
      const dd = make("dd", f.untrusted === true ? "untrusted" : "");
      if (f.untrusted === true) dd.append(make("blockquote", "", v));
      else dd.textContent = v;
      list.append(dd);
    }
  }
}

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

export class View {
  private readonly cards = new Map<string, RequestDom>();
  private readonly logged = new WeakSet<object>();
  private readonly announcer = byId("announcer");
  private count = 0;
  private readonly model: Model;

  constructor(model: Model) {
    this.model = model;
    byId("check").addEventListener("click", () => model.checkConnection());
    byId("restart").addEventListener("click", () => model.restart());
  }

  announce(text: string): void {
    // A trailing no-break space alternates so that the same sentence twice is still announced.
    const same = this.announcer.textContent?.replace(/ $/, "") === text;
    this.announcer.textContent = same && !this.announcer.textContent?.endsWith(" ") ? `${text} ` : text;
  }

  render(): void {
    this.renderBroker();
    for (const e of this.model.requests.values()) this.renderRequest(e);
    show(byId("requests-empty"), this.model.requests.size === 0, byId("requests-heading"));
    this.renderActivity();
  }

  private renderBroker(): void {
    const m = this.model;
    const heading = byId("broker-heading");
    const state = m.connection;
    const banner = bannerText(state);
    show(byId("banner"), banner !== null, heading);
    if (banner !== null) {
      setText(byId("banner-title"), banner.title);
      setText(byId("banner-text"), banner.text);
      setText(byId("banner-detail"), state !== null && "detail" in state ? `Details: ${state.detail}` : "");
    }
    show(byId("restart-row"), state?.name === "disconnected", heading);
    setDisabled(byId("restart"), m.restarting);
    setText(byId("restart-note"), m.restarting ? "Starting the broker…" : "");

    const connected = state?.name === "connected";
    setText(byId("connection"), state === null ? "Connecting to the broker…"
      : !connected ? (state.name === "disconnected" ? "Stopped" : "Not started")
      : m.phase === "live" ? "Running and up to date"
      : m.phase === "failed" ? "Running, but the current state could not be read"
      : "Running; reading the current state…");
    setText(byId("broker-name"), m.broker === null ? "Not known yet" : `${m.broker.broker.name}, build ${m.broker.broker.build}`);
    setText(byId("schema-version"), m.broker === null ? "Not known yet" : String(m.broker.schema_version));
    setText(byId("last-check"), m.lastCheck === null ? "None yet"
      : `Answered in ${m.lastCheck.ms} ms, at ${formatTimeOfDay(m.lastCheck.at, m.names.zone)}`);
    setDisabled(byId("check"), !connected || m.checking);
    setText(byId("check-note"), !connected ? "Not available while the broker is not running." : m.checking ? "Checking…" : "");
    textLine(byId("problem"), m.problem, heading);
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
    const article = make("article", "request");
    const heading = make("h3");
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
    byId("requests").append(article);
    const dom: RequestDom = {
      article, heading, state, ruling, delay, note, card, facts, factsKey: "", fieldset, radios, why, whyKey: "",
      changed, changedText, controls, deny, allow, status, outcome,
    };
    this.cards.set(id, dom);
    return dom;
  }

  private renderActivity(): void {
    const list = byId("activity");
    for (const item of this.model.activity) {
      if (this.logged.has(item)) continue;
      this.logged.add(item);
      list.prepend(make("li", "", `Notice ${item.sequence}: ${item.text}`));
    }
    while (list.children.length > ACTIVITY_LIMIT) list.lastElementChild?.remove();
    show(byId("activity-empty"), this.model.activity.length === 0, byId("activity-heading"));
  }
}
