---
branch: docs/progress-entries
pr: 64
---
- Branch docs/progress-entries: the progress log moved from one appended section of `docs/PLAN.md` to one file per entry in `docs/progress/` (`scripts/progress.sh new|list`), so branches stop conflicting on the last lines of PLAN.md. The 330 entries on `main` were backfilled verbatim as `…-bNNN-…` files, and the section in PLAN.md is now a pointer.
- Checked: `new` creates the file, rejects bad slugs and duplicates; `list` prints the timeline; two branches each adding an entry merge without conflict; the backfilled files hold exactly the old entry text, in order.
- Not verified: CI (no code changed); open branches that still append to PLAN.md conflict once on rebase and should move their line into a new entry file.
- Per-PR tracking of the log: entries carry `branch:`/`pr:`; `scripts/progress.sh pr <N>` prints one PR's entries and `list` shows a PR column.
- Plan checklists: the 94 `- [ ]`/`- [x]` items of PLAN.md moved, unchanged, into `docs/plans/pr-<N>.md` by the PR whose squash commit added them (`pre-pr.md` for the 47 older ones); PLAN.md keeps phases, prose and Done-when. `scripts/plan.sh show <phase>|pr <N>|status` rebuilds a phase with `(#PR)` tags and counts done/open per phase and per PR. Item text is identical (checked: sorted lists equal, 94/94).
