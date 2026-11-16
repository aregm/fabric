#!/usr/bin/env sh
# Checks the desktop application (work packages CD-W02 and CD-W04).
#
#   sh scripts/verify-desktop.sh                the sources
#   sh scripts/verify-desktop.sh --packaged     the bundle that scripts/build-desktop.sh made
#   sh scripts/verify-desktop.sh --live-proofs  the broker with the real Isabelle (this Mac only)
#
# scripts/verify.sh already checks the broker (apps/fabric-broker) and the host's relay
# (apps/fabric-desktop/relay) on every platform, because they are members of the Cargo workspace.
# This script checks what that one cannot: the webview's TypeScript, and the Tauri host, which is
# its own Cargo workspace, needs Node and builds only on macOS.
#
# Without a flag it installs the pinned npm development packages, typechecks the TypeScript, runs
# its tests, builds the webview files, and on macOS builds the debug broker, compiles the release
# broker (it is not run) and runs its configuration tests in release, and formats, lints and tests
# the Tauri host. The host's tests start the
# debug broker with the test key in temporary directories.
#
# With --packaged it copies the bundle to a directory whose path contains spaces and, from there:
#   1. runs the broker's process tests and the relay round trip against the broker inside the
#      bundle; each test starts it with the test key in a temporary directory, and one of them
#      kills it and checks that its store survives;
#   2. starts the application with a scratch data directory, the test key and the synthetic agent,
#      and reads the host's trace: the webview's hello, snapshot and subscribe are answered, a
#      request notice arrives, and the notice sequences rise by one (echo and progress);
#   3. starts the application a second time on the same directory: the second one reports a
#      conflict and starts no broker;
#   4. kills the broker: the host reports the disconnect and starts no new broker by itself;
#   5. kills the host: the broker does not outlive it.
# No step opens the real store or the Keychain: every launch here names the scratch directory and
# the test key. It opens the application's window on this Mac for a few seconds. It cannot press a
# key in the webview, so the approval in the webview, the restart button, Quit and VoiceOver are
# checked by a person (apps/fabric-desktop/README.md has the steps). The application must not be
# running. Its launches unset FABRIC_BROKER_ISABELLE_HOME, FABRIC_BROKER_ISABELLE_USER_HOME and
# FABRIC_BROKER_RULES_DIR, so values exported in the terminal never turn them into practice runs.
#
# With --live-proofs it runs the tests that need the real Isabelle, which CI never runs: the
# broker's live tests (apps/fabric-broker/tests/live.rs) and the rule theory's corpus
# (crates/fabric-broker-policy/tests/corpus.rs). Isabelle must be installed beforehand by hand
# with sh scripts/setup-demo.sh; this script never installs it. The live tests take Isabelle's
# place from FABRIC_BROKER_ISABELLE_HOME, FABRIC_BROKER_ISABELLE_USER_HOME and
# FABRIC_BROKER_RULES_DIR, all three or none; with none, the repository's development layout
# (.tooling/isabelle and isabelle). The test that starts Isabelle from the broker's empty
# environment runs first, by name. Every store uses the test key in a temporary directory; nothing
# reaches the Keychain. The first run can build Isabelle's session heap for several minutes.
set -eu

die() {
    echo "error: $*" >&2
    exit 1
}

USAGE="usage: sh scripts/verify-desktop.sh [--packaged | --live-proofs]"
mode=sources
for argument in "$@"; do
    [ "$mode" = sources ] || die "$USAGE"
    case "$argument" in
        --packaged) mode=packaged ;;
        --live-proofs) mode=live ;;
        *) die "$USAGE" ;;
    esac
done

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
RUN="$ROOT/scripts/run.sh"

# The broker's store is SQLCipher with Apple's CommonCrypto; an OpenSSL left in the environment
# must not reach its build.
unset OPENSSL_DIR OPENSSL_LIB_DIR OPENSSL_INCLUDE_DIR
DESKTOP="$ROOT/apps/fabric-desktop"
HOST_MANIFEST="$DESKTOP/src-tauri/Cargo.toml"
TARGET_TRIPLE=aarch64-apple-darwin

# The live proofs need no Node.
if [ "$mode" != live ]; then
    command -v node >/dev/null 2>&1 || die "node was not found; install Node 22.18 or later."
    node -e '
const [major, minor] = process.versions.node.split(".").map(Number);
if (major < 22 || (major === 22 && minor < 18)) {
    console.error("error: Node " + process.versions.node + " cannot run TypeScript files directly; install Node 22.18 or later.");
    process.exit(1);
}'
fi

sources() {
    command -v npm >/dev/null 2>&1 || die "npm was not found; it comes with Node."

    echo "[1/5] The npm development packages"
    (cd "$DESKTOP" && npm ci --no-audit --no-fund)

    echo "[2/5] The TypeScript typecheck"
    (cd "$DESKTOP" && npm run --silent typecheck)

    echo "[3/5] The TypeScript tests"
    (cd "$DESKTOP" && npm test --silent)

    echo "[4/5] The webview files"
    (cd "$DESKTOP" && npm run --silent build)

    echo "[5/5] The Tauri host"
    if [ "$(uname -s)" != "Darwin" ]; then
        echo "skipped: the Tauri host builds only on macOS."
        return
    fi
    # The host's build step looks for the broker it will bundle, so the broker is built first.
    sh "$RUN" cargo build --locked -p fabric-broker
    mkdir -p "$DESKTOP/src-tauri/binaries"
    cp "$ROOT/target/debug/fabric-broker" "$DESKTOP/src-tauri/binaries/fabric-broker-$TARGET_TRIPLE"
    # The release broker, which drops the test key and the synthetic agent, must compile too, and
    # its configuration must refuse them. The library tests parse variables, build plans and run
    # the prover worker's unit tests against test doubles: they open no store, start no Isabelle
    # and never reach the Keychain.
    sh "$RUN" cargo check --release --locked -p fabric-broker
    sh "$RUN" cargo test --release --locked -p fabric-broker --lib
    sh "$RUN" cargo fmt --manifest-path "$HOST_MANIFEST" -- --check
    sh "$RUN" cargo clippy --manifest-path "$HOST_MANIFEST" --all-targets --locked -- -D warnings
    sh "$RUN" cargo test --manifest-path "$HOST_MANIFEST" --all-targets --locked
}

# Waits until a line matching the pattern is in the file, for at most the given number of seconds.
wait_for() {
    file=$1
    pattern=$2
    seconds=$3
    tries=$((seconds * 5))
    while [ "$tries" -gt 0 ]; do
        if grep -Eq -- "$pattern" "$file" 2>/dev/null; then
            return 0
        fi
        sleep 0.2
        tries=$((tries - 1))
    done
    echo "error: no line matching '$pattern' within $seconds s in the trace:" >&2
    sed 's/^/    /' "$file" >&2 || true
    return 1
}

# The process ids of every running copy of the given executable. `comm` is the executable's
# path without its arguments, so the awk process, whose arguments hold the path, is not counted.
running() {
    ps -axo pid=,comm= | awk -v path="$1" 'index($0, path) { print $1 }'
}

packaged() {
    [ "$(uname -s)" = "Darwin" ] || die "the bundle exists only on macOS."
    APP="$ROOT/target/debug/bundle/macos/Fabric Calendar.app"
    [ -d "$APP" ] || die "there is no bundle; run sh scripts/build-desktop.sh --configuration debug first."

    WORK=$(mktemp -d)
    host_pid=
    second_pid=
    cleanup() {
        for pid in $host_pid $second_pid; do
            kill -KILL "$pid" 2>/dev/null || true
        done
        rm -rf "$WORK"
    }
    trap cleanup EXIT INT TERM

    SPACED="$WORK/a path with spaces"
    mkdir -p "$SPACED"
    cp -R "$APP" "$SPACED/"
    COPY="$SPACED/Fabric Calendar.app"
    HOST="$COPY/Contents/MacOS/fabric-desktop"
    BROKER="$COPY/Contents/MacOS/fabric-broker"
    codesign --verify --deep --strict "$COPY" || die "the copied bundle's signature does not verify."
    echo "The bundle under test: $COPY"

    echo "[1/5] The packaged broker: its process tests and the relay round trip"
    FABRIC_BROKER_UNDER_TEST="$BROKER" sh "$RUN" cargo test --locked -p fabric-broker --tests

    # Every launch below uses this scratch directory, the test key and the synthetic agent, so
    # none opens the real store; the same directory each time, so the second launch meets the
    # first one's lock.
    # The three prover variables are unset, so a developer's exported values never make this a
    # practice run. The subshell ends in exec, so $! is the host's own process id.
    DATA="$WORK/data"
    launch() {
        (
            unset FABRIC_BROKER_ISABELLE_HOME FABRIC_BROKER_ISABELLE_USER_HOME FABRIC_BROKER_RULES_DIR
            FABRIC_DESKTOP_DATA_DIR="$DATA" FABRIC_BROKER_KEY_SOURCE=test FABRIC_BROKER_SYNTHETIC_AGENT=1 \
                FABRIC_DESKTOP_TRACE=1 exec "$HOST"
        ) 2>"$1" &
    }

    echo "[2/5] The application: echo and progress through the webview"
    launch "$WORK/first.trace"
    host_pid=$!
    wait_for "$WORK/first.trace" ' state run=[0-9]+ (connected|conflict|configuration_error)' 30
    if grep -q ' conflict' "$WORK/first.trace"; then
        die "Fabric Calendar is already running on this Mac; quit it and run this again."
    fi
    wait_for "$WORK/first.trace" ' state run=1 connected pid=[0-9]+ ' 1
    wait_for "$WORK/first.trace" ' <- response hello id=[^ ]+ result ' 30
    wait_for "$WORK/first.trace" ' <- response get_snapshot id=[^ ]+ result ' 10
    wait_for "$WORK/first.trace" ' <- response subscribe id=[^ ]+ result ' 10
    wait_for "$WORK/first.trace" ' <- notice sequence=[0-9]+ change=request ' 20
    sequences=$(sed -n 's/.* <- notice sequence=\([0-9]*\) .*/\1/p' "$WORK/first.trace" | tr '\n' ' ')
    previous=
    for sequence in $sequences; do
        if [ -n "$previous" ] && [ "$sequence" -ne $((previous + 1)) ]; then
            die "the notice sequences do not rise by one: $sequences"
        fi
        previous=$sequence
    done
    if grep -q ' refused ' "$WORK/first.trace"; then
        die "the host refused a frame that the webview built."
    fi
    broker_pid=$(sed -n 's/.* state run=1 connected pid=\([0-9]*\) .*/\1/p' "$WORK/first.trace" | head -n 1)
    [ "$(running "$BROKER" | tr '\n' ' ')" = "$broker_pid " ] || die "expected exactly one broker, with pid $broker_pid."
    echo "ok: hello, snapshot and subscribe were answered and notices $sequences arrived in order; the broker is pid $broker_pid"

    echo "[3/5] A second launch"
    launch "$WORK/second.trace"
    second_pid=$!
    wait_for "$WORK/second.trace" ' state run=0 conflict' 30
    sleep 1
    [ "$(running "$BROKER" | tr '\n' ' ')" = "$broker_pid " ] || die "the second launch started a broker."
    kill -KILL "$second_pid" 2>/dev/null || true
    wait "$second_pid" 2>/dev/null || true
    second_pid=
    echo "ok: the second launch reported a conflict and started no broker"

    echo "[4/5] The broker dies"
    kill -KILL "$broker_pid"
    wait_for "$WORK/first.trace" ' state run=1 disconnected reason=exited ' 10
    sleep 3
    [ -z "$(running "$BROKER")" ] || die "a broker is running again; the restart must be the owner's action."
    echo "ok: the host reported the disconnect and started no broker by itself"
    kill -KILL "$host_pid" 2>/dev/null || true
    wait "$host_pid" 2>/dev/null || true
    host_pid=

    echo "[5/5] The host dies"
    launch "$WORK/third.trace"
    host_pid=$!
    wait_for "$WORK/third.trace" ' state run=1 connected pid=[0-9]+ ' 30
    broker_pid=$(sed -n 's/.* state run=1 connected pid=\([0-9]*\) .*/\1/p' "$WORK/third.trace" | head -n 1)
    kill -KILL "$host_pid"
    wait "$host_pid" 2>/dev/null || true
    host_pid=
    tries=25
    while [ -n "$(running "$BROKER")" ] && [ "$tries" -gt 0 ]; do
        sleep 0.2
        tries=$((tries - 1))
    done
    [ -z "$(running "$BROKER")" ] || die "broker $broker_pid outlived the host."
    echo "ok: the broker ended when the host was killed"

    echo "The trace of the first launch:"
    sed 's/^/    /' "$WORK/first.trace"
    echo "The packaged application passed. Still to be checked by a person: the approval in the"
    echo "webview, the restart button, Quit and VoiceOver (apps/fabric-desktop/README.md)."
}

# The live test that must run first and alone: whether the broker, started by the relay with the
# relay's environment (its variables and a fixed PATH), can start Isabelle at all.
FIRST_LIVE_TEST=the_built_broker_starts_isabelle_with_an_empty_environment

live() {
    [ "$(uname -s)" = "Darwin" ] || die "the live proofs run only on macOS."
    set_count=0
    for name in FABRIC_BROKER_ISABELLE_HOME FABRIC_BROKER_ISABELLE_USER_HOME FABRIC_BROKER_RULES_DIR; do
        eval "value=\${$name+set}"
        [ -z "$value" ] || set_count=$((set_count + 1))
    done
    case "$set_count" in
        0)
            # The repository's development layout, the one scripts/setup-demo.sh installs.
            FABRIC_BROKER_ISABELLE_HOME="$ROOT/.tooling/isabelle/Isabelle2025-2"
            FABRIC_BROKER_ISABELLE_USER_HOME="$ROOT/.tooling/isabelle/home"
            FABRIC_BROKER_RULES_DIR="$ROOT/isabelle"
            export FABRIC_BROKER_ISABELLE_HOME FABRIC_BROKER_ISABELLE_USER_HOME FABRIC_BROKER_RULES_DIR
            ;;
        3)
            for value in "$FABRIC_BROKER_ISABELLE_HOME" "$FABRIC_BROKER_ISABELLE_USER_HOME" "$FABRIC_BROKER_RULES_DIR"; do
                case "$value" in
                    /*) ;;
                    *) die "the three FABRIC_BROKER_ISABELLE_HOME, FABRIC_BROKER_ISABELLE_USER_HOME and FABRIC_BROKER_RULES_DIR must be absolute paths; this one is not: $value" ;;
                esac
            done
            ;;
        *) die "the rule prover needs FABRIC_BROKER_ISABELLE_HOME, FABRIC_BROKER_ISABELLE_USER_HOME and FABRIC_BROKER_RULES_DIR together, or none" ;;
    esac
    # The corpus always uses the repository's layout, so it must be installed in every case.
    [ -x "$ROOT/.tooling/isabelle/Isabelle2025-2/bin/isabelle" ] \
        || die "Isabelle is not installed in $ROOT/.tooling/isabelle; run sh scripts/setup-demo.sh by hand (this script never installs it)."
    [ -x "$FABRIC_BROKER_ISABELLE_HOME/bin/isabelle" ] \
        || die "there is no Isabelle at FABRIC_BROKER_ISABELLE_HOME=$FABRIC_BROKER_ISABELLE_HOME; run sh scripts/setup-demo.sh by hand, or fix the variable."
    [ -d "$FABRIC_BROKER_ISABELLE_USER_HOME" ] \
        || die "FABRIC_BROKER_ISABELLE_USER_HOME=$FABRIC_BROKER_ISABELLE_USER_HOME is not a directory; run sh scripts/setup-demo.sh by hand, or fix the variable."
    [ -f "$FABRIC_BROKER_RULES_DIR/Fabric_Broker/Fabric_Broker.thy" ] \
        || die "FABRIC_BROKER_RULES_DIR=$FABRIC_BROKER_RULES_DIR holds no Fabric_Broker/Fabric_Broker.thy."

    WORK=$(mktemp -d)
    trap 'rm -rf "$WORK"' EXIT INT TERM

    echo "[1/4] The Isabelle version"
    version=$(sh "$ROOT/scripts/isabelle.sh" version) \
        || die "Isabelle did not run; run sh scripts/setup-demo.sh by hand."
    [ "$version" = Isabelle2025-2 ] || die "expected Isabelle2025-2, found: $version"
    echo "ok: $version"

    # Output to a file and no pipe: `set -e` does not see a failure inside a pipe. The count must be
    # exactly one, so a name that matches no test cannot pass.
    echo "[2/4] The broker starts Isabelle with the relay's environment (its variables and a fixed PATH) ($FIRST_LIVE_TEST)"
    status=0
    sh "$RUN" cargo test --locked -p fabric-broker --test live -- \
        --ignored --exact "$FIRST_LIVE_TEST" >"$WORK/first.log" 2>&1 || status=$?
    cat "$WORK/first.log"
    [ "$status" -eq 0 ] || die "the live test $FIRST_LIVE_TEST failed; the other live tests were not run."
    grep -q 'test result: ok. 1 passed' "$WORK/first.log" \
        || die "the live test $FIRST_LIVE_TEST did not run exactly once."

    echo "[3/4] The rule theory's corpus"
    sh "$RUN" cargo test --locked -p fabric-broker-policy --test corpus -- --ignored --test-threads=1

    echo "[4/4] The broker's live tests"
    sh "$RUN" cargo test --locked -p fabric-broker --test live -- --ignored --test-threads=1 --nocapture
}

case "$mode" in
    sources) sources ;;
    packaged) packaged ;;
    live) live ;;
esac

echo "All desktop checks of this mode passed."
