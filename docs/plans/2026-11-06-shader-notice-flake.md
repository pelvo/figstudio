---
branch: test/shader-notice-flake
pr: 79
title: Fix two flaky app UI tests (shader notice click, stranded tiles)
---

## P5 — Delivery
- [x] `a_file_with_shaders_but_no_code_says_so_and_takes_their_zips` no longer flakes: its clicks wait for the page warning chip the shader notice hangs under
- [x] Opening a file (or changing a file's shaders) no longer leaves another open file's canvas blank: `apply_shaders` clears every tab's tile requests, since the stamp that drops stale tiles is shared (the `another_file_fades_in_over_the_last_one_never_over_the_bare_card` flake)

## Progress

### 2026-11-06T063119Z shader-notice-flake
- What changed and how it was checked: the shader notice sits under the page warning chip, which shows once tiles report what they skipped. When the chip appeared in the frame that took the "Choose shader zips…" click, the notice moved down 38 px and the click missed (the `build + test` failure on #78 and on `main`). `counted` now also waits for the chip. Reproduced locally with the old steps run 96 times, 24 at once: the only failure was the run whose chip appeared in the click's frame (logged chip state and button rect before/after). The fixed test passed 96 of 96 runs under the same load.
- Not verified: the full `scripts/check.sh` locally (only the figstudio shader UI tests and `clippy -p figstudio`).

### 2026-11-06T092054Z shader-notice-flake
- What changed and how it was checked: `ui_tests::chrome::another_file_fades_in_over_the_last_one_never_over_the_bare_card` timed out waiting for tiles (on this PR and on `main`). A diagnostic copy run 192 times under load stalled twice: the active tab sent 20 tile requests, got 20 back, cached none and never asked again. Cause, an app bug: `apply_shaders` (run on every file open) bumps the shared freshness stamp but cleared only its own file's tile requests, so another file's tiles in flight came back stale, were dropped, and stayed listed as requested; `next_batch` then never re-sent them and that canvas stayed blank until the view changed. Fix: clear every tab's requests. New test `shaders_changing_in_one_file_do_not_strand_another_files_tiles` failed before the fix and passes after; the shader and tile tests pass (35); the fade test passed 96 of 96 runs under load (the old code failed about 1 in 96).
- Not verified: `clippy -p figstudio` locally (the disk filled up; CI's `fmt + clippy` checks it); the full `scripts/check.sh`.
