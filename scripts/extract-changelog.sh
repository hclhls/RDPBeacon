#!/usr/bin/env bash
# Print the CHANGELOG.md section for one version (for example "0.4").
# Usage: scripts/extract-changelog.sh <version> [changelog-file]
set -euo pipefail

version="${1:?usage: extract-changelog.sh <version> [changelog-file]}"
file="${2:-CHANGELOG.md}"

notes="$(awk -v v="$version" '
    /^## \[/ {
        if (found) exit
        if ($0 ~ "^## \\[" v "\\]") { found = 1; next }
    }
    found && /^\[[^]]+\]: / { exit }
    found { print }
' "$file")"

if [ -z "$(printf '%s' "$notes" | tr -d '[:space:]')" ]; then
    echo "No changelog section found for version $version in $file" >&2
    exit 1
fi

printf '%s\n' "$notes"
