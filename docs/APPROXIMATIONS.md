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
| shadow effects approximated | 5349 (3567 after fix #1) | 24 |
| effects skipped | 483 (533 after fix #1) | 22 |
| instance ink approximated | 436 | 10 |
| stroke alignments approximated | 245 | 9 |
| paints skipped | 189 | 2 |
| text truncation ignored | 42 | 3 |

## What is behind each counter

### C1. shadow effects approximated (3567 after fix #1 and its review fixes; 5349 before)

Code: `effects::is_approximate`, `effects::approximate_silhouette` (`crates/render/src/effects.rs`),
`shadow_layer.rs` (whole-node layers).

**Fix #1 (done):** a zero-spread, normal-blend drop shadow with a uniform transform is cast
from the node's whole rendered alpha (fills, strokes, children) when the fill shape is not
exact. One such shadow wraps the content in one `DropShadow` layer. Several shadows draw the
content once per shadow into a `DropShadowOnly` layer (in list order, behind) and then the
real content once, so a shadow is never cast from another shadow; the redraws are charged
to the work budget and their counters are discarded. When a node mixes whole-node shadows
with fill-mask ones (spread...), every drop shadow is drawn behind in list order, so the
paths do not reorder them; the redraw keeps the node's inner shadows. Plain opaque fill-shape nodes keep the
old path, pixel-identical. Guards (all fall back to the old path and stay counted):
rendered-alpha area over `MAX_SHADOW_PIXELS`, more than `MAX_WHOLE_SHADOWS` shadows, a subtree
over `MAX_REPLAY_DESCENDANTS` nodes, the render's total layer budget, or redraw work over its
runtime allowance. Everything below is enforced at run time, not predicted.

*Memory:* a layer's pixels are the box of everything the node actually draws: its box and
ink (never trimmed by its own clip), its decoded fill paths, its stored stroke outlines and
glyphs (path blobs boxed once per render without building the path), and, unless it clips,
its descendants' the same way. A clipping node bounds everything below it, instances and
overrides included. Otherwise any instance in the subtree, any override or scale applying to
the node, a vector known only by its network, text laid out from system fonts, or over 256
nodes means the whole target stands in (no list of override keys to keep in sync; provably an
upper bound). The area is clamped to the render target grown by the blur reach: Vello draws a
filter layer into the target plus the filter's source padding (vello_common 0.3.0
`viewport.rs:94-107`) and allocates that bbox plus the filter padding (`filter/mod.rs:156-157`),
so it cannot allocate more. Every whole-node layer pushed, nested and redraw layers included,
is charged to `MAX_SHADOW_LAYER_PIXELS_TOTAL` (64M px) at plan time and reported in
`Stats::shadow_layer_pixels`; past it the shadow falls back, counted. The browser build runs
the same walk. A fill-less node's refused shadow is skipped and counted, never drawn as a
whole-content layer or cast from the node box.

*Time:* all redraws of one node (nested ones included) share an allowance of
`REPLAY_WORK_FACTOR` x subtree size x shadows node visits, checked at every child visit. When
it runs out the traversal stops at once and the half-drawn redraw layer is erased (a `Clear`
fill over the output bounds of every path actually issued to the scene while it ran, spread by
the filters opened inside it, recorded by `track.rs`; also recorded for parallel strips), then the shadow falls back to
the fill mask if the node casts one, else is skipped and counted. Planning is charged to the
work budget.

**Still counted:** spread other than 0 (vello_cpu 0.3 has no morphology filter), non-NORMAL
blend, blur under a non-uniform scale, the guards above, and a shadow listed next to a
whole-node one that falls back. In the measured file most of the remainder is non-zero
spread; the remainder was not broken down again after the fix.

**vello_cpu quirk:** a filter layer is clipped to the canvas *before* the shadow offset is
applied, so a shadow whose source lies off the canvas edge can be missing or doubled.
Tests keep their shadows on the canvas.

### C2. effects skipped (533 after fix #1; 483 before)

Code: the effects loop in `Ctx::draw_node` (`crates/render/src/lib.rs`).

| Reason | Count |
|---|---:|
| Second and later drop shadows on a node without fills, when a whole-node layer is refused | most of 427 |
| Inner shadow on a node without a fill shape | 12 |
| Per-node cap (8), work budget, progressive or unsupported effects | 0 |

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
| 1 | Shadows from an offscreen layer of the whole node | C1, C2 (~3300 cleared) | large, measure speed | done; spread still counted |
| 2 | Scale inherited radius, dash, outlines and glyphs | C3 (436) | medium | open |
| 3 | Clip inside/outside strokes on open vectors (A14) | C4 (245) | medium | open |
| 4 | Vertex/segment styles in rebuilt network fills | C5 (189) | small | open |
| 5 | Ellipsis for truncated fallback text | C6 (42) | small | open |

Limits that may stay counted: blur under a non-uniform scale (vello_cpu blurs with one
sigma), and anything whose exact form costs too much time on large pages.

## Log

- 2026-10-11 — First measurement and breakdown recorded (this file).
- 2026-10-11 — Fix #1, whole-node shadow layers (`shadow_layer.rs`). 42 pages at
  `--scale 0.1`, CPU time (user+sys, min of two runs per binary, interleaved, load 12-30):
  110.5 s before, 110.8 s after (+0.3%) with `MAX_REPLAY_DESCENDANTS` 64 (shadows 2308);
  124.8 s / 125.3 s (+0.4%) with 512 (shadows 2084, skipped 427), the kept value; the
  second run was on a busier machine. Worst page +12% (+0.3-0.4 s); none over 25%. At
  `--scale 1` the three pages with most shadows: -1.2%, -13%, +1%. Fixture thumbnail sheet
  unchanged; two real-file crops show softer shadow edges that follow the content.
- 2026-10-11 — Review fixes for fix #1: guard on rendered alpha bounds, list order across
  paths, inner shadows in the redraw, replay-stable planning with a nested redraw cost,
  refused effects not drawn. 42 pages at `--scale 0.1`, CPU time: 115.0 s before, 116.6 s
  after (+1.4%), worst page +18.6% in the full run and +8.7% (+0.25 s) on a re-time on a
  quieter machine; none over 25%. Scale 1, three shadow-heaviest pages: +5.6%, +3%, -1.2%.
  Approximated shadows 2233, skipped effects 468 (the stricter guards refuse more than the
  first version's 2084 / 427).
- 2026-10-11 — Second review pass on fix #1: memory guard on target-bounded, ink-aware bounds
  (clip never trims own stroke; uncertain instances assume the whole target; fill-less
  fallback obeys the guard) and a run-time redraw work cap per node. `MAX_SHADOW_PIXELS`
  8M, `MAX_REPLAY_DESCENDANTS` 128 (512 made one page +31-45% on a quiet machine). 42 pages
  at `--scale 0.1`, CPU time, quiet-machine re-times merged: 107.2 s before, 108.2 s after
  (+0.9%), worst page +8.2%; none over 25%. Scale 1, three shadow-heaviest pages: +2%,
  +1.3%, +0.5%. Approximated shadows 2800, skipped 477.
- 2026-10-11 — Third review pass on fix #1, runtime enforcement: whole target as bound for any
  instance or override in scope, an aggregate layer-pixel budget (64M px, `Stats::shadow_layer_pixels`),
  a per-node redraw allowance checked at every child visit with the partial redraw erased,
  and casts-only fallback. 42 pages at `--scale 0.1`, CPU time, quiet machine: 105.2 s before,
  105.6 s after (+0.4%), worst page +5.5%; none over 25%. Scale 1, three shadow-heaviest
  pages: +2.6%, +2.1%, +0.4%. Approximated shadows 3566, skipped 533 (the instance rule is the
  price of not predicting overrides). Counted-shadow events by guard, overlapping, 42 pages:
  spread/blend/anisotropic 1717, aggregate memory 934, fill-less mixed lists 760,
  bounds uncertain on a target over the cap 465, subtree over the node cap 411, replay work 16.
  A first version cleared the discarded layer with a +-1e6 rect and cost one page +35%;
  clearing only the bounds the redraw drew removed that.
- 2026-10-11 — Fourth review pass on fix #1: shadow layers bounded by the geometry actually
  drawn. Plan-time bounds include the decoded fill paths, stored stroke outlines and glyphs
  (blobs boxed once); a clipping node bounds everything below it. A discarded redraw is erased
  over the union of the paths actually issued to the scene while it ran (`track.rs`), not over
  size-based bounds. 42 pages at `--scale 0.1`, CPU time, quiet machine: 104.9 s before,
  105.3 s after (+0.4%), worst page +5.3%; none over 25%. Scale 1, three shadow-heaviest pages:
  +1.9%, +1.2%, -0.4%. Approximated shadows 3567, skipped 533. Counted-shadow events by
  guard, overlapping, 42 pages: spread/blend/anisotropic 1717, aggregate memory 935,
  fill-less mixed lists 760, bounds uncertain on a target over the cap 465, subtree over the
  node cap 411, replay work 16. A first version returned the whole target for any instance
  before looking at the clip: counters 4660 / 855.
