#!/usr/bin/env bash
# Progress entries: one file per entry in docs/progress/, so branches never edit the same
# file and never conflict. Order comes from the file name (UTC timestamp), the commit that
# added each entry is the audit trail.
#
#   scripts/progress.sh new <slug>   create docs/progress/<utc-time>-<slug>.md, print its path
#   scripts/progress.sh list [N]     timeline, oldest first (last N when given) + adding commit
set -euo pipefail
cd "$(dirname "$0")/../docs/progress"

case "${1:-}" in
  new)
    slug="${2:-}"
    [[ "$slug" =~ ^[a-z0-9]+(-[a-z0-9]+)*$ ]] || { echo "usage: $0 new <kebab-case-slug>" >&2; exit 2; }
    f="$(date -u +%Y-%m-%dT%H%M%SZ)-$slug.md"
    [ ! -e "$f" ] || { echo "$f exists" >&2; exit 1; }
    printf -- '- PR/branch: \n- What changed and how it was checked:\n- Not verified:\n' > "$f"
    echo "docs/progress/$f"
    ;;
  list)
    for f in $(ls [0-9]*.md 2>/dev/null | sort | { if [ -n "${2:-}" ]; then tail -n "$2"; else cat; fi; }); do
      c=$(git log -1 --format=%h --diff-filter=A -- "$f" 2>/dev/null || true)
      printf '%s  %-8s  %s\n' "${f%.md}" "${c:--}" "$(grep -m1 -v '^[[:space:]]*$' "$f" | cut -c1-100)"
    done
    ;;
  *) echo "usage: $0 new <slug> | list [N]" >&2; exit 2 ;;
esac
