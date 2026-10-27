# Plan state, one file per PR

`docs/PLAN.md` keeps the requirements, architecture, phases, prose and "Done when" checks.
Everything that changes per PR lives here as `pr-<N>.md` (`pre-pr.md` holds what came in
before PRs were used). A file has:

- a header: `pr:`, `branch:`, `title:`
- `## <phase heading>` sections with the `- [ ]` / `- [x]` checklist items the PR added
- a `## Progress` section of `### <UTC time> <slug>` entries: what changed, how it was
  checked, what was not verified

Two PRs never edit the same file, so nothing conflicts. Edit only your own PR's file. Ticking
an item another PR added is a one-line edit in that PR's file; correct an old progress entry
with a new one that names it.

- **No PR yet** (the normal start): on your branch run `scripts/plan.sh new "<title>"`. It makes
  `pr-wip-<branch>.md` (`pr:` empty). `plan.sh note <slug>` and `plan.sh pr` find the file by
  your current branch, so you never need a number. Commit it with your work.
- **PR opened**: run `scripts/plan.sh bind` (reads the number from `gh`, or `bind <N>`). It sets
  `pr:` and renames the file to `pr-<N>.md`; commit that. Do it before merge, because the
  backfill and `show` tag items `(#N)` from the header, and `(wip)` until bound.
- Already know the number: `plan.sh new "<title>" <N>`.
- Log work: `plan.sh note <slug> [N]` appends a timestamped entry (default: this branch's file).
- Read: `plan.sh show <phase>` (items from every PR, tagged `(#PR)` or `(wip)`), `plan.sh pr [N]`
  (one file: items and progress), `plan.sh log [N]` (timeline of all entries, oldest first),
  `plan.sh status` (done/open per phase and per file, entry counts).
- Backfilled entries from the old PLAN.md log keep their original text; their `### ` heading is
  `<date>T<time>Z-bNNN-<words>` (`NNN` is the old order; time is 000000 when the adding commit
  was on another day).
