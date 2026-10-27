# Plan state, one file per branch

`docs/PLAN.md` keeps the requirements, architecture, phases, prose and "Done when" checks.
Everything that changes per branch lives here as `<YYYY-MM-DD>-<slug>.md`: the UTC date the
work started and a short kebab-case name. The name is fixed when the file is created, so it
never needs renaming and nothing depends on a PR number. `2026-10-08-before-prs.md` holds the
items that came in before PRs were used. A file has:

- a header: `branch:`, `pr:` (optional, filled in once the PR exists), `title:`
- `## <phase heading>` sections with the `- [ ]` / `- [x]` checklist items the branch added
- a `## Progress` section of `### <UTC time> <slug>` entries: what changed, how it was
  checked, what was not verified

Two branches never edit the same file, so nothing conflicts. Edit only your own branch's file.
Ticking an item another branch added is a one-line edit in that file; correct an old progress
entry with a new one that names it.

- **Start** (before any PR): on your branch run `scripts/plan.sh new <slug> "<title>"`. The tool
  finds your file afterwards by the `branch:` header, so `note`, `link` and `pr` need no name
  or number. Commit the file with your work.
- **Log work**: `plan.sh note <slug>` appends a timestamped entry.
- **PR opened** (optional): `plan.sh link` writes the PR number into `pr:` (from `gh`, or
  `link <N>`). Until then items are tagged with the slug instead of `#N`. Forgetting it loses
  nothing but the tag.
- **Read**: `plan.sh show <phase>` (items from every file, tagged), `plan.sh pr [N|slug]` (one
  file whole; default this branch's), `plan.sh log [N]` (timeline of all entries, oldest
  first), `plan.sh status` (done/open per phase and per file, entry counts).
- Backfilled entries from the old PLAN.md log keep their original text; their `### ` heading is
  `<date>T<time>Z-bNNN-<words>` (`NNN` is the old order; time is 000000 when the adding commit
  was on another day). The date and slug of a backfilled file come from the PR it belonged to.
