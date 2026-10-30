---
pr: 50
branch: fix/boolean-labels
title: fix(render,app,web): draw boolean results, Figma-style canvas labels, browser text layout
---

## P1 — Renderer (`crates/render`)
- [x] browser text without stored outlines uses supplied UI/CJK/symbol fonts on workers

## Progress

### 2026-10-21T000000Z b292-boolean-operations-paint-their-stored-result
- 2026-10-21 — Boolean operations paint their stored result with their own fills, strokes and effects, not their operand children; missing usable geometry falls back to operands and increments `approximated_booleans` in CLI, app, MCP and diff reports. Synthetic tests cover all four operations, nested operands, stroke-only results, selection, recorded tiles and masks; the two bug regressions failed on the original renderer. All seven public fixture renders are pixel-identical (none contains booleans); inspected before/after thumbnail sheets. No private-file or live Figma verification.

### 2026-10-21T000000Z b293-canvas-label-blockers-now-test-the
- 2026-10-21 — Canvas label blockers now test the actual transformed frame quad after the AABB pre-filter, keeping labels in rotated neighbours' empty corners while hiding true overlaps at normal and low zoom. Collision and hidden-name find-outline tests pass; old AABB logic and disabled collision/fallback mutations fail the corresponding regressions.

### 2026-10-21T000000Z b294-canvas-label-overlap-now-uses-a
- 2026-10-21 — Canvas label overlap now uses a screen-space grid, one propagated transform per candidate and section subtree ranges instead of ancestor walks; paint and find reuse each candidate's shaped layout. A 5,000-frame/50-section optimized benchmark preserves ordered label IDs at three zooms; negative controls catch repeated shaping, full scans and missing section exemptions.

### 2026-10-21T000000Z b295-boolean-stored-results-now-require-non
- 2026-10-21 — Boolean stored results now require non-empty geometry for every visible paint kind; incomplete results fall back to operands and count an approximation. Synthetic missing/empty geometry and hidden-paint regressions pass; all seven public fixture renders remain pixel-identical.

### 2026-10-21T000000Z b296-canvas-component-and-component-set-labels
- 2026-10-21 — Canvas component and component-set labels use the palette purple and Component icon; instance labels use the Instance icon. Colour selection follows the page background, not the app appearance; plain frame labels use readable grey (179 on dark pages) with a WCAG 4.5:1 fallback. Shared label layout reserves icon width for ellipsis and find highlights; section pills are unchanged. Original-code regressions caught grey component text and insufficient frame contrast; removing icon painting also fails the painter test. Inspected before/after headless WGPU label captures on light/dark synthetic pages. Native-window and live Figma checks were not run.

### 2026-10-21T000000Z b297-enabled-shared-fallback-layout-from-browser
- 2026-10-21: Enabled shared fallback layout from browser-supplied font bytes, with tile/recording and Recents refresh. Synthetic delayed-font CPU/GPU checks passed; registration and stale-tile mutation controls failed as intended. All seven native public fixture PNGs stayed byte-identical. No deployment, private-file or allocation-profile verification.

### 2026-10-22T000000Z b298-pr-50-review-threads-4343663378-4343663983
- 2026-10-22 — PR #50 review threads (4343663378, 4343663983, 4343664002, 4343664005, 4343664007): a boolean mask with missing or partly decoded `fillGeometry` now counts in `approximated_booleans`; `strokeGeometry` is decoded only for booleans and stroked nodes again, so a stale blob on an unstroked node no longer counts as a skipped paint; a SOLID paint with colour alpha 0 counts as hidden for the boolean decision (`effects::paint_has_alpha`); hit testing does not reach a boolean's operands outside the boolean's box; a canvas label's backdrop is the innermost section that the label itself lies in (a frame at a section's top edge has its label above the section), and a section's layer opacity is folded into its surface. Checked: each new test (boolean mask counted, unstroked stale stroke geometry, alpha-0 stroke paint, operand outside the box, label at a section's top, see-through section) failed on the previous head and passes now. Not checked: a stroke-only boolean used as a mask (masks still clip to fill geometry for every kind), macOS, the browser build.

### 2026-10-22T000000Z b299-pr-50-self-review-a-fallback
- 2026-10-22 — PR #50 self-review: a fallback boolean (fill result missing) kept no stroke, though a whole stored `strokeGeometry` is the result's outline and nothing else draws it; it now draws. An unstroked boolean decodes `strokeGeometry` without counting a stale blob as a skipped paint. The relay's MCP calls on the browser UI thread took the workers' blocking font lock (a main thread cannot wait on an atomic), once per tab; they now use the UI thread's copy from the last sync, once per call. Supplied faces were in `font_data`'s name order, so a monospace or icon face came before the proportional ones in the render fallback; `canvas_font_order::by_priority` follows the families (proportional first). Checked: the stroke and stale-stroke tests fail on the previous head and pass; the order test; browser-build clippy. Not changed, by choice: the label blockers' 64-point margin (a far-off-screen frame can let a wide label show until it pans in), the second copy of each supplied font in the snapshot, font changes dropping every tile, and the `Option` layout fallback in labels.

### 2026-10-22T000000Z b300-relay-rendering-now-registers-the-canvas
- Relay rendering now registers the canvas supplied-font snapshot; a no-system-font MCP draw regression proves visible outline-less text and the font counter.

### 2026-10-22T000000Z b301-empty-stored-boolean-paths-and-empty
- Empty stored boolean paths and empty geometry arrays are trusted when every entry decodes; absent, wrong-type and partially malformed results still draw operands and count an approximation. Synthetic red/green regressions cover both paint kinds.

### 2026-10-22T000000Z b302-supplied-font-registration-retains-shared-face
- Supplied font registration retains shared face bytes in each renderer cache and preserves the selected collection index through canvas and relay snapshots. Pointer identity fails before repair and passes for four renderers; collection and invalid-index tests pass. Peak allocation profiling was not run.

### 2026-10-22T000000Z b303-label-contrast-now-composites-the-opacity
- Label contrast now composites the opacity-weighted average of gradient stop colours over enclosing backdrops; image fills retain the existing backdrop. Dark-section label and transparent-stop regressions failed before repair and pass for all four gradient kinds. Blocked-label shaping keeps egui's existing galley cache rather than adding a second cache.

### 2026-10-22T000000Z b304-collapsed-frame-quads-now-block-labels
- Collapsed frame quads now block labels as segments or points; the broad phase retains zero-extent bounds and the narrow phase skips zero axes. Both broad/narrow regressions failed first, while strict edge-touch semantics and rotated-frame tests remain passing.

### 2026-10-22T000000Z b305-empty-stored-boolean-masks-now-install
- Empty stored boolean masks now install an empty clip instead of skipping clipping. A synthetic mask regression failed with leaked sibling pixels and now passes for empty paths and arrays in direct and recorded rendering.

### 2026-10-22T000000Z b306-a-boolean-without-a-stored-fill
- A boolean without a stored fill result now draws its whole stored stroke above the operands that stand in, and hit testing clips to its box only when it draws the stored result, so operands painted past the box stay clickable. A frame label's colour is sampled at the middle of the drawn label (font row height, icon and pill padding) instead of a fixed 12 points. All three regressions failed first.

### 2026-10-22T000000Z b307-one-predicate-clips-hits-now-decides
- One predicate, `clips_hits`, now decides whether a boolean is confined to its box, for page hit testing, culling and picking inside instances: only a boolean that draws its stored result is. A fallback is no longer culled by its box, and inside an instance a stored result's operand past the box is no longer picked. Hit testing decodes stored geometry only for a point outside the box. Both new regressions failed first.

### 2026-10-22T000000Z b308-page-bounds-fit-thumbnails-tile-cover
- Page bounds (fit, thumbnails, tile cover) also use `clips_hits`, so a stored-result boolean's operands no longer widen them. Culling decodes a boolean's geometry only when its box misses the output. A test now pins that a stored result is culled by its box.

### 2026-10-23T051551Z b309-pr-50-change-request-without-installed
- 2026-10-23 — PR #50 change request: without installed fonts the supplied faces are the text fallbacks in the order supplied (Noto Emoji no longer sets browser line height ahead of Ubuntu-Light); a font arrival repaints only the tab's own Recents card (path and handle); `check.sh --test` runs the supplied-font render test (`tests-browser-fonts`); a stored-result boolean clips inside/outside strokes to its result; label height and gap shared; IMPLEMENTATION.md updated. Each new test failed before its fix.
