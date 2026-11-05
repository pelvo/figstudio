---
branch: fix/mask-filter-clip
pr:
title: Clip shadows and blurs to masks; plan.sh link on macOS
---

## P1 — Renderer (`crates/render`)
- [x] A mask clips its siblings' shadows and blurs (the filter layer is wrapped in a clip layer of the innermost clip, frame or mask)

## P5 — Delivery
- [x] `scripts/plan.sh link` works on macOS (no `sed -i`)

## Progress

### 2026-11-05T054757Z mask-filter-clip
- What changed and how it was checked: `Ctx::clip` holds the innermost clip (frame or mask); filter layers are wrapped in a clip layer of it. New test `a_mask_clips_its_siblings_shadow_and_blur` failed first (shadow alpha 255 outside the mask). `plan.sh link 99` reproduced the macOS `sed -i` failure, then passed after the fix. Fixture renders byte-identical. check.sh on 1.99.0: every slice passes except tests-npm `the test hold is capped` (799 ms, machine under load; npm untouched); browser-fonts, release and wasm32 passed with `--test-one`.
- Not verified: a real file with a mask over a shadowed layer; GPU canvas on a device; `plan.sh link` on Linux (the temp-file form is plain POSIX).
