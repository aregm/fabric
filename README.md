# Fabric

Initial Rust workspace for the Personal Knowledge Fabric. The first executable
vertical is a deterministic, in-memory two-agent meeting-agreement simulator.

The scaffold begins as a small modular monolith:

- apps/fabric-cli — diagnostic CLI plus the `demo-meeting` executable fixture;
- crates/fabric-core — shared domain library, including checked calendar intervals and Rendezvous agreement semantics;
- scripts — isolated setup, command runner, and verification for Windows and macOS/Linux.

Google adapters, encrypted transport, persistence, and desktop crates should be added only when they contain real behavior.

## Product documentation

- [Product requirements](docs/PRD.md) — problem statement, market research, product experience, requirements, roadmap, and evaluation.
- [Technical architecture](docs/architecture.md) — platform architecture, data model, retrieval, agents, security, and implementation guidance.
- [Calendar vertical](docs/calendar.md) — the Google Calendar-first product test, private scheduling protocol, consent model, and release gates.

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
