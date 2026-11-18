theory Broker_Examples
  imports "Fabric_Broker.Fabric_Broker"
begin

section \<open>Worked cases of the broker's rules\<close>

text \<open>
  Each lemma is one concrete situation with the outcome the frozen matrix requires
  (plan section 7, ADR-108 D2), proven when the session builds. A case that is not permitted
  also names the first rule that fails, since that is the rule the interface shows. The cases
  are written by hand, apart from the Rust code that renders real contexts, so they also say
  what that code must produce.

  Accounts, calendars, grants and digests are synthetic. Character-code lists stand for
  addresses and for the provider's ids; their content does not matter, only whether two of
  them are equal.
\<close>

subsection \<open>A base situation\<close>

text \<open>
  One connected account with four selected calendars: Agent Drafts, the owner's own calendar, a
  shared calendar the owner may write, and one the owner may only read. The agent has a live
  session and no grant. The time is 1000 ms after the session was last active.
\<close>

definition cals :: "calendar list" where
  "cals =
     [\<lparr>cal_account = ''acct-1'', cal_id = ''drafts'', cal_writable = True, cal_drafts = True\<rparr>,
      \<lparr>cal_account = ''acct-1'', cal_id = ''primary'', cal_writable = True, cal_drafts = False\<rparr>,
      \<lparr>cal_account = ''acct-1'', cal_id = ''team'', cal_writable = True, cal_drafts = False\<rparr>,
      \<lparr>cal_account = ''acct-1'', cal_id = ''readonly'', cal_writable = False, cal_drafts = False\<rparr>]"

definition base :: broker_context where
  "base =
     \<lparr>ctx_actor = Agent ''agent-1'', ctx_paused = False, ctx_now = 1000,
      ctx_session = Some (''session-1'', 0), ctx_profile = careful, ctx_connected = [''acct-1''],
      ctx_calendars = cals, ctx_warning_calendars = [], ctx_grants = [], ctx_recipient_sets = [],
      ctx_event = None, ctx_digest = ''d-this'', ctx_relied = [], ctx_result_held = False,
      ctx_pending_effects = [], ctx_pause_epoch = 0, ctx_revocation_epoch = 0\<rparr>"

text \<open>The owner's approval of exactly the action being checked, not yet used.\<close>

definition approval :: "grant" where
  "approval =
     \<lparr>g_id = ''g-approval'', g_tools = [], g_duration = One_request ''d-this'' False,
      g_ended = False, g_calendars = [], g_window = None, g_fields = [], g_recipients = None,
      g_send_updates = None\<rparr>"

text \<open>A standing grant with an empty scope; the cases fill in what they need.\<close>

definition standing_grant :: "grant" where
  "standing_grant =
     \<lparr>g_id = ''g-standing'', g_tools = [], g_duration = Until_revoked, g_ended = False,
      g_calendars = [], g_window = None, g_fields = [], g_recipients = None,
      g_send_updates = None\<rparr>"

text \<open>People: Bob and Carol are in the recipient set "team"; Dan is in no set.\<close>

definition bob :: recipient where "bob = \<lparr>rc_address = [1], rc_domain = [10]\<rparr>"
definition carol :: recipient where "carol = \<lparr>rc_address = [2], rc_domain = [10]\<rparr>"
definition dan :: recipient where "dan = \<lparr>rc_address = [3], rc_domain = [11]\<rparr>"

definition team :: recipient_set where
  "team = \<lparr>rs_id = ''set-team'', rs_members = [[1], [2]], rs_domains = []\<rparr>"

text \<open>An ordinary event Fabric created, with no attendees, at revision [7].\<close>

definition own_event :: event where
  "own_event =
     \<lparr>ev_id = [100], ev_revision = [7], ev_fabric_created = True, ev_recurring = False,
      ev_default_type = True, ev_timed = True, ev_invitation = False, ev_attendees = [],
      ev_occurrence = None\<rparr>"

text \<open>A profile no built-in one allows: every row a custom profile may set is set to Allows.\<close>

definition widest :: profile where
  "widest = \<lparr>p_list = Allows, p_read = Allows, p_drafts = Allows, p_existing = Allows,
     p_edit_fabric = Allows, p_edit_other = Allows, p_invite = Allows, p_respond = Allows,
     p_delete = Allows, p_sharing = Allows\<rparr>"

lemmas example_defs =
  cals_def base_def approval_def standing_grant_def bob_def carol_def dan_def team_def
  own_event_def widest_def

text \<open>
  How each case is proven: the outcome by simplification over the definitions, and the failing
  rule by showing that it fails while the rules before it hold.
\<close>

definition first_failing :: "broker_context \<Rightarrow> action \<Rightarrow> rule option" where
  "first_failing c a = (case filter (\<lambda>r. \<not> holds r c a) (block_rules @ approval_rules) of [] \<Rightarrow> None | r # _ \<Rightarrow> Some r)"

subsection \<open>Reading (CD-M01, CD-M02)\<close>

lemma availability_without_a_grant_asks:
  "outcome base (Read_availability ''acct-1'' [''primary''] 0 100) = Need_approval"
  "first_failing base (Read_availability ''acct-1'' [''primary''] 0 100) = Some Authority"
  by (simp_all add: broker_simps example_defs first_failing_def)

definition availability_grant :: "grant" where
  "availability_grant = standing_grant\<lparr>g_id := ''g-availability'',
     g_tools := [Tool_list, Tool_availability],
     g_calendars := [(''acct-1'', ''primary'')], g_window := Some (0, 1000)\<rparr>"

lemma availability_inside_a_grant_is_permitted:
  "outcome (base\<lparr>ctx_grants := [availability_grant]\<rparr>)
     (Read_availability ''acct-1'' [''primary''] 0 100) = Permit"
  "outcome (base\<lparr>ctx_grants := [availability_grant]\<rparr>) List_calendars = Permit"
  by (simp_all add: broker_simps example_defs availability_grant_def)

lemma availability_outside_the_grants_window_or_calendars_asks:
  "outcome (base\<lparr>ctx_grants := [availability_grant]\<rparr>)
     (Read_availability ''acct-1'' [''primary''] 0 1001) = Need_approval"
  "outcome (base\<lparr>ctx_grants := [availability_grant]\<rparr>)
     (Read_availability ''acct-1'' [''primary'', ''team''] 0 100) = Need_approval"
  by (simp_all add: broker_simps example_defs availability_grant_def)

text \<open>An availability grant does not let the agent read event details.\<close>

lemma an_availability_grant_does_not_cover_event_details:
  "outcome (base\<lparr>ctx_grants := [availability_grant]\<rparr>)
     (Read_events ''acct-1'' [''primary''] 0 100 [Field_time]) = Need_approval"
  by (simp add: broker_simps example_defs availability_grant_def)

definition read_grant :: "grant" where
  "read_grant = standing_grant\<lparr>g_id := ''g-read'', g_tools := [Tool_read],
     g_duration := Session ''session-1'',
     g_calendars := [(''acct-1'', ''primary'')], g_window := Some (0, 1000),
     g_fields := [Field_time, Field_title]\<rparr>"

text \<open>
  CD-M02 under Careful: once the owner approved a grant for fields and a window, reads inside
  it run for the grant's duration; a field or a time outside it asks again.
\<close>

lemma careful_reads_run_inside_the_approved_grant:
  "outcome (base\<lparr>ctx_grants := [read_grant]\<rparr>)
     (Read_events ''acct-1'' [''primary''] 10 20 [Field_title]) = Permit"
  by (simp add: broker_simps example_defs read_grant_def)

lemma a_field_or_a_time_outside_the_grant_asks:
  "outcome (base\<lparr>ctx_grants := [read_grant]\<rparr>)
     (Read_events ''acct-1'' [''primary''] 10 20 [Field_title, Field_attendees]) = Need_approval"
  "outcome (base\<lparr>ctx_grants := [read_grant]\<rparr>)
     (Read_events ''acct-1'' [''primary''] 10 2000 [Field_title]) = Need_approval"
  by (simp_all add: broker_simps example_defs read_grant_def)

text \<open>The session's idle timeout: 30 minutes after the last activity the grant is not live.\<close>

lemma a_session_grant_stops_at_the_idle_timeout:
  "outcome (base\<lparr>ctx_grants := [read_grant], ctx_now := 1799999\<rparr>)
     (Read_events ''acct-1'' [''primary''] 10 20 [Field_title]) = Permit"
  "outcome (base\<lparr>ctx_grants := [read_grant], ctx_now := 1800000\<rparr>)
     (Read_events ''acct-1'' [''primary''] 10 20 [Field_title]) = Need_approval"
  "outcome (base\<lparr>ctx_grants := [read_grant], ctx_session := Some (''session-2'', 0)\<rparr>)
     (Read_events ''acct-1'' [''primary''] 10 20 [Field_title]) = Need_approval"
  "outcome (base\<lparr>ctx_grants := [read_grant], ctx_session := None\<rparr>)
     (Read_events ''acct-1'' [''primary''] 10 20 [Field_title]) = Need_approval"
  by (simp_all add: broker_simps example_defs read_grant_def)

text \<open>
  Revocation and expiry between the first proof and the fresh one: a request that rests on a
  grant is blocked once that grant is not live, and the rule says why.
\<close>

lemma a_request_is_blocked_when_its_grant_is_revoked_or_expires:
  "outcome (base\<lparr>ctx_grants := [read_grant\<lparr>g_ended := True\<rparr>], ctx_relied := [''g-read'']\<rparr>)
     (Read_events ''acct-1'' [''primary''] 10 20 [Field_title]) = Block"
  "first_failing (base\<lparr>ctx_grants := [read_grant\<lparr>g_ended := True\<rparr>], ctx_relied := [''g-read'']\<rparr>)
     (Read_events ''acct-1'' [''primary''] 10 20 [Field_title]) = Some Relied_grant_live"
  "outcome (base\<lparr>ctx_grants := [read_grant], ctx_relied := [''g-read''], ctx_now := 1800000\<rparr>)
     (Read_events ''acct-1'' [''primary''] 10 20 [Field_title]) = Block"
  by (simp_all add: broker_simps example_defs read_grant_def first_failing_def)

text \<open>A custom profile that blocks reads blocks them even inside a grant.\<close>

lemma a_blocked_row_is_blocked_inside_a_grant:
  "outcome (base\<lparr>ctx_grants := [read_grant], ctx_profile := careful\<lparr>p_read := Blocks\<rparr>\<rparr>)
     (Read_events ''acct-1'' [''primary''] 10 20 [Field_title]) = Block"
  "first_failing (base\<lparr>ctx_grants := [read_grant], ctx_profile := careful\<lparr>p_read := Blocks\<rparr>\<rparr>)
     (Read_events ''acct-1'' [''primary''] 10 20 [Field_title]) = Some Mode_not_blocked"
  by (simp_all add: broker_simps example_defs read_grant_def first_failing_def)

subsection \<open>Creating (CD-M03, CD-M04)\<close>

definition create_grant :: "grant" where
  "create_grant = standing_grant\<lparr>g_id := ''g-create'', g_tools := [Tool_create],
     g_calendars := [(''acct-1'', ''drafts''), (''acct-1'', ''team'')]\<rparr>"

lemma creating_in_agent_drafts_is_allowed_in_a_grant:
  "outcome (base\<lparr>ctx_grants := [create_grant]\<rparr>) (Create_event ''acct-1'' ''drafts'') = Permit"
  "outcome base (Create_event ''acct-1'' ''drafts'') = Need_approval"
  by (simp_all add: broker_simps example_defs create_grant_def)

text \<open>
  CD-M04: under Careful a standing grant does not cover a create in an existing calendar; the
  owner approves each one. Under Flexible a grant naming the calendar covers it.
\<close>

lemma careful_asks_for_each_create_in_an_existing_calendar:
  "outcome (base\<lparr>ctx_grants := [create_grant]\<rparr>) (Create_event ''acct-1'' ''team'') = Need_approval"
  "outcome (base\<lparr>ctx_grants := [approval]\<rparr>) (Create_event ''acct-1'' ''team'') = Permit"
  by (simp_all add: broker_simps example_defs create_grant_def)

lemma flexible_allows_creates_in_the_calendars_a_grant_names:
  "outcome (base\<lparr>ctx_grants := [create_grant], ctx_profile := flexible\<rparr>)
     (Create_event ''acct-1'' ''team'') = Permit"
  "outcome (base\<lparr>ctx_grants := [create_grant], ctx_profile := flexible\<rparr>)
     (Create_event ''acct-1'' ''primary'') = Need_approval"
  by (simp_all add: broker_simps example_defs create_grant_def)

text \<open>A safer profile narrows at once: the same grant no longer covers the same action.\<close>

lemma switching_to_careful_stops_a_standing_grant:
  "outcome (base\<lparr>ctx_grants := [create_grant], ctx_profile := flexible\<rparr>)
     (Create_event ''acct-1'' ''team'') = Permit"
  "outcome (base\<lparr>ctx_grants := [create_grant], ctx_profile := careful\<rparr>)
     (Create_event ''acct-1'' ''team'') = Need_approval"
  by (simp_all add: broker_simps example_defs create_grant_def)

text \<open>What no approval can make possible.\<close>

lemma creates_that_are_blocked:
  "outcome (base\<lparr>ctx_grants := [approval]\<rparr>) (Create_event ''acct-1'' ''readonly'') = Block"
  "first_failing (base\<lparr>ctx_grants := [approval]\<rparr>) (Create_event ''acct-1'' ''readonly'') = Some Calendar_writable"
  "first_failing (base\<lparr>ctx_grants := [approval]\<rparr>) (Create_event ''acct-1'' ''unselected'') = Some Calendar_selected"
  "first_failing (base\<lparr>ctx_grants := [approval]\<rparr>) (Create_event ''acct-2'' ''primary'') = Some Account_connected"
  "first_failing (base\<lparr>ctx_grants := [approval], ctx_paused := True\<rparr>) (Create_event ''acct-1'' ''team'') = Some Agent_not_paused"
  by (simp_all add: broker_simps example_defs first_failing_def)

subsection \<open>Editing, and editing an event that has attendees (CD-M05, CD-M06, CD-M07)\<close>

definition edit_grant :: "grant" where
  "edit_grant = standing_grant\<lparr>g_id := ''g-edit'', g_tools := [Tool_update],
     g_calendars := [(''acct-1'', ''drafts''), (''acct-1'', ''team'')]\<rparr>"

definition invite_grant :: "grant" where
  "invite_grant = standing_grant\<lparr>g_id := ''g-invite'', g_tools := [Tool_invite],
     g_calendars := [(''acct-1'', ''team'')], g_recipients := Some (Sets [''set-team'']),
     g_send_updates := Some Notify_all\<rparr>"

definition edit_own :: action where
  "edit_own = Update_event ''acct-1'' ''team'' [100] [7] Notify_all"

text \<open>CD-M05 under Flexible, an event with no attendees: the grant to edit is enough.\<close>

lemma flexible_edits_a_fabric_created_event_inside_a_grant:
  "outcome (base\<lparr>ctx_profile := flexible, ctx_grants := [edit_grant], ctx_event := Some own_event\<rparr>)
     edit_own = Permit"
  by (simp add: broker_simps example_defs edit_grant_def edit_own_def)

text \<open>
  The combination CD-G0 asked this theory to state. The same edit of the same event, when the
  event has attendees, reaches them: it falls under the invitation row too. A grant to edit is
  not enough; with a grant for invitations whose recipient set names every attendee it is
  permitted; with one attendee outside every approved set it asks, under any profile.
\<close>

lemma an_edit_that_reaches_attendees_asks_without_invitation_authority:
  "outcome (base\<lparr>ctx_profile := flexible, ctx_grants := [edit_grant],
       ctx_event := Some (own_event\<lparr>ev_attendees := [bob]\<rparr>)\<rparr>) edit_own = Need_approval"
  "first_failing (base\<lparr>ctx_profile := flexible, ctx_grants := [edit_grant],
       ctx_event := Some (own_event\<lparr>ev_attendees := [bob]\<rparr>)\<rparr>) edit_own = Some No_strangers"
  by (simp_all add: broker_simps example_defs edit_grant_def edit_own_def first_failing_def)

lemma an_edit_that_reaches_approved_attendees_is_permitted:
  "outcome (base\<lparr>ctx_profile := flexible, ctx_grants := [edit_grant, invite_grant],
       ctx_recipient_sets := [team],
       ctx_event := Some (own_event\<lparr>ev_attendees := [bob, carol]\<rparr>)\<rparr>) edit_own = Permit"
  by (simp add: broker_simps example_defs edit_grant_def invite_grant_def edit_own_def)

lemma an_edit_that_reaches_a_stranger_asks:
  "outcome (base\<lparr>ctx_profile := flexible, ctx_grants := [edit_grant, invite_grant],
       ctx_recipient_sets := [team],
       ctx_event := Some (own_event\<lparr>ev_attendees := [bob, dan]\<rparr>)\<rparr>) edit_own = Need_approval"
  "first_failing (base\<lparr>ctx_profile := flexible, ctx_grants := [edit_grant, invite_grant],
       ctx_recipient_sets := [team],
       ctx_event := Some (own_event\<lparr>ev_attendees := [bob, dan]\<rparr>)\<rparr>) edit_own = Some No_strangers"
  "outcome (base\<lparr>ctx_profile := widest, ctx_grants := [edit_grant, invite_grant],
       ctx_recipient_sets := [team],
       ctx_event := Some (own_event\<lparr>ev_attendees := [bob, dan]\<rparr>)\<rparr>) edit_own = Need_approval"
  by (simp_all add: broker_simps example_defs edit_grant_def invite_grant_def edit_own_def
      first_failing_def)

text \<open>
  A grant for invitations that notifies differently does not cover the edit, and Careful asks
  for the invitation row whatever grants exist.
\<close>

lemma an_edit_that_reaches_attendees_needs_matching_notification_and_profile:
  "outcome (base\<lparr>ctx_profile := flexible,
       ctx_grants := [edit_grant, invite_grant\<lparr>g_send_updates := Some Notify_none\<rparr>],
       ctx_recipient_sets := [team],
       ctx_event := Some (own_event\<lparr>ev_attendees := [bob]\<rparr>)\<rparr>) edit_own = Need_approval"
  "first_failing (base\<lparr>ctx_profile := flexible,
       ctx_grants := [edit_grant, invite_grant\<lparr>g_send_updates := Some Notify_none\<rparr>],
       ctx_recipient_sets := [team],
       ctx_event := Some (own_event\<lparr>ev_attendees := [bob]\<rparr>)\<rparr>) edit_own = Some Invitation_authority"
  "outcome (base\<lparr>ctx_profile := careful, ctx_grants := [edit_grant, invite_grant],
       ctx_recipient_sets := [team],
       ctx_event := Some (own_event\<lparr>ev_attendees := [bob]\<rparr>)\<rparr>) edit_own = Need_approval"
  by (simp_all add: broker_simps example_defs edit_grant_def invite_grant_def edit_own_def
      first_failing_def)

text \<open>The owner's approval of exactly this edit covers both rows: the card showed who is reached.\<close>

lemma the_owners_approval_of_the_edit_covers_both_rows:
  "outcome (base\<lparr>ctx_profile := careful, ctx_grants := [approval\<lparr>g_recipients := Some (Exact [[1]])\<rparr>],
       ctx_event := Some (own_event\<lparr>ev_attendees := [bob]\<rparr>)\<rparr>) edit_own = Permit"
  by (simp add: broker_simps example_defs edit_own_def)

text \<open>CD-M05 under Careful: "Ask outside Drafts".\<close>

lemma careful_edits_fabric_created_events_in_drafts_only:
  "outcome (base\<lparr>ctx_grants := [edit_grant], ctx_event := Some own_event\<rparr>)
     (Update_event ''acct-1'' ''drafts'' [100] [7] Notify_none) = Permit"
  "outcome (base\<lparr>ctx_grants := [edit_grant], ctx_event := Some own_event\<rparr>) edit_own = Need_approval"
  by (simp_all add: broker_simps example_defs edit_grant_def edit_own_def)

text \<open>
  CD-M06: an event Fabric did not create asks in both built-in profiles, whatever grants
  exist; a custom profile may set the row to Allows (G0-CUSTOM).
\<close>

lemma a_pre_existing_event_asks_unless_a_custom_profile_allows:
  "outcome (base\<lparr>ctx_profile := flexible, ctx_grants := [edit_grant],
       ctx_event := Some (own_event\<lparr>ev_fabric_created := False\<rparr>)\<rparr>) edit_own = Need_approval"
  "outcome (base\<lparr>ctx_profile := flexible, ctx_grants := [approval],
       ctx_event := Some (own_event\<lparr>ev_fabric_created := False\<rparr>)\<rparr>) edit_own = Permit"
  "outcome (base\<lparr>ctx_profile := widest, ctx_grants := [edit_grant],
       ctx_event := Some (own_event\<lparr>ev_fabric_created := False\<rparr>)\<rparr>) edit_own = Permit"
  by (simp_all add: broker_simps example_defs edit_grant_def edit_own_def)

text \<open>A change against an old revision, or to an event that cannot be changed in version 1.\<close>

lemma edits_that_are_blocked:
  "first_failing (base\<lparr>ctx_grants := [approval], ctx_event := Some (own_event\<lparr>ev_revision := [8]\<rparr>)\<rparr>)
     edit_own = Some Revision_current"
  "first_failing (base\<lparr>ctx_grants := [approval]\<rparr>) edit_own = Some Event_known"
  "first_failing (base\<lparr>ctx_grants := [approval], ctx_event := Some (own_event\<lparr>ev_recurring := True\<rparr>)\<rparr>)
     edit_own = Some Event_supported"
  "first_failing (base\<lparr>ctx_grants := [approval], ctx_event := Some (own_event\<lparr>ev_timed := False\<rparr>)\<rparr>)
     edit_own = Some Event_supported"
  "first_failing (base\<lparr>ctx_grants := [approval], ctx_event := Some (own_event\<lparr>ev_default_type := False\<rparr>)\<rparr>)
     edit_own = Some Event_supported"
  "outcome (base\<lparr>ctx_grants := [approval], ctx_event := Some (own_event\<lparr>ev_revision := [8]\<rparr>)\<rparr>)
     edit_own = Block"
  by (simp_all add: broker_simps example_defs edit_own_def first_failing_def)

subsection \<open>Invitations (CD-M07, G0-STRANGERS)\<close>

definition invite :: "recipient list \<Rightarrow> action" where
  "invite rs = Change_invitation ''acct-1'' ''team'' [100] [7] rs Notify_all"

lemma flexible_allows_invitations_to_an_approved_recipient_set:
  "outcome (base\<lparr>ctx_profile := flexible, ctx_grants := [invite_grant], ctx_recipient_sets := [team],
       ctx_event := Some own_event\<rparr>) (invite [bob, carol]) = Permit"
  by (simp add: broker_simps example_defs invite_grant_def invite_def)

lemma careful_asks_for_every_invitation:
  "outcome (base\<lparr>ctx_profile := careful, ctx_grants := [invite_grant], ctx_recipient_sets := [team],
       ctx_event := Some own_event\<rparr>) (invite [bob, carol]) = Need_approval"
  "first_failing (base\<lparr>ctx_profile := careful, ctx_grants := [invite_grant], ctx_recipient_sets := [team],
       ctx_event := Some own_event\<rparr>) (invite [bob, carol]) = Some Invitation_authority"
  by (simp_all add: broker_simps example_defs invite_grant_def invite_def first_failing_def)

text \<open>Strangers always ask: under Flexible, and under a custom profile with every row Allows.\<close>

lemma a_stranger_asks_in_every_profile:
  "outcome (base\<lparr>ctx_profile := flexible, ctx_grants := [invite_grant], ctx_recipient_sets := [team],
       ctx_event := Some own_event\<rparr>) (invite [bob, dan]) = Need_approval"
  "first_failing (base\<lparr>ctx_profile := flexible, ctx_grants := [invite_grant], ctx_recipient_sets := [team],
       ctx_event := Some own_event\<rparr>) (invite [bob, dan]) = Some No_strangers"
  "outcome (base\<lparr>ctx_profile := widest, ctx_grants := [invite_grant], ctx_recipient_sets := [team],
       ctx_event := Some own_event\<rparr>) (invite [bob, dan]) = Need_approval"
  by (simp_all add: broker_simps example_defs invite_grant_def invite_def first_failing_def)

text \<open>
  A domain in a recipient set covers every address of that domain; removing an attendee who
  is a stranger still reaches that stranger.
\<close>

lemma domains_and_removed_attendees:
  "outcome (base\<lparr>ctx_profile := flexible, ctx_grants := [invite_grant],
       ctx_recipient_sets := [team\<lparr>rs_domains := [[11]]\<rparr>], ctx_event := Some own_event\<rparr>)
     (invite [bob, dan]) = Permit"
  "outcome (base\<lparr>ctx_profile := flexible, ctx_grants := [invite_grant], ctx_recipient_sets := [team],
       ctx_event := Some (own_event\<lparr>ev_attendees := [dan]\<rparr>)\<rparr>) (invite [bob]) = Need_approval"
  by (simp_all add: broker_simps example_defs invite_grant_def invite_def)

text \<open>The owner's approval of exactly this invitation names its recipients, so it is permitted.\<close>

lemma the_owners_approval_of_an_invitation_to_a_stranger:
  "outcome (base\<lparr>ctx_grants := [approval\<lparr>g_recipients := Some (Exact [[1], [3]])\<rparr>],
       ctx_event := Some own_event\<rparr>) (invite [bob, dan]) = Permit"
  "outcome (base\<lparr>ctx_grants := [approval\<lparr>g_recipients := Some (Exact [[1]])\<rparr>],
       ctx_event := Some own_event\<rparr>) (invite [bob, dan]) = Need_approval"
  by (simp_all add: broker_simps example_defs invite_def)

subsection \<open>Responding (CD-M08, CD-Q05b)\<close>

definition invitation :: event where
  "invitation = own_event\<lparr>ev_id := [200], ev_fabric_created := False, ev_invitation := True\<rparr>"

definition occurrence :: event where
  "occurrence = invitation\<lparr>ev_id := [201], ev_recurring := True, ev_occurrence := Some 5000\<rparr>"

lemma a_response_asks_in_both_built_in_profiles:
  "outcome (base\<lparr>ctx_event := Some invitation\<rparr>)
     (Respond ''acct-1'' ''primary'' (Whole_event [200] [7]) Accepted Notify_all) = Need_approval"
  "outcome (base\<lparr>ctx_profile := flexible, ctx_event := Some invitation\<rparr>)
     (Respond ''acct-1'' ''primary'' (Whole_event [200] [7]) Accepted Notify_all) = Need_approval"
  "outcome (base\<lparr>ctx_grants := [approval], ctx_event := Some invitation\<rparr>)
     (Respond ''acct-1'' ''primary'' (Whole_event [200] [7]) Accepted Notify_all) = Permit"
  by (simp_all add: broker_simps example_defs invitation_def)

text \<open>
  One occurrence of a recurring invitation: the instance id, the original start time and the
  revision must all be the occurrence's own. The series as a whole cannot be answered.
\<close>

lemma a_response_to_one_occurrence:
  "outcome (base\<lparr>ctx_grants := [approval], ctx_event := Some occurrence\<rparr>)
     (Respond ''acct-1'' ''primary'' (Occurrence [201] 5000 [7]) Declined Notify_all) = Permit"
  "first_failing (base\<lparr>ctx_grants := [approval], ctx_event := Some occurrence\<rparr>)
     (Respond ''acct-1'' ''primary'' (Occurrence [201] 5001 [7]) Declined Notify_all) = Some Invitation_target"
  "first_failing (base\<lparr>ctx_grants := [approval], ctx_event := Some occurrence\<rparr>)
     (Respond ''acct-1'' ''primary'' (Occurrence [201] 5000 [8]) Declined Notify_all) = Some Revision_current"
  "first_failing (base\<lparr>ctx_grants := [approval], ctx_event := Some occurrence\<rparr>)
     (Respond ''acct-1'' ''primary'' (Occurrence [202] 5000 [7]) Declined Notify_all) = Some Event_known"
  "first_failing (base\<lparr>ctx_grants := [approval], ctx_event := Some (invitation\<lparr>ev_recurring := True\<rparr>)\<rparr>)
     (Respond ''acct-1'' ''primary'' (Whole_event [200] [7]) Declined Notify_all) = Some Event_supported"
  "first_failing (base\<lparr>ctx_grants := [approval], ctx_event := Some own_event\<rparr>)
     (Respond ''acct-1'' ''primary'' (Whole_event [100] [7]) Accepted Notify_all) = Some Invitation_target"
  by (simp_all add: broker_simps example_defs invitation_def occurrence_def first_failing_def)

subsection \<open>Deleting (CD-Q05a) and what is always blocked (CD-M10, G0-WARNDISC)\<close>

definition delete_own :: action where
  "delete_own = Delete_event ''acct-1'' ''team'' [100] [7] Notify_all"

lemma deleting_needs_the_owners_approval_each_time:
  "outcome (base\<lparr>ctx_grants := [approval], ctx_event := Some own_event\<rparr>) delete_own = Permit"
  "outcome (base\<lparr>ctx_profile := widest,
       ctx_grants := [standing_grant\<lparr>g_tools := [Tool_delete], g_calendars := [(''acct-1'', ''team'')],
                       g_send_updates := Some Notify_all\<rparr>],
       ctx_event := Some own_event\<rparr>) delete_own = Need_approval"
  "outcome (base\<lparr>ctx_grants := [approval\<lparr>g_duration := One_request ''d-this'' True\<rparr>],
       ctx_event := Some own_event\<rparr>) delete_own = Need_approval"
  "outcome (base\<lparr>ctx_grants := [approval\<lparr>g_duration := One_request ''d-other'' False\<rparr>],
       ctx_event := Some own_event\<rparr>) delete_own = Need_approval"
  by (simp_all add: broker_simps example_defs delete_own_def)

lemma only_events_fabric_created_are_deleted:
  "outcome (base\<lparr>ctx_grants := [approval], ctx_event := Some (own_event\<lparr>ev_fabric_created := False\<rparr>)\<rparr>)
     delete_own = Block"
  "first_failing (base\<lparr>ctx_grants := [approval], ctx_event := Some (own_event\<lparr>ev_fabric_created := False\<rparr>)\<rparr>)
     delete_own = Some Fabric_created"
  by (simp_all add: broker_simps example_defs delete_own_def first_failing_def)

lemma calendars_and_sharing_are_never_touched:
  "outcome (base\<lparr>ctx_profile := widest, ctx_grants := [approval]\<rparr>) (Delete_calendar ''acct-1'' ''team'') = Block"
  "first_failing (base\<lparr>ctx_profile := widest, ctx_grants := [approval]\<rparr>) (Change_sharing ''acct-1'' ''team'') = Some Mode_not_blocked"
  "outcome (base\<lparr>ctx_profile := widest, ctx_grants := [approval]\<rparr>) Disclose_warning = Block"
  by (simp_all add: broker_simps example_defs first_failing_def)

subsection \<open>Releasing results (plan section 9.2)\<close>

definition one_read :: "grant" where
  "one_read = approval\<lparr>g_id := ''g-read-once''\<rparr>"

definition the_read :: action where
  "the_read = Read_events ''acct-1'' [''primary''] 10 20 [Field_title]"

text \<open>
  A one-request read grant permits one release of the result. After it is used, a second
  release asks; so does a release after the session grant behind the read has ended.
\<close>

lemma a_read_result_is_released_once:
  "outcome (base\<lparr>ctx_grants := [one_read], ctx_result_held := True\<rparr>) (Disclose the_read) = Permit"
  "outcome (base\<lparr>ctx_grants := [one_read\<lparr>g_duration := One_request ''d-this'' True\<rparr>],
       ctx_result_held := True\<rparr>) (Disclose the_read) = Need_approval"
  "outcome (base\<lparr>ctx_grants := [read_grant\<lparr>g_ended := True\<rparr>], ctx_result_held := True\<rparr>)
     (Disclose the_read) = Need_approval"
  "outcome (base\<lparr>ctx_grants := [read_grant], ctx_result_held := True\<rparr>) (Disclose the_read) = Permit"
  by (simp_all add: broker_simps example_defs one_read_def the_read_def read_grant_def)

lemma releases_that_are_blocked:
  "first_failing (base\<lparr>ctx_grants := [one_read]\<rparr>) (Disclose the_read) = Some Result_held"
  "first_failing (base\<lparr>ctx_grants := [one_read], ctx_result_held := True, ctx_paused := True\<rparr>)
     (Disclose the_read) = Some Agent_not_paused"
  "first_failing (base\<lparr>ctx_grants := [one_read], ctx_result_held := True\<rparr>)
     (Disclose (Disclose the_read)) = Some Result_held"
  "first_failing (base\<lparr>ctx_grants := [one_read], ctx_result_held := True, ctx_connected := []\<rparr>)
     (Disclose the_read) = Some Account_connected"
  by (simp_all add: broker_simps example_defs one_read_def the_read_def first_failing_def)

text \<open>The outcome of a change the agent made is released without a grant, unless it is paused.\<close>

lemma the_outcome_of_a_change_is_released_to_its_agent:
  "outcome (base\<lparr>ctx_result_held := True\<rparr>) (Disclose (Create_event ''acct-1'' ''team'')) = Permit"
  "outcome (base\<lparr>ctx_result_held := True, ctx_paused := True\<rparr>)
     (Disclose (Create_event ''acct-1'' ''team'')) = Block"
  by (simp_all add: broker_simps example_defs)

subsection \<open>The broker's own reads (plan sections 8 and 11)\<close>

definition system :: broker_context where
  "system = base\<lparr>ctx_actor := System, ctx_paused := True, ctx_session := None\<rparr>"

text \<open>
  A refresh needs a calendar the owner selected, or one chosen for warnings. A reconciliation
  read needs a change the broker handed to Google and has not seen applied; pausing the agent
  does not stop it. An agent can ask for neither.
\<close>

lemma the_brokers_own_reads:
  "outcome system (System_refresh ''acct-1'' ''primary'') = Permit"
  "outcome (system\<lparr>ctx_warning_calendars := [(''acct-1'', ''private'')]\<rparr>)
     (System_refresh ''acct-1'' ''private'') = Permit"
  "first_failing system (System_refresh ''acct-1'' ''private'') = Some System_authority"
  "outcome (system\<lparr>ctx_pending_effects := [(''acct-1'', ''team'', [100])]\<rparr>)
     (System_reconcile ''acct-1'' ''team'' [100]) = Permit"
  "first_failing system (System_reconcile ''acct-1'' ''team'' [100]) = Some System_authority"
  "first_failing base (System_refresh ''acct-1'' ''primary'') = Some Actor"
  "first_failing system (Create_event ''acct-1'' ''team'') = Some Actor"
  by (simp_all add: broker_simps example_defs system_def first_failing_def)

subsection \<open>What a lasting approval is worth (contract README, section 10)\<close>

text \<open>
  When the owner approves a request for a session or until revoked, the broker makes a grant
  shaped like the request: its one tool, its calendar, its window and fields for a read, who is
  notified for a change, and the recipients for an invitation. Each case below is the broker's
  fresh proof right after such an approval: that grant is the only grant of the context, and
  the request rests on it. Where the outcome is Permit, a lasting approval serves a request of
  that kind. Where the request asks again, only an approval of this one request serves it.
  Every case is stated for a grant until revoked and for a grant of the live session.

  Two profiles stand for the two modes a row can have besides Blocks. In widest, every row a
  profile may set is Allows; in asking, every one is Asks.
\<close>

definition asking :: profile where
  "asking = \<lparr>p_list = Asks, p_read = Asks, p_drafts = Asks, p_existing = Asks,
     p_edit_fabric = Asks, p_edit_other = Asks, p_invite = Asks, p_respond = Asks,
     p_delete = Asks, p_sharing = Blocks\<rparr>"

definition lasting :: "tool \<Rightarrow> string \<Rightarrow> grant" where
  "lasting t cal = standing_grant\<lparr>g_id := ''g-lasting'', g_tools := [t],
     g_calendars := [(''acct-1'', cal)]\<rparr>"

definition for_session :: "grant \<Rightarrow> grant" where
  "for_session g = g\<lparr>g_duration := Session ''session-1''\<rparr>"

definition after_approval :: "profile \<Rightarrow> grant \<Rightarrow> broker_context" where
  "after_approval p g = base\<lparr>ctx_profile := p, ctx_grants := [g], ctx_relied := [''g-lasting'']\<rparr>"

definition lasting_availability :: grant where
  "lasting_availability = (lasting Tool_availability ''primary'')\<lparr>g_window := Some (0, 100)\<rparr>"

definition lasting_read :: grant where
  "lasting_read = (lasting Tool_read ''primary'')
     \<lparr>g_window := Some (0, 100), g_fields := [Field_time, Field_title]\<rparr>"

definition lasting_edit :: "string \<Rightarrow> send_updates \<Rightarrow> grant" where
  "lasting_edit cal su = (lasting Tool_update cal)\<lparr>g_send_updates := Some su\<rparr>"

definition lasting_invitation :: grant where
  "lasting_invitation = (lasting Tool_invite ''team'')
     \<lparr>g_recipients := Some (Exact [[1], [3]]), g_send_updates := Some Notify_all\<rparr>"

definition lasting_response :: grant where
  "lasting_response = (lasting Tool_respond ''primary'')\<lparr>g_send_updates := Some Notify_all\<rparr>"

definition lasting_delete :: grant where
  "lasting_delete = (lasting Tool_delete ''team'')\<lparr>g_send_updates := Some Notify_all\<rparr>"

lemmas lasting_defs =
  asking_def lasting_def for_session_def after_approval_def lasting_availability_def
  lasting_read_def lasting_edit_def lasting_invitation_def lasting_response_def
  lasting_delete_def edit_own_def delete_own_def invite_def invitation_def

text \<open>Availability (CD-M01).\<close>

lemma a_lasting_approval_of_availability_serves_under_allow:
  "outcome (after_approval widest lasting_availability)
     (Read_availability ''acct-1'' [''primary''] 0 100) = Permit"
  "outcome (after_approval widest (for_session lasting_availability))
     (Read_availability ''acct-1'' [''primary''] 0 100) = Permit"
  by (simp_all add: broker_simps example_defs lasting_defs)

lemma a_lasting_approval_of_availability_does_not_serve_under_ask:
  "outcome (after_approval asking lasting_availability)
     (Read_availability ''acct-1'' [''primary''] 0 100) = Need_approval"
  "first_failing (after_approval asking lasting_availability)
     (Read_availability ''acct-1'' [''primary''] 0 100) = Some Authority"
  "outcome (after_approval asking (for_session lasting_availability))
     (Read_availability ''acct-1'' [''primary''] 0 100) = Need_approval"
  by (simp_all add: broker_simps example_defs lasting_defs first_failing_def)

text \<open>Event details (CD-M02): under Asks too, reads run inside the approved grant.\<close>

lemma a_lasting_approval_of_a_read_serves_under_allow:
  "outcome (after_approval widest lasting_read)
     (Read_events ''acct-1'' [''primary''] 0 100 [Field_time, Field_title]) = Permit"
  "outcome (after_approval widest (for_session lasting_read))
     (Read_events ''acct-1'' [''primary''] 0 100 [Field_time, Field_title]) = Permit"
  by (simp_all add: broker_simps example_defs lasting_defs)

lemma a_lasting_approval_of_a_read_serves_under_ask:
  "outcome (after_approval asking lasting_read)
     (Read_events ''acct-1'' [''primary''] 0 100 [Field_time, Field_title]) = Permit"
  "outcome (after_approval asking (for_session lasting_read))
     (Read_events ''acct-1'' [''primary''] 0 100 [Field_time, Field_title]) = Permit"
  by (simp_all add: broker_simps example_defs lasting_defs)

text \<open>Creating (CD-M03, CD-M04).\<close>

lemma a_lasting_approval_of_a_create_in_agent_drafts_serves_under_allow:
  "outcome (after_approval widest (lasting Tool_create ''drafts''))
     (Create_event ''acct-1'' ''drafts'') = Permit"
  "outcome (after_approval widest (for_session (lasting Tool_create ''drafts'')))
     (Create_event ''acct-1'' ''drafts'') = Permit"
  by (simp_all add: broker_simps example_defs lasting_defs)

lemma a_lasting_approval_of_a_create_in_agent_drafts_does_not_serve_under_ask:
  "outcome (after_approval asking (lasting Tool_create ''drafts''))
     (Create_event ''acct-1'' ''drafts'') = Need_approval"
  "first_failing (after_approval asking (lasting Tool_create ''drafts''))
     (Create_event ''acct-1'' ''drafts'') = Some Authority"
  "outcome (after_approval asking (for_session (lasting Tool_create ''drafts'')))
     (Create_event ''acct-1'' ''drafts'') = Need_approval"
  by (simp_all add: broker_simps example_defs lasting_defs first_failing_def)

lemma a_lasting_approval_of_a_create_in_another_calendar_serves_under_allow:
  "outcome (after_approval widest (lasting Tool_create ''team''))
     (Create_event ''acct-1'' ''team'') = Permit"
  "outcome (after_approval widest (for_session (lasting Tool_create ''team'')))
     (Create_event ''acct-1'' ''team'') = Permit"
  by (simp_all add: broker_simps example_defs lasting_defs)

lemma a_lasting_approval_of_a_create_in_another_calendar_does_not_serve_under_ask:
  "outcome (after_approval asking (lasting Tool_create ''team''))
     (Create_event ''acct-1'' ''team'') = Need_approval"
  "first_failing (after_approval asking (lasting Tool_create ''team''))
     (Create_event ''acct-1'' ''team'') = Some Authority"
  "outcome (after_approval asking (for_session (lasting Tool_create ''team'')))
     (Create_event ''acct-1'' ''team'') = Need_approval"
  by (simp_all add: broker_simps example_defs lasting_defs first_failing_def)

text \<open>Editing an event Fabric created that has no attendees (CD-M05): under Asks a lasting
  approval serves in Agent Drafts and nowhere else.\<close>

lemma a_lasting_approval_of_an_edit_in_agent_drafts_serves_under_allow:
  "outcome ((after_approval widest (lasting_edit ''drafts'' Notify_none))\<lparr>ctx_event := Some own_event\<rparr>)
     (Update_event ''acct-1'' ''drafts'' [100] [7] Notify_none) = Permit"
  "outcome ((after_approval widest (for_session (lasting_edit ''drafts'' Notify_none)))\<lparr>ctx_event := Some own_event\<rparr>)
     (Update_event ''acct-1'' ''drafts'' [100] [7] Notify_none) = Permit"
  by (simp_all add: broker_simps example_defs lasting_defs)

lemma a_lasting_approval_of_an_edit_in_agent_drafts_serves_under_ask:
  "outcome ((after_approval asking (lasting_edit ''drafts'' Notify_none))\<lparr>ctx_event := Some own_event\<rparr>)
     (Update_event ''acct-1'' ''drafts'' [100] [7] Notify_none) = Permit"
  "outcome ((after_approval asking (for_session (lasting_edit ''drafts'' Notify_none)))\<lparr>ctx_event := Some own_event\<rparr>)
     (Update_event ''acct-1'' ''drafts'' [100] [7] Notify_none) = Permit"
  by (simp_all add: broker_simps example_defs lasting_defs)

lemma a_lasting_approval_of_an_edit_outside_agent_drafts_serves_under_allow:
  "outcome ((after_approval widest (lasting_edit ''team'' Notify_all))\<lparr>ctx_event := Some own_event\<rparr>)
     edit_own = Permit"
  "outcome ((after_approval widest (for_session (lasting_edit ''team'' Notify_all)))\<lparr>ctx_event := Some own_event\<rparr>)
     edit_own = Permit"
  by (simp_all add: broker_simps example_defs lasting_defs)

lemma a_lasting_approval_of_an_edit_outside_agent_drafts_does_not_serve_under_ask:
  "outcome ((after_approval asking (lasting_edit ''team'' Notify_all))\<lparr>ctx_event := Some own_event\<rparr>)
     edit_own = Need_approval"
  "first_failing ((after_approval asking (lasting_edit ''team'' Notify_all))\<lparr>ctx_event := Some own_event\<rparr>)
     edit_own = Some Authority"
  "outcome ((after_approval asking (for_session (lasting_edit ''team'' Notify_all)))\<lparr>ctx_event := Some own_event\<rparr>)
     edit_own = Need_approval"
  by (simp_all add: broker_simps example_defs lasting_defs first_failing_def)

text \<open>Editing an event Fabric did not create (CD-M06).\<close>

lemma a_lasting_approval_of_an_edit_of_a_pre_existing_event_serves_under_allow:
  "outcome ((after_approval widest (lasting_edit ''team'' Notify_all))\<lparr>ctx_event := Some (own_event\<lparr>ev_fabric_created := False\<rparr>)\<rparr>)
     edit_own = Permit"
  "outcome ((after_approval widest (for_session (lasting_edit ''team'' Notify_all)))\<lparr>ctx_event := Some (own_event\<lparr>ev_fabric_created := False\<rparr>)\<rparr>)
     edit_own = Permit"
  by (simp_all add: broker_simps example_defs lasting_defs)

lemma a_lasting_approval_of_an_edit_of_a_pre_existing_event_does_not_serve_under_ask:
  "outcome ((after_approval asking (lasting_edit ''team'' Notify_all))\<lparr>ctx_event := Some (own_event\<lparr>ev_fabric_created := False\<rparr>)\<rparr>)
     edit_own = Need_approval"
  "first_failing ((after_approval asking (lasting_edit ''team'' Notify_all))\<lparr>ctx_event := Some (own_event\<lparr>ev_fabric_created := False\<rparr>)\<rparr>)
     edit_own = Some Authority"
  "outcome ((after_approval asking (for_session (lasting_edit ''team'' Notify_all)))\<lparr>ctx_event := Some (own_event\<lparr>ev_fabric_created := False\<rparr>)\<rparr>)
     edit_own = Need_approval"
  by (simp_all add: broker_simps example_defs lasting_defs first_failing_def)

text \<open>
  An edit of an event that has attendees reaches them, so it falls under the row for
  invitations too (CD-M07). The grant of an approved edit names neither the invitation tool nor
  any recipient, so it never serves the next such edit, whatever the two rows say.
\<close>

lemma a_lasting_approval_of_an_edit_that_reaches_attendees_never_serves:
  "outcome ((after_approval widest (lasting_edit ''team'' Notify_all))\<lparr>ctx_event := Some (own_event\<lparr>ev_attendees := [bob]\<rparr>)\<rparr>)
     edit_own = Need_approval"
  "first_failing ((after_approval widest (lasting_edit ''team'' Notify_all))\<lparr>ctx_event := Some (own_event\<lparr>ev_attendees := [bob]\<rparr>)\<rparr>)
     edit_own = Some No_strangers"
  "outcome ((after_approval widest (for_session (lasting_edit ''team'' Notify_all)))\<lparr>ctx_event := Some (own_event\<lparr>ev_attendees := [bob]\<rparr>)\<rparr>)
     edit_own = Need_approval"
  "outcome ((after_approval (widest\<lparr>p_invite := Asks\<rparr>) (lasting_edit ''team'' Notify_all))\<lparr>ctx_event := Some (own_event\<lparr>ev_attendees := [bob]\<rparr>)\<rparr>)
     edit_own = Need_approval"
  "outcome ((after_approval (asking\<lparr>p_invite := Allows\<rparr>) (lasting_edit ''team'' Notify_all))\<lparr>ctx_event := Some (own_event\<lparr>ev_attendees := [bob]\<rparr>)\<rparr>)
     edit_own = Need_approval"
  "outcome ((after_approval asking (lasting_edit ''team'' Notify_all))\<lparr>ctx_event := Some (own_event\<lparr>ev_attendees := [bob]\<rparr>)\<rparr>)
     edit_own = Need_approval"
  by (simp_all add: broker_simps example_defs lasting_defs first_failing_def)

text \<open>Invitations (CD-M07). The grant carries the approved attendees as exact addresses. Under
  Allows it serves a later invitation of the same people on that calendar; under Asks every
  invitation asks.\<close>

lemma a_lasting_approval_of_an_invitation_serves_under_allow:
  "outcome ((after_approval widest lasting_invitation)\<lparr>ctx_event := Some own_event\<rparr>)
     (invite [bob, dan]) = Permit"
  "outcome ((after_approval widest (for_session lasting_invitation))\<lparr>ctx_event := Some own_event\<rparr>)
     (invite [bob, dan]) = Permit"
  by (simp_all add: broker_simps example_defs lasting_defs)

lemma a_lasting_approval_of_an_invitation_does_not_serve_under_ask:
  "outcome ((after_approval asking lasting_invitation)\<lparr>ctx_event := Some own_event\<rparr>)
     (invite [bob, dan]) = Need_approval"
  "first_failing ((after_approval asking lasting_invitation)\<lparr>ctx_event := Some own_event\<rparr>)
     (invite [bob, dan]) = Some Invitation_authority"
  "outcome ((after_approval asking (for_session lasting_invitation))\<lparr>ctx_event := Some own_event\<rparr>)
     (invite [bob, dan]) = Need_approval"
  by (simp_all add: broker_simps example_defs lasting_defs first_failing_def)

text \<open>Responding (CD-M08).\<close>

lemma a_lasting_approval_of_a_response_serves_under_allow:
  "outcome ((after_approval widest lasting_response)\<lparr>ctx_event := Some invitation\<rparr>)
     (Respond ''acct-1'' ''primary'' (Whole_event [200] [7]) Accepted Notify_all) = Permit"
  "outcome ((after_approval widest (for_session lasting_response))\<lparr>ctx_event := Some invitation\<rparr>)
     (Respond ''acct-1'' ''primary'' (Whole_event [200] [7]) Accepted Notify_all) = Permit"
  by (simp_all add: broker_simps example_defs lasting_defs)

lemma a_lasting_approval_of_a_response_does_not_serve_under_ask:
  "outcome ((after_approval asking lasting_response)\<lparr>ctx_event := Some invitation\<rparr>)
     (Respond ''acct-1'' ''primary'' (Whole_event [200] [7]) Accepted Notify_all) = Need_approval"
  "first_failing ((after_approval asking lasting_response)\<lparr>ctx_event := Some invitation\<rparr>)
     (Respond ''acct-1'' ''primary'' (Whole_event [200] [7]) Accepted Notify_all) = Some Authority"
  "outcome ((after_approval asking (for_session lasting_response))\<lparr>ctx_event := Some invitation\<rparr>)
     (Respond ''acct-1'' ''primary'' (Whole_event [200] [7]) Accepted Notify_all) = Need_approval"
  by (simp_all add: broker_simps example_defs lasting_defs first_failing_def)

text \<open>Deleting always asks (CD-Q05a): no lasting approval serves it, in any profile.\<close>

lemma a_lasting_approval_of_a_delete_never_serves:
  "outcome ((after_approval asking lasting_delete)\<lparr>ctx_event := Some own_event\<rparr>)
     delete_own = Need_approval"
  "first_failing ((after_approval asking lasting_delete)\<lparr>ctx_event := Some own_event\<rparr>)
     delete_own = Some Authority"
  "outcome ((after_approval asking (for_session lasting_delete))\<lparr>ctx_event := Some own_event\<rparr>)
     delete_own = Need_approval"
  "outcome ((after_approval widest lasting_delete)\<lparr>ctx_event := Some own_event\<rparr>)
     delete_own = Need_approval"
  by (simp_all add: broker_simps example_defs lasting_defs first_failing_def)

subsection \<open>Which grants a permitted action rests on\<close>

text \<open>
  A check also proves, for every grant of the agent, whether it authorizes the action. The
  broker takes the grants a request rests on from that proof, and names them again in the
  fresh proof before the change is sent.
\<close>

definition both_edit_grants :: broker_context where
  "both_edit_grants = base\<lparr>ctx_profile := flexible, ctx_grants := [edit_grant, invite_grant],
     ctx_recipient_sets := [team], ctx_event := Some (own_event\<lparr>ev_attendees := [bob]\<rparr>)\<rparr>"

lemma an_edit_that_reaches_attendees_rests_on_two_grants:
  "authorizes both_edit_grants edit_own edit_grant"
  "authorizes both_edit_grants edit_own invite_grant"
  "outcome both_edit_grants edit_own = Permit"
  by (simp_all add: broker_simps example_defs edit_grant_def invite_grant_def edit_own_def
      both_edit_grants_def)

lemma only_the_grants_that_cover_the_action_authorize_it:
  "\<not> authorizes (base\<lparr>ctx_grants := [availability_grant, read_grant]\<rparr>) the_read availability_grant"
  "authorizes (base\<lparr>ctx_grants := [availability_grant, read_grant]\<rparr>) the_read read_grant"
  "\<not> authorizes (base\<lparr>ctx_grants := [create_grant]\<rparr>) (Create_event ''acct-1'' ''team'') create_grant"
  "authorizes (base\<lparr>ctx_grants := [create_grant], ctx_profile := flexible\<rparr>)
     (Create_event ''acct-1'' ''team'') create_grant"
  by (simp_all add: broker_simps example_defs availability_grant_def read_grant_def the_read_def
      create_grant_def)

lemma the_owners_approval_authorizes_until_it_is_used:
  "authorizes (base\<lparr>ctx_grants := [approval], ctx_event := Some own_event\<rparr>) delete_own approval"
  "\<not> authorizes (base\<lparr>ctx_event := Some own_event\<rparr>) delete_own
     (approval\<lparr>g_duration := One_request ''d-this'' True\<rparr>)"
  "\<not> authorizes (base\<lparr>ctx_event := Some own_event\<rparr>) delete_own
     (approval\<lparr>g_duration := One_request ''d-other'' False\<rparr>)"
  by (simp_all add: broker_simps example_defs delete_own_def)

text \<open>The outcome of a change is released without a grant, so no grant authorizes that release.\<close>

lemma releasing_the_outcome_of_a_change_rests_on_no_grant:
  "\<not> authorizes (base\<lparr>ctx_grants := [approval], ctx_result_held := True\<rparr>)
     (Disclose (Create_event ''acct-1'' ''team'')) approval"
  by (simp add: broker_simps example_defs)

text \<open>
  A request that rests on two grants is blocked as soon as one of them ends, and the rule says
  why.
\<close>

lemma a_request_resting_on_two_grants_is_blocked_when_one_ends:
  "outcome (both_edit_grants\<lparr>ctx_relied := [''g-edit'', ''g-invite'']\<rparr>) edit_own = Permit"
  "outcome (both_edit_grants\<lparr>ctx_grants := [edit_grant, invite_grant\<lparr>g_ended := True\<rparr>],
       ctx_relied := [''g-edit'', ''g-invite'']\<rparr>) edit_own = Block"
  "first_failing (both_edit_grants\<lparr>ctx_grants := [edit_grant, invite_grant\<lparr>g_ended := True\<rparr>],
       ctx_relied := [''g-edit'', ''g-invite'']\<rparr>) edit_own = Some Relied_grant_live"
  by (simp_all add: broker_simps example_defs edit_grant_def invite_grant_def edit_own_def
      both_edit_grants_def first_failing_def)

end
