---
branch: feat/prototype-preview
pr: 71
title: feat: prototype Preview in the app, browser, CLI and MCP
---

## P3 — Desktop app (`apps/figstudio`)
- [ ] Prototype Preview (⇧Space, View → Preview prototype): the flow's or selection's screen
  alone, fitted on the prototype background; "On click" navigate and back links play; ←,
  Backspace, R and Esc; a click on no link flashes the links; unplayed links counted in the bar
- [ ] Shared model `figstudio_render::prototype` (flows, screens, links, player), one draw-order
  walk per screen (`Inspect::Links`), links inside instances with overrides
- [ ] `figstudio-cli prototype` lists flows, reachable screens and links; `-o` renders them
- [ ] MCP `get_prototype`, native and through the browser bridge

## Progress

### 2026-11-04T090000Z prototype-preview
- What changed and how it was checked: Preview in the app (desktop and browser share the code), `figstudio-cli prototype`, MCP `get_prototype` (native and browser bridge), one shared model in `figstudio_render::prototype`. Tests: render (6), CLI (3), MCP protocol (2) plus the stdio, http and wasm RPC tool lists, app kittest (2). Five deliberate breaks each failed a test (Back not played, own links ignored, missing destination played, tiles drawing the page, a click not followed). fmt and workspace clippy `-D warnings` clean on nightly 1.101. The CLI render of a screen was looked at. On local private files (counts only): links inside instances match `path_bounds` for 3,036 of 3,036; a 834-link screen reads in about 0.1 s.
- Not verified: full `scripts/check.sh` (stopped for time; CI runs it); the desktop window by eye; the browser build and any browser run; GPU tests; a test run of every binary after the last fix (the http tool-count test was fixed and rerun alone).

### 2026-11-04T150000Z prototype-preview-browser
- What changed and how it was checked: browser build (`scripts/web-build.sh`, nightly-2026-10-09) driven by Playwright (`web/e2e/prototype.spec.mjs`, Chromium): Shift+Space starts Preview (`figstudioStatus.preview`), a click on no link flashes the link (the spec finds it from that flash), the link shows the second screen with "1 not played", Left arrow goes back to the first screen, Esc returns to the canvas. Passed on the rebuilt app; screenshots looked at. The run found "←" drawn as a box (the UI font has no arrow), so the button now reads "Back".
- Not verified: the spec failing without the feature (no mutated wasm build); the font step of web-build.sh (no Noto CJK here, the wasm was built before it stopped); the desktop window: the computer-use tool's input did not reach the egui window (its main loop stayed idle), so only kittest drives the desktop path.

### 2026-11-05T030000Z prototype-preview-review-fixes
- What changed and how it was checked: an adversarial review (Claude Opus and GPT-6.1, each in its own worktree) found seven problems, each rerun and confirmed here: Esc after a cross-page link lost the canvas; a click picked an unplayed action before a navigate; one layer past 5,000 links was cut without `truncated`; clipped-out layers and the corners of rotated layers were clickable; a link to a hidden frame was played (and `prototype -o` exited 1); `get_prototype nodeId` took pages and inner layers as screens and counted links twice. Fixed in `prototype.rs` (cap per link, hit by shape inside the clipping frames, first played action of the front layer, hidden destinations not played), the no-draw walk (tracks clipping frames), `Tab::stop_preview`, the CLI and `prototype_tool.rs`. Eight regression tests from the reviewers' repros: the six that existed before the fixes failed without them and pass with them. clippy `-D warnings` clean on the five touched crates.
- Not verified: full `scripts/check.sh` (CI); keyboard input while a text field has focus, switching tabs and closing the file during Preview (no reviewer reached them).

### 2026-11-05T060000Z prototype-present-stepping
- What changed and how it was checked: Figma's Present was observed on a private file (frame ids read from Figma's URL while stepping; 13 frames on one page): it steps visible frames that are not component sets, at the top level or in sections, in reading order, and starts a page without flows at its first frame. figstudio now does the same (`prototype::frames`, → / ← with a "< n / N >" counter, Backspace for Back), has Present (⌥⌘↩: the editor hidden, canvas edge to edge) and Figma's four scaling options. Tests: frame order (render), scaling math, stepping and Present hiding the editor (kittest); each of the two behaviour tests failed with its feature broken. clippy `-D warnings` clean.
- Not verified: Figma's order rule on frames whose rows do not line up exactly (only aligned rows were observed); the per-page comparison with Figma, which is next.

### 2026-11-05T100000Z prototype-present-order
- What changed and how it was checked: Figma's Present was captured on a private file, all 29 pages with Present frames (702 frames), with frame ids and Figma's own "n / N" counter read from its accessibility tree (scripts kept local, not in the repo). Figma's frame set: visible frames, not component sets and not groups (`resizeToFit`), at the top level or in sections. Order: each level is sorted by "left first when the boxes overlap or touch vertically, else top first" with a stable insertion sort from layer order; the comparison is not transitive, so the sort is part of the rule. With a fresh local copy, 26 of 29 pages match Figma's set and order exactly (24 of 27 by order alone); a search over sort algorithms, start orders and comparator variants found nothing that fits more. Regression test for the non-transitive case; it fails when the start order changes.
- Not verified: pages 10, 11 and 43 of that file still differ in order (page 10 has prototype interactions that jump between frames, which scrambles stepping); page 34's frames show no counter in Figma (not understood yet); the picture comparison (figstudio Present screenshots against Figma's) has not run.

### 2026-11-05T140000Z prototype-present-pictures
- What changed and how it was checked: figstudio's Present was captured on the same private file and compared with Figma's, pair by pair (scripts kept local). That found: Figma's Fit keeps 48 pt at the sides and 60 pt at the top and bottom and never enlarges (Fit width neither); Present is black where the page sets no prototype background; Present fitted the first screen before the panels were gone; and the renderer applied a bound colour variable's alpha on top of the paint opacity (a 40% black overlay drew as 16%), which also affected the canvas, CLI and MCP renders. Each fixed with a test that fails without the fix (the refit only on screen: the test harness lays out the panels at once). clippy `-D warnings` clean.
- Not verified: the full set of picture pairs (running next).

### 2026-11-06T000000Z prototype-present-comparison-result
- What changed and how it was checked: Preview and Present cut each screen at its frame, as Figma's prototypes do, even when the frame does not clip its content (two pairs whose content overflowed: 0.271 -> 0.006 and 0.118 -> 0.025). Full picture comparison on the private file, figstudio's Present against Figma's, 701 pairs: 464 below 0.03 RMSE and 236 at 0.03-0.06 (looked at: same picture, text antialiasing), one above 0.2 (a Figma capture taken after a prototype interaction jumped to another screen). clippy `-D warnings` clean.
- Not verified: the frame clip has no automated test (it is a painter clip; checked on screen through the pairs). Open renderer differences seen in one pair: mixed text colour ranges and text on a highlight. Order differences on pages 10, 11 and 43 of that file remain.

### 2026-11-06T060000Z prototype-interactive-components
- What changed and how it was checked: Preview and Present play navigate, back and "change to" (Figma's interactive components, about 95% of the not-played links on the private file) on click, hover, press, mouse enter/leave/down/up, after delay and drag. `Player` takes pointer events and time; "while hovering/pressing" swaps are undone when the pointer leaves the layer's shape or is released; the renderer draws the swapped variant's look where the instance did not override it; each prototype state has its own tile document number. Found and fixed while testing: the hover state flickered (the hover variant has no hover link of its own), and a navigate did not refit the new screen. Tests: render 20, app 7; each new test fails with its feature disabled. clippy `-D warnings` clean; the five touched crates' tests pass.
- Not verified: hover states against Figma on the real file; overlays, scrolling and transitions are not played yet (listed and counted).
