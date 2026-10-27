#!/usr/bin/env bash
# Plan state lives one file per branch in docs/plans/<YYYY-MM-DD>-<slug>.md, so two branches
# never edit the same file and no rename is ever needed. A file has a header (branch, pr,
# title), "## <phase>" checklist sections and a "## Progress" section of
# "### <utc time> <slug>" entries. docs/PLAN.md keeps the phases, prose and Done-when checks.
# The file is found by its `branch:` header, so no PR number is needed; `pr:` is optional.
#
#   scripts/plan.sh new <slug> "<title>"   start this branch's file (date = today, UTC)
#   scripts/plan.sh link [pr]              record the PR number in the header (from gh when omitted)
#   scripts/plan.sh note <slug>            add a timestamped progress entry to this branch's file
#   scripts/plan.sh show <phase>           one phase's items from every file, tagged (#PR) or (slug)
#   scripts/plan.sh pr [pr|slug]           one file whole (default: this branch's)
#   scripts/plan.sh log [N]                timeline of all progress entries, oldest first (last N)
#   scripts/plan.sh status                 done/open per phase and per file
set -euo pipefail
cd "$(dirname "$0")/../docs/plans"

branch=$(git rev-parse --abbrev-ref HEAD)
files() { ls [0-9]*.md | sort; }
hdr() { sed -n "s/^$2: *//p" "$1" | head -1; }
tag_of() { local pr; pr=$(hdr "$1" pr); [ -n "$pr" ] && echo "#$pr" || { local s=${1%.md}; echo "${s#????-??-??-}"; }; }
mine() { local f; f=$(grep -lx "branch: $branch" [0-9]*.md 2>/dev/null | head -1 || true); [ -n "$f" ] && echo "$f" || return 1; }
find_file() { # arg: empty = this branch; number = PR; else slug or file name
  local a="${1:-}" f
  if [ -z "$a" ]; then mine || { echo "no plan file for branch $branch; run: plan.sh new <slug> \"<title>\"" >&2; return 1; }
  elif [[ "$a" =~ ^[0-9]+$ ]]; then f=$(grep -lx "pr: $a" [0-9]*.md | head -1 || true); [ -n "$f" ] && echo "$f" || { echo "no file with pr: $a" >&2; return 1; }
  else f=$(ls [0-9]*"${a%.md}".md 2>/dev/null | head -1 || true); [ -n "$f" ] && echo "$f" || { echo "no file for $a" >&2; return 1; }; fi
}

case "${1:-}" in
  new)
    s="${2:-}"; t="${3:-}"
    [[ "$s" =~ ^[a-z0-9]+(-[a-z0-9]+)*$ && -n "$t" ]] || { echo 'usage: plan.sh new <kebab-slug> "<title>"' >&2; exit 2; }
    ! mine >/dev/null 2>&1 || { echo "branch $branch already has $(mine)" >&2; exit 1; }
    f="$(date -u +%Y-%m-%d)-$s.md"; [ ! -e "$f" ] || { echo "$f exists" >&2; exit 1; }
    printf -- '---\nbranch: %s\npr:\ntitle: %s\n---\n\n## P? — <phase heading exactly as in PLAN.md>\n- [ ] item\n\n## Progress\n' "$branch" "$t" > "$f"
    echo "docs/plans/$f" ;;
  link)
    f=$(find_file); pr="${2:-$(gh pr view --json number -q .number 2>/dev/null || true)}"
    [[ "$pr" =~ ^[0-9]+$ ]] || { echo "no PR found for $branch yet; open it, or pass the number" >&2; exit 1; }
    sed -i "s/^pr:.*/pr: $pr/" "$f"; echo "docs/plans/$f -> #$pr" ;;
  note)
    [[ "${2:-}" =~ ^[a-z0-9]+(-[a-z0-9]+)*$ ]] || { echo 'usage: plan.sh note <kebab-slug>' >&2; exit 2; }
    f=$(find_file)
    grep -q '^## Progress$' "$f" || printf '\n## Progress\n' >> "$f"
    printf '\n### %s %s\n- What changed and how it was checked:\n- Not verified:\n' "$(date -u +%Y-%m-%dT%H%M%SZ)" "$2" >> "$f"
    echo "docs/plans/$f" ;;
  show)
    ph="${2:?phase}"
    for f in $(files); do
      awk -v ph="$ph" -v tag="$(tag_of "$f")" '
        /^## /{on=($2==ph); if(blk)print blk; blk=""; next}
        on&&/^- \[/{if(blk)print blk; blk=$0 "  (" tag ")"; next}
        on&&/^ /{blk=blk "\n" $0; next}
        END{if(blk)print blk}' "$f"
    done ;;
  pr) f=$(find_file "${2:-}") || exit 1; cat "$f" ;;
  log)
    for f in $(files); do
      sed -n '/^## Progress$/,$p' "$f" | awk -v t="$(tag_of "$f")" '/^### /{printf "%s  %-24s %s\n",$2,t,substr($0,5+length($2)+1)}'
    done | sort | { if [ -n "${2:-}" ]; then tail -n "$2"; else cat; fi; } ;;
  status)
    printf '%-10s %5s %5s\n' phase done open
    cat $(files) | awk '/^## /{p=$2} /^- \[x\]/{d[p]++} /^- \[ \]/{o[p]++} END{for(k in d)ks[k];for(k in o)ks[k];for(k in ks)printf "%-10s %5d %5d\n",k,d[k],o[k]}' | sort -V
    printf '\n%-44s %-6s %5s %5s %8s\n' file pr done open entries
    for f in $(files); do
      printf '%-44s %-6s %5d %5d %8d\n' "${f%.md}" "$(hdr "$f" pr)" "$(grep -c '^- \[x\]' "$f")" "$(grep -c '^- \[ \]' "$f")" "$(grep -c '^### ' "$f")"; done ;;
  *) echo "usage: plan.sh new|link|note|show|pr|log|status ..." >&2; exit 2 ;;
esac
