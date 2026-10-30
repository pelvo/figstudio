---
pr:
branch:
title: items added before pull requests were used
---

## P0 — Decoder (`crates/fig`)
- [x] zip and raw containers, deflate / zlib / zstd chunks
- [x] schema read from the file itself (format v20–v106 all decode)
- [x] owned `Val` tree, `Node`, parent/child order by fractional index, pages vs internal pages
- [x] embedded images, `meta.json` file name, stored thumbnail, 1 GiB size cap
- [x] tests: 7 fixtures decode, node counts match the TypeScript decoder, bad input rejected

## P1 — Renderer (`crates/render`)
- [x] fill / stroke geometry blobs, base shapes (rect, rounded rect, ellipse)
- [x] solid, linear, radial, angular paints; diamond approximated as radial
- [x] image fills (stretch / fill / fit / tile), missing image placeholder
- [x] frame clipping, masks, opacity layers
- [x] instances: expand symbol, apply `symbolOverrides` + `derivedSymbolData`, cycle guard
- [x] text: stored glyph outlines; system-font layout fallback (wrap, align, line height)
- [x] drop shadow and layer blur via Vello filters; other effects counted as skipped
- [x] **visual check against Figma thumbnails** for all 7 fixtures, fix what is wrong
      (fixed: path blobs that continue after a close; vector `styleOverrideTable` colours)
- [x] render tests: every fixture draws, vector style colours, stored-glyph text, instances,
      clipping, hit test, hidden layers, malformed blobs (8 tests)
- [ ] progressive blur (Figma `blurOpType: PROGRESSIVE`) — skipped and counted for now
- [ ] noise / grain / texture effects — skipped and counted
- [ ] blend modes, inner shadow, background blur (currently skipped)
- [ ] gradient transforms verified on a file with rotated gradients

## P1b — Large files (requester's file cannot be shared)
- [x] round-trip writer (needed for P6 anyway): `Document` keeps schema, codecs, extra
      chunks, other message fields and archive entries; `Val` is encoded straight from the
      schema. All 7 fixtures encode to **byte-identical** Kiwi messages (Figma writes
      `nodeChanges` and `blobs` last; we do the same)
- [x] synthetic stress files: `figstudio-cli synth` copies the first page ×N (new ids,
      grid layout, optional extra pages). 103k / 115k / 76k node files built and checked
- [x] measure and fix hot spots:
      - decoder now reads Kiwi straight into `Val` (no `kiwi_schema::Value` tree, no hash
        map per object); field and enum names interned; objects are small vectors
      - viewport culling for leaves, clipping frames/instances and boolean ops, with a
        generous overhang for strokes, effects and text; sub-quarter-pixel leaves skipped
      - `FIGSTUDIO_PROFILE=1` prints UI and render times per frame
- [x] `figstudio-cli doctor <file>`: counts, Figma type/field names, unsupported features,
      timings, memory, write-back check. Never prints names, text, fonts, image hashes,
      colours or paths; errors scrubbed; panic hook hides payloads. Tested on all fixtures
      against every layer name, text, font, image hash and path (and the test was checked
      to fail when a name is printed)

## P2 — Headless CLI (`apps/figstudio-cli`)
- [x] `info`, `tree --depth`, `render --page/--node/--scale/--background`, `thumbnail`
- [x] renders all 7 fixtures (3–366 ms each, release build)
- [ ] `render --all-pages` and JSON output for `info`

## P3 — Desktop app (`apps/figstudio`)
- [x] theme and layout in the PhotoCraft style: menu bar, left tool rail, document tabs,
      canvas, docks, status bar (Pages + Layers on the left, Properties on the right)
- [x] open via menu, file dialog (rfd), drag and drop, command-line path; several tabs
- [x] canvas: pan (drag with Hand / Space / middle button, scroll), zoom (Ctrl+wheel,
      pinch, +/-, Zoom tool, Alt to zoom out), ⇧1 fit, ⇧0 100%, double-click to fit selection
- [x] re-render in a background thread; the last frame is re-projected until the new one lands
- [x] click to select (hit test), selection and hover outlines (rotation aware)
- [x] Layers tree: expand/collapse, hidden layers greyed, components purple, search box,
      selecting on canvas expands and scrolls the tree to the layer
- [x] Properties: type, id, X/Y/W/H, rotation, radius, clip, auto layout, opacity, blend,
      fills with swatches, strokes, effects, text and font, instance main component, overrides
- [x] export page or selection to PNG at 1x / 2x / 3x (Ctrl+Shift+E = selection @2x)
- [x] CJK-capable UI font (first installed Noto CJK / Source Han / PingFang / Hiragino)
- [x] status bar: zoom, cursor position, layers drawn, render time, unsupported features
      (since the Arc redesign: the tool pill shows the zoom, the status pill the rest, a
      warning chip the unsupported features)
- [x] errors shown as toasts (bad file stays non-fatal)
- [ ] save dialog path checked by hand on a desktop (Xvfb has no portal, so the export
      menu was checked but the dialog itself was not exercised)
- [ ] check on a real Wayland session and with a big file

## P4 — Platforms
- [ ] Linux: release build runs on X11 and Wayland
- [ ] macOS: `cargo check --target aarch64-apple-darwin` here; build steps in README;
      requester builds and launches on a Mac by hand when free

## P5 — Delivery
- [ ] README (build, run, CLI usage, known gaps) — task T2
- [x] agent guide: `AGENTS.md`, `docs/IMPLEMENTATION.md`, `docs/TESTING.md`; scripts
      `check.sh`, `thumb-sheet.py`, `gui-smoke.sh` (all three run here)
- [x] `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` clean (1.99.0)
- [x] repo location: private `pelvo/figstudio` (requester asked, 2026-10-09); `main` pushed

## P6 — Editing (later, not started until asked)
- [ ] command/undo model over `Document`
- [ ] move/resize, rename, show/hide, reorder layers, edit text, edit fills
- [ ] save `.fig` (re-encode message, keep other archive entries), verify Figma opens it

## Progress

### 2026-10-08T131732Z b018-2026-10-08-21-5x-rust
- 2026-10-08 21:5x — Rust spike: kiwi-schema decode + tiny-skia render of 7 fixtures.

### 2026-10-08T131732Z b020-2026-10-08-22-1x-workspace
- 2026-10-08 22:1x — Workspace created. P0 done (5 tests pass). P1 renderer compiles with
  instances, clipping, masks, images, text, shadows. CLI renders all 7 fixtures; output not yet
  visually checked. GUI not started. Plan committed (92a5207).

### 2026-10-08T132308Z b019-2026-10-08-22-0x-requirements
- 2026-10-08 22:0x — Requirements fixed (macOS + Linux + headless, wasm optional, Vello,
  PhotoCraft-style UI). Switched renderer to `vello_cpu`.

### 2026-10-08T132308Z b021-2026-10-08-22-2x-requester
- 2026-10-08 22:2x — Requester: editing may come later, so not "view" → renamed project to
  `figstudio` (working name). Mac tested by hand by requester. Large private file cannot be
  shared → added P1b (round-trip writer, synthetic stress file, privacy-safe `doctor`) and P6.
  `kiwi-schema` has `Value::encode_bb`, so writing `.fig` back is possible.

### 2026-10-08T133210Z b022-2026-10-08-22-3x-p1
- 2026-10-08 22:3x — P1 visual check. Side-by-side sheet of 7 fixtures vs thumbnails.
  Fixed path blobs that draw after a close (text in noise_and_grain was invisible) and vector
  per-path colours from `styleOverrideTable`. Progressive blur skipped instead of a wrong
  uniform blur. 8 render tests + 5 decode tests pass. Remaining P1 items are effect gaps.

### 2026-10-08T134531Z b023-2026-10-08-22-4x-p3
- 2026-10-08 22:4x — P3 desktop app built (egui 0.36 / glow). Checked in Xvfb with xdotool
  clicks and screenshots. CLI now prints usage only for usage errors. Image cache no longer
  caches "missing" across documents. clippy + fmt clean on 1.99.0, 13 tests pass.

### 2026-10-09T000943Z b024-2026-10-09-09-xx-p1b
- 2026-10-09 09:xx — P1b. Writer with byte-identical round trip on all fixtures; direct
  Kiwi→Val decoder (memory 2.8 GB → 470 MB, decode 2.0 s → 0.3 s on 103k nodes); culling;
  `synth`, `resave`, `doctor` commands; doctor privacy test. 23 tests, clippy/fmt clean.

### 2026-10-09T003257Z b025-2026-10-09-09-2x-pushed
- 2026-10-09 09:2x — Pushed to private `pelvo/figstudio` (42d40d9). Requester asked for a test
  and implementation guide other agents can follow: added `AGENTS.md` (+ `CLAUDE.md`),
  `docs/IMPLEMENTATION.md`, `docs/TESTING.md` and `scripts/check.sh`, `thumb-sheet.py`,
  `gui-smoke.sh`; all three scripts run here. Found `figstudio-cli` had no `[lints]`, so the
  workspace lints never applied to it: added them, changed `unsafe_code` forbid → deny with
  one local allow for `getrusage`, checked that unsafe elsewhere fails the build.
