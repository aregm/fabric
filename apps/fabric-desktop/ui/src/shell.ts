// The frame around the screens: the header's build line, the attention strip and the Status
// screen, and the polite live region. Everything is updated in place.
import type { Account } from "../../../../contracts/desktop/v1/typescript/src/contract.ts";
import { button, byId, keyed, make, setDisabled, setText, show, textLine, uid } from "./dom.ts";
import type { Model } from "./model.ts";
import { ACTIVITY_LIMIT } from "./model.ts";
import type { Nav } from "./nav.ts";
import type { ReviewView } from "./review-view.ts";
import { attention, brokerText, buildLine, calendarLine, connectionText, lastCheckText, proverCauseText, proverText, storeText } from "./status.ts";
import type { AttentionItem } from "./status.ts";
import { ACCOUNT_TEXT } from "./text.ts";

interface ItemDom {
  readonly node: HTMLLIElement;
  readonly title: HTMLParagraphElement;
  readonly text: HTMLParagraphElement;
  readonly detail: HTMLParagraphElement;
  readonly control: HTMLElement | null;
  readonly note: HTMLParagraphElement;
}

interface AccountDom {
  readonly node: HTMLElement;
  readonly heading: HTMLHeadingElement;
  readonly state: HTMLParagraphElement;
  readonly calendars: HTMLUListElement;
  calendarsKey: string;
}

function part(section: HTMLElement, title: string): { readonly part: HTMLDivElement; readonly heading: HTMLHeadingElement } {
  const node = make("div", "part");
  const heading = make("h3", "", title);
  heading.id = uid("part");
  heading.tabIndex = -1;
  node.append(heading);
  section.append(node);
  return { part: node, heading };
}

export class Shell {
  private readonly model: Model;
  private readonly nav: Nav;
  private readonly review: ReviewView;
  private readonly announcer = byId("announcer");
  private readonly strip = byId("attention");
  private readonly stripList = byId("attention-list");
  private readonly items = new WeakMap<HTMLElement, ItemDom>();
  private readonly accountNodes = new WeakMap<HTMLElement, AccountDom>();
  private readonly logged = new WeakSet<object>();
  // The Status screen.
  private readonly statusHeading = byId("status-heading");
  private readonly connection: HTMLElement;
  private readonly brokerName: HTMLElement;
  private readonly schemaVersion: HTMLElement;
  private readonly lastCheck: HTMLElement;
  private readonly problem: HTMLParagraphElement;
  private readonly check: HTMLButtonElement;
  private readonly checkNote: HTMLParagraphElement;
  private readonly store: HTMLParagraphElement;
  private readonly prover: HTMLParagraphElement;
  private readonly proverCause: HTMLParagraphElement;
  private readonly accountsHeading: HTMLHeadingElement;
  private readonly accountsEmpty: HTMLParagraphElement;
  private readonly accounts: HTMLDivElement;
  private readonly activityHeading: HTMLHeadingElement;
  private readonly activityEmpty: HTMLParagraphElement;
  private readonly activity: HTMLUListElement;

  constructor(model: Model, nav: Nav, review: ReviewView) {
    this.model = model;
    this.nav = nav;
    this.review = review;
    const section = byId("status");

    const broker = part(section, "Broker");
    const facts = make("dl", "facts");
    const fact = (term: string): HTMLElement => {
      const dd = make("dd");
      facts.append(make("dt", "", term), dd);
      return dd;
    };
    this.connection = fact("Connection");
    this.brokerName = fact("Broker");
    this.schemaVersion = fact("Schema version");
    this.lastCheck = fact("Last check");
    this.problem = make("p", "problem");
    this.problem.hidden = true;
    const actions = make("div", "actions");
    this.checkNote = make("p", "note");
    this.checkNote.id = uid("check-note");
    this.check = button("Check connection", () => model.checkConnection());
    this.check.setAttribute("aria-describedby", this.checkNote.id);
    actions.append(this.check, this.checkNote);
    broker.part.append(facts, this.problem, actions);

    this.store = make("p");
    part(section, "Store").part.append(this.store);
    this.prover = make("p");
    this.proverCause = make("p");
    this.proverCause.hidden = true;
    part(section, "Rule checker").part.append(this.prover, this.proverCause);

    const accounts = part(section, "Accounts");
    this.accountsHeading = accounts.heading;
    this.accountsEmpty = make("p", "", "No account is connected.");
    this.accounts = make("div", "list");
    accounts.part.append(
      make("p", "note", "Connecting an account, choosing calendars and naming the Agent Drafts calendar are not in this build."),
      this.accountsEmpty, this.accounts,
    );

    const activity = part(section, "Activity");
    this.activityHeading = activity.heading;
    this.activityEmpty = make("p", "", "Nothing yet.");
    this.activity = make("ul", "activity");
    activity.part.append(make("p", "note", "What the broker has reported, newest first."), this.activityEmpty, this.activity);
  }

  announce(text: string): void {
    // A trailing space alternates so that the same sentence twice is still announced.
    const same = this.announcer.textContent?.replace(/ $/, "") === text;
    this.announcer.textContent = same && !this.announcer.textContent?.endsWith(" ") ? `${text} ` : text;
  }

  render(): void {
    const m = this.model;
    setText(byId("build-line"), buildLine(m));
    this.renderStrip();

    const connected = m.connection?.name === "connected";
    setText(this.connection, connectionText(m));
    setText(this.brokerName, brokerText(m));
    setText(this.schemaVersion, m.broker === null ? "Not known yet" : String(m.broker.schema_version));
    setText(this.lastCheck, lastCheckText(m));
    textLine(this.problem, m.problem, this.statusHeading);
    setDisabled(this.check, !connected || m.checking);
    setText(this.checkNote, !connected ? "Not available while the broker is not running." : m.checking ? "Checking…" : "");
    setText(this.store, `${storeText(m)}.`);
    setText(this.prover, `${proverText(m)}.`);
    textLine(this.proverCause, proverCauseText(m), this.statusHeading);

    const accounts = [...m.accounts.values()];
    keyed(this.accounts, accounts, (a) => a.account_id, () => this.createAccount(), (node, a) => this.updateAccount(node, a), this.accountsHeading);
    show(this.accountsEmpty, accounts.length === 0, this.accountsHeading);

    for (const item of m.activity) {
      if (this.logged.has(item)) continue;
      this.logged.add(item);
      this.activity.prepend(make("li", "", `Notice ${item.sequence}: ${item.text}`));
    }
    while (this.activity.children.length > ACTIVITY_LIMIT) this.activity.lastElementChild?.remove();
    show(this.activityEmpty, m.activity.length === 0, this.activityHeading);
  }

  private renderStrip(): void {
    const fallback = byId(`${this.nav.current}-heading`);
    const items = attention(this.model);
    keyed(this.stripList, items, (i) => `${i.key}:${i.action ?? ""}`, (i) => this.createItem(i), (node, i) => this.updateItem(node, i), fallback);
    show(this.strip, items.length > 0, fallback);
  }

  private createItem(item: AttentionItem): HTMLLIElement {
    const node = make("li", "item");
    const title = make("p", "title");
    const text = make("p");
    const detail = make("p", "detail");
    const note = make("p", "note");
    note.id = uid("attention-note");
    node.append(title, text, detail);
    let control: HTMLElement | null = null;
    if (item.action === "restart") control = button("Restart broker", () => this.model.restart());
    else if (item.action === "retry") control = button("Read the current state again", () => this.model.retrySync());
    else if (item.action === "review") {
      const link = make("a", "", "Review the waiting requests");
      link.href = "#review";
      link.addEventListener("click", (event) => {
        event.preventDefault();
        this.nav.go("review", () => this.review.focusFirstWaiting());
      });
      control = link;
    }
    if (control !== null) {
      control.setAttribute("aria-describedby", note.id);
      const actions = make("div", "actions");
      actions.append(control, note);
      node.append(actions);
    }
    this.items.set(node, { node, title, text, detail, control, note });
    return node;
  }

  private updateItem(node: HTMLLIElement, item: AttentionItem): void {
    const dom = this.items.get(node);
    if (dom === undefined) return;
    const kind = `item ${item.kind === "problem" ? "panel" : "notice"}`;
    if (node.className !== kind) node.className = kind;
    setText(dom.title, item.title);
    textLine(dom.text, item.text, node);
    textLine(dom.detail, item.detail, node);
    if (item.action === "restart" && dom.control instanceof HTMLButtonElement) {
      setDisabled(dom.control, this.model.restarting);
      setText(dom.note, this.model.restarting ? "Starting the broker…" : "");
    }
  }

  private createAccount(): HTMLElement {
    const node = make("article", "item");
    const heading = make("h4");
    heading.id = uid("account");
    node.setAttribute("aria-labelledby", heading.id);
    const state = make("p", "state");
    const calendars = make("ul");
    node.append(heading, state, calendars);
    this.accountNodes.set(node, { node, heading, state, calendars, calendarsKey: "" });
    return node;
  }

  private updateAccount(node: HTMLElement, a: Account): void {
    const dom = this.accountNodes.get(node);
    if (dom === undefined) return;
    setText(dom.heading, a.email ?? `Account ${a.account_id}`);
    setText(dom.state, `${ACCOUNT_TEXT[a.state]}.`);
    const lines = a.calendars.length === 0 ? ["No calendar is known for this account."] : a.calendars.map(calendarLine);
    const key = JSON.stringify(lines);
    if (key === dom.calendarsKey) return;
    dom.calendarsKey = key;
    dom.calendars.replaceChildren(...lines.map((l) => make("li", "", l)));
  }
}
