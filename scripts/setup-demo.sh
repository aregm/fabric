#!/usr/bin/env sh
# Installs Isabelle2025-2 for macOS into .tooling/isabelle/, the counterpart of scripts/setup-demo.ps1.
# Run scripts/setup.sh first for the Rust toolchain.
set -eu

die() {
    echo "error: $*" >&2
    exit 1
}

hash_file() {
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    elif command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v openssl >/dev/null 2>&1; then
        openssl dgst -sha256 "$1" | awk '{print $NF}'
    else
        die "SHA-256 tool not found; install shasum, sha256sum, or openssl."
    fi
}

[ "$(uname -s)" = Darwin ] || die "this script installs Isabelle for macOS; on Windows run scripts/setup-demo.ps1"

ISABELLE_VERSION="Isabelle2025-2"
# The Isabelle site publishes no checksum, so this pins the archive downloaded and approved on 2026-10-26.
ARCHIVE_BYTES=1730395791
ARCHIVE_SHA256="8f187496e295f169952e944745af9e4ae00c9c1cd2ed4cadbcf7d898e444913e"
# Since Isabelle2026 replaced it as the current release, Isabelle2025-2 is served from its own archived
# release site. The one macOS archive runs on Apple Silicon and Intel Macs.
ARCHIVE_URL="https://isabelle.in.tum.de/website-$ISABELLE_VERSION/dist/${ISABELLE_VERSION}_macos.tar.gz"

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
ISABELLE_TOOLING="$ROOT/.tooling/isabelle"
DOWNLOAD_DIR="$ISABELLE_TOOLING/download"
ARCHIVE="$DOWNLOAD_DIR/${ISABELLE_VERSION}_macos.tar.gz"
APP="$ISABELLE_TOOLING/$ISABELLE_VERSION.app"
# The Rust gate and scripts/isabelle.sh expect the distribution at this path on every platform.
ISABELLE_HOME_LINK="$ISABELLE_TOOLING/$ISABELLE_VERSION"

mkdir -p "$DOWNLOAD_DIR"

if [ ! -d "$APP" ]; then
    if [ ! -f "$ARCHIVE" ]; then
        partial="$ARCHIVE.part"
        echo "Downloading $ISABELLE_VERSION for macOS (about 1.7 GB; rerun this script to resume)..."
        curl --fail --location --retry 5 --retry-delay 10 --retry-all-errors --continue-at - \
            --silent --show-error --output "$partial" "$ARCHIVE_URL" \
            || die "downloading $ISABELLE_VERSION failed. Rerun this script to resume."
        mv -f "$partial" "$ARCHIVE"
    fi

    actual_bytes=$(wc -c < "$ARCHIVE" | tr -d ' ')
    actual_hash=$(hash_file "$ARCHIVE")
    if [ "$actual_bytes" != "$ARCHIVE_BYTES" ] || [ "$actual_hash" != "$ARCHIVE_SHA256" ]; then
        die "$ISABELLE_VERSION archive mismatch. Expected $ARCHIVE_BYTES bytes with SHA-256 $ARCHIVE_SHA256, received $actual_bytes bytes with $actual_hash. Delete $ARCHIVE only after investigating."
    fi

    echo "Unpacking $ISABELLE_VERSION into .tooling/isabelle..."
    tar -xzf "$ARCHIVE" -C "$ISABELLE_TOOLING"
    [ -d "$APP" ] || die "unpacking did not produce $APP"
fi

# The macOS archive is an application bundle whose distribution directory holds bin/isabelle.
distribution=$(find "$APP" -maxdepth 4 -type f -name isabelle -path '*/bin/isabelle' | head -n 1)
[ -n "$distribution" ] || die "no bin/isabelle inside $APP"
distribution=$(CDPATH= cd -- "$(dirname -- "$distribution")/.." && pwd -P)
rm -f "$ISABELLE_HOME_LINK"
ln -s "$distribution" "$ISABELLE_HOME_LINK"

reported=$(sh "$ROOT/scripts/isabelle.sh" version | tail -n 1)
[ "$reported" = "$ISABELLE_VERSION" ] || die "expected 'isabelle version' to report $ISABELLE_VERSION, received: $reported"

echo ""
echo "Isabelle for the Fabric demo is ready."
echo "  version: $ISABELLE_VERSION"
echo "  location: $distribution"
echo ""
echo "Run Isabelle tools without changing your shell:"
echo "  sh scripts/isabelle.sh version"
