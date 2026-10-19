# Phase 1 spike results (gate G1)

**Run:** 2026-10-18 on the target machine: Windows 11, 64 GB RAM, Quadro T2000 with 4 GB.
**Versions:** Isabelle2025-2, Ollama 0.40.5, `claude-opus-5-5`, Rust 1.97.1.
**Spec:** [docs/isabelle-calendar-demo.md](../../docs/isabelle-calendar-demo.md), tasks T1.1–T1.3. How to rerun: [README.md](README.md).

## Summary

| Question | Answer |
|---|---|
| Can Rust drive Isabelle on Windows? | Yes. `isabelle server` starts through the bundled Cygwin, and the TCP protocol works with `/cygdrive/c/...` paths as `master_dir`. |
| Can one pass prove each rule or its negation (D6)? | Yes. A failed proof does not stop Isabelle from checking the later lemmas of the same theory. |
| Are the verdicts right? | 9 of 9 screening verdicts correct in every run, each denial attributed to the right rule. |
| How fast is a check? | About 1.0 s for one action and 1.5 s for a 9-action screening batch, once `headless_consolidate_delay` is lowered to 0.1 s. The default 2.0 s puts a floor of about 2 s under every check. |
| Does Claude plan well with the schema? | Yes, after one fix. 12 of 12 valid outputs; the correct plan in all 5 turns once a fixture bug was fixed; about 7 s and 3 cents per turn. |
| Does a small local model plan well? | No. All three candidates return valid JSON, but pick the correct plan in only 2–4 of 10 turns, and take 20–136 s per turn. |

## T1.1 — Isabelle server from Rust

`src/bin/t1_isabelle.rs` starts the server, opens a HOL session from the prebuilt heap, and runs generated theories against [theories/Spike_Policy.thy](theories/Spike_Policy.thy), a 7-rule stand-in shaped like the real policy (records, an action datatype, one function per rule, notes as character codes). Each check states every rule and its negation as lemmas, proven with `simp` (kernel-checked, D7).

**E1, failed proofs.** In a theory with a failing lemma, a provable one, then another failing one, Isabelle reported errors at exactly the two failing lines and proved the middle one; node status `failed: 2, finished: 16, unprocessed: 0`. The verdict procedure in D6 works in a single pass, with no second "evaluate first" round.

**Verdicts.** The 9-action batch covered 3 permits (read inside the window, a good proposal, a second-round proposal) and 6 denials (event details, read past the window, 5 candidates, a note with digits, an off-grid start, a start after the window). Every run returned exactly the expected verdict and rule for each. `value` returned the numbers that denial messages quote, such as the candidate count of 5.

**Timing.** Seconds; warm runs repeat a check with a new theory name each time, purging after each.

| `headless_consolidate_delay` | Server ready | HOL session | One action, first | One action, warm | 9-action batch (126 lemmas) |
|---|---|---|---|---|---|
| 2.0 (default) | 10.0 | 21.3 | 4.22 | 2.08–2.10 | 2.83 (2.12–4.20) |
| 0.1 | 5.4 | 8.9 | 3.01 | 0.99–1.13 | 1.45 (1.34–1.55) |
| 0.02 | 5.1 | 7.9 | 2.81 | 1.07–1.12 | 1.60 (1.41–1.79) |

The first run's slower startup probably reflects cold disk caches. `use_theories`' own `check_delay` (0.5 vs 0.05 s) made no difference. Purging the first check theory took under 10 ms; later purges were not timed separately.

**Operational findings.**
- On Windows, `session_stop` always reports `return code 130 (INTERRUPT)`. The demo must treat this as normal and still send `shutdown`. The first run treated it as fatal and skipped `shutdown`, so the server's Java process kept running and held the parent console's output handle.
- The server's per-session `tmp_dir` is under the user's Temp folder; check theories should live in a repo-local `master_dir` instead, as the spike does.

## T1.3 — Claude (`claude-opus-5-5`)

Raw HTTPS, adaptive thinking, `effort: "medium"`, structured output with the plan schema, `fallbacks: "default"`, and the `anthropic-workspace-id` header.

**First attempt: placeholder plans.** Four of five outputs were schema-valid but empty of content ("label": "placeholder", one plan that just closes the negotiation), and one turn ended with `stop_reason: refusal`. Diagnostics (`t1_claude_debug`) showed the model planned well without structured output, and still degenerated with it at `effort: "high"` even after sensible thinking. **Cause:** `serde_json` sorts object keys by default, so the schema listed every object's properties alphabetically. Constrained decoding follows that order, which forced `actions` before `label` and `rationale`, and fields such as `end` before `kind`; the model only chose actions whose `kind` sorts first. **Fix:** keep insertion order (`serde_json` `preserve_order`), put `kind` first in every action, and order each plan as label, rationale, actions. Neither placeholders nor refusals occurred in the 13 calls after the fix (one diagnostic call and the 12 below).

**After the fix.**

| Run | Calls | Schema-valid | Every plan passes the pre-check | Correct plan picked | Mean / max latency | Cost per call |
|---|---|---|---|---|---|---|
| All five turns, 2 repetitions | 10 | 10 | 10 | 8 | 7.0 s / 9.2 s | $0.027 |
| Turn E again, after the fixture fix | 2 | 2 | 2 | 2 | 8.7 s / 9.4 s | $0.033 |

The two misses were a fixture bug, not the model: turn E's state said `decisions_exchanged`, so Claude correctly skipped sending Alice's decisions. With the state fixed to say they were not yet sent, it planned `send_decide`, `add_meeting` (the right meeting, c3), `send_close` both times. Every turn was served by `claude-opus-5-5`; the fallback never triggered. Inputs were 3,000–3,900 tokens and outputs 400–1,000 per turn. Bob takes about four turns in a negotiation, roughly 12 cents per run. Phase 1's total Claude spend, including diagnostics, was about $0.49.

## T1.2 — Local models through Ollama

Same five turns and schema; `think: false`, temperature 0, `num_ctx` 8192, schema also included in the prompt as Ollama recommends. Two repetitions per turn.

| Model | Size | GPU share | Schema-valid | Correct plan picked | Every plan passes the pre-check | Mean / max latency |
|---|---|---|---|---|---|---|
| `granite4.1:3b` | 2.1 GB | 75–85% | 10/10 | 2/10 | 7/10 | 20 s / 35 s |
| `qwen3:4b` | 2.5 GB | about 66% | 10/10 | 2/10 | 7/10 | 64 s / 82 s |
| `qwen3.5:4b` | 3.3 GB | about 48% | 10/10 | 4/10 | 4/10 | 136 s / 253 s |

Correct plans by turn (A–E, out of 2 each): `granite4.1:3b` 2-0-0-0-0, `qwen3:4b` 2-0-0-0-0, `qwen3.5:4b` 0-0-0-2-2. Claude after the fixes: 2-2-2-2-2.

**Failure modes.**
1. **Re-reading instead of acting.** Granite and qwen3 asked to read the calendar again in turns B, D and E, although the free/busy data was in their input.
2. **Rationale and actions disagree.** Granite's turn-E rationale correctly chose c3, but its only action was another calendar read.
3. **Weak time reasoning.** `qwen3.5:4b` proposed Monday evening, a morning slot, and a time Alice is busy, despite her stated preference and calendar. These pass the rules, so Isabelle cannot catch them.
4. **Rule violations Isabelle would catch:** reading before other actions, and notes with digits; for example "Only c2 is available" breaks the no-digits rule because of the candidate ID.

**Implication.** Isabelle keeps a weak local planner safe, but it cannot make it useful: prompted the same way as Claude, a 3–4B model on this machine would stall the demo or propose bad times, at minutes per turn.

## Decisions for gate G1

| Item | Result | Proposed spec change |
|---|---|---|
| D6, prove the rule or its negation | Confirmed in one pass | None |
| D7, kernel-checked `simp` | Confirmed; about 1 s per action | None |
| D8, batched screening | Confirmed; 126 lemmas in about 1.5 s | None |
| Isabelle session options | Default adds about 1 s per check | Start sessions with `headless_consolidate_delay=0.1`; always send `shutdown` after `session_stop` |
| Plan schema | Key order decides output quality | Emit schemas in a fixed property order with `kind` first and reasoning fields before actions; add a test that checks the order |
| D25, Claude settings | Confirmed: `claude-opus-5-5`, adaptive thinking, `effort: "medium"`, fallbacks | None |
| D25, Alice's local model | **Open: none of the three is good enough as tested** | Your choice, below |

Options for Alice's local model:
1. **Guided planning, tested in a short follow-up spike (recommended).** The runtime gives the local model a compact state with precomputed facts: free grid-aligned slots that match the owner's preference, the received candidate IDs, and which action kinds are possible next. It also drops the inline schema. The model still writes alternative plans, and Isabelle still checks every action. Measure `qwen3.5:4b` and `qwen3:4b` with thinking off and on.
2. **A larger local model.** For example `qwen3.5:9b` (6.6 GB), mostly on the CPU: likely better plans, likely several minutes per turn.
3. **Claude for both agents by default.** Keep Ollama as a per-run option. This changes Q5.
4. **Keep the setup as tested.** Rely on re-planning, accepting that the demo may stall.
