# figstudio — implementation plan

A Rust app for Figma `.fig` local copies. Version 1 opens and displays files; editing may
come later, so the design must not block it. (`figstudio` is a working name.)

This file is the progress tracker. Tick a box only after its **Done when** check has
actually passed, and add a line to the progress log at the bottom.

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
- [ ] **visual check against Figma thumbnails** for all 7 fixtures, fix what is wrong
- [ ] render tests: pixel probes for clip, instance override, image fill, text, mask
- [ ] blend modes, inner shadow, background blur (currently skipped)
- [ ] gradient transforms verified on a file with rotated gradients

**Done when:** every fixture renders without obvious differences from its thumbnail
(side-by-side sheet reviewed), and `cargo test -p figstudio-render` passes.

### P1b — Large files (requester's file cannot be shared)
- [ ] round-trip writer (needed for P6 anyway): `Document` keeps schema + raw message,
      `Val` -> Kiwi encode, decode(encode(x)) == x on all fixtures
- [ ] synthetic stress file: duplicate fixture nodes ×N and encode a 100k+ node `.fig`
- [ ] measure decode time, peak memory, render time on the stress file; fix hot spots
- [ ] `figstudio-cli doctor <file>`: prints only structure (node type counts, feature usage,
      unsupported features, timings, memory) — no names, text or images — so the requester
      can run it on the private file and paste the output

**Done when:** the stress file opens and renders in reasonable time, and `doctor` output has
been checked to contain no document content.

### P2 — Headless CLI (`apps/figstudio-cli`)
- [x] `info`, `tree --depth`, `render --page/--node/--scale/--background`, `thumbnail`
- [x] renders all 7 fixtures (3–366 ms each, release build)
- [ ] CLI tests (exit codes, bad arguments, output files)
- [ ] `render --all-pages` and JSON output for `info`

**Done when:** CLI tests pass and output is checked for every fixture.

### P3 — Desktop app (`apps/figstudio`)
- [ ] theme and layout in the PhotoCraft style: menu bar, left tool rail, document tabs,
      canvas, right dock (Properties, Layers, Pages), status bar
- [ ] open via menu, file dialog (rfd), drag and drop, command-line path
- [ ] canvas: pan (drag / space / middle button), zoom (wheel, pinch, +/-), fit, 100%
- [ ] re-render on view change in a background thread, keep the previous frame while waiting
- [ ] click to select a layer (hit test), selection outline, hover outline
- [ ] Layers tree with expand/collapse, hidden layers greyed, click to select and reveal
- [ ] Properties: type, id, position, size, rotation, opacity, fills, strokes, effects,
      text content and font, component name for instances
- [ ] export selection or page to PNG (1x / 2x / 3x)
- [ ] CJK-capable UI font (load a system Noto/Hiragino/PingFang font when present)
- [ ] status bar: zoom, cursor position, render stats (what was skipped)
- [ ] tool rail has Move/Select and Hand only for now; edit tools slot in later

**Done when:** on Linux the app opens every fixture, pan/zoom/select/export work, and an
Xvfb screenshot of each panel has been reviewed.

### P4 — Platforms
- [ ] Linux: release build runs on X11 and Wayland
- [ ] macOS: `cargo check --target aarch64-apple-darwin` here; build steps in README;
      requester builds and launches on a Mac by hand when free
- [ ] wasm (optional): `cargo check -p figstudio-fig -p figstudio-render --target wasm32-unknown-unknown`,
      then an eframe web shell that opens a dropped `.fig` file

**Done when:** Linux and wasm checks pass here; macOS item is ticked only after the
requester confirms it runs.

### P5 — Delivery
- [ ] README (build, run, CLI usage, known gaps)
- [ ] `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` clean
- [ ] decide repo location, push, open PR — only after the requester / owner agrees

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

1. Repo location: new repo or a crate inside `figma-read-mcp`? Creating a GitHub repo
   needs the owner's approval. Until then the work stays local.
2. Final name (`figstudio` is a working name; avoid "view" since editing may come).

## Risks

- Figma's text layout cannot be matched exactly; files saved without stored glyph outlines
  fall back to installed fonts, so line breaks may differ.
- `vello_cpu` 0.3 is young; filter support is limited to blur and drop shadow.
- Very large files: decoding keeps the whole message in memory; P1b measures this.
- Writing `.fig` that Figma accepts is unverified until someone opens a saved file in Figma.

## Progress log

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
