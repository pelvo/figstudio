# Approximation counters: what each one means and how to clear it

The status bar and the CLI report counters for things that are drawn approximately or not
drawn at all (AGENTS.md rule 6, "unsupported means counted, not hidden"). This file keeps a
record of what each counter is made of, so the work to clear it can be tracked.

The numbers come from a large private real-world file (42 pages, ~170k layers). They were
measured with a temporary debug build that printed the reason at each counter site; the
build was thrown away. Re-measure after each fix and update the table and the log below.

## Measurement (2026-10-11, branch `fix/known-gaps` at `4e36127`)

Totals over all 42 pages, `figstudio-cli render --page N --scale 0.1`:

| Counter (status bar text) | Total | Pages |
|---|---:|---:|
| shadow effects approximated | 5324 | 24 |
| effects skipped | 483 | 22 |
| instance ink approximated | 436 | 10 |
| stroke alignments approximated | 245 | 9 |
| paints skipped | 189 | 2 |
| text truncation ignored | 42 | 3 |

## What is behind each counter

### C1. shadow effects approximated (5324)

Code: `effects::is_approximate`, `effects::approximate_silhouette` (`crates/render/src/effects.rs`).

| Reason | Count |
|---|---:|
| Shadow cast from the fill shape, but the node has children, strokes, style-override fills or translucent/gradient/image fills | 4873 |
| Positive spread on the fallback path (node without a fill) | 376 |
| Negative spread | 100 |
| Blend mode, compound inner shape, anisotropic blur | 0 |

Silhouette reasons seen on shadow-casting nodes (a node can have several): children ~96k,
strokes ~107k, style-override fills ~44k, translucent or non-solid fills ~5k.

**Exact fix:** draw the node (fills, strokes and children) into an offscreen layer and build
the shadow from that layer's alpha. vello_cpu 0.3 has `FilterPrimitive::DropShadowOnly`
(shadow of a layer's alpha without the layer). Spread needs an alpha dilate/erode. Inner
shadow needs the inverted alpha clipped to the layer. Cost must be measured: about 800
shadows on one page.

### C2. effects skipped (483)

Code: the effects loop in `Ctx::draw_node` (`crates/render/src/lib.rs`).

| Reason | Count |
|---|---:|
| Second and later drop shadows on a node without fills (container frames/instances use one whole-node filter layer) | 471 |
| Inner shadow on a node without a fill shape | 12 |
| Per-node cap (8) or work budget | 0 |
| Progressive or other blurs | 0 |

**Exact fix:** the same offscreen-layer shadow as C1. It clears both counters.

### C3. instance ink approximated (436 nodes)

Code: `layout::approximate_ink` (`crates/render/src/layout.rs`). A node can have several
reasons.

| Reason | Count |
|---|---:|
| Corner radius not scaled by an outer uniform instance scale (FRAME 855, INSTANCE 764) | 1619 |
| Text with stored glyphs under an outer scale | 236 |
| Stored stroke outlines under an outer scale | 158 |
| Dash pattern under an outer scale (ellipses) | 58 |

**Exact fix:** scale corner radii and dash lengths by the outer factor. Transform stored
stroke outlines and glyphs by the inherited scale when the nested instance supplied them.

### C4. stroke alignments approximated (245)

Code: `Ctx::draw_strokes` (`crates/render/src/lib.rs`). All 245 are INSIDE strokes on open
VECTORs (gap A14).

**Exact fix:** clip the doubled stroke to the fill region the open path implies (the
vector-network regions or the closed fill path), the same way closed vectors are clipped.

### C5. paints skipped (189, 2 pages)

Code: `network::fills` (`crates/render/src/network.rs`). Seven vectors with no stored fill
geometry whose networks use per-vertex and per-segment styles (10–18 of each).

**Exact fix:** carry vertex/segment style ids into the rebuilt paths and resolve them through
`styleOverrideTable`.

### C6. text truncation ignored (42)

Code: the text path in `Ctx::draw_node` (`crates/render/src/lib.rs`). Text with
`textTruncation: ENDING` and no usable stored glyphs.

**Exact fix:** in the system-font layout fallback, cut the last visible line and add "…".

## Plan

One fix at a time (they all touch `crates/render/src/lib.rs`), each with a synthetic test
seen failing first, a review by a different model family, and a re-measurement here.

| # | Fix | Clears | Size | Status |
|---|---|---|---|---|
| 1 | Shadows from an offscreen layer of the whole node | C1, C2 (~5800) | large, measure speed | open |
| 2 | Scale inherited radius, dash, outlines and glyphs | C3 (436) | medium | open |
| 3 | Clip inside/outside strokes on open vectors (A14) | C4 (245) | medium | open |
| 4 | Vertex/segment styles in rebuilt network fills | C5 (189) | small | open |
| 5 | Ellipsis for truncated fallback text | C6 (42) | small | open |

Limits that may stay counted: blur under a non-uniform scale (vello_cpu blurs with one
sigma), and anything whose exact form costs too much time on large pages.

## Log

- 2026-10-11 — First measurement and breakdown recorded (this file).
