#!/usr/bin/env bash
# Progress entries: one file per entry in docs/progress/, so branches never edit the same
# file and never conflict. Order comes from the file name (UTC timestamp). Each entry is
# tied to a PR: by its `branch:`/`pr:` header, else by the "(#N)" the squash-merge commit
# that added it carries, else by the first "PR #N" in its text.
#
#   scripts/progress.sh new <slug> [pr]   create docs/progress/<utc-time>-<slug>.md, print its path
#   scripts/progress.sh list [N]          timeline, oldest first (last N), with PR and adding commit
#   scripts/progress.sh pr <number>       the entries of one PR, oldest first, in full
set -euo pipefail
cd "$(dirname "$0")/../docs/progress"

pr_of() { # file -> PR number or "-"
  local f=$1 n
  n=$(sed -n '1,/^---$/{s/^pr:[[:space:]]*#\{0,1\}\([0-9]\{1,\}\).*/\1/p}' "$f" | head -1)
  [ -n "$n" ] || n=$(git log --diff-filter=A --format=%s -- "$f" 2>/dev/null | head -1 | sed -n 's/.*(#\([0-9]\{1,\}\))$/\1/p')
  [ -n "$n" ] || n=$(grep -o -m1 'PR #[0-9]\{1,\}' "$f" | head -1 | tr -dc 0-9)
  echo "${n:--}"
}

case "${1:-}" in
  new)
    slug="${2:-}"
    [[ "$slug" =~ ^[a-z0-9]+(-[a-z0-9]+)*$ ]] || { echo "usage: $0 new <kebab-case-slug> [pr-number]" >&2; exit 2; }
    f="$(date -u +%Y-%m-%dT%H%M%SZ)-$slug.md"
    [ ! -e "$f" ] || { echo "$f exists" >&2; exit 1; }
    br=$(git rev-parse --abbrev-ref HEAD)
    printf -- '---\nbranch: %s\npr: %s\n---\n- What changed and how it was checked:\n- Not verified:\n' "$br" "${3:-}" > "$f"
    echo "docs/progress/$f"
    ;;
  list)
    for f in $(ls [0-9]*.md 2>/dev/null | sort | { if [ -n "${2:-}" ]; then tail -n "$2"; else cat; fi; }); do
      c=$(git log -1 --format=%h --diff-filter=A -- "$f" 2>/dev/null || true)
      printf '%s  PR %-5s %-8s %s\n' "${f%.md}" "$(pr_of "$f")" "${c:--}" \
        "$(grep -m1 -v '^\(---\|branch:\|pr:\)\|^[[:space:]]*$' "$f" | cut -c1-100)"
    done
    ;;
  pr)
    n="${2:-}"; [[ "$n" =~ ^[0-9]+$ ]] || { echo "usage: $0 pr <number>" >&2; exit 2; }
    for f in $(ls [0-9]*.md 2>/dev/null | sort); do
      [ "$(pr_of "$f")" = "$n" ] && { printf '== %s\n' "${f%.md}"; cat "$f"; }
    done
    ;;
  *) echo "usage: $0 new <slug> [pr] | list [N] | pr <number>" >&2; exit 2 ;;
esac
