#!/usr/bin/env bash
# Plan state lives one file per PR in docs/plans/, so two PRs never edit the same file.
# Each file has a header (pr, branch, title), "## <phase>" checklist sections, and a
# "## Progress" section of "### <utc time> <slug>" entries. docs/PLAN.md keeps the phases,
# prose and Done-when checks.
#
#   scripts/plan.sh new <pr|0> <branch> "<title>"   create docs/plans/pr-<pr>.md
#   scripts/plan.sh note <pr> <slug>                add a timestamped progress entry to that PR's file
#   scripts/plan.sh show <phase>                    one phase's items from every PR, tagged (#PR)
#   scripts/plan.sh pr <pr>                         one PR's whole file: items and progress
#   scripts/plan.sh log [N]                         timeline of all progress entries, oldest first (last N)
#   scripts/plan.sh status                          done/open per phase and per PR
set -euo pipefail
cd "$(dirname "$0")/../docs/plans"

files() { echo pre-pr.md; ls pr-*.md | sort -V; }
tag_of() { local pr; pr=$(sed -n 's/^pr: *//p' "$1" | head -1); [ -n "$pr" ] && echo "#$pr" || { [ "$1" = pre-pr.md ] && echo pre-PR || echo wip; }; }

case "${1:-}" in
  new)
    [[ "${2:-}" =~ ^[0-9]+$ && -n "${3:-}" && -n "${4:-}" ]] || { echo 'usage: plan.sh new <pr|0> <branch> "<title>"' >&2; exit 2; }
    f="pr-$2.md"; [ "$2" != 0 ] || f="pr-wip-${3//\//-}.md"
    [ ! -e "$f" ] || { echo "$f exists" >&2; exit 1; }
    printf -- '---\npr: %s\nbranch: %s\ntitle: %s\n---\n\n## P? — <phase heading exactly as in PLAN.md>\n- [ ] item\n\n## Progress\n' \
      "$([ "$2" = 0 ] || echo "$2")" "$3" "$4" > "$f"
    echo "docs/plans/$f" ;;
  note)
    [[ "${2:-}" =~ ^[0-9]+$ && "${3:-}" =~ ^[a-z0-9]+(-[a-z0-9]+)*$ ]] || { echo 'usage: plan.sh note <pr> <kebab-slug>' >&2; exit 2; }
    f="pr-$2.md"; [ -e "$f" ] || { echo "no $f; run: plan.sh new $2 <branch> \"<title>\"" >&2; exit 1; }
    grep -q '^## Progress$' "$f" || printf '\n## Progress\n' >> "$f"
    printf '\n### %s %s\n- What changed and how it was checked:\n- Not verified:\n' "$(date -u +%Y-%m-%dT%H%M%SZ)" "$3" >> "$f"
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
  pr)
    f="pr-${2:?pr}.md"; [ -e "$f" ] || { echo "no $f" >&2; exit 1; }; cat "$f" ;;
  log)
    for f in $(files); do
      t=$(tag_of "$f")
      sed -n '/^## Progress$/,$p' "$f" | awk -v t="$t" '/^### /{printf "%s  %-7s %s\n",$2,t,substr($0,5+length($2)+1)}'
    done | sort | { if [ -n "${2:-}" ]; then tail -n "$2"; else cat; fi; } ;;
  status)
    printf '%-10s %5s %5s\n' phase done open
    cat $(files) | awk '/^## /{p=$2} /^- \[x\]/{d[p]++} /^- \[ \]/{o[p]++} END{for(k in d)ks[k];for(k in o)ks[k];for(k in ks)printf "%-10s %5d %5d\n",k,d[k],o[k]}' | sort -V
    printf '\n%-10s %5s %5s %8s\n' pr done open entries
    for f in $(files); do
      printf '%-10s %5d %5d %8d\n' "${f%.md}" "$(grep -c '^- \[x\]' "$f")" "$(grep -c '^- \[ \]' "$f")" "$(grep -c '^### ' "$f")"; done ;;
  *) echo "usage: plan.sh new|note|show|pr|log|status ..." >&2; exit 2 ;;
esac
