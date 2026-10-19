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
| UI in the Arc browser style | accepted (2026-10-19; was PhotoCraft-like) | same toolkit (egui 0.36): tinted window gradient per file, sidebar, rounded canvas card, command bar, springs; no Arc assets copied |
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
- Anything not drawn exactly is counted in `Stats` and shown to the user (app: the warning chip
  and status pill on the canvas; CLI).
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
      (since the Arc redesign: the tool pill shows the zoom, the status pill the rest, a
      warning chip the unsupported features)
- [x] Arc-style redesign (2026-10-19): per-file gradient (light and dark, follows the system,
      View → Appearance), sidebar with window controls, command field, pages, layers and a
      file switcher; frameless window on macOS; the canvas as a rounded card with tool and
      status pills; Properties as a card; ⌘K command bar (commands, files, pages, layers);
      hidden sidebar peeks from the left edge; zoom jumps ease around their anchor; springs
      and fades, View → Reduce motion
      (the browser follows `prefers-reduced-motion`)
- [ ] macOS vibrancy (translucent sidebar): `window-vibrancy` puts its NSVisualEffectView above
      wgpu's Metal layer, which hides the whole UI; fixing the layer order needs `unsafe`
      AppKit calls, which ground rule 10 forbids. Waiting for a decision from the owner
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
- [x] MCP inside the app (`docs/MCP-APP.md`, issue #9): L0–L9 done. Shared `crates/mcp-core`; right-click Copy and
      marquee Copy region (text and PNG) in the app; View → Agent access starts the live server on loopback
      (`list_open_tabs`, `get_selection`, `reveal_node`; nodeId defaults from the selection; `figstudio://` links)
- [ ] MCP inside the app, open: a live client session (`claude mcp add`), clicking the toggle, the clipboard paths
      in a real window and browser, paste into Preview or an editor
- [x] Web app + MCP bridge (`docs/WEB-MCP-PLAN.md`, L1–L4): `figstudio-mcp --bridge`, the browser tab pairs over a
      token-guarded WebSocket and uploads its files; automated end-to-end check `tests/bridge_cli.rs`
- [x] Remote agents (WEB-MCP-PLAN L5): `--http=ADDR` non-loopback needs a bearer token + Host allowlist, app "Allow remote agents"
- [ ] Web bridge and remote agents, open: a real Tailscale/SSH-tunnel session; manual runs in a real browser (local :8765, a deployed site in Chrome and Safari)
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

- 2026-10-14 — Join an existing MCP room: Agent access gained Copy room link and Use room
  token (bare token, MCP URL or room link; invalid input toasts and changes nothing), and
  a `#room=` fragment joins on startup and `hashchange`, consumed with
  `history.replaceState` so the token leaves the address bar. Pure helper tests with five
  mutation controls and a Playwright second-browser join test shipped; full
  `scripts/check.sh --web` gate and 3 e2e tests passed. Menu Apply itself not e2e-driven;
  Safari/Firefox and deployed hibernation unverified.
  (Superseded 2026-10-15: the fragment path now asks before joining — see the
  room-link join guard entry.)

- 2026-10-14 — Browser MCP Lane 1: byte-backed entry readers, SDK-free tab JSON-RPC,
  shared prompt text, native schema drift guard and no-fs inline assets shipped. Core
  tests, seven mutation controls, pinned nightly wasm32 check and Rust 1.99.0 full gate
  passed; public-fixture sheets inspected. Browser workers, UI reveal and relay unverified.

- 2026-10-14 — Shared native/drawn menu actions and Agent access/About windows shipped;
  macOS attach/Open/Close/Quit checked with a public fixture, Windows cross-check passed.
  Full native checklist and browser GUI not verified; two live-port tests excluded locally.

- 2026-10-12 — PR #7 review (Linux, AMD): wgpu uncaptured errors are logged and turn the GPU
  tiles off instead of panicking (desktop and web); software adapters (`DeviceType::Cpu`) use the
  CPU canvas unless the GPU is asked for; GPU.md records the Linux results. Tests seen failing first.

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
- 2026-10-12 — MCP-APP L0: golden harness (`apps/figstudio-mcp/tests/golden.rs`, `scripts/mcp-golden.sh <rev>`) records every tool's reply over the 7 fixtures plus a synthetic FigJam board, incl. `original=true` (197 records); 3 same-tree runs identical, description and output mutations caught, check.sh passed. Not verified: cross-machine byte equality, add_code_connect_map (writes a sidecar, not recorded).
- 2026-10-12 — MCP-APP L1: `crates/mcp-core` (node ids moved, `CoreError`, `figstudio://` and figma.com links, `file_for` in the MCP adapter, wasm32 build step in check.sh); `scripts/mcp-golden.sh 72083b7` identical (197 records), check.sh passed, 7 mutations caught. Not verified: a real MCP client with a figstudio:// link, web build.
- 2026-10-12 — MCP-APP L2: right-click Copy in Layers, canvas and Properties (name, text, node ID, link, agent block; `apps/figstudio/src/copy*.rs`); Properties values were already selectable (egui 0.36 default), Text section gets a Copy button; `Copy properties as JSON` waits for L5. Gates: check.sh passed, web build exit 0, `scripts/mcp-golden.sh be6dedf` identical (197 records), 4 mutations caught. Not verified: a real right-click in the window, paste into an editor, the web clipboard. Canvas and Layers copy show document nodes only (no instance paths yet).
- 2026-10-12 — MCP-APP L3: marquee (Select tool drag) with right-click Copy region (link + page + node IDs/names, top-level first, ≤ 50) and Copy region as PNG (off-thread render, 4096 px / 16 MP cap, clipboard image, web download fallback); `region::nodes_in` in `crates/mcp-core`. Gates: check.sh passed, web build exit 0, `scripts/mcp-golden.sh b2932ca` identical (197 records), 6 mutations caught. Not verified: a real drag/right-click, paste into Preview, the browser clipboard path.
- 2026-10-12 — MCP-APP L4: text tools (`get_metadata`, `get_figjam` text, `search_design_system`/`get_libraries`, `get_variable_defs`, `get_context_for_code_connect`) moved to `crates/mcp-core` behind an SDK-free `Output`; adapter keeps schemas, `spawn_blocking` and old wrappers. Gates: check.sh passed (incl. wasm32), `scripts/mcp-golden.sh aa14d6a` identical (197 records) after every move, `apps/figstudio-mcp/tests` untouched. Not verified: a live MCP client session, `get_figjam` images (still adapter-side until L5).
- 2026-10-12 — MCP-APP L5: rendering tools (`Rendering`, `get_screenshot`, `download_assets` incl. originals, `get_design_context`, `get_figjam` pictures) moved to `crates/mcp-core` with features `fs` and `system-fonts`; lazy image reads unchanged. Gates: check.sh passed (incl. wasm32 `--no-default-features`), `scripts/mcp-golden.sh a226dcc` identical (197 records) after every move, `apps/figstudio-mcp/tests` untouched, `rg 'rmcp|tokio|axum' crates/mcp-core` empty, 2 mutations caught. Not verified: a live MCP client session, a browser build that calls the rendering tools, `fs`-off design assets beyond the unit test.
- 2026-10-12 — MCP-APP L6: `get_screenshot` accepts a `figstudio://…?page=…&rect=…` region link (same crop and caps as the app's Copy region PNG, shared in `crates/mcp-core/src/region_shot.rs`); new tool `get_nodes_in_region` (`region` or `page`+`rect`, `limit` ≤ 500, JSON `{page, rect, nodes, total, truncated}`). Golden differs only in `00-tools-list.json` and the new tool. Not verified: a link pasted from a real app session, a live MCP client.
- 2026-10-12 — MCP-APP L7: live mode in `figstudio-mcp` (`live.rs` snapshot/commands, `Files::{Registry,Live}`, `FigstudioServer::live` with `list_open_tabs`/`get_selection`/`reveal_node`, nodeId defaults from the selection, standalone "nodeId is required"), bare `--http` falls back 3845→3846–3855, `--http=ADDR` never does; http tests assert no CORS header. Golden (`f753974`) differs only in `00-tools-list.json` (nodeId optional on 5 tools). check.sh passed, 20 mutations caught. Not verified: a real app driving the snapshot (L8), a live MCP client.
- 2026-10-12 — MCP-APP L8: the desktop app hosts the live MCP server (feature `mcp`, default on, desktop only; `mcp_host.rs`): View → "Agent access (MCP server)" (off at every launch), loopback 3845→3855, status bar shows the address, `reveal_node` selects and fits via `Tab.fit_node`, snapshot swapped only on key change. Gates: check.sh passed (both feature sets, new `--no-default-features` clippy step), web build exit 0, `scripts/mcp-golden.sh bdafdc4` identical (197 records), 11 mutations caught; manual curl session against the app. Not verified: clicking the toggle, `claude mcp add`, the visual fit.
- 2026-10-12 — MCP-APP L9: docs. `apps/figstudio-mcp/README.md` (app mode, `figstudio://` links and the file rule,
  selection defaults, live tools, security), `docs/IMPLEMENTATION.md` (`mcp-core`, copy menus, `mcp_host` data flow),
  `docs/TESTING.md` (live and golden tests, mcp-core without default features). Every tool, flag, port and reply named was
  grepped against the code; two wrong statements fixed. check.sh passed (see the commit). Not verified: a live client
  session, GitHub rendering of the README.
- 2026-10-12 — Headless GUI tests: `egui_kittest` 0.36 (dev-dep, `eframe` feature) drives the real app in `cargo test` (`ui_tests.rs`, `ui_copy_tests.rs`, `ui_mcp_tests.rs`, 8 tests): View-menu MCP toggle (status label, port, `list_open_tabs`, port closed), node right-click Copy menu, marquee Copy region / PNG, click-select, Hand-tool pan. 10 mutations seen failing. Not verified: real OS clipboard/paste, real window manager.
- 2026-10-12 — Text with an inside/outside stroke: stored `strokeGeometry` (a centred band of twice the
  weight) is now clipped to the glyph outlines like box shapes, instead of drawn unclipped, which
  made thin grey text look bold and smeared. Test `stored_inside_text_stroke_is_clipped_to_the_glyphs`
  (seen failing without the fix). check.sh passed; checked against a private file render.
  Not verified: outside text strokes on a real file.
- 2026-10-12 — Outside strokes erase with the fill union (DestOut in an isolated layer), not an even-odd clip, so the band no longer shows inside overlapping glyph contours (e.g. variable-font A, K, R, 4). Inside strokes keep the NonZero clip. Test `stored_outside_text_stroke_skips_overlapping_glyph_contours` (seen failing before the fix at the overlap pixel). check.sh passed; CLI render of structure.fig works. Not verified: the look of outside text strokes on a real variable font or private file; GPU tile path only compiled and gate-tested, not pixel-compared for this case.
- 2026-10-13 — Logging and errors (docs/LOGGING-PLAN.md lanes 1-3): `tracing` to stderr in the CLI, MCP binary, desktop and browser app (default `warn`, `RUST_LOG` overrides); `FigError` is a canonical struct whose `source()` returns the I/O, zip or codec cause (Display text unchanged); CLI and MCP binary use `anyhow` with context, exit codes kept. Tests: `opening_a_missing_file_keeps_the_io_error_as_source`, CLI trace-logging stdout/privacy, truncated-file and cause-printed-once tests (each seen failing). check.sh passed. Not verified: MCP `listening on` line by hand, lanes 4 (spans) and 5 (remaining `Result<_, String>`).
- 2026-10-13 — Logging lanes 4-5: debug spans on document open/decode, render, GPU tile submit and MCP tool calls (tool name and duration only; test `tracing_spans.rs` checks counts are logged and no layer names); `mcp-core` render/asset functions return `CoreError` with real causes (test `errors.rs`); app internals use anyhow; `figstudio_fig::one_line` prints an error chain without repeating causes (CLI and app toasts; tests seen failing). check.sh and web-build passed; MCP stdio `whoami` keeps stdout pure JSON and logs `tool_call` at debug; `--http` still prints `listening on`. Not verified: app toasts in a real window, browser console output.
- 2026-10-13 — Logging follow-up: PNG encode failures in mcp-core use `CoreError::encode` (cause kept, text unchanged; test `an_encode_error_keeps_its_cause_and_the_old_text`); app `ImageSource::read`, `ExportDone::result`, region PNG, web.rs and the reveal reply use anyhow, formatted with `one_line` at the UI/tool boundary (test `a_missing_file_keeps_its_io_cause`, seen failing). check.sh and web-build passed. Not verified: browser paths run in a real browser.
- 2026-10-13 — Review fixes for PR #12: `listening on` is now plain stderr (survives `RUST_LOG=error`; test `listening_line_survives_a_quiet_rust_log`, seen failing with the tracing version); rmcp is capped at info unless `RUST_LOG` names it (test `trace_logs_do_not_leak_requests_or_layer_names`, seen failing without the cap); tool errors log `ok=false`; CLI drops the `open document`/`parse background`/`save document` context prefixes (text unchanged); over-inflated archive entries are `malformed`. Docs updated. Checked: the new and changed tests, `cargo test` for mcp, cli and fig on 1.99.0. Not verified: `scripts/check.sh` as a whole (see the final gate run), a real MCP client reading the port, and the `ok=` log field in a live run.
- 2026-10-13 — Browser CJK font is now a choice: View → CJK font picks Noto Sans CJK JP, SC, TC, HK or KR (`scripts/web-font.sh` writes `fonts/cjk-<code>.otf` for each; KR adds Hangul). The default follows the browser language (JP when nothing matches) and the choice is kept in localStorage; changing it replaces the loaded font. Tests: language → variant mapping (seen failing with a broken mapping) and font replacement. check.sh passed; web-build.sh wrote all five fonts (fontTools: right face names, kana and ideographs present, Hangul only in KR). Not verified: clicking the menu in a real browser.
- 2026-10-13 — CJK font review fixes: a file whose only CJK is Hangul switches to KR for the session (`theme::cjk_need`; test `hangul_without_other_cjk_asks_for_the_korean_face`, seen failing); `web-font.sh` refuses a non-`.ttc` override and checks every output's full face name (tried: a single-face `.otf` and wrong face indexes both fail); `web-build.sh` clears `dist/fonts` so the old `cjk.otf` is not deployed. cjk tests, native clippy and web-build passed. Not verified: full check.sh, Hangul-only file in a real browser.
- 2026-10-13 — CJK font second review: CJK punctuation and full-width forms no longer stop a Hangul-only file from getting KR (`CjkNeed::Shared`, every face has them); the session KR override is separate from the saved menu choice (`Inbox::korean_override`), the menu shows the saved choice plus a note, and any pick (the current one too) is saved and ends the override; the font-replacement test clears texture deltas so it passes in debug builds too. `web/wrangler.jsonc` is now a local, git-ignored file (it names one account's domain). Tests: `hangul_without_other_cjk_asks_for_the_korean_face` seen failing without the punctuation rule and with a fold that stops at Hangul. theme/cjk tests (debug), native clippy and web-build passed. Not verified: full check.sh, the menu note in a real browser.
- 2026-10-13 — Web MCP bridge L1–L4: protocol in mcp-core, `figstudio-mcp --bridge` (Origin + token checked
  WebSocket, uploads to a temp dir), wasm client in the app (View → Agent access, `#mcp=` auto-connect),
  README section. Tests: protocol, bridge, shared view state, pure client helpers, and a binary-level
  end-to-end (`bridge_cli.rs`), each seen failing under a mutation. check.sh --web passed. Not verified: a
  real browser session (see WEB-MCP-PLAN.md L4 TODOs).
- 2026-10-13 — Web MCP L5: opt-in remote agents (bearer token via `subtle`, Host allowlist, wildcard refused without a flag, app checkbox, README Tailscale + SSH `-R` tunnel). `tests/remote.rs` and unit tests seen failing under mutations; check.sh passed. Not verified: a session from another machine.
- 2026-10-13 — Web MCP review fixes: constant-time pairing-token compare, `error {tab}` names the failed upload (the tab drops it by id), 1 s bound on bridge sends so a stalled peer cannot block re-pairing; tests for hello timeout, error tab, stalled peer, remote `--bridge` bearer, each seen failing under a mutation. Not verified: a real browser session.
- 2026-10-13 — PR #16 F1: browser uploads copy each slice into a non-shared buffer before `WebSocket.send` (threads build). Wasm-only, no native test; check.sh and check.sh --web passed. Not verified: an upload in a real browser (reviewer confirmed the fix on the reviewed build).
- 2026-10-13 — PR #16 F2: uploads are bound to the tab id, not the file name, so two picked files with one name each stream their own bytes (`mcp_wire::Uploads`; test `two_files_with_one_name_stay_bound_to_their_own_tabs`, seen failing with a name-keyed mutation). check.sh and check.sh --web passed. Not verified: picking two same-name files in a real browser.
- 2026-10-13 — PR #16 F3: a remote `--bridge --http=<addr>` also listens on `127.0.0.1:<port>` for `/bridge` (and loopback `/mcp`), so the browser on this PC pairs; the banner prints `Browser bridge: ws://127.0.0.1:<port>/bridge` once it is listening. Test `a_remote_bridge_still_pairs_the_browser_on_loopback` (skips with no LAN address); seen failing with the loopback branch disabled. README Remote agents updated. check.sh passed. Not verified: a browser on a remote-bound bridge.
- 2026-10-13 — PR #16 F4: a wildcard bind (`0.0.0.0`/`[::]`) with no `--allow-host` prints a hint that clients must send a name or IP passed with `--allow-host` (pure `access::wildcard_hint`, test `a_wildcard_bind_hints_at_allow_host_only_when_none_is_given`, seen failing when disabled); test `a_wildcard_bind_refuses_an_unnamed_lan_host_even_with_the_token` (403 for an unnamed LAN Host, 200 for the loopback one). No interface enumeration (no new crate). README Remote agents updated. Not verified: a LAN client in a real network.
- 2026-10-13 — PR #16 F5: a socket that never says `welcome` gives up after 8 s (`CONNECT_TIMEOUT_MS`, stale timers ignored by generation) and the 1006 close shows the same message: it names local network access and that Safari blocks https -> ws:// (`mcp_wire::connect_failed_message`; test `a_failed_connect_names_local_network_access_and_the_https_limit`, seen failing with the hint removed). check.sh and check.sh --web passed. Not verified: the timeout and the 1006 message in Chrome (Local Network Access) or Safari (mixed content).
- 2026-10-13 — PR #16 F6: upload errors show the message only (`mcp_wire::js_error_text`, not a JS stack); a tab whose upload fails sends `close` so the bridge drops the partial file (bridge test `close_during_an_upload_drops_it`, seen failing with the discard removed; the tab side is wasm-only, compile-checked); `#mcp=` changes are picked up by a `hashchange` listener and reconnect with the new token; the `Bearer` scheme matches case-insensitively (RFC 7235; test `the_bearer_scheme_is_case_insensitive`, seen failing under the case-sensitive match). check.sh and check.sh --web passed. Not verified: the toast text, the hashchange reconnect and the partial-file cleanup in a real browser.
- 2026-10-14 — Browser Wasm MCP Lane 2: persistent Web-Crypto room capability, same-origin relay and local-bridge selector, copy/regenerate controls, Rayon RPC snapshots and main-thread reveal/replies; split browser modules below 300 lines. Eight helper mutations and a generated-JS persistence mutation failed as intended, then restored tests passed (17 client helpers, 17 relay contracts, 2 Chromium e2e). `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh --web` ended `all checks passed`; final-build e2e passed against local Wrangler. Not verified: deployment/hibernation, non-Chromium browsers, real local upload after the split, large-file lazy images or disconnect during RPC. Optional CJK web fonts unavailable.
- 2026-10-14 — Browser relay review fixes (B1–B3, S1, S3, S5, S7, S9, S10 + dep nit): oversized replies become an `isError` result under a 7.5 MiB tab budget and the worker fails only that call (not the socket); `reveal_node` waits for the UI ack (polled with a `web_time` deadline because std `Instant` panics on wasm32 — caught by the e2e) and a UI error/timeout fails the call; `notifications/*` requests with an id return -32601; the newest-ready-tab routing is documented and pinned (menu text, Node tests; the 16-tab cap already counted open sockets); plan doc updated (native-parity tools, JSON `body`, `ready` without tab title, Cloudflare Access coverage of `/mcp/*`+`/bridge/*`, eviction drops in-flight calls); `rpc_reply` status is never optional; stalled-peer bridge test renamed to what it proves; `getrandom` is out of native release builds and `hex` is dropped. Every new test seen failing under a mutation, then restored. `node --test web/worker/` 20 pass; Chromium e2e passed 3× after the rebuild (one earlier run served a stale cached wasm); `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh --web` ended `all checks passed`. Not verified: deployment/real hibernation or Cloudflare Access, non-Chromium browsers, the reveal 2 s timeout in a real browser (native test only).
- 2026-10-15 — Room-link join guard (4762d52 findings): a `#room=` link at startup or `hashchange` no longer saves/connects silently — it stages a `RoomProposal` (`mcp_join.rs`) behind a confirmation dialog (masked room, says it grants access to open documents; Stay is the Enter/Escape default, Join takes a click or Tab+Enter); declining changes nothing, the sensitive fragment key is stripped on staging even when a changed `#mcp=` pairing wins, an unchanged `#mcp=` no longer discards the link, same-room links ask nothing, and the `Use room token` field is password-masked. Pre-fix build failed the new e2e (silent join: stored room + bridge socket with no consent); five mutations (masking, precedence, strip, Enter-default, decline-joins) failed focused tests before restore; kittest covers the dialog keyboard/masking. `node --test web/worker/` 20 pass; all 3 Chromium e2e pass on the final build (Wrangler 8791); `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh --web` ended `all checks passed` (one flaky bridge_cli run passed on rerun and standalone). Not verified: the masked field's dots in a real screenshot, a same-room link in e2e (native test only), Safari/Firefox.
- 2026-10-13 — PR #16 fix round 2 (R1–R9): an uploaded `File` is tied to its own load by a pick id (`Uploads::pick/start/discard/bind/fail`, tests incl. pick A → pick B → bind A); a remote IPv6 bind also listens on `[::1]`, the banner prints after every listener is bound and the first listener error ends the bridge; the connect timeout is cleared with its connection; `js_error_text` reads a DOMException `message`; `closed_message` is pure and tested; the deploy domain is gone from the repo (`--allow-origin https://<your-deploy-domain>` prints its `Web:` line). Each test seen failing with its fix reverted. check.sh and `check.sh --web` passed. Not verified: the wasm-only wiring (`web.rs`, `main.rs`, `mcp_web.rs`) beyond the build, and any real-browser run.
- 2026-10-15 — CJK font deploy fix: a rebuild on a machine with the Noto TTC but no
  fontTools made `scripts/web-font.sh` warn and exit 0, `web-build.sh` had already
  cleared `dist/fonts`, and the deploy served `/fonts/cjk-jp.otf` as 404 (verified with
  an authenticated production fetch; not an SPA HTML fallback), so CJK names showed as
  boxes. The script now fails on missing inputs by default and only
  `FIGSTUDIO_ALLOW_MISSING_CJK=1` opts out for a local dev build; new test
  `apps/figstudio-cli/tests/web-font.rs` (missing input seen exiting 0 before the fix)
  covers failure, the opt-out and the success path (five OTTO faces, ideographs in all,
  kana in JP, Hangul only in KR, OFL license). fontTools provisioned once via
  `uv tool install fonttools`. Verified in a real (headless Chromium) browser against a
  served dist: `/fonts/cjk-jp.otf` answers 200 `font/otf` with the exact dist bytes, the
  browser decodes it (`FontFace.load`) and rasterizes あ through that face. Not verified:
  the deploy itself (owner decides; the production 404 stands until a redeploy from a
  font-ready machine), and the app's own lazy fetch when a CJK-named file is open
  (unchanged code).
- 2026-10-15 — PR18 threads-only relay fixes: the worker accepts the 202/null
  notification frame `rpc_reply` builds (was a 502 "Invalid browser reply"; the new
  worker test round-trips it, seen failing 502 !== 202, and pins that only 202 may have
  a null body), and the encode-failure fallback now carries the caller's id
  (`encode_failed_reply`; the old inline frame sent `id: null`, which the worker's
  `body.id` match can never accept). A reveal whose worker already expired is no longer
  applied by a late UI frame: the UI must win an atomic claim (pending -> claimed)
  before `apply_reveal` mutates, and the worker expires only unclaimed reveals — a
  claimed one reports its real outcome even past the deadline (native test seen failing
  `Some(Guid(1,25)) != None` before the fix). The tab bounds in-flight relay jobs at 8
  and answers the rest with the worker's own busy shape (503; native test seen failing
  `9 != 8` replies), so a burst or 30 s-timeout retries cannot stack renders on the
  shared Rayon pool; stopping a render whose call already 504'd remains open
  architecture (needs a cancel path the render loop checks, worker- or tab-initiated).
  The relay tab now sends a 30 s keepalive `ping` driven by the repaint cadence (the
  DO auto-response only ever answered pings nothing sent; plan doc corrected), and the
  worker comment no longer claims the 30 s deadline bounds eviction-dropped requests —
  a hard eviction cancels the timers with the isolate, which a new Node test pins
  (revived isolate ignores the late reply; a fresh call still works). Focused checks:
  `node --test web/worker/` 22 pass; `cargo +1.99.0 test -p figstudio` 164 pass;
  clippy -D warnings (release, default and no-default-features), fmt and a wasm32
  check of the touched app code pass (verified in an isolated worktree at 3084f07 plus
  these changes, because executor-auth work was landing concurrently in the shared
  tree). Not verified: the full `scripts/check.sh --web` gate, the browser e2e (pings
  are already tolerated by the spec), real Cloudflare eviction/keepalive behavior.
- 2026-10-16 — PR18 integration: the executor credential is wired into the Worker. The
  outer Worker now routes `/bridge/*` through `checkBridgeUpgrade`
  (`web/executor-auth.mjs`) before `idFromName`/`get`, so a caller that only knows the
  room (the MCP URL) can no longer create or reach a Durable Object through the bridge;
  the Durable Object re-verifies the same proof itself (one SHA-256; it never trusts a
  header claiming the outer Worker already checked, which would be forgeable) and its
  101 echoes only the constant `figstudio-exec-v1` marker as the selected subprotocol —
  never the offered executor hex. A socket is no longer the executor on connect:
  attachments start `{ready:false, order}` and only the `ready` frame stores
  `{ready:true, order}` (a restored hibernated attachment without `ready` — including
  one written by an earlier deploy — is ineligible), so an admitted-but-silent client
  cannot take the executor role. The forwarded 202/null notification ack now returns the
  same body-less 202 as the direct path (was a literal JSON `null` body). New gates,
  each seen failing before its fix: worker-level proof-before-DO (403 with no
  `idFromName` for missing proof, forged Origin, marker-only, or a mismatched secret),
  DO-level re-verification, ready-gating (503 before `ready`, restore-ineligible), and
  a Chromium e2e proving the caller room alone cannot open the bridge while the
  two-subprotocol proof can (`web/e2e/relay.spec.mjs`; the e2e also needed two spec
  fixes: `expect.poll(...).toBeTruthy()` resolves with undefined, so values are re-read
  after the poll, and the post-join socket assertion became a poll — the join saves
  before the reconnect frame). `sha2` 0.10 (RustCrypto) is the new dependency for the
  in-app room derivation: trusted, maintained, synchronous and wasm-safe where a
  hand-rolled SHA-256 would be review burden (ground rule 12). Residual, reported not
  solved: the proof stops caller impersonation, but anyone can still POST to well-formed
  `/mcp/<rm_…>` names or mint their own pairs and spend Worker requests and Durable
  Object invocations; a public deployment needs Cloudflare Access over `/mcp/*` and
  `/bridge/*` and/or rate limits (documented in `docs/BROWSER-WASM-MCP-PLAN.md`, no
  false safe-public claim), and an in-flight render whose call already 504'd still runs
  to completion (cancellation remains open architecture). Gates: focused
  `cargo +1.99.0 test -p figstudio` 168 pass; `node --test web/worker/` 29 pass;
  `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh --web` all checks passed (CJK fonts
  present, no opt-out); Chromium e2e 4/4 against `wrangler@4 dev` (workerd) on :8793.
  Not verified: a live deploy, Access policies, real Cloudflare eviction.
- 2026-10-16 — PR18 review round 2: two review gaps closed, verified together. (1) The
  old-socket auth bypass: a hibernated socket stored by an earlier deploy (`{order}` or
  `{ready,order}`, accepted when the room alone admitted the bridge) could be
  re-promoted by a re-sent `ready`. Attachments now carry an
  `authenticatedProtocol:"figstudio-exec-v1"` marker written only by a proved
  `connect`; the constructor closes every restored socket without the exact marker
  (`1008`), every frame requires it before processing (pending calls fail, never
  promote), `activeTab` needs marker AND `ready`, and `ready` only flips readiness —
  client-claimed auth fields are ignored. The unused `executorFromInput` JS copy (it
  disagreed with the Rust parser on duplicated fragment keys and had no production
  callers) is deleted. (2) The keepalive was send-only: a black-holed relay socket kept
  its "connected" status forever. New `mcp_heartbeat.rs` state machine (pure,
  synthetic-clock tested): pings every 30 s counted from the last pong, a 10 s
  `PONG_DEADLINE` drops the connection ("Cloud relay stopped responding"; a refused
  send drops it immediately), pongs are generation-tagged so a stale socket's answer
  never clears a new connection's wait, events drain before the deadline tick so a
  throttled background tab can only lengthen the deadline, and expiry retries at most
  once per connection that had proven itself (storm bound). The keepalive row in
  `docs/BROWSER-WASM-MCP-PLAN.md` now states the pong-timeout semantics; the local
  bridge path is untouched. Each fix seen failing first: worker red 6 failures on the
  old worker (round2-red.log), heartbeat red 8/8 on stub bodies, and the new e2e
  "keepalives go unanswered is closed by the page" red against the previous build
  (round2-e2e-red.log). Mutations killed and reverted (round2-mutations.log): M1
  constructor close, M2 frame guard, M3 activeTab check, M4 the heartbeat deadline
  branch. Gates: `node --test web/worker/` 32 pass; `cargo +1.99.0 test -p figstudio
  mcp` 61 pass; `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh --web` all checks passed
  (round2-fullgate.log; CJK fonts present, no opt-out); Chromium e2e 5/5 against
  `wrangler@4 dev` (workerd) on :8795 including the blackhole test
  (round2-e2e-green.log). Not verified: a live deploy, Access policies, real
  Cloudflare eviction and hibernation restore in production.
- 2026-10-16 — PR18 review round 3: relay loss always requests the next UI frame,
  including unproven expiry, so an idle canvas removes Connected and displays the
  disconnect notice. Refused ping sends retain the heartbeat's proof bit and retry
  once; the replacement starts unproven and cannot retry itself. The unmarked-socket
  frame guard tolerates a throwing close after failing pending calls, without granting
  readiness or authentication. Each regression failed without its own fix. Chromium
  observes a real draw in a later animation frame after self-close (not just an idle
  animation callback), and a real pong followed by injected send refusals produces
  exactly two sockets and two closes. Screenshot inspected: disconnect notice visible,
  Connected absent. The backward-clock panic claim is false: web-time 1.1.0's Wasm
  `duration_since` already uses `checked_duration_since(...).unwrap_or_default()`;
  native web-time re-exports the saturating std implementation. No clock change.
  Gates: Worker 32/32, heartbeat 8/8, foreground
  `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh --web` all checks passed with CJK fonts,
  final-build Chromium 6/6 against local workerd; fixture thumbnail sheet inspected.
  Not verified: production hibernation, real NAT expiry or background throttling,
  non-Chromium browsers, deployment or Access policies. No new dependencies.
- 2026-10-16 — PR18 review round 4 (test-only): the blackhole spec's init script wraps
  `WebSocket.close` before the paint hooks and skips a missing `GPUQueue` or WebGL2
  interface, so a browser without WebGPU still records the close instead of timing out;
  it also asserts at least one draw hook was installed. Chromium with `GPUQueue` deleted:
  old script left `atClose` unset, new one records it. The rAF-id finding is false: the
  expression-bodied arrow already returns the native id, and Chromium cancels the
  callback through it. Final-build Chromium 6/6 against local workerd. No product change.
- 2026-10-15 — Version compare Phase 2: added deterministic natural JSON and escaped script-free HTML reports with optional relative PNG links; removed reserved child rows, suppressed redundant move properties and counted ungrouped changes. Nine mutation gates and the finite-float precision regression failed as intended before passing; diff/fig crate tests (including binary roundtrips), targeted clippy and fmt passed. Natural JSON normalizes enum/byte variants and rejects nonfinite numbers. Not verified: full check.sh, browser display or large-file performance.
- 2026-10-15 — Phase 2 review fixes: nonfinite floats now round-trip losslessly through natural JSON as a sole `{"$float_bits":"<16 hex>"}` object (nested NaN inside arrays/objects preserved bit-for-bit; the HTML Debug fallback became unreachable and was deleted), and ungrouped added/removed nodes are counted in the summary alongside changed ones, with the coverage note reworded. New red/green tests for both; diff and fig crate tests, clippy and fmt pass. Not verified: full check.sh.
- 2026-10-15 — Version compare Phase 1: added the structural diff crate with global identity matching, subtree counts, moves, leaf properties and text/font deltas; the seven required tests failed before implementation and passed after it. All 21 crate tests, targeted clippy and workspace fmt passed, and FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh ended with all checks passed (exit 0). Not verified: large-file performance; serialization, rendering and app integrations remain out of scope.
- 2026-10-15 — Version compare Phase 3: added the render lane to the diff crate — both sides of a changed, added or removed page are drawn on the union-bounds canvas under a 16M-pixel budget (scaled down with a truncation note), compared per pixel at the ±8 channel threshold, with renderer counters stored in the report and PNG bytes returned to the caller; the diff image tints changed pixels red over the desaturated after side. Five render tests (recolor, unchanged, added, removed, oversized) failed before implementation and passed after; all 34 diff crate tests, targeted clippy and fmt passed. Not verified: full check.sh, PNG writing by callers, CLI integration or large-file performance.
- 2026-10-15 — Phase 3 review: uniform budget scale, max-side notes, known page-bg ink, straight-alpha PNG, worst_channel_delta, two renderers. Not verified: full check.sh.
- 2026-10-15 — Version compare Phase 4: `figstudio-cli compare <old.fig> <new.fig> -o <dir>` writes `report.json`, `report.html` and per-page `render/<guid>-{before,after,diff}.png` (paths set in the report before HTML rendering so links appear), with `--no-render`, `--json-only`, `--scale`, `--no-system-fonts` (now plumbed through `RenderOpts`); stdout is one counts line plus the artifact dir, so names/text stay off the terminal; exit codes 0/1/2 match the other commands. The HTML render section shows pixels changed, worst channel delta and canvas size. Five CLI tests (empty pair, modified pair, flags, scale, usage/errors incl. a secrets scan) and one HTML test failed before implementation and passed after; all 39 diff-crate tests, all CLI tests, targeted clippy and fmt passed. Not verified: full check.sh, large-file performance, browser display of the HTML.
- 2026-10-15 — Version compare Phase 4 review fixes: `compare` prints fixed text for unreadable inputs ("cannot read the older/newer file") and for bad arguments ("invalid compare arguments" plus usage), so no member name, path or user value is echoed; the diff render lane now reads lazily held archive images (`attach_render_diffs_with_sink` / `render_page_pair_with_readers`, drawing with `render_complete`), streams each page's PNGs to the sink before the next page renders, and stops at `RenderOpts::max_pages` (default 512) with one "not rendered" note; the CLI writes PNGs per page as they are drawn. New tests: `compare_errors.rs` (4), `render_lazy.rs` (2), `recolor_pair_shows_the_change_in_the_pngs`, success-path stderr/secret scan; each seen failing without its fix. Diff and CLI crate tests, clippy and fmt passed. Not verified: full check.sh; equivalence on a real file above the 256 MiB lazy threshold (the in-memory `lazy_images` test covers the lane logic only).
- 2026-10-15 — Version compare Phase 5: native `compare_files`/`compare_page` MCP tools in `apps/figstudio-mcp/src/tools/compare.rs` (figstudio-diff stays out of mcp-core so the wasm32 browser build cannot link the native renderer; wasm schema test lists both as explained omissions). `compare_files` writes `report.json`/`report.html` (render PNGs only with `includeRender`) under the assets dir and returns counts plus at most 100 page entries; `compare_page` re-runs the diff over registry-cached documents (no report store). Errors are fixed strings. Unit tests cover summary counts, the 100-page cap, page lookup, artifact-dir sanitization and artifact writing; an MCP round-trip test covers both tools over public fixtures. Not verified: full check.sh, rendering through the tools on a real file, large-file timing.
- 2026-10-15 — Phase 5 fixes: on-disk report.json now asserted with the diff crate's snake_case field names in tests, and `compare_page`'s page id is the `session:local` string (not serde's `[hi, lo]` array) so it matches the summary pages list and the pageId input; compare.rs split (summary/artifact helpers into `compare/summary.rs`), http/stdio tool-count tests updated to 16. All figstudio-mcp tests, fmt and clippy -D warnings pass; wasm32 no-default-features mcp-core build passes. Not verified: full check.sh.
- 2026-10-16 — Phase 5 Sol re-review: `rendering_for_path` matches `Path` equality (non-UTF8 unix names); MCP compare Guid fields are `session:local` strings (disk report.json unchanged); inline page names asserted at 128 bytes including `~`.
- 2026-10-16 — Phase 5 review-fix close-out: fixed-string load errors (no decoder text echoed), per-request tempfile artifact dirs (`compare-…`, never the predictable old name), render resolution by picked path (duplicate stems unambiguous), inline page names capped at 128 bytes with `nameTruncated` (disk report keeps full names), explicit http/stdio schema assertions for both tools, and integration tests on real fixture literals (previous counts were fabricated and never run). `cargo +1.99.0 test -p figstudio-mcp`, `clippy --all-targets -D warnings` and `fmt --check` all pass; figstudio-cli/fig/diff crate tests unaffected. Not verified: full check.sh.
- 2026-10-16 — Bot review round 2: largest-row inline trimming and name-truncation flags, off-page moved_out correction, and README alignment; regressions red then green, MCP 203 tests passed (2 ignored), diff 45 passed, fmt and touched-crate clippy passed; retention size cap (D) and TOCTOU (E) remain follow-ups; full check.sh and visual inspection not run.
- 2026-10-16 — Compare bot review fix round: page counts and move notes, precise HTML labels and single style-run rows, standalone system fonts, bounded/schema-safe MCP pages, 24-hour artifact retention and root-symlink rejection; behavior regressions seen red then green; diff 44, CLI 28 and MCP 196 tests passed (2 ignored), targeted clippy and fmt passed. Not verified: full check.sh, browser display, real-font rendering or concurrent root replacement.
- 2026-10-16 — Added the repo-local compare-fig-versions agent skill and discovery links; identical-fixture and distinct-public-fixture CLI runs verified artifact files and summary counts; full check.sh passed. Not verified: live MCP client calls or browser display of the HTML report.
- 2026-10-19 — Component property bindings: `parameterConsumptionMap` PROP_REF entries now win per field over a stale legacy `componentPropRefs` binding (a copied layer rebound to another property was hidden with the old one). Regression red then green; a real user file re-rendered and matched Figma. Other pages of that file not re-rendered.
- 2026-10-19 — macOS app bundle and releases: `scripts/macos-app.sh` builds an ad-hoc signed `figstudio.app` (app + figstudio-mcp + figstudio-cli; `--install` copies it to ~/Applications); release-please (simple strategy, TOML extra-files for the workspace version and the figstudio entries in Cargo.lock) keeps a release PR, and on release a macos-latest job attaches the zipped app, a CLI/MCP tarball and SHA256SUMS. Verified locally: bundle built, `codesign --verify --deep --strict` passed, unzipped app launched, both helper binaries ran, the TOML updaters bumped exactly the 8 version fields. Not verified: the workflow on GitHub (needs the Actions PR permission), Gatekeeper on a downloaded copy (not notarized), Intel Macs, opening a .fig by double-click.
- 2026-10-19 — PR review fixes for the macOS release: `macos-app.sh` runs under macOS Bash 3.2 without FIGSTUDIO_TOOLCHAIN (old script failed with an unbound `TC[@]`, seen red then green with a cargo stub), rejects unknown arguments, builds `--locked` and honours CARGO_TARGET_DIR; the workflow scopes permissions per job, pins actions to SHAs, hashes only the two archives and can rebuild an existing tag by hand (`workflow_dispatch`). actionlint 1.7.12 clean, real build under /bin/bash passed codesign. Not verified: the workflow on GitHub, the CARGO_TARGET_DIR path.
- 2026-10-19 — Faster gate: `scripts/check.sh` lints and tests with a new `fast` profile (release optimisation, no LTO, incremental) instead of `--release`. Thin LTO linked each of the ~100 test binaries; after a one-line change in `crates/fig`, `cargo test --release --workspace --no-run` took 20 min 24 s, while `--profile fast` takes 36 s. The whole gate after that change took 8 min 10 s, 4 min of it the LTO release build, which stays in the gate. All checks passed on Rust 1.99.1. Not measured: the full gate before the change (the test build step alone was 20 min).
- 2026-10-19 — Arc-style UI redesign (`chrome.rs`, `command.rs`, `anim.rs`, `icons.rs`, new `theme.rs`): per-file tinted gradient in light and dark, sidebar (window controls, command field, pages with a sliding pill, layer tree with rotating chevrons, file switcher dots), frameless macOS window, rounded canvas card (tiles cut at the corners), tool / status pills and a warning chip, Properties card, ⌘K command bar, peeking sidebar, eased zoom jumps, springs and fades with Reduce motion. New tests seen red first (the UI tests' labels and actions did not exist; the clip, tint, contrast, score and zoom-anchor tests failed on purposely broken code); 192 app tests, `cargo check` for wasm32, full check.sh; screenshots checked in light and dark on macOS (command bar, peek, hidden sidebar, zoom easing). Not verified: Linux and Windows look, the browser build in a real browser, window dragging from the sidebar by hand, macOS vibrancy (blocked, see P3)

