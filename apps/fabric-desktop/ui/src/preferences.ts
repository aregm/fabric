// The Calendar preferences screen without the DOM (plan S06): one form over the broker's
// preferences, saved whole. Nothing ships with default hours (CD-Q11i): a new entry has no day
// and no times, and the save names what is missing. The page checks shapes only (contract
// `preferences_settings`, `time_range`); whether a start comes before an end is the broker's.
import type {
  CalendarRef, ParamsSavePreferences, PreferencesSettings, TimeRange, Weekday,
} from "../../../../contracts/desktop/v1/typescript/src/contract.ts";
import type { Availability } from "./groups.ts";
import type { Model } from "./model.ts";
import { isLocalTime, isZone } from "./shape.ts";
import { calendarName } from "./text.ts";

export const MAX_PROTECTED = 50;
export const MAX_CALENDARS = 200;
export const UNINTERRUPTED_MAX = 1440;
export const BUFFER_MAX = 240;

// The contract's weekdays, in its order, with their names.
export const DAYS: readonly { readonly day: Weekday; readonly name: string }[] = [
  { day: "monday", name: "Monday" }, { day: "tuesday", name: "Tuesday" }, { day: "wednesday", name: "Wednesday" },
  { day: "thursday", name: "Thursday" }, { day: "friday", name: "Friday" }, { day: "saturday", name: "Saturday" },
  { day: "sunday", name: "Sunday" },
];

export type PartOfDay = "any" | "morning" | "afternoon";
export const PART_OF_DAY: Readonly<Record<PartOfDay, string>> = { any: "Any time of day", morning: "Morning", afternoon: "Afternoon" };

export type PartId = "zone" | "workday" | "protected" | "uninterrupted" | "buffer" | "calendars";
export const PARTS: readonly PartId[] = ["zone", "workday", "protected", "uninterrupted", "buffer", "calendars"];

// A problem of the form, tied to its part (and, for a protected time, to its entry).
export interface Problem {
  readonly part: PartId;
  readonly entry: string | null; // the key of a protected time
  readonly text: string;
}

export interface RangeDraft {
  readonly days: readonly Weekday[]; // in the contract's order
  readonly start: string;
  readonly end: string;
}

export interface ProtectedDraft {
  readonly key: string; // stable while the entry is on the page
  readonly enabled: boolean;
  readonly range: RangeDraft;
}

// The form as the owner fills it: text as typed, switches as set.
export interface PreferencesDraft {
  readonly zone: string;
  readonly workdaySet: boolean;
  readonly workday: RangeDraft;
  readonly protectedTimes: readonly ProtectedDraft[];
  readonly uninterruptedSet: boolean;
  readonly uninterruptedEnabled: boolean;
  readonly uninterruptedMinutes: string;
  readonly partOfDay: PartOfDay;
  readonly bufferSet: boolean;
  readonly bufferEnabled: boolean;
  readonly bufferMinutes: string;
  readonly calendars: readonly CalendarRef[];
}

export interface CalendarChoice {
  readonly key: string;
  readonly ref: CalendarRef;
  readonly label: string;
  readonly chosen: boolean;
  readonly known: boolean; // false: chosen before, and no account lists it any more
}

export interface PreferencesForm {
  readonly draft: PreferencesDraft;
  readonly problems: readonly Problem[]; // empty while the form has no changes
  readonly dirty: boolean;
  readonly stale: string | null;
  readonly stored: Readonly<Partial<Record<PartId, string>>>; // while stale: the stored value of each part that differs
  readonly refusal: Problem | null; // the broker's refusal of a part, from its pointer
  readonly note: string | null; // the result of the last save that did not succeed
  readonly save: Availability;
  readonly discard: Availability;
  readonly add: Availability;
  readonly calendars: readonly CalendarChoice[];
  readonly zoneButton: string;
}

export type RangeTarget = "workday" | string; // "workday" or the key of a protected time

interface Range { days: Weekday[]; start: string; end: string }
interface Draft {
  zone: string;
  workdaySet: boolean;
  workday: Range;
  protectedTimes: { key: string; enabled: boolean; range: Range }[];
  uninterruptedSet: boolean;
  uninterruptedEnabled: boolean;
  uninterruptedMinutes: string;
  partOfDay: PartOfDay;
  bufferSet: boolean;
  bufferEnabled: boolean;
  bufferMinutes: string;
  calendars: CalendarRef[];
}

const EMPTY_RANGE = (): Range => ({ days: [], start: "", end: "" });
const refKey = (r: CalendarRef): string => `${r.account_id}\n${r.calendar_id}`;
const ordered = (days: readonly Weekday[]): Weekday[] => DAYS.map((d) => d.day).filter((d) => days.includes(d));
const dayNames = (days: readonly Weekday[]): string => ordered(days).map((d) => DAYS.find((x) => x.day === d)?.name ?? d).join(", ");

// JSON with the keys of every object sorted, so that two equal values give the same text.
function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (value !== null && typeof value === "object") {
    const o = value as Record<string, unknown>;
    return `{${Object.keys(o).sort().map((k) => `${JSON.stringify(k)}:${canonical(o[k])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

// The stored settings with the days of every range in the contract's order: days are a set.
function normal(s: PreferencesSettings): PreferencesSettings {
  const range = (r: TimeRange): TimeRange => ({ ...r, days: ordered(r.days) });
  return {
    ...s,
    workday: s.workday === null ? null : range(s.workday),
    protected_times: s.protected_times.map((p) => ({ ...p, range: range(p.range) })),
  };
}

function settingsPart(s: PreferencesSettings, part: PartId): unknown {
  switch (part) {
    case "zone": return s.zone;
    case "workday": return s.workday;
    case "protected": return s.protected_times;
    case "uninterrupted": return s.uninterrupted;
    case "buffer": return s.buffer;
    case "calendars": return s.warning_calendars;
  }
}

function minutes(text: string, max: number): number | null {
  const t = text.trim();
  if (!/^[0-9]{1,5}$/.test(t)) return null;
  const n = Number(t);
  return n >= 1 && n <= max ? n : null;
}

function rangeText(r: TimeRange): string {
  return `${dayNames(r.days)}, from ${r.start} to ${r.end}`;
}

const onOff = (enabled: boolean): string => (enabled ? "on" : "off");

export class PreferencesModel {
  private readonly model: Model;
  private draft: Draft | null = null;
  private baseRevision = 0;
  private base: PreferencesSettings | null = null; // the stored settings the draft was made from
  private edits = 0;
  private keys = 0;
  private inFlight = false;
  private note: string | null = null;
  private refusal: Problem | null = null;
  private said: number | null = null; // the stored revision whose staleness was last announced

  constructor(model: Model) {
    this.model = model;
  }

  // ---- What the view shows ----

  // Null until the broker's preferences are known.
  get form(): PreferencesForm | null {
    this.follow();
    const d = this.draft;
    const base = this.base;
    const current = this.model.preferences;
    if (d === null || base === null || current === null) return null;
    const built = this.build();
    const dirty = PARTS.some((p) => this.partDiffers(built, base, p));
    // While the owner's own save is awaited, its notice (which the broker sends before the answer)
    // is not a change made by someone else: no staleness is reported until the save ends.
    const stale = !this.inFlight && dirty && current.revision !== this.baseRevision
      ? "The preferences were changed while you were editing them. Your changes are kept. The stored version is shown in each part that differs."
      : null;
    const stored: Partial<Record<PartId, string>> = {};
    if (stale !== null) {
      for (const p of PARTS) if (this.partDiffers(built, current.settings, p)) stored[p] = `Stored now: ${this.partText(current.settings, p)}`;
    }
    const problems = dirty ? built.problems : [];
    const reason = !this.model.ready ? this.model.notReadyText
      : this.inFlight ? "Saving…"
      : stale !== null ? "Look at the stored version, then press \"I have seen the current version\"."
      : !dirty ? "Nothing has changed."
      : problems[0]?.text ?? null;
    const full = d.protectedTimes.length >= MAX_PROTECTED;
    return {
      draft: d,
      problems,
      dirty,
      stale,
      stored,
      refusal: this.refusal,
      note: this.note,
      save: { enabled: reason === null, reason },
      discard: { enabled: dirty, reason: dirty ? null : "There are no changes to discard." },
      add: { enabled: !full, reason: full ? `At most ${MAX_PROTECTED} protected times can be kept.` : null },
      calendars: this.calendarChoices(d),
      zoneButton: `Use this computer's zone (${this.model.zone})`,
    };
  }

  // Says once that the form became stale. The view calls it while it shows the stale notice;
  // the same stored revision is not said twice.
  sayStale(): void {
    const current = this.model.preferences;
    if (current === null || this.form?.stale == null || this.said === current.revision) return;
    this.said = current.revision;
    this.model.announce("Your calendar preferences changed while you were editing. Your changes are kept. Look at the current version before you save.");
  }

  // The settings the draft would save, or what stops it, each problem tied to its part.
  settings(): { readonly settings: PreferencesSettings } | { readonly problems: readonly Problem[] } {
    const b = this.build();
    return b.problems.length > 0 || b.settings === null ? { problems: b.problems } : { settings: b.settings };
  }

  // ---- What the owner does ----

  setZone(text: string): void {
    this.change((d) => void (d.zone = text));
  }

  useComputerZone(): void {
    this.setZone(this.model.zone);
  }

  // Working hours, uninterrupted time and time between events: off means not set.
  setOn(part: "workday" | "uninterrupted" | "buffer", on: boolean): void {
    this.change((d) => {
      if (part === "workday") d.workdaySet = on;
      else if (part === "uninterrupted") d.uninterruptedSet = on;
      else d.bufferSet = on;
    });
  }

  // Whether a set entry gives warnings now.
  setEnabled(target: "uninterrupted" | "buffer" | string, enabled: boolean): void {
    this.change((d) => {
      if (target === "uninterrupted") d.uninterruptedEnabled = enabled;
      else if (target === "buffer") d.bufferEnabled = enabled;
      else this.entry(d, target).forEach((p) => void (p.enabled = enabled));
    });
  }

  setDay(target: RangeTarget, day: Weekday, chosen: boolean): void {
    this.change((d) => {
      for (const r of this.ranges(d, target)) r.days = ordered(chosen ? [...r.days, day] : r.days.filter((x) => x !== day));
    });
  }

  setTime(target: RangeTarget, which: "start" | "end", text: string): void {
    this.change((d) => {
      for (const r of this.ranges(d, target)) r[which] = text;
    });
  }

  setMinutes(part: "uninterrupted" | "buffer", text: string): void {
    this.change((d) => {
      if (part === "uninterrupted") d.uninterruptedMinutes = text;
      else d.bufferMinutes = text;
    });
  }

  setPartOfDay(value: PartOfDay): void {
    this.change((d) => void (d.partOfDay = value));
  }

  // A new protected time: on, with no day and no times. Returns its key, or null when full.
  addProtected(): string | null {
    this.follow();
    const d = this.draft;
    if (d === null || d.protectedTimes.length >= MAX_PROTECTED) return null;
    const key = this.newKey();
    this.change((x) => void x.protectedTimes.push({ key, enabled: true, range: EMPTY_RANGE() }));
    return key;
  }

  removeProtected(key: string): void {
    this.change((d) => void (d.protectedTimes = d.protectedTimes.filter((p) => p.key !== key)));
  }

  setCalendar(ref: CalendarRef, chosen: boolean): void {
    this.change((d) => {
      const others = d.calendars.filter((c) => refKey(c) !== refKey(ref));
      d.calendars = chosen ? [...others, { account_id: ref.account_id, calendar_id: ref.calendar_id }] : others;
    });
  }

  // The owner saw the stored version: the draft is now written against it.
  acknowledge(): void {
    const current = this.model.preferences;
    if (this.draft === null || current === null) return;
    this.baseRevision = current.revision;
    this.base = current.settings;
    this.note = null;
    this.model.changed();
  }

  discard(): void {
    const current = this.model.preferences;
    if (current === null || this.form?.discard.enabled !== true) return;
    this.load(current.revision, current.settings);
    this.model.changed();
  }

  save(): boolean {
    const form = this.form;
    const built = this.build();
    if (form === null || !form.save.enabled || built.settings === null) return false;
    const params: ParamsSavePreferences = { expected_revision: this.baseRevision, settings: built.settings };
    const edits = this.edits;
    this.inFlight = true;
    this.note = null;
    this.refusal = null;
    this.model.command("save_preferences", params, (result) => {
      this.inFlight = false;
      const p = result.preferences;
      if (this.edits === edits) this.load(p.revision, p.settings);
      else {
        // The owner changed the form while the save was on its way: the newer values stay, written against the saved version.
        this.baseRevision = p.revision;
        this.base = p.settings;
      }
      this.model.announce("Your calendar preferences are saved.");
    }, (f) => {
      this.inFlight = false;
      if (f.code === null) this.note = "It is not known whether this was saved. Saving the same again is safe.";
      else if (f.code === "conflict") {
        this.note = "Nothing was saved: the preferences changed since you opened them. The current version is read again.";
        this.model.refresh();
      } else {
        this.note = `The broker did not save this: ${f.text}`;
        if (f.error?.code === "invalid_params") this.refusal = this.refusalAt(f.error.detail.pointer, f.text, params.settings);
      }
      this.model.announce(this.note);
    });
    this.model.changed();
    return true;
  }

  // ---- Inside ----

  private change(edit: (d: Draft) => void): void {
    this.follow();
    const d = this.draft;
    if (d === null) return;
    edit(d);
    this.edits += 1;
    this.model.changed();
  }

  private newKey(): string {
    this.keys += 1;
    return `protected-${this.keys}`;
  }

  private entry(d: Draft, key: string): Draft["protectedTimes"] {
    return d.protectedTimes.filter((p) => p.key === key);
  }

  private ranges(d: Draft, target: RangeTarget): Range[] {
    return target === "workday" ? [d.workday] : this.entry(d, target).map((p) => p.range);
  }

  // The form made from stored settings. Protected times keep the keys of the entries they replace,
  // so that the page keeps its nodes.
  private load(revision: number, s: PreferencesSettings): void {
    const keys = this.draft?.protectedTimes.map((p) => p.key) ?? [];
    const range = (r: TimeRange): Range => ({ days: ordered(r.days), start: r.start, end: r.end });
    this.draft = {
      zone: s.zone ?? "",
      workdaySet: s.workday !== null,
      workday: s.workday === null ? EMPTY_RANGE() : range(s.workday),
      protectedTimes: s.protected_times.map((p, i) => ({ key: keys[i] ?? this.newKey(), enabled: p.enabled, range: range(p.range) })),
      uninterruptedSet: s.uninterrupted !== null,
      uninterruptedEnabled: s.uninterrupted?.enabled ?? true,
      uninterruptedMinutes: s.uninterrupted === null ? "" : String(s.uninterrupted.minimum_minutes),
      partOfDay: s.uninterrupted?.part_of_day ?? "any",
      bufferSet: s.buffer !== null,
      bufferEnabled: s.buffer?.enabled ?? true,
      bufferMinutes: s.buffer === null ? "" : String(s.buffer.minutes),
      calendars: [...s.warning_calendars],
    };
    this.baseRevision = revision;
    this.base = s;
    this.note = null;
    this.refusal = null;
  }

  // A form without changes follows the stored preferences; one with changes is kept.
  private follow(): void {
    const current = this.model.preferences;
    if (current === null || this.inFlight) return;
    if (this.draft === null || this.base === null) return this.load(current.revision, current.settings);
    if (current.revision === this.baseRevision) return;
    const built = this.build();
    const base = this.base;
    if (!PARTS.some((p) => this.partDiffers(built, base, p))) this.load(current.revision, current.settings);
  }

  // Every part of the draft as settings, and the problems of those that have the wrong shape.
  private build(): { readonly settings: PreferencesSettings | null; readonly parts: ReadonlySet<PartId>; readonly problems: Problem[] } {
    const d = this.draft;
    const problems: Problem[] = [];
    if (d === null) return { settings: null, parts: new Set(), problems };
    const add = (part: PartId, text: string, entry: string | null = null): void => void problems.push({ part, entry, text });
    const range = (r: RangeDraft, part: PartId, label: string, entry: string | null): TimeRange => {
      if (r.days.length === 0) add(part, `${label}: choose at least one day.`, entry);
      for (const which of ["start", "end"] as const) {
        const t = r[which];
        const when = which === "start" ? "starts" : "ends";
        if (t === "") add(part, `${label}: give the time it ${when}.`, entry);
        else if (!isLocalTime(t)) add(part, `${label}: ${t} is not a time of day like 09:30.`, entry);
      }
      return { days: ordered(r.days), start: r.start, end: r.end };
    };
    const zone = d.zone.trim();
    if (zone !== "" && !isZone(zone)) add("zone", `Time zone: ${zone} is not the name of a time zone, like America/Chicago.`);
    const workday = d.workdaySet ? range(d.workday, "workday", "Working hours", null) : null;
    const protectedTimes = d.protectedTimes.map((p, i) => ({ enabled: p.enabled, range: range(p.range, "protected", `Protected time ${i + 1}`, p.key) }));
    if (d.protectedTimes.length > MAX_PROTECTED) add("protected", `At most ${MAX_PROTECTED} protected times can be kept.`);
    let uninterrupted: PreferencesSettings["uninterrupted"] = null;
    if (d.uninterruptedSet) {
      const n = minutes(d.uninterruptedMinutes, UNINTERRUPTED_MAX);
      if (n === null) add("uninterrupted", `Uninterrupted time: give the minutes as a whole number from 1 to ${UNINTERRUPTED_MAX}.`);
      uninterrupted = { enabled: d.uninterruptedEnabled, minimum_minutes: n ?? 0, part_of_day: d.partOfDay };
    }
    let buffer: PreferencesSettings["buffer"] = null;
    if (d.bufferSet) {
      const n = minutes(d.bufferMinutes, BUFFER_MAX);
      if (n === null) add("buffer", `Time between events: give the minutes as a whole number from 1 to ${BUFFER_MAX}.`);
      buffer = { enabled: d.bufferEnabled, minutes: n ?? 0 };
    }
    if (d.calendars.length > MAX_CALENDARS) add("calendars", `At most ${MAX_CALENDARS} calendars can be chosen for warnings.`);
    const settings: PreferencesSettings = {
      zone: zone === "" ? null : zone, workday, protected_times: protectedTimes, uninterrupted, buffer,
      warning_calendars: d.calendars.map((c) => ({ account_id: c.account_id, calendar_id: c.calendar_id })),
    };
    return { settings, parts: new Set(problems.map((p) => p.part)), problems };
  }

  private partDiffers(built: ReturnType<PreferencesModel["build"]>, stored: PreferencesSettings, part: PartId): boolean {
    if (built.settings === null || built.parts.has(part)) return true;
    return canonical(settingsPart(built.settings, part)) !== canonical(settingsPart(normal(stored), part));
  }

  private partText(s: PreferencesSettings, part: PartId): string {
    switch (part) {
      case "zone": return s.zone ?? "not set";
      case "workday": return s.workday === null ? "not set" : rangeText(s.workday);
      case "protected": return s.protected_times.length === 0 ? "none"
        : s.protected_times.map((p, i) => `${i + 1}: ${onOff(p.enabled)}, ${rangeText(p.range)}`).join("; ");
      case "uninterrupted": return s.uninterrupted === null ? "not set"
        : `${onOff(s.uninterrupted.enabled)}, at least ${s.uninterrupted.minimum_minutes} minutes, ${PART_OF_DAY[s.uninterrupted.part_of_day].toLowerCase()}`;
      case "buffer": return s.buffer === null ? "not set" : `${onOff(s.buffer.enabled)}, ${s.buffer.minutes} minutes`;
      case "calendars": return s.warning_calendars.length === 0 ? "none" : s.warning_calendars.map((c) => this.calendarLabel(c)).join("; ");
    }
  }

  private calendarLabel(ref: CalendarRef): string {
    const known = this.model.accounts.get(ref.account_id)?.calendars.some((c) => c.calendar_id === ref.calendar_id) === true;
    return known ? calendarName(this.model.names, ref) : `A calendar the broker no longer lists (${ref.calendar_id})`;
  }

  // Every calendar of the accounts, then each chosen one that no account lists any more.
  private calendarChoices(d: Draft): CalendarChoice[] {
    const chosen = new Set(d.calendars.map(refKey));
    const listed: CalendarChoice[] = [...this.model.accounts.values()].flatMap((a) => a.calendars.map((c) => {
      const ref = { account_id: a.account_id, calendar_id: c.calendar_id };
      return { key: refKey(ref), ref, label: calendarName(this.model.names, ref), chosen: chosen.has(refKey(ref)), known: true };
    }));
    const keys = new Set(listed.map((c) => c.key));
    const gone = d.calendars.filter((c) => !keys.has(refKey(c)))
      .map((ref) => ({ key: refKey(ref), ref, label: this.calendarLabel(ref), chosen: true, known: false }));
    return [...listed, ...gone];
  }

  // The part a refusal's pointer names (`/params/settings/<member>/…`), with words for it.
  private refusalAt(pointer: string, message: string, sent: PreferencesSettings): Problem | null {
    const [member, index] = pointer.replace(/^\/params\/settings\//, "").split("/");
    const i = Number(index);
    switch (member) {
      case "zone": return { part: "zone", entry: null, text: `The broker refused the time zone: ${message}` };
      case "workday": return { part: "workday", entry: null, text: `The broker refused the working hours: ${message}` };
      case "uninterrupted": return { part: "uninterrupted", entry: null, text: `The broker refused the uninterrupted time: ${message}` };
      case "buffer": return { part: "buffer", entry: null, text: `The broker refused the time between events: ${message}` };
      case "protected_times": {
        const key = this.draft?.protectedTimes[i]?.key ?? null;
        return { part: "protected", entry: key, text: `The broker refused protected time ${Number.isInteger(i) ? i + 1 : ""}: ${message}`.replace(" :", ":") };
      }
      case "warning_calendars": {
        const ref = sent.warning_calendars[i];
        return { part: "calendars", entry: null, text: ref === undefined ? `The broker refused a calendar: ${message}` : `The broker refused ${this.calendarLabel(ref)}: ${message}` };
      }
      default: return null;
    }
  }
}
