#!/usr/bin/env sh
# Runs an Isabelle tool from the repository-local installation, with Isabelle's per-user
# directory (settings and session heaps) kept in .tooling/isabelle/home. The macOS counterpart
# of scripts/isabelle.ps1.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
ISABELLE_TOOLING="$ROOT/.tooling/isabelle"
ISABELLE_HOME_DIR="$ISABELLE_TOOLING/Isabelle2025-2"
USER_HOME_DIR="$ISABELLE_TOOLING/home"

if [ ! -x "$ISABELLE_HOME_DIR/bin/isabelle" ]; then
    echo "error: Isabelle is not installed. Run: sh scripts/setup-demo.sh" >&2
    exit 1
fi
[ "$#" -gt 0 ] || {
    echo "usage: sh scripts/isabelle.sh <tool> [arguments], for example: sh scripts/isabelle.sh version" >&2
    exit 64
}

mkdir -p "$USER_HOME_DIR"
cd "$ROOT"
USER_HOME="$USER_HOME_DIR" exec "$ISABELLE_HOME_DIR/bin/isabelle" "$@"
