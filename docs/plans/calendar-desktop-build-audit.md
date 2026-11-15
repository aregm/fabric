# Fabric local build and synchronization audit

Date: 2026-11-15 (America/Chicago). Checkout: `/Users/intel/Documents/ChatGPT/fabric`. Host: `sphynx`; architecture: `arm64`.

## Git state and preservation

The starting branch was `codex/design-calendar-guided-architecture`, containing local architecture commit `0ea3f33` and untracked `.agents/` skill files. After fetching, GitHub main was `4fb3330`, 40 commits ahead of the branch's common baseline. The skill files' Git blob hashes matched those on remote main exactly.

The skill files were recorded as preservation commit `b393512`. Main was merged as `54dd653`; the only content conflict was the README documentation list, resolved by retaining both sets of links. The architecture document `docs/system-design.md` was preserved. The inactive local `main` was fast-forwarded to `4fb3330`. No source work was discarded. The desktop planning work uses branch `codex/calendar-desktop-plan`.

For current publication status and exact plan commit, use:

```sh
git status --short --branch
git rev-parse HEAD
git rev-parse main origin/main
git rev-list --left-right --count HEAD...origin/codex/calendar-desktop-plan
```

The last command is meaningful after the plan branch has been published. A `0 0` comparison means the active local branch and its remote match. It does not mean the proposed planning changes have been merged into GitHub main.

## Verified environment

| Component | Observation |
|---|---|
| macOS | 27.0.1, build 26A434 |
| Xcode | 27.1, build 27A9275; active macOS SDK available |
| Swift | 6.4, arm64 target |
| Rust/Cargo | Repository-local 1.97.1, aarch64-apple-darwin |
| Claude Code | 2.1.318 |
| Codex CLI | 0.169.0-alpha.11.1; MCP management command present |
| Ollama client | 0.40.8; no running server confirmed by version command |
| Isabelle | Not installed in the repository-local location |

No client account configurations, API keys, refresh tokens, Google calendars, or personal messages were inspected. Installed CLI support does not establish desktop-client integration.

## Build evidence after integrating current main

```sh
sh scripts/verify.sh
sh scripts/run.sh cargo run --offline --locked -p fabric-cli -- demo-meeting
sh scripts/isabelle.sh version
```

- Verification passed formatting, workspace check, Clippy with warnings denied, tests, and documentation.
- Rust tests: **93 passed, 0 failed, 11 ignored**.
- Missing locked Cargo dependencies were downloaded after network approval; the simulation subsequently ran offline.
- Simulation output includes `result=agreed`, `selected=c3`, `agents_hold_identical_record=true`, `scheduled=false`, `e2ee=false`, and `provider_writes=false`.
- Isabelle version check failed with the actionable message to run `sh scripts/setup-demo.sh` because the local installation is absent.

Ignored tests require Isabelle (six), OS credential-store/command execution (three), a Claude API call (one), or Ollama (one). They were not run. No live external effect or paid model call was made.

The updated Rust code is buildable. There is no desktop application or live Google broker to build yet. The existing iOS Expo project is outside the Cargo workspace; its npm dependencies, iOS build, and tests were not installed/run in this Calendar-desktop planning audit. Historical mobile test results in other documents are not new verification evidence.

## Boundaries of the result

This audit establishes the integrated existing Rust workspace compiles and its ordinary tests pass on this Mac. It does not establish live Isabelle operation, Keychain command integration, cloud/local model operation, Google OAuth, invitation delivery, native GUI behavior, signing/notarization, or another Mac's compatibility. Those are explicit gates in the standalone implementation plan.
