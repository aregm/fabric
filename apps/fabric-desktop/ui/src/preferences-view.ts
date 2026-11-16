// The Calendar preferences screen (plan S06) in #preferences: one form, one part per group of
// settings. The controls are made once and updated in place; every decision is
// PreferencesModel's.
import type { Weekday } from "../../../../contracts/desktop/v1/typescript/src/contract.ts";
import { button, byId, keyed, make, setDisabled, setText, show, textLine, uid } from "./dom.ts";
import type { Model } from "./model.ts";
import { DAYS, PART_OF_DAY, PreferencesModel } from "./preferences.ts";
import type { CalendarChoice, PartId, PartOfDay, PreferencesForm, ProtectedDraft, RangeDraft, RangeTarget } from "./preferences.ts";

interface PartDom {
  readonly node: HTMLDivElement;
  readonly heading: HTMLHeadingElement;
  readonly stored: HTMLParagraphElement;
  readonly problems: HTMLDivElement;
}

interface RangeDom {
  readonly days: Map<Weekday, HTMLInputElement>;
  readonly start: HTMLInputElement;
  readonly end: HTMLInputElement;
}

interface EntryDom {
  readonly legend: HTMLLegendElement;
  readonly enabled: HTMLInputElement;
  readonly range: RangeDom;
  readonly remove: HTMLButtonElement;
  readonly removeName: HTMLSpanElement;
  readonly problems: HTMLDivElement;
}

function setLines(node: HTMLElement, lines: readonly string[]): void {
  const key = lines.join("\n");
  if (node.dataset["lines"] !== key) {
    node.dataset["lines"] = key;
    node.replaceChildren(...lines.map((l) => make("p", "", l)));
  }
  node.hidden = lines.length === 0;
}

function setValue(control: HTMLInputElement | HTMLSelectElement, value: string): void {
  if (control.value !== value) control.value = value;
}

function setChecked(control: HTMLInputElement, checked: boolean): void {
  if (control.checked !== checked) control.checked = checked;
}

function describe(control: HTMLElement, ...ids: string[]): void {
  control.setAttribute("aria-describedby", ids.join(" "));
}

// A checkbox with its label to the right.
function checkbox(label: string, onChange: (checked: boolean) => void): { readonly node: HTMLLabelElement; readonly input: HTMLInputElement } {
  const node = make("label", "choice");
  const input = make("input");
  input.type = "checkbox";
  input.addEventListener("change", () => onChange(input.checked));
  node.append(input, make("span", "", label));
  return { node, input };
}

// A label above its control.
function field(label: string, control: HTMLInputElement | HTMLSelectElement): HTMLDivElement {
  const node = make("div", "field");
  const name = make("label", "", label);
  control.id = uid("pref");
  name.htmlFor = control.id;
  node.append(name, control);
  return node;
}

function timeInput(onInput: (text: string) => void): HTMLInputElement {
  const input = make("input");
  input.type = "time";
  input.addEventListener("input", () => onInput(input.value));
  return input;
}

function minutesInput(max: number, onInput: (text: string) => void): HTMLInputElement {
  const input = make("input");
  input.type = "number";
  input.inputMode = "numeric";
  input.min = "1";
  input.max = String(max);
  input.step = "1";
  input.addEventListener("input", () => onInput(input.value));
  return input;
}

export class PreferencesView {
  private readonly prefs: PreferencesModel;
  private readonly model: Model;
  private readonly unknown: HTMLParagraphElement;
  private readonly form: HTMLDivElement;
  private readonly parts: Record<PartId, PartDom>;
  private readonly stale: HTMLDivElement;
  private readonly staleText: HTMLParagraphElement;
  // Time zone.
  private readonly zone: HTMLInputElement;
  private readonly zoneButton: HTMLButtonElement;
  // Working hours.
  private readonly workdaySet: HTMLInputElement;
  private readonly workdayControls: HTMLDivElement;
  private readonly workday: RangeDom;
  // Protected times.
  private readonly entries: HTMLDivElement;
  private readonly entryNodes = new WeakMap<HTMLElement, EntryDom>();
  private readonly add: HTMLButtonElement;
  private readonly addReason: HTMLParagraphElement;
  // Uninterrupted time.
  private readonly uninterruptedSet: HTMLInputElement;
  private readonly uninterruptedControls: HTMLDivElement;
  private readonly uninterruptedEnabled: HTMLInputElement;
  private readonly uninterruptedMinutes: HTMLInputElement;
  private readonly partOfDay: HTMLSelectElement;
  // Time between events.
  private readonly bufferSet: HTMLInputElement;
  private readonly bufferControls: HTMLDivElement;
  private readonly bufferEnabled: HTMLInputElement;
  private readonly bufferMinutes: HTMLInputElement;
  // Calendars.
  private readonly calendars: HTMLDivElement;
  private readonly calendarsEmpty: HTMLParagraphElement;
  private readonly calendarNodes = new WeakMap<HTMLElement, { readonly input: HTMLInputElement; readonly text: HTMLSpanElement }>();
  // Save.
  private readonly save: HTMLButtonElement;
  private readonly saveReason: HTMLParagraphElement;
  private readonly note: HTMLParagraphElement;

  constructor(model: Model) {
    this.model = model;
    this.prefs = new PreferencesModel(model);
    const p = this.prefs;
    const section = byId("preferences");
    section.append(make("p", "note",
      "These preferences are for warnings about your time. They apply to every agent and every profile. A warning never blocks an action. This build only stores them; the warnings come with a later build."));
    this.unknown = make("p", "note");
    this.form = make("div");
    section.append(this.unknown, this.form);

    this.stale = make("div", "panel");
    this.staleText = make("p");
    this.stale.append(this.staleText, button("I have seen the current version", () => p.acknowledge()));
    this.stale.hidden = true;
    this.form.append(this.stale);

    const parts = {} as Record<PartId, PartDom>;
    const part = (id: PartId, title: string): PartDom => {
      const node = make("div", "part");
      const heading = make("h3", "", title);
      heading.id = uid("part");
      heading.tabIndex = -1;
      const stored = make("p", "stored");
      stored.id = uid("stored");
      stored.hidden = true;
      const problems = make("div", "problem");
      problems.id = uid("problems");
      problems.hidden = true;
      node.append(heading);
      this.form.append(node);
      parts[id] = { node, heading, stored, problems };
      return parts[id];
    };

    // Time zone.
    const zonePart = part("zone", "Time zone");
    this.zone = make("input");
    this.zone.type = "text";
    this.zone.autocomplete = "off";
    this.zone.spellcheck = false;
    this.zone.addEventListener("input", () => p.setZone(this.zone.value));
    const zoneHelp = make("p", "note", "A name like America/Chicago. Empty means not set.");
    zoneHelp.id = uid("help");
    const zoneField = field("Time zone", this.zone);
    zoneField.append(zoneHelp);
    describe(this.zone, zoneHelp.id, zonePart.stored.id, zonePart.problems.id);
    const zoneActions = make("div", "actions");
    this.zoneButton = button("", () => p.useComputerZone());
    zoneActions.append(this.zoneButton);
    zonePart.node.append(zoneField, zoneActions, zonePart.stored, zonePart.problems);

    // Working hours.
    const workPart = part("workday", "Working hours");
    const workSet = checkbox("Set my working hours", (on) => p.setOn("workday", on));
    this.workdaySet = workSet.input;
    describe(this.workdaySet, workPart.stored.id, workPart.problems.id);
    this.workdayControls = make("div");
    this.workday = this.range("workday", this.workdayControls, [workPart.stored.id, workPart.problems.id]);
    workPart.node.append(workSet.node, this.workdayControls, workPart.stored, workPart.problems);

    // Protected times.
    const protPart = part("protected", "Protected times");
    this.entries = make("div", "list");
    const addActions = make("div", "actions");
    this.addReason = make("p", "note");
    this.addReason.id = uid("add-reason");
    this.add = button("Add a protected time", () => this.addEntry());
    describe(this.add, this.addReason.id);
    addActions.append(this.add, this.addReason);
    protPart.node.append(
      make("p", "note", "Times you want to keep free. Turn one off to keep it without warnings."),
      this.entries, addActions, protPart.stored, protPart.problems,
    );

    // Uninterrupted time.
    const unPart = part("uninterrupted", "Uninterrupted time");
    const unSet = checkbox("Keep a block of free time", (on) => p.setOn("uninterrupted", on));
    this.uninterruptedSet = unSet.input;
    this.uninterruptedControls = make("div");
    const unOn = checkbox("Turned on", (on) => p.setEnabled("uninterrupted", on));
    this.uninterruptedEnabled = unOn.input;
    this.uninterruptedMinutes = minutesInput(1440, (t) => p.setMinutes("uninterrupted", t));
    this.partOfDay = make("select");
    for (const [value, text] of Object.entries(PART_OF_DAY)) {
      const option = make("option", "", text);
      option.value = value;
      this.partOfDay.append(option);
    }
    this.partOfDay.addEventListener("change", () => p.setPartOfDay(this.partOfDay.value as PartOfDay));
    this.uninterruptedControls.append(unOn.node, field("At least this many free minutes in a row (1 to 1440)", this.uninterruptedMinutes),
      field("Part of day", this.partOfDay));
    for (const c of [this.uninterruptedSet, this.uninterruptedEnabled, this.uninterruptedMinutes, this.partOfDay]) {
      describe(c, unPart.stored.id, unPart.problems.id);
    }
    unPart.node.append(unSet.node, this.uninterruptedControls, unPart.stored, unPart.problems);

    // Time between events.
    const bufPart = part("buffer", "Time between events");
    const bufSet = checkbox("Leave time between events", (on) => p.setOn("buffer", on));
    this.bufferSet = bufSet.input;
    this.bufferControls = make("div");
    const bufOn = checkbox("Turned on", (on) => p.setEnabled("buffer", on));
    this.bufferEnabled = bufOn.input;
    this.bufferMinutes = minutesInput(240, (t) => p.setMinutes("buffer", t));
    this.bufferControls.append(bufOn.node, field("Minutes between events (1 to 240)", this.bufferMinutes));
    for (const c of [this.bufferSet, this.bufferEnabled, this.bufferMinutes]) describe(c, bufPart.stored.id, bufPart.problems.id);
    bufPart.node.append(bufSet.node, this.bufferControls, bufPart.stored, bufPart.problems);

    // Calendars used for warnings.
    const calPart = part("calendars", "Calendars used for warnings");
    this.calendars = make("div", "choices");
    this.calendarsEmpty = make("p", "", "No calendar is known yet.");
    calPart.node.append(make("p", "note", "The events of these calendars are what warnings are about."),
      this.calendarsEmpty, this.calendars, calPart.stored, calPart.problems);
    this.parts = parts;

    this.form.append(make("p", "note", "Travel time between places is not in this version."));
    const actions = make("div", "actions");
    this.saveReason = make("p", "note");
    this.saveReason.id = uid("save-reason");
    this.save = button("Save preferences", () => p.save(), "primary");
    this.note = make("p", "problem");
    this.note.id = uid("save-note");
    this.note.hidden = true;
    describe(this.save, this.saveReason.id, this.note.id);
    actions.append(this.save, button("Discard changes", () => p.discard()), this.saveReason);
    this.form.append(actions, this.note);
  }

  render(): void {
    const f = this.prefs.form;
    const sectionHeading = byId("preferences-heading");
    textLine(this.unknown, f === null ? `The preferences are not known yet. ${this.model.notReadyText ?? ""}`.trim() : null, sectionHeading);
    show(this.form, f !== null, sectionHeading);
    if (f === null) return;
    const d = f.draft;

    show(this.stale, f.stale !== null, this.save);
    setText(this.staleText, f.stale ?? "");
    for (const [id, dom] of Object.entries(this.parts) as [PartId, PartDom][]) {
      textLine(dom.stored, f.stored[id] ?? null, dom.heading);
      const lines = f.problems.filter((x) => x.part === id && x.entry === null).map((x) => x.text);
      if (f.refusal?.part === id && f.refusal.entry === null) lines.push(f.refusal.text);
      setLines(dom.problems, lines);
    }

    setValue(this.zone, d.zone);
    setText(this.zoneButton, f.zoneButton);

    setChecked(this.workdaySet, d.workdaySet);
    show(this.workdayControls, d.workdaySet, this.workdaySet);
    this.updateRange(this.workday, d.workday);

    keyed(this.entries, d.protectedTimes.map((e, i) => ({ e, i })), (x) => x.e.key, (x) => this.createEntry(x.e.key),
      (node, x) => this.updateEntry(node, x.e, x.i, f), this.add);
    setDisabled(this.add, !f.add.enabled);
    setText(this.addReason, f.add.reason ?? "");

    setChecked(this.uninterruptedSet, d.uninterruptedSet);
    show(this.uninterruptedControls, d.uninterruptedSet, this.uninterruptedSet);
    setChecked(this.uninterruptedEnabled, d.uninterruptedEnabled);
    setValue(this.uninterruptedMinutes, d.uninterruptedMinutes);
    setValue(this.partOfDay, d.partOfDay);

    setChecked(this.bufferSet, d.bufferSet);
    show(this.bufferControls, d.bufferSet, this.bufferSet);
    setChecked(this.bufferEnabled, d.bufferEnabled);
    setValue(this.bufferMinutes, d.bufferMinutes);

    keyed(this.calendars, f.calendars, (c) => c.key, (c) => this.createCalendar(c), (node, c) => this.updateCalendar(node, c),
      this.parts.calendars.heading);
    show(this.calendarsEmpty, f.calendars.length === 0, this.parts.calendars.heading);

    setDisabled(this.save, !f.save.enabled);
    setText(this.saveReason, f.save.reason ?? "");
    textLine(this.note, f.note, this.save);
  }

  // Days, from and to. `describedBy` names the part's stored value and problems.
  private range(target: RangeTarget, parent: HTMLElement, describedBy: string[]): RangeDom {
    const days = new Map<Weekday, HTMLInputElement>();
    const set = make("fieldset");
    set.append(make("legend", "", "Days"));
    const choices = make("div", "choices");
    for (const { day, name } of DAYS) {
      const c = checkbox(name, (on) => this.prefs.setDay(target, day, on));
      describe(c.input, ...describedBy);
      days.set(day, c.input);
      choices.append(c.node);
    }
    set.append(choices);
    const start = timeInput((t) => this.prefs.setTime(target, "start", t));
    const end = timeInput((t) => this.prefs.setTime(target, "end", t));
    describe(start, ...describedBy);
    describe(end, ...describedBy);
    parent.append(set, field("From", start), field("To", end));
    return { days, start, end };
  }

  private updateRange(dom: RangeDom, r: RangeDraft): void {
    for (const [day, input] of dom.days) setChecked(input, r.days.includes(day));
    setValue(dom.start, r.start);
    setValue(dom.end, r.end);
  }

  private addEntry(): void {
    const key = this.prefs.addProtected();
    if (key === null) return;
    // The model's change has rendered the new entry; its first control takes focus.
    for (const node of this.entries.children) {
      if (node instanceof HTMLElement && node.dataset["key"] === key) this.entryNodes.get(node)?.enabled.focus();
    }
  }

  private createEntry(key: string): HTMLFieldSetElement {
    const node = make("fieldset", "item");
    node.dataset["key"] = key;
    const legend = make("legend");
    const problems = make("div", "problem");
    problems.id = uid("problems");
    problems.hidden = true;
    const on = checkbox("Turned on", (checked) => this.prefs.setEnabled(key, checked));
    describe(on.input, problems.id);
    node.append(legend, on.node);
    const range = this.range(key, node, [problems.id]);
    const removeName = make("span", "visually-hidden");
    const remove = button("Remove", () => {
      this.add.focus();
      this.prefs.removeProtected(key);
    });
    remove.append(removeName);
    const actions = make("div", "actions");
    actions.append(remove);
    node.append(actions, problems);
    this.entryNodes.set(node, { legend, enabled: on.input, range, remove, removeName, problems });
    return node;
  }

  private updateEntry(node: HTMLFieldSetElement, e: ProtectedDraft, index: number, f: PreferencesForm): void {
    const dom = this.entryNodes.get(node);
    if (dom === undefined) return;
    const name = `Protected time ${index + 1}`;
    setText(dom.legend, name);
    setText(dom.removeName, ` ${name.toLowerCase()}`);
    setChecked(dom.enabled, e.enabled);
    this.updateRange(dom.range, e.range);
    const lines = f.problems.filter((x) => x.entry === e.key).map((x) => x.text);
    if (f.refusal?.entry === e.key) lines.push(f.refusal.text);
    setLines(dom.problems, lines);
  }

  private createCalendar(c: CalendarChoice): HTMLLabelElement {
    const box = checkbox(c.label, (on) => this.prefs.setCalendar(c.ref, on));
    describe(box.input, this.parts.calendars.stored.id, this.parts.calendars.problems.id);
    const text = box.node.querySelector("span") ?? make("span");
    this.calendarNodes.set(box.node, { input: box.input, text });
    return box.node;
  }

  private updateCalendar(node: HTMLLabelElement, c: CalendarChoice): void {
    const dom = this.calendarNodes.get(node);
    if (dom === undefined) return;
    setText(dom.text, c.label);
    setChecked(dom.input, c.chosen);
  }
}
