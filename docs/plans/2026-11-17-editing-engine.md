---
branch: feat/editing-engine
pr: 91
title: feat(edit): complete the editing engine and its controls
---

## P6 — Editing (plan in `docs/EDITING-PLAN.md`)
Engine first (E0–E3), then the controls (U1–U8). Stacked on PR #90 (`feat/editing`).
- [x] E0 world geometry in `crates/fig`, groups as `FRAME resizeToFit`, group fitting pass (typed specs move to E1 with their ops)
- [ ] E1 Group/Ungroup, Frame selection, Align, Distribute, Flip, Rotate pivot, Reorder
- [ ] E1 fills/strokes as paint lists (solid, gradients, image), stroke style, effects list
- [ ] E1 node-level typography
- [ ] E2 component properties, variants, generic instance overrides
- [ ] E2 styles (all kinds), variables (collections, modes, all types, binding)
- [ ] E3 vertex editing, convert/flatten/outline stroke, N-operand exact booleans
- [ ] E3 masks, layout grids, export settings, prototype reactions
- [ ] U1 multi-selection model and box select
- [ ] U2 canvas interaction (multi-move/resize, rotate handle, nudge, Alt-duplicate, snapping)
- [ ] U3 right-click menus and shortcuts for every op family
- [ ] U4 design panel sections and widget kit
- [ ] U5 layers panel editing
- [ ] U6 text editing on canvas
- [ ] U7 components and assets UI
- [ ] U8 vector edit mode, prototype and export UI

## Progress

### 2026-11-17T113000Z editing-engine-e0
- What changed and how it was checked: `edit::space` (Xform, world transform, layer bounds)
  agrees with the renderer's `absolute_transform` on every fixture node
  (`crates/render/tests/edit_space.rs`). `create_shape GROUP` writes a `FRAME` with
  `resizeToFit: true`, as Figma saves groups. `edit::groups::fit_groups` refits every group above a
  touched node after a batch or paste, inner first, keeping page positions, in the same undo
  entry (`crates/fig/tests/edit_groups.rs`; both behaviour tests seen failing with the change
  reverted). Gate: `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh` passed.
- Not verified: that Figma reopens a group made here (no Figma run); groups with rotated or
  flipped children were reasoned, not tested.
