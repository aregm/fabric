#!/usr/bin/env sh
# Builds the Fabric Calendar desktop application as an arm64 .app bundle (work package CD-W02).
#
#   sh scripts/build-desktop.sh --configuration debug
#
# What it does, in order:
#   1. builds the broker (apps/fabric-broker) and puts it where the Tauri bundler looks for it;
#   2. installs the pinned npm development packages of apps/fabric-desktop (the Tauri CLI, the
#      TypeScript compiler and the Node types) from the lockfile;
#   3. runs the Tauri CLI, which builds the webview files and the Tauri host and bundles
#      "Fabric Calendar.app" with the broker in Contents/MacOS;
#   4. checks the bundle: both executables are arm64 only, the name, the identifier and the
#      minimum macOS version are the decided ones, the signature is ad hoc and valid, and the
#      rule theory's two files (isabelle/Fabric_Broker/ROOT and Fabric_Broker.thy) are in
#      Contents/Resources/isabelle/Fabric_Broker, equal to the repository's, with nothing beside
#      them (no ROOTS).
#
# Everything is done from the command line. There is no Xcode project and no step in a GUI.
#
# Signing is ad hoc only. Developer ID signing, notarization and the DMG are work package CD-W11
# and need the owner; this script refuses the flags for them. It also removes every Apple signing
# and notarization variable from the environment of the build, so that a value in the caller's
# shell cannot make the Tauri CLI use a real identity or reach the Keychain.
#
# Needs macOS on Apple Silicon, Xcode's command line tools, Node 22.18 or later with npm, and the
# repository's Rust toolchain in .tooling (scripts/setup.sh installs it).
set -eu

die() {
    echo "error: $*" >&2
    exit 1
}

usage="usage: sh scripts/build-desktop.sh --configuration debug"

configuration=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --configuration)
            [ "$#" -ge 2 ] || die "$usage"
            configuration=$2
            shift 2
            ;;
        --sign | --notarize)
            die "$1 is not implemented: signing with a Developer ID and notarization are work package CD-W11 and need the owner."
            ;;
        *)
            die "$usage"
            ;;
    esac
done

case "$configuration" in
    debug) ;;
    release) die "the release configuration is not implemented: release builds are work package CD-W11." ;;
    *) die "$usage" ;;
esac

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
RUN="$ROOT/scripts/run.sh"
DESKTOP="$ROOT/apps/fabric-desktop"
TARGET_TRIPLE=aarch64-apple-darwin
APP="$ROOT/target/debug/bundle/macos/Fabric Calendar.app"

# The configuration is checked before anything is built.
[ "$(uname -s)" = "Darwin" ] || die "the desktop application builds only on macOS."
[ "$(uname -m)" = "arm64" ] || die "the desktop application builds only on Apple Silicon (arm64)."
[ -x "$ROOT/.tooling/cargo/bin/cargo" ] || die "the Rust toolchain is missing; run sh scripts/setup.sh first."
command -v node >/dev/null 2>&1 || die "node was not found; install Node 22.18 or later."
command -v npm >/dev/null 2>&1 || die "npm was not found; it comes with Node."
node -e '
const [major, minor] = process.versions.node.split(".").map(Number);
if (major < 22 || (major === 22 && minor < 18)) {
    console.error("error: Node " + process.versions.node + " cannot run TypeScript files directly; install Node 22.18 or later.");
    process.exit(1);
}'
for tool in codesign lipo /usr/libexec/PlistBuddy; do
    command -v "$tool" >/dev/null 2>&1 || die "$tool was not found; install Xcode's command line tools."
done

echo "[1/4] The broker"
unset OPENSSL_DIR OPENSSL_LIB_DIR OPENSSL_INCLUDE_DIR
sh "$RUN" cargo build --locked -p fabric-broker
mkdir -p "$DESKTOP/src-tauri/binaries"
cp "$ROOT/target/debug/fabric-broker" "$DESKTOP/src-tauri/binaries/fabric-broker-$TARGET_TRIPLE"

echo "[2/4] The npm development packages"
(cd "$DESKTOP" && npm ci --no-audit --no-fund)

echo "[3/4] The webview, the Tauri host and the bundle"
rm -rf "$APP"
sh "$RUN" env \
    -u APPLE_SIGNING_IDENTITY -u APPLE_CERTIFICATE -u APPLE_CERTIFICATE_PASSWORD \
    -u APPLE_ID -u APPLE_PASSWORD -u APPLE_TEAM_ID \
    -u APPLE_API_KEY -u APPLE_API_ISSUER -u APPLE_API_KEY_PATH \
    -u APPLE_PROVIDER_SHORT_NAME \
    npm --prefix "$DESKTOP" run tauri -- build --debug --bundles app -- --locked

echo "[4/4] The bundle"
[ -d "$APP" ] || die "the bundle was not made: $APP"
for executable in fabric-desktop fabric-broker; do
    path="$APP/Contents/MacOS/$executable"
    [ -x "$path" ] || die "the bundle has no executable $executable."
    architectures=$(lipo -archs "$path")
    [ "$architectures" = "arm64" ] || die "$executable is built for '$architectures', not for arm64 only."
done
plist() {
    /usr/libexec/PlistBuddy -c "Print :$1" "$APP/Contents/Info.plist"
}
[ "$(plist CFBundleName)" = "Fabric Calendar" ] || die "the bundle's name is '$(plist CFBundleName)'."
[ "$(plist CFBundleIdentifier)" = "ai.paracels.fabric.calendar" ] || die "the bundle's identifier is '$(plist CFBundleIdentifier)'."
[ "$(plist LSMinimumSystemVersion)" = "15.0" ] || die "the bundle's minimum macOS version is '$(plist LSMinimumSystemVersion)'."
codesign --verify --deep --strict "$APP" || die "the bundle's signature does not verify."
signature=$(codesign -dv "$APP" 2>&1)
echo "$signature" | grep -q '^Signature=adhoc$' || die "the bundle is not signed ad hoc."
echo "$signature" | grep -q '^TeamIdentifier=not set$' || die "the bundle carries a team identifier; this build must be ad hoc."
# The installed application reads its rule theory from the bundle: exactly the repository's two
# files of the Fabric_Broker session, and no ROOTS that would make Isabelle look for others.
rules="$APP/Contents/Resources/isabelle"
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

echo "Built: $APP"
echo "Check it with: sh scripts/verify-desktop.sh --packaged"
