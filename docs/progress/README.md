# Progress entries

One file per entry, named `<UTC time>-<slug>.md` (`scripts/progress.sh new <slug>`).
Two branches never touch the same file, so the log cannot conflict. File names sort into
the timeline; `scripts/progress.sh list` prints it with the commit that added each entry.
Never edit another branch's entry; correct an old one with a new entry that names it.
Entries before 2026-10-27 are in the frozen "Progress log" at the bottom of `docs/PLAN.md`.
