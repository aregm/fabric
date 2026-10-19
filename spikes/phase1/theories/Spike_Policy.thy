theory Spike_Policy
  imports Main
begin

text \<open>A cut-down stand-in for Fabric_Policy, shaped like the real rules, to measure how
  Isabelle behaves when it is asked to prove each rule or its negation for concrete actions.
  Times are UTC milliseconds; notes are lists of character codes.\<close>

record request =
  window_start :: int
  window_end :: int
  duration :: int

record candidate =
  cstart :: int
  cdur :: int

datatype action =
    ReadFreeBusy int int
  | ReadEventDetails int int
  | SendPropose nat "candidate list" "nat list"

definition max_candidates :: nat where "max_candidates = 3"
definition max_rounds :: nat where "max_rounds = 3"
definition grid_ms :: int where "grid_ms = 900000"
definition horizon_ms :: int where "horizon_ms = 1209600000"
definition note_max :: nat where "note_max = 200"

lemmas policy_defs = max_candidates_def max_rounds_def grid_ms_def horizon_ms_def note_max_def

fun cands_of :: "action \<Rightarrow> candidate list" where
  "cands_of (SendPropose _ cs _) = cs"
| "cands_of _ = []"

fun view_details_never :: "action \<Rightarrow> bool" where
  "view_details_never (ReadEventDetails _ _) = False"
| "view_details_never _ = True"

fun read_window :: "request \<Rightarrow> action \<Rightarrow> bool" where
  "read_window req (ReadFreeBusy s e) = (window_start req \<le> s \<and> s < e \<and> e \<le> window_end req)"
| "read_window _ _ = True"

fun cand_count :: "action \<Rightarrow> bool" where
  "cand_count (SendPropose _ cs _) = (1 \<le> length cs \<and> length cs \<le> max_candidates)"
| "cand_count _ = True"

fun round_limit :: "action \<Rightarrow> bool" where
  "round_limit (SendPropose r _ _) = (1 \<le> r \<and> r \<le> max_rounds)"
| "round_limit _ = True"

fun horizon :: "request \<Rightarrow> int \<Rightarrow> action \<Rightarrow> bool" where
  "horizon req clock (SendPropose _ cs _) =
     (window_end req - window_start req \<le> horizon_ms \<and>
      (\<forall>c \<in> set cs. window_start req \<le> cstart c \<and> cstart c + cdur c \<le> window_end req \<and> clock < cstart c))"
| "horizon _ _ _ = True"

fun grid :: "request \<Rightarrow> action \<Rightarrow> bool" where
  "grid req (SendPropose _ cs _) = (\<forall>c \<in> set cs. cstart c mod grid_ms = 0 \<and> cdur c = duration req)"
| "grid _ _ = True"

fun note_ok :: "action \<Rightarrow> bool" where
  "note_ok (SendPropose _ _ n) =
     (length n \<le> note_max \<and> (\<forall>ch \<in> set n. 32 \<le> ch \<and> ch \<le> 126 \<and> \<not> (48 \<le> ch \<and> ch \<le> 57)))"
| "note_ok _ = True"

end
