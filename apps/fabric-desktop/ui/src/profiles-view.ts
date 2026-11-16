// The Agent permissions screen in #permissions (plan S02). Every node is made once and updated in
// place; what is shown and what is disabled comes decided from ProfilesModel.
import type { Mode, PolicyRow } from "../../../../contracts/desktop/v1/typescript/src/contract.ts";
import { button, byId, keyed, make, setDisabled, setText, show, textLine, uid } from "./dom.ts";
import type { Model } from "./model.ts";
import { EMAIL_TEXT, LOADING_TEXT, MODES, MODE_NAME, ONE_IN_FORCE, ProfilesModel, ROWS } from "./profiles.ts";
import type { Action, EditableRow, ListItem, RowState } from "./profiles.ts";

interface RowDom {
  readonly node: HTMLElement;
  readonly title: HTMLElement; // the legend, or the locked row's title
  readonly radios: ReadonlyMap<Mode, HTMLInputElement>;
  readonly stored: HTMLParagraphElement;
  readonly meaning: HTMLParagraphElement;
}

interface ChoiceDom {
  readonly input: HTMLInputElement;
  readonly name: HTMLSpanElement;
  readonly tags: HTMLSpanElement;
  tagsKey: string;
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

// A button with the visible reason beside it, tied by aria-describedby.
interface ActionDom {
  readonly button: HTMLButtonElement;
  readonly reason: HTMLParagraphElement;
}

function actionButton(onPress: () => void): ActionDom {
  const reason = make("p", "note");
  reason.id = uid("reason");
  const node = button("", onPress);
  node.setAttribute("aria-describedby", reason.id);
  return { button: node, reason };
}

function renderAction(a: ActionDom, state: Action): void {
  setText(a.button, state.label);
  setDisabled(a.button, state.disabled !== null);
  setText(a.reason, state.disabled ?? "");
}

export class ProfilesView {
  private readonly profiles: ProfilesModel;
  private readonly heading = byId("permissions-heading");
  private readonly loading: HTMLParagraphElement;
  private readonly content: HTMLDivElement;
  private readonly listHeading: HTMLHeadingElement;
  private readonly choices: HTMLDivElement;
  private readonly choiceNodes = new WeakMap<HTMLElement, ChoiceDom>();
  private readonly listGroup = uid("profile");
  private readonly activate: ActionDom;
  private readonly activateNote: HTMLParagraphElement;
  private readonly calendarHeading: HTMLHeadingElement;
  private readonly stale: HTMLDivElement;
  private readonly staleText: HTMLParagraphElement;
  private readonly nameField: HTMLDivElement;
  private readonly nameLabel: HTMLLabelElement;
  private readonly nameInput: HTMLInputElement;
  private readonly nameStored: HTMLParagraphElement;
  private readonly nameMessage: HTMLParagraphElement;
  private readonly rows = new Map<PolicyRow, RowDom>();
  private readonly save: ActionDom;
  private readonly discard: ActionDom;
  private readonly saveNote: HTMLParagraphElement;
  private readonly panel: HTMLDivElement;
  private readonly panelHeading: HTMLHeadingElement;
  private readonly panelList: HTMLUListElement;
  private readonly confirm: ActionDom;
  private panelShown = false;
  private staleShown = false;
  private nameAttention = 0;

  constructor(model: Model) {
    this.profiles = new ProfilesModel(model);
    const section = byId("permissions");
    this.loading = make("p", "", LOADING_TEXT);
    this.content = make("div");
    section.append(this.loading, this.content);

    // Profiles: which one is shown, and which one is in force.
    const list = part(this.content, "Profiles");
    this.listHeading = list.heading;
    const fieldset = make("fieldset");
    fieldset.append(make("legend", "", "Profile shown below"));
    this.choices = make("div", "choices");
    fieldset.append(this.choices);
    this.activate = actionButton(() => this.profiles.activate());
    this.activateNote = make("p", "problem");
    this.activateNote.hidden = true;
    const activateActions = make("div", "actions");
    activateActions.append(this.activate.button, this.activate.reason);
    list.part.append(fieldset, make("p", "note", ONE_IN_FORCE), activateActions, this.activateNote);

    // Calendar: the rows of the shown profile.
    const calendar = part(this.content, "Calendar");
    this.calendarHeading = calendar.heading;
    this.stale = make("div", "notice");
    this.staleText = make("p");
    const seen = button("I have seen the current version", () => {
      this.profiles.acknowledgeStale();
      this.save.button.focus();
    });
    this.stale.append(this.staleText, seen);
    this.stale.hidden = true;

    this.nameField = make("div", "field");
    this.nameLabel = make("label");
    this.nameInput = make("input");
    this.nameInput.type = "text";
    this.nameInput.id = uid("profile-name");
    this.nameLabel.htmlFor = this.nameInput.id;
    this.nameStored = make("p", "stored");
    this.nameStored.id = uid("name-stored");
    this.nameMessage = make("p", "problem");
    this.nameMessage.id = uid("name-message");
    this.nameInput.setAttribute("aria-describedby", `${this.nameMessage.id} ${this.nameStored.id}`);
    this.nameInput.addEventListener("input", () => this.profiles.setName(this.nameInput.value));
    this.nameField.append(this.nameLabel, this.nameInput, this.nameStored, this.nameMessage);
    this.nameField.hidden = true;
    for (const row of ROWS) calendar.part.append(this.createRow(row));
    // After the rows, right before the buttons: the name, then why Save may wait.
    calendar.part.append(this.nameField, this.stale);

    this.save = actionButton(() => this.profiles.save());
    this.save.button.className = "primary";
    this.discard = actionButton(() => this.profiles.discard());
    const actions = make("div", "actions");
    actions.append(this.save.button, this.save.reason, this.discard.button, this.discard.reason);
    this.saveNote = make("p", "problem");
    this.saveNote.hidden = true;

    // The expansion the broker listed, before a wider profile is saved.
    this.panel = make("div", "panel");
    this.panel.hidden = true;
    this.panelHeading = make("h4", "", "This profile permits more than before:");
    this.panelHeading.id = uid("expansion");
    this.panelHeading.tabIndex = -1;
    this.panel.setAttribute("role", "group");
    this.panel.setAttribute("aria-labelledby", this.panelHeading.id);
    this.panelList = make("ul");
    this.confirm = actionButton(() => this.profiles.confirmExpansion());
    this.confirm.button.className = "primary";
    const back = button("Go back", () => {
      this.profiles.goBack();
      this.save.button.focus();
    });
    const panelActions = make("div", "actions");
    panelActions.append(this.confirm.button, this.confirm.reason, back);
    this.panel.append(this.panelHeading, this.panelList, panelActions);
    calendar.part.append(actions, this.saveNote, this.panel);

    part(this.content, "Email").part.append(make("p", "", EMAIL_TEXT));
  }

  render(): void {
    const s = this.profiles.screen;
    show(this.loading, s === null, this.heading);
    show(this.content, s !== null, this.heading);
    if (s === null) return;

    keyed(this.choices, s.list, (i) => i.id, (i) => this.createChoice(i), (node, i) => this.updateChoice(node, i, s.shown), this.listHeading);
    renderAction(this.activate, s.activate);
    textLine(this.activateNote, s.activateNote, this.listHeading);

    textLine(this.staleText, s.stale, this.calendarHeading);
    show(this.stale, s.stale !== null, this.save.button);
    show(this.nameField, s.name !== null, this.calendarHeading);
    if (s.name !== null) {
      setText(this.nameLabel, s.name.label);
      if (this.nameInput.value !== s.name.value) this.nameInput.value = s.name.value;
      textLine(this.nameStored, s.name.stored, this.nameInput);
      textLine(this.nameMessage, s.name.message, this.nameInput);
      this.nameInput.setAttribute("aria-invalid", String(s.name.message !== null));
    }
    for (const r of s.rows) this.updateRow(r);

    renderAction(this.save, s.save);
    renderAction(this.discard, s.discard);
    textLine(this.saveNote, s.saveNote, this.calendarHeading);

    renderAction(this.confirm, s.confirm);
    const lines = s.panel ?? [];
    if (lines.length !== this.panelList.children.length || lines.some((l, i) => this.panelList.children[i]?.textContent !== l)) {
      this.panelList.replaceChildren(...lines.map((l) => make("li", "", l)));
    }
    show(this.panel, s.panel !== null, this.save.button);
    if (s.panel !== null && !this.panelShown) this.panelHeading.focus();
    this.panelShown = s.panel !== null;
    if (s.stale !== null && !this.staleShown) this.profiles.announceStale();
    this.staleShown = s.stale !== null;
    if (s.nameAttention !== this.nameAttention && s.name !== null) this.nameInput.focus();
    this.nameAttention = s.nameAttention;
  }

  private createRow(row: PolicyRow): HTMLElement {
    const stored = make("p", "stored");
    const meaning = make("p", "note");
    meaning.id = uid("meaning");
    const radios = new Map<Mode, HTMLInputElement>();
    let node: HTMLElement;
    let title: HTMLElement;
    if (row === "delete_event" || row === "delete_calendar_or_change_sharing") {
      node = make("div", "locked");
      title = make("p", "title");
      node.append(title, meaning);
    } else {
      const editable: EditableRow = row;
      node = make("fieldset");
      title = make("legend");
      node.setAttribute("aria-describedby", meaning.id);
      const choices = make("div", "choices");
      const group = uid("row");
      for (const mode of MODES) {
        const label = make("label", "choice");
        const input = make("input");
        input.type = "radio";
        input.name = group;
        input.value = mode;
        input.addEventListener("change", () => {
          if (input.checked) this.profiles.setMode(editable, mode);
        });
        label.append(input, make("span", "", MODE_NAME[mode]));
        choices.append(label);
        radios.set(mode, input);
      }
      node.append(title, choices, stored, meaning);
    }
    this.rows.set(row, { node, title, radios, stored, meaning });
    return node;
  }

  private updateRow(r: RowState): void {
    const dom = this.rows.get(r.row);
    if (dom === undefined) return;
    setText(dom.title, r.name);
    for (const [mode, input] of dom.radios) if (input.checked !== (mode === r.mode)) input.checked = mode === r.mode;
    textLine(dom.stored, r.stored, dom.node);
    setText(dom.meaning, r.meaning);
  }

  private createChoice(item: ListItem): HTMLElement {
    const label = make("label", "choice");
    const input = make("input");
    input.type = "radio";
    input.name = this.listGroup;
    input.value = item.id;
    input.addEventListener("change", () => {
      if (input.checked) this.profiles.choose(item.id);
    });
    const name = make("span");
    const tags = make("span");
    label.append(input, name, tags);
    this.choiceNodes.set(label, { input, name, tags, tagsKey: "" });
    return label;
  }

  private updateChoice(node: HTMLElement, item: ListItem, shown: string): void {
    const dom = this.choiceNodes.get(node);
    if (dom === undefined) return;
    setText(dom.name, item.name);
    const key = JSON.stringify(item.tags);
    if (key !== dom.tagsKey) {
      dom.tagsKey = key;
      // Each tag is spoken after a comma; the comma itself is not shown.
      dom.tags.replaceChildren(...item.tags.flatMap((t) => [make("span", "visually-hidden", ","), make("span", "tag", t)]));
    }
    if (dom.input.checked !== (item.id === shown)) dom.input.checked = item.id === shown;
  }
}
