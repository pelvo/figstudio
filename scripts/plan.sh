#!/usr/bin/env bash
# Plan checklists live one file per PR in docs/plans/, so two PRs never edit the same file.
# docs/PLAN.md keeps the phases, their prose and "Done when" checks.
#
#   scripts/plan.sh new <pr> <branch> "<title>"   create docs/plans/pr-<pr>.md (use 0 for no PR yet)
#   scripts/plan.sh show <phase>                  all items of one phase (P0, P1b, P7, Deferred), tagged with their PR
#   scripts/plan.sh pr <pr>                       one PR's items, by phase
#   scripts/plan.sh status                        done/open count per phase and per PR
set -euo pipefail
cd "$(dirname "$0")/../docs/plans"

case "${1:-}" in
  new)
    [[ "${2:-}" =~ ^[0-9]+$ && -n "${3:-}" && -n "${4:-}" ]] || { echo 'usage: plan.sh new <pr|0> <branch> "<title>"' >&2; exit 2; }
    f="pr-$2.md"; [ "$2" != 0 ] || f="pr-wip-${3//\//-}.md"
    [ ! -e "$f" ] || { echo "$f exists" >&2; exit 1; }
    printf -- '---\npr: %s\nbranch: %s\ntitle: %s\n---\n\n## P? — <phase heading exactly as in PLAN.md>\n- [ ] item\n' "$([ "$2" = 0 ] || echo "$2")" "$3" "$4" > "$f"
    echo "docs/plans/$f" ;;
  show)
    ph="${2:?phase}"
    for f in pre-pr.md $(ls pr-*.md | sort -V); do
      pr=$(sed -n 's/^pr: *//p' "$f" | head -1); tag="${pr:+#$pr}"; tag="${tag:-$([ "$f" = pre-pr.md ] && echo pre-PR || echo wip)}"
      awk -v ph="$ph" -v tag="$tag" '
        /^## /{on=($2==ph)} 
        on&&/^- \[/{if(blk)print blk; blk=$0 "  (" tag ")"; next}
        on&&/^ /{blk=blk "\n" $0; next}
        END{if(blk)print blk}' "$f"
    done ;;
  pr)
    f="pr-${2:?pr}.md"; [ -e "$f" ] || { echo "no $f" >&2; exit 1; }; cat "$f" ;;
  status)
    printf '%-10s %5s %5s\n' phase done open
    cat pre-pr.md pr-*.md | awk '/^## /{p=$2} /^- \[x\]/{d[p]++} /^- \[ \]/{o[p]++} END{for(k in d)ks[k];for(k in o)ks[k];for(k in ks)printf "%-10s %5d %5d\n",k,d[k],o[k]}' | sort -V
    printf '\n%-10s %5s %5s\n' pr done open
    for f in pre-pr.md $(ls pr-*.md | sort -V); do
      printf '%-10s %5d %5d\n' "${f%.md}" "$(grep -c '^- \[x\]' "$f")" "$(grep -c '^- \[ \]' "$f")"; done ;;
  *) echo "usage: plan.sh new|show|pr|status ..." >&2; exit 2 ;;
esac
