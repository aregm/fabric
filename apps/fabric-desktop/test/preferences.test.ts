// The Calendar preferences screen's model against the fake host: no browser, no DOM. Every frame
// is judged by the contract's checker; broker answers come from the contract's fixtures.
import assert from "node:assert/strict";
import { test } from "node:test";
import { WEEKDAYS } from "../../../contracts/desktop/v1/typescript/src/contract.ts";
import type { OwnerError, Preferences, Snapshot } from "../../../contracts/desktop/v1/typescript/src/contract.ts";
import { DAYS, PreferencesModel } from "../ui/src/preferences.ts";
import { assertRequestsValid, connect, fixture, setup, snapshot } from "./support.ts";

const RICH: Snapshot = fixture("accept-owner-response-get-snapshot-rich.json").result;
const CONFIGURED: Preferences = fixture("accept-owner-notice-preferences.json").change.preferences; // revision 3
const CONFLICT = fixture("accept-owner-response-error-conflict.json").error;
const NOT_KNOWN = "It is not known whether this was saved. Saving the same again is safe.";

function screen(snap: Snapshot = RICH) {
  const s = setup();
  connect(s.host, 1, snap);
  return { ...s, prefs: new PreferencesModel(s.model), saves: () => s.host.sent.filter((f) => f.method === "save_preferences") };
}

// The rich snapshot with these preferences.
const withPreferences = (preferences: Preferences): Snapshot => ({ ...RICH, preferences });

const form = (p: PreferencesModel) => {
  const f = p.form;
  assert.ok(f !== null, "the form is shown");
  return f;
};
const texts = (p: PreferencesModel) => form(p).problems.map((x) => x.text);
const settingsOf = (p: PreferencesModel) => {
  const r = p.settings();
  assert.ok("settings" in r, `the form has the right shape: ${JSON.stringify(r)}`);
  return r.settings;
};

test("the days are the contract's weekdays, in its order", () => {
  assert.deepEqual(DAYS.map((d) => d.day), [...WEEKDAYS]);
});

test("first run: an empty form that cannot be saved unchanged", () => {
  const { prefs, saves } = screen(snapshot(0));
  const f = form(prefs);
  assert.deepEqual(f.draft, {
    zone: "", workdaySet: false, workday: { days: [], start: "", end: "" }, protectedTimes: [],
    uninterruptedSet: false, uninterruptedEnabled: true, uninterruptedMinutes: "", partOfDay: "any",
    bufferSet: false, bufferEnabled: true, bufferMinutes: "", calendars: [],
  });
  assert.equal(f.dirty, false);
  assert.deepEqual(f.save, { enabled: false, reason: "Nothing has changed." });
  assert.deepEqual(f.calendars, []);
  assert.deepEqual(settingsOf(prefs), fixture("accept-owner-request-save-preferences-not-configured.json").params.settings);
  assert.equal(prefs.save(), false);
  assert.equal(saves().length, 0);
});

test("nothing has default hours: working hours switched on and a new protected time have no day and no times", () => {
  const { prefs, saves } = screen();
  prefs.setOn("workday", true);
  assert.deepEqual(form(prefs).draft.workday, { days: [], start: "", end: "" });
  const key = prefs.addProtected();
  assert.ok(key !== null);
  assert.deepEqual(form(prefs).draft.protectedTimes, [{ key, enabled: true, range: { days: [], start: "", end: "" } }]);
  assert.deepEqual(texts(prefs), [
    "Working hours: choose at least one day.", "Working hours: give the time it starts.", "Working hours: give the time it ends.",
    "Protected time 1: choose at least one day.", "Protected time 1: give the time it starts.", "Protected time 1: give the time it ends.",
  ]);
  assert.deepEqual(form(prefs).problems.slice(3).map((x) => [x.part, x.entry]), [["protected", key], ["protected", key], ["protected", key]]);
  assert.equal(form(prefs).save.reason, "Working hours: choose at least one day.");
  assert.equal(prefs.save(), false);
  assert.equal(saves().length, 0);
});

test("each part's shape problems stop the save and are named at their part", () => {
  const { prefs, saves } = screen();
  prefs.setZone("not a zone!");
  prefs.setOn("uninterrupted", true);
  prefs.setMinutes("uninterrupted", "0");
  prefs.setOn("buffer", true);
  prefs.setMinutes("buffer", "241");
  assert.deepEqual(form(prefs).problems, [
    { part: "zone", entry: null, text: "Time zone: not a zone! is not the name of a time zone, like America/Chicago." },
    { part: "uninterrupted", entry: null, text: "Uninterrupted time: give the minutes as a whole number from 1 to 1440." },
    { part: "buffer", entry: null, text: "Time between events: give the minutes as a whole number from 1 to 240." },
  ]);
  assert.equal(prefs.save(), false);
  prefs.setZone("Europe/Berlin");
  prefs.setMinutes("uninterrupted", "1441");
  prefs.setMinutes("buffer", "2.5");
  assert.equal(texts(prefs).length, 2);
  prefs.setMinutes("uninterrupted", "1440");
  prefs.setMinutes("buffer", "1");
  const key = prefs.addProtected() ?? "";
  prefs.setDay(key, "monday", true);
  prefs.setTime(key, "start", "25:00");
  prefs.setTime(key, "end", "10:00");
  assert.deepEqual(texts(prefs), ["Protected time 1: 25:00 is not a time of day like 09:30."]);
  prefs.setTime(key, "start", "09:00");
  assert.deepEqual(texts(prefs), []);
  assert.equal(form(prefs).save.enabled, true);
  assert.equal(prefs.save(), true);
  assert.equal(saves().length, 1);
});

test("a full form builds exactly the expected settings", () => {
  const { host, prefs, announced } = screen();
  prefs.useComputerZone();
  assert.equal(form(prefs).zoneButton, "Use this computer's zone (America/Chicago)");
  prefs.setOn("workday", true);
  for (const day of ["friday", "monday", "tuesday", "wednesday", "thursday"] as const) prefs.setDay("workday", day, true);
  prefs.setTime("workday", "start", "09:00");
  prefs.setTime("workday", "end", "17:30");
  const first = prefs.addProtected() ?? "";
  for (const day of ["monday", "wednesday", "friday"] as const) prefs.setDay(first, day, true);
  prefs.setTime(first, "start", "12:00");
  prefs.setTime(first, "end", "13:00");
  const second = prefs.addProtected() ?? "";
  prefs.setEnabled(second, false);
  prefs.setDay(second, "saturday", true);
  prefs.setDay(second, "sunday", true);
  prefs.setTime(second, "start", "00:00");
  prefs.setTime(second, "end", "23:59");
  prefs.setOn("uninterrupted", true);
  prefs.setMinutes("uninterrupted", "120");
  prefs.setPartOfDay("morning");
  prefs.setOn("buffer", true);
  prefs.setMinutes("buffer", " 10 ");
  prefs.setCalendar({ account_id: "acct-1", calendar_id: "cal-primary" }, true);
  prefs.setCalendar({ account_id: "acct-2", calendar_id: "cal-work-primary" }, true);
  const request = fixture("accept-owner-request-save-preferences-configured.json").params;
  assert.deepEqual(settingsOf(prefs), request.settings);
  assert.equal(prefs.save(), true);
  assert.deepEqual(host.last("save_preferences").params, request);
  assert.equal(form(prefs).save.reason, "Saving…");
  const result = fixture("accept-owner-response-save-preferences.json").result;
  host.notice(RICH.cursor + 1, { kind: "preferences", preferences: result.preferences });
  host.reply("save_preferences", result);
  assert.equal(announced.at(-1), "Your calendar preferences are saved.");
  assert.equal(form(prefs).dirty, false);
  assert.deepEqual(form(prefs).draft.protectedTimes.map((p) => p.key), [first, second], "the entries keep their keys");
});

test("switching a part off sends null", () => {
  const { host, prefs } = screen(withPreferences(CONFIGURED));
  assert.equal(form(prefs).dirty, false);
  assert.equal(form(prefs).draft.workdaySet, true);
  prefs.setOn("workday", false);
  prefs.setOn("uninterrupted", false);
  prefs.setOn("buffer", false);
  prefs.setZone("");
  const s = settingsOf(prefs);
  assert.equal(s.zone, null);
  assert.equal(s.workday, null);
  assert.equal(s.uninterrupted, null);
  assert.equal(s.buffer, null);
  assert.equal(s.protected_times.length, 2);
  prefs.save();
  assert.equal(host.last("save_preferences").params.expected_revision, 3);
  assert.equal(host.last("save_preferences").params.settings.workday, null);
});

test("switching a part off and on again, or reordering days, is no change", () => {
  const { prefs } = screen(withPreferences({
    ...CONFIGURED,
    settings: { ...CONFIGURED.settings, workday: { days: ["friday", "monday"], start: "09:00", end: "17:00" } },
  }));
  assert.equal(form(prefs).dirty, false, "the stored order of days does not matter");
  prefs.setOn("buffer", false);
  assert.equal(form(prefs).dirty, true);
  prefs.setOn("buffer", true);
  assert.equal(form(prefs).dirty, false);
  prefs.setDay("workday", "monday", false);
  prefs.setDay("workday", "monday", true);
  assert.equal(form(prefs).dirty, false);
});

test("the calendars offered are those of the accounts, and an unknown stored one is kept until removed", () => {
  const gone = { account_id: "acct-1", calendar_id: "cal-gone" };
  const { prefs } = screen(withPreferences({
    revision: 2, settings: { ...CONFIGURED.settings, warning_calendars: [{ account_id: "acct-1", calendar_id: "cal-primary" }, gone] },
  }));
  const choices = form(prefs).calendars;
  assert.deepEqual(choices.map((c) => [c.label, c.chosen, c.known]), [
    ["Owner (owner@example.com)", true, true],
    ["Agent Drafts (owner@example.com)", false, true],
    ["Team schedule (owner@example.com)", false, true],
    ["Work (owner.work@example.org)", false, true],
    ["Family (owner.work@example.org)", false, true],
    ["Holidays (owner.work@example.org)", false, true],
    ["A calendar the broker no longer lists (cal-gone)", true, false],
  ]);
  assert.deepEqual(settingsOf(prefs).warning_calendars, [{ account_id: "acct-1", calendar_id: "cal-primary" }, gone]);
  assert.equal(form(prefs).dirty, false);
  prefs.setCalendar(gone, false);
  assert.equal(form(prefs).calendars.some((c) => !c.known), false);
  assert.deepEqual(settingsOf(prefs).warning_calendars, [{ account_id: "acct-1", calendar_id: "cal-primary" }]);
});

test("the broker's refusal of a warning calendar is shown at the calendars", () => {
  const gone = { account_id: "acct-1", calendar_id: "cal-gone" };
  const { host, prefs } = screen(withPreferences({
    revision: 2, settings: { ...CONFIGURED.settings, warning_calendars: [{ account_id: "acct-1", calendar_id: "cal-primary" }, gone] },
  }));
  prefs.setMinutes("buffer", "15");
  prefs.save();
  const refusal: OwnerError = {
    ...fixture("accept-owner-response-error-invalid-params.json").error,
    message: "A warning calendar names no calendar the broker knows.",
    detail: { pointer: "/params/settings/warning_calendars/1" },
  };
  host.fail("save_preferences", refusal);
  assert.deepEqual(form(prefs).refusal, {
    part: "calendars", entry: null,
    text: "The broker refused A calendar the broker no longer lists (cal-gone): A warning calendar names no calendar the broker knows.",
  });
  assert.equal(form(prefs).note, "The broker did not save this: A warning calendar names no calendar the broker knows.");
  assert.equal(form(prefs).draft.bufferMinutes, "15", "the draft stays");
});

test("conflict: nothing saved, the state is read again, stale until the owner has seen it, then the save carries the new revision", () => {
  const { host, prefs, saves } = screen();
  prefs.setZone("Europe/Berlin");
  prefs.save();
  assert.equal(host.last("save_preferences").params.expected_revision, 0);
  host.fail("save_preferences", CONFLICT);
  assert.equal(form(prefs).note, "Nothing was saved: the preferences changed since you opened them. The current version is read again.");
  assert.deepEqual(host.methods().slice(-1), ["get_snapshot"]);
  host.reply("get_snapshot", withPreferences(CONFIGURED));
  host.reply("subscribe", { cursor: RICH.cursor });
  const f = form(prefs);
  assert.match(f.stale ?? "", /^The preferences were changed while you were editing them/);
  assert.equal(f.draft.zone, "Europe/Berlin", "the owner's edits stay");
  assert.equal(f.stored.zone, "Stored now: America/Chicago");
  assert.equal(f.stored.workday, "Stored now: Monday, Tuesday, Wednesday, Thursday, Friday, from 09:00 to 17:30");
  assert.equal(f.stored.buffer, "Stored now: on, 10 minutes");
  assert.equal(f.save.enabled, false);
  assert.equal(prefs.save(), false);
  prefs.acknowledge();
  assert.equal(form(prefs).stale, null);
  assert.equal(prefs.save(), true);
  assert.equal(saves().length, 2);
  assert.equal(host.last("save_preferences").params.expected_revision, 3);
});

test("a preferences notice: a form with changes becomes stale, one without follows", () => {
  const { host, prefs } = screen();
  host.notice(RICH.cursor + 1, { kind: "preferences", preferences: CONFIGURED });
  assert.equal(form(prefs).draft.zone, "America/Chicago");
  assert.equal(form(prefs).stale, null);
  prefs.setMinutes("buffer", "20");
  host.notice(RICH.cursor + 2, { kind: "preferences", preferences: { revision: 4, settings: { ...CONFIGURED.settings, zone: "Europe/Paris" } } });
  const f = form(prefs);
  assert.notEqual(f.stale, null);
  assert.equal(f.draft.zone, "America/Chicago");
  assert.equal(f.stored.zone, "Stored now: Europe/Paris");
  assert.equal(f.stored.buffer, "Stored now: on, 10 minutes");
  prefs.discard();
  assert.equal(form(prefs).draft.zone, "Europe/Paris");
  assert.equal(form(prefs).dirty, false);
});

test("no answer: the draft stays and the same save reuses its command id", () => {
  const { host, prefs, saves } = screen();
  prefs.setZone("Europe/Berlin");
  prefs.save();
  const first = host.last("save_preferences").id;
  host.state({ name: "disconnected", reason: "exited", detail: "exit code 1" });
  assert.equal(form(prefs).note, NOT_KNOWN);
  connect(host, 2, RICH);
  assert.equal(form(prefs).draft.zone, "Europe/Berlin");
  assert.equal(prefs.save(), true);
  assert.equal(saves().length, 2);
  assert.equal(host.last("save_preferences").id, first);
});

test("nothing is sent while the broker is not ready, and the draft survives a disconnect and a restart", () => {
  const { host, model, prefs, saves } = screen();
  prefs.setOn("buffer", true);
  prefs.setMinutes("buffer", "5");
  host.state({ name: "disconnected", reason: "exited", detail: "exit code 1" });
  assert.deepEqual(form(prefs).save, { enabled: false, reason: model.notReadyText });
  assert.equal(prefs.save(), false);
  prefs.setEnabled("buffer", false);
  connect(host, 2, RICH);
  assert.equal(form(prefs).draft.bufferMinutes, "5");
  assert.equal(form(prefs).draft.bufferEnabled, false);
  assert.equal(saves().length, 0);
  assert.equal(prefs.save(), true);

  const before = setup();
  assert.equal(new PreferencesModel(before.model).form, null, "nothing to show before the broker's preferences are known");
});

test("protected times: at most 50, and removing one keeps the others' keys", () => {
  const { prefs } = screen();
  const keys = Array.from({ length: 50 }, () => prefs.addProtected());
  assert.equal(keys.includes(null), false);
  assert.deepEqual(form(prefs).add, { enabled: false, reason: "At most 50 protected times can be kept." });
  assert.equal(prefs.addProtected(), null);
  prefs.removeProtected(keys[0] ?? "");
  assert.deepEqual(form(prefs).draft.protectedTimes.map((p) => p.key), keys.slice(1));
  assert.equal(form(prefs).add.enabled, true);
  assert.match(texts(prefs)[0] ?? "", /^Protected time 1: /);
});

test("every command of the screen sends a valid owner request", () => {
  const { host, prefs } = screen();
  prefs.setZone("America/Chicago");
  prefs.setOn("workday", true);
  prefs.setDay("workday", "monday", true);
  prefs.setTime("workday", "start", "08:00");
  prefs.setTime("workday", "end", "16:00");
  const key = prefs.addProtected() ?? "";
  prefs.setDay(key, "sunday", true);
  prefs.setTime(key, "start", "00:00");
  prefs.setTime(key, "end", "23:59");
  prefs.setOn("uninterrupted", true);
  prefs.setMinutes("uninterrupted", "90");
  prefs.setPartOfDay("afternoon");
  prefs.setOn("buffer", true);
  prefs.setMinutes("buffer", "5");
  prefs.setCalendar({ account_id: "acct-2", calendar_id: "cal-holidays" }, true);
  prefs.save();
  host.error("save_preferences", "internal");
  assert.equal(form(prefs).note, "The broker did not save this: Refused by the test.");
  prefs.save();
  const saved = { revision: 1, settings: settingsOf(prefs) };
  host.notice(RICH.cursor + 1, { kind: "preferences", preferences: saved });
  host.reply("save_preferences", { commit_sequence: RICH.cursor + 1, preferences: saved });
  prefs.setOn("workday", false);
  prefs.save();
  assert.deepEqual(host.sent.filter((f) => f.method === "save_preferences").map((f) => f.params.expected_revision), [0, 0, 1]);
  assertRequestsValid(host);
});
