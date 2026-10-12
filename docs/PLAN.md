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
- [x] missing VECTOR fills counted only when a visible fill paint has no path (A19)
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

### P7 — Offline MCP server (`apps/figstudio-mcp`, plan in `docs/MCP-PLAN.md`)
- [x] L0 schema facts (auto-layout, text runs, tokens, FigJam; none of the fixtures has variables, FigJam or motion data)
- [x] L1 skeleton: rmcp server, stdio and Streamable HTTP, node ids, file registry, `whoami`, `list_files`
- [x] L2 `get_metadata`
- [x] L3 `get_screenshot`
- [x] L4 public token model (`fig::tokens`) and `get_variable_defs`
- [x] L5 public instance resolver (`fig::resolve`) and instance paths `I…;…`
- [x] L6 `get_design_context` (React + Tailwind, assets)
- [x] L7 `download_assets` (PNG/JPG, original image bytes)
- [x] L8 `get_figjam`
- [x] L9 Code Connect (local sidecar) and design-system search (`get_context_for_code_connect`,
      `get_code_connect_map`, `add_code_connect_map`, `search_design_system`, `get_libraries`)
- [x] L10 `create_design_system_rules` prompt
- [x] L11 merged into L1 (Streamable HTTP)
- [x] L12 docs: `apps/figstudio-mcp/README.md`, `docs/TESTING.md` section 12, code map and task board
- [ ] Open (task board M1–M6): `get_motion_context`, SVG export, ancestor masks on instance-child
      screenshots, nested-instance override visibility in `original=true`, live-client check,
      real FigJam and motion files

**Done when:** every lane's gate passed (it did, L0–L12). The open boxes need a real motion
file, a real FigJam file or a person with the clients.

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

- 2026-10-12 — T20 L5: docs for the GPU canvas (IMPLEMENTATION desktop + browser, GPU.md open
  gaps). L1–L4 and P1–P3 reviewed until clean; branch feat/vello-hybrid.

- 2026-10-12 — T20 L1 (wgpu) reviewed three times; GPUs with a texture limit below the
  window size can still crash on some resize/fullscreen paths. Accepted and documented in
  `docs/GPU.md` ("Known limit"); the L3/L4 CPU fallback is the planned answer.

- 2026-10-11: MCP L4: public read-only token/style model and `get_variable_defs` with all
  stored modes, bounded aliases, legacy/current refs and text-run bindings. Foreground
  `scripts/check.sh` passed; seven public fixture PNGs byte-identical at scale 0.25.
  Synthetic and SDK protocol tests pass (mutation-proven). Not checked: wasm target absent,
  private/library documents, full-scale visual inspection; instance expansion remains L5.

- 2026-10-11 — Approximation counters measured on a large real file and broken down by
  reason; plan to clear them recorded in `docs/APPROXIMATIONS.md` (task T19).

- 2026-10-11 — A19: exclude stroke-only vectors and hidden/zero-opacity fills from the
  missing-fill counter. Synthetic counter regression observed red/green; workspace and
  wasm gates passed. Figma comparison not verified.

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
- 2026-10-11 — A16: a frame that clips its content now draws its stroke above its children,
  so a table's outer border is no longer hidden by the cells that fill it; non-clipping frames
  keep the stroke below. Test in `stroke_order.rs`. Not verified: live compare in Figma. The
  grey grid lines and header fill are the file's own stored paints and are left as stored.
- 2026-10-11 — A17: constraints no longer move the in-flow children of an auto-layout frame
  that an instance resized; auto layout keeps them at the padding, so a label in a
  hug-width chip stays centred instead of drifting left. Absolute-positioned children still
  follow constraints. Test in `auto_layout.rs`. Not verified: live compare in Figma. A full
  re-run of auto layout for instances without derived transforms was tried and dropped: it
  moved stored positions that already follow property defaults (see the report).
- 2026-10-11 — A20: fill-based drop shadows are drawn behind the node in list order, and
  inner shadows over its fills. Positive spread is applied; negative spread and non-normal
  blend modes are counted as approximate. Tests in `shadows.rs`. Not verified: live compare
  in Figma; spread on rounded shapes uses a stroked outline.
- 2026-10-11 — A20 review: inner clips install their fill rule; drop masks draw each path
  under its own rule; transparent solid fills no longer cast opaque geometry shadows.
  Compound inner masks, complex paint/stroke/child silhouettes and anisotropic blur are
  counted once per effect. No-fill nodes retain the alpha-preserving first-drop filter,
  counting unsupported spread/blend and every skipped later drop or inner shadow. Shadow
  attempts use the work budget, with at most eight per node admitted. Synthetic pixel,
  counter and mutation checks cover clipping, union, transparency, order, blur and spread.
  Not verified: live Figma parity or exact rendering of the counted approximations.
- 2026-10-11 — A18: a uniformly scaled instance (`uniformScaleFactor`) now also scales the
  size and offset that a nested instance states in its own unscaled derived data; before,
  only the innermost scale applied to omitted offsets and sizes were never scaled, so stored
  outlines drawn at the scaled size sat off base-shape discs in tooth-chart-like icons.
  Test in `instance_overrides.rs`. Not verified: live compare in Figma.
- 2026-10-11 — A18 review fix: place omitted constrained translations before inherited
  scaling; preserve explicit placement and auto-layout flow. Scale inherited stored glyphs
  and stroke weights by their field owners, keeping outer-stated values untouched; remaining
  inherited ink approximations are counted in status. Synthetic red/green and ownership
  mutation tests, workspace/wasm gates, PII scan and local render comparisons passed.
  Not verified: live Figma comparison, fallback font or stored-outline recomputation.
- 2026-10-11 — MCP L1: add the offline rmcp server, stdio and Streamable HTTP,
  lazy local-file registry, node-ID parser, whoami/list_files; nine failing-first tests,
  workspace gate and clean-stdout release smoke passed. Not verified: live client setup,
  OS-signal shutdown or independent review; no renderer or decoder changes.
- 2026-10-11 — MCP L1 review: nonblocking OnceCell cache, cause-bearing tool errors,
  unambiguous --http=ADDR, non-loopback warning, SIGTERM shutdown and stdio purity tests.
  scripts/check.sh passed; regression and mutation tests proven. Not verified: live clients, real Ctrl-C,
  Windows or HTTP OS signals; no renderer or decoder changes.
- 2026-10-11 — MCP L2: get_metadata page lists and sparse parent-relative XML, quick-xml
  escaping, depth/5 000-node bounds, Unicode text cap and instance leaves. Public fixture,
  synthetic and end-to-end tests proven by mutations; scripts/check.sh: all checks passed.
  Not verified: live clients, official capture, private files or instance expansion (L5).
- 2026-10-11 — MCP L2 review: correct Figma counter-clockwise rotation, omit page/document
  geometry and unsized dimensions, sanitize XML controls, mark truncated text, filter internal
  subtrees and self-close empty leaves. Regression tests proven failing first;
  scripts/check.sh: all checks passed. Not verified: live clients, official capture,
  private files or L5 expansion.
- 2026-10-11 — MCP L3: get_screenshot renders a node to a base64 PNG with size and
  approximation lines (4096 px cap, base64 crate). Tests decode the PNG and were
  proven by mutations. Not verified: pixels, live clients, huge pages.
- 2026-10-11 — MCP L3 review: screenshots are cropped to the node's own box (381x191 for the
  fixture node), the 4096 px cap works below the old 0.01 scale floor and reports the real
  scale, approximations come from Stats' fields incl. culled. New tests proven by mutations;
  scripts/check.sh: all checks passed. Not verified: live clients, private files, L5 expansion.
- 2026-10-11 — MCP L4 review: read real Number messages and mode sort positions, ignore
  detached style sentinels, remove f32 decimal noise, qualify cross-collection collisions,
  share font weights and preserve first-stored-mode/valued-only asset rendering. Regression
  proofs and the full gate pass; parallel clippy passes; seven scale-0.25 PNGs byte-match
  before/after and thumbnail sheet inspected. Renderer equivalence also passes on fb03bf8.
  Not verified: wasm, live clients, private/library documents, full-scale visual accuracy.
- 2026-10-11 — MCP L5: public instance resolver `fig::resolve` shared by the renderer and the
  MCP tools; get_metadata expands instances (I…;… ids, overrides), and instance paths resolve
  in get_metadata, get_screenshot (instance cropped to the child box) and get_variable_defs.
  28 fixture renders byte-identical before/after, thumb sheet viewed, new tests proven by
  mutations, scripts/check.sh passed. Not verified: private/large files, screenshot box of
  scaled or resized instances, live clients.
- 2026-10-11 — MCP L5 review: instance paths resolve by tree position (metadata depth
  counts tree levels), instance-child screenshots draw only that node with the renderer's
  scaled layout (Renderer::render_path / path_bounds), variable defs ignore override and
  property storage, swapped instances report their new component, the walk queues children
  lazily. Each regression failed on 4518cbc; 28 fixture PNGs byte-identical; check.sh passed.
  Not verified: private/large files, live clients, ancestor masks in child screenshots.
- 2026-10-11 — MCP L6: `get_design_context` writes React + Tailwind JSX (flex auto-layout,
  absolute children, fills/gradients, borders, radii, shadows/blurs, opacity/blend, text with
  style runs, var(--name,fallback)) for nodes and instance paths, with rendered PNG assets in
  `--assets-dir` (default ./figma-assets) or `--assets inline`, node/size caps and counted
  approximations. Built-document and fixture tests, each mutation-proven; check.sh passed.
  Not verified: live clients, private/large files, visual fidelity of the generated code.
- 2026-10-11 — MCP L7: `download_assets` renders up to 20 nodes to PNG/JPG (JPG flattened onto
  white, same scale range and 4096 px cap as get_screenshot via a shared `render_node`) or
  returns the original image-fill bytes (`original=true`). 9 tests in `tests/assets.rs`, each
  proven by a mutation; 28 fixture PNGs unaffected (no render crate change); check.sh passed.
  Not verified: lazily opened files, very large originals, live clients.
- 2026-10-11 — MCP L8: `get_figjam` = get_metadata XML plus FigJam attributes and text
  (sticky colour/text, shape type, connector endpoints and magnets, section status, code, table
  cell, widget), optional small screenshots (max 20, scale <= 0.5) and an approximations
  comment for kinds the renderer does not draw. 5 tests on an in-test board, mutation-proven;
  check.sh passed. Not verified: real FigJam files, connector geometry, table structure.
- 2026-10-11: MCP L9 adds offline Code Connect sidecars and local design-system discovery; mutation-proven tests and full gate passed. Not verified: live parity, Windows, cross-process/concurrent filesystem writers.
- 2026-10-11: MCP L10 adds create_design_system_rules prompt with capped local inventory; SDK protocol mutation proofs and full gate passed. Not verified: live rule-file generation or official text parity.
- 2026-10-11 — MCP L9/L10 second-pass review: real component-set fields and variants,
  normalized/capped properties, bounded permission-preserving sidecars, requested map keys,
  snippet imports and bounded prompt names; failing-first regressions and mutation proofs;
  foreground `scripts/check.sh`: all checks passed.
  Not verified: live clients, official-server parity, private files, Windows or concurrent writers.
- 2026-10-11 — MCP L12: `apps/figstudio-mcp/README.md` (flags, transports, client setup for Claude Code,
  Cursor/VS Code and Codex, tool table, node ids, not-replicated list, privacy, writes, limits);
  TESTING.md section 12; IMPLEMENTATION.md map and tasks M1–M6; MCP-PLAN status. Tool list checked
  against the release binary over stdio (13 tools, 1 prompt); docs only, foreground
  `scripts/check.sh`: all checks passed. Not verified: client configs against a live client.
- 2026-10-12 — MCP lazy documents (PR #5 review lane 4): `FileRegistry` opens via `open_auto_above`
  (`with_lazy_above` for tests); all MCP renders use `render_complete`/`render_path_complete` with a
  per-file shared `Images` cache and on-demand `archive::read_entry`; `download_assets original=true`
  reads lazy originals after a size check against the budget. `tests/lazy.rs` proven by mutations;
  CLI render of 7 fixtures at 0.25 and 1 byte-identical to 407102c; foreground `scripts/check.sh`:
  all checks passed. Not verified: a real >256 MiB file, the wasm UI (`web.rs` unchanged).
- 2026-10-12 PR #5 review round 2 (figstudio-mcp): five lanes, one commit each. Text caps
  24 000 with `--max-output-bytes` (37c13f3); download_assets 10 MiB reply and 7.5 MiB per
  image (12624bf); default screenshots 2048 px (81e9dad); wrapped auto-layout
  `content-start`/`content-between` (d644547); sidecar cross-process lock (849d195). Each
  behaviour change proven by a test that fails with the change removed; foreground
  `scripts/check.sh` all checks passed before each commit. Not verified: a live Claude Code
  session against the 1.7 GB file (the reviewer's measurements are the evidence for the
  limits); the sidecar lock on network filesystems; the wasm UI (unchanged).

- 2026-10-11 — T20 L1: app runs eframe on wgpu (desktop + web) instead of glow; window screenshot
  on macOS matches the glow build; web build and `scripts/check.sh` passed. Not verified: Linux,
  Windows, a browser run (WebGPU or WebGL2).
- 2026-10-11 — T20 L1 fix: request only the GPU limits the adapter has (WebGL2 with a 4096 texture
  limit started failing on wgpu); unit test seen failing first; headless Chrome with forced WebGL2/4096
  opens a fixture. Not verified: a real 4096-limit device, Linux, Windows.
- 2026-10-11 — T20 L1 fix 2: keep the GPU surface within the device texture limit (browser canvas
  cap, desktop max window size) and request the adapter's full texture size; unit tests seen failing;
  forced WebGL2/4096 resize to 5000 px no longer panics. Not verified: desktop cap on a large screen.
- 2026-10-12 — T20 P1: the canvas renders a margin (half the canvas per side, capped by texture
  limit, u16 and 32 Mpx) and requests a new frame only near the margin's edge or on zoom/size/page
  change; 6 unit tests seen failing under 5 mutations; scripted pan showed no empty edge and one
  request per ~320 pt of pan. Not verified: real hand-tool drag (no input tool), web by eye.
- 2026-10-12 — T20 P1 fix: overscan frames stay within the pixel cap (32 Mpx native, 16 Mpx web)
  and the texture limit by rendering an over-cap canvas at lower resolution; one planner decides
  requests, so an over-limit canvas no longer re-requests every update; web-build.sh exits 0
  without the CJK font. Tests seen failing under mutations. Not verified: over-limit canvas by eye.
- 2026-10-12 — T20 P2: the canvas is drawn from cached 512 px tiles per zoom level (LRU, 256 MiB
  native / 128 MiB web) with coarser/finer fallback under missing tiles and centre-first requests;
  overscan.rs removed; 12 unit tests seen failing under mutations; scripted pan/zoom logged no
  uncovered tile during pans. Not verified: real hand-tool drag, web by eye, large file by eye.
- 2026-10-12 — T20 P2 fix: full coverage of views over half the cache (coarser level, on-screen
  tiles protected), evicted tiles re-requested (worker skips only unacknowledged tiles), 2-px
  tile gutters (seam 37/255 → ≤3/255 in a CPU model of the GPU's sampling), tab close cancels its
  batch and closed documents store no images. Tests seen failing first. Not verified: GPU seams.
- 2026-10-12 — T20 P2 fix 2: planned tiles evict drawn fallback tiles (covered ones first), so a
  cache full of fallback settles instead of re-rendering; closed documents are tombstoned in the
  image cache and checked under its lock after decode. Tests seen failing first.
- 2026-10-12 — T20 P3: tiles of one page and zoom level replay one recording of the page (walked
  once per batch area); over-limit or budget-cut recordings fall back to direct renders, counted as
  "tiles not recorded". Tests seen failing first. Not verified: scripted window pan, wasm timing.
- 2026-10-12 — T20 L2: feature `gpu` of figstudio-render replays a recorded tile into a
  `vello_gpu` scene (`Recording::gpu_scene`) after a scan that refuses image paints, angular
  gradients, other filter graphs, huge filter reach and >10k layers (`GpuUnsupported`, counted as
  `Stats::gpu_fallback_tiles` once L3 wires the app). GPU vs CPU tiles on Metal: ≤ 2/255 (filters ≤ 5/255).
  Not verified: Vulkan/DX12/GL/WebGPU/WebGL2 pixels, GPU-side panics beyond the scanned cases.
- 2026-10-12 — T20 L2 fix 1: the GPU scan sizes nested filter layers as vello_gpu does (each filter
  grows the intermediate viewport), refusing tiles that would fail with `TooLarge`;
  `gpu_scene` takes the texture limit. Test seen failing first. Not verified: non-Metal GPUs.
- 2026-10-12 — T20 L3: the desktop app draws canvas tiles with vello_gpu on eframe's wgpu device
  (render thread renders, UI registers), falling back to counted CPU tiles; FIGSTUDIO_RENDERER and
  a status-bar renderer label. Not verified: Vulkan/DX12/GL pixels, device loss, a sub-4096 GPU.
- 2026-10-12 — T20 L4: the browser draws canvas tiles with vello_gpu (WebGPU, else WebGL2 through
  wgpu's GL backend, else CPU): the worker builds scenes, the main thread renders ≤ 4 per frame;
  `?renderer=cpu|gpu`. Headless Chrome, structure.fig: both GPU paths within 1/255 of the CPU.
  Not verified: a real non-Apple GPU, device loss, a late wgpu error scope firing, Firefox/Safari.
