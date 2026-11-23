#!/usr/bin/env sh
# Installs Isabelle2025-2 for macOS where the installed Fabric Calendar looks for it: the
# application's own folder for this user, ~/Library/Application Support/ai.paracels.fabric.calendar,
# as <that folder>/Isabelle2025-2.
#
#   sh scripts/install-isabelle.sh [--support-dir <dir>] [--archive <file> | --url <url>]
#
#   --support-dir <dir>  install into <dir>/Isabelle2025-2 instead (an absolute path; for tests)
#   --archive <file>     use this local archive and download nothing
#   --url <url>          download the archive from this place instead of the Isabelle site
#   --help               print this text
#
# What it does, in order:
#   1. if <support>/Isabelle2025-2 already exists: says so and stops when it is Isabelle2025-2,
#      and refuses otherwise; it never deletes or replaces a folder it did not create;
#   2. downloads the pinned archive (about 1.7 GB) into a temporary file, or takes --archive;
#   3. checks the archive's size and SHA-256 against the pin of scripts/setup-demo.sh before
#      anything is unpacked;
#   4. unpacks it into a new temporary folder inside the support folder and renames the unpacked
#      Isabelle2025-2.app to <support>/Isabelle2025-2 in one step, after checking that it holds
#      bin/isabelle and that etc/ISABELLE_IDENTIFIER reads Isabelle2025-2.
#
# It runs nothing from the archive and does not build the session heap: the first start of the
# application builds it. It creates the support folder (mode 0700) only when it is missing and
# its parent exists. It never creates broker/ and never touches the broker's store. No sudo.
# On a failure or an interrupt it removes its temporary folder and its download.
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

USAGE="usage: sh scripts/install-isabelle.sh [--support-dir <dir>] [--archive <file> | --url <url>]"

# The same pin as scripts/setup-demo.sh; scripts/verify-desktop.sh checks that these four lines
# are equal in both scripts.
ISABELLE_VERSION="Isabelle2025-2"
ARCHIVE_BYTES=1730395791
ARCHIVE_SHA256="8f187496e295f169952e944745af9e4ae00c9c1cd2ed4cadbcf7d898e444913e"
ARCHIVE_URL="https://isabelle.in.tum.de/website-$ISABELLE_VERSION/dist/${ISABELLE_VERSION}_macos.tar.gz"

support_dir=
archive=
url=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --support-dir | --archive | --url)
            [ "$#" -ge 2 ] && [ -n "$2" ] || die "$USAGE"
            case "$1" in
                --support-dir) support_dir=$2 ;;
                --archive) archive=$2 ;;
                --url) url=$2 ;;
            esac
            shift 2
            ;;
        --help | -h)
            sed -n '2,/^set -eu$/p' "$0" | sed -e '$d' -e 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            die "$USAGE"
            ;;
    esac
done
[ -z "$archive" ] || [ -z "$url" ] || die "give --archive or --url, not both."

[ "$(uname -s)" = Darwin ] || die "this script installs Isabelle for the macOS application Fabric Calendar; on this system there is nothing to install."

if [ -z "$support_dir" ]; then
    [ -n "${HOME:-}" ] || die "HOME is not set; name the folder with --support-dir."
    support_dir="$HOME/Library/Application Support/ai.paracels.fabric.calendar"
fi
case "$support_dir" in
    /*) ;;
    *) die "--support-dir must be an absolute path; this one is not: $support_dir" ;;
esac
# Without a trailing slash, so that the messages name the folders plainly.
while [ "${support_dir%/}" != "$support_dir" ] && [ "$support_dir" != / ]; do
    support_dir=${support_dir%/}
done
TARGET="$support_dir/$ISABELLE_VERSION"

# What the application checks before it starts Isabelle: bin/isabelle is a file and
# etc/ISABELLE_IDENTIFIER, without surrounding white space, reads Isabelle2025-2.
installed() {
    [ -f "$1/bin/isabelle" ] && [ -f "$1/etc/ISABELLE_IDENTIFIER" ] || return 1
    identifier=$(awk 'NF' "$1/etc/ISABELLE_IDENTIFIER" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')
    [ "$identifier" = "$ISABELLE_VERSION" ]
}

if [ -e "$TARGET" ] || [ -L "$TARGET" ]; then
    if installed "$TARGET"; then
        echo "$ISABELLE_VERSION is already installed in $TARGET; nothing to do."
        exit 0
    fi
    found=
    if [ -f "$TARGET/bin/isabelle" ] && [ -f "$TARGET/etc/ISABELLE_IDENTIFIER" ]; then
        found=" It holds ${identifier:-an empty identifier}."
    fi
    die "$TARGET exists but is not an $ISABELLE_VERSION installation (it needs bin/isabelle and etc/ISABELLE_IDENTIFIER reading $ISABELLE_VERSION).$found Move it away and run this again; this script does not delete it."
fi
if [ -e "$support_dir" ] && [ ! -d "$support_dir" ]; then
    die "$support_dir exists but is not a folder."
fi
if [ ! -d "$support_dir" ]; then
    parent=$(dirname -- "$support_dir")
    [ -d "$parent" ] || die "the folder $parent does not exist; this script creates only $support_dir itself."
fi

# Everything this run creates, removed again unless the run succeeds.
download_dir=
unpack_dir=
created_support=
finished=
cleanup() {
    if [ -n "$unpack_dir" ]; then
        rm -rf "$unpack_dir"
    fi
    if [ -n "$download_dir" ]; then
        rm -rf "$download_dir"
    fi
    if [ -n "$created_support" ] && [ -z "$finished" ]; then
        rmdir "$support_dir" 2>/dev/null || true
    fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

if [ -n "$archive" ]; then
    [ -f "$archive" ] || die "the archive $archive is not a file."
else
    [ -n "$url" ] || url=$ARCHIVE_URL
    command -v curl >/dev/null 2>&1 || die "curl was not found."
    download_dir=$(mktemp -d)
    archive="$download_dir/${ISABELLE_VERSION}_macos.tar.gz"
    echo "Downloading $ISABELLE_VERSION for macOS (about 1.7 GB) from $url..."
    curl --fail --location --retry 5 --retry-delay 10 --retry-all-errors \
        --silent --show-error --output "$archive" "$url" \
        || die "downloading $ISABELLE_VERSION from $url failed; nothing was installed."
fi

echo "Checking the archive's size and SHA-256..."
actual_bytes=$(wc -c < "$archive" | tr -d ' ')
if [ "$actual_bytes" != "$ARCHIVE_BYTES" ]; then
    die "the archive $archive does not match the pinned $ISABELLE_VERSION archive: expected $ARCHIVE_BYTES bytes, received $actual_bytes. Nothing was unpacked."
fi
actual_hash=$(hash_file "$archive")
if [ "$actual_hash" != "$ARCHIVE_SHA256" ]; then
    die "the archive $archive does not match the pinned $ISABELLE_VERSION archive: expected SHA-256 $ARCHIVE_SHA256, received $actual_hash. Nothing was unpacked."
fi

if [ ! -d "$support_dir" ]; then
    mkdir -m 700 "$support_dir" || die "could not create $support_dir."
    created_support=1
fi
unpack_dir=$(mktemp -d "$support_dir/.install-isabelle.XXXXXX")

echo "Unpacking $ISABELLE_VERSION (about 3.3 GB)..."
tar -xzf "$archive" -C "$unpack_dir" || die "unpacking $archive failed; nothing was installed."
unpacked="$unpack_dir/$ISABELLE_VERSION.app"
[ -d "$unpacked" ] || die "the archive did not hold $ISABELLE_VERSION.app; nothing was installed."
installed "$unpacked" || die "the unpacked $ISABELLE_VERSION.app has no bin/isabelle or its etc/ISABELLE_IDENTIFIER is not $ISABELLE_VERSION; nothing was installed."

# Checked again just before the rename: a folder that appeared meanwhile is never replaced.
if [ -e "$TARGET" ] || [ -L "$TARGET" ]; then
    die "$TARGET appeared while this script ran; it was left as it is and nothing was installed."
fi
mv "$unpacked" "$TARGET" || die "could not move the unpacked $ISABELLE_VERSION to $TARGET; nothing was installed."
# A folder that appeared between that check and the move takes the copy inside it instead: the
# copy is this run's, so it goes back to the temporary folder, which is then removed.
if [ -e "$TARGET/$ISABELLE_VERSION.app" ]; then
    mv "$TARGET/$ISABELLE_VERSION.app" "$unpacked" \
        || die "$TARGET appeared while this script ran, and this run's copy $TARGET/$ISABELLE_VERSION.app could not be moved out of it; remove that copy by hand."
    die "$TARGET appeared while this script ran; it was left as it is and nothing was installed."
fi
finished=1
installed "$TARGET" || die "$TARGET does not hold $ISABELLE_VERSION after the move."

echo "$ISABELLE_VERSION is installed in $TARGET. Quit Fabric Calendar and open it again to use it."
