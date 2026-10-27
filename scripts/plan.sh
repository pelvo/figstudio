#!/usr/bin/env bash
# Plan state lives one file per PR in docs/plans/, so two PRs never edit the same file.
# Each file has a header (pr, branch, title), "## <phase>" checklist sections, and a
# "## Progress" section of "### <utc time> <slug>" entries. docs/PLAN.md keeps the phases,
# prose and Done-when checks.
#
#   scripts/plan.sh new "<title>" [pr]    start this branch's file (pr-<pr>.md, or pr-wip-<branch>.md before the PR exists)
#   scripts/plan.sh bind [pr]              the PR now exists: rename pr-wip-<branch>.md to pr-<pr>.md, set pr: (pr from gh when omitted)
#   scripts/plan.sh note <slug> [pr]       add a timestamped progress entry to this branch's file (or PR <pr>'s)
#   scripts/plan.sh show <phase>           one phase's items from every PR, tagged (#PR) or (wip)
#   scripts/plan.sh pr [pr]                one file whole: items and progress (default: this branch's)
#   scripts/plan.sh log [N]                timeline of all progress entries, oldest first (last N)
#   scripts/plan.sh status                 done/open per phase and per PR/branch
set -euo pipefail
cd "$(dirname "$0")/../docs/plans"

files() { echo pre-pr.md; ls pr-*.md | sort -V; }
tag_of() { local pr; pr=$(sed -n 's/^pr: *//p' "$1" | head -1); [ -n "$pr" ] && echo "#$pr" || { [ "$1" = pre-pr.md ] && echo pre-PR || echo wip; }; }

branch=$(git rev-parse --abbrev-ref HEAD)
wipfile() { echo "pr-wip-${branch//\//-}.md"; }
# this branch's file: the one whose branch: header matches, else the wip name
mine() { local f; f=$(grep -l "^branch: $branch\$" pr-*.md 2>/dev/null | head -1 || true); echo "${f:-$(wipfile)}"; }
target() { if [ -n "${1:-}" ]; then echo "pr-$1.md"; else mine; fi; }

case "${1:-}" in
  new)
    t="${2:-}"; pr="${3:-}"; [ -n "$t" ] || { echo 'usage: plan.sh new "<title>" [pr]' >&2; exit 2; }
    [ -z "$pr" ] || [[ "$pr" =~ ^[0-9]+$ ]] || { echo "pr must be a number" >&2; exit 2; }
    f=$([ -n "$pr" ] && echo "pr-$pr.md" || wipfile)
    [ ! -e "$f" ] || { echo "$f exists" >&2; exit 1; }
    printf -- '---\npr: %s\nbranch: %s\ntitle: %s\n---\n\n## P? — <phase heading exactly as in PLAN.md>\n- [ ] item\n\n## Progress\n' "$pr" "$branch" "$t" > "$f"
    echo "docs/plans/$f" ;;
  bind)
    f=$(mine); [ -e "$f" ] || { echo "no plan file for branch $branch; run: plan.sh new \"<title>\"" >&2; exit 1; }
    pr="${2:-$(gh pr view --json number -q .number 2>/dev/null || true)}"
    [[ "$pr" =~ ^[0-9]+$ ]] || { echo "no PR found for $branch yet; open it, or pass the number" >&2; exit 1; }
    [ ! -e "pr-$pr.md" ] || [ "$f" = "pr-$pr.md" ] || { echo "pr-$pr.md exists" >&2; exit 1; }
    sed -i "s/^pr:.*/pr: $pr/" "$f"; [ "$f" = "pr-$pr.md" ] || git mv "$f" "pr-$pr.md" 2>/dev/null || mv "$f" "pr-$pr.md"
    echo "docs/plans/pr-$pr.md" ;;
  note)
    [[ "${2:-}" =~ ^[a-z0-9]+(-[a-z0-9]+)*$ && ( -z "${3:-}" || "${3:-}" =~ ^[0-9]+$ ) ]] || { echo 'usage: plan.sh note <kebab-slug> [pr]' >&2; exit 2; }
    f=$(target "${3:-}"); [ -e "$f" ] || { echo "no $f; run: plan.sh new \"<title>\"" >&2; exit 1; }
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
  pr)
    f=$(target "${2:-}"); [ -e "$f" ] || { echo "no $f" >&2; exit 1; }; cat "$f" ;;
  log)
    for f in $(files); do
      t=$(tag_of "$f")
      sed -n '/^## Progress$/,$p' "$f" | awk -v t="$t" '/^### /{printf "%s  %-7s %s\n",$2,t,substr($0,5+length($2)+1)}'
    done | sort | { if [ -n "${2:-}" ]; then tail -n "$2"; else cat; fi; } ;;
  status)
    printf '%-10s %5s %5s\n' phase done open
    cat $(files) | awk '/^## /{p=$2} /^- \[x\]/{d[p]++} /^- \[ \]/{o[p]++} END{for(k in d)ks[k];for(k in o)ks[k];for(k in ks)printf "%-10s %5d %5d\n",k,d[k],o[k]}' | sort -V
    printf '\n%-24s %5s %5s %8s\n' file done open entries
    for f in $(files); do
      printf '%-24s %5d %5d %8d\n' "${f%.md}" "$(grep -c '^- \[x\]' "$f")" "$(grep -c '^- \[ \]' "$f")" "$(grep -c '^### ' "$f")"; done ;;
  *) echo "usage: plan.sh new|note|show|pr|log|status ..." >&2; exit 2 ;;
esac
