theory Check_000001
  imports "Fabric_Broker.Fabric_Broker"
begin

definition ctx :: broker_context where "ctx = \<lparr>ctx_actor = Agent ''agent-1'', ctx_paused = False, ctx_now = 1700000000000, ctx_session = Some (''session-1'', 1699999990000), ctx_profile = \<lparr>p_list = Allows, p_read = Allows, p_drafts = Allows, p_existing = Allows, p_edit_fabric = Allows, p_edit_other = Asks, p_invite = Allows, p_respond = Asks, p_delete = Asks, p_sharing = Blocks\<rparr>, ctx_connected = [''acct-1''], ctx_calendars = [\<lparr>cal_account = ''acct-1'', cal_id = ''drafts'', cal_writable = True, cal_drafts = True\<rparr>, \<lparr>cal_account = ''acct-1'', cal_id = ''team'', cal_writable = True, cal_drafts = False\<rparr>], ctx_warning_calendars = [(''acct-1'', ''private'')], ctx_grants = [\<lparr>g_id = ''g-invite'', g_tools = [Tool_invite, Tool_update], g_duration = Session ''session-1'', g_ended = False, g_calendars = [(''acct-1'', ''team'')], g_window = Some ((-5), 1000), g_fields = [Field_time, Field_attendees], g_recipients = Some (Sets [''set-team'']), g_send_updates = Some Notify_all\<rparr>, \<lparr>g_id = ''g-once'', g_tools = [], g_duration = One_request ''0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef'' False, g_ended = False, g_calendars = [], g_window = None, g_fields = [], g_recipients = Some (Exact [[98, 111, 98, 64, 101, 120, 97, 109, 112, 108, 101, 46, 99, 111, 109]]), g_send_updates = None\<rparr>, \<lparr>g_id = ''g-standing'', g_tools = [Tool_list], g_duration = Until_revoked, g_ended = True, g_calendars = [], g_window = None, g_fields = [], g_recipients = None, g_send_updates = None\<rparr>], ctx_recipient_sets = [\<lparr>rs_id = ''set-team'', rs_members = [[98, 111, 98, 64, 101, 120, 97, 109, 112, 108, 101, 46, 99, 111, 109]], rs_domains = [[101, 120, 97, 109, 112, 108, 101, 46, 111, 114, 103]]\<rparr>], ctx_event = Some (\<lparr>ev_id = [101, 118, 116, 49], ev_revision = [55], ev_fabric_created = True, ev_recurring = False, ev_default_type = True, ev_timed = True, ev_invitation = False, ev_attendees = [\<lparr>rc_address = [98, 111, 98, 64, 101, 120, 97, 109, 112, 108, 101, 46, 99, 111, 109], rc_domain = [101, 120, 97, 109, 112, 108, 101, 46, 99, 111, 109]\<rparr>], ev_occurrence = None\<rparr>), ctx_digest = ''0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef'', ctx_relied = Some ''g-invite'', ctx_result_held = False, ctx_pending_effects = [(''acct-1'', ''team'', [101, 118, 116, 48])], ctx_pause_epoch = 3, ctx_revocation_epoch = 5\<rparr>"

definition act :: action where "act = Change_invitation ''acct-1'' ''team'' [101, 118, 116, 49] [55] [\<lparr>rc_address = [98, 111, 98, 64, 101, 120, 97, 109, 112, 108, 101, 46, 99, 111, 109], rc_domain = [101, 120, 97, 109, 112, 108, 101, 46, 99, 111, 109]\<rparr>, \<lparr>rc_address = [99, 97, 114, 111, 108, 64, 101, 120, 97, 109, 112, 108, 101, 46, 111, 114, 103], rc_domain = [101, 120, 97, 109, 112, 108, 101, 46, 111, 114, 103]\<rparr>] Notify_all"

lemma rule_actor_holds: "holds Actor ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_actor_fails: "\<not> holds Actor ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_agent_not_paused_holds: "holds Agent_not_paused ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_agent_not_paused_fails: "\<not> holds Agent_not_paused ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_mode_not_blocked_holds: "holds Mode_not_blocked ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_mode_not_blocked_fails: "\<not> holds Mode_not_blocked ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_account_connected_holds: "holds Account_connected ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_account_connected_fails: "\<not> holds Account_connected ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_calendar_selected_holds: "holds Calendar_selected ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_calendar_selected_fails: "\<not> holds Calendar_selected ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_calendar_writable_holds: "holds Calendar_writable ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_calendar_writable_fails: "\<not> holds Calendar_writable ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_event_known_holds: "holds Event_known ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_event_known_fails: "\<not> holds Event_known ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_revision_current_holds: "holds Revision_current ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_revision_current_fails: "\<not> holds Revision_current ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_event_supported_holds: "holds Event_supported ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_event_supported_fails: "\<not> holds Event_supported ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_fabric_created_holds: "holds Fabric_created ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_fabric_created_fails: "\<not> holds Fabric_created ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_invitation_target_holds: "holds Invitation_target ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_invitation_target_fails: "\<not> holds Invitation_target ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_relied_grant_live_holds: "holds Relied_grant_live ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_relied_grant_live_fails: "\<not> holds Relied_grant_live ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_result_held_holds: "holds Result_held ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_result_held_fails: "\<not> holds Result_held ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_warnings_owner_only_holds: "holds Warnings_owner_only ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_warnings_owner_only_fails: "\<not> holds Warnings_owner_only ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_system_authority_holds: "holds System_authority ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_system_authority_fails: "\<not> holds System_authority ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_no_strangers_holds: "holds No_strangers ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_no_strangers_fails: "\<not> holds No_strangers ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_invitation_authority_holds: "holds Invitation_authority ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_invitation_authority_fails: "\<not> holds Invitation_authority ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_authority_holds: "holds Authority ctx act" by (simp add: act_def ctx_def broker_simps)
lemma rule_authority_fails: "\<not> holds Authority ctx act" by (simp add: act_def ctx_def broker_simps)

end
