# Plan checklists, one file per PR

`docs/PLAN.md` keeps the requirements, architecture, phases, prose and "Done when" checks.
Its checklist items live here as `pr-<N>.md` (items added by PR #N; `pre-pr.md` is the
backfill of items that came in before PRs were used). Each file has a `pr:`/`branch:`/`title:`
header and `## <phase heading>` sections with the same `- [ ]` / `- [x]` items.

- A PR adds or ticks its own items in its own file. Ticking an item that another PR added:
  edit that item in its original file in a one-line change (a conflict is only possible when
  two PRs edit the same item).
- Start a file with `scripts/plan.sh new <pr|0> <branch> "<title>"`.
- `scripts/plan.sh show <phase>` rebuilds a phase's checklist with a `(#PR)` tag on each item;
  `scripts/plan.sh pr <N>` prints one PR's items; `scripts/plan.sh status` counts done/open by
  phase and by PR.
- Timeline of what happened and was checked per PR: `scripts/progress.sh pr <N>`.
