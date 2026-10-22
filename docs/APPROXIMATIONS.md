# Approximation counters: what each one means and how to clear it

The app (the warning chip on the canvas) and the CLI report counters for things that are drawn approximately or not
drawn at all (AGENTS.md rule 6, "unsupported means counted, not hidden"). This file keeps a
record of what each counter is made of, so the work to clear it can be tracked.

The numbers come from a large private real-world file (42 pages, ~170k layers). They were
measured with a temporary debug build that printed the reason at each counter site; the
build was thrown away. Re-measure after each fix and update the table and the log below.

## Font fallback

`text_system_fonts` counts text laid out from font bytes rather than stored glyph
outlines. Native uses installed fonts; the browser uses its existing UI, CJK and
symbol faces. Both use the same approximate layout. `text_missing` still counts
text for which neither stored outlines nor usable fallback fonts are available.
Browser font arrival discards cached tiles and recordings and refreshes Recents.

## Measurement (2026-10-11, branch `fix/known-gaps` at `4e36127`)

Totals over all 42 pages, `figstudio-cli render --page N --scale 0.1`:

| Counter (warning chip text) | Total | Pages |
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

### Boolean operations approximated (not measured in the historical sample)

Code: `Ctx::draw_with_data` and `Ctx::draw_children` (`crates/render/src/lib.rs`),
`Stats::approximated_booleans`.

A boolean with no usable stored fill or stroke geometry draws its operand children instead
of a computed result. Each such drawn boolean counts once. A boolean mask with missing or
partly decoded `fillGeometry` also counts once: it clips to what decoded, or not at all. With stored geometry, only the
result is painted with the boolean's own fills, strokes and effects; operands remain
available for selection inside the boolean's box and for bounds. The CLI, warning chip and MCP screenshots report the
fallback, and tile summaries preserve it.

**Exact fix:** compute the requested boolean operation from its operands when the file has
no usable result geometry. The current fallback does not attempt boolean path arithmetic.

### C6. text truncation ignored (42)

Code: the text path in `Ctx::draw_node` (`crates/render/src/lib.rs`). Text with
`textTruncation: ENDING` and no usable stored glyphs.

**Exact fix:** in the system-font layout fallback, cut the last visible line and add "…".

### C7. emoji approximated / emoji skipped

Code: `text::stored_glyphs`, `Ctx::emojis`, `Ctx::draw_placed_emojis`, and
`emoji_font::resolve` (`crates/render/src`).
Counts are per visible stored emoji glyph, not per text node; not included in the older totals above.
`emojiCodePoints` marks an editor placeholder, so its stored square is never painted.

- `emoji_approximated`: an outline-only face (including flattened COLR layers) drawn with the
  text paint, a multi-code-point sequence drawn using its first base scalar, or an injected
  raster whose source cannot attest complete-cluster shaping. Limitations count once per
  glyph. COLR palette, clips and compositing are not reproduced. Browser counts are
  conservative: Canvas cannot identify the font/glyphs used, so even correctly drawn emoji
  remain counted rather than silently accepting tofu or partial cluster fallback.
- `emoji_skipped`: no usable installed face, invalid marker, unsupported or malformed strike,
  or a build without `system-fonts` and without an injected `EmojiSource`. Empty or
  transparent platform rasters are skipped. Skipped glyphs leave no box; truncated-away
  markers do not count. The browser's lazy monochrome font is UI-only.
- Native colour PNG strikes use `ttf-parser` for sbix/outline lookup, Fontations
  `read-fonts` for a single walk of CBDT/CBLC, bdat/bloc and EBDT/EBLC size records,
  and `image` for PNG decoding, bounded to
  1024 pixels per side and 8 MiB decoder allocation; encoded strikes are capped at 4 MiB.
  Native caches retain only font/strike metadata and lazily decoded images, not copied PNG
  payloads. Character hits refresh LRU recency; decoded strikes have bounded retention too,
  so an over-budget font cannot force whole-character reallocation on every frame.
  Placement uses strike pixel origin and ppem at the stored font size and baseline. sbix
  adds the contour's `(xMin, yMin)` in design units; contourless sbix and CBDT do not.
  Draws retain node transforms, parent clips and paint/node opacity. Emoji shadows use a
  counted rectangular ink bound rather than the bitmap's alpha silhouette.
  Recordings replay the same image paint; GPU-unsupported image tiles fall back to CPU and
  increment the existing `gpu_fallback_tiles` counter.
- The browser app injects a platform-free `EmojiSource` (`emoji_source.rs`) through its
  renderer factory, used by the canvas worker and PNG exports. `emoji_canvas.rs` creates
  OffscreenCanvas and its 2D context on the calling worker; JS handles never cross threads.
  It draws full stored clusters (ZWJ/variation selectors included), using measured baseline
  ink bounds and advance. Oversized boxes are remeasured at a smaller ppem and scaled at draw
  time; absent ink metrics use font bounds or an em square. Successful rasters are drawn,
  but counted unless the source explicitly attests verified shaping (`with_verified_shaping`).
  Browser Canvas provides no such attestation; pixel/tofu comparisons cannot prove support.
  Straight-alpha RGBA is premultiplied once. Renderer-local LRU caches bound results to 256
  entries and 16 MiB of pixels. Sources declare a canonical request size so capped rasters
  serve all larger zooms. Misses get one immediate retry, then one retry per 64 later cache
  calls; negative entries share the same LRU bound. Oversized results draw uncached without
  evicting hot entries.
  Keys and raster dimensions are bounded too. Recordings retain owned pixels and need no
  browser API on replay.

The warning chip, copied-image warning, CLI render/doctor counts, CLI info emoji summary,
MCP screenshot notes and diff JSON carry these counters. `Stats::max_with` retains them
across tiles. As with other render counters, work-budget truncation makes totals lower bounds.

**Remaining work:** shape complete sequences natively and support native colour-outline
formats. Browser shaping and colours depend on the browser's installed emoji fonts;
unsupported clusters can still produce the platform's fallback glyph. UI-only monochrome
fallback does not clear canvas counters. Browser UI names now get four lazy monochrome outline subsets,
validated before installation and kept after CJK fallbacks; their combined size is 1,705,404
bytes for the checked source fonts. Cross-platform colour faces and real COLR fonts remain
to be checked.

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
