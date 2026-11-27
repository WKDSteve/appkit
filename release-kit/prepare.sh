#!/usr/bin/env bash
# release-kit/prepare.sh: the work of the action release-kit/prepare (see prepare/action.yml).
#
# Which commit gets the rolling build `latest`? The newest commit of the branch whose CI run is green, not the tip: see
# pick_target.py. Then: does the pre-release `latest` exist?
#
# It is in a script of its own because the action runs it twice over: with `bash` on Linux runners and with Git
# Bash by its full path on Windows runners (on a self-hosted Windows runner `shell: bash` is the WSL launcher).
#
# Environment: GH_TOKEN, CHANNEL (latest, nightly or stable; default latest), CHANNELS (the channels the app has switched on; default latest), BRANCH, GITHUB_REPOSITORY, GITHUB_OUTPUT, KIT (this folder); SHA (a full commit id built
# by hand, empty for the newest green one), CI_WORKFLOW (default ci.yml), PYTHON (default python3),
# WAIT_FOR_DESCENDANTS (true or false, default true: latest builds nothing while a descendant of its commit is in CI),
# FINISHED_SHA, FINISHED_CONCLUSION, FINISHED_WORKFLOW, FINISHED_EVENT, FINISHED_BRANCH (the CI run that started
# this run, github.event.workflow_run; empty when started by hand).
set -eu

: "${GH_TOKEN:?}" "${BRANCH:?}" "${GITHUB_REPOSITORY:?}" "${GITHUB_OUTPUT:?}" "${KIT:?}"
SHA="${SHA:-}"
PYTHON="${PYTHON:-python3}"
CHANNEL="${CHANNEL:-latest}"
CHANNELS="${CHANNELS:-latest}"
set --
case "${WAIT_FOR_DESCENDANTS:-true}" in
    true) ;;
    false) set -- --build-every-green ;;
    *)
        echo "::error::wait-for-descendants must be true or false, got '${WAIT_FOR_DESCENDANTS}'"
        exit 1
        ;;
esac
# The CI run that started this run counts only when it is a push run of the CI workflow on the branch.
finished=""
workflow="${FINISHED_WORKFLOW:-}"
if [ "${FINISHED_EVENT:-}" = push ] && [ "${FINISHED_BRANCH:-}" = "$BRANCH" ] \
    && [ "${workflow##*/}" = "${CI_WORKFLOW:-ci.yml}" ]; then
    finished="${FINISHED_SHA:-}"
fi

picked="$("$PYTHON" "$KIT/pick_target.py" --repo "$GITHUB_REPOSITORY" --branch "$BRANCH" \
    --ci-workflow "${CI_WORKFLOW:-ci.yml}" --channel "$CHANNEL" --channels "$CHANNELS" --requested "$SHA" \
    --finished "$finished" --finished-conclusion "${FINISHED_CONCLUSION:-}" "$@")"
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
if ! gh release view "$CHANNEL" --repo "$GITHUB_REPOSITORY" >/dev/null 2>&1; then
    gh release create "$CHANNEL" --repo "$GITHUB_REPOSITORY" --prerelease --latest=false --target "$SHA" \
        --title "Rolling build (starting)" --notes "The first build of this channel is being made."
fi
