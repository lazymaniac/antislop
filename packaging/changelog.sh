#!/bin/sh
# Prints the feature and fix commits since the last tag (or all of them) as
# a draft CHANGELOG section; edit it into user-facing lines before release.
set -eu
since=$(git describe --tags --abbrev=0 2>/dev/null || true)
range=${since:+$since..}HEAD
echo "## Unreleased"
echo
git log --no-merges --format=%s "$range" |
    grep -E '^(feat|fix)(\([^)]*\))?!?:' |
    sed -E 's/^[a-z]+(\([^)]*\))?!?: */- /'
