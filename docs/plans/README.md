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

- Start: `scripts/plan.sh new <pr|0> <branch> "<title>"` (use 0 before the PR exists, then
  rename the file and set `pr:` when it opens).
- Log work: `scripts/plan.sh note <pr> <slug>` appends a timestamped entry to your file.
- Read: `plan.sh show <phase>` (items from every PR, tagged `(#PR)`), `plan.sh pr <N>` (one PR:
  items and progress), `plan.sh log [N]` (timeline of all entries, oldest first),
  `plan.sh status` (done/open per phase and per PR, entry counts).
- Backfilled entries from the old PLAN.md log keep their original text; their `### ` heading is
  `<date>T<time>Z-bNNN-<words>` (`NNN` is the old order; time is 000000 when the adding commit
  was on another day).
