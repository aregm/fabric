// The Agent permissions screen's model against the fake host of support.ts: no browser, no DOM.
// Every frame it sends, and every frame the tests feed it, is judged by the contract's checker.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { MODES as CONTRACT_MODES, POLICY_ROWS } from "../../../contracts/desktop/v1/typescript/src/contract.ts";
import type { OwnerError, Profile, Profiles, Snapshot } from "../../../contracts/desktop/v1/typescript/src/contract.ts";
import { CONTRACT_ROOT } from "../../../contracts/desktop/v1/typescript/src/judge.ts";
import type { Model } from "../ui/src/model.ts";
import { LOCKED, MEANING, MODES, MODE_NAME, NO_ANSWER, ProfilesModel, ROWS, ROW_NAME, STALE_TEXT } from "../ui/src/profiles.ts";
import type { EditableRow, Screen } from "../ui/src/profiles.ts";
import { assertRequestsValid, connect, fixture, flush, setup, snapshot } from "./support.ts";
import type { FakeHost } from "./support.ts";

const RICH: Snapshot = fixture("accept-owner-response-get-snapshot-rich.json").result;
const SAVED: Profiles = fixture("accept-owner-response-save-profile.json").result.profiles; // RICH's profiles and Lockdown, revision 8
const CONFLICT: OwnerError = fixture("accept-owner-response-error-conflict.json").error;
const EXPANSION: OwnerError = fixture("accept-owner-response-error-expansion-not-acknowledged.json").error;
const FIRST_ATTEMPT = fixture("accept-owner-request-save-profile-custom-expansion-first-attempt.json").params;
const CONFIRMED = fixture("accept-owner-request-save-profile-custom-expansion-confirmed.json").params;
const NAME_TAKEN: OwnerError = {
  code: "invalid_params", message: "Careful and Flexible are the built-in profiles, so choose another name.",
  retryable: false, recovery: "report_problem", correlation_id: null, detail: { pointer: "/params/change/profile/name" },
};

function live(snap: Snapshot = RICH) {
  const s = setup();
  connect(s.host, 1, snap);
  return { ...s, pm: new ProfilesModel(s.model) };
}

function screen(pm: ProfilesModel): Screen {
  const s = pm.screen;
  assert.ok(s !== null);
  return s;
}

const modeOf = (pm: ProfilesModel, row: string) => screen(pm).rows.find((r) => r.row === row)?.mode;
const withProfiles = (snap: Snapshot, profiles: Profiles): Snapshot => ({ ...snap, profiles });

// The broker's way: the notice with the new profiles first, then the command's result.
function commit(host: FakeHost, model: Model, profiles: Profiles): void {
  host.notice(model.cursor + 1, { kind: "profiles", profiles });
  host.reply("save_profile", { commit_sequence: model.cursor, profiles });
}

// Sets every editable row of the shown profile to these modes.
function setRows(pm: ProfilesModel, rows: Partial<Record<EditableRow, "allow" | "ask" | "block">>): void {
  for (const [row, mode] of Object.entries(rows)) pm.setMode(row as EditableRow, mode);
}

const ALL_ALLOW = {
  list_and_availability: "allow", read_event_details: "allow", create_in_agent_drafts: "allow",
  create_in_existing_calendar: "allow", edit_fabric_created_event: "allow", edit_preexisting_event: "allow",
  send_or_change_invitations: "allow", respond_to_invitation: "allow",
} as const;
const ALL_BLOCK = Object.fromEntries(Object.keys(ALL_ALLOW).map((r) => [r, "block"])) as Record<EditableRow, "block">;

const saves = (host: FakeHost) => host.sent.filter((f) => f.method === "save_profile");

// ---- Tests ----

test("first run: Careful is in force and shown, built in, and cannot be put in force again", () => {
  const { pm } = live(snapshot(0));
  const s = screen(pm);
  assert.equal(s.shown, "careful");
  assert.deepEqual(s.list, [
    { id: "careful", name: "Careful", tags: ["built in", "in force"] },
    { id: "flexible", name: "Flexible", tags: ["built in"] },
  ]);
  assert.equal(s.name, null);
  assert.equal(s.dirty, false);
  assert.deepEqual(s.activate, { label: "Put Careful in force", disabled: "Careful is in force already." });
  assert.equal(s.save.disabled, "There are no changes to save.");
  assert.equal(s.discard.disabled, "There are no changes to discard.");
});

test("before the profiles are read there is no screen", () => {
  const s = setup();
  assert.equal(new ProfilesModel(s.model).screen, null);
});

test("the profile in force is shown first; a chosen profile stays shown until it disappears", () => {
  const active = withProfiles(RICH, { ...RICH.profiles, active_profile_id: "custom-assistant" });
  const { host, model, pm } = live(active);
  assert.equal(screen(pm).shown, "custom-assistant");
  assert.deepEqual(screen(pm).list[2], { id: "custom-assistant", name: "Assistant hours", tags: ["custom, version 2", "in force"] });
  pm.choose("flexible");
  assert.equal(screen(pm).shown, "flexible");
  pm.choose("no-such-profile");
  assert.equal(screen(pm).shown, "flexible");
  // A snapshot without the custom profile: the chosen one stays; choose it and let it disappear.
  pm.choose("custom-assistant");
  model.refresh();
  host.reply("get_snapshot", withProfiles(RICH, { ...RICH.profiles, revision: 9, items: RICH.profiles.items.slice(0, 2) }));
  assert.equal(screen(pm).shown, "careful", "the shown profile disappeared: the one in force is shown");
});

test("choosing another profile keeps the draft of each", () => {
  const { pm } = live();
  pm.choose("flexible");
  pm.setMode("edit_preexisting_event", "block");
  pm.choose("careful");
  assert.equal(screen(pm).dirty, false);
  assert.equal(modeOf(pm, "edit_preexisting_event"), "ask");
  assert.deepEqual(screen(pm).list.find((i) => i.id === "flexible")?.tags, ["built in", "unsaved changes"]);
  pm.choose("flexible");
  assert.equal(screen(pm).dirty, true);
  assert.equal(modeOf(pm, "edit_preexisting_event"), "block");
});

test("changing a row of a built-in drafts a new profile and asks for its name; edited back it is no draft", () => {
  const { pm } = live();
  assert.equal(screen(pm).name, null);
  pm.setMode("read_event_details", "block");
  const s = screen(pm);
  assert.equal(s.dirty, true);
  assert.equal(s.stale, null);
  assert.deepEqual(s.name, { label: "Name of the new profile", value: "", stored: null, message: null });
  assert.deepEqual(s.save, { label: "Save as a custom profile", disabled: null });
  assert.equal(s.discard.disabled, null);
  assert.equal(s.activate.disabled, "Careful is in force already.");
  const row = s.rows.find((r) => r.row === "read_event_details");
  assert.deepEqual(row, {
    row: "read_event_details", name: "Read the details of events", mode: "block", locked: false,
    stored: "Now in Careful: Ask", meaning: "Never allowed. The agent is told that your profile blocks it.",
  });
  pm.setName("Mine");
  pm.setMode("read_event_details", "ask");
  assert.equal(screen(pm).dirty, false, "the rows are Careful's again");
  assert.equal(screen(pm).name, null);
  assert.equal(screen(pm).activate.disabled, "Careful is in force already.");
});

test("a custom profile shows its name; changing the name alone is a draft of a new version", () => {
  const { pm } = live();
  pm.choose("custom-assistant");
  assert.deepEqual(screen(pm).name, { label: "Name of this profile", value: "Assistant hours", stored: null, message: null });
  assert.equal(screen(pm).save.label, "Save changes");
  pm.setName("Assistant days");
  assert.equal(screen(pm).dirty, true);
  assert.equal(screen(pm).name?.stored, "Now: Assistant hours");
  pm.setName("Assistant hours ");
  assert.equal(screen(pm).dirty, false, "surrounding spaces are not a change");
  pm.setName("Assistant days");
  pm.discard();
  assert.equal(screen(pm).dirty, false);
  assert.equal(screen(pm).name?.value, "Assistant hours");
});

test("an empty or too long name is not sent and the field names the problem", () => {
  const { host, pm } = live();
  pm.setMode("respond_to_invitation", "block");
  assert.equal(pm.save(), false);
  assert.equal(screen(pm).name?.message, "Give the profile a name.");
  pm.setName("   ");
  assert.equal(screen(pm).name?.message, null, "typing clears the message");
  assert.equal(pm.save(), false);
  assert.equal(screen(pm).name?.message, "Give the profile a name.");
  pm.setName("x".repeat(201));
  assert.equal(pm.save(), false);
  assert.equal(screen(pm).name?.message, "The name is too long: at most 200 characters.");
  assert.equal(saves(host).length, 0);
  pm.setName(` ${"x".repeat(200)} `);
  assert.equal(pm.save(), true);
  assert.equal(saves(host)[0].params.change.profile.name, "x".repeat(200), "the name is sent trimmed");
});

test("activate: the frame, success, and disabled while the shown profile has changes", () => {
  const { host, model, pm, announced } = live();
  pm.choose("custom-assistant");
  assert.equal(screen(pm).activate.label, "Put Assistant hours in force");
  pm.setMode("respond_to_invitation", "ask");
  assert.equal(screen(pm).activate.disabled, "Save or discard your changes first.");
  assert.equal(pm.activate(), false);
  pm.discard();
  assert.equal(pm.activate(), true);
  assert.deepEqual(host.last("save_profile").params, fixture("accept-owner-request-save-profile-activate-custom.json").params);
  assert.equal(screen(pm).activate.disabled, "Putting it in force…");
  assert.equal(pm.activate(), false);
  commit(host, model, { ...RICH.profiles, revision: 8, active_profile_id: "custom-assistant" });
  assert.equal(announced.at(-1), "Assistant hours is now in force for every agent.");
  assert.equal(screen(pm).activate.disabled, "Assistant hours is in force already.");
  assert.deepEqual(screen(pm).list[2]?.tags, ["custom, version 2", "in force"]);
});

test("activate: a conflict changes nothing and reads the current state", () => {
  const { host, pm } = live();
  pm.choose("flexible");
  pm.activate();
  host.fail("save_profile", CONFLICT);
  assert.match(screen(pm).activateNote ?? "", /^Nothing changed/);
  assert.equal(host.methods().at(-1), "get_snapshot");
  assert.equal(screen(pm).activate.disabled, "Not available until the broker's current state has been read.");
});

test("activate: no answer, then the retry is the same command", async () => {
  const { host, pm } = live();
  pm.choose("flexible");
  pm.activate();
  const first = host.last("save_profile").id;
  host.state({ name: "disconnected", reason: "exited", detail: "exit code 1" });
  assert.equal(screen(pm).activateNote, "It is not known whether Flexible was put in force. Trying again is safe.");
  assert.equal(screen(pm).activate.disabled, "Not available while the broker is not running.");
  connect(host, 2, RICH);
  await flush();
  pm.activate();
  assert.equal(host.last("save_profile").id, first);
  assert.equal(screen(pm).activateNote, null);
});

test("saving a narrower custom profile succeeds without a panel and selects the new profile", () => {
  const { host, model, pm, announced } = live();
  setRows(pm, ALL_BLOCK);
  pm.setName("Lockdown");
  assert.equal(pm.save(), true);
  const frame = host.last("save_profile");
  assert.deepEqual(frame.params, {
    expected_revision: 7,
    change: { kind: "save_custom", profile: { profile_id: null, name: "Lockdown", rows: { ...ALL_BLOCK, delete_event: "ask", delete_calendar_or_change_sharing: "block" } }, acknowledged_expansion: null },
  });
  assert.equal(screen(pm).save.disabled, "Saving…");
  assert.equal(pm.save(), false);
  commit(host, model, SAVED);
  const s = screen(pm);
  assert.equal(s.panel, null);
  assert.equal(s.shown, "custom-lockdown");
  assert.equal(s.dirty, false);
  assert.deepEqual(s.list.find((i) => i.id === "custom-lockdown")?.tags, ["custom, version 1"]);
  assert.equal(announced.at(-1), "Lockdown is saved as a custom profile. Careful is in force for every agent.");
  pm.choose("careful");
  assert.equal(screen(pm).dirty, false, "Careful's draft became the new profile");
});

test("saving a wider profile: the broker's expansion, confirm with its digest, and what drops the panel", () => {
  const { host, model, pm, announced } = live();
  setRows(pm, ALL_ALLOW);
  pm.setName("Assistant hours");
  pm.save();
  const first = host.last("save_profile");
  assert.deepEqual(first.params, FIRST_ATTEMPT);
  host.fail("save_profile", EXPANSION);
  assert.deepEqual(screen(pm).panel, [
    "Change events that were already in your calendar: from Ask to Allow",
    "Answer invitations: from Ask to Allow",
  ]);
  assert.equal(screen(pm).saveNote, null);
  assert.deepEqual(screen(pm).confirm, { label: "Confirm and save", disabled: null });

  // An edit drops the panel; so does "Go back".
  pm.setMode("list_and_availability", "ask");
  assert.equal(screen(pm).panel, null);
  assert.equal(pm.confirmExpansion(), false);
  pm.setMode("list_and_availability", "allow");
  pm.save();
  host.fail("save_profile", EXPANSION);
  pm.goBack();
  assert.equal(screen(pm).panel, null);
  pm.save();
  host.fail("save_profile", EXPANSION);

  assert.equal(pm.confirmExpansion(), true);
  const confirmed = host.last("save_profile");
  assert.notEqual(confirmed.id, first.id);
  assert.deepEqual(confirmed.params, CONFIRMED);
  assert.equal(screen(pm).confirm.disabled, "Saving…");
  const created: Profile = { kind: "custom", profile_id: "custom-0123456789abcdef", name: "Assistant hours", version: 1, rows: CONFIRMED.change.profile.rows };
  commit(host, model, { ...RICH.profiles, revision: 8, items: [...RICH.profiles.items, created] });
  assert.equal(screen(pm).panel, null);
  assert.equal(screen(pm).shown, "custom-0123456789abcdef", "the new one, not the other of the same name");
  assert.match(announced.at(-1) ?? "", /^Assistant hours is saved as a custom profile\./);
});

test("the broker's refusal of a name is shown at the name field", () => {
  const { host, pm, announced } = live();
  pm.setMode("read_event_details", "block");
  pm.setName("CAREFUL");
  pm.save();
  host.fail("save_profile", NAME_TAKEN);
  assert.equal(screen(pm).name?.message, NAME_TAKEN.message);
  assert.equal(screen(pm).saveNote, null);
  assert.equal(announced.at(-1), NAME_TAKEN.message);
  assert.equal(screen(pm).dirty, true);
  pm.setName("Mine");
  assert.equal(screen(pm).name?.message, null);
});

test("another refusal and no answer keep the draft with a note", async () => {
  const { host, pm } = live();
  pm.setMode("read_event_details", "block");
  pm.setName("Mine");
  pm.save();
  host.fail("save_profile", fixture("accept-owner-response-error-invalid-params.json").error);
  assert.equal(screen(pm).saveNote, "The broker did not save this: The duration is not one the broker offers.");
  host.refusal = { kind: "io", detail: "broken pipe" };
  pm.save();
  const first = host.last("save_profile").id;
  await flush();
  assert.equal(screen(pm).saveNote, NO_ANSWER);
  assert.equal(screen(pm).dirty, true);
  host.refusal = null;
  pm.save();
  assert.equal(host.last("save_profile").id, first, "the same save again is the same command");
});

test("conflict: nothing saved, the state is read again, the draft is stale until the owner has seen it", () => {
  const { host, model, pm, announced } = live();
  pm.choose("custom-assistant");
  pm.setMode("respond_to_invitation", "ask");
  pm.save();
  assert.equal(host.last("save_profile").params.expected_revision, 7);
  host.fail("save_profile", CONFLICT);
  assert.equal(screen(pm).saveNote, "Nothing was saved: your profiles changed while you were editing. The current version is read again.");
  assert.equal(announced.at(-1), screen(pm).saveNote);
  assert.equal(host.methods().at(-1), "get_snapshot");
  const newer = withProfiles(RICH, { ...RICH.profiles, revision: 8, active_profile_id: "custom-assistant" });
  host.reply("get_snapshot", newer);
  host.reply("subscribe", { cursor: newer.cursor });
  let s = screen(pm);
  assert.equal(s.stale, STALE_TEXT);
  assert.equal(s.dirty, true);
  assert.equal(modeOf(pm, "respond_to_invitation"), "ask", "the owner's edit is kept");
  assert.equal(s.save.disabled, "Look at the current version first: press “I have seen the current version”.");
  assert.equal(pm.save(), false);
  pm.acknowledgeStale();
  s = screen(pm);
  assert.equal(s.stale, null);
  assert.equal(s.save.disabled, null);
  pm.save();
  assert.equal(host.last("save_profile").params.expected_revision, 8);
  const v3: Profile = { ...RICH.profiles.items[2] as Profile & { kind: "custom" }, version: 3, rows: host.last("save_profile").params.change.profile.rows };
  commit(host, model, { ...newer.profiles, revision: 9, items: [...RICH.profiles.items.slice(0, 2), v3] });
  assert.equal(announced.at(-1), "Assistant hours is saved. It is in force, so it applies at once to every agent.");
  assert.equal(screen(pm).dirty, false);
  assert.equal(screen(pm).shown, "custom-assistant");
});

test("a profiles notice makes a dirty draft stale; a clean form follows it", () => {
  const { host, model, pm } = live();
  pm.choose("custom-assistant");
  const changed: Profile = { ...RICH.profiles.items[2] as Profile & { kind: "custom" }, version: 3, rows: { ...ALL_ALLOW, read_event_details: "ask", delete_event: "ask", delete_calendar_or_change_sharing: "block" } };
  host.notice(model.cursor + 1, { kind: "profiles", profiles: { ...RICH.profiles, revision: 8, items: [...RICH.profiles.items.slice(0, 2), changed] } });
  assert.equal(modeOf(pm, "read_event_details"), "ask", "a clean form follows the stored profile");
  assert.equal(screen(pm).stale, null);
  pm.choose("flexible");
  pm.setMode("respond_to_invitation", "block");
  host.notice(model.cursor + 1, { kind: "profiles", profiles: { ...RICH.profiles, revision: 9, active_profile_id: "custom-assistant", items: [...RICH.profiles.items.slice(0, 2), changed] } });
  assert.equal(screen(pm).stale, STALE_TEXT, "another profile was put in force: the whole revision moved");
  assert.equal(modeOf(pm, "respond_to_invitation"), "block");
});

test("nothing is sent while the broker is not ready; the draft survives a disconnect and a restart", async () => {
  const { host, pm } = live();
  pm.setMode("read_event_details", "block");
  pm.setName("Mine");
  host.state({ name: "disconnected", reason: "exited", detail: "exit code 1" });
  assert.equal(screen(pm).save.disabled, "Not available while the broker is not running.");
  pm.setMode("create_in_agent_drafts", "ask"); // editing stays possible
  assert.equal(pm.save(), false);
  pm.choose("flexible");
  assert.equal(pm.activate(), false);
  assert.equal(saves(host).length, 0);
  host.state({ name: "connected" }, 2);
  pm.choose("careful");
  assert.equal(screen(pm).save.disabled, "Not available until the broker's current state has been read.");
  host.reply("hello", fixture("accept-owner-response-hello.json").result);
  host.reply("get_snapshot", RICH);
  host.reply("subscribe", { cursor: RICH.cursor });
  await flush();
  pm.choose("careful");
  const s = screen(pm);
  assert.equal(s.dirty, true);
  assert.equal(s.stale, null);
  assert.equal(s.name?.value, "Mine");
  assert.equal(modeOf(pm, "create_in_agent_drafts"), "ask");
  assert.equal(pm.save(), true);
});

test("the built-in modes shown are the policy matrix; the locked rows are those with one custom mode", () => {
  const matrix = JSON.parse(readFileSync(`${CONTRACT_ROOT}data/policy-matrix.json`, "utf8")) as {
    rows: { row: string; careful: { mode: string }; flexible: { mode: string }; custom_modes: string[] }[];
  };
  const { pm } = live(snapshot(0));
  assert.deepEqual(matrix.rows.map((r) => r.row), [...POLICY_ROWS]);
  for (const id of ["careful", "flexible"] as const) {
    pm.choose(id);
    assert.deepEqual(screen(pm).rows.map((r) => [r.row, r.mode]), matrix.rows.map((r) => [r.row, r[id].mode]));
  }
  assert.deepEqual(
    screen(pm).rows.filter((r) => r.locked).map((r) => r.row),
    matrix.rows.filter((r) => r.custom_modes.length === 1).map((r) => r.row),
  );
  for (const r of screen(pm).rows.filter((x) => x.locked)) assert.equal(r.meaning, LOCKED[r.row as keyof typeof LOCKED]);
});

test("every table of words is complete and in the contract's order", () => {
  assert.deepEqual([...ROWS], [...POLICY_ROWS]);
  assert.deepEqual([...MODES].sort(), [...CONTRACT_MODES].sort());
  for (const row of POLICY_ROWS) assert.ok(ROW_NAME[row].length > 0, row);
  for (const mode of CONTRACT_MODES) assert.ok(MODE_NAME[mode].length > 0, mode);
  assert.deepEqual(Object.keys(LOCKED).sort(), ["delete_calendar_or_change_sharing", "delete_event"]);
  const editable = POLICY_ROWS.filter((r) => !(r in LOCKED));
  assert.deepEqual(Object.keys(MEANING), editable);
  for (const row of editable) for (const mode of CONTRACT_MODES) assert.ok(MEANING[row as EditableRow][mode].length > 0, `${row} ${mode}`);
  for (const mode of CONTRACT_MODES) assert.match(MEANING.send_or_change_invitations[mode], /Guests may get email from Google\.$/);
});

test("every frame of the screen's commands is a valid owner request", () => {
  const { host, model, pm } = live();
  pm.choose("flexible");
  pm.activate();
  commit(host, model, { ...RICH.profiles, revision: 8, active_profile_id: "flexible" });
  pm.choose("careful");
  setRows(pm, ALL_ALLOW);
  pm.setName("Wide");
  pm.save();
  host.fail("save_profile", { ...EXPANSION, correlation_id: null });
  pm.confirmExpansion();
  const wide: Profile = { kind: "custom", profile_id: "custom-0123456789abcdef", name: "Wide", version: 1, rows: host.last("save_profile").params.change.profile.rows };
  commit(host, model, { ...RICH.profiles, revision: 9, active_profile_id: "flexible", items: [...RICH.profiles.items, wide] });
  pm.choose("custom-assistant");
  pm.setName("Assistant days");
  pm.save();
  host.fail("save_profile", CONFLICT);
  assert.ok(saves(host).length >= 3);
  assertRequestsValid(host);
});
