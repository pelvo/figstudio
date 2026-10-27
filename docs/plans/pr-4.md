---
pr: 4
branch: fix/known-gaps
title: fix(render,app): close stroke, shadow, fill and nested-instance gaps
---

## P1 — Renderer (`crates/render`)
- [x] missing VECTOR fills counted only when a visible fill paint has no path (A19)

## Progress

### 2026-10-10T000000Z b034-strokes-a11-strokejoin-strokecap-dashpattern-and
- 2026-10-10 — Strokes (A11): `strokeJoin`, `strokeCap`, `dashPattern` and per-side border
  weights drawn when no stored geometry applies; stroke overrides drop stale stored outlines.
  Pixel tests in `stroke_style.rs`. The two reported dialogs already matched Figma's border
  weight and radius after the strokeAlign checkpoint; this change does not alter their pixels.
  Not verified: dashed strokes and caps against Figma on a real file.

### 2026-10-11T141129Z b013-approximation-counters-measured-on-a-large
- 2026-10-11 — Approximation counters measured on a large real file and broken down by
  reason; plan to clear them recorded in `docs/APPROXIMATIONS.md` (task T19).

### 2026-10-11T141129Z b014-a19-exclude-stroke-only-vectors-and
- 2026-10-11 — A19: exclude stroke-only vectors and hidden/zero-opacity fills from the
  missing-fill counter. Synthetic counter regression observed red/green; workspace and
  wasm gates passed. Figma comparison not verified.

### 2026-10-11T141129Z b035-a7-closed-vector-nodes-clip-stored
- 2026-10-11 — A7: closed VECTOR nodes clip stored inside/outside stroke geometry to the fill,
  as boxes already did (`geom::closed_network` reads the vector network). The tooth-chart A–E
  labels on a private file no longer render bold; test in `stroke_align.rs`. Not verified:
  live compare in Figma; open vectors still draw these strokes unclipped (counted).

### 2026-10-11T141129Z b036-a16-a-frame-that-clips-its
- 2026-10-11 — A16: a frame that clips its content now draws its stroke above its children,
  so a table's outer border is no longer hidden by the cells that fill it; non-clipping frames
  keep the stroke below. Test in `stroke_order.rs`. Not verified: live compare in Figma. The
  grey grid lines and header fill are the file's own stored paints and are left as stored.

### 2026-10-11T141129Z b037-a17-constraints-no-longer-move-the
- 2026-10-11 — A17: constraints no longer move the in-flow children of an auto-layout frame
  that an instance resized; auto layout keeps them at the padding, so a label in a
  hug-width chip stays centred instead of drifting left. Absolute-positioned children still
  follow constraints. Test in `auto_layout.rs`. Not verified: live compare in Figma. A full
  re-run of auto layout for instances without derived transforms was tried and dropped: it
  moved stored positions that already follow property defaults (see the report).

### 2026-10-11T141129Z b038-a20-fill-based-drop-shadows-are
- 2026-10-11 — A20: fill-based drop shadows are drawn behind the node in list order, and
  inner shadows over its fills. Positive spread is applied; negative spread and non-normal
  blend modes are counted as approximate. Tests in `shadows.rs`. Not verified: live compare
  in Figma; spread on rounded shapes uses a stroked outline.

### 2026-10-11T141129Z b039-a20-review-inner-clips-install-their
- 2026-10-11 — A20 review: inner clips install their fill rule; drop masks draw each path
  under its own rule; transparent solid fills no longer cast opaque geometry shadows.
  Compound inner masks, complex paint/stroke/child silhouettes and anisotropic blur are
  counted once per effect. No-fill nodes retain the alpha-preserving first-drop filter,
  counting unsupported spread/blend and every skipped later drop or inner shadow. Shadow
  attempts use the work budget, with at most eight per node admitted. Synthetic pixel,
  counter and mutation checks cover clipping, union, transparency, order, blur and spread.
  Not verified: live Figma parity or exact rendering of the counted approximations.

### 2026-10-11T141129Z b040-a18-a-uniformly-scaled-instance-uniformscalefactor
- 2026-10-11 — A18: a uniformly scaled instance (`uniformScaleFactor`) now also scales the
  size and offset that a nested instance states in its own unscaled derived data; before,
  only the innermost scale applied to omitted offsets and sizes were never scaled, so stored
  outlines drawn at the scaled size sat off base-shape discs in tooth-chart-like icons.
  Test in `instance_overrides.rs`. Not verified: live compare in Figma.

### 2026-10-11T141129Z b041-a18-review-fix-place-omitted-constrained
- 2026-10-11 — A18 review fix: place omitted constrained translations before inherited
  scaling; preserve explicit placement and auto-layout flow. Scale inherited stored glyphs
  and stroke weights by their field owners, keeping outer-stated values untouched; remaining
  inherited ink approximations are counted in status. Synthetic red/green and ownership
  mutation tests, workspace/wasm gates, PII scan and local render comparisons passed.
  Not verified: live Figma comparison, fallback font or stored-outline recomputation.
