# Editing plan: a complete engine, then the controls for it

Status: engine (E0–E3) and controls (U1–U8) done on 2026-11-28; plan agreed with the
requester on 2026-11-17, as a follow-up PR stacked on PR #90
(branch `feat/editing-engine`, base `feat/editing`). Checklists and progress live in
`docs/plans/2026-11-17-editing-engine.md`; this file holds the
design, the order and the "Done when" checks.

## Why this order

The engine (`crates/fig/src/edit`) already has 45 `EditOp`s, so every Figma area has some op.
Many of them handle only one case where Figma handles a list or several variants: one solid fill,
one shadow, two-operand booleans, a whole-network vector replace. The GUI exposes a fraction of
even that, and holds a single selected layer. A good panel cannot be designed on top of
missing primitives, so the plan is:

1. **Engine first (E0–E3).** Each capability is an `EditOp`, tested through
   `execute_edit_ops`, and immediately usable by agents over MCP (`apply_edits`).
2. **Then the controls (U1–U8).** The GUI builds `EditOp` batches only, never raw `Val`s, so the
   app, the Live MCP and the standalone MCP share one path. Visual parity with Figma is not a
   goal; good ergonomics are: always-open sections, scrub fields, mixed values for
   multi-selection, keyboard shortcuts for every action, right-click menus.

## Rules for every step

- **One path.** GUI action → `EditOp` batch → `execute_edit_ops` → one undo entry. A GUI that
  needs something the ops cannot express gets a new op, not a side channel.
- **Keep the document writable** (ground rule 7). Ops that edit a list item by index keep every
  field of that item they do not set (unknown paint, effect, override fields included).
- **Never panic on input** (ground rule 5). Ops validate indices, enums, colours and counts and
  return `FigError`; recursive walks use explicit stacks.
- **Tests per op.** For each op: apply, undo, redo, save, reopen. Where the result is visible,
  add a pixel test (`crates/render/tests/`). Each test is seen failing first.
- **MCP parity.** `describe_edit_ops` documents every op; `apps/figstudio-mcp/tests/editing.rs`
  calls each new op family once.
- **Gate:** `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh` ends with `all checks passed` before
  each commit; rendering changes also get `scripts/thumb-sheet.py` and a look at the sheet.

## E0 — Foundations

Shared pieces the later ops need.

| Item | Design |
|---|---|
| World geometry in `crates/fig` | `edit::geometry::world_transform(doc, id)` and `layer_bounds(doc, id, space)` (transform × size, rotation included; no stroke or effect outsets, as Figma aligns on layer bounds). `crates/render` keeps its own; a test checks they agree. |
| Groups as Figma stores them | Saved files have no `GROUP` node type: a group is a `FRAME` with `resizeToFit: true` and no fills (306 in the fixtures). `create_shape GROUP` is changed to write that. |
| Group fitting pass | A derive pass, like the Auto-Layout one: after a batch, every group ancestor of a touched node is refitted to the union of its children's bounds, and the children's translations are compensated so nothing moves on the page. It runs before Auto-Layout and is part of the same undo entry. |
| Typed specs | `PaintSpec`, `EffectSpec`, `StrokeSpec`, `TypographySpec` (serde, in `edit::specs`) shared by ops, MCP JSON and GUI. Converting to `Val` fills only the fields the spec sets. Added with the E1 ops that use them, so none sits unused. |

**Done when:** moving a child of a group refits the group, keeps every page position, and
undoes in one step; `world_transform` matches `figstudio_render::absolute_transform` on every
fixture node.

## E1 — Engine tier 1: what any editing session needs

| Capability | Op(s) | Notes |
|---|---|---|
| Group / Ungroup | `Group { targets, name? }`, `Ungroup { target }` | Targets must share a parent; the group takes the topmost target's z position. Ungroup keeps page positions and z order. |
| Frame selection | `FrameSelection { targets }` | Like Group but a real `FRAME` (fills, clip), sized to the union. |
| Align | `Align { targets, edge: Left\|HCenter\|Right\|Top\|VCenter\|Bottom, to: Selection\|Parent }` | One target aligns to its parent. Rotated layers use their bounds. |
| Distribute | `Distribute { targets, axis, spacing: Option<f64> }` | `None` = equal gaps; `Some` = fixed gap (Figma's "tidy"). |
| Flip | `Flip { target, axis }` | Mirrors the transform about the layer centre. |
| Rotate by pivot | `Rotate { target, degrees, pivot: TopLeft\|Center }` | `pivot` defaults to `TopLeft` (today's behaviour); the GUI uses `Center`. |
| Z order | `Reorder { target, to: Front\|Back\|Forward\|Backward }` | Compiles to `Reparent` with the right index and fractional position. |
| Fills and strokes as lists | `SetPaints { target, slot: Fill\|Stroke, paints }`, `AddPaint`, `UpdatePaint { index, patch }`, `RemovePaint { index }`, `MovePaint { from, to }` | `PaintSpec`: Solid (hex, opacity), Gradient (Linear/Radial/Angular/Diamond, stops, two handles), Image (hash, scale mode, crop transform, rotation, filters). Each has visible, opacity and blend mode. Editing a styled paint detaches the style, as today. `SetSolidFill` stays as a shorthand. |
| Stroke style | `SetStrokeStyle { target, weight?, align?, cap?, join?, miter?, dashes?, sides? }` | `sides` = per-side weights for box shapes. Drops stale `strokeGeometry`; LINE and VECTOR get restroked by the renderer (VECTOR from its network, new). |
| Effects as a list | `SetEffects { target, effects }`, `AddEffect`, `UpdateEffect`, `RemoveEffect` | `EffectSpec`: Drop/Inner shadow (colour, offset, blur, spread, show behind), Layer blur, Background blur. The current single-shadow form stays accepted. |
| Node typography | `SetTypography` gains line height, letter spacing, paragraph spacing, vertical align, decoration, case, truncation | Applies to the whole text: sets the node field and removes that field from per-run overrides; drops `derivedTextData`. |
| Nudge | covered by `Move { relative: true }` | Listed so the GUI table is complete. |

**Done when:** every row has passing apply/undo/save/reopen tests; Group, Align, Distribute,
Flip, gradients, strokes and effects each have a pixel test; MCP `apply_edits` runs one batch
using every new op.

## E2 — Engine tier 2: components and the design system

| Capability | Op(s) | Notes |
|---|---|---|
| Component properties | `AddComponentProp { component, name, kind: Bool\|Text\|InstanceSwap\|Variant, default }`, `EditComponentProp`, `RemoveComponentProp`, `BindComponentProp { node, field, prop }` | Binds visibility, characters or nested instance to a prop, as Figma does. |
| Variants | `CombineAsVariants { components }`, `AddVariant { set, from }`, `RenameVariantProp` | Writes the `STATE_GROUP` frame and variant names (`Prop=Value, …`). |
| Generic instance override | `SetInstanceOverride` gains `fields` (validated against the schema and a list of overridable fields) | Covers stroke, effects, text style, corner radius, etc. |
| Styles | `CreateStyle { kind: Fill\|Stroke\|Text\|Effect\|Grid, name, spec \| from_node }`, `UpdateStyle`, `DetachStyle { target, slot }`, `DeleteStyle` | Replaces the fill-only `CreateFillStyle` (kept as shorthand). |
| Variables | `CreateVariableCollection { name, modes }`, `AddMode`, `RenameMode`, `CreateVariable { collection, name, type: Color\|Number\|String\|Bool, values }`, `SetVariableValue`, `DeleteVariable` | Generalises `SetVariableColor`. |
| Variable binding | `BindVariable { target, field, variable }` | Number → radius, gap, padding, size, opacity, stroke weight; String → characters; Bool → visible; Color → paints (today's `BindPaintVariable`). |

**Done when:** a component with each prop kind, a variant set, a style of each kind and a
variable of each type can be made, bound, edited and deleted, saved, reopened, and rendered
the way the bindings say.

## E3 — Engine tier 3: vectors and the rest

| Capability | Op(s) | Notes |
|---|---|---|
| Vertex editing | `EditVectorNetwork { target, edits: [MoveVertex, AddVertex, RemoveVertex, SetHandles, AddSegment, RemoveSegment, SetRegionFill] }` | Rebuilds geometry from the network; keeps vertex/segment ids stable where possible. |
| Shape → vector, Flatten, Outline stroke | `ConvertToVector`, `Flatten { targets }`, `OutlineStroke { target }` | Flatten merges into one vector; outline stroke uses the stroker the renderer already has. |
| Booleans | `BooleanCombine { operation, targets }` (N operands) | Keeps a live `BOOLEAN_OPERATION` with the operands as children, as Figma does, and stores the computed result. Exact curves: pick a maintained crate (candidates `i_overlay`, `flo_curves`; decision recorded here) instead of today's 16-segment flattening. |
| Masks | `SetMask { target, mask: Option<Alpha\|Vector\|Luminance> }` | Mask a group's following siblings, as Figma does. |
| Layout grids | `SetLayoutGrids { target, grids }` | Columns, rows, grid; stretch/center/min/max. |
| Export settings | `SetExportSettings { target, settings }` | Format, scale or size, suffix; the app's export worker reads them. |
| Prototype | `SetReactions { target, reactions }`, `SetFlowStart`, `SetScrollBehavior` | The prototype preview already plays them. |

**Done when:** each row passes its tests; booleans and outline stroke match Figma's stored
geometry on the fixtures within a set tolerance.

## U — The controls

UI work starts after E1 lands (it needs the ops), and each U step only uses ops that exist.

| Step | What | Key decisions |
|---|---|---|
| U1 Selection model | `Tab.selection` becomes a `Selection` (ordered set + primary). Shift-click adds/removes; a Select-tool drag on empty canvas box-selects. | Region copy, which uses that drag today, moves to a modifier drag and the context menu. Existing single-node readers use `primary()`. The MCP snapshot reports all selected ids. |
| U2 Canvas interaction | Union selection box with handles (rotated box for one rotated layer), multi-move and multi-resize, Alt-drag duplicates, arrow nudge (Shift = 10), rotate handle, snapping and smart guides with distances. | Snapping in page space, toggled by a setting. |
| U3 Shortcuts and menus | Right-click menu on canvas and layer rows: Cut, Copy, Paste, Paste here, Duplicate, Delete, Group, Ungroup, Frame selection, Bring to front/forward, Send backward/to back, Flip, Show/Hide, Lock, Create component, Detach instance, the existing Copy-as items. Shortcuts for each (keymap, rebindable). | One `menu::Action` per op family, so the native menu, command bar and context menu stay in sync. |
| U4 Design panel | Always-open sections: Position (align, X/Y, rotation, flip), Layout (W/H, aspect lock, constraints, Auto Layout), Appearance (opacity, blend, radius with per-corner, visibility), Fill and Stroke lists, Effects, Typography, Export. | Widget kit first: scrub-label number field (drag the label, type an expression), swatch + colour picker with hex and opacity, segmented buttons, icon toggles, paint/effect rows. Multi-selection shows `Mixed` and edits all. Drafts are kept while a field has focus. |
| U5 Layers panel | Hover eye and lock, double-click rename, drag to reorder and reparent, Shift/Cmd multi-select, expand/collapse. | Drag uses `Reparent`/`Reorder` batches. |
| U6 Text on canvas | Double-click a text layer to edit in place (caret, selection, typing, IME), range styling from the panel. | Uses `SpliceText` and `StyleTextRange`; reuses `text_select` hit testing. |
| U7 Components and assets | Assets panel: local components (drag to canvas), styles, variables table with modes. Instance section: props, variant dropdowns, swap, reset, detach. Component section: define props. | Uses E2 ops. |
| U8 Vectors, prototype, export | Vector edit mode (vertices, handles, pen tool), boolean and mask buttons, prototype connections, export section. | Uses E3 ops. |

**Done when (each U step):** kittest UI tests drive every new control through its op;
a screenshot of the panel/canvas is reviewed; keyboard-only use works (accessibility names
on every control, `a11y` helpers).

## Open items carried in

- ⌘V with a clipboard that holds only HTML gives the app no event (egui-winit 0.36); needs an
  upstream change or a native accelerator decision (thread on PR #90).
- The fill-colour draft has no automated test yet; U4's field widget replaces it and gets one.

## Size and review

This work is a follow-up PR stacked on PR #90, so #90 stays reviewable. Each phase lands as
its own commits with the gate passing, the plan file updated, and a short PR comment listing
what changed and what is not verified, so a reviewer can review phase by phase.
