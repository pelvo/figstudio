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
- [x] U2 canvas interaction (multi-move/resize, rotate handle, nudge, Alt-duplicate, snapping)
- [x] U3 right-click menus and shortcuts for every op family
- [x] U4 design panel sections and widget kit
- [x] U5 layers panel editing
- [x] U6 text editing on canvas
- [x] U7 components and assets UI
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
- Not verified: the multi-selection outlines in a real window. (Correction, 2026-11-27: the
  computer-use run had driven the installed figstudio.app, to which the test build handed the
  file, not the new build; on a separately bundled build, accessibility clicks still carry no
  Shift.) Figma-style "touch" box select
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

### 2026-11-27T090000Z editing-engine-u4a-design
- What changed and how it was checked: `editor::widgets` (number field with a scrubbing label,
  typed numbers and one-operator expressions, `Mixed` for differing values; colour row with
  swatch picker, draft-keeping hex and opacity) and `editor::design`: Position (six align
  buttons, X/Y, rotation, flip), Layout (W/H with aspect lock, clip content), Appearance
  (opacity, visibility, blend mode, corner radius with per-corner split), Fill and Stroke
  lists (top paint first; colour or gradient/image label, opacity, show/hide, remove, add in the
  title row; stroke position and weight), Effects list (blur, show/hide, remove; shadow offset,
  spread and colour). They replace the read-only Layout, Layer, Fill, Stroke and Effects
  sections; "Edit Layer" is now a collapsed "More" (rename, auto layout, text, components).
  Each control edits every selected layer as one undo step (scrubbing coalesces).
  `SetLayerProps` gains `proportions_locked`. kittest tests `ui_design_tests.rs`; editing
  every selected layer, Mixed, the aspect lock and top-first order fail when reverted; the
  shared test helper scrolls a control into view before clicking. Screenshot of a bundled
  build reviewed (section + buttons moved into the title row, "-0" fixed). Gate:
  `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh` passed.
- Not verified: the colour picker popup and the blend menu opened in a real window; gradient
  stops and image settings are shown but not editable yet (U4b).

### 2026-11-27T120000Z editing-engine-u4b-sections
- What changed and how it was checked: `editor::design_more`: a name field; Auto layout ("+"
  adds Figma's default: vertical, 10 gap and padding, hug; direction, wrap, remove, gap, four
  paddings, alignment per axis, Fixed/Hug per axis); "In auto layout" (fill container,
  stretch, absolute) or Constraints for a child; Text (font, style, size, line height with an
  Auto hint, letter spacing, horizontal and vertical alignment, resizing, decoration, case,
  characters); Instance (reset, detach); Export (settings list, scale and format, an export
  button that runs the app's PNG/SVG export). The collapsed "More" block and
  `inspector_sections.rs` are gone. Title-row "+" buttons now clear the floating scroll bar
  (a click could land on the bar). kittest tests in `ui_design_tests.rs`; the export button,
  padding kept per side, the scroll-bar gap and the Constraints section fail when reverted.
  Screenshots of a bundled build reviewed: the gap icon follows the direction, the
  alignment/sizing/constraint menus got axis tags, and Auto line height no longer reads
  "Mixed". Gate: `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh` passed.
- Not verified: the colour picker popup, the blend menu and the combo popups opened in a real
  window; the Auto line-height hint has no test (seen in a screenshot); the export button test
  covers PNG only.

### 2026-11-27T150000Z editing-engine-u2-canvas
- What changed and how it was checked: `editor::canvas_select` takes over the Select tool's
  gestures from `canvas_gizmos`: dragging moves every selected layer (an unselected one is
  selected once its drag starts; a top-level frame or section with layers still draws a box);
  Alt-drag moves copies (one undo step); the handles resize one layer along its own axes or
  several inside their page box (Shift or the aspect lock keeps the ratio); just outside a
  corner rotates about the centre (several about their shared centre; Shift: 15° steps);
  one rotated layer gets its own turned box; moves snap edges and centres to siblings and the
  parent frame within 5 points, with red guides and distances (⌘/Ctrl moves freely; Settings
  → Canvas → Snapping); arrow keys nudge by 1, Shift by 10. The inspector leaves a canvas
  gesture's undo step to the gesture (it sealed an Alt-drag midway when the copies became
  selected). Tests: `canvas_select_tests.rs` (box resize, snap, gaps, multi resize, shared
  rotation, the turned box) and `ui_canvas_gesture_tests.rs` (seven gestures driven frame by
  frame on the app); twelve reverts each fail. Screenshot of a bundled build: the turned box
  and handles on a rotated layer. Gate: `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh` passed.
- Not verified: drags in a real window (computer use does not deliver drags to the background
  app; the UI tests drive them). Not done: resize does not snap; a rotated layer among several
  scales along its own axes instead of skewing (shortcut comment); Alt-resize from the centre.

### 2026-11-27T180000Z editing-engine-u5-layers
- What changed and how it was checked: `layers_edit` and the rows in `panels.rs`: Cmd/Ctrl-click
  adds or removes a row, Shift-click selects the rows between the primary layer and the one
  clicked; double-click renames in place (Enter or clicking away keeps it, Escape drops it;
  the global Escape waits while a rename is open); eye and lock buttons on hover, and always
  on a hidden or locked layer; dragging rows reorders and reparents (top/bottom quarter of a
  container row: above/below, middle: inside at the top; below an open container: inside),
  several keep their order, never into themselves or an instance. `EditOp::Reparent` gains
  `keep_position` (opt-in, serde default false): the layer stays put on the page.
  Tests: `layers_edit_tests.rs`, `ui_layers_edit_tests.rs`, and an engine test in
  `edit_arrange.rs`; ten reverts each fail. Real window (bundled build): the eye-off and lock
  icons on a hidden, locked row; a double-click opens the name field and Escape keeps the
  name. Gate: `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh` passed.
- Not verified: dragging rows in a real window (computer use cannot drag the background app).
  Not done: a collapsed frame does not open while hovered during a drag; no autoscroll at the
  list edges; rows do not drag in the filtered list.

### 2026-11-27T210000Z editing-engine-u6-text
- What changed and how it was checked: `text_edit`: in text mode on a plain text layer,
  typing and IME commits insert, Backspace/Delete remove whole characters, Enter breaks the
  line, arrows move (Shift extends; ⌘←/→, Home/End: line ends; ↑/↓: lines), ⌘A selects all,
  ⌘C/⌘X/⌘V use the text, Escape leaves the text; all as `SpliceText`, one undo step per
  typing run (a caret move ends it). These keys reach the text before the app's shortcuts
  (letters no longer pick tools, Backspace no longer deletes the layer). The caret is drawn
  and the IME output set. `Renderer::text_glyphs` (render, via `SystemFonts::carets`) lays the
  text out again when an edit dropped `derivedTextData`, so carets and hit tests keep working;
  text without stored glyphs now gets a caret where clicked instead of a whole selection.
  Line breaks end the cluster before them in `text_select`. The Design panel's Text section
  styles a selected range with `StyleTextRange` (font, size, line height, letter spacing,
  decoration, case). Tests: `text_edit_tests.rs`, `ui_text_edit_tests.rs`, the updated
  `ui_text_select_tests.rs`; nine reverts each fail. Real window (bundled build): typing
  appended to a text layer on the canvas and in the panel. Gate:
  `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh` passed.
- Not verified: the caret drawing and the IME candidate window in a real window. Not done:
  word-wise moves (⌥←/→) and double-click to select a word; with a range selected the panel
  shows the layer's values, and a range's fill colour is not wired; text inside instances
  stays read-only.

### 2026-11-28T000000Z editing-engine-u7-components
- What changed and how it was checked: fig `component_props::{component_props,
  instance_props}` read a component's or instance's properties (values, variant options) and
  `Tokens` gains `collections()`/`styles()`. Design panel: Instance (swap the main component,
  go to it, variant drop-downs, boolean/text/instance-swap values, reset, detach), Component
  properties (rename, default, remove; add boolean/text/swap; add a variant), Combine as
  variants for several components, and Component property bindings (visibility, text, swap)
  for a layer inside a component. Sidebar: a Layers | Assets switch; Assets lists components
  (click: place in the middle of the view; drag onto the canvas), styles (click applies; a
  colour style with Shift goes to the stroke) and variables (a table per collection with a
  column per mode: edit values, rename, delete, add variables, modes and collections), built
  once per document revision. Tests: an engine test in `edit_components.rs`,
  `assets_tests.rs`, `ui_components_tests.rs`; ten reverts each fail; the accessibility test
  caught the new tab labels without a focus ring (fixed). Real window (bundled build): the
  Assets tab, an instance's section, the variables table (columns widened after a look).
  Gate: `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh` passed.
- Not verified: dragging a component onto the canvas in a real window; the colour picker
  popups. Not done: a dropped component lands on the page, not in the frame under the
  pointer; aliases are shown read-only; no variable binding on layer fields; text, effect
  and grid styles have no preview; creating a style from the selection.

### 2026-11-28T030000Z editing-engine-u8a-vectors
- What changed and how it was checked: `editor::vector_edit`. Vector edit mode (double-click
  a vector, or Enter): drag points and the selected point's curve handles (a straight point's
  handles sit on it and are hidden, so it stays draggable); Delete removes a point, arrows
  nudge it (Shift: 10); Enter, Escape or a click away leaves; canvas clicks do not pick other
  layers meanwhile. Pen tool (P): click adds a point, dragging pulls out mirrored handles,
  clicking the first point closes the path (with a region), Enter or Escape finishes it
  open, another tool keeps what was drawn; one undo step; a 1 pt black centred stroke, no
  fill. fig: `EditVectorNetwork` refits the layer to its curves (size and transform, the
  network re-expressed in layer units) so nothing moves on the page. Tests:
  `vector_edit_tests.rs`, `ui_vector_tests.rs`, an `edit_vectors.rs` refit test; ten reverts
  each fail (two tests strengthened after a revert passed). Real window (bundled build): the
  points of a logo vector in edit mode, the Pen tool in the toolbar. Gate:
  `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh` passed.
- Not verified: dragging points in a real window (computer use cannot drag the background
  app). Not done: adding a point on a segment; Alt-dragging a straight point into a curve;
  the pen's new layer is sized to its points, not its curves. U8 stays open (U8b: boolean and
  mask buttons, prototype).
