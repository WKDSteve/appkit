#!/bin/sh
# Install an app for the current user (Linux, macOS), without administrator rights (release-kit of appkit).
#
#   install.sh [--version TAG | --rolling] [-- ARGS...]
#   install.sh --file FILE [--sums FILE | --sha256 HEX] [-- ARGS...]
#
# It downloads the build of this system from the GitHub release of the app, checks it against the
# SHA256SUMS of that release, and installs it in the way the app is built:
#
#   hub       a program with its own `install` command (a .tar.gz with the program, or the bare
#             program): runs `<program> install ARGS`, which copies it to its fixed place
#   appimage  an AppImage: copied to ${XDG_BIN_HOME:-~/.local/bin}/<program> and made executable
#
# The copy attached to a release is stamped by the release workflow: it knows its repository, its
# app and its channel (the release it came with, or the rolling build `latest`) and runs without
# options:
#
#   curl -fsSL https://github.com/OWNER/NAME/releases/download/latest/install.sh | sh
#
# Options:
#   --version TAG     a release to install instead of the channel of this copy
#   --rolling         the rolling build of main (the pre-release `latest`) instead
#   --file FILE       a build you already have (no download); the checksum then comes from
#                     --sums FILE, --sha256 HEX, or a SHA256SUMS next to the file
#   --allow-root      install although this runs as root (a build machine)
#   --repo, --app, --kind, --program, --release-asset, --rolling-asset
#                     what the release workflow stamps into this copy; given here, they win.
#                     --release-asset may hold {version} (the tag without its v) and {triple}
#   --help            this text
#   -- ARGS...        more arguments for `<program> install` (hub)
#
# Needs: sh, tar (for an archive), curl or wget (for a download), sha256sum or shasum.

set -eu

# release-kit:stamp:begin (filled by the publish action of release-kit; empty in the source)
STAMP_REPO=""
STAMP_CHANNEL=""
STAMP_APP=""
STAMP_KIND=""
STAMP_PROGRAM=""
STAMP_RELEASE_ASSET=""
STAMP_ROLLING_ASSET=""
# release-kit:stamp:end

REPO="$STAMP_REPO"
CHANNEL="$STAMP_CHANNEL"
APP="$STAMP_APP"
KIND="$STAMP_KIND"
PROGRAM="$STAMP_PROGRAM"
RELEASE_ASSET="$STAMP_RELEASE_ASSET"
ROLLING_ASSET="$STAMP_ROLLING_ASSET"
FILE=""
SUMS=""
SHA256=""
ALLOW_ROOT=""

say() { printf '%s\n' "$*"; }
fail() {
    printf 'install.sh: %s\n' "$*" >&2
    exit 1
}

usage() {
    # Piped into sh there is no file to read the header from.
    if [ -f "$0" ] && head -n 2 "$0" | grep -q 'release-kit'; then
        sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'
    else
        say "install.sh of release-kit: https://github.com/WKDSteve/appkit/tree/main/release-kit"
    fi
}

need_value() {
    [ $# -ge 2 ] || fail "$1 needs a value"
}

while [ $# -gt 0 ]; do
    case "$1" in
        --version) need_value "$@"; CHANNEL="$2"; shift 2 ;;
        --rolling) CHANNEL="latest"; shift ;;
        --file | --archive) need_value "$@"; FILE="$2"; shift 2 ;;
        --sums) need_value "$@"; SUMS="$2"; shift 2 ;;
        --sha256) need_value "$@"; SHA256="$2"; shift 2 ;;
        --allow-root) ALLOW_ROOT=1; shift ;;
        --repo) need_value "$@"; REPO="$2"; shift 2 ;;
        --app) need_value "$@"; APP="$2"; shift 2 ;;
        --kind) need_value "$@"; KIND="$2"; shift 2 ;;
        --program) need_value "$@"; PROGRAM="$2"; shift 2 ;;
        --release-asset) need_value "$@"; RELEASE_ASSET="$2"; shift 2 ;;
        --rolling-asset) need_value "$@"; ROLLING_ASSET="$2"; shift 2 ;;
        -h | --help) usage; exit 0 ;;
        --) shift; break ;;
        *) fail "unknown option: $1 (see --help)" ;;
    esac
done
# What is left are the arguments of the install step.

plain() {
    case "$1" in
        "" | *[!A-Za-z0-9._-]*) return 1 ;;
        *) return 0 ;;
    esac
}

[ -n "$APP" ] || APP="app"
plain "$APP" || fail "--app must be a plain word: $APP"
case "$KIND" in
    hub | appimage) ;;
    *) fail "--kind must be hub or appimage (this copy is not stamped: give --kind and the rest)" ;;
esac
plain "$PROGRAM" || fail "--program must be the file name of the program"

# The app belongs to one user: as root it would be root's.
if [ -z "$ALLOW_ROOT" ] && [ "$(id -u)" = "0" ]; then
    fail "do not run this as root: it installs for one user (--allow-root for a build machine)"
fi

fetch() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL --proto '=https' --proto-redir '=https' -o "$2" "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget -q --https-only -O "$2" "$1"
    else
        fail "need curl or wget to download"
    fi
}

# A private repository answers an anonymous download with 404. With a token in GH_TOKEN (or
# GITHUB_TOKEN; read access to the repository is enough) a file is fetched through the API instead:
# the asset id is looked up by name, then the file comes from the asset address; curl does not pass
# the token on to the host the API redirects to.
TOKEN="${GH_TOKEN:-${GITHUB_TOKEN:-}}"

# fetch_release_file REPO CHANNEL NAME OUT
fetch_release_file() {
    if [ -z "$TOKEN" ]; then
        fetch "https://github.com/$1/releases/download/$2/$3" "$4"
        return
    fi
    command -v curl >/dev/null 2>&1 || fail "need curl to download with a token"
    curl -fsSL --proto '=https' --proto-redir '=https' -H "Authorization: Bearer $TOKEN" \
        -o "$WORK/release.json" "https://api.github.com/repos/$1/releases/tags/$2" || return 1
    # In an asset the "id" comes right before "node_id" and "name"; the file name is a plain word.
    id="$(tr -d '\n' <"$WORK/release.json" |
        sed -n "s/.*\"id\"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\),[[:space:]]*\"node_id\"[[:space:]]*:[[:space:]]*\"[^\"]*\",[[:space:]]*\"name\"[[:space:]]*:[[:space:]]*\"$3\".*/\1/p" |
        head -n 1)"
    [ -n "$id" ] || return 1
    curl -fsSL --proto '=https' --proto-redir '=https' -H "Authorization: Bearer $TOKEN" \
        -H "Accept: application/octet-stream" -o "$4" "https://api.github.com/repos/$1/releases/assets/$id"
}

# The tag of the newest published release; empty when the repository has none (404). Any other
# failure (no network, a rate limit) stops: it must not quietly turn into the rolling build.
newest_tag() {
    if command -v curl >/dev/null 2>&1; then
        if [ -n "$TOKEN" ]; then
            code="$(curl -sSL --proto '=https' --proto-redir '=https' -H "Authorization: Bearer $TOKEN" -o "$WORK/newest.json" -w '%{http_code}' "$1")" ||
                fail "cannot ask GitHub for the newest release of $REPO"
        else
            code="$(curl -sSL --proto '=https' --proto-redir '=https' -o "$WORK/newest.json" -w '%{http_code}' "$1")" ||
                fail "cannot ask GitHub for the newest release of $REPO"
        fi
    else
        fail "need curl to ask GitHub for the newest release (or give --version TAG or --rolling)"
    fi
    case "$code" in
        200) sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$WORK/newest.json" | head -n 1 ;;
        404) ;;
        *) fail "GitHub answered $code for the newest release of $REPO (try again later, or give --version TAG)" ;;
    esac
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d ' ' -f 1
    else
        fail "need sha256sum or shasum to check the file"
    fi
}

WORK="$(mktemp -d 2>/dev/null || mktemp -d -t "$APP-install")"
trap 'rm -rf "$WORK"' EXIT INT TERM

if [ -z "$FILE" ]; then
    case "$REPO" in
        */*)
            if ! plain "${REPO%%/*}" || ! plain "${REPO#*/}"; then
                fail "--repo must be OWNER/NAME"
            fi
            ;;
        *) fail "--repo must be OWNER/NAME (this copy is not stamped)" ;;
    esac
    case "$(uname -s)-$(uname -m)" in
        Linux-x86_64) TRIPLE="x86_64-unknown-linux-gnu" ;;
        Darwin-arm64) TRIPLE="aarch64-apple-darwin" ;;
        *) fail "no build for $(uname -s) on $(uname -m)" ;;
    esac

    # The channel: what was asked, else what this copy was stamped with, else the newest published
    # release (and the rolling build when there is none yet).
    if [ -z "$CHANNEL" ]; then
        say "looking for the newest release of $REPO"
        CHANNEL="$(newest_tag "https://api.github.com/repos/$REPO/releases/latest")"
        if [ -z "$CHANNEL" ]; then
            say "no published release yet: taking the rolling build of main"
            CHANNEL="latest"
        fi
    fi
    # A tag is a plain word: it goes into a URL and a file name.
    plain "$CHANNEL" || fail "the version has a character that is not allowed: $CHANNEL"
    if [ "$CHANNEL" = "latest" ]; then
        PATTERN="$ROLLING_ASSET"
    else
        PATTERN="$RELEASE_ASSET"
    fi
    # A build is made for Linux x86-64 unless its name says the system ({triple}), in the rolling
    # build and in a release alike (an AppImage has none): on another system its file would
    # download, pass its checksum and then not run.
    case "$PATTERN" in
        "" | *"{triple}"*) ;;
        *)
            [ "$TRIPLE" = "x86_64-unknown-linux-gnu" ] ||
                fail "the build of this channel ($CHANNEL) is for Linux x86-64 only, not for $TRIPLE"
            ;;
    esac
    NAME="$(printf '%s' "$PATTERN" | sed -e "s/{version}/${CHANNEL#v}/g" -e "s/{triple}/$TRIPLE/g")"
    [ -n "$NAME" ] || fail "the channel $CHANNEL has no build for this system"
    plain "$NAME" || fail "the file name is not a plain word: $NAME"

    BASE="https://github.com/$REPO/releases/download/$CHANNEL"
    say "downloading $NAME ($CHANNEL)"
    fetch_release_file "$REPO" "$CHANNEL" "$NAME" "$WORK/$NAME" || fail "cannot download $BASE/$NAME (a private repository needs GH_TOKEN)"
    if ! fetch_release_file "$REPO" "$CHANNEL" "SHA256SUMS" "$WORK/SHA256SUMS"; then
        # It cannot be told from here whether a build is being published or the last one stopped half way.
        [ "$CHANNEL" = "latest" ] && fail "SHA256SUMS of the rolling build is missing: either a build is being published right now (try again in a few minutes), or its last publish failed and the next green build of the app brings them back"
        fail "cannot download $BASE/SHA256SUMS"
    fi
    FILE="$WORK/$NAME"
    SUMS="$WORK/SHA256SUMS"
else
    [ -f "$FILE" ] || fail "no such file: $FILE"
    # An absolute path: a bare name would be checked here but run from the PATH (an old install).
    case "$FILE" in
        /* | [A-Za-z]:/* | [A-Za-z]:\\*) ;;
        *) FILE="$PWD/$FILE" ;;
    esac
fi

# The checksum: never skipped. A file you have already needs one of the three sources.
LEAF="$(basename "$FILE")"
if [ -z "$SHA256" ]; then
    if [ -z "$SUMS" ] && [ -f "$(dirname "$FILE")/SHA256SUMS" ]; then
        SUMS="$(dirname "$FILE")/SHA256SUMS"
    fi
    [ -n "$SUMS" ] || fail "no checksum for $FILE: give --sums FILE or --sha256 HEX"
    [ -f "$SUMS" ] || fail "no such file: $SUMS"
    # Lines are "<hash>  <file>" (or "<hash> *<file>"); the file name must match whole.
    SHA256="$(awk -v f="$LEAF" '{ n = $2; sub(/^\*/, "", n); if (n == f && $1 ~ /^[0-9A-Fa-f]+$/ && length($1) == 64) { print $1; exit } }' "$SUMS")"
    [ -n "$SHA256" ] || fail "$LEAF is not listed in $SUMS"
fi
EXPECTED="$(printf '%s' "$SHA256" | tr 'A-F' 'a-f')"
ACTUAL="$(sha256_of "$FILE")"
[ "$ACTUAL" = "$EXPECTED" ] || fail "the checksum of $FILE does not match (expected $EXPECTED, got $ACTUAL): nothing was installed"
say "checksum ok"

case "$KIND" in
    hub)
        EXE="$FILE"
        case "$FILE" in
            *.tar.gz | *.tgz)
                # A path that leaves the folder it is unpacked into is refused before anything is
                # written.
                # Through stdin: GNU tar takes a name with a colon (C:/...) for a remote host.
                tar -tzf - < "$FILE" > "$WORK/entries" || fail "cannot read the archive $FILE"
                if grep -E '(^/|(^|/)\.\.(/|$))' "$WORK/entries" >/dev/null; then
                    fail "the archive holds a path that leaves its folder: nothing was installed"
                fi
                mkdir "$WORK/unpacked"
                tar -xzf - -C "$WORK/unpacked" < "$FILE"
                EXE="$WORK/unpacked/$PROGRAM"
                [ -f "$EXE" ] || fail "the archive holds no $PROGRAM"
                ;;
        esac
        chmod +x "$EXE"
        NEW_VERSION="$("$EXE" --version)" || fail "the program does not run"
        say "installing $NEW_VERSION"
        # `install` copies the program to its fixed place; its exit status is ours.
        "$EXE" install "$@" || fail "$PROGRAM install failed (see above)"
        say "installed $NEW_VERSION"
        ;;
    appimage)
        DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
        mkdir -p "$DIR"
        # Beside the target first, then one rename: a running copy is never half written.
        cp "$FILE" "$DIR/.$PROGRAM.new"
        chmod 755 "$DIR/.$PROGRAM.new"
        mv -f "$DIR/.$PROGRAM.new" "$DIR/$PROGRAM"
        say "installed $DIR/$PROGRAM"
        case ":$PATH:" in
            *":$DIR:"*) ;;
            *) say "note: $DIR is not on your PATH" ;;
        esac
        ;;
esac
