# figstudio — implementation plan

A Rust app for Figma `.fig` local copies. Version 1 opens and displays files; editing may
come later, so the design must not block it. (`figstudio` is a working name.)

This file is the progress tracker. Tick a box only after its **Done when** check has
actually passed, and add a line to the progress log at the bottom.

Working on it? Start with `AGENTS.md`, then `docs/IMPLEMENTATION.md` (architecture, recipes,
task board) and `docs/TESTING.md` (tests, checks, how to prove a test).

## Requirements (from the requester, 2026-10-08)

| Requirement | Priority | How |
|---|---|---|
| Open and display `.fig` files | must (v1) | `crates/fig` decoder + `crates/render` + `apps/figstudio` |
| Editing and saving `.fig` | later, maybe | keep the document round-trippable from day one (P6) |
| macOS and Linux desktop | must | eframe (egui 0.36); macOS tested by hand by the requester |
| Headless use | must | `apps/figstudio-cli` (info / tree / render / thumbnail / doctor) |
| Browser (wasm) | nice to have | same crates built for `wasm32-unknown-unknown`; eframe web shell |
| Vello renderer | accepted | `vello_cpu` 0.3 (CPU, runs headless and in wasm); GPU `vello` is a later option |
| UI similar to PhotoCraft | accepted | same toolkit (egui 0.36) and layout; no PhotoCraft code copied |
| Large private files | must work | requester cannot share them: synthetic stress file here, `doctor` report there |

Out of scope for v1: editing, Figma API / login, prototyping/interactions, variables and
styles panels, plugins.

## Architecture

```
crates/fig            .fig container + Kiwi schema decode -> Document (nodes, tree, blobs, images)
crates/render         Document -> Vello CPU drawing -> Pixmap; hit test, bounds, render stats
apps/figstudio-cli    headless: info, tree, render PNG, thumbnail, doctor
apps/figstudio        desktop app (eframe); later also the wasm web shell
tests/fixtures        7 sample .fig files (from fig2sketch, see LICENSE-fig2sketch)
```

Rules:
- `crates/fig` and `crates/render` do no file-system or UI work beyond `Document::open`, so
  they build for wasm.
- Never panic on bad input: every malformed file returns `FigError`.
- Anything not drawn exactly is counted in `Stats` and shown to the user (app status bar, CLI).
- Edit-ready: the document keeps everything needed to write the file back (schema, all
  node fields, blobs, other archive entries). The UI reads the document through one
  model so edit commands can be added later without restructuring.

## Phases

### P0 — Decoder (`crates/fig`)
- [x] zip and raw containers, deflate / zlib / zstd chunks
- [x] schema read from the file itself (format v20–v106 all decode)
- [x] owned `Val` tree, `Node`, parent/child order by fractional index, pages vs internal pages
- [x] embedded images, `meta.json` file name, stored thumbnail, 1 GiB size cap
- [x] tests: 7 fixtures decode, node counts match the TypeScript decoder, bad input rejected

**Done when:** `cargo test -p figstudio-fig` passes. ✅ 5/5 on 2026-10-08.

### P1 — Renderer (`crates/render`)
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

Known differences that are **not** bugs: Figma thumbnails show the canvas background
(black in these fixtures) while we use the page colour; `broken_images.fig` really has
7-byte placeholder images, so grey placeholders are correct.

**Done when:** every fixture renders without obvious differences from its thumbnail
(side-by-side sheet reviewed), and `cargo test -p figstudio-render` passes.

### P1b — Large files (requester's file cannot be shared)
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

**Numbers on the 103k-node stress file** (this machine, release build):

| | before | after |
|---|---|---|
| decode | 1.98 s | 0.30 s |
| memory after open | 2.8 GB | 470 MB |
| render whole page (fit) | ~0.6 s | 0.40–0.45 s |
| render 1400×880 at 100% | whole page | 6–11 ms (9 000 layers culled) |
| encode back | — | 0.75 s, identical |
| GUI frame (9 000 rows in Layers) | — | ~11 ms median, 19 ms max |

Still open: fit-to-page of a huge page takes ~0.4 s per frame (a level-of-detail cache or
tiling would fix it); the Layers panel is not virtualised yet (fine at 9 000 rows).

- [x] **Image-heavy files bigger than the Wasm memory** (requester's 1.7 GB file, 194k
      layers, 1 893 images = 1.5 GB of the file). Before: the browser read the whole file
      into the 4 GiB Wasm memory, kept every image's bytes and decoded them at full size,
      so it ran out of memory (desktop needed 4.7 GB to open and grew to 8 GB rendering).
      Now:
      - `fig::archive` reads the zip piece by piece (end record, central directory, zip64,
        stored and deflated entries); `Document::open_lazy` / `from_lazy` read only the
        canvas, `meta.json` and the thumbnail, and list images in `lazy_images`. A lazily
        opened document cannot be saved (the writer refuses).
      - `render::images`: decoded images are cached per (image, level), level n being the
        image shrunk 2ⁿ times, at the level the view needs; Figma's `imageThumbnail`
        (≤ 512 px) is drawn when the image is ≤ 256 px on screen. 1 GiB budget, least
        recently drawn dropped first. The choice depends only on the paint and the view,
        so a frame never depends on what was decoded earlier (before, page 7 of the file
        rendered differently on the second render).
      - Frames list unread images in `Frame::wanted`; the app reads them on workers (four
        at a time) and renders again as they arrive; `render_complete` reads them inline
        (exports). The browser keeps the `File` on the main thread and reads slices with
        `Blob.slice().arrayBuffer()`; files over 256 MB open this way, smaller ones as before.
      Result on the requester's file (numbers only): browser opens in 1.4 s at 537 MB of
      Wasm memory; all 9 pages render with no missing images (each settles in ≤ 1.1 s,
      e.g. 532 images read for one page); zooming to 260% peaks at 906 MB. Desktop opens in
      0.7 s at 672 MB. Lazy and whole renders of every page match pixel for pixel at fit and
      8× zoom. CLI commands other than the app still use `Document::open` (1 GiB cap).

### P2 — Headless CLI (`apps/figstudio-cli`)
- [x] `info`, `tree --depth`, `render --page/--node/--scale/--background`, `thumbnail`
- [x] renders all 7 fixtures (3–366 ms each, release build)
- [ ] CLI tests (partial: usage/runtime errors and core command output checks; T1 remains open)
- [ ] `render --all-pages` and JSON output for `info`

**Done when:** CLI tests pass and output is checked for every fixture.

### P3 — Desktop app (`apps/figstudio`)
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
- [x] errors shown as toasts (bad file stays non-fatal)
- [ ] save dialog path checked by hand on a desktop (Xvfb has no portal, so the export
      menu was checked but the dialog itself was not exercised)
- [ ] check on a real Wayland session and with a big file

**Checked so far (Xvfb + xdotool, 1400×880):** empty state, structure.fig open, tree expand,
canvas click selects `social-twitter 1` and fills Properties, Ctrl+wheel 73%→241%,
`=` zoom, ⇧0 to 100%, bad file toast, File → Export submenus.

### P4 — Platforms
- [ ] Linux: release build runs on X11 and Wayland
- [ ] macOS: `cargo check --target aarch64-apple-darwin` here; build steps in README;
      requester builds and launches on a Mac by hand when free
- [x] wasm (optional): browser app with threads (Web Workers + shared memory,
      `scripts/web-build.sh`); opens dropped `.fig` files; checked in headless Chromium
      (crossOriginIsolated, N workers, renders fixtures and a stress file). Not checked:
      Firefox / Safari, the file picker dialog, export download in a real browser

**Done when:** Linux and wasm checks pass here; macOS item is ticked only after the
requester confirms it runs.

### P5 — Delivery
- [ ] README (build, run, CLI usage, known gaps) — task T2
- [x] agent guide: `AGENTS.md`, `docs/IMPLEMENTATION.md`, `docs/TESTING.md`; scripts
      `check.sh`, `thumb-sheet.py`, `gui-smoke.sh` (all three run here)
- [x] `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` clean (1.99.0)
- [x] repo location: private `pelvo/figstudio` (requester asked, 2026-10-09); `main` pushed

### P6 — Editing (later, not started until asked)
- [ ] command/undo model over `Document`
- [ ] move/resize, rename, show/hide, reorder layers, edit text, edit fills
- [ ] save `.fig` (re-encode message, keep other archive entries), verify Figma opens it

## Verification commands

```
export PATH=$HOME/.cargo/bin:$PATH
cargo +1.99.0 test --workspace
cargo +1.99.0 clippy --workspace --all-targets -- -D warnings
cargo +1.99.0 build --release -p figstudio-cli
target/release/figstudio-cli render tests/fixtures/structure.fig -o /tmp/structure.png --scale 2
```

## Open questions

1. ~~Repo location~~ — decided: private `pelvo/figstudio`.
2. Final name (`figstudio` is a working name; avoid "view" since editing may come).

## Risks

- Figma's text layout cannot be matched exactly; files saved without stored glyph outlines
  fall back to installed fonts, so line breaks may differ.
- `vello_cpu` 0.3 is young; filter support is limited to blur and drop shadow.
- Very large files: decoding keeps the whole node tree in memory (670 MB for 194k layers);
  images are read lazily (P1b). A file whose node tree alone exceeds the Wasm memory would
  still fail in the browser.
- Writing `.fig` that Figma accepts is unverified until someone opens a saved file in Figma.

## Progress log

- 2026-10-09 — Rust audit fixes (F1–F11, see `target/rustaudit/report.md`, not tracked).
  Loader rejects parent cycles (iterative, all nodes), `page_of` and `descendant_count` bounded.
  Decompression capped per item (512 MiB) and per archive (1 GiB) with `take(limit+1)`. Save
  uses a unique temp file and removes it on failure. Empty images kept; malformed blobs error.
  Integer writes checked (u8/i32/u32). Render has a shared 2,000,000-attempt work budget
  across bounded bounds preparation and traversal; ordinary recursion and hit testing are guarded,
  and omitted work is reported in `Stats`. This leaves >160x headroom over the measured real-file
  page render (12,418 work units from a prior measurement; final attempted-work telemetry not
  collected directly). Image cache is bounded by 256 MiB and 256 successful entries;
  conflicting bytes under one image key invalidate cache hits. App exports admit one in-flight job
  and release admission on completion/failure/unwind; byte-backed drops share open deduplication.
  `scripts/check.sh` passed (fmt, clippy, tests, release build); focused package tests and CLI
  privacy/output tests passed. The review follow-up is recorded in `target/auditfix/fix2-report.md`.
  Not done: live GUI check, Pillow thumb sheet, pixel-parity review, and RSS measurement.

- 2026-10-09 — Pony audit L1 privacy/trust: doctor now bounds its input reads and keeps archive
  paths/schema prelude out of output; Kiwi list lengths and schema interning are bounded, and
  ID-less node records are rejected rather than dropped. L2: iterative layer/CLI walks, descendant
  fit bounds, retained source-byte cache accounting, indexed override lookup, unsupported draw
  telemetry and clip/mask pixel checks. Thumb sheet skipped (Pillow unavailable). Both lanes passed
  `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh`; L3 synth IDs, render-worker unavailable state,
  wrapper/test cleanup, decoded-item and schema limits, and doctor prelude privacy passed the gate.
  Private-file smoke passed info/doctor/render/thumbnail/resave/reopen with the input unchanged.
  Pony audit follow-up: all 15 non-false-positive findings fixed; see `target/ponyfix/report.md`.
- 2026-10-09 — CLI validation: unknown commands now fail as usage errors before file access;
  black-box tests check usage/runtime errors and core command outputs. PNG outputs are decoded
  and dimension-checked; doctor missing-file usage and resave confirmation are asserted. CLI,
  doctor privacy and `scripts/check.sh` pass; T1 remains open.
- 2026-10-08 21:5x — Rust spike: kiwi-schema decode + tiny-skia render of 7 fixtures.
- 2026-10-08 22:0x — Requirements fixed (macOS + Linux + headless, wasm optional, Vello,
  PhotoCraft-style UI). Switched renderer to `vello_cpu`.
- 2026-10-08 22:1x — Workspace created. P0 done (5 tests pass). P1 renderer compiles with
  instances, clipping, masks, images, text, shadows. CLI renders all 7 fixtures; output not yet
  visually checked. GUI not started. Plan committed (92a5207).
- 2026-10-08 22:2x — Requester: editing may come later, so not "view" → renamed project to
  `figstudio` (working name). Mac tested by hand by requester. Large private file cannot be
  shared → added P1b (round-trip writer, synthetic stress file, privacy-safe `doctor`) and P6.
  `kiwi-schema` has `Value::encode_bb`, so writing `.fig` back is possible.
- 2026-10-08 22:3x — P1 visual check. Side-by-side sheet of 7 fixtures vs thumbnails.
  Fixed path blobs that draw after a close (text in noise_and_grain was invisible) and vector
  per-path colours from `styleOverrideTable`. Progressive blur skipped instead of a wrong
  uniform blur. 8 render tests + 5 decode tests pass. Remaining P1 items are effect gaps.
- 2026-10-08 22:4x — P3 desktop app built (egui 0.36 / glow). Checked in Xvfb with xdotool
  clicks and screenshots. CLI now prints usage only for usage errors. Image cache no longer
  caches "missing" across documents. clippy + fmt clean on 1.99.0, 13 tests pass.
- 2026-10-09 09:xx — P1b. Writer with byte-identical round trip on all fixtures; direct
  Kiwi→Val decoder (memory 2.8 GB → 470 MB, decode 2.0 s → 0.3 s on 103k nodes); culling;
  `synth`, `resave`, `doctor` commands; doctor privacy test. 23 tests, clippy/fmt clean.
- 2026-10-09 09:2x — Pushed to private `pelvo/figstudio` (42d40d9). Requester asked for a test
  and implementation guide other agents can follow: added `AGENTS.md` (+ `CLAUDE.md`),
  `docs/IMPLEMENTATION.md`, `docs/TESTING.md` and `scripts/check.sh`, `thumb-sheet.py`,
  `gui-smoke.sh`; all three scripts run here. Found `figstudio-cli` had no `[lints]`, so the
  workspace lints never applied to it: added them, changed `unsafe_code` forbid → deny with
  one local allow for `getrusage`, checked that unsafe elsewhere fails the build.
- 2026-10-09 13:xx — Browser build with threads ("method A"): `wasm-bindgen-rayon` pool,
  render loop and decoding on Web Workers, UI thread never blocks. Renderer records the walk
  once and replays it per strip on the pool (`render_parallel`, exact stats; pixels within 1).
  Vello's own multithreading cannot run in wasm (it spawns std threads). 100% zoom 15 → 6 ms
  in Chromium; fit zoom unchanged (walk-bound, T15).
- 2026-10-09 14:xx — Requester's 1.7 GB file ran out of memory in the browser. Lazy archive
  reading + per-level image cache + images read on demand (see P1b). Opens in 1.4 s at
  537 MB Wasm memory, all pages render, peak 906 MB; lazy matches whole pixel for pixel.
  Found and fixed: frames depended on the decode order of shared images.
- 2026-10-09 14:5x — Requester's browser showed CJK page and layer names as boxes. The
  browser app now fetches a 5 MB Noto Sans CJK HK subset (`scripts/web-font.sh`) only when
  a file has CJK names or text; desktop fallback list gained macOS / Windows fonts
  (PingFang HK, Heiti TC, Hiragino Sans GB, Arial Unicode MS, Microsoft YaHei; not checked
  on a Mac). Checked in headless Chromium: Latin file fetches nothing, CJK file renders.
- 2026-10-09 — Correct instance override paths through ordinary frames/groups. Synthetic
  nested-instance pixel regression fails without the fix; private visual comparison improves
  state selection and chart layout. Full workspace gate recorded in the local render report.
- 2026-10-09 — Added scoped component properties, exact-version paint-variable alias resolution,
  and omitted derived translations for scaled/resized instances. Synthetic pixel regressions
  proved red/green; visual comparison restores table values and improves radial leaders.
  Reference still differs in some component states and right-panel structure; not claimed exact.
- 2026-10-09 — Strengthened stale-property-text regression with a stored old glyph so it proves
  stale outlines are rejected rather than only proving an empty text node remains empty.
- 2026-10-09 — Honor `stackReverseZIndex` when painting siblings, restoring the second chart
  formerly covered by a history list. Synthetic overlapping-instance pixel regression proved
  red/green; private visual comparison confirms chart content. State-colour investigation open.
- 2026-10-09 20:xx — Merged `fix/c1-rendering` into `web-threads`. Its byte-bounded image cache
  is replaced by the per-level cache (`render::images`), which now keys entries by document
  (`Document::instance`) so equal hashes with different bytes never alias; its cache tests
  are rewritten against it. Work budget kept on both the serial and parallel paths; masks
  use `push_clip_path`, now recorded and replayed per strip (pixel test against serial).
  Lazy archive reads keep the decoder's size limits. Desktop exports run in the background
  and read lazy images. Big file: lazy matches fully read pixel for pixel on every page at
  fit and 8x; in headless Chromium it opens in 1.5 s, all pages draw, peak 869 MB.
- 2026-10-10 — Strokes (A11): `strokeJoin`, `strokeCap`, `dashPattern` and per-side border
  weights drawn when no stored geometry applies; stroke overrides drop stale stored outlines.
  Pixel tests in `stroke_style.rs`. The two reported dialogs already matched Figma's border
  weight and radius after the strokeAlign checkpoint; this change does not alter their pixels.
  Not verified: dashed strokes and caps against Figma on a real file.
- 2026-10-11 — A7: closed VECTOR nodes clip stored inside/outside stroke geometry to the fill,
  as boxes already did (`geom::closed_network` reads the vector network). The tooth-chart A–E
  labels on a private file no longer render bold; test in `stroke_align.rs`. Not verified:
  live compare in Figma; open vectors still draw these strokes unclipped (counted).
