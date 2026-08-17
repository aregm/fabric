#!/usr/bin/env sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
RUN="$ROOT/scripts/run.sh"

sh "$RUN" cargo fmt --all -- --check
sh "$RUN" cargo check --workspace --all-targets --all-features --locked
sh "$RUN" cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
sh "$RUN" cargo test --workspace --all-targets --all-features --locked
sh "$RUN" cargo doc --workspace --no-deps --locked

echo "All Fabric verification checks passed."

