#!/usr/bin/env sh
# The cross-language contract suite for fabric-desktop/1 (gate CD-G1a).
#
# Runs the Rust side and the TypeScript side against the same fixtures in
# contracts/desktop/v1/fixtures and fails on any disagreement:
#
#   1. the Rust fixture tests;
#   2. the TypeScript fixture tests;
#   3. both sides print a report of what they computed for every fixture, and the
#      two reports must be equal byte for byte;
#   4. the Rust side derives mutants from the accepted messages; both sides judge
#      every mutant, and the two lists of verdicts must be equal byte for byte;
#   5. the Python reference in contracts/desktop/v1/reference, written apart from
#      both sides, prints its own report and verdicts, and they must equal the
#      Rust ones byte for byte (skipped when python3 is not installed);
#   6. with --typecheck, the TypeScript types are checked against the accepted
#      fixtures by the TypeScript compiler (this installs two npm packages).
#
# Needs Node 22.18 or later, which runs TypeScript files directly.
set -eu

die() {
    echo "error: $*" >&2
    exit 1
}

typecheck=0
for argument in "$@"; do
    case "$argument" in
        --typecheck) typecheck=1 ;;
        *) die "usage: sh scripts/verify-contracts.sh [--typecheck]" ;;
    esac
done

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
RUN="$ROOT/scripts/run.sh"
CONTRACT="$ROOT/contracts/desktop/v1"
TYPESCRIPT="$CONTRACT/typescript"
REFERENCE="$CONTRACT/reference/reference.py"

command -v node >/dev/null 2>&1 || die "node was not found; install Node 22.18 or later."
node -e '
const [major, minor] = process.versions.node.split(".").map(Number);
if (major < 22 || (major === 22 && minor < 18)) {
    console.error("error: Node " + process.versions.node + " cannot run TypeScript files directly; install Node 22.18 or later.");
    process.exit(1);
}'

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT INT TERM

rust_report() {
    sh "$RUN" cargo run --quiet --locked -p fabric-desktop-contracts --example contract_report -- "$@"
}

typescript_report() {
    (cd "$TYPESCRIPT" && node src/report.ts "$@")
}

same() {
    if ! cmp -s "$1" "$2"; then
        echo "error: $4 disagree on $3:" >&2
        diff "$1" "$2" | head -n 40 >&2 || true
        exit 1
    fi
}

echo "[1/6] Rust fixture tests"
sh "$RUN" cargo test --quiet --locked -p fabric-desktop-contracts

echo "[2/6] TypeScript fixture tests"
(cd "$TYPESCRIPT" && node --test)

echo "[3/6] Reports of both sides over the fixtures"
rust_report report "$CONTRACT" >"$WORK/rust-report.txt"
typescript_report report >"$WORK/typescript-report.txt"
same "$WORK/rust-report.txt" "$WORK/typescript-report.txt" "the fixtures" "Rust and TypeScript"
checks=$(wc -l <"$WORK/rust-report.txt" | tr -d ' ')
[ "$checks" -gt 0 ] || die "the report is empty."

echo "[4/6] Verdicts of both sides over the mutants"
rust_report mutants "$CONTRACT" "$WORK/mutants.jsonl"
rust_report judge "$CONTRACT" "$WORK/mutants.jsonl" >"$WORK/rust-mutants.txt"
typescript_report judge "$WORK/mutants.jsonl" >"$WORK/typescript-mutants.txt"
same "$WORK/rust-mutants.txt" "$WORK/typescript-mutants.txt" "the mutants" "Rust and TypeScript"
mutants=$(wc -l <"$WORK/rust-mutants.txt" | tr -d ' ')
[ "$mutants" -gt 0 ] || die "no mutant was judged."
accepted=$(grep -c "	accept$" "$WORK/rust-mutants.txt" || true)

if command -v python3 >/dev/null 2>&1; then
    echo "[5/6] The Python reference over the fixtures and the mutants"
    python3 "$REFERENCE" report "$CONTRACT" >"$WORK/reference-report.txt"
    same "$WORK/rust-report.txt" "$WORK/reference-report.txt" "the fixtures" "Rust and the Python reference"
    python3 "$REFERENCE" judge "$WORK/mutants.jsonl" "$CONTRACT" >"$WORK/reference-mutants.txt"
    same "$WORK/rust-mutants.txt" "$WORK/reference-mutants.txt" "the mutants" "Rust and the Python reference"
    sides="all three sides"
else
    echo "[5/6] Skipped: python3 was not found, so the Python reference did not run."
    sides="both sides"
fi

if [ "$typecheck" -eq 1 ]; then
    echo "[6/6] TypeScript types against the accepted fixtures"
    command -v npm >/dev/null 2>&1 || die "npm was not found."
    (cd "$TYPESCRIPT" && npm ci --no-audit --no-fund && npm run --silent typecheck)
else
    echo "[6/6] Skipped: run with --typecheck to check the TypeScript types with the compiler."
fi

echo
echo "The contract suite passed."
echo "  fixture checks, equal on $sides: $checks"
echo "  mutants judged, equal on $sides: $mutants ($accepted accepted)"
