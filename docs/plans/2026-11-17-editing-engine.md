---
branch: feat/editing-engine
pr: 91
title: feat(edit): complete the editing engine and its controls
---

## P6 — Editing (plan in `docs/EDITING-PLAN.md`)
Engine first (E0–E3), then the controls (U1–U8). Stacked on PR #90 (`feat/editing`).
- [x] E0 world geometry in `crates/fig`, groups as `FRAME resizeToFit`, group fitting pass (typed specs move to E1 with their ops)
- [x] E1 Group/Ungroup, Frame selection, Align, Distribute, Flip, Rotate pivot, Reorder
- [x] E1 fills/strokes as paint lists (solid, gradients, image), stroke style, effects list
- [x] E1 node-level typography
- [x] E2 component properties, variants, generic instance overrides
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

### 2026-11-18T020000Z editing-engine-e1-arrange
- What changed and how it was checked: new ops `group`, `frame_selection`, `ungroup`, `align`,
  `distribute`, `flip`, `reorder` and a `pivot` on `rotate` (`compile_arrange.rs`); `reparent`
  now shares the placement helper. Engine tests in `crates/fig/tests/edit_arrange.rs` (page
  positions kept, z order, refusals, JSON forms), a pixel test in
  `crates/render/tests/edit_arrange.rs` (grouping draws the same, flip mirrors, align moves) and
  one MCP batch using every op (`apps/figstudio-mcp/tests/editing.rs`). Gate:
  `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh` passed (a first run failed only because the
  build cache evicted rlibs mid-link).
- Not verified: align/distribute across layers with different rotated parents beyond the
  page-space maths; Figma reopening the results.

### 2026-11-18T040000Z editing-engine-e1-style
- What changed and how it was checked: `edit::specs` (PaintSpec solid/gradient/image, PaintPatch,
  EffectSpec, EffectPatch, TextLength, TypographyExtra). New ops `set_paints`, `add_paint`,
  `update_paint`, `remove_paint`, `move_paint`, `set_stroke_style`, `add_effect`,
  `update_effect`, `remove_effect`, `move_effect`; `set_effects` takes a full list; whole-node
  typography on `set_typography`. Editing a styled list detaches its style. The renderer
  restrokes a VECTOR from its network when its stored stroke geometry was dropped. Image "Crop"
  is stored as Figma's `STRETCH` (was an invalid `CROP`). Tests:
  `crates/fig/tests/edit_style.rs`, pixel tests `crates/render/tests/edit_style.rs` (gradient
  direction, dash gaps, drop shadow, vector restroke), one MCP batch; the vector restroke, the
  crop mapping and the run-clearing typography tests fail with their change reverted. Gate:
  `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh` passed.
- Not verified: Figma reopening edited paints/effects; image crop transforms and filters drawn
  by the renderer (stored only); per-side stroke weights on non-box shapes.

### 2026-11-18T060000Z editing-engine-e2-components
- What changed and how it was checked: `edit::component_props` and ops `add_component_prop`,
  `edit_component_prop`, `remove_component_prop` (drops bindings and instance assignments),
  `bind_component_prop` / `unbind_component_prop` (BOOL → visible, TEXT → characters,
  INSTANCE_SWAP → nested instance), `combine_as_variants` (a `FRAME isStateGroup` with VARIANT
  definitions and `variantPropSpecs`; names only for files whose schema has neither),
  `add_variant`; `set_instance_override` takes schema-checked `fields` and refuses structural
  ones. Tests `crates/fig/tests/edit_components.rs` (instances resolved through
  `resolve_instance`, the renderer's merge) on an old and a new schema, and an MCP batch; the
  binding removal, structural-field refusal and old-schema handling fail when reverted. Gate:
  `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh` passed.
- Not verified: Figma reopening a set or properties made here; NUMBER/COLOR and other newer
  property kinds (not offered yet).
