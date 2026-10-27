# figstudio — implementation plan

A Rust app for Figma `.fig` local copies. Version 1 opens and displays files; editing may
come later, so the design must not block it. (`figstudio` is a working name.)

This file is the progress tracker. Tick a box only after its **Done when** check has
actually passed, and add a progress entry (`scripts/progress.sh new <slug>`, see
`docs/progress/README.md`; never edit this file for it).

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
- [x] browser text without stored outlines uses supplied UI/CJK/symbol fonts on workers
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
- [x] internal pages open from a collapsed "Internal pages (N)" section in the Pages list;
      mouse and keyboard selection use the same navigation as normal pages
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
      status pills; Properties as a card; ⌘L command bar (commands, files, pages, layers);
      hidden sidebar peeks from the left edge; zoom jumps ease around their anchor; springs
      and fades, View → Reduce motion
      (the browser follows `prefers-reduced-motion`)
- [x] spaces (2026-10-19): per file a preset or custom colour, an emoji or icon, a name;
      edited in Arc-style popovers from a space dot's menu (or the Space menu / command bar),
      saved per file on this computer, never in the `.fig`; two-finger swipes over the
      sidebar move between spaces
- [x] ⌘F finds text on the page (inside instances too), highlighted; ⌘L opens the command bar
- [x] paste a figma.com or `figstudio://` URL into the command bar, then Enter or click
      **Open Figma link** to open the matching local copy and reveal its node; use the existing
      link window for progress, ambiguous matches and failures
- [x] Settings (⌘,) in Arc's layout: General (appearance, Reduce motion, translucent sidebar,
      panels), Canvas (renderer for the next start, frame names, scroll wheel pans or zooms),
      Shortcuts (searchable, keycaps; every shortcut can be changed, removed or reset),
      Agents (agent access), About
- [x] desktop Agent access, remote access and bind address survive a restart; older saved
      preferences still default to access off, and bearer tokens are not stored
- [x] desktop open files/spaces reopen after restart in saved order, with the active space
      restored; missing files do not block others, and closed spaces stay closed
- [x] persistence review: requested MCP enablement survives plan/bind/runtime failures, with
      a live minted-token test proving credentials are not persisted
- [x] persistence review: workspace paths use a separate private atomic store; malformed or
      oversized workspace metadata cannot reset other preferences
- [x] persistence review: saved files and startup decoders are bounded; canonical aliases,
      interim closes and failed-active files preserve the correct restore/focus intent
- [x] persistence re-review: failed private migration retains restorable legacy metadata
      without blocking other preference saves; successful manual aliases keep focus during restore
- [x] persistence warning re-review: unreadable private metadata explains current-session loss
      and repair/restart for future saves; alias coverage includes third-tab focus while loading
- [x] persistence warning follow-up: oversized legacy fallback retains recovery guidance
      alongside the saved-path limit warning without overwriting damaged metadata
- [x] the window, panel widths, sidebar state and appearance survive a restart (eframe
      persistence)
- [x] macOS vibrancy (translucent sidebar, 2026-10-19): an `NSVisualEffectView` behind the
      content view of a transparent window (`vibrancy.rs`; one `unsafe` block, approved by the
      owner); the gradient is 60% opaque over it. Checked by hand in light and dark
- [x] errors shown as toasts (bad file stays non-fatal)
- [x] File → Open Figma link… (Cmd+Shift+L by default): paste a figma.com link; the matching tab, or the
      `<file name>.fig` in `~/Downloads` (`FIGSTUDIO_LINK_DIRS`, or a chosen folder), is opened
      and the node selected and fitted; when several files match equally, the window lists them
- [x] onboarding (2026-10-20): a welcome sheet on the first launch (the app, spaces, the
      command field) and one-time tips beside the command field, the space dots and the
      sidebar button; done when dismissed or used; Help → Welcome tour shows them again
- [x] Recents page (2026-10-20, desktop): with no file open, the files opened before as
      cards in their space colours with a picture of their page (or rows), a filter, Open,
      Show in Finder, Remove from recents; moved files marked missing
- [x] Recents page in the browser (2026-10-21): the list, handles and pictures in
      IndexedDB; Chrome/Edge open a card from its file handle (picker in its folder if the
      file moved), Firefox/Safari open the picker with a toast naming the file
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

### Deferred image formats
- [ ] JPEG XL output: deferred because mature encoders are GPL/AGPL, incompatible with figstudio's Apache-2.0 distribution; no JXL code planned.
- [ ] WebP compression: libwebp would give ~40% more saving; swap behind `figstudio_render::encoding::encode` if needed (owner approval required).

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
- [x] `reveal_node` with a figma.com link whose file no tab shows: the app opens the local copy from its
      link folders, then reveals the node (`Command::OpenLink`)
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

Moved to `docs/progress/`: one file per entry, no shared file to conflict on. Print the
timeline with `scripts/progress.sh list`; add an entry with `scripts/progress.sh new <slug>`.
