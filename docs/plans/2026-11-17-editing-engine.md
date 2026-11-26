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
- [x] E2 styles (all kinds), variables (collections, modes, all types, binding)
- [x] E3 vertex editing, convert/flatten/outline stroke, N-operand exact booleans
- [x] E3 masks, layout grids, export settings, prototype reactions
- [x] U1 multi-selection model and box select
- [ ] U2 canvas interaction (multi-move/resize, rotate handle, nudge, Alt-duplicate, snapping)
- [x] U3 right-click menus and shortcuts for every op family
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

### 2026-11-18T090000Z editing-engine-e2-design
- What changed and how it was checked: `edit::styles` (create from specs or a layer, update
  propagated to every user for fill, stroke, effect, grid and text styles, delete detaches with
  the look kept; text-style binding copies its text fields) and `edit::variables` (collections,
  modes add/rename/remove, COLOR/FLOAT/STRING/BOOLEAN variables with literal or alias values,
  rename, delete keeping users' values, `bind_variable` for radius, opacity, stroke weight,
  gaps, padding, size, font size, paragraph spacing, visibility and characters). Value and mode
  changes refresh bound layers in the same undo entry. Unhandled ops now fail instead of being
  ignored. Tests `crates/fig/tests/edit_design.rs` and an MCP batch; propagation, text-style
  fields, the refresh and alias-keeping fail when reverted. Gate:
  `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh` passed.
- Not verified: Figma reopening files with collections or styles made here; published
  (library) styles and variables are read-only to these ops.

### 2026-11-18T130000Z editing-engine-e3-vectors
- What changed and how it was checked: `edit::outline` (layer outlines as kurbo paths, stroke
  outlines with align/caps/joins/dashes, path → vector network) and `edit::vector_ops`. Ops
  `boolean_combine` on 2+ siblings (live: the group-fitting pass refits the box and recomputes
  the result when an operand changes; a text operand drops the stored result so the renderer
  draws the operands and counts it), `convert_to_vector` (shape's own vertex order),
  `flatten`, `outline_stroke`, `edit_vector_network`. Decision: exact curve booleans use
  `linesweeper` 0.5 (Linebender, MIT/Apache, no unsafe, on the kurbo 0.13 already in the
  tree); `i_overlay` was rejected because it flattens curves. The hand-rolled 16-segment
  boolean is removed. Fix: Exclude is stored as the file format's `XOR` (was `EXCLUDE`, which
  failed schema validation). Tests `crates/fig/tests/edit_vectors.rs` (areas on stored
  geometry), pixel test `crates/render/tests/edit_vectors.rs`, MCP batch; live refresh, XOR,
  vertex order and inside-stroke clipping fail when reverted. Gate:
  `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh` passed, plus `--test-one wasm32`.
- Not verified: Figma reopening booleans made here; outline of text (refused: needs fonts);
  vertex corner radii in networks.

### 2026-11-24T110000Z editing-engine-e3-proto
- What changed and how it was checked: `compile_proto.rs` ops `set_mask` (alpha, outline,
  luminance; clearing drops `maskType`), `set_layout_grids` (columns, rows, square grid; frames
  only; detaches a grid style), `set_export_settings` (png, jpg, svg, pdf by scale, width or
  height), `set_reactions` (triggers incl. after-timeout in seconds, navigate/overlay/swap/
  scroll-to/back/close/URL, transition, duration, easing; destinations must exist),
  `set_flow_start` (top-level frames only), `set_scroll_behavior`. Tests
  `crates/fig/tests/edit_proto.rs`, `crates/render/tests/edit_proto.rs` (the prototype player
  lists the flow and plays the link), MCP batch; four behaviours fail when reverted. Gate:
  `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh` passed incl. wasm32. The engine phase E0–E3 is
  complete.
- Not verified: Figma reopening reactions or grids made here; variable-driven reactions
  (set variable, conditionals) are not offered.

### 2026-11-27T010000Z editing-engine-u1-selection
- What changed and how it was checked: `Tab.also` holds the layers selected besides the
  primary `selection` (which every existing reader keeps using); `selected()`,
  `toggle_selected()`, `select_all_of()` and `prune_selection()` (after edits and undo).
  Shift-click adds or removes a layer; a marquee drag also selects the top-level layers it
  encloses (sections looked into, hidden and locked skipped) and keeps the region menu; ⌘A
  outside text editing selects the page's layers; delete and duplicate act on the whole
  selection in one undo step. The canvas outlines every selected layer and the box around
  them. The Live MCP snapshot carries every selected id (`list_tabs.selectionIds`,
  `get_selection.nodes`). kittest tests in `ui_multi_select_tests.rs` (real pointer and key
  events) each fail when their behaviour is reverted; existing copy/pick/marquee/menu tests
  pass. Gate: `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh` passed.
- Not verified: the multi-selection outlines in a real window. The computer-use tool drives
  the app in the background, so Shift and ⌘ never reach it; Figma-style "touch" box select
  (this selects only fully enclosed layers).

### 2026-11-27T050000Z editing-engine-u3-menus
- What changed and how it was checked: `editor::commands` (one `EditCmd` per command, with
  label, enabled-for-selection and a single-batch `run`); `Action::Edit`; the Edit menu gains
  Group, Ungroup, Frame selection and Arrange (z-order, flip, rotate 90°), Align (six edges,
  distribute), Show/Hide, Lock, Create component, Detach, Boolean (four ops, flatten, outline
  stroke, use as mask) submenus. The canvas and layer-row right-click menus show Duplicate,
  Delete and the same items above the Copy items; a right-click on a selected layer keeps the
  selection, one on another row selects it. Rebindable shortcuts in a new "Arrange" section:
  Figma's defaults except group ⌥⌘G / ungroup ⇧⌥⌘G (⌘G is Find next here) and lock ⇧⌥L
  (⇧⌘L is Open Figma link); they wait while a text field has focus (⌥A types å). kittest tests
  in `ui_multi_select_tests.rs`; keep-selection, row select, the text-field guard and
  ungroup selecting its children fail when reverted; all 738 app tests pass. Gate:
  `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh` passed.
- Not verified: the submenus in the native macOS menu bar and the menus in a real window.
