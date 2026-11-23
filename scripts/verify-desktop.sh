#!/usr/bin/env sh
# Checks the desktop application (work packages CD-W02 and CD-W04).
#
#   sh scripts/verify-desktop.sh                the sources
#   sh scripts/verify-desktop.sh --packaged     the bundle that scripts/build-desktop.sh made
#   sh scripts/verify-desktop.sh --live-proofs  the broker with the real Isabelle (this Mac only)
#   sh scripts/verify-desktop.sh --installed-proofs
#                                               the bundle with an Isabelle that install-isabelle.sh
#                                               installed (this Mac only, by hand)
#
# scripts/verify.sh already checks the broker (apps/fabric-broker) and the host's relay
# (apps/fabric-desktop/relay) on every platform, because they are members of the Cargo workspace.
# This script checks what that one cannot: the webview's TypeScript, and the Tauri host, which is
# its own Cargo workspace, needs Node and builds only on macOS.
#
# Without a flag it checks that scripts/install-isabelle.sh and scripts/setup-demo.sh pin the same
# Isabelle archive (version, URL, size and SHA-256), installs the pinned npm development packages,
# typechecks the TypeScript, runs
# its tests, builds the webview files, and on macOS builds the debug broker, compiles the release
# broker (it is not run) and runs its configuration tests in release, and formats, lints and tests
# the Tauri host. The host's tests start the
# debug broker with the test key in temporary directories.
#
# With --packaged it copies the bundle to a directory whose path contains spaces, checks that the
# copy's Contents/Resources/isabelle holds the repository's two rule files and nothing else (no
# ROOTS), and, from there:
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
# Without them the host looks for Isabelle in the scratch directory, finds none there, and each run
# stays a synthetic run without the rule checker.
#
# With --live-proofs it runs the tests that need the real Isabelle, which CI never runs: the
# broker's live tests (apps/fabric-broker/tests/live.rs) and the rule theory's corpus
# (crates/fabric-broker-policy/tests/corpus.rs). Isabelle must be installed beforehand by hand
# with sh scripts/setup-demo.sh; this script never installs it. The live tests take Isabelle's
# place from FABRIC_BROKER_ISABELLE_HOME, FABRIC_BROKER_ISABELLE_USER_HOME and
# FABRIC_BROKER_RULES_DIR, all three or none; with none, the repository's development layout
# (.tooling/isabelle and isabelle). The test that starts Isabelle from the relay's environment
# (its variables and a fixed PATH) runs first, by name. Every store uses the test key in a
# temporary directory; nothing reaches the Keychain. The first run can build Isabelle's session
# heap for several minutes.
#
# With --installed-proofs, run by a person on this Mac and never in CI, it checks the installed
# layout end to end. It needs the bundle that scripts/build-desktop.sh made, the pinned archive in
# .tooling/isabelle/download (sh scripts/setup-demo.sh puts it there; this script never downloads)
# and about 4 GB free. In a new scratch directory (mode 0700) it:
#   1. installs Isabelle with sh scripts/install-isabelle.sh --support-dir <scratch> --archive
#      <the archive>;
#   2. runs the installer again and expects "already installed";
#   3. runs it with a wrong small file as --archive into a second scratch folder and expects the
#      refusal, with nothing left behind;
#   4. copies the bundle to a path with spaces, verifies its signature and its two rule files;
#   5. starts the application with FABRIC_DESKTOP_DATA_DIR=<scratch>, the test key and the
#      synthetic agent, the three variables above unset, and waits for the first request notice:
#      the practice agent proposes only while the rule checker is ready, so the notice shows a live
#      Isabelle from the installed layout. The first start builds the session heap and can take
#      several minutes;
#   6. kills the application it started (the broker then stops Isabelle), verifies the bundle's
#      signature again, checks that a Fabric_Broker heap is in <scratch>/isabelle-user-home and
#      that nothing was written below the bundle's Contents/Resources.
# It removes its scratch directory on every exit. It never uses the Keychain and never touches the
# real ~/Library/Application Support/ai.paracels.fabric.calendar. A Fabric Calendar that is
# already running is left alone: this run has its own directory, store and Isabelle server.
set -eu

die() {
    echo "error: $*" >&2
    exit 1
}

USAGE="usage: sh scripts/verify-desktop.sh [--packaged | --live-proofs | --installed-proofs]"
mode=sources
for argument in "$@"; do
    [ "$mode" = sources ] || die "$USAGE"
    case "$argument" in
        --packaged) mode=packaged ;;
        --live-proofs) mode=live ;;
        --installed-proofs) mode=installed ;;
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

# The live and the installed proofs need no Node.
if [ "$mode" = sources ] || [ "$mode" = packaged ]; then
    command -v node >/dev/null 2>&1 || die "node was not found; install Node 22.18 or later."
    node -e '
const [major, minor] = process.versions.node.split(".").map(Number);
if (major < 22 || (major === 22 && minor < 18)) {
    console.error("error: Node " + process.versions.node + " cannot run TypeScript files directly; install Node 22.18 or later.");
    process.exit(1);
}'
fi

# The lines of a script that pin the Isabelle archive.
archive_pin() {
    grep -E '^(ISABELLE_VERSION|ARCHIVE_BYTES|ARCHIVE_SHA256|ARCHIVE_URL)=' "$1" || true
}

sources() {
    command -v npm >/dev/null 2>&1 || die "npm was not found; it comes with Node."

    echo "[1/6] The pinned Isabelle archive"
    installer_pin=$(archive_pin "$ROOT/scripts/install-isabelle.sh")
    demo_pin=$(archive_pin "$ROOT/scripts/setup-demo.sh")
    names=$(printf '%s\n' "$installer_pin" | cut -d= -f1 | LC_ALL=C sort -u | tr '\n' ' ')
    [ "$names" = "ARCHIVE_BYTES ARCHIVE_SHA256 ARCHIVE_URL ISABELLE_VERSION " ] \
        && [ "$(printf '%s\n' "$installer_pin" | wc -l | tr -d ' ')" = 4 ] \
        || die "scripts/install-isabelle.sh must set ISABELLE_VERSION, ARCHIVE_BYTES, ARCHIVE_SHA256 and ARCHIVE_URL once each; it sets: $names"
    [ "$installer_pin" = "$demo_pin" ] \
        || die "scripts/install-isabelle.sh and scripts/setup-demo.sh pin different Isabelle archives (version, URL, size or SHA-256)."
    echo "ok: both scripts pin the same archive"

    echo "[2/6] The npm development packages"
    (cd "$DESKTOP" && npm ci --no-audit --no-fund)

    echo "[3/6] The TypeScript typecheck"
    (cd "$DESKTOP" && npm run --silent typecheck)

    echo "[4/6] The TypeScript tests"
    (cd "$DESKTOP" && npm test --silent)

    echo "[5/6] The webview files"
    (cd "$DESKTOP" && npm run --silent build)

    echo "[6/6] The Tauri host"
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

# Checks that the bundle's Contents/Resources/isabelle holds exactly the repository's two rule
# files of the Fabric_Broker session and no ROOTS (scripts/build-desktop.sh checks the same).
rule_files() {
    rules="$1/Contents/Resources/isabelle"
    for name in ROOT Fabric_Broker.thy; do
        [ -f "$rules/Fabric_Broker/$name" ] || die "the bundle has no rule file Contents/Resources/isabelle/Fabric_Broker/$name."
        cmp -s "$ROOT/isabelle/Fabric_Broker/$name" "$rules/Fabric_Broker/$name" \
            || die "the bundle's Contents/Resources/isabelle/Fabric_Broker/$name differs from the repository's isabelle/Fabric_Broker/$name."
    done
    for roots in "$rules/ROOTS" "$rules/Fabric_Broker/ROOTS"; do
        [ ! -e "$roots" ] || die "the bundle has a ROOTS file beside the rule files: $roots"
    done
    listing=$(CDPATH= cd -- "$rules" && find . -mindepth 1 | LC_ALL=C sort | tr '\n' ' ')
    [ "$listing" = "./Fabric_Broker ./Fabric_Broker/Fabric_Broker.thy ./Fabric_Broker/ROOT " ] \
        || die "the bundle's Contents/Resources/isabelle must hold only Fabric_Broker/ROOT and Fabric_Broker/Fabric_Broker.thy; it holds: $listing"
    echo "ok: the bundle holds the repository's two rule files and no ROOTS"
}

packaged() {
    [ "$(uname -s)" = "Darwin" ] || die "the bundle exists only on macOS."
    APP="$ROOT/target/debug/bundle/macos/Fabric Calendar.app"
    [ -d "$APP" ] || die "there is no bundle; run sh scripts/build-desktop.sh --configuration debug first."

    WORK=$(mktemp -d)
    # The copy runs from the folder's physical path: Tauri does not resolve the resources folder
    # of an executable whose path goes through a symbolic link (on macOS /var is one).
    WORK=$(cd "$WORK" && pwd -P)
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
    rule_files "$COPY"

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

# How long the first start may take before its first request notice: the server's start, the
# build of the Fabric_Broker session heap on a slow Mac (the server allows 600 s for it), and a
# check.
FIRST_START_SECONDS=900

installed() {
    [ "$(uname -s)" = "Darwin" ] || die "the installed proofs run only on macOS."
    APP="$ROOT/target/debug/bundle/macos/Fabric Calendar.app"
    [ -d "$APP" ] || die "there is no bundle; run sh scripts/build-desktop.sh --configuration debug first."
    ARCHIVE="$ROOT/.tooling/isabelle/download/Isabelle2025-2_macos.tar.gz"
    [ -f "$ARCHIVE" ] \
        || die "the pinned archive $ARCHIVE is missing; run sh scripts/setup-demo.sh by hand (this script never downloads)."
    INSTALLER="$ROOT/scripts/install-isabelle.sh"

    # mktemp makes the scratch directory with mode 0700. Every process this run starts has a path
    # below it in its command line: the copied bundle's host and broker, and the Isabelle that the
    # broker starts from the scratch installation.
    WORK=$(mktemp -d)
    # The copy runs from the folder's physical path: Tauri does not resolve the resources folder
    # of an executable whose path goes through a symbolic link (on macOS /var is one).
    WORK=$(cd "$WORK" && pwd -P)
    host_pid=
    started_here() {
        ps -axwwo pid=,command= | WORK_PATH="$WORK" awk 'index($0, ENVIRON["WORK_PATH"]) { print $1 }'
    }
    cleanup() {
        if [ -n "$host_pid" ]; then
            kill -KILL "$host_pid" 2>/dev/null || true
        fi
        for pass in 1 2; do
            for pid in $(started_here); do
                kill -KILL "$pid" 2>/dev/null || true
            done
            [ "$pass" = 2 ] || sleep 1
        done
        rm -rf "$WORK"
    }
    trap cleanup EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    trap 'exit 129' HUP

    free_kb=$(df -Pk "$WORK" | awk 'NR == 2 { print $4 }')
    [ "$free_kb" -ge $((4 * 1024 * 1024)) ] \
        || die "Isabelle needs about 4 GB in $WORK; its volume has $((free_kb / 1024)) MB free."

    SPACED="$WORK/a path with spaces"
    mkdir "$SPACED"
    # The host's own folder in this run: Isabelle is installed into it, and the broker's store is
    # its broker/ folder.
    SUPPORT="$SPACED/Application Support"
    ISABELLE="$SUPPORT/Isabelle2025-2"

    echo "[1/6] The installer"
    sh "$INSTALLER" --support-dir "$SUPPORT" --archive "$ARCHIVE" >"$WORK/install.log" 2>&1 \
        || { sed 's/^/    /' "$WORK/install.log" >&2; die "the installer failed."; }
    sed 's/^/    /' "$WORK/install.log"
    [ "$(tail -n 1 "$WORK/install.log")" = "Isabelle2025-2 is installed in $ISABELLE. Quit Fabric Calendar and open it again to use it." ] \
        || die "the installer did not end with the line that names $ISABELLE."
    [ "$(stat -f %Lp "$SUPPORT")" = 700 ] || die "the installer created $SUPPORT with mode $(stat -f %Lp "$SUPPORT"), not 700."
    [ "$(ls -A "$SUPPORT")" = Isabelle2025-2 ] || die "the installer left more than Isabelle2025-2 in $SUPPORT: $(ls -A "$SUPPORT" | tr '\n' ' ')"
    [ -f "$ISABELLE/bin/isabelle" ] || die "the installation has no bin/isabelle."
    [ "$(cat "$ISABELLE/etc/ISABELLE_IDENTIFIER")" = Isabelle2025-2 ] || die "the installation's etc/ISABELLE_IDENTIFIER is not Isabelle2025-2."
    echo "ok: Isabelle2025-2 is in $ISABELLE"

    echo "[2/6] The installer again"
    output=$(sh "$INSTALLER" --support-dir "$SUPPORT" --archive "$ARCHIVE" 2>&1) \
        || die "the second run of the installer failed: $output"
    [ "$output" = "Isabelle2025-2 is already installed in $ISABELLE; nothing to do." ] \
        || die "the second run of the installer did not say that Isabelle is already installed: $output"
    echo "ok: $output"

    echo "[3/6] The installer with a wrong archive"
    printf 'not the Isabelle archive\n' >"$WORK/wrong.tar.gz"
    mkdir "$WORK/second"
    status=0
    output=$(sh "$INSTALLER" --support-dir "$WORK/second/Application Support" --archive "$WORK/wrong.tar.gz" 2>&1) || status=$?
    [ "$status" -ne 0 ] || die "the installer accepted a wrong archive: $output"
    case "$output" in
        *"does not match the pinned Isabelle2025-2 archive"*"bytes, received 25."*"Nothing was unpacked."*) ;;
        *) die "the installer refused the wrong archive with another message: $output" ;;
    esac
    [ -z "$(ls -A "$WORK/second")" ] || die "the refused installation left something behind: $(ls -A "$WORK/second" | tr '\n' ' ')"
    # A wrong archive of exactly the pinned size, so that the SHA-256 refuses it. The file is
    # sparse and takes almost no room.
    rm -f "$WORK/wrong.tar.gz"
    dd if=/dev/null of="$WORK/wrong.tar.gz" bs=1 seek="$(wc -c <"$ARCHIVE" | tr -d ' ')" 2>/dev/null \
        || die "could not make a wrong archive of the pinned size."
    status=0
    output=$(sh "$INSTALLER" --support-dir "$WORK/second/Application Support" --archive "$WORK/wrong.tar.gz" 2>&1) || status=$?
    [ "$status" -ne 0 ] || die "the installer accepted a wrong archive of the pinned size: $output"
    case "$output" in
        *"does not match the pinned Isabelle2025-2 archive: expected SHA-256"*"Nothing was unpacked."*) ;;
        *) die "the installer refused the wrong archive of the pinned size with another message: $output" ;;
    esac
    [ -z "$(ls -A "$WORK/second")" ] || die "the refused installation left something behind: $(ls -A "$WORK/second" | tr '\n' ' ')"
    rm -f "$WORK/wrong.tar.gz"
    echo "ok: the installer refused a small wrong archive and one of the pinned size, and left nothing behind"

    echo "[4/6] The bundle"
    cp -R "$APP" "$SPACED/"
    COPY="$SPACED/Fabric Calendar.app"
    HOST="$COPY/Contents/MacOS/fabric-desktop"
    BROKER="$COPY/Contents/MacOS/fabric-broker"
    codesign --verify --deep --strict "$COPY" || die "the copied bundle's signature does not verify."
    echo "The bundle under test: $COPY"
    rule_files "$COPY"
    resources_state() {
        find "$COPY/Contents/Resources" -exec stat -f '%m %z %N' {} + | LC_ALL=C sort
    }
    resources_state >"$WORK/resources.before"

    echo "[5/6] The application with the installed Isabelle"
    # The developer variables are unset, so the host passes the installed layout: the bundle's
    # resources and its own folder, which FABRIC_DESKTOP_DATA_DIR moves to the scratch directory.
    # The subshell ends in exec, so $! is the host's own process id.
    (
        unset FABRIC_BROKER_ISABELLE_HOME FABRIC_BROKER_ISABELLE_USER_HOME FABRIC_BROKER_RULES_DIR
        unset FABRIC_BROKER_APP_RESOURCES_DIR FABRIC_BROKER_APP_SUPPORT_DIR
        FABRIC_DESKTOP_DATA_DIR="$SUPPORT" FABRIC_BROKER_KEY_SOURCE=test FABRIC_BROKER_SYNTHETIC_AGENT=1 \
            FABRIC_DESKTOP_TRACE=1 exec "$HOST"
    ) 2>"$WORK/app.trace" &
    host_pid=$!
    wait_for "$WORK/app.trace" ' state run=[0-9]+ (connected|conflict|configuration_error)' 30
    if grep -Eq ' (conflict|configuration_error)' "$WORK/app.trace"; then
        sed 's/^/    /' "$WORK/app.trace" >&2
        die "the application did not start its broker."
    fi
    wait_for "$WORK/app.trace" ' state run=1 connected pid=[0-9]+ ' 1
    broker_pid=$(sed -n 's/.* state run=1 connected pid=\([0-9]*\) .*/\1/p' "$WORK/app.trace" | head -n 1)
    echo "Waiting up to $((FIRST_START_SECONDS / 60)) minutes for the first change=request notice. The practice agent proposes only"
    echo "once the rule checker is ready, and this first start builds Isabelle's Fabric_Broker session heap"
    echo "in $SUPPORT/isabelle-user-home."
    wait_for "$WORK/app.trace" ' <- notice sequence=[0-9]+ change=request ' "$FIRST_START_SECONDS"
    # A run that found no Isabelle is a synthetic run whose classifier also proposes, so the
    # notice shows a live Isabelle only together with these two: the host passed the installed
    # layout, and the broker, which writes to the host's stderr, did not say it found none.
    grep -Eq ' state run=1 connected pid=[0-9]+ .* installed$' "$WORK/app.trace" \
        || die "the host did not pass the installed layout to its broker."
    if grep -Eq 'fabric-broker: Isabelle2025-2 is (not installed|taken as not installed)' "$WORK/app.trace"; then
        sed 's/^/    /' "$WORK/app.trace" >&2
        die "the broker did not find the installed Isabelle."
    fi
    echo "ok: a request notice arrived from broker $broker_pid with the rule checker of the installed layout"

    echo "[6/6] Quit, then the bundle, the heap and the resources"
    kill -KILL "$host_pid" 2>/dev/null || true
    wait "$host_pid" 2>/dev/null || true
    host_pid=
    # The broker stops Isabelle when its input closes; both end within half a minute.
    tries=150
    while [ -n "$(started_here)" ] && [ "$tries" -gt 0 ]; do
        sleep 0.2
        tries=$((tries - 1))
    done
    [ -z "$(started_here)" ] \
        || die "processes of this run still run 30 s after the application ended: $(started_here | tr '\n' ' ')"
    codesign --verify --deep --strict "$COPY" || die "the bundle's signature no longer verifies after the run."
    resources_state >"$WORK/resources.after"
    cmp -s "$WORK/resources.before" "$WORK/resources.after" || {
        diff "$WORK/resources.before" "$WORK/resources.after" | sed 's/^/    /' >&2 || true
        die "something was written below the bundle's Contents/Resources."
    }
    heap=$(find "$SUPPORT/isabelle-user-home" -type f -path '*/heaps/*' -name Fabric_Broker 2>/dev/null | head -n 1)
    [ -n "$heap" ] || die "there is no Fabric_Broker heap below $SUPPORT/isabelle-user-home."
    echo "ok: the signature verifies, nothing changed below Contents/Resources, and the heap is $heap"

    echo "The trace of the run:"
    sed 's/^/    /' "$WORK/app.trace"
}

case "$mode" in
    sources) sources ;;
    packaged) packaged ;;
    live) live ;;
    installed) installed ;;
esac

echo "All desktop checks of this mode passed."
