// The Contacts screen (plan S04) in #contacts: the groups, "New group", and the form below them.
// Every decision is GroupsModel's; this file only puts it on the page, updated in place.
import { button, byId, keyed, make, setDisabled, setText, show, textLine, uid } from "./dom.ts";
import { GroupsModel } from "./groups.ts";
import type { GroupField, GroupItem } from "./groups.ts";
import type { Model } from "./model.ts";

interface ItemDom {
  readonly heading: HTMLHeadingElement;
  readonly counts: HTMLParagraphElement;
  readonly source: HTMLParagraphElement;
  readonly edit: HTMLButtonElement;
  readonly del: HTMLButtonElement;
  readonly reason: HTMLParagraphElement;
  readonly confirm: HTMLDivElement;
  readonly question: HTMLParagraphElement;
  readonly confirmButton: HTMLButtonElement;
  readonly keep: HTMLButtonElement;
  readonly note: HTMLParagraphElement;
}

interface FieldDom {
  readonly control: HTMLInputElement | HTMLTextAreaElement;
  readonly stored: HTMLParagraphElement;
  readonly problems: HTMLDivElement;
  problemsKey: string;
}

// A button with a short visible text and the rest of its name for screen readers.
function named(visible: string, onPress: () => void, className = ""): { readonly node: HTMLButtonElement; readonly hidden: HTMLSpanElement } {
  const node = button(visible, onPress, className);
  const hidden = make("span", "visually-hidden");
  node.append(hidden);
  return { node, hidden };
}

function part(parent: HTMLElement, title: string): { readonly part: HTMLDivElement; readonly heading: HTMLHeadingElement } {
  const node = make("div", "part");
  const heading = make("h3", "", title);
  heading.id = uid("part");
  heading.tabIndex = -1;
  node.append(heading);
  parent.append(node);
  return { part: node, heading };
}

export class GroupsView {
  private readonly groups: GroupsModel;
  private readonly sectionHeading = byId("contacts-heading");
  private readonly listHeading: HTMLHeadingElement;
  private readonly empty: HTMLParagraphElement;
  private readonly list: HTMLDivElement;
  private readonly newButton: HTMLButtonElement;
  private readonly newReason: HTMLParagraphElement;
  private readonly items = new WeakMap<HTMLElement, ItemDom>();
  private readonly headings = new Map<string, HTMLHeadingElement>();
  private readonly deleteButtons = new Map<string, HTMLButtonElement>();
  // The form.
  private readonly formPart: HTMLDivElement;
  private readonly formHeading: HTMLHeadingElement;
  private readonly stale: HTMLDivElement;
  private readonly staleText: HTMLParagraphElement;
  private readonly deleted: HTMLParagraphElement;
  private readonly fields: Record<GroupField, FieldDom>;
  private readonly widening: HTMLParagraphElement;
  private readonly save: HTMLButtonElement;
  private readonly saveReason: HTMLParagraphElement;
  private readonly note: HTMLParagraphElement;

  constructor(model: Model) {
    this.groups = new GroupsModel(model);
    const section = byId("contacts");
    section.append(make("p", "note",
      "A group only names people. It gives no permission by itself; it can be chosen when you give one. Deleting a group changes nothing in Google."));

    const list = part(section, "Groups");
    this.listHeading = list.heading;
    this.empty = make("p", "", "No group yet.");
    this.list = make("div", "list");
    const actions = make("div", "actions");
    this.newReason = make("p", "note");
    this.newReason.id = uid("new-reason");
    this.newButton = button("New group", () => this.groups.openNew());
    this.newButton.setAttribute("aria-describedby", this.newReason.id);
    actions.append(this.newButton, this.newReason);
    list.part.append(this.empty, this.list, actions);

    const form = part(section, "");
    this.formPart = form.part;
    this.formHeading = form.heading;
    this.formPart.hidden = true;
    this.stale = make("div", "panel");
    this.staleText = make("p");
    this.stale.append(this.staleText, button("I have seen the current version", () => {
      this.groups.acknowledge();
      this.save.focus();
    }));
    this.deleted = make("p", "panel");
    this.fields = {
      name: this.field("name", "Name", make("input")),
      addresses: this.field("addresses", "Addresses, one per line", make("textarea")),
      domains: this.field("domains", "Domains, one per line", make("textarea"),
        "Everyone whose address ends in exactly this domain belongs to the group. example.com does not include mail.example.com."),
    };
    (this.fields.name.control as HTMLInputElement).type = "text";
    this.widening = make("p", "notice");
    this.widening.id = uid("widening");
    const formActions = make("div", "actions");
    this.saveReason = make("p", "note");
    this.saveReason.id = uid("save-reason");
    this.save = button("", () => this.groups.save(), "primary");
    this.note = make("p", "problem");
    this.note.id = uid("save-note");
    this.save.setAttribute("aria-describedby", `${this.widening.id} ${this.saveReason.id} ${this.note.id}`);
    formActions.append(this.save, button("Discard changes", () => this.groups.discard()), this.saveReason);
    // What stops the save stands right before the save button that names it.
    this.formPart.append(this.fields.name.control.parentElement as HTMLElement,
      this.fields.addresses.control.parentElement as HTMLElement, this.fields.domains.control.parentElement as HTMLElement,
      this.widening, this.deleted, this.stale, formActions, this.note);
  }

  render(): void {
    const g = this.groups;
    const items = g.groups;
    keyed(this.list, items, (i) => i.id, (i) => this.createItem(i), (node, i) => this.updateItem(node, i), this.listHeading);
    for (const id of [...this.headings.keys()]) {
      if (items.some((i) => i.id === id)) continue;
      this.headings.delete(id);
      this.deleteButtons.delete(id);
    }
    show(this.empty, items.length === 0, this.listHeading);
    const fresh = g.newGroup;
    setDisabled(this.newButton, !fresh.enabled);
    setText(this.newReason, fresh.reason ?? "");

    const f = g.form;
    if (f === null) {
      show(this.formPart, false, this.newButton);
    } else {
      setText(this.formHeading, f.title);
      show(this.formPart, true, this.formHeading);
      show(this.stale, f.stale !== null, this.save);
      setText(this.staleText, f.stale ?? "");
      textLine(this.deleted, f.deleted, this.save);
      if (f.stale !== null || f.deleted !== null) g.sayChanged();
      for (const name of ["name", "addresses", "domains"] as const) {
        const dom = this.fields[name];
        if (dom.control.value !== f[name]) dom.control.value = f[name];
        textLine(dom.stored, f.stored?.[name] ?? null, dom.control);
        const lines = f.problems[name];
        const key = lines.join("\n");
        if (key !== dom.problemsKey) {
          dom.problemsKey = key;
          dom.problems.replaceChildren(...lines.map((l) => make("p", "", l)));
        }
        dom.problems.hidden = lines.length === 0;
      }
      textLine(this.widening, f.widening, this.save);
      setText(this.save, f.saveLabel);
      setDisabled(this.save, !f.save.enabled);
      setText(this.saveReason, f.save.reason ?? "");
      textLine(this.note, f.note, this.save);
    }

    const focus = g.takeFocus();
    if (focus?.kind === "form") this.formHeading.focus();
    else if (focus?.kind === "group") (this.headings.get(focus.id) ?? this.newButton).focus();
    else if (focus?.kind === "delete") (this.deleteButtons.get(focus.id) ?? this.newButton).focus();
    else if (focus?.kind === "new") this.newButton.focus();
  }

  private field(which: GroupField, label: string, control: HTMLInputElement | HTMLTextAreaElement, help = ""): FieldDom {
    const node = make("div", "field");
    const name = make("label", "", label);
    control.id = uid("group-field");
    name.htmlFor = control.id;
    const stored = make("p", "stored");
    stored.id = uid("stored");
    stored.hidden = true;
    const problems = make("div", "problem");
    problems.id = uid("problems");
    problems.hidden = true;
    const ids = [stored.id, problems.id];
    node.append(name);
    if (help !== "") {
      const h = make("p", "note", help);
      h.id = uid("help");
      ids.unshift(h.id);
      node.append(h);
    }
    node.append(control, stored, problems);
    control.setAttribute("aria-describedby", ids.join(" "));
    const dom: FieldDom = { control, stored, problems, problemsKey: "" };
    control.addEventListener("input", () => this.groups.edit(which, control.value));
    return dom;
  }

  private createItem(item: GroupItem): HTMLElement {
    const node = make("article", "item");
    const heading = make("h4");
    heading.id = uid("group");
    heading.tabIndex = -1;
    node.setAttribute("aria-labelledby", heading.id);
    const counts = make("p");
    const source = make("p", "detail");
    const reason = make("p", "note");
    reason.id = uid("group-reason");
    const edit = named("Edit", () => this.groups.openEdit(item.id));
    const del = named("Delete", () => {
      if (this.groups.askDelete(item.id)) dom.keep.focus();
    });
    const confirmButton = button("", () => this.groups.confirmDelete(), "primary");
    for (const b of [edit.node, del.node, confirmButton]) b.setAttribute("aria-describedby", reason.id);
    const actions = make("div", "actions");
    actions.append(edit.node, del.node, reason);
    // The second step: the question, then "Keep it" (it takes focus), then the confirming button.
    const confirm = make("div", "panel");
    confirm.hidden = true;
    const question = make("p", "title");
    const keep = button("Keep it", () => this.groups.cancelDelete());
    const confirmActions = make("div", "actions");
    confirmActions.append(keep, confirmButton);
    confirm.append(question, confirmActions);
    const note = make("p", "problem");
    note.hidden = true;
    node.append(heading, counts, source, actions, confirm, note);
    const dom: ItemDom = {
      heading, counts, source, edit: edit.node, del: del.node, reason, confirm, question, confirmButton, keep, note,
    };
    this.items.set(node, dom);
    this.headings.set(item.id, heading);
    this.deleteButtons.set(item.id, del.node);
    return node;
  }

  private updateItem(node: HTMLElement, item: GroupItem): void {
    const dom = this.items.get(node);
    if (dom === undefined) return;
    setText(dom.heading, item.name);
    setText(dom.counts, item.counts);
    setText(dom.source, item.source);
    // The hidden rest of each button's name: "Edit" + " the group Family".
    const rest = (b: HTMLButtonElement, label: string, visible: string): void => {
      const span = b.querySelector("span");
      if (span !== null) setText(span, label.slice(visible.length));
    };
    rest(dom.edit, item.editLabel, "Edit");
    rest(dom.del, item.deleteLabel, "Delete");
    setText(dom.confirmButton, item.confirmLabel);
    setDisabled(dom.edit, !item.edit.enabled);
    setDisabled(dom.del, !item.del.enabled);
    setText(dom.reason, item.edit.reason ?? item.del.reason ?? "");
    setText(dom.question, item.confirming ?? "");
    setDisabled(dom.confirmButton, !item.del.enabled);
    // While the second step is open, the first step's buttons are hidden: no two buttons share a name.
    const confirming = item.confirming !== null;
    show(dom.confirm, confirming, dom.heading);
    show(dom.edit, !confirming, dom.keep);
    show(dom.del, !confirming, dom.keep);
    textLine(dom.note, item.note, dom.heading);
  }
}
