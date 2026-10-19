theory Fabric_Policy
  imports Main
begin

section \<open>Access rules for the Fabric calendar demo\<close>

text \<open>
  This theory is the contract for docs/isabelle-calendar-demo.md, section 8. Before an agent
  action runs, the policy gate writes a check theory that imports this one, defines the concrete
  context and action, and asks Isabelle to prove, for every rule, either that it holds or that it
  fails. The action is permitted only when every rule is proven to hold.

  Conventions: times are UTC milliseconds since the Unix epoch and lengths are milliseconds;
  identifiers (people's key IDs, negotiation and candidate IDs) are strings; notes are lists of
  character codes, so free text never has to be escaped into HOL syntax.
\<close>

subsection \<open>Parameters\<close>

text \<open>The defaults from calendar.md section 23.7 and the demo specification, section 8.3.\<close>

definition max_candidates :: nat where "max_candidates = 3"
definition max_rounds :: nat where "max_rounds = 3"
definition horizon_ms :: int where "horizon_ms = 14 * 24 * 60 * 60 * 1000"
definition grid_ms :: int where "grid_ms = 15 * 60 * 1000"
definition consent_ttl_ms :: int where "consent_ttl_ms = 24 * 60 * 60 * 1000"
definition note_max :: nat where "note_max = 200"

lemmas parameter_defs =
  max_candidates_def max_rounds_def horizon_ms_def grid_ms_def consent_ttl_ms_def note_max_def

subsection \<open>What the trusted host knows\<close>

text \<open>
  The gate builds these facts from its own records, never from the agent's claims.
  A request is the owner's pre-authorization: whom to meet, for how long, inside which window,
  and until when the negotiation may run.
\<close>

record request =
  req_peer :: string
  req_duration :: int
  req_window_start :: int
  req_window_end :: int
  req_opened_at :: int
  req_expires_at :: int

record candidate =
  cand_id :: string
  cand_start :: int
  cand_length :: int

record answer =
  ans_id :: string
  ans_available :: bool

record decision =
  dec_id :: string
  dec_accept :: bool

datatype role = Coordinator | Invitee

text \<open>
  The negotiation as the messaging gateway has seen it. neg_round is the current round
  (0 before the first proposal); neg_candidates are this round's candidates, as proposed
  by the coordinator or as received by the invitee; neg_mutual lists the IDs of the
  mutual options in proposal order.
\<close>

record negotiation =
  neg_id :: string
  neg_role :: role
  neg_round :: nat
  neg_handshake :: bool
  neg_candidates :: "candidate list"
  neg_mutual :: "string list"

text \<open>
  A consent record whose signature the gate has already verified. Unverified records never
  reach Isabelle.
\<close>

record consent =
  con_signer :: string
  con_negotiation :: string
  con_start :: int
  con_length :: int
  con_yes :: bool
  con_expires :: int

record gate_context =
  ctx_owner :: string
  ctx_now :: int
  ctx_request :: request
  ctx_negotiation :: negotiation
  ctx_consents :: "consent list"

subsection \<open>Agent actions\<close>

datatype close_reason = Agreed | No_match | Withdrawn

text \<open>
  Arguments, in order: ReadFreeBusy and ReadEventDetails take a window start and
  end; Handshake takes the peer's key ID; each Send action takes the negotiation
  ID, then the round (or close reason), then its payload, and ends with the note;
  AddMeeting takes the negotiation ID, the meeting start and its length.
\<close>

datatype action =
    ReadFreeBusy int int
  | ReadEventDetails int int
  | Handshake string
  | SendPropose string nat "candidate list" "nat list"
  | SendRespond string nat "answer list" "nat list"
  | SendDecide string nat "decision list" "nat list"
  | SendClose string close_reason "nat list"
  | AddMeeting string int int

fun is_send :: "action \<Rightarrow> bool" where
  "is_send (SendPropose _ _ _ _) = True"
| "is_send (SendRespond _ _ _ _) = True"
| "is_send (SendDecide _ _ _ _) = True"
| "is_send (SendClose _ _ _) = True"
| "is_send _ = False"

fun note_of :: "action \<Rightarrow> nat list" where
  "note_of (SendPropose _ _ _ n) = n"
| "note_of (SendRespond _ _ _ n) = n"
| "note_of (SendDecide _ _ _ n) = n"
| "note_of (SendClose _ _ n) = n"
| "note_of _ = []"

text \<open>The number of candidates, answers or decisions; denial messages quote it.\<close>

fun payload_count :: "action \<Rightarrow> nat" where
  "payload_count (SendPropose _ _ cs _) = length cs"
| "payload_count (SendRespond _ _ ans _) = length ans"
| "payload_count (SendDecide _ _ ds _) = length ds"
| "payload_count _ = 0"

subsection \<open>Conditions used by the rules\<close>

definition inside_window :: "request \<Rightarrow> int \<Rightarrow> int \<Rightarrow> bool" where
  "inside_window r s e \<longleftrightarrow> req_window_start r \<le> s \<and> s < e \<and> e \<le> req_window_end r"

definition window_ok :: "gate_context \<Rightarrow> bool" where
  "window_ok c \<longleftrightarrow> req_window_end (ctx_request c) - req_window_start (ctx_request c) \<le> horizon_ms"

definition candidate_ok :: "gate_context \<Rightarrow> candidate \<Rightarrow> bool" where
  "candidate_ok c x \<longleftrightarrow>
     req_window_start (ctx_request c) \<le> cand_start x \<and>
     cand_start x + cand_length x \<le> req_window_end (ctx_request c) \<and>
     ctx_now c < cand_start x"

text \<open>Printable ASCII (character codes 32 to 126) with no digits (codes 48 to 57).\<close>

definition note_ok :: "nat list \<Rightarrow> bool" where
  "note_ok n \<longleftrightarrow>
     length n \<le> note_max \<and> (\<forall>ch \<in> set n. 32 \<le> ch \<and> ch \<le> 126 \<and> \<not> (48 \<le> ch \<and> ch \<le> 57))"

definition said_yes :: "gate_context \<Rightarrow> string \<Rightarrow> string \<Rightarrow> int \<Rightarrow> int \<Rightarrow> bool" where
  "said_yes c who n s l \<longleftrightarrow>
     (\<exists>x \<in> set (ctx_consents c).
        con_signer x = who \<and> con_negotiation x = n \<and> con_start x = s \<and> con_length x = l \<and>
        con_yes x \<and> ctx_now c < con_expires x)"

text \<open>A decision must equal the owner's own signed record for that candidate.\<close>

definition decision_matches :: "gate_context \<Rightarrow> decision \<Rightarrow> bool" where
  "decision_matches c d \<longleftrightarrow>
     (\<exists>x \<in> set (neg_candidates (ctx_negotiation c)). cand_id x = dec_id d \<and>
        (\<exists>r \<in> set (ctx_consents c).
           con_signer r = ctx_owner c \<and> con_negotiation r = neg_id (ctx_negotiation c) \<and>
           con_start r = cand_start x \<and> con_length r = cand_length x \<and> con_yes r = dec_accept d))"

fun well_formed :: "negotiation \<Rightarrow> gate_context \<Rightarrow> action \<Rightarrow> bool" where
  "well_formed g c (SendPropose n r cs _) \<longleftrightarrow>
     neg_handshake g \<and> n = neg_id g \<and> neg_role g = Coordinator \<and> r = Suc (neg_round g) \<and>
     distinct (map cand_id cs)"
| "well_formed g c (SendRespond n r ans _) \<longleftrightarrow>
     neg_handshake g \<and> n = neg_id g \<and> neg_role g = Invitee \<and> r = neg_round g \<and>
     map ans_id ans = map cand_id (neg_candidates g)"
| "well_formed g c (SendDecide n r ds _) \<longleftrightarrow>
     neg_handshake g \<and> n = neg_id g \<and> r = neg_round g \<and> map dec_id ds = neg_mutual g \<and>
     (\<forall>d \<in> set ds. decision_matches c d)"
| "well_formed g c (SendClose n _ _) \<longleftrightarrow> neg_handshake g \<and> n = neg_id g"
| "well_formed g c _ \<longleftrightarrow> True"

subsection \<open>The rules\<close>

datatype rule =
    View_details_never
  | Read_window
  | Cand_count
  | Answer_count
  | Round_limit
  | Horizon
  | Grid
  | Consent_lifetime
  | Note
  | Handshake_target
  | Consent
  | Well_formed

definition all_rules :: "rule list" where
  "all_rules =
     [View_details_never, Read_window, Cand_count, Answer_count, Round_limit, Horizon, Grid,
      Consent_lifetime, Note, Handshake_target, Consent, Well_formed]"

text \<open>Which actions each rule governs (specification section 8.2, column "Applies to").\<close>

fun applies :: "rule \<Rightarrow> action \<Rightarrow> bool" where
  "applies View_details_never a = (case a of ReadEventDetails _ _ \<Rightarrow> True | _ \<Rightarrow> False)"
| "applies Read_window a = (case a of ReadFreeBusy _ _ \<Rightarrow> True | _ \<Rightarrow> False)"
| "applies Cand_count a = (case a of SendPropose _ _ _ _ \<Rightarrow> True | _ \<Rightarrow> False)"
| "applies Answer_count a = (case a of SendRespond _ _ _ _ \<Rightarrow> True | _ \<Rightarrow> False)"
| "applies Round_limit a =
     (case a of SendPropose _ _ _ _ \<Rightarrow> True | SendRespond _ _ _ _ \<Rightarrow> True | _ \<Rightarrow> False)"
| "applies Horizon a =
     (case a of SendPropose _ _ _ _ \<Rightarrow> True | SendRespond _ _ _ _ \<Rightarrow> True | _ \<Rightarrow> False)"
| "applies Grid a = (case a of SendPropose _ _ _ _ \<Rightarrow> True | _ \<Rightarrow> False)"
| "applies Consent_lifetime a = is_send a"
| "applies Note a = is_send a"
| "applies Handshake_target a = (case a of Handshake _ \<Rightarrow> True | _ \<Rightarrow> False)"
| "applies Consent a = (case a of AddMeeting _ _ _ \<Rightarrow> True | _ \<Rightarrow> False)"
| "applies Well_formed a = is_send a"

text \<open>What each rule requires of an action it governs (column "Holds when").\<close>

fun satisfied :: "rule \<Rightarrow> gate_context \<Rightarrow> action \<Rightarrow> bool" where
  "satisfied View_details_never c a = False"
| "satisfied Read_window c a =
     (case a of ReadFreeBusy s e \<Rightarrow> inside_window (ctx_request c) s e | _ \<Rightarrow> True)"
| "satisfied Cand_count c a =
     (case a of SendPropose _ _ cs _ \<Rightarrow> 1 \<le> length cs \<and> length cs \<le> max_candidates | _ \<Rightarrow> True)"
| "satisfied Answer_count c a =
     (case a of SendRespond _ _ ans _ \<Rightarrow> length ans \<le> max_candidates | _ \<Rightarrow> True)"
| "satisfied Round_limit c a =
     (case a of
        SendPropose _ r _ _ \<Rightarrow> 1 \<le> r \<and> r \<le> max_rounds
      | SendRespond _ r _ _ \<Rightarrow> 1 \<le> r \<and> r \<le> max_rounds
      | _ \<Rightarrow> True)"
| "satisfied Horizon c a =
     (case a of
        SendPropose _ _ cs _ \<Rightarrow> window_ok c \<and> (\<forall>x \<in> set cs. candidate_ok c x)
      | SendRespond _ _ _ _ \<Rightarrow>
          window_ok c \<and> (\<forall>x \<in> set (neg_candidates (ctx_negotiation c)). candidate_ok c x)
      | _ \<Rightarrow> True)"
| "satisfied Grid c a =
     (case a of
        SendPropose _ _ cs _ \<Rightarrow>
          (\<forall>x \<in> set cs. cand_start x mod grid_ms = 0 \<and> cand_length x = req_duration (ctx_request c))
      | _ \<Rightarrow> True)"
| "satisfied Consent_lifetime c a =
     (req_expires_at (ctx_request c) - req_opened_at (ctx_request c) \<le> consent_ttl_ms \<and>
      ctx_now c < req_expires_at (ctx_request c))"
| "satisfied Note c a = note_ok (note_of a)"
| "satisfied Handshake_target c a =
     (case a of Handshake k \<Rightarrow> k = req_peer (ctx_request c) | _ \<Rightarrow> True)"
| "satisfied Consent c a =
     (case a of
        AddMeeting n s l \<Rightarrow> said_yes c (ctx_owner c) n s l \<and> said_yes c (req_peer (ctx_request c)) n s l
      | _ \<Rightarrow> True)"
| "satisfied Well_formed c a = well_formed (ctx_negotiation c) c a"

definition holds :: "rule \<Rightarrow> gate_context \<Rightarrow> action \<Rightarrow> bool" where
  "holds r c a \<longleftrightarrow> (applies r a \<longrightarrow> satisfied r c a)"

definition permitted :: "gate_context \<Rightarrow> action \<Rightarrow> bool" where
  "permitted c a \<longleftrightarrow> (\<forall>r \<in> set all_rules. holds r c a)"

text \<open>The simplification rules every check uses.\<close>

lemmas policy_simps =
  holds_def permitted_def all_rules_def parameter_defs inside_window_def window_ok_def
  candidate_ok_def note_ok_def said_yes_def decision_matches_def

subsection \<open>Sanity lemmas\<close>

text \<open>Properties of the rules themselves, proven once when the session builds (decision D10).\<close>

lemma event_details_never_permitted: "\<not> permitted c (ReadEventDetails s e)"
  by (simp add: permitted_def holds_def all_rules_def)

lemma add_meeting_needs_both_owners_yes:
  assumes "permitted c (AddMeeting n s l)"
  shows "said_yes c (ctx_owner c) n s l \<and> said_yes c (req_peer (ctx_request c)) n s l"
  using assms by (simp add: permitted_def holds_def all_rules_def)

lemma proposals_have_one_to_three_candidates:
  assumes "permitted c (SendPropose n r cs note)"
  shows "1 \<le> length cs \<and> length cs \<le> 3"
  using assms by (simp add: permitted_def holds_def all_rules_def max_candidates_def)

lemma handshake_only_with_the_requested_peer:
  assumes "permitted c (Handshake k)"
  shows "k = req_peer (ctx_request c)"
  using assms by (simp add: permitted_def holds_def all_rules_def)

lemma reads_stay_inside_the_window:
  assumes "permitted c (ReadFreeBusy s e)"
  shows "inside_window (ctx_request c) s e"
  using assms by (simp add: permitted_def holds_def all_rules_def)

lemma sent_notes_have_no_digits:
  assumes "permitted c a" and "is_send a" and "ch \<in> set (note_of a)"
  shows "\<not> (48 \<le> ch \<and> ch \<le> 57)"
  using assms by (auto simp add: permitted_def holds_def all_rules_def note_ok_def)

end
