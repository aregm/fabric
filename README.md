# Fabric

Initial Rust workspace for the Personal Knowledge Fabric. The first executable
vertical is a deterministic, in-memory two-agent meeting-agreement simulator.

The scaffold begins as a small modular monolith:

- apps/fabric-cli — diagnostic CLI plus the `demo-meeting` executable fixture;
- crates/fabric-core — shared domain library, including checked calendar intervals and Rendezvous agreement semantics;
- apps/fabric-demo, crates/fabric-policy, crates/fabric-token, crates/fabric-wire and isabelle/ — the Isabelle-gated calendar demo below;
- apps/paracels — an Expo (React Native) iOS app outside the Cargo workspace, where the chat prototype is being built;
- scripts — isolated setup, command runner, and verification for Windows and macOS/Linux.

Google adapters, encrypted transport, persistence, and desktop crates should be added only when they contain real behavior.

## Product documentation

- [Product requirements](docs/PRD.md) — problem statement, market research, product experience, requirements, roadmap, and evaluation.
- [Technical architecture](docs/architecture.md) — platform architecture, data model, retrieval, agents, security, and implementation guidance.
- [Calendar vertical](docs/calendar.md) — the Google Calendar-first product test, private scheduling protocol, consent model, and release gates.
- [Paracels chat prototype](docs/paracels-chat.md) — a dummy chat in the paracels iOS app: decisions, design, protocol, and phased plan.
- [Architecture decision records](docs/adr/) — ADR-007 (operation log and sync evolution), ADR-009 (sharing crypto and metadata model) and ADR-014 (mobile client and messaging relay) are drafted; the rest are listed in the architecture document, section 19.

## Current executable slice

`fabric-schedule-sim/0` demonstrates the minimum honest agreement loop with two
independent agents and synthetic private calendars:

1. disclose one round of at most three exact UTC candidates;
2. evaluate only those candidates against each agent's private local intervals;
3. exchange candidate-specific eligibility bits, never calendar ranges or reasons;
4. record explicit owner `Yes`/`No` decisions;
5. select the first unanimously eligible and accepted candidate in proposal order;
6. let two independent reducer instances derive the same exact unsigned in-memory
   agreement and stable comparison key.

macOS or Linux:

    sh scripts/run.sh cargo run -p fabric-cli -- demo-meeting

Windows:

    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run.ps1 cargo run -p fabric-cli -- demo-meeting

The demo prints `result=agreed`, selects `c3`, and confirms that both agents hold
the same agreement record. It also prints its trust boundary explicitly:
`transport=in-process e2ee=false provider_writes=false` and `scheduled=false`.

This is a deterministic semantics and data-minimization/type-shape simulator—not
secure network scheduling or a participant-visibility model. Its stdout is a
coordinator-sensitive debug transcript: it intentionally displays proposed
times and bounded per-candidate bits and must not be treated as public-safe log
output. The underlying event records and conflict reasons remain agent-local.
Fixture IDs are bounded output-safe labels, not identity or authorization proof;
never place PII or provider identifiers in them.

It does not yet implement Google Calendar, a relay, MLS/TOFU, signatures,
encrypted persistence, recurrence/DST, holds/revalidation, or the provider
commit saga. Its result is **Agreed**, never **Scheduled**.

## Isabelle-gated calendar demo (Windows and macOS)

Two LLM agents, Alice's and Bob's, negotiate a meeting through an encrypted relay. Isabelle/HOL
proves every action they plan against explicit rules, and a single-use token bound to the
action's exact arguments gates every effect on a calendar or the messaging gateway. The
[specification](docs/isabelle-calendar-demo.md) describes the design, the rules and the results.

- isabelle/Fabric_Policy — the policy theory.
- crates/fabric-policy — actions, contexts, the HOL renderer, and the policy gate on Isabelle's server.
- crates/fabric-token — deterministic CBOR, COSE_Sign1 tokens, proof of possession, and keys in the OS credential store (Windows Credential Manager or the macOS login Keychain).
- crates/fabric-wire — frames, signed messages, Noise IK sessions, and consent records.
- apps/fabric-demo — the calendar service, relay, trusted host, agent runtime (Claude, Ollama or scripted), dashboard and launcher, in one binary.

Set up once. The second script downloads Isabelle2025-2 (about 1 GB on Windows, 1.7 GB on macOS) into `.tooling/isabelle/`.

Windows:

    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\setup.ps1
    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\setup-demo.ps1

macOS:

    sh scripts/setup.sh
    sh scripts/setup-demo.sh

Then copy `.env.example` to `.env` and fill in your Anthropic API key. For a local agent, install
Ollama and pull a model.

Run it, with Claude for both agents, or with a local model for Alice. Windows:

    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\demo.ps1
    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\demo.ps1 -Alice ollama:granite4.1:3b

macOS:

    sh scripts/demo.sh
    sh scripts/demo.sh --alice ollama:granite4.1:3b

It prints the dashboard URL. Open it, press **Start scenario**, and answer each mutual option in
both columns. The rehearsal script has Bob say no to Tuesday and both say yes to Thursday. The
agents then add the meeting to both calendars. The fault switches show Isabelle's denials and the
token checks. Stop with Ctrl+C, or from another terminal:

    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\demo.ps1 -Command down
    sh scripts/demo.sh down

- The first run creates both people's keys in the OS credential store: Windows Credential Manager under `fabric-demo/alice/` and `fabric-demo/bob/`, or the macOS login Keychain under the service `fabric-demo`. `-Command reset-keys` (Windows) or `sh scripts/demo.sh reset-keys` (macOS) deletes them. On macOS, a rebuilt `fabric-demo` makes macOS ask once per key whether the new binary may use it; choose Always Allow, or reset the keys first.
- Every run starts from the fixtures in `demo/fixtures/`. Logs and audit files go to `.local/demo/run/`.
- With Claude, both owners' free/busy and the agents' messages go to Anthropic. A rehearsal run cost about $0.35.
- A small local model usually stalls, for example by reading the calendar again instead of answering. Isabelle keeps it safe but cannot make it useful.
- It is not production security: see section 15 of the specification.

The tests that need Isabelle, the credential store, the API key or Ollama run only locally:

    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run.ps1 cargo test --workspace -- --ignored
    sh scripts/run.sh cargo test --workspace -- --ignored

On macOS, two of them (`calendar_command` and `messaging_commands`) read a Keychain item that another binary created, so macOS asks for permission; click Allow.

## Paracels chat prototype (iOS)

`apps/paracels` is an Expo (React Native) app for iOS that sits outside the Cargo workspace. The
[chat specification](docs/paracels-chat.md) plans a dummy group chat in it, as a first step toward
Fabric's messaging. Alice and Bob chat in one room from the iOS Simulator and an iPhone on the same
Wi-Fi. Expo Router API routes and a small WebSocket server on this Mac log every sent message to
SQLite and deliver it live, and unsent messages wait in an outbox on the phone.

The prototype keeps seams for Fabric's encryption, policy and local-first storage but implements
none of them: there is no TLS and no end-to-end encryption, and a shared room key is the only
protection.

Status: complete. All six phases are done and every gate, G0 through G6, is closed (2026-10-26).
Whether the prototype graduates is ADR-014 in [docs/architecture.md](docs/architecture.md).
Setup and run steps are in [apps/paracels/README.md](apps/paracels/README.md).
It needs macOS with Xcode, plus Node.js and npm, which are host prerequisites: the setup scripts
here install only Rust.

## Toolchain decision

Rust is installed with the official rustup bootstrap into repository-local directories:

| State | Location |
|---|---|
| rustup toolchains and settings | .tooling/rustup |
| Cargo proxy, registry, git cache, and installed Cargo tools | .tooling/cargo |
| build artifacts | target |

The scripts never modify a shell profile, user PATH, global rustup installation, or package-manager state. The compiler is pinned to Rust 1.97.1 in rust-toolchain.toml. The rustup bootstrap is pinned to 1.29.0 and its archive binary is checked against a committed official SHA-256 for each supported host.

This is installation, version, and cache isolation. It is not an operating-system sandbox: Cargo build scripts, procedural macros, the compiler, and linked tools execute with the current user’s privileges.

Chezmoi is intentionally not used because it manages workstation dotfiles rather than project toolchains. uv is intentionally not used because it manages Python. Mise is a reasonable future orchestrator when Fabric also needs Node, Python, protoc, or other runtimes, but adding it now would delegate Rust back to rustup while adding another bootstrap and configuration authority.

## Supported bootstrap hosts

- macOS arm64 and x86_64;
- GNU/Linux arm64 and x86_64;
- Windows arm64 and x86_64 using the MSVC toolchain.

Native linkers and SDKs remain host prerequisites:

- macOS: Xcode Command Line Tools; run xcode-select --install if missing.
- Windows: Visual Studio Build Tools with Desktop development with C++ and a Windows SDK.
- Linux: a C compiler, linker, and libc development package, commonly supplied by build-essential or an equivalent package.

The setup scripts detect missing prerequisites and explain them. They do not elevate privileges or install system packages.

## Set up

Windows PowerShell:

    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\setup.ps1

macOS or Linux:

    sh scripts/setup.sh

Setup is noninteractive and idempotent. It downloads the pinned rustup bootstrap from static.rust-lang.org, verifies its committed checksum, installs the exact toolchain and components locally, generates Cargo.lock if necessary, and runs all verification checks.

## Run commands in the isolated environment

Windows:

    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run.ps1 cargo run
    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run.ps1 cargo test --workspace --locked

macOS or Linux:

    sh scripts/run.sh cargo run
    sh scripts/run.sh cargo test --workspace --locked

The run wrappers set CARGO_HOME, RUSTUP_HOME, CARGO_TARGET_DIR, PATH, RUSTC, RUSTDOC, and RUSTUP_TOOLCHAIN for the requested command. The PowerShell wrapper snapshots and restores the caller's process environment even when a command fails. Both wrappers resolve the Rust commands to the repository-local proxies, discard ambient compiler/wrapper overrides, pin rustup's official distribution endpoints, and disable auto-install so ordinary commands cannot silently download another compiler.

Do not invoke a globally installed Cargo directly if strict project isolation matters. Use the run wrappers.

## Verify

Windows:

    powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify.ps1

macOS or Linux:

    sh scripts/verify.sh

Verification runs formatting, check, Clippy with warnings denied, tests, and documentation for the entire locked workspace.

The included GitHub Actions workflow runs the same bootstrap on all six advertised host tuples: Apple Silicon and Intel macOS, arm64 and x86_64 GNU/Linux, and arm64 and x86_64 Windows/MSVC.

## Update the compiler

1. Review the target Rust release and release notes.
2. Change the exact channel in rust-toolchain.toml.
3. Change workspace.package.rust-version in Cargo.toml only when the minimum supported Rust version changes.
4. Rerun setup on every supported platform.
5. Commit Cargo.lock and all resulting source changes, but never commit .tooling or target.

The rustup bootstrap version and platform checksums are constants in both setup scripts. Updating rustup requires reviewing its release, replacing those constants from the official archive checksum files, and testing every supported host.

## Security and future tooling

- Rustup downloads use HTTPS and hash verification, but rustup does not provide artifact signature verification. Committed hashes prevent an unreviewed moving bootstrap from being accepted.
- Keep Google OAuth refresh tokens in macOS Keychain, Windows Credential Manager, or the appropriate Linux secret service. Never place them in .tooling, environment files, or the repository.
- A Dev Container can later provide a stronger Linux service-test boundary, but it cannot replace native macOS/Windows desktop builds.
- Add uv only when Python-based connectors or model workers exist.
- Add chezmoi only for personal-machine onboarding outside the repository.
- Add mise only if one cross-runtime manifest becomes more valuable than keeping Rust’s single source of truth in rust-toolchain.toml.

## Primary documentation

- [Rustup installation and custom homes](https://rust-lang.github.io/rustup/installation/)
- [Rustup manual installers](https://rust-lang.github.io/rustup/installation/other.html)
- [Rustup security](https://rust-lang.github.io/rustup/security.html)
- [Rustup toolchain files](https://rust-lang.github.io/rustup/overrides.html)
- [Cargo workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html)
- [Cargo home](https://doc.rust-lang.org/cargo/guide/cargo-home.html)
- [Rust 1.97.1 release](https://blog.rust-lang.org/2026/07/16/Rust-1.97.1/)
