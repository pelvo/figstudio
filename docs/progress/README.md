# Progress entries

One file per entry, named `<UTC time>-<slug>.md` (`scripts/progress.sh new <slug>`).
Two branches never touch the same file, so the log cannot conflict. File names sort into
the timeline; `scripts/progress.sh list` prints it with the commit that added each entry.
Never edit another branch's entry; correct an old one with a new entry that names it.
Entries `…-bNNN-…` are the old `docs/PLAN.md` log, backfilled verbatim: the date is the one
in the entry (time of day from the commit that added it when that was the same day, else
000000) and `NNN` keeps their original order in that file. Entries added afterwards have no `bNNN`.
