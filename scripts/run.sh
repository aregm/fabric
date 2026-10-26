#!/usr/bin/env sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
TOOLING="$ROOT/.tooling"
export CARGO_HOME="$TOOLING/cargo"
export RUSTUP_HOME="$TOOLING/rustup"
export CARGO_TARGET_DIR="$ROOT/target"
export PATH="$CARGO_HOME/bin:$PATH"
export RUSTC="$CARGO_HOME/bin/rustc"
export RUSTDOC="$CARGO_HOME/bin/rustdoc"
export RUSTUP_AUTO_INSTALL=0
export RUSTUP_DIST_SERVER=https://static.rust-lang.org
export RUSTUP_UPDATE_ROOT=https://static.rust-lang.org/rustup
unset RUSTUP_TOOLCHAIN || true
unset RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER || true
unset CARGO_BUILD_RUSTC CARGO_BUILD_RUSTC_WRAPPER || true
unset CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER CARGO_BUILD_RUSTDOC || true

channel=$(awk -F '"' '/^[[:space:]]*channel[[:space:]]*=/ {print $2; exit}' "$ROOT/rust-toolchain.toml")
[ -n "$channel" ] || {
    echo "error: could not read channel from rust-toolchain.toml" >&2
    exit 1
}
export RUSTUP_TOOLCHAIN="$channel"

if [ ! -x "$CARGO_HOME/bin/cargo" ]; then
    sh "$ROOT/scripts/setup.sh" --skip-verify
fi

[ "$#" -gt 0 ] || {
    echo "usage: sh scripts/run.sh <command> [arguments]" >&2
    echo "example: sh scripts/run.sh cargo test --workspace --locked" >&2
    exit 64
}

executable=$1
shift
case "$executable" in
    cargo|rustc|rustdoc|rustup)
        executable="$CARGO_HOME/bin/$executable"
        ;;
esac

cd "$ROOT"
exec "$executable" "$@"
