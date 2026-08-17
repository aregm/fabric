# Fabric

Initial Rust workspace for the Personal Knowledge Fabric.

The scaffold begins as a small modular monolith:

- apps/fabric-cli — diagnostic command-line application;
- crates/fabric-core — shared domain library;
- scripts — isolated setup, command runner, and verification for Windows and macOS/Linux.

Calendar, Google adapter, storage, protocol, and desktop crates should be added only when they contain real behavior.

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
