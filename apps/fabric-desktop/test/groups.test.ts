// The Contacts screen's model against the fake host: no browser, no DOM. Every frame is judged
// by the contract's checker; broker answers come from the contract's fixtures where one fits.
import assert from "node:assert/strict";
import { test } from "node:test";
import type { RecipientSet, Snapshot } from "../../../contracts/desktop/v1/typescript/src/contract.ts";
import { GroupsModel, WIDENING } from "../ui/src/groups.ts";
import { assertRequestsValid, connect, fixture, setup } from "./support.ts";

const RICH: Snapshot = fixture("accept-owner-response-get-snapshot-rich.json").result;
const TEAM = RICH.recipient_sets[0] as RecipientSet;
const CONFLICT = fixture("accept-owner-response-error-conflict.json").error;

function screen(snap: Snapshot = RICH) {
  const s = setup();
  connect(s.host, 1, snap);
  return { ...s, groups: new GroupsModel(s.model), saves: () => s.host.sent.filter((f) => f.method === "save_group") };
}

const form = (g: GroupsModel) => {
  const f = g.form;
  assert.ok(f !== null, "a form is open");
  return f;
};
const item = (g: GroupsModel, id: string) => {
  const i = g.groups.find((x) => x.id === id);
  assert.ok(i !== undefined, `the group ${id} is listed`);
  return i;
};
const withTeam = (team: RecipientSet): Snapshot => ({ ...RICH, recipient_sets: [team, ...RICH.recipient_sets.slice(1)] });

test("the groups are listed with their counts and where they come from", () => {
  const { groups } = screen();
  assert.deepEqual(groups.groups.map((g) => [g.name, g.counts]), [
    ["Team", "2 addresses and 1 domain."],
    ["Family (Google contacts)", "2 addresses and 0 domains."],
  ]);
  assert.equal(item(groups, "rset-team").source, "Made in Fabric.");
  assert.match(item(groups, "rset-people-family").source, /^Imported from Google contacts of owner@example\.com on Friday, November 20, 2026/);
  assert.equal(item(groups, "rset-team").editLabel, "Edit the group Team");
  assert.equal(item(groups, "rset-team").deleteLabel, "Delete the group Team");
  assert.equal(groups.form, null);
});

test("a new group is saved with recipient_set_id null and expected_revision 0", () => {
  const { host, groups, announced } = screen();
  assert.equal(groups.openNew(), true);
  assert.deepEqual(groups.takeFocus(), { kind: "form" });
  assert.equal(form(groups).isNew, true);
  assert.equal(form(groups).dirty, false);
  assert.equal(form(groups).save.reason, "Nothing has changed.");
  groups.edit("name", "Contractors");
  groups.edit("addresses", "pat@example.net");
  groups.edit("domains", "example.org");
  assert.equal(form(groups).widening, null, "a new group widens nothing");
  assert.equal(groups.save(), true);
  assert.deepEqual(host.last("save_group").params, fixture("accept-owner-request-save-group-create.json").params);
  assert.equal(form(groups).save.reason, "Saving…");
  const saved = fixture("accept-owner-response-save-group-saved.json").result;
  host.notice(RICH.cursor + 1, { kind: "recipient_set", recipient_set: saved.recipient_set });
  host.reply("save_group", saved);
  assert.equal(groups.form, null);
  assert.deepEqual(announced.at(-1), "The group Contractors is saved.");
  assert.deepEqual(groups.takeFocus(), { kind: "group", id: "rset-contractors" });
});

test("the normal form: spaces dropped, the domain lowered, the part before the @ kept, duplicates and empty lines dropped, no alias rule", () => {
  const { host, groups } = screen();
  groups.openNew();
  groups.edit("name", "  Mixed  ");
  groups.edit("addresses", "  Pat.Lee@Example.NET \n\n a.b@x.com\nab@x.com\nPat.Lee@example.net\n\n");
  groups.edit("domains", " Example.ORG \nexample.org\n");
  assert.deepEqual(form(groups).problems, { name: [], addresses: [], domains: [] });
  groups.save();
  assert.deepEqual(host.last("save_group").params.change.definition, {
    name: "Mixed", members: ["Pat.Lee@example.net", "a.b@x.com", "ab@x.com"], domains: ["example.org"],
  });
});

test("an invalid line is named at its field and nothing is sent", () => {
  const { groups, saves } = screen();
  groups.openNew();
  groups.edit("name", "Friends");
  groups.edit("addresses", "pat@example.net\nnot an address\n");
  groups.edit("domains", "*.example.com");
  const f = form(groups);
  assert.deepEqual(f.problems.addresses, ["Line 2 is not an address: not an address"]);
  assert.deepEqual(f.problems.domains, ["Line 1 is not a domain: *.example.com"]);
  assert.equal(f.save.enabled, false);
  assert.equal(f.save.reason, "Line 2 is not an address: not an address");
  assert.equal(groups.save(), false);
  assert.equal(saves().length, 0);
  groups.edit("name", " ");
  assert.deepEqual(form(groups).problems.name, ["Give the group a name."]);
  groups.edit("name", "x".repeat(201));
  assert.deepEqual(form(groups).problems.name, ["The name is longer than 200 characters."]);
});

test("the bounds: at most 2000 addresses and 50 domains", () => {
  const { groups, saves } = screen();
  groups.openNew();
  groups.edit("name", "Many");
  const addresses = (n: number) => Array.from({ length: n }, (_, i) => `p${i}@example.net`).join("\n");
  const domains = (n: number) => Array.from({ length: n }, (_, i) => `d${i}.example.net`).join("\n");
  groups.edit("addresses", addresses(2001));
  groups.edit("domains", domains(51));
  assert.deepEqual(form(groups).problems.addresses, ["There are 2001 addresses; a group holds at most 2000."]);
  assert.deepEqual(form(groups).problems.domains, ["There are 51 domains; a group holds at most 50."]);
  assert.equal(groups.save(), false);
  groups.edit("addresses", addresses(2000));
  groups.edit("domains", domains(50));
  assert.equal(form(groups).save.enabled, true);
  assert.equal(groups.save(), true);
  assert.equal(saves().length, 1);
});

test("an update carries the set's id and revision", () => {
  const { host, groups } = screen();
  assert.equal(groups.openEdit("rset-team"), true);
  const f = form(groups);
  assert.equal(f.title, "Edit the group Team");
  assert.equal(f.addresses, "ally@example.net\nsam.lee@example.org");
  assert.equal(f.dirty, false);
  groups.edit("addresses", `${f.addresses}\nkim@example.net`);
  groups.save();
  assert.deepEqual(host.last("save_group").params, fixture("accept-owner-request-save-group-update.json").params);
});

test("the widening line appears only when the draft adds an address or a domain", () => {
  const { groups } = screen();
  groups.openEdit("rset-team");
  groups.edit("name", "The team");
  assert.equal(form(groups).widening, null);
  groups.edit("addresses", "ally@example.net");
  assert.equal(form(groups).widening, null, "removing someone narrows");
  groups.edit("addresses", "ally@example.net\nsam.lee@example.org\nkim@example.net");
  assert.equal(form(groups).widening, WIDENING);
  groups.edit("addresses", "ally@example.net\nsam.lee@example.org");
  groups.edit("domains", "example.com\nexample.org");
  assert.equal(form(groups).widening, WIDENING);
  groups.edit("domains", "EXAMPLE.com");
  assert.equal(form(groups).widening, null, "the same domain in capitals is the same domain");
});

test("delete takes two steps and sends the set's revision", () => {
  const { host, groups, announced, saves } = screen();
  assert.equal(groups.askDelete("rset-team"), true);
  assert.equal(item(groups, "rset-team").confirming, "Delete Team? Permissions that name it end.");
  assert.equal(saves().length, 0, "the first step sends nothing");
  groups.cancelDelete();
  assert.equal(item(groups, "rset-team").confirming, null);
  assert.deepEqual(groups.takeFocus(), { kind: "group", id: "rset-team" });
  groups.askDelete("rset-team");
  assert.equal(groups.confirmDelete(), true);
  assert.deepEqual(host.last("save_group").params, { expected_revision: 2, change: { kind: "delete", recipient_set_id: "rset-team" } });
  assert.equal(item(groups, "rset-team").del.reason, "Deleting…");
  host.notice(RICH.cursor + 1, fixture("accept-owner-notice-recipient-set-deleted.json").change);
  host.reply("save_group", fixture("accept-owner-response-save-group-deleted.json").result);
  assert.equal(announced.at(-1), "The group Team is deleted.");
  assert.deepEqual(groups.groups.map((g) => g.id), ["rset-people-family"]);
  assert.deepEqual(groups.takeFocus(), { kind: "new" });
});

test("one form at a time: while it has changes, the other groups and New group wait", () => {
  const { groups } = screen();
  groups.openEdit("rset-team");
  assert.equal(groups.openEdit("rset-people-family"), true, "a form without changes is simply replaced");
  assert.equal(form(groups).title, "Edit the group Family (Google contacts)");
  groups.edit("name", "Family");
  const reason = "Save or discard your changes to Family (Google contacts) first.";
  assert.deepEqual(groups.newGroup, { enabled: false, reason });
  assert.deepEqual(item(groups, "rset-team").edit, { enabled: false, reason });
  assert.deepEqual(item(groups, "rset-team").del, { enabled: false, reason });
  assert.equal(item(groups, "rset-people-family").edit.enabled, true);
  assert.equal(groups.openNew(), false);
  assert.equal(groups.openEdit("rset-team"), false);
  assert.equal(groups.askDelete("rset-team"), false);
  assert.equal(form(groups).name, "Family", "nothing was discarded");
  groups.discard();
  assert.equal(groups.form, null);
  assert.deepEqual(groups.takeFocus(), { kind: "group", id: "rset-people-family" });
  assert.equal(groups.newGroup.enabled, true);
  groups.openNew();
  groups.edit("name", "Neighbours");
  assert.equal(item(groups, "rset-team").edit.reason, "Save or discard your changes to Neighbours first.");
  groups.discard();
  assert.deepEqual(groups.takeFocus(), { kind: "new" });
});

test("conflict: nothing saved, the state is read again, the form is stale until the owner has seen it, then the save carries the new revision", () => {
  const { host, groups, saves } = screen();
  groups.openEdit("rset-team");
  groups.edit("addresses", "ally@example.net\nsam.lee@example.org\nkim@example.net");
  groups.save();
  host.fail("save_group", CONFLICT);
  assert.equal(form(groups).note, "Nothing was saved: the group changed since you opened it. The current version is read again.");
  assert.deepEqual(host.methods().slice(-1), ["get_snapshot"]);
  const newer: RecipientSet = { ...TEAM, revision: 3, definition: { ...TEAM.definition, members: ["ally@example.net"] } };
  host.reply("get_snapshot", withTeam(newer));
  host.reply("subscribe", { cursor: RICH.cursor });
  const f = form(groups);
  assert.match(f.stale ?? "", /^The group Team was changed while you were editing it/);
  assert.deepEqual(f.stored, { name: null, addresses: "Stored now: ally@example.net", domains: null });
  assert.equal(f.addresses, "ally@example.net\nsam.lee@example.org\nkim@example.net", "the owner's edits stay");
  assert.equal(f.save.enabled, false);
  assert.equal(groups.save(), false);
  groups.acknowledge();
  assert.equal(form(groups).stale, null);
  assert.equal(groups.save(), true);
  assert.equal(saves().length, 2);
  assert.equal(host.last("save_group").params.expected_revision, 3);
});

test("a notice for the edited group: a form with changes becomes stale, one without follows", () => {
  const { host, groups } = screen();
  groups.openEdit("rset-team");
  const renamed: RecipientSet = { ...TEAM, revision: 3, definition: { ...TEAM.definition, name: "Team A" } };
  host.notice(RICH.cursor + 1, { kind: "recipient_set", recipient_set: renamed });
  assert.equal(form(groups).name, "Team A");
  assert.equal(form(groups).stale, null);
  groups.edit("domains", "");
  host.notice(RICH.cursor + 2, { kind: "recipient_set", recipient_set: { ...renamed, revision: 4, definition: { ...renamed.definition, name: "Team B" } } });
  const f = form(groups);
  assert.notEqual(f.stale, null);
  assert.equal(f.name, "Team A");
  assert.deepEqual(f.stored, { name: "Stored now: Team B", addresses: null, domains: "Stored now: example.com" });
  assert.equal(f.save.enabled, false);
});

test("the edited group deleted meanwhile: the text stays to copy and cannot be saved", () => {
  const { host, groups } = screen();
  groups.openEdit("rset-team");
  groups.edit("name", "Team (old)");
  host.notice(RICH.cursor + 1, fixture("accept-owner-notice-recipient-set-deleted.json").change);
  const f = form(groups);
  assert.equal(f.deleted, "The group Team was deleted while you were editing it. Your text stays here so that you can copy it; it cannot be saved.");
  assert.equal(f.name, "Team (old)");
  assert.equal(f.save.reason, "This group no longer exists, so it cannot be saved.");
  assert.equal(groups.save(), false);
  groups.discard();
  assert.deepEqual(groups.takeFocus(), { kind: "new" });

  const other = screen();
  other.groups.openEdit("rset-team");
  other.host.notice(RICH.cursor + 1, fixture("accept-owner-notice-recipient-set-deleted.json").change);
  assert.equal(other.groups.form, null, "a form without changes closes with its group");
});

test("no answer: the draft stays and the same save reuses its command id", () => {
  const { host, groups, saves } = screen();
  groups.openEdit("rset-team");
  groups.edit("name", "Core team");
  groups.save();
  const first = host.last("save_group").id;
  host.state({ name: "disconnected", reason: "exited", detail: "exit code 1" });
  assert.equal(form(groups).note, "It is not known whether this was saved. Saving the same again is safe.");
  assert.equal(form(groups).name, "Core team");
  connect(host, 2, RICH);
  assert.equal(groups.save(), true);
  assert.equal(saves().length, 2);
  assert.equal(host.last("save_group").id, first);
});

test("nothing is sent while the broker is not ready, and the draft survives a disconnect and a restart", () => {
  const { host, model, groups, saves } = screen();
  groups.openNew();
  groups.edit("name", "Later");
  host.state({ name: "disconnected", reason: "exited", detail: "exit code 1" });
  const f = form(groups);
  assert.equal(f.save.enabled, false);
  assert.equal(f.save.reason, model.notReadyText);
  assert.equal(groups.save(), false);
  groups.edit("addresses", "pat@example.net");
  host.state({ name: "connected" }, 2);
  assert.equal(form(groups).save.reason, "Not available until the broker's current state has been read.");
  host.reply("hello", fixture("accept-owner-response-hello.json").result);
  host.reply("get_snapshot", RICH);
  host.reply("subscribe", { cursor: RICH.cursor });
  assert.equal(form(groups).name, "Later");
  assert.equal(form(groups).addresses, "pat@example.net");
  assert.equal(saves().length, 0);
  assert.equal(groups.save(), true);
});

test("every command of the screen sends a valid owner request", () => {
  const { host, groups } = screen();
  groups.openNew();
  groups.edit("name", "Contractors");
  groups.edit("addresses", "Pat@Example.net\n");
  groups.edit("domains", "Example.ORG");
  groups.save();
  host.error("save_group", "internal");
  assert.equal(form(groups).note, "The broker did not save this: Refused by the test.");
  groups.save();
  host.reply("save_group", fixture("accept-owner-response-save-group-saved.json").result);
  groups.openEdit("rset-team");
  groups.edit("addresses", "ally@example.net");
  groups.save();
  host.reply("save_group", { commit_sequence: RICH.cursor + 1, recipient_set: { ...TEAM, revision: 3, definition: { ...TEAM.definition, members: ["ally@example.net"] } } });
  assert.equal(groups.askDelete("rset-people-family"), true);
  groups.confirmDelete();
  host.error("save_group", "not_found");
  assert.equal(item(groups, "rset-people-family").note, "The broker did not delete the group Family (Google contacts): Refused by the test.");
  assert.deepEqual(host.sent.filter((f) => f.method === "save_group").map((f) => f.params.change.kind), ["save", "save", "save", "delete"]);
  assertRequestsValid(host);
});
