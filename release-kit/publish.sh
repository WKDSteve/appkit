#!/usr/bin/env bash
# release-kit/publish.sh: put the built files of an app, its stamped install scripts and the checksums
# of all of them on a GitHub release. Called by release-kit/publish/action.yml; everything comes in
# through the environment (see there), and the GitHub CLI `gh` does the talking.
#
#   TAG=latest   the rolling build: the pre-release `latest` is made if missing; SHA256SUMS goes
#                first (no stale checksums for files that change), then the files, then files of an
#                earlier build that this one no longer has, then the checksums, and LAST the tag
#                moves to SHA and the page names it. A run that stops in between leaves the page
#                naming the previous commit.
#   Both carry `version.json` (made here, see version_json.py) next to the install scripts, and it is
#   listed in SHA256SUMS like every other file.
#   TAG=vX.Y.Z   a release: a draft is made if there is none (a published release is never touched
#                again), the files and the checksums are attached, and it stays a draft for a person
#                to open (NREL-07).

set -euo pipefail

say() { printf '%s\n' "$*"; }
fail() {
    printf '::error::release-kit publish: %s\n' "$*" >&2
    exit 1
}

: "${GITHUB_REPOSITORY:?}"
: "${TAG:?}" "${FILES:?}" "${EXPECT:?}" "${KIT:?}"
SHA="${SHA:-}"
APP="${APP:-}"
EXTRA="${EXTRA:-}"
NOTES_FILE="${NOTES_FILE:-}"
TITLE="${TITLE:-}"
VERSION="${VERSION:-}"
PYTHON="${PYTHON:-python3}"

case "$TAG" in
    latest)
        [[ "$SHA" =~ ^[0-9a-f]{40}$ ]] || fail "the rolling build needs sha, a full commit id (got '$SHA')"
        ;;
    v[0-9]*.[0-9]*.[0-9]*)
        [[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+[A-Za-z0-9.-]*$ ]] || fail "the tag $TAG is not vX.Y.Z"
        ;;
    *) fail "tag must be latest or vX.Y.Z, got '$TAG'" ;;
esac
# The title of a release page (a rolling build names its commit itself).
TITLE="${TITLE:-$TAG}"
[[ "$TITLE" != *$'\n'* ]] || fail "title must be one line"
[[ "$EXPECT" =~ ^[0-9]+$ ]] || fail "expect must be a number, got '$EXPECT'"
[ -d "$FILES" ] || fail "no folder $FILES (download the build artifacts there first)"
[[ "$APP" =~ ^[A-Za-z0-9._-]+$ ]] || fail "app must be a plain word, got '$APP'"

# The built files: exactly as many as expected, plain names, nothing in sub folders.
mapfile -t built < <(find "$FILES" -mindepth 1 -maxdepth 1 -type f -printf '%f\n' | LC_ALL=C sort)
if [ "${#built[@]}" -ne "$EXPECT" ]; then
    fail "${#built[@]} file(s) in $FILES instead of $EXPECT: a system is missing (${built[*]:-none})"
fi
if [ -n "$(find "$FILES" -mindepth 1 -maxdepth 1 ! -type f -print -quit)" ]; then
    fail "$FILES holds a folder: download the artifacts with merge-multiple: true"
fi

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT

add() {
    local from="$1" name
    name="$(basename "$from")"
    [[ "$name" =~ ^[A-Za-z0-9._-]+$ ]] || fail "the file name $name is not a plain word"
    [ "$name" != "SHA256SUMS" ] || fail "SHA256SUMS is made here, not given"
    [ ! -e "$stage/$name" ] || fail "two files are called $name"
    cp "$from" "$stage/$name"
}

for name in "${built[@]}"; do
    add "$FILES/$name"
done
while IFS= read -r extra; do
    [ -n "$extra" ] || continue
    [ -f "$extra" ] || fail "no such extra file: $extra"
    add "$extra"
done <<< "$EXTRA"

# The install scripts of the kit, stamped for this app and this channel.
stamp() {
    local source="$1" out="$2" kind="$3" program="$4" release_asset="$5" rolling_asset="$6"
    [ ! -e "$stage/$out" ] || fail "two files are called $out (an extra file and the stamped script)"
    "$PYTHON" "$KIT/stamp.py" "$KIT/scripts/$source" "$stage/$out" \
        --repo "$GITHUB_REPOSITORY" --channel "$TAG" --app "$APP" --kind "$kind" --program "$program" \
        --release-asset "$release_asset" --rolling-asset "$rolling_asset" ||
        fail "cannot stamp $source"
}
if [ -n "${WINDOWS_KIND:-}" ]; then
    stamp install.ps1 install.ps1 "$WINDOWS_KIND" "${WINDOWS_PROGRAM:-}" \
        "${WINDOWS_RELEASE_ASSET:-}" "${WINDOWS_ROLLING_ASSET:-}"
fi
if [ -n "${UNIX_KIND:-}" ]; then
    stamp install.sh install.sh "$UNIX_KIND" "${UNIX_PROGRAM:-}" \
        "${UNIX_RELEASE_ASSET:-}" "${UNIX_ROLLING_ASSET:-}"
fi

# version.json: what a hub reads to tell that this build is newer than the installed one. A built or
# extra file of that name would be shadowed by it, so the name is the kit's.
[ ! -e "$stage/version.json" ] || fail "two files are called version.json (it is made here, not given)"
"$PYTHON" "$KIT/version_json.py" "$stage/version.json" --app "$APP" --channel "$TAG" \
    --version "$VERSION" --commit "$SHA" ||
    fail "cannot write version.json"

mapfile -t names < <(find "$stage" -mindepth 1 -maxdepth 1 -type f -printf '%f\n' | LC_ALL=C sort)
# "<hash>  <name>" everywhere (sha256sum of Git Bash marks binary mode with "<hash> *<name>").
(cd "$stage" && sha256sum "${names[@]}") | sed 's/^\([0-9a-f]\{64\}\) \*/\1  /' > "$stage.sums"
say "files of $TAG:"
cat "$stage.sums"

# The words of the release page; {tag} in the notes file is the tag (the command to install this release).
notes() {
    if [ -n "$NOTES_FILE" ]; then
        [ -f "$NOTES_FILE" ] || fail "no such notes file: $NOTES_FILE"
        sed "s/{tag}/$TAG/g" "$NOTES_FILE"
    fi
}

if [ "$TAG" = "latest" ]; then
    if ! gh release view latest --repo "$GITHUB_REPOSITORY" >/dev/null 2>&1; then
        gh release create latest --repo "$GITHUB_REPOSITORY" --prerelease --latest=false --target "$SHA" \
            --title "Rolling build (starting)" --notes "The first rolling build is being made."
    fi
    # No checksums for files that are about to change.
    gh release delete-asset latest SHA256SUMS --repo "$GITHUB_REPOSITORY" --yes 2>/dev/null || true
    (cd "$stage" && gh release upload latest "${names[@]}" --repo "$GITHUB_REPOSITORY" --clobber)
    # A file of an earlier build that this one no longer has (a renamed asset) would be offered
    # without a checksum: it goes. (The list first, so a failing `gh` stops here.)
    present="$(gh release view latest --repo "$GITHUB_REPOSITORY" --json assets --jq '.assets[].name')"
    while IFS= read -r old; do
        [ -n "$old" ] || continue
        [ "$old" = "SHA256SUMS" ] && continue
        if ! printf '%s\n' "${names[@]}" | grep -Fxq -- "$old"; then
            say "removing $old (not part of this build)"
            gh release delete-asset latest "$old" --repo "$GITHUB_REPOSITORY" --yes
        fi
    done <<< "$present"
    cp "$stage.sums" "$stage/SHA256SUMS"
    (cd "$stage" && gh release upload latest SHA256SUMS --repo "$GITHUB_REPOSITORY" --clobber)

    # The words of the page, last: only now does it name this commit. The downloads go by the name
    # of the release, not by the commit of its tag; when the tag cannot move (a token without the
    # right to point a ref past a change of .github/workflows), the page still names the commit.
    if ! gh api -X PATCH "repos/$GITHUB_REPOSITORY/git/refs/tags/latest" -f sha="$SHA" -F force=true >/dev/null; then
        printf '::warning::release-kit publish: the tag latest could not be moved to %s; the files and the page are of this commit\n' "$SHA" >&2
    fi
    {
        notes
        printf '\nBuilt from commit %s%s%s.\n' '`' "$SHA" '`'
    } > "$stage.notes"
    gh release edit latest --repo "$GITHUB_REPOSITORY" --prerelease --latest=false \
        --title "Rolling build (${SHA:0:7})" --notes-file "$stage.notes"
else
    if draft="$(gh release view "$TAG" --repo "$GITHUB_REPOSITORY" --json isDraft --jq .isDraft 2>/dev/null)"; then
        [ "$draft" = "true" ] || fail "the release $TAG is published: it is never cut again (NREL-10), make a patch release"
        # A draft of an earlier cut (the tag was set again) gets the words of this one.
        if [ -n "$NOTES_FILE" ]; then
            notes > "$stage.notes"
            gh release edit "$TAG" --repo "$GITHUB_REPOSITORY" --draft --title "$TITLE" --notes-file "$stage.notes"
        else
            gh release edit "$TAG" --repo "$GITHUB_REPOSITORY" --draft --title "$TITLE"
        fi
    else
        notes > "$stage.notes"
        gh release create "$TAG" --repo "$GITHUB_REPOSITORY" --draft --verify-tag --title "$TITLE" \
            --notes-file "$stage.notes"
    fi
    cp "$stage.sums" "$stage/SHA256SUMS"
    (cd "$stage" && gh release upload "$TAG" "${names[@]}" SHA256SUMS --repo "$GITHUB_REPOSITORY" --clobber)
fi
rm -f "$stage.sums" "$stage.notes"
say "published ${#names[@]} file(s) and SHA256SUMS on $TAG"
