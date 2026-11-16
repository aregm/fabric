// The Contacts screen without the DOM (plan S04): the groups the broker holds, one form at a
// time, and the two-step delete. A group is a recipient set: a name, exact addresses and
// domains. The page only checks the shape of what the owner types (contract `email`, `domain`,
// `label`, and the bounds of `recipient_set_definition`); what a group permits is the broker's.
import type { ParamsSaveGroup, RecipientSet, RecipientSetDefinition } from "../../../../contracts/desktop/v1/typescript/src/contract.ts";
import type { Model } from "./model.ts";
import { isLabel, normalAddress, normalDomain } from "./shape.ts";
import { accountName, formatInstant } from "./text.ts";

export const MAX_ADDRESSES = 2000;
export const MAX_DOMAINS = 50;
const NAMED_LINES = 10; // invalid lines named one by one; the rest are counted

export interface Availability {
  readonly enabled: boolean;
  readonly reason: string | null; // why it cannot be used now; null when it can
}

export type GroupField = "name" | "addresses" | "domains";

// Where focus goes after an action the owner started.
export type GroupFocus =
  | { readonly kind: "form" }
  | { readonly kind: "group"; readonly id: string } // the group's heading
  | { readonly kind: "delete"; readonly id: string } // the group's "Delete" button
  | { readonly kind: "new" };

export interface GroupItem {
  readonly id: string;
  readonly name: string;
  readonly counts: string; // "2 addresses and 1 domain."
  readonly source: string;
  readonly edit: Availability;
  readonly editLabel: string;
  readonly del: Availability;
  readonly deleteLabel: string;
  readonly confirming: string | null; // the second step of delete: its question, or null
  readonly confirmLabel: string; // "Yes, delete the group Family": a name of its own, not the first step's
  readonly note: string | null; // the last problem of a delete of this group
}

export interface GroupForm {
  readonly isNew: boolean;
  readonly title: string;
  readonly name: string;
  readonly addresses: string;
  readonly domains: string;
  readonly problems: Readonly<Record<GroupField, readonly string[]>>;
  readonly stored: Readonly<Record<GroupField, string | null>> | null; // while stale: the stored value beside a differing field
  readonly dirty: boolean;
  readonly stale: string | null; // what changed while the owner was editing
  readonly deleted: string | null; // the group was deleted while the owner was editing it
  readonly widening: string | null;
  readonly note: string | null; // the result of the last save that did not succeed
  readonly save: Availability;
  readonly saveLabel: string;
}

interface Draft {
  id: string | null; // null: a new group
  baseRevision: number; // 0 for a new group
  base: RecipientSetDefinition; // the stored definition the draft was made from
  name: string;
  addresses: string;
  domains: string;
  edits: number; // counts the owner's edits, to know whether a save in flight saved the current text
}

interface Parsed {
  readonly name: string;
  readonly members: readonly string[];
  readonly domains: readonly string[];
  readonly problems: Readonly<Record<GroupField, readonly string[]>>;
}

const EMPTY: RecipientSetDefinition = { name: "", members: [], domains: [] };
export const WIDENING = "Adding someone ends the permissions that name this group; the agents ask again.";
const NOT_KNOWN = "It is not known whether this was saved. Saving the same again is safe.";

function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

// The lines of a text in normal form: duplicates and empty lines dropped, invalid lines named.
function parseLines(text: string, normal: (line: string) => string | null, what: string, limit: number, many: string): {
  readonly values: string[];
  readonly problems: string[];
} {
  const values: string[] = [];
  const invalid: string[] = [];
  text.split("\n").forEach((line, index) => {
    if (line.trim() === "") return;
    const value = normal(line);
    if (value === null) invalid.push(`Line ${index + 1} is not ${what}: ${line.trim()}`);
    else if (!values.includes(value)) values.push(value);
  });
  const problems = invalid.slice(0, NAMED_LINES);
  if (invalid.length > NAMED_LINES) problems.push(`${plural(invalid.length - NAMED_LINES, "more line is", "more lines are")} not ${what}.`);
  if (values.length > limit) problems.push(`There are ${values.length} ${many}; a group holds at most ${limit}.`);
  return { values, problems };
}

function parse(d: Draft): Parsed {
  const name = d.name.trim();
  const nameProblems = name === "" ? ["Give the group a name."] : isLabel(name) ? [] : ["The name is longer than 200 characters."];
  const a = parseLines(d.addresses, normalAddress, "an address", MAX_ADDRESSES, "addresses");
  const m = parseLines(d.domains, normalDomain, "a domain", MAX_DOMAINS, "domains");
  return { name, members: a.values, domains: m.values, problems: { name: nameProblems, addresses: a.problems, domains: m.problems } };
}

const sameList = (a: readonly string[], b: readonly string[]): boolean => a.length === b.length && a.every((x, i) => x === b[i]);

function differs(p: Parsed, field: GroupField, def: RecipientSetDefinition): boolean {
  if (field === "name") return p.name !== def.name;
  if (p.problems[field].length > 0) return true;
  return !sameList(field === "addresses" ? p.members : p.domains, field === "addresses" ? def.members : def.domains);
}

const FIELDS: readonly GroupField[] = ["name", "addresses", "domains"];

export class GroupsModel {
  private readonly model: Model;
  private draft: Draft | null = null;
  private confirming: string | null = null; // the group whose delete waits for its second step
  private inFlight: "Saving…" | "Deleting…" | null = null; // the command on its way, in words
  private note: string | null = null;
  private deleteNote: { readonly id: string; readonly text: string } | null = null;
  private focus: GroupFocus | null = null;
  private said: string | null = null; // the stale or deleted state last announced

  constructor(model: Model) {
    this.model = model;
  }

  // ---- What the view shows ----

  get groups(): GroupItem[] {
    this.follow();
    const blocked = this.blockedReason();
    return [...this.model.recipientSets.values()].map((set) => {
      const name = set.definition.name;
      const other = this.draft?.id !== set.recipient_set_id ? blocked : null;
      const busy = !this.model.ready ? this.model.notReadyText : this.inFlight;
      return {
        id: set.recipient_set_id,
        name,
        counts: `${plural(set.definition.members.length, "address", "addresses")} and ${plural(set.definition.domains.length, "domain", "domains")}.`,
        source: this.sourceText(set),
        edit: { enabled: other === null, reason: other },
        editLabel: `Edit the group ${name}`,
        del: { enabled: other === null && busy === null, reason: other ?? busy },
        deleteLabel: `Delete the group ${name}`,
        confirming: this.confirming === set.recipient_set_id ? `Delete ${name}? Permissions that name it end.` : null,
        confirmLabel: `Yes, delete the group ${name}`,
        note: this.deleteNote?.id === set.recipient_set_id ? this.deleteNote.text : null,
      };
    });
  }

  get newGroup(): Availability {
    this.follow();
    const blocked = this.draft?.id === null ? null : this.blockedReason();
    return { enabled: blocked === null, reason: blocked };
  }

  get form(): GroupForm | null {
    this.follow();
    const d = this.draft;
    if (d === null) return null;
    const p = parse(d);
    const dirty = this.isDirty(d, p);
    const current = d.id === null ? null : this.model.recipientSets.get(d.id) ?? null;
    // While the owner's own save or delete is awaited, its notice (which the broker sends before
    // the answer) is not a change made by someone else: no staleness is reported until it ends.
    const awaited = this.inFlight !== null;
    const deleted = !awaited && d.id !== null && current === null
      ? `The group ${d.base.name} was deleted while you were editing it. Your text stays here so that you can copy it; it cannot be saved.`
      : null;
    const stale = !awaited && deleted === null && current !== null && current.revision !== d.baseRevision
      ? `The group ${current.definition.name} was changed while you were editing it. Your changes are kept. The stored version is shown beside each field that differs.`
      : null;
    let stored: Record<GroupField, string | null> | null = null;
    if (stale !== null && current !== null) {
      const def = current.definition;
      const text = (f: GroupField): string => (f === "name" ? def.name : (f === "addresses" ? def.members : def.domains).join(", ") || "none");
      stored = { name: null, addresses: null, domains: null };
      for (const f of FIELDS) if (differs(p, f, def)) stored[f] = `Stored now: ${text(f)}`;
    }
    const problems = dirty && deleted === null ? p.problems : { name: [], addresses: [], domains: [] };
    const first = FIELDS.flatMap((f) => problems[f])[0];
    const reason = !this.model.ready ? this.model.notReadyText
      : this.inFlight !== null ? this.inFlight
      : deleted !== null ? "This group no longer exists, so it cannot be saved."
      : stale !== null ? "Look at the stored version, then press \"I have seen the current version\"."
      : !dirty ? "Nothing has changed."
      : first ?? null;
    const reference = current?.definition ?? d.base;
    const widens = d.id !== null && deleted === null
      && (p.members.some((m) => !reference.members.includes(m)) || p.domains.some((x) => !reference.domains.includes(x)));
    return {
      isNew: d.id === null,
      title: d.id === null ? "New group" : `Edit the group ${d.base.name}`,
      name: d.name,
      addresses: d.addresses,
      domains: d.domains,
      problems,
      stored,
      dirty,
      stale,
      deleted,
      widening: widens ? WIDENING : null,
      note: this.note,
      save: { enabled: reason === null, reason },
      saveLabel: d.id === null ? "Save the new group" : `Save the group ${d.base.name}`,
    };
  }

  // Says once that the form became stale, or that its group was deleted meanwhile. The view
  // calls it when it shows either; the same state is not said twice.
  sayChanged(): void {
    const d = this.draft;
    const f = this.form;
    if (d === null || f === null || (f.stale === null && f.deleted === null)) return;
    const current = d.id === null ? undefined : this.model.recipientSets.get(d.id);
    const key = `${d.id ?? ""}:${current?.revision ?? "deleted"}`;
    if (this.said === key) return;
    this.said = key;
    this.model.announce(f.deleted !== null
      ? `The group ${d.base.name} was deleted while you were editing. Your text stays so that you can copy it; it cannot be saved.`
      : "The group changed while you were editing. Your changes are kept. Look at the current version before you save.");
  }

  // The focus an action asked for, once.
  takeFocus(): GroupFocus | null {
    const f = this.focus;
    this.focus = null;
    return f;
  }

  // ---- What the owner does ----

  openNew(): boolean {
    if (!this.newGroup.enabled) return false;
    if (this.draft?.id !== null) this.open({ id: null, baseRevision: 0, base: EMPTY, name: "", addresses: "", domains: "", edits: 0 });
    this.focus = { kind: "form" };
    this.model.changed();
    return true;
  }

  openEdit(id: string): boolean {
    const set = this.model.recipientSets.get(id);
    const item = this.groups.find((g) => g.id === id);
    if (set === undefined || item === undefined || !item.edit.enabled) return false;
    if (this.draft?.id !== id) this.open(this.fromSet(set));
    this.focus = { kind: "form" };
    this.model.changed();
    return true;
  }

  edit(field: GroupField, text: string): void {
    this.follow();
    const d = this.draft;
    if (d === null || d[field] === text) return;
    d[field] = text;
    d.edits += 1;
    this.model.changed();
  }

  // The owner saw the stored version: the draft is now written against it.
  acknowledge(): void {
    const d = this.draft;
    const current = d?.id == null ? undefined : this.model.recipientSets.get(d.id);
    if (d === null || current === undefined) return;
    d.baseRevision = current.revision;
    d.base = current.definition;
    this.note = null;
    this.model.changed();
  }

  discard(): void {
    const d = this.draft;
    if (d === null) return;
    this.close(d.id);
    this.model.changed();
  }

  save(): boolean {
    const d = this.draft;
    const form = this.form;
    if (d === null || form === null || !form.save.enabled) return false;
    const p = parse(d);
    const definition: RecipientSetDefinition = { name: p.name, members: p.members, domains: p.domains };
    const params: ParamsSaveGroup = { expected_revision: d.baseRevision, change: { kind: "save", recipient_set_id: d.id, definition } };
    const edits = d.edits;
    this.inFlight = "Saving…";
    this.note = null;
    this.model.command("save_group", params, (result) => {
      this.inFlight = null;
      const set = result.recipient_set;
      this.model.announce(`The group ${definition.name} is saved.`);
      if (set === null || this.draft !== d) return;
      if (d.edits === edits) {
        this.close(set.recipient_set_id);
      } else {
        // The owner typed on while the save was on its way: the newer text stays, written against the saved version.
        d.id = set.recipient_set_id;
        d.baseRevision = set.revision;
        d.base = set.definition;
      }
    }, (f) => {
      this.inFlight = null;
      const text = f.code === null ? NOT_KNOWN
        : f.code === "conflict" ? "Nothing was saved: the group changed since you opened it. The current version is read again."
        : `The broker did not save this: ${f.text}`;
      if (f.code === "conflict") this.model.refresh();
      if (this.draft === d) this.note = text;
      this.model.announce(text);
    });
    this.model.changed();
    return true;
  }

  askDelete(id: string): boolean {
    const item = this.groups.find((g) => g.id === id);
    if (item === undefined || !item.del.enabled) return false;
    this.confirming = id;
    this.deleteNote = null;
    this.model.changed();
    return true;
  }

  cancelDelete(): void {
    const id = this.confirming;
    if (id === null) return;
    this.confirming = null;
    this.focus = { kind: "delete", id };
    this.model.changed();
  }

  confirmDelete(): boolean {
    const id = this.confirming;
    const set = id === null ? undefined : this.model.recipientSets.get(id);
    const item = this.groups.find((g) => g.id === id);
    if (id === null || set === undefined || item === undefined || !item.del.enabled) return false;
    const name = set.definition.name;
    const params: ParamsSaveGroup = { expected_revision: set.revision, change: { kind: "delete", recipient_set_id: id } };
    this.inFlight = "Deleting…";
    this.deleteNote = null;
    this.model.command("save_group", params, () => {
      this.inFlight = null;
      if (this.confirming === id) this.confirming = null;
      if (this.draft?.id === id) this.draft = null;
      this.note = null;
      this.focus = { kind: "new" };
      this.model.announce(`The group ${name} is deleted.`);
    }, (f) => {
      this.inFlight = null;
      const text = f.code === null
        ? `It is not known whether the group ${name} was deleted. Deleting it again is safe.`
        : f.code === "conflict"
          ? `Nothing was deleted: the group ${name} changed since you saw it. Look at it again before you delete it.`
          : `The broker did not delete the group ${name}: ${f.text}`;
      if (f.code !== null && this.confirming === id) this.confirming = null; // only the same delete may be sent again
      if (f.code === "conflict") this.model.refresh();
      this.deleteNote = { id, text };
      this.model.announce(text);
    });
    this.model.changed();
    return true;
  }

  // ---- Inside ----

  private open(d: Draft): void {
    this.draft = d;
    this.note = null;
  }

  private close(focusId: string | null): void {
    this.draft = null;
    this.note = null;
    this.focus = focusId !== null && this.model.recipientSets.has(focusId) ? { kind: "group", id: focusId } : { kind: "new" };
  }

  private fromSet(set: RecipientSet): Draft {
    const def = set.definition;
    return {
      id: set.recipient_set_id, baseRevision: set.revision, base: def,
      name: def.name, addresses: def.members.join("\n"), domains: def.domains.join("\n"), edits: 0,
    };
  }

  private isDirty(d: Draft, p: Parsed = parse(d)): boolean {
    return FIELDS.some((f) => differs(p, f, d.base));
  }

  // A draft without changes follows the stored group: a newer version replaces it, and a
  // deleted group closes it. A draft with changes is kept as it is.
  private follow(): void {
    const d = this.draft;
    if (d === null || d.id === null || this.inFlight !== null || this.isDirty(d)) return;
    const current = this.model.recipientSets.get(d.id);
    if (current === undefined) this.draft = null;
    else if (current.revision !== d.baseRevision) this.draft = { ...this.fromSet(current), edits: d.edits };
  }

  // Why another group cannot be opened or deleted now: the open form has unsaved changes.
  private blockedReason(): string | null {
    const d = this.draft;
    if (d === null || !this.isDirty(d)) return null;
    const name = d.id === null ? (d.name.trim() || "the new group") : d.base.name;
    return `Save or discard your changes to ${name} first.`;
  }

  private sourceText(set: RecipientSet): string {
    const s = set.source;
    if (s.kind === "local") return "Made in Fabric.";
    return `Imported from Google contacts of ${accountName(this.model.names, s.account_id)} on ${formatInstant(s.imported_at, this.model.zone)}.`;
  }
}
