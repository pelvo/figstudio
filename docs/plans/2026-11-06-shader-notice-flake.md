---
branch: test/shader-notice-flake
pr:
title: Shader notice test: wait for the warning chip
---

## P5 — Delivery
- [x] `a_file_with_shaders_but_no_code_says_so_and_takes_their_zips` no longer flakes: its clicks wait for the page warning chip the shader notice hangs under

## Progress

### 2026-11-06T063119Z shader-notice-flake
- What changed and how it was checked: the shader notice sits under the page warning chip, which shows once tiles report what they skipped. When the chip appeared in the frame that took the "Choose shader zips…" click, the notice moved down 38 px and the click missed (the `build + test` failure on #78 and on `main`). `counted` now also waits for the chip. Reproduced locally with the old steps run 96 times, 24 at once: the only failure was the run whose chip appeared in the click's frame (logged chip state and button rect before/after). The fixed test passed 96 of 96 runs under the same load.
- Not verified: the full `scripts/check.sh` locally (only the figstudio shader UI tests and `clippy -p figstudio`).
