# Phase 1 spikes

Throwaway experiments for gate G1 of [the demo specification](../../docs/isabelle-calendar-demo.md).
This is a standalone Cargo workspace, so the main workspace stays dependency-free and CI does not build it.
Results are in [RESULTS.md](RESULTS.md).

| Binary | Task | What it does |
|---|---|---|
| `t1_isabelle` | T1.1 | Starts `isabelle server` through Isabelle's bundled Cygwin, opens a HOL session, and times checks that prove each rule or its negation for concrete actions ([theories/Spike_Policy.thy](theories/Spike_Policy.thy)). |
| `t1_planners` | T1.2, T1.3 | Sends five scripted turns from the demo scenario to a planner backend with the plan schema, then checks schema validity, a rough rule pre-check, and whether the chosen plan is the right one. |
| `t1_claude_debug` | T1.3 | Sends one turn to Claude with request variants and prints the response shape; used to find why early plans degenerated. |

Run from the repository root, after `scripts/setup.ps1` and `scripts/setup-demo.ps1`:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run.ps1 cargo build --manifest-path spikes/phase1/Cargo.toml --bins
.\target\debug\t1_isabelle.exe --consolidate-delay 0.1
.\target\debug\t1_planners.exe ollama:granite4.1:3b 2
.\target\debug\t1_planners.exe claude 2
```

`t1_planners claude` and `t1_claude_debug` read `ANTHROPIC_API_KEY` and `ANTHROPIC_WORKSPACE_ID` from the git-ignored `.env` and never print them; each Claude call costs about three cents. Raw outputs and logs go to the git-ignored `.local/spikes/`.

The rule pre-check in `src/plans.rs` is a rough Rust stand-in used only to estimate how often plans would be denied. Isabelle is the policy decision in the demo itself.
