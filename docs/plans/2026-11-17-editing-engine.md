---
branch: feat/editing-engine
pr:
title: feat(edit): complete the editing engine and its controls
---

## P6 — Editing (plan in `docs/EDITING-PLAN.md`)
Engine first (E0–E3), then the controls (U1–U8). Stacked on PR #90 (`feat/editing`).
- [ ] E0 world geometry in `crates/fig`, groups as `FRAME resizeToFit`, group fitting pass, typed specs
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
