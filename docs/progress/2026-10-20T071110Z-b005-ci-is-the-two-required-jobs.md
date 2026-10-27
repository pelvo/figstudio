- 2026-10-20 — CI is the two required jobs again (`fmt + clippy`, `build + test`). Slice
  jobs compiled the same crates ten times and the pull request waited on the slowest of
  them, so they are gone. In-job `parallel` steps failed the workflow in zero seconds with
  no jobs. The cache prefix is `v1-rust`. An exact hit of `v0-rust` did not save, so
  those entries have no workspace crates. The macOS release job uses the same prefix and
  does not set `shared-key`. `scripts/check.sh --test` still runs the release build and
  the wasm32 build. A pull request never runs the macOS release job, so dropping either
  would leave the required check green. Not timed on a new run.
