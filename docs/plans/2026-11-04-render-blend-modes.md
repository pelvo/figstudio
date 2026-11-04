---
branch: feat/render-blend-bg-blur
pr: 72
title: Draw layer and paint blend modes
---

## P1 — Renderer (`crates/render`)
- [x] Layer blend modes drawn with a blend layer around the node; `NORMAL`/`PASS_THROUGH` exact and no longer counted
- [x] Fill and stroke paint blend modes drawn; `LINEAR_BURN` and unknown modes still counted
- [x] Clipping frames and image-paint crops clip without isolating, so blended children see the backdrop
- [x] Recorder (parallel/tiles) and GPU scene replay blend layers
- [x] Shadows and blurs inside a clipping frame stay clipped (filter layer wrapped in the frame's clip layer)
- [x] Background blur still counted as skipped (vello_cpu 0.3 has no backdrop filter)
- [ ] Open: masks clip with a clip path too, so a masked sibling's shadow or blur probably spreads past the mask (same Vello limit; predates this branch; not tested)

## Progress

### 2026-11-04T132503Z render-blend-modes
- What changed and how it was checked: layer and paint blend modes drawn (`paint::blend_mode`); frames clip with a clip path and wrap filter layers in the frame's clip layer. New tests seen failing first (blend.rs, shadows.rs clip test, replay.rs blend replay via a recorder mutation). Fixture renders differ by at most 2/255 at edges; thumb sheet and a one-tile-per-mode sheet reviewed. check.sh on 1.99.0: all slices pass except tests-npm `the test hold is capped` (timing under load average 159; npm untouched).
- Not verified: GPU canvas on a device, a real file with blend modes, re-measured counts on the large file.
