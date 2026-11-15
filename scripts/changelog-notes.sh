#!/usr/bin/env bash
# Print one version's section of CHANGELOG.md (the lines under "## <version> ...", up to the
# next "## "), without leading or trailing blank lines. Exits 1 when there is no such section.
#   bash scripts/changelog-notes.sh 0.4.1 [CHANGELOG.md]
set -euo pipefail
version="${1:?usage: changelog-notes.sh <version> [file]}"
file="${2:-CHANGELOG.md}"
awk -v v="$version" '
  /^## / { if (found) exit; if ($2 == v) found = 1; next }
  found { lines[++n] = $0; if (NF) { if (!first) first = n; last = n } }
  END {
    if (!last) exit 1
    for (i = first; i <= last; i++) print lines[i]
  }
' "$file"
