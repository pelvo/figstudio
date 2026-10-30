---
pr: 64
branch: docs/progress-entries
title: docs: conflict-free plan and progress tracking, per PR
---

## Progress

### 2026-10-27T030900Z conflict-free-plan-tracking
- The plan file was edited by every PR (checkboxes, plus a progress log appended at the bottom), so every branch conflicted on it. Both now live in one file per PR, `docs/plans/<date>-<slug>.md`: the 94 checkbox items moved unchanged to the PR whose squash commit added them (`pre-pr.md` for the 47 older ones), and the 330 progress entries moved verbatim under `## Progress` in the file of the PR they belong to (matched by the "PR #N" in the text, else by the squash commit). `docs/PLAN.md` keeps phases, prose and Done-when checks. `scripts/plan.sh new|bind|note|show|pr|log|status` creates, logs, and reads this; `log` is the timeline.
- Checked: sorted checkbox lines before and after are equal (94/94); every old log line is present (711 of 711, none altered); each script command runs; a trial merge with #62 has no docs conflict.
- Not verified: CI (docs and a shell script only). 226 old entries name no PR and were filed by the commit that added them, so some sit under `pre-pr` or a nearby PR. Open branches that still edit PLAN.md will conflict once when rebased and should move their lines into their own plan file.
- Naming: files are `<YYYY-MM-DD>-<slug>.md` (date the work started, short kebab name), found by their `branch:` header, so no PR number is needed to start, log or read, and no file is ever renamed. `plan.sh link` records the PR number in the optional `pr:` header; `pr <N|slug>` and `show` work with either. The 40 earlier `pr-<N>.md` files were renamed from their PR's creation date and branch name (`2026-10-08-before-prs.md` for the pre-PR items). Checked: new/note/link/pr/log/status/show on a scratch branch (a second `new` on the same branch is refused), then discarded; `link` without a number reads `gh` and was not run against a real PR here.
