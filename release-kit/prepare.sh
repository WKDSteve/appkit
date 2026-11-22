#!/usr/bin/env bash
# release-kit/prepare.sh: the work of the action release-kit/prepare (see prepare/action.yml).
#
# Which commit gets the rolling build `latest`? The newest commit of the branch whose CI run is green (rule
# GIT-08), not the tip: see pick_target.py. Then: does the pre-release `latest` exist?
#
# It is in a script of its own because the action runs it twice over: with `bash` on Linux runners and with Git
# Bash by its full path on Windows runners (on a self-hosted Windows runner `shell: bash` is the WSL launcher).
#
# Environment: GH_TOKEN, BRANCH, GITHUB_REPOSITORY, GITHUB_OUTPUT, KIT (this folder); SHA (a full commit id built
# by hand, empty for the newest green one), CI_WORKFLOW (default ci.yml), PYTHON (default python3).
set -eu

: "${GH_TOKEN:?}" "${BRANCH:?}" "${GITHUB_REPOSITORY:?}" "${GITHUB_OUTPUT:?}" "${KIT:?}"
SHA="${SHA:-}"
PYTHON="${PYTHON:-python3}"

picked="$("$PYTHON" "$KIT/pick_target.py" --repo "$GITHUB_REPOSITORY" --branch "$BRANCH"     --ci-workflow "${CI_WORKFLOW:-ci.yml}" --requested "$SHA")"
target="$(printf '%s
' "$picked" | sed -n 's/^target=//p')"
reason="$(printf '%s
' "$picked" | sed -n 's/^reason=//p')"
if [ -z "$target" ]; then
    echo "current=false" >> "$GITHUB_OUTPUT"
    echo "::notice::nothing is built: $reason"
    exit 0
fi
SHA="$target"
short="$(printf '%s' "$SHA" | cut -c1-7)"
echo "::notice::building $short: $reason"
{
    echo "sha=$SHA"
    echo "short=$short"
    echo "current=true"
} >> "$GITHUB_OUTPUT"
if ! gh release view latest --repo "$GITHUB_REPOSITORY" >/dev/null 2>&1; then
    gh release create latest --repo "$GITHUB_REPOSITORY" --prerelease --latest=false --target "$SHA" \
        --title "Rolling build (starting)" --notes "The first rolling build is being made."
fi
