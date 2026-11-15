theory Check_000001
  imports "Fabric_Policy.Fabric_Policy"
begin

definition ctx :: gate_context where "ctx = \<lparr>ctx_owner = ''alice'', ctx_now = 0, ctx_request = \<lparr>req_peer = ''bob'', req_duration = 1800000, req_window_start = 0, req_window_end = 3600000, req_opened_at = 0, req_expires_at = 3600000\<rparr>, ctx_negotiation = \<lparr>neg_id = ''n1'', neg_role = Coordinator, neg_round = 0, neg_handshake = True, neg_candidates = [], neg_mutual = []\<rparr>, ctx_consents = []\<rparr>"

definition a0 :: action where "a0 = Handshake ''bob''"
lemma a0_view_details_never_holds: "holds View_details_never ctx a0" by (simp add: a0_def ctx_def policy_simps)
lemma a0_view_details_never_fails: "\<not> holds View_details_never ctx a0" by (simp add: a0_def ctx_def policy_simps)
lemma a0_read_window_holds: "holds Read_window ctx a0" by (simp add: a0_def ctx_def policy_simps)
lemma a0_read_window_fails: "\<not> holds Read_window ctx a0" by (simp add: a0_def ctx_def policy_simps)
lemma a0_cand_count_holds: "holds Cand_count ctx a0" by (simp add: a0_def ctx_def policy_simps)
lemma a0_cand_count_fails: "\<not> holds Cand_count ctx a0" by (simp add: a0_def ctx_def policy_simps)
lemma a0_answer_count_holds: "holds Answer_count ctx a0" by (simp add: a0_def ctx_def policy_simps)
lemma a0_answer_count_fails: "\<not> holds Answer_count ctx a0" by (simp add: a0_def ctx_def policy_simps)
lemma a0_round_limit_holds: "holds Round_limit ctx a0" by (simp add: a0_def ctx_def policy_simps)
lemma a0_round_limit_fails: "\<not> holds Round_limit ctx a0" by (simp add: a0_def ctx_def policy_simps)
lemma a0_horizon_holds: "holds Horizon ctx a0" by (simp add: a0_def ctx_def policy_simps)
lemma a0_horizon_fails: "\<not> holds Horizon ctx a0" by (simp add: a0_def ctx_def policy_simps)
lemma a0_grid_holds: "holds Grid ctx a0" by (simp add: a0_def ctx_def policy_simps)
lemma a0_grid_fails: "\<not> holds Grid ctx a0" by (simp add: a0_def ctx_def policy_simps)
lemma a0_consent_lifetime_holds: "holds Consent_lifetime ctx a0" by (simp add: a0_def ctx_def policy_simps)
lemma a0_consent_lifetime_fails: "\<not> holds Consent_lifetime ctx a0" by (simp add: a0_def ctx_def policy_simps)
lemma a0_note_holds: "holds Note ctx a0" by (simp add: a0_def ctx_def policy_simps)
lemma a0_note_fails: "\<not> holds Note ctx a0" by (simp add: a0_def ctx_def policy_simps)
lemma a0_handshake_target_holds: "holds Handshake_target ctx a0" by (simp add: a0_def ctx_def policy_simps)
lemma a0_handshake_target_fails: "\<not> holds Handshake_target ctx a0" by (simp add: a0_def ctx_def policy_simps)
lemma a0_consent_holds: "holds Consent ctx a0" by (simp add: a0_def ctx_def policy_simps)
lemma a0_consent_fails: "\<not> holds Consent ctx a0" by (simp add: a0_def ctx_def policy_simps)
lemma a0_well_formed_holds: "holds Well_formed ctx a0" by (simp add: a0_def ctx_def policy_simps)
lemma a0_well_formed_fails: "\<not> holds Well_formed ctx a0" by (simp add: a0_def ctx_def policy_simps)

definition a1 :: action where "a1 = SendPropose ''n1'' 1 [\<lparr>cand_id = ''c1'', cand_start = 900000, cand_length = 1800000\<rparr>] [104, 105]"
lemma a1_view_details_never_holds: "holds View_details_never ctx a1" by (simp add: a1_def ctx_def policy_simps)
lemma a1_view_details_never_fails: "\<not> holds View_details_never ctx a1" by (simp add: a1_def ctx_def policy_simps)
lemma a1_read_window_holds: "holds Read_window ctx a1" by (simp add: a1_def ctx_def policy_simps)
lemma a1_read_window_fails: "\<not> holds Read_window ctx a1" by (simp add: a1_def ctx_def policy_simps)
lemma a1_cand_count_holds: "holds Cand_count ctx a1" by (simp add: a1_def ctx_def policy_simps)
lemma a1_cand_count_fails: "\<not> holds Cand_count ctx a1" by (simp add: a1_def ctx_def policy_simps)
lemma a1_answer_count_holds: "holds Answer_count ctx a1" by (simp add: a1_def ctx_def policy_simps)
lemma a1_answer_count_fails: "\<not> holds Answer_count ctx a1" by (simp add: a1_def ctx_def policy_simps)
lemma a1_round_limit_holds: "holds Round_limit ctx a1" by (simp add: a1_def ctx_def policy_simps)
lemma a1_round_limit_fails: "\<not> holds Round_limit ctx a1" by (simp add: a1_def ctx_def policy_simps)
lemma a1_horizon_holds: "holds Horizon ctx a1" by (simp add: a1_def ctx_def policy_simps)
lemma a1_horizon_fails: "\<not> holds Horizon ctx a1" by (simp add: a1_def ctx_def policy_simps)
lemma a1_grid_holds: "holds Grid ctx a1" by (simp add: a1_def ctx_def policy_simps)
lemma a1_grid_fails: "\<not> holds Grid ctx a1" by (simp add: a1_def ctx_def policy_simps)
lemma a1_consent_lifetime_holds: "holds Consent_lifetime ctx a1" by (simp add: a1_def ctx_def policy_simps)
lemma a1_consent_lifetime_fails: "\<not> holds Consent_lifetime ctx a1" by (simp add: a1_def ctx_def policy_simps)
lemma a1_note_holds: "holds Note ctx a1" by (simp add: a1_def ctx_def policy_simps)
lemma a1_note_fails: "\<not> holds Note ctx a1" by (simp add: a1_def ctx_def policy_simps)
lemma a1_handshake_target_holds: "holds Handshake_target ctx a1" by (simp add: a1_def ctx_def policy_simps)
lemma a1_handshake_target_fails: "\<not> holds Handshake_target ctx a1" by (simp add: a1_def ctx_def policy_simps)
lemma a1_consent_holds: "holds Consent ctx a1" by (simp add: a1_def ctx_def policy_simps)
lemma a1_consent_fails: "\<not> holds Consent ctx a1" by (simp add: a1_def ctx_def policy_simps)
lemma a1_well_formed_holds: "holds Well_formed ctx a1" by (simp add: a1_def ctx_def policy_simps)
lemma a1_well_formed_fails: "\<not> holds Well_formed ctx a1" by (simp add: a1_def ctx_def policy_simps)
value "payload_count a1"
value "length (note_of a1)"

end
