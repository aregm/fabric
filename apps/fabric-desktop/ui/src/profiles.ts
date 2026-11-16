// The Agent permissions screen (plan S02, plan 7), with no DOM: which profile is shown, one draft
// per profile, the expansion the broker reported, and every word and every decision the view
// shows. The page computes no expansion, no comparison of profiles and no effective policy: it
// shows the broker's profiles and sends the owner's commands. A draft is "dirty" when its name or
// rows differ from the stored profile it was made from, and "stale" when the profiles revision is
// no longer the one it was made from (the revision covers every profile, contract 7).
import type {
  CustomProfileId, Digest, Mode, ParamsSaveProfile, PolicyRow, Profile, ProfileRows, Profiles,
} from "../../../../contracts/desktop/v1/typescript/src/contract.ts";
import type { Model } from "./model.ts";
import { isLabel } from "./shape.ts";

// Rows that no profile can change (plan 7, G0): the schema fixes their one mode.
export type LockedRow = "delete_event" | "delete_calendar_or_change_sharing";
export type EditableRow = Exclude<PolicyRow, LockedRow>;

// In the contract's order (POLICY_ROWS; the page imports the contract's types only).
export const ROW_NAME: Record<PolicyRow, string> = {
  list_and_availability: "See calendar names and when you are free or busy",
  read_event_details: "Read the details of events",
  create_in_agent_drafts: "Create events in the Agent Drafts calendar",
  create_in_existing_calendar: "Create events in your other selected calendars",
  edit_fabric_created_event: "Change events that Fabric created",
  edit_preexisting_event: "Change events that were already in your calendar",
  send_or_change_invitations: "Send or change invitations",
  respond_to_invitation: "Answer invitations",
  delete_event: "Delete events",
  delete_calendar_or_change_sharing: "Delete a calendar or change who it is shared with",
};

export const MODE_NAME: Record<Mode, string> = { allow: "Allow", ask: "Ask", block: "Block" };

export const ROWS = Object.keys(ROW_NAME) as readonly PolicyRow[];
export const MODES = Object.keys(MODE_NAME) as readonly Mode[];

// What a mode means, from the rule theory (`standing_serves` in isabelle/Fabric_Broker).
const GENERAL: Record<Mode, string> = {
  allow: "Allowed inside a permission you gave that lasts longer than one request. Without one, it needs your approval.",
  ask: "Needs your approval each time.",
  block: "Never allowed. The agent is told that your profile blocks it.",
};
const GUESTS = "Guests may get email from Google.";

export const MEANING: Record<EditableRow, Record<Mode, string>> = {
  list_and_availability: GENERAL,
  read_event_details: {
    ...GENERAL,
    ask: "Needs your approval once for the fields and dates it wants; reads inside that permission then run for as long as it lasts.",
  },
  create_in_agent_drafts: GENERAL,
  create_in_existing_calendar: GENERAL,
  edit_fabric_created_event: {
    allow: "Allowed inside a permission you gave. Adding a guest also needs the invitation row.",
    ask: "Needs your approval each time, except for events in the Agent Drafts calendar.",
    block: GENERAL.block,
  },
  edit_preexisting_event: GENERAL,
  send_or_change_invitations: {
    allow: `Allowed inside a permission that names the guests or a group. An address outside every group you approved always needs your approval. ${GUESTS}`,
    ask: `${GENERAL.ask} ${GUESTS}`,
    block: `${GENERAL.block} ${GUESTS}`,
  },
  respond_to_invitation: GENERAL,
};

export const LOCKED: Record<LockedRow, string> = {
  delete_event: "Always Ask, and only for events that Fabric created. No profile can change this, so an agent never deletes an event without your approval.",
  delete_calendar_or_change_sharing: "Always blocked. No profile can change this: no agent may delete a calendar or change who it is shared with.",
};

const isLocked = (row: PolicyRow): row is LockedRow => Object.hasOwn(LOCKED, row);

export const STALE_TEXT = "Your profiles changed while you were editing: one was saved or put in force. Your changes are kept.";
export const STALE_ANNOUNCEMENT = "Your profiles changed while you were editing. Your changes are kept. Look at the current version before you save.";
export const ONE_IN_FORCE = "One profile is in force for every agent. Putting another in force does not widen a permission you already gave.";
export const EMAIL_TEXT = "Rules for email are planned for a later version. There is nothing to set here yet.";
export const LOADING_TEXT = "The profiles have not been read from the broker yet.";
export const NO_ANSWER = "It is not known whether this was saved. Saving the same again is safe.";
const NAME_POINTER = "/params/change/profile/name";

type Rows = { -readonly [R in PolicyRow]: ProfileRows[R] };

interface Draft {
  readonly base: string; // the id of the stored profile it was made from
  readonly profileId: CustomProfileId | null; // null: a new custom profile, made from a built-in
  name: string;
  rows: Rows;
  baseRevision: number; // profiles.revision when the draft was made, or when the owner saw the newer one
  note: string | null; // the last problem with saving it
  nameNote: string | null; // the problem with its name
}

interface Panel {
  readonly base: string;
  readonly lines: readonly string[]; // the broker's rows, as it sent them
  readonly digest: Digest;
  readonly params: ParamsSaveProfile; // the save it answered; confirming sends it again with the digest
}

export interface ListItem {
  readonly id: string;
  readonly name: string;
  readonly tags: readonly string[];
}

export interface RowState {
  readonly row: PolicyRow;
  readonly name: string;
  readonly mode: Mode; // the chosen mode
  readonly stored: string | null; // the stored mode, in words, when the chosen one differs
  readonly meaning: string; // what the chosen mode means for this row, or why the row is locked
  readonly locked: boolean;
}

// A button: its text, and why it does nothing now (null: it can be used).
export interface Action {
  readonly label: string;
  readonly disabled: string | null;
}

export interface NameField {
  readonly label: string;
  readonly value: string;
  readonly stored: string | null;
  readonly message: string | null;
}

export interface Screen {
  readonly list: readonly ListItem[];
  readonly shown: string;
  readonly rows: readonly RowState[];
  readonly name: NameField | null;
  readonly nameAttention: number; // rises each time the name needs the owner: focus goes to the field
  readonly dirty: boolean;
  readonly stale: string | null;
  readonly save: Action;
  readonly discard: Action;
  readonly activate: Action;
  readonly confirm: Action;
  readonly saveNote: string | null;
  readonly activateNote: string | null;
  readonly panel: readonly string[] | null; // "<row>: from <mode> to <mode>" per row the broker listed
}

const sameRows = (a: ProfileRows, b: ProfileRows): boolean => ROWS.every((r) => a[r] === b[r]);

export class ProfilesModel {
  private readonly model: Model;
  private chosen: string | null = null; // the profile the owner chose to show; null: the one in force
  private readonly drafts = new Map<string, Draft>(); // by the id of the profile it was made from
  private panel: Panel | null = null;
  private saving: string | null = null; // the base of the draft whose save is awaited
  private activating = false;
  private activateNote: { readonly id: string; readonly text: string } | null = null;
  private nameAttention = 0; // raised each time the name needs the owner; the view then moves focus to it

  constructor(model: Model) {
    this.model = model;
  }

  // The profile shown: the owner's choice while it exists, otherwise the one in force.
  get shownId(): string | null {
    const p = this.model.profiles;
    if (p === null) return null;
    return this.chosen !== null && p.items.some((i) => i.profile_id === this.chosen) ? this.chosen : p.active_profile_id;
  }

  // Everything the view shows; null until the broker's profiles are known.
  get screen(): Screen | null {
    const profiles = this.model.profiles;
    const shown = this.shownProfile();
    if (profiles === null || shown === null) return null;
    const draft = this.draft(shown.profile_id);
    const held = this.draft(shown.profile_id, true);
    const rows = draft?.rows ?? shown.rows;
    // A draft whose own save is awaited is not stale: the broker's notice of that save comes before its answer.
    const stale = draft !== null && this.saving !== draft.base && draft.baseRevision !== profiles.revision;
    const inForce = shown.profile_id === profiles.active_profile_id;
    const custom = shown.kind === "custom";
    const notReady = this.model.notReadyText;
    const sendBlock = this.saving !== null ? "Saving…" : notReady ?? (stale ? "Look at the current version first: press “I have seen the current version”." : null);
    const panel = this.panel !== null && this.panel.base === shown.profile_id && draft !== null ? this.panel : null;
    return {
      list: profiles.items.map((p) => this.listItem(p, profiles)),
      shown: shown.profile_id,
      rows: ROWS.map((row) => {
        const mode = rows[row];
        const differs = mode !== shown.rows[row];
        return {
          row, name: ROW_NAME[row], mode, locked: isLocked(row),
          stored: differs ? `Now in ${shown.name}: ${MODE_NAME[shown.rows[row]]}` : null,
          meaning: isLocked(row) ? LOCKED[row] : MEANING[row][mode],
        };
      }),
      name: custom || draft !== null ? {
        label: custom ? "Name of this profile" : "Name of the new profile",
        value: held?.name ?? shown.name, // as typed, also while it differs only by spaces
        stored: custom && draft !== null && draft.name.trim() !== shown.name ? `Now: ${shown.name}` : null,
        message: held?.nameNote ?? null,
      } : null,
      nameAttention: this.nameAttention,
      dirty: draft !== null,
      stale: stale ? STALE_TEXT : null,
      save: {
        label: custom ? "Save changes" : "Save as a custom profile",
        disabled: draft === null ? "There are no changes to save." : sendBlock,
      },
      discard: { label: "Discard changes", disabled: draft === null ? "There are no changes to discard." : null },
      activate: {
        label: `Put ${shown.name} in force`,
        disabled: inForce ? `${shown.name} is in force already.`
          : draft !== null ? "Save or discard your changes first."
          : this.activating ? "Putting it in force…"
          : notReady,
      },
      confirm: { label: "Confirm and save", disabled: sendBlock },
      saveNote: draft?.note ?? null,
      activateNote: this.activateNote?.id === shown.profile_id ? this.activateNote.text : null,
      panel: panel?.lines ?? null,
    };
  }

  // ---- What the owner does ----

  choose(profileId: string): void {
    if (this.model.profiles?.items.some((p) => p.profile_id === profileId) !== true) return;
    this.chosen = profileId;
    this.model.changed();
  }

  setMode(row: EditableRow, mode: Mode): void {
    const d = this.edit();
    if (d === null) return;
    d.rows[row] = mode;
    this.edited(d);
  }

  setName(text: string): void {
    const d = this.edit();
    if (d === null) return;
    d.name = text;
    d.nameNote = null;
    this.edited(d);
  }

  discard(): void {
    const id = this.shownId;
    if (id === null || !this.drafts.delete(id)) return;
    if (this.panel?.base === id) this.panel = null;
    this.model.changed();
  }

  // The owner saw the newer stored version: the draft is now written against it.
  acknowledgeStale(): void {
    const d = this.shownDraft();
    const revision = this.model.profiles?.revision;
    if (d === null || revision === undefined) return;
    d.baseRevision = revision;
    if (this.panel?.base === d.base) this.panel = null;
    this.model.changed();
  }

  // Sends the shown draft. A name of the wrong shape is named at the field and nothing is sent.
  save(): boolean {
    const d = this.shownDraft();
    if (d === null || this.screen?.save.disabled !== null) return false;
    const name = d.name.trim();
    if (!isLabel(name)) {
      d.nameNote = name === "" ? "Give the profile a name." : "The name is too long: at most 200 characters.";
      this.nameAttention += 1;
      this.model.announce(d.nameNote);
      this.model.changed();
      return false;
    }
    if (this.panel?.base === d.base) this.panel = null;
    this.send(d, {
      expected_revision: d.baseRevision,
      change: { kind: "save_custom", profile: { profile_id: d.profileId, name, rows: { ...d.rows } }, acknowledged_expansion: null },
    });
    return true;
  }

  // The owner confirms the expansion the broker listed: the same draft, with its digest, as a new command.
  confirmExpansion(): boolean {
    const p = this.panel;
    const d = p === null ? null : this.draft(p.base);
    if (p === null || d === null || p.base !== this.shownId || p.params.change.kind !== "save_custom" || this.screen?.confirm.disabled !== null) return false;
    this.send(d, { ...p.params, change: { ...p.params.change, acknowledged_expansion: p.digest } });
    return true;
  }

  // Said by the view when the shown draft has just become stale (the model cannot see that moment).
  announceStale(): void {
    if ((this.screen?.stale ?? null) !== null) this.model.announce(STALE_ANNOUNCEMENT);
  }

  goBack(): void {
    if (this.panel === null) return;
    this.panel = null;
    this.model.changed();
  }

  activate(): boolean {
    const profiles = this.model.profiles;
    const shown = this.shownProfile();
    if (profiles === null || shown === null || this.screen?.activate.disabled !== null) return false;
    const id = shown.profile_id;
    const name = shown.name;
    this.activating = true;
    this.activateNote = null;
    this.model.command("save_profile", { expected_revision: profiles.revision, change: { kind: "activate", profile_id: id } }, () => {
      this.activating = false;
      this.model.announce(`${name} is now in force for every agent.`);
    }, (f) => {
      this.activating = false;
      let text: string;
      if (f.code === "conflict") {
        text = "Nothing changed: your profiles changed meanwhile. The current version is read again; then try again.";
        this.model.refresh();
      } else if (f.code === null) {
        text = `It is not known whether ${name} was put in force. Trying again is safe.`;
      } else {
        text = `The broker did not put ${name} in force: ${f.text}`;
      }
      this.activateNote = { id, text };
      this.model.announce(text);
    });
    this.model.changed();
    return true;
  }

  // ---- Inside ----

  private shownProfile(): Profile | null {
    const id = this.shownId;
    return this.model.profiles?.items.find((p) => p.profile_id === id) ?? null;
  }

  // The draft of this profile while it differs from the stored profile ("dirty": the name trimmed);
  // a draft equal to it is none. `asTyped`: the name as typed, so that a draft whose name differs
  // only by spaces is still held while the owner types.
  private draft(id: string, asTyped = false): Draft | null {
    const d = this.drafts.get(id);
    const stored = this.model.profiles?.items.find((p) => p.profile_id === id);
    if (d === undefined || stored === undefined) return null;
    const nameDiffers = stored.kind === "custom" && (asTyped ? d.name : d.name.trim()) !== stored.name;
    return nameDiffers || !sameRows(d.rows, stored.rows) ? d : null;
  }

  private shownDraft(): Draft | null {
    const id = this.shownId;
    return id === null ? null : this.draft(id);
  }

  // The draft to edit for the shown profile, made from the stored profile when there is none.
  private edit(): Draft | null {
    const profiles = this.model.profiles;
    const shown = this.shownProfile();
    if (profiles === null || shown === null) return null;
    const held = this.draft(shown.profile_id, true);
    if (held !== null) return held;
    const custom = shown.kind === "custom";
    const d: Draft = {
      base: shown.profile_id, profileId: custom ? shown.profile_id : null, name: custom ? shown.name : "",
      rows: { ...shown.rows }, baseRevision: profiles.revision, note: null, nameNote: null,
    };
    this.drafts.set(shown.profile_id, d);
    return d;
  }

  // After any edit: the expansion and the last note no longer hold; a draft edited back is dropped.
  private edited(d: Draft): void {
    if (this.panel?.base === d.base) this.panel = null;
    d.note = null;
    if (this.draft(d.base, true) === null) this.drafts.delete(d.base);
    this.model.changed();
  }

  private send(d: Draft, params: ParamsSaveProfile): void {
    if (params.change.kind !== "save_custom") return;
    const sent = params.change.profile;
    const known = new Set(this.model.profiles?.items.map((p) => p.profile_id));
    this.saving = d.base;
    d.note = null;
    this.model.command("save_profile", params, (result) => {
      this.saving = null;
      if (this.panel?.base === d.base) this.panel = null;
      // Edits made while the save was awaited stay a draft.
      if (this.drafts.get(d.base) === d && d.name.trim() === sent.name && sameRows(d.rows, sent.rows)) this.drafts.delete(d.base);
      const created = sent.profile_id === null ? result.profiles.items.find((p) => !known.has(p.profile_id)) : undefined;
      if (created !== undefined) this.chosen = created.profile_id;
      this.model.announce(this.savedText(sent.name, sent.profile_id, created, result.profiles));
    }, (f) => {
      this.saving = null;
      if (f.error?.code === "expansion_not_acknowledged") {
        this.panel = {
          base: d.base, digest: f.error.detail.expansion_digest, params,
          lines: f.error.detail.expansion.map((e) => `${ROW_NAME[e.row]}: from ${MODE_NAME[e.from]} to ${MODE_NAME[e.to]}`),
        };
        return;
      }
      if (f.error?.code === "invalid_params" && f.error.detail.pointer === NAME_POINTER) {
        d.nameNote = f.text;
        this.nameAttention += 1;
        this.model.announce(f.text);
        return;
      }
      if (f.code === "conflict") {
        if (this.panel?.base === d.base) this.panel = null;
        d.note = "Nothing was saved: your profiles changed while you were editing. The current version is read again.";
        this.model.refresh();
      } else {
        d.note = f.code === null ? NO_ANSWER : `The broker did not save this: ${f.text}`;
      }
      this.model.announce(d.note);
    });
    this.model.changed();
  }

  private savedText(name: string, id: CustomProfileId | null, created: Profile | undefined, profiles: Profiles): string {
    const active = profiles.items.find((p) => p.profile_id === profiles.active_profile_id)?.name ?? profiles.active_profile_id;
    if (id !== null && id === profiles.active_profile_id) return `${name} is saved. It is in force, so it applies at once to every agent.`;
    const what = created !== undefined || id === null ? `${name} is saved as a custom profile.` : `${name} is saved.`;
    return `${what} ${active} is in force for every agent.`;
  }

  private listItem(p: Profile, profiles: Profiles): ListItem {
    const tags = [p.kind === "built_in" ? "built in" : `custom, version ${p.version}`];
    if (p.profile_id === profiles.active_profile_id) tags.push("in force");
    if (this.draft(p.profile_id) !== null) tags.push("unsaved changes");
    return { id: p.profile_id, name: p.name, tags };
  }
}
