theory Fabric_Broker
  imports Main
begin

section \<open>The rules of the Fabric Calendar permission broker\<close>

text \<open>
  This theory is the rule contract of work package CD-W03 (docs/plans/calendar-desktop-plan.md,
  sections 7 and 8; docs/adr/ADR-108). Before the broker lets an agent's action reach Google, or
  releases a result to an agent, it writes a check theory that imports this one, defines the
  concrete context and action, and asks Isabelle to prove, for every rule, either that it holds
  or that it fails. The outcome follows from those results.

  The outcomes are Permit, Need_approval and Block. A rule of the first group that fails
  blocks the action. Otherwise a rule of the second group that fails sends the request to the
  owner. Otherwise the action is permitted. A check Isabelle cannot finish is undecided, and
  the broker treats it as a refusal; that case is outside this theory.

  The context holds only facts the broker derived from its own records. Nothing an agent or a
  language model said is a fact here.

  Conventions: times are UTC milliseconds since the Unix epoch. The broker's own identifiers
  (accounts, calendars, agents, grants, sessions, recipient sets) are strings. Text that comes
  from outside (addresses, domains, the provider's event ids and revisions) is a list of
  character codes, so it never has to be escaped into HOL syntax.
\<close>

type_synonym chars = "nat list"

subsection \<open>Parameters\<close>

text \<open>A session ends after 30 minutes without activity (CD-M-DUR).\<close>

definition session_idle_timeout_ms :: int where "session_idle_timeout_ms = 30 * 60 * 1000"

subsection \<open>Profiles\<close>

text \<open>
  The rows of the policy matrix (plan section 7, ADR-108 D2), in its order: list calendars and
  read availability; read event details; create in Agent Drafts; create in a selected existing
  calendar; edit an event Fabric created; edit a pre-existing event; send or change
  invitations; respond to an invitation; delete an event; delete a calendar or change sharing.
\<close>

datatype row =
    Row_list
  | Row_read
  | Row_drafts
  | Row_existing
  | Row_edit_fabric
  | Row_edit_other
  | Row_invite
  | Row_respond
  | Row_delete
  | Row_sharing

datatype mode = Blocks | Asks | Allows

text \<open>A profile gives a mode to each row, in the order of the rows above.\<close>

record profile =
  p_list :: mode
  p_read :: mode
  p_drafts :: mode
  p_existing :: mode
  p_edit_fabric :: mode
  p_edit_other :: mode
  p_invite :: mode
  p_respond :: mode
  p_delete :: mode
  p_sharing :: mode

text \<open>
  The mode in force. Two rows are fixed in every profile, whatever the profile says: deleting
  an event asks (CD-Q05a), and deleting a calendar or changing sharing is blocked (CD-M10).
\<close>

fun mode_of :: "profile \<Rightarrow> row \<Rightarrow> mode" where
  "mode_of p Row_list = p_list p"
| "mode_of p Row_read = p_read p"
| "mode_of p Row_drafts = p_drafts p"
| "mode_of p Row_existing = p_existing p"
| "mode_of p Row_edit_fabric = p_edit_fabric p"
| "mode_of p Row_edit_other = p_edit_other p"
| "mode_of p Row_invite = p_invite p"
| "mode_of p Row_respond = p_respond p"
| "mode_of p Row_delete = Asks"
| "mode_of p Row_sharing = Blocks"

text \<open>The built-in profiles, as frozen at gate CD-G0.\<close>

definition careful :: profile where
  "careful = \<lparr>p_list = Allows, p_read = Asks, p_drafts = Allows, p_existing = Asks,
     p_edit_fabric = Asks, p_edit_other = Asks, p_invite = Asks, p_respond = Asks,
     p_delete = Asks, p_sharing = Blocks\<rparr>"

definition flexible :: profile where
  "flexible = \<lparr>p_list = Allows, p_read = Allows, p_drafts = Allows, p_existing = Allows,
     p_edit_fabric = Allows, p_edit_other = Asks, p_invite = Allows, p_respond = Asks,
     p_delete = Asks, p_sharing = Blocks\<rparr>"

subsection \<open>What the broker knows\<close>

datatype tool =
    Tool_list
  | Tool_availability
  | Tool_read
  | Tool_create
  | Tool_update
  | Tool_invite
  | Tool_respond
  | Tool_delete

datatype event_field =
    Field_time
  | Field_title
  | Field_location
  | Field_description
  | Field_attendees
  | Field_organizer
  | Field_own_response

datatype send_updates = Notify_all | Notify_external | Notify_none

datatype response = Accepted | Declined

text \<open>A recipient: the address as written, and its domain in lowercase.\<close>

record recipient =
  rc_address :: chars
  rc_domain :: chars

text \<open>
  A recipient set is a selector: exact addresses, and domains matched by equality. A recipient
  rule says whom a grant covers: exact addresses, or the members of named recipient sets. No
  rule covers everyone.
\<close>

record recipient_set =
  rs_id :: string
  rs_members :: "chars list"
  rs_domains :: "chars list"

datatype recipient_rule = Exact "chars list" | Sets "string list"

text \<open>
  The three durations (CD-M-DUR). A one-request grant is bound to one action digest and is
  used once. A session grant is bound to one session of the agent. A grant until revoked has
  no expiry.
\<close>

datatype duration = One_request string bool | Session string | Until_revoked

text \<open>
  A grant of the agent whose action is checked. g_window, g_fields, g_recipients and
  g_send_updates are the parts of its scope that apply to reads, to invitations and to changes
  that notify; a part that is absent covers nothing.
\<close>

record grant =
  g_id :: string
  g_tools :: "tool list"
  g_duration :: duration
  g_ended :: bool
  g_calendars :: "(string \<times> string) list"
  g_window :: "(int \<times> int) option"
  g_fields :: "event_field list"
  g_recipients :: "recipient_rule option"
  g_send_updates :: "send_updates option"

text \<open>A calendar the owner selected, named with its account.\<close>

record calendar =
  cal_account :: string
  cal_id :: string
  cal_writable :: bool
  cal_drafts :: bool

text \<open>
  The broker's own observation of the event an action names. ev_invitation says that the
  owner is an attendee and not the organizer. ev_attendees are the other people on the event.
  ev_occurrence is set when the event is one occurrence of a recurring event, and holds its
  original start time; ev_id is then the instance id.
\<close>

record event =
  ev_id :: chars
  ev_revision :: chars
  ev_fabric_created :: bool
  ev_recurring :: bool
  ev_default_type :: bool
  ev_timed :: bool
  ev_invitation :: bool
  ev_attendees :: "recipient list"
  ev_occurrence :: "int option"

datatype actor = Agent string | System

text \<open>
  ctx_session is the agent's current session and when it was last active. ctx_digest is the
  action digest of the action being checked; for a disclosure it is the digest of the action
  whose result is disclosed. ctx_relied names the grant that an allowed or approved request
  rests on, once it has one. ctx_pending_effects are the changes the broker has handed to
  Google and not yet seen applied. The two epochs rise with every pause and every revocation,
  so a proof belongs to one state of both.
\<close>

record broker_context =
  ctx_actor :: actor
  ctx_paused :: bool
  ctx_now :: int
  ctx_session :: "(string \<times> int) option"
  ctx_profile :: profile
  ctx_connected :: "string list"
  ctx_calendars :: "calendar list"
  ctx_warning_calendars :: "(string \<times> string) list"
  ctx_grants :: "grant list"
  ctx_recipient_sets :: "recipient_set list"
  ctx_event :: "event option"
  ctx_digest :: string
  ctx_relied :: "string option"
  ctx_result_held :: bool
  ctx_pending_effects :: "(string \<times> string \<times> chars) list"
  ctx_pause_epoch :: nat
  ctx_revocation_epoch :: nat

subsection \<open>Actions\<close>

text \<open>
  What a response answers: an event with its revision, or one occurrence of a recurring
  invitation with its instance id, original start time and revision (CD-Q05b).
\<close>

datatype target = Whole_event chars chars | Occurrence chars int chars

text \<open>
  Arguments, in order. Read_availability: account, calendars, window start and end.
  Read_events: the same, then the fields. Create_event: account, calendar. Update_event,
  Delete_event: account, calendar, event id, revision, who is notified. Change_invitation:
  account, calendar, event id, revision, the complete attendee list after the change, who is
  notified. Respond: account, calendar, target, response, who is notified. Disclose: the action
  whose result is released to the agent. Disclose_warning: a warning released to an agent.
  Delete_calendar, Change_sharing: account, calendar; no agent tool exists for them, and they
  are here so that the theory can say they are always blocked. System_refresh: account,
  calendar. System_reconcile: account, calendar, event id; the broker itself asks for these.
\<close>

datatype action =
    List_calendars
  | Read_availability string "string list" int int
  | Read_events string "string list" int int "event_field list"
  | Create_event string string
  | Update_event string string chars chars send_updates
  | Change_invitation string string chars chars "recipient list" send_updates
  | Respond string string target response send_updates
  | Delete_event string string chars chars send_updates
  | Disclose action
  | Disclose_warning
  | Delete_calendar string string
  | Change_sharing string string
  | System_refresh string string
  | System_reconcile string string chars

fun is_system :: "action \<Rightarrow> bool" where
  "is_system (System_refresh _ _) = True"
| "is_system (System_reconcile _ _ _) = True"
| "is_system _ = False"

fun is_read :: "action \<Rightarrow> bool" where
  "is_read List_calendars = True"
| "is_read (Read_availability _ _ _ _) = True"
| "is_read (Read_events _ _ _ _ _) = True"
| "is_read _ = False"

fun is_change :: "action \<Rightarrow> bool" where
  "is_change (Create_event _ _) = True"
| "is_change (Update_event _ _ _ _ _) = True"
| "is_change (Change_invitation _ _ _ _ _ _) = True"
| "is_change (Respond _ _ _ _ _) = True"
| "is_change (Delete_event _ _ _ _ _) = True"
| "is_change _ = False"

text \<open>The action a check is about: for a disclosure, the action whose result is released.\<close>

fun subject :: "action \<Rightarrow> action" where
  "subject (Disclose a) = a"
| "subject a = a"

text \<open>The calendars an action names, each with its account.\<close>

fun named_calendars :: "action \<Rightarrow> (string \<times> string) list" where
  "named_calendars (Read_availability acct cals _ _) = map (Pair acct) cals"
| "named_calendars (Read_events acct cals _ _ _) = map (Pair acct) cals"
| "named_calendars (Create_event acct cal) = [(acct, cal)]"
| "named_calendars (Update_event acct cal _ _ _) = [(acct, cal)]"
| "named_calendars (Change_invitation acct cal _ _ _ _) = [(acct, cal)]"
| "named_calendars (Respond acct cal _ _ _) = [(acct, cal)]"
| "named_calendars (Delete_event acct cal _ _ _) = [(acct, cal)]"
| "named_calendars (Delete_calendar acct cal) = [(acct, cal)]"
| "named_calendars (Change_sharing acct cal) = [(acct, cal)]"
| "named_calendars (System_refresh acct cal) = [(acct, cal)]"
| "named_calendars (System_reconcile acct cal _) = [(acct, cal)]"
| "named_calendars _ = []"

text \<open>The event an action names, with the revision it was written against.\<close>

fun named_event :: "action \<Rightarrow> (chars \<times> chars) option" where
  "named_event (Update_event _ _ e r _) = Some (e, r)"
| "named_event (Change_invitation _ _ e r _ _) = Some (e, r)"
| "named_event (Delete_event _ _ e r _) = Some (e, r)"
| "named_event (Respond _ _ (Whole_event e r) _ _) = Some (e, r)"
| "named_event (Respond _ _ (Occurrence i _ r) _ _) = Some (i, r)"
| "named_event _ = None"

subsection \<open>Conditions used by the rules\<close>

definition selected :: "broker_context \<Rightarrow> string \<times> string \<Rightarrow> bool" where
  "selected c k \<longleftrightarrow> (\<exists>x \<in> set (ctx_calendars c). (cal_account x, cal_id x) = k)"

definition writable :: "broker_context \<Rightarrow> string \<times> string \<Rightarrow> bool" where
  "writable c k \<longleftrightarrow> (\<exists>x \<in> set (ctx_calendars c). (cal_account x, cal_id x) = k \<and> cal_writable x)"

definition drafts :: "broker_context \<Rightarrow> string \<times> string \<Rightarrow> bool" where
  "drafts c k \<longleftrightarrow> (\<exists>x \<in> set (ctx_calendars c). (cal_account x, cal_id x) = k \<and> cal_drafts x)"

text \<open>Whether every calendar the action names is an Agent Drafts calendar.\<close>

definition in_drafts :: "broker_context \<Rightarrow> action \<Rightarrow> bool" where
  "in_drafts c a \<longleftrightarrow> named_calendars a \<noteq> [] \<and> (\<forall>k \<in> set (named_calendars a). drafts c k)"

definition fabric_created :: "broker_context \<Rightarrow> bool" where
  "fabric_created c \<longleftrightarrow> (case ctx_event c of Some e \<Rightarrow> ev_fabric_created e | None \<Rightarrow> False)"

text \<open>The other people on the event the action names, as the broker last saw it.\<close>

definition attendees :: "broker_context \<Rightarrow> recipient list" where
  "attendees c = (case ctx_event c of Some e \<Rightarrow> ev_attendees e | None \<Rightarrow> [])"

text \<open>
  The people a change reaches. A change to the attendee list reaches the new list and the
  people already on the event. An edit reaches the people already on the event: they see the
  new time or text whether or not a notice is sent. This is how the row for editing combines
  with the row for invitations (CD-M05 with CD-M07).
\<close>

fun reached :: "broker_context \<Rightarrow> action \<Rightarrow> recipient list" where
  "reached c (Change_invitation _ _ _ _ rs _) = rs @ attendees c"
| "reached c (Update_event _ _ _ _ _) = attendees c"
| "reached c _ = []"

text \<open>
  The rows an action falls under. An edit of an event that has attendees falls under the row
  for invitations as well. Releasing the result of a read falls under the read's row;
  releasing the outcome of a change falls under no row.
\<close>

fun rows :: "broker_context \<Rightarrow> action \<Rightarrow> row list" where
  "rows c List_calendars = [Row_list]"
| "rows c (Read_availability _ _ _ _) = [Row_list]"
| "rows c (Read_events _ _ _ _ _) = [Row_read]"
| "rows c (Create_event acct cal) = [if drafts c (acct, cal) then Row_drafts else Row_existing]"
| "rows c (Update_event _ _ _ _ _) =
     (if fabric_created c then Row_edit_fabric else Row_edit_other) #
     (if attendees c = [] then [] else [Row_invite])"
| "rows c (Change_invitation _ _ _ _ _ _) = [Row_invite]"
| "rows c (Respond _ _ _ _ _) = [Row_respond]"
| "rows c (Delete_event _ _ _ _ _) = [Row_delete]"
| "rows c (Delete_calendar _ _) = [Row_sharing]"
| "rows c (Change_sharing _ _) = [Row_sharing]"
| "rows c (Disclose a) = (if is_read a then rows c a else [])"
| "rows c _ = []"

text \<open>
  A session is live while it is the agent's current session and was active less than the idle
  timeout ago. A grant is live while it has not ended, its one use is not spent, and its
  session is live.
\<close>

definition session_live :: "broker_context \<Rightarrow> string \<Rightarrow> bool" where
  "session_live c s \<longleftrightarrow>
     (case ctx_session c of
        Some (current, active) \<Rightarrow> current = s \<and> ctx_now c < active + session_idle_timeout_ms
      | None \<Rightarrow> False)"

definition live :: "broker_context \<Rightarrow> grant \<Rightarrow> bool" where
  "live c g \<longleftrightarrow>
     \<not> g_ended g \<and>
     (case g_duration g of
        One_request _ used \<Rightarrow> \<not> used
      | Session s \<Rightarrow> session_live c s
      | Until_revoked \<Rightarrow> True)"

text \<open>The owner approved exactly this action, once.\<close>

definition exact :: "broker_context \<Rightarrow> grant \<Rightarrow> bool" where
  "exact c g \<longleftrightarrow> (case g_duration g of One_request d _ \<Rightarrow> d = ctx_digest c | _ \<Rightarrow> False)"

definition standing :: "grant \<Rightarrow> bool" where
  "standing g \<longleftrightarrow> (case g_duration g of One_request _ _ \<Rightarrow> False | _ \<Rightarrow> True)"

definition covers_calendars :: "grant \<Rightarrow> (string \<times> string) list \<Rightarrow> bool" where
  "covers_calendars g ks \<longleftrightarrow> (\<forall>k \<in> set ks. k \<in> set (g_calendars g))"

definition covers_window :: "grant \<Rightarrow> int \<Rightarrow> int \<Rightarrow> bool" where
  "covers_window g s e \<longleftrightarrow> (case g_window g of Some (lo, hi) \<Rightarrow> lo \<le> s \<and> e \<le> hi | None \<Rightarrow> False)"

definition covers_fields :: "grant \<Rightarrow> event_field list \<Rightarrow> bool" where
  "covers_fields g fs \<longleftrightarrow> (\<forall>f \<in> set fs. f \<in> set (g_fields g))"

definition in_rule :: "broker_context \<Rightarrow> recipient_rule \<Rightarrow> recipient \<Rightarrow> bool" where
  "in_rule c allowed r \<longleftrightarrow>
     (case allowed of
        Exact addresses \<Rightarrow> rc_address r \<in> set addresses
      | Sets ids \<Rightarrow>
          (\<exists>s \<in> set (ctx_recipient_sets c).
             rs_id s \<in> set ids \<and> (rc_address r \<in> set (rs_members s) \<or> rc_domain r \<in> set (rs_domains s))))"

definition covers_recipients :: "broker_context \<Rightarrow> grant \<Rightarrow> recipient list \<Rightarrow> bool" where
  "covers_recipients c g rs \<longleftrightarrow>
     (case g_recipients g of Some allowed \<Rightarrow> (\<forall>r \<in> set rs. in_rule c allowed r) | None \<Rightarrow> False)"

text \<open>
  Whether a standing grant covers an action for one of its rows. No standing grant covers a
  deletion. For an edit, the row for invitations needs a grant that names the invitation tool
  and covers the people the edit reaches.
\<close>

fun scope_covers :: "broker_context \<Rightarrow> grant \<Rightarrow> row \<Rightarrow> action \<Rightarrow> bool" where
  "scope_covers c g r List_calendars \<longleftrightarrow> Tool_list \<in> set (g_tools g)"
| "scope_covers c g r (Read_availability acct cals s e) \<longleftrightarrow>
     Tool_availability \<in> set (g_tools g) \<and> covers_calendars g (map (Pair acct) cals) \<and>
     covers_window g s e"
| "scope_covers c g r (Read_events acct cals s e fs) \<longleftrightarrow>
     Tool_read \<in> set (g_tools g) \<and> covers_calendars g (map (Pair acct) cals) \<and>
     covers_window g s e \<and> covers_fields g fs"
| "scope_covers c g r (Create_event acct cal) \<longleftrightarrow>
     Tool_create \<in> set (g_tools g) \<and> covers_calendars g [(acct, cal)]"
| "scope_covers c g r (Update_event acct cal ev rv su) \<longleftrightarrow>
     covers_calendars g [(acct, cal)] \<and>
     (if r = Row_invite
      then Tool_invite \<in> set (g_tools g) \<and> covers_recipients c g (attendees c) \<and>
           g_send_updates g = Some su
      else Tool_update \<in> set (g_tools g))"
| "scope_covers c g r (Change_invitation acct cal ev rv rs su) \<longleftrightarrow>
     Tool_invite \<in> set (g_tools g) \<and> covers_calendars g [(acct, cal)] \<and>
     covers_recipients c g (rs @ attendees c) \<and> g_send_updates g = Some su"
| "scope_covers c g r (Respond acct cal t resp su) \<longleftrightarrow>
     Tool_respond \<in> set (g_tools g) \<and> covers_calendars g [(acct, cal)] \<and>
     g_send_updates g = Some su"
| "scope_covers c g r _ \<longleftrightarrow> False"

text \<open>
  Whether a standing grant may serve a row at all. Under Allows it may. Under Asks every
  request needs the owner's own approval, with two exceptions that the frozen matrix states:
  a read of event details runs inside the grant the owner approved for it (CD-M02), and an
  edit of an event Fabric created asks only outside Agent Drafts (CD-M05).
\<close>

definition standing_serves :: "broker_context \<Rightarrow> action \<Rightarrow> row \<Rightarrow> bool" where
  "standing_serves c a r \<longleftrightarrow>
     (case mode_of (ctx_profile c) r of
        Allows \<Rightarrow> True
      | Asks \<Rightarrow> r = Row_read \<or> (r = Row_edit_fabric \<and> in_drafts c a)
      | Blocks \<Rightarrow> False)"

text \<open>
  A row is authorized when a live grant of the agent either is the owner's approval of exactly
  this action, or is a standing grant that may serve the row and covers the action.
\<close>

definition row_authorized :: "broker_context \<Rightarrow> action \<Rightarrow> row \<Rightarrow> bool" where
  "row_authorized c a r \<longleftrightarrow>
     (\<exists>g \<in> set (ctx_grants c).
        live c g \<and> (exact c g \<or> (standing g \<and> standing_serves c a r \<and> scope_covers c g r a)))"

text \<open>A recipient is approved when the recipient rule of some live grant names it.\<close>

definition approved_recipient :: "broker_context \<Rightarrow> recipient \<Rightarrow> bool" where
  "approved_recipient c r \<longleftrightarrow>
     (\<exists>g \<in> set (ctx_grants c).
        live c g \<and> (case g_recipients g of Some allowed \<Rightarrow> in_rule c allowed r | None \<Rightarrow> False))"

text \<open>
  Whether the event the action names can be changed in version 1: the default event type, a
  timed event, and not recurring. The one exception is a response to one occurrence of a
  recurring invitation (CD-Q05b, CD-Q05c, CD-Q05e).
\<close>

fun event_supported :: "event \<Rightarrow> action \<Rightarrow> bool" where
  "event_supported e (Respond _ _ (Occurrence _ _ _) _ _) \<longleftrightarrow>
     ev_default_type e \<and> ev_timed e \<and> ev_occurrence e \<noteq> None"
| "event_supported e _ \<longleftrightarrow>
     ev_default_type e \<and> ev_timed e \<and> \<not> ev_recurring e \<and> ev_occurrence e = None"

fun invitation_target :: "event \<Rightarrow> action \<Rightarrow> bool" where
  "invitation_target e (Respond _ _ (Occurrence _ start _) _ _) \<longleftrightarrow>
     ev_invitation e \<and> ev_occurrence e = Some start"
| "invitation_target e _ \<longleftrightarrow> ev_invitation e"

text \<open>What the broker may do on its own authority.\<close>

fun system_authorized :: "broker_context \<Rightarrow> action \<Rightarrow> bool" where
  "system_authorized c (System_refresh acct cal) \<longleftrightarrow>
     selected c (acct, cal) \<or> (acct, cal) \<in> set (ctx_warning_calendars c)"
| "system_authorized c (System_reconcile acct cal ev) \<longleftrightarrow>
     (acct, cal, ev) \<in> set (ctx_pending_effects c)"
| "system_authorized c _ \<longleftrightarrow> False"

subsection \<open>The rules\<close>

datatype rule =
    Actor
  | Agent_not_paused
  | Mode_not_blocked
  | Account_connected
  | Calendar_selected
  | Calendar_writable
  | Event_known
  | Revision_current
  | Event_supported
  | Fabric_created
  | Invitation_target
  | Relied_grant_live
  | Result_held
  | Warnings_owner_only
  | System_authority
  | No_strangers
  | Invitation_authority
  | Authority

text \<open>A rule of this list that fails blocks the action.\<close>

definition block_rules :: "rule list" where
  "block_rules =
     [Actor, Agent_not_paused, Mode_not_blocked, Account_connected, Calendar_selected,
      Calendar_writable, Event_known, Revision_current, Event_supported, Fabric_created,
      Invitation_target, Relied_grant_live, Result_held, Warnings_owner_only, System_authority]"

text \<open>
  A rule of this list that fails, when no rule above fails, sends the request to the owner.
\<close>

definition approval_rules :: "rule list" where
  "approval_rules = [No_strangers, Invitation_authority, Authority]"

text \<open>Which actions each rule governs.\<close>

fun applies :: "rule \<Rightarrow> broker_context \<Rightarrow> action \<Rightarrow> bool" where
  "applies Actor c a = True"
| "applies Agent_not_paused c a = (\<not> is_system a)"
| "applies Mode_not_blocked c a = True"
| "applies Account_connected c a = (named_calendars (subject a) \<noteq> [])"
| "applies Calendar_selected c a = (\<not> is_system a)"
| "applies Calendar_writable c a = is_change a"
| "applies Event_known c a = (named_event a \<noteq> None)"
| "applies Revision_current c a = (named_event a \<noteq> None)"
| "applies Event_supported c a = (named_event a \<noteq> None)"
| "applies Fabric_created c a = (case a of Delete_event _ _ _ _ _ \<Rightarrow> True | _ \<Rightarrow> False)"
| "applies Invitation_target c a = (case a of Respond _ _ _ _ _ \<Rightarrow> True | _ \<Rightarrow> False)"
| "applies Relied_grant_live c a = (ctx_relied c \<noteq> None)"
| "applies Result_held c a = (case a of Disclose _ \<Rightarrow> True | _ \<Rightarrow> False)"
| "applies Warnings_owner_only c a = (case a of Disclose_warning \<Rightarrow> True | _ \<Rightarrow> False)"
| "applies System_authority c a = is_system a"
| "applies No_strangers c a = (Row_invite \<in> set (rows c a))"
| "applies Invitation_authority c a = (Row_invite \<in> set (rows c a))"
| "applies Authority c a = (\<not> is_system a)"

text \<open>What each rule requires of an action it governs.\<close>

fun satisfied :: "rule \<Rightarrow> broker_context \<Rightarrow> action \<Rightarrow> bool" where
  "satisfied Actor c a = (if is_system a then ctx_actor c = System else ctx_actor c \<noteq> System)"
| "satisfied Agent_not_paused c a = (\<not> ctx_paused c)"
| "satisfied Mode_not_blocked c a = (\<forall>r \<in> set (rows c a). mode_of (ctx_profile c) r \<noteq> Blocks)"
| "satisfied Account_connected c a =
     (\<forall>k \<in> set (named_calendars (subject a)). fst k \<in> set (ctx_connected c))"
| "satisfied Calendar_selected c a = (\<forall>k \<in> set (named_calendars (subject a)). selected c k)"
| "satisfied Calendar_writable c a = (\<forall>k \<in> set (named_calendars a). writable c k)"
| "satisfied Event_known c a =
     (case (ctx_event c, named_event a) of (Some e, Some (i, _)) \<Rightarrow> ev_id e = i | _ \<Rightarrow> False)"
| "satisfied Revision_current c a =
     (case (ctx_event c, named_event a) of (Some e, Some (_, r)) \<Rightarrow> ev_revision e = r | _ \<Rightarrow> False)"
| "satisfied Event_supported c a =
     (case ctx_event c of Some e \<Rightarrow> event_supported e a | None \<Rightarrow> False)"
| "satisfied Fabric_created c a = fabric_created c"
| "satisfied Invitation_target c a =
     (case ctx_event c of Some e \<Rightarrow> invitation_target e a | None \<Rightarrow> False)"
| "satisfied Relied_grant_live c a =
     (case ctx_relied c of
        Some i \<Rightarrow> (\<exists>g \<in> set (ctx_grants c). g_id g = i \<and> live c g)
      | None \<Rightarrow> True)"
| "satisfied Result_held c a =
     (ctx_result_held c \<and> (case a of Disclose b \<Rightarrow> is_read b \<or> is_change b | _ \<Rightarrow> True))"
| "satisfied Warnings_owner_only c a = False"
| "satisfied System_authority c a = system_authorized c a"
| "satisfied No_strangers c a = (\<forall>r \<in> set (reached c (subject a)). approved_recipient c r)"
| "satisfied Invitation_authority c a = row_authorized c (subject a) Row_invite"
| "satisfied Authority c a =
     (\<forall>r \<in> set (rows c a). r = Row_invite \<or> row_authorized c (subject a) r)"

definition holds :: "rule \<Rightarrow> broker_context \<Rightarrow> action \<Rightarrow> bool" where
  "holds r c a \<longleftrightarrow> (applies r c a \<longrightarrow> satisfied r c a)"

subsection \<open>The outcome\<close>

datatype outcome = Permit | Need_approval | Block

definition outcome :: "broker_context \<Rightarrow> action \<Rightarrow> outcome" where
  "outcome c a =
     (if \<exists>r \<in> set block_rules. \<not> holds r c a then Block
      else if \<exists>r \<in> set approval_rules. \<not> holds r c a then Need_approval
      else Permit)"

text \<open>The simplification rules every check uses.\<close>

lemmas broker_simps =
  holds_def outcome_def block_rules_def approval_rules_def session_idle_timeout_ms_def
  selected_def writable_def drafts_def in_drafts_def fabric_created_def attendees_def
  session_live_def live_def exact_def standing_def covers_calendars_def covers_window_def
  covers_fields_def in_rule_def covers_recipients_def standing_serves_def row_authorized_def
  approved_recipient_def careful_def flexible_def

subsection \<open>What the rules guarantee\<close>

text \<open>
  Properties of the rules themselves, proven once when the session builds. Each is one of the
  decisions of gate CD-G0, stated for every context and so for every profile, custom ones
  included.
\<close>

lemma permit_iff:
  "outcome c a = Permit \<longleftrightarrow> (\<forall>r \<in> set block_rules. holds r c a) \<and> (\<forall>r \<in> set approval_rules. holds r c a)"
  by (auto simp add: outcome_def)

lemma block_iff: "outcome c a = Block \<longleftrightarrow> (\<exists>r \<in> set block_rules. \<not> holds r c a)"
  by (auto simp add: outcome_def)

lemma need_approval_iff:
  "outcome c a = Need_approval \<longleftrightarrow>
     (\<forall>r \<in> set block_rules. holds r c a) \<and> (\<exists>r \<in> set approval_rules. \<not> holds r c a)"
  by (auto simp add: outcome_def)

text \<open>
  These three lemmas are why a check proves each rule and not the outcome. A check theory
  proves, for the concrete context and action, every rule or its negation. The outcome then
  follows from those eighteen results by the lemmas above, which hold for every context.
\<close>

text \<open>The two fixed rows (CD-Q05a, CD-M10): no profile changes them.\<close>

lemma delete_row_always_asks: "mode_of p Row_delete = Asks"
  by simp

lemma sharing_row_always_blocks: "mode_of p Row_sharing = Blocks"
  by simp

lemma sharing_is_always_blocked:
  "outcome c (Delete_calendar acct cal) = Block" "outcome c (Change_sharing acct cal) = Block"
  by (simp_all add: block_iff block_rules_def holds_def)

text \<open>Warnings are for the owner only (G0-WARNDISC).\<close>

lemma warnings_are_never_disclosed: "outcome c Disclose_warning = Block"
  by (simp add: block_iff block_rules_def holds_def)

text \<open>A paused agent can do nothing, and the broker's own reads are not an agent's to ask for.\<close>

lemma paused_agent_is_blocked:
  assumes "ctx_paused c" and "\<not> is_system a"
  shows "outcome c a = Block"
  using assms by (auto simp add: block_iff block_rules_def holds_def)

lemma agents_cannot_run_system_actions:
  assumes "is_system a" and "ctx_actor c \<noteq> System"
  shows "outcome c a = Block"
  using assms by (auto simp add: block_iff block_rules_def holds_def)

lemma the_broker_cannot_act_as_an_agent:
  assumes "\<not> is_system a" and "ctx_actor c = System"
  shows "outcome c a = Block"
  using assms by (auto simp add: block_iff block_rules_def holds_def)

text \<open>
  Every permitted read or change rests on a live grant of the agent: nothing is permitted by
  a profile alone.
\<close>

lemma authority_gives_a_live_grant:
  assumes "holds Authority c a" and "\<not> is_system a" and "rows c a \<noteq> []"
    and "Row_invite \<notin> set (rows c a)"
  shows "\<exists>g \<in> set (ctx_grants c). live c g"
proof -
  obtain r where "r \<in> set (rows c a)"
    using assms(3) by (cases "rows c a") auto
  then have "row_authorized c (subject a) r"
    using assms by (auto simp add: holds_def)
  then show ?thesis
    by (auto simp add: row_authorized_def)
qed

lemma invitation_authority_gives_a_live_grant:
  assumes "holds Invitation_authority c a" and "Row_invite \<in> set (rows c a)"
  shows "\<exists>g \<in> set (ctx_grants c). live c g"
  using assms by (auto simp add: holds_def row_authorized_def)

lemma permit_rests_on_a_live_grant:
  assumes "outcome c a = Permit" and "\<not> is_system a" and "rows c a \<noteq> []"
  shows "\<exists>g \<in> set (ctx_grants c). live c g"
proof (cases "Row_invite \<in> set (rows c a)")
  case True
  have "holds Invitation_authority c a"
    using assms(1) by (simp add: permit_iff approval_rules_def)
  then show ?thesis
    using True by (rule invitation_authority_gives_a_live_grant)
next
  case False
  have "holds Authority c a"
    using assms(1) by (simp add: permit_iff approval_rules_def)
  then show ?thesis
    using assms(2,3) False by (rule authority_gives_a_live_grant)
qed

text \<open>
  Deleting an event (CD-Q05a): only an event Fabric created, and only with the owner's
  approval of exactly this deletion. No standing grant and no profile permits it.
\<close>

lemma delete_needs_a_fabric_created_event:
  assumes "outcome c (Delete_event acct cal ev rv su) = Permit"
  shows "fabric_created c"
  using assms by (simp add: permit_iff block_rules_def holds_def)

lemma delete_needs_the_owners_exact_approval:
  assumes "outcome c (Delete_event acct cal ev rv su) = Permit"
  shows "\<exists>g \<in> set (ctx_grants c). live c g \<and> exact c g"
  using assms
  by (auto simp add: permit_iff approval_rules_def holds_def row_authorized_def)

text \<open>
  Strangers always ask (G0-STRANGERS): an invitation is permitted only when every person it
  reaches is named by the recipient rule of a live grant. This holds under every profile.
\<close>

lemma strangers_always_ask:
  assumes "outcome c (Change_invitation acct cal ev rv rs su) = Permit"
  shows "\<forall>r \<in> set rs. approved_recipient c r"
  using assms by (simp add: permit_iff approval_rules_def holds_def)

lemma a_stranger_is_never_permitted:
  assumes "r \<in> set rs" and "\<not> approved_recipient c r"
  shows "outcome c (Change_invitation acct cal ev rv rs su) \<noteq> Permit"
  using assms strangers_always_ask by blast

text \<open>
  The row for editing combined with the row for invitations (CD-M05 with CD-M07): an edit of
  an event that has attendees is permitted only when the invitation row is authorized too, and
  only when every attendee is an approved recipient. A grant to edit is not enough.
\<close>

lemma an_edit_that_reaches_attendees_needs_invitation_authority:
  assumes "outcome c (Update_event acct cal ev rv su) = Permit" and "attendees c \<noteq> []"
  shows "row_authorized c (Update_event acct cal ev rv su) Row_invite"
    and "\<forall>r \<in> set (attendees c). approved_recipient c r"
  using assms by (simp_all add: permit_iff approval_rules_def holds_def)

text \<open>
  A read of event details (CD-M02) is permitted only inside a live grant: either the owner's
  approval of exactly this read, or a standing grant that names the read tool and covers its
  calendars, its window and its fields.
\<close>

lemma a_read_stays_inside_its_grant:
  assumes "outcome c (Read_events acct cals s e fs) = Permit"
  shows "\<exists>g \<in> set (ctx_grants c). live c g \<and>
           (exact c g \<or>
            (Tool_read \<in> set (g_tools g) \<and> covers_calendars g (map (Pair acct) cals) \<and>
             covers_window g s e \<and> covers_fields g fs))"
  using assms
  by (auto simp add: permit_iff approval_rules_def holds_def row_authorized_def)

text \<open>
  A recurring event is never changed, except by a response to one occurrence that names the
  occurrence's own original start time (CD-Q05b).
\<close>

lemma recurring_events_are_not_edited:
  assumes "ctx_event c = Some e" and "ev_recurring e \<or> ev_occurrence e \<noteq> None"
  shows "outcome c (Update_event acct cal ev rv su) = Block"
    and "outcome c (Change_invitation acct cal ev rv rs su) = Block"
    and "outcome c (Delete_event acct cal ev rv su) = Block"
  using assms by (auto simp add: block_iff block_rules_def holds_def)

lemma a_response_to_an_occurrence_names_it:
  assumes "outcome c (Respond acct cal (Occurrence i start rv) resp su) = Permit"
  shows "\<exists>e. ctx_event c = Some e \<and> ev_id e = i \<and> ev_revision e = rv \<and>
           ev_occurrence e = Some start \<and> ev_invitation e"
  using assms
  by (auto simp add: permit_iff block_rules_def holds_def split: option.splits)

text \<open>
  A change is made against the revision the broker last saw: an action written against an
  older revision is blocked, and the request is then stale.
\<close>

lemma a_change_names_the_current_revision:
  assumes "outcome c a = Permit" and "named_event a = Some (i, r)"
  shows "\<exists>e. ctx_event c = Some e \<and> ev_id e = i \<and> ev_revision e = r"
  using assms
  by (auto simp add: permit_iff block_rules_def holds_def split: option.splits)

text \<open>
  Revocation and expiry: once the grant a request rests on is no longer live, the request is
  blocked, even if another grant would cover the action.
\<close>

lemma a_request_does_not_outlive_its_grant:
  assumes "ctx_relied c = Some i" and "\<forall>g \<in> set (ctx_grants c). g_id g = i \<longrightarrow> \<not> live c g"
  shows "outcome c a = Block"
  using assms by (auto simp add: block_iff block_rules_def holds_def)

lemma an_ended_grant_is_not_live: "g_ended g \<Longrightarrow> \<not> live c g"
  by (simp add: live_def)

lemma a_session_grant_ends_with_its_session:
  assumes "g_duration g = Session s"
    and "ctx_session c = None \<or> (\<exists>other active. ctx_session c = Some (other, active) \<and> other \<noteq> s)"
  shows "\<not> live c g"
  using assms by (auto simp add: live_def session_live_def)

lemma a_session_grant_ends_after_the_idle_timeout:
  assumes "g_duration g = Session s" and "ctx_session c = Some (s, active)"
    and "active + 30 * 60 * 1000 \<le> ctx_now c"
  shows "\<not> live c g"
  using assms by (simp add: live_def session_live_def session_idle_timeout_ms_def)

lemma a_one_request_grant_is_used_once:
  assumes "g_duration g = One_request d True"
  shows "\<not> live c g"
  using assms by (simp add: live_def)

text \<open>Releasing a result needs the result, and releasing a read needs the read's authority.\<close>

lemma a_result_is_released_only_when_held:
  assumes "outcome c (Disclose a) = Permit"
  shows "ctx_result_held c"
  using assms by (simp add: permit_iff block_rules_def holds_def)

lemma releasing_a_read_needs_its_authority:
  assumes "outcome c (Disclose a) = Permit" and "is_read a" and "r \<in> set (rows c a)"
  shows "row_authorized c a r"
proof -
  have invite: "r \<noteq> Row_invite"
    using assms(2,3) by (cases a) auto
  have "holds Authority c (Disclose a)"
    using assms(1) by (simp add: permit_iff approval_rules_def)
  then show ?thesis
    using assms(2,3) invite by (auto simp add: holds_def)
qed

text \<open>The built-in profiles are the frozen matrix.\<close>

lemma careful_is_the_frozen_matrix:
  "map (mode_of careful)
     [Row_list, Row_read, Row_drafts, Row_existing, Row_edit_fabric, Row_edit_other, Row_invite,
      Row_respond, Row_delete, Row_sharing] =
   [Allows, Asks, Allows, Asks, Asks, Asks, Asks, Asks, Asks, Blocks]"
  by (simp add: careful_def)

lemma flexible_is_the_frozen_matrix:
  "map (mode_of flexible)
     [Row_list, Row_read, Row_drafts, Row_existing, Row_edit_fabric, Row_edit_other, Row_invite,
      Row_respond, Row_delete, Row_sharing] =
   [Allows, Allows, Allows, Allows, Allows, Asks, Allows, Asks, Asks, Blocks]"
  by (simp add: flexible_def)

end
