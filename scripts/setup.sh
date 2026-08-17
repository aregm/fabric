#!/usr/bin/env sh
set -eu

die() {
    echo "error: $*" >&2
    exit 1
}

hash_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    elif command -v openssl >/dev/null 2>&1; then
        openssl dgst -sha256 "$1" | awk '{print $NF}'
    else
        die "SHA-256 tool not found; install sha256sum, shasum, or openssl."
    fi
}

skip_verify=0
if [ "$#" -gt 0 ]; then
    if [ "$1" = "--skip-verify" ] && [ "$#" -eq 1 ]; then
        skip_verify=1
    else
        die "usage: sh scripts/setup.sh [--skip-verify]"
    fi
fi

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
TOOLING="$ROOT/.tooling"
export CARGO_HOME="$TOOLING/cargo"
export RUSTUP_HOME="$TOOLING/rustup"
export CARGO_TARGET_DIR="$ROOT/target"
export PATH="$CARGO_HOME/bin:$PATH"
export RUSTUP_INIT_SKIP_PATH_CHECK=yes
export RUSTUP_DIST_SERVER=https://static.rust-lang.org
export RUSTUP_UPDATE_ROOT=https://static.rust-lang.org/rustup
unset RUSTUP_TOOLCHAIN || true
unset RUSTC RUSTDOC RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER || true
unset CARGO_BUILD_RUSTC CARGO_BUILD_RUSTC_WRAPPER || true
unset CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER CARGO_BUILD_RUSTDOC || true

mkdir -p "$CARGO_HOME" "$RUSTUP_HOME" "$TOOLING/bootstrap" "$CARGO_TARGET_DIR"

os=$(uname -s)
arch=$(uname -m)
case "$os/$arch" in
    Darwin/arm64)
        host_target=aarch64-apple-darwin
        expected_hash=aeb4105778ca1bd3c6b0e75768f581c656633cd51368fa61289b6a71696ac7e1
        ;;
    Darwin/x86_64)
        host_target=x86_64-apple-darwin
        expected_hash=33cf85df9142bc6d29cbc62fa5ca1d4c29622cddb55213a4c1a43c457fb9b2d7
        ;;
    Linux/aarch64|Linux/arm64)
        host_target=aarch64-unknown-linux-gnu
        expected_hash=9732d6c5e2a098d3521fca8145d826ae0aaa067ef2385ead08e6feac88fa5792
        ;;
    Linux/x86_64|Linux/amd64)
        host_target=x86_64-unknown-linux-gnu
        expected_hash=4acc9acc76d5079515b46346a485974457b5a79893cfb01112423c89aeb5aa10
        ;;
    *)
        die "unsupported host $os/$arch; supported: macOS and GNU/Linux on x86_64 or arm64"
        ;;
esac

if [ "$os" = "Darwin" ]; then
    command -v xcrun >/dev/null 2>&1 || die "Xcode Command Line Tools are required. Run: xcode-select --install"
    xcrun --find clang >/dev/null 2>&1 || die "Apple clang was not found. Run: xcode-select --install"
elif ! command -v cc >/dev/null 2>&1 && ! command -v clang >/dev/null 2>&1 && ! command -v gcc >/dev/null 2>&1; then
    die "A native C compiler/linker is required. Install your distribution's build-essential or equivalent package."
fi

command -v curl >/dev/null 2>&1 || die "curl is required to download the pinned official rustup bootstrap."

rustup_version=1.29.0
rustup="$CARGO_HOME/bin/rustup"
if [ ! -x "$rustup" ]; then
    rustup_init="$TOOLING/bootstrap/rustup-init-$rustup_version-$host_target"
    if [ ! -f "$rustup_init" ]; then
        url="https://static.rust-lang.org/rustup/archive/$rustup_version/$host_target/rustup-init"
        echo "Downloading pinned rustup $rustup_version for $host_target..."
        curl --proto '=https' --tlsv1.2 --fail --location --silent --show-error "$url" --output "$rustup_init.download"
        mv "$rustup_init.download" "$rustup_init"
    fi

    actual_hash=$(hash_file "$rustup_init")
    [ "$actual_hash" = "$expected_hash" ] || die "rustup-init checksum mismatch: expected $expected_hash, received $actual_hash. Delete $rustup_init only after investigating."
    chmod 700 "$rustup_init"
    "$rustup_init" -y --no-modify-path --default-toolchain none --profile minimal
fi

export RUSTUP_AUTO_INSTALL=0
rustup_version_output=$("$rustup" --version 2>&1 | awk '/^rustup / {print; exit}')
case "$rustup_version_output" in
    *"rustup $rustup_version"*) ;;
    *) die "expected project-local rustup $rustup_version, received: $rustup_version_output" ;;
esac

channel=$(awk -F '"' '/^[[:space:]]*channel[[:space:]]*=/ {print $2; exit}' "$ROOT/rust-toolchain.toml")
[ -n "$channel" ] || die "could not read channel from rust-toolchain.toml"

"$rustup" toolchain install "$channel" \
    --profile minimal \
    --component clippy \
    --component rustfmt \
    --component rust-analyzer \
    --component rust-src

export RUSTUP_TOOLCHAIN="$channel"

cargo="$CARGO_HOME/bin/cargo"
rustc="$CARGO_HOME/bin/rustc"
export RUSTC="$rustc"
export RUSTDOC="$CARGO_HOME/bin/rustdoc"
reported_home=$("$rustup" show home | tail -n 1)
[ "$reported_home" = "$RUSTUP_HOME" ] || die "isolation check failed: rustup home is $reported_home, expected $RUSTUP_HOME"
sysroot=$("$rustc" --print sysroot)
case "$sysroot" in
    "$RUSTUP_HOME"/*) ;;
    *) die "isolation check failed: rustc sysroot $sysroot is outside $RUSTUP_HOME" ;;
esac

cd "$ROOT"
if [ ! -f Cargo.lock ]; then
    "$cargo" generate-lockfile
fi
"$cargo" fetch --locked

if [ "$skip_verify" -eq 0 ]; then
    "$cargo" fmt --all -- --check
    "$cargo" check --workspace --all-targets --all-features --locked
    "$cargo" clippy --workspace --all-targets --all-features --locked -- -D warnings
    "$cargo" test --workspace --all-targets --all-features --locked
    "$cargo" doc --workspace --no-deps --locked
fi

echo
echo "Fabric Rust environment is ready."
echo "  rustup: $rustup_version_output"
echo "  channel: $channel"
echo "  RUSTUP_HOME: $RUSTUP_HOME"
echo "  CARGO_HOME: $CARGO_HOME"
echo
echo "Run commands without changing your global shell:"
echo "  sh scripts/run.sh cargo test --workspace --locked"
