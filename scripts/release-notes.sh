#!/usr/bin/env bash
# Prints a release's body: what changed, then the install instructions.
#
# What changed is the "## Release notes" section of the pull request merged in
# <sha>, written for players. A PR without one gets the commit subjects since
# the previous release instead. The install instructions are for the GitHub
# release page and come after an <!-- install --> marker (invisible there);
# the app's update screen shows only what's above it
# (app/src/settings/releaseNotes.ts).
#
#   scripts/release-notes.sh <version> [<sha>]
#
# Needs the full git history with tags, and gh (GH_TOKEN in CI).
set -euo pipefail

version=$1
sha=$(git rev-parse "${2:-HEAD}")
repo=${GITHUB_REPOSITORY:-$(gh repo view --json nameWithOwner --jq .nameWithOwner)}

notes=""
pr=$(gh api "repos/$repo/commits/$sha/pulls" --jq '.[0].number // empty' 2>/dev/null || true)
if [ -n "$pr" ]; then
  notes=$(gh pr view "$pr" --repo "$repo" --json body --jq .body | tr -d '\r' \
    | awk '/^##[[:space:]]+Release notes[[:space:]]*$/ { keep = 1; next } /^##[[:space:]]/ { keep = 0 } keep' \
    | sed -e '/./,$!d')
fi
# No words at all (a missing section, or the template's empty "- "): fall back.
if ! printf '%s' "$notes" | grep -q '[[:alnum:]]'; then
  previous=$(git describe --tags --abbrev=0 "$sha^" 2>/dev/null || true)
  notes=$(git log --no-merges --format='- %s' ${previous:+$previous..}"$sha" | head -n 20)
fi

cat <<EOF
## What's new

$notes

<!-- install -->
**Install:** download \`EVE.Chatterer_${version}_x64-setup.exe\` below and run it. It installs for your Windows user only (no admin needed), starts with Windows, and updates itself from then on.

**Portable:** or unzip \`eve-chatterer_${version}_x64-portable.zip\` anywhere and run it. The portable copy doesn't update itself.
EOF
