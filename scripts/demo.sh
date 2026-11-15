#!/usr/bin/env sh
# Builds the demo and runs `fabric-demo up`, `down` or `reset-keys` from the repository root.
# The macOS counterpart of scripts/demo.ps1.
#
#   sh scripts/demo.sh [up|down|reset-keys] [--alice <backend>] [--bob <backend>]
#
# A backend is claude, ollama:<model> or scripted (tests only). Both default to claude.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
RUN="$ROOT/scripts/run.sh"
EXE="$ROOT/target/release/fabric-demo"

command=up
alice=claude
bob=claude
while [ "$#" -gt 0 ]; do
    case "$1" in
        up|down|reset-keys) command=$1 ;;
        --alice) shift; alice=$1 ;;
        --bob) shift; bob=$1 ;;
        *)
            echo "usage: sh scripts/demo.sh [up|down|reset-keys] [--alice <backend>] [--bob <backend>]" >&2
            exit 64
            ;;
    esac
    shift
done

# `down` and `reset-keys` reuse the binary `up` built, so they never rebuild under a running demo.
if [ "$command" = up ] || [ ! -x "$EXE" ]; then
    sh "$RUN" cargo build -p fabric-demo --release
fi

case "$command" in
    up) exec "$EXE" up --alice "$alice" --bob "$bob" --repository "$ROOT" ;;
    down) exec "$EXE" down --repository "$ROOT" ;;
    reset-keys) exec "$EXE" reset-keys --repository "$ROOT" ;;
esac
