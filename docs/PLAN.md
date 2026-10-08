# figview — implementation plan

Read-only viewer for Figma `.fig` local copies, written in Rust.

This file is the progress tracker. Tick a box only after its **Done when** check has
actually passed, and add a line to the progress log at the bottom.

## Requirements (from the requester, 2026-10-08)

| Requirement | Priority | How |
|---|---|---|
| Open and view `.fig` files (no editing) | must | `crates/fig` decoder + `crates/render` + `apps/figview` |
| macOS and Linux desktop | must | eframe (egui 0.36) |
| Headless use | must | `apps/figview-cli` (info / tree / render / thumbnail) |
| Browser (wasm) | nice to have | same crates built for `wasm32-unknown-unknown`; eframe web shell |
| Vello renderer | accepted | `vello_cpu` 0.3 (CPU, runs headless and in wasm); GPU `vello` is a later option |
| UI similar to PhotoCraft | accepted | same toolkit (egui 0.36) and layout; no PhotoCraft code copied |

Out of scope for v1: editing, saving `.fig`, Figma API / login, prototyping/interactions,
variables and styles panels, plugins.

## Architecture

```
crates/fig         .fig container + Kiwi schema decode -> Document (nodes, tree, blobs, images)
crates/render      Document -> Vello CPU drawing -> Pixmap; hit test, bounds, render stats
apps/figview-cli   headless: info, tree, render PNG, extract thumbnail
apps/figview       desktop app (eframe); later also the wasm web shell
tests/fixtures     7 sample .fig files (from fig2sketch, see LICENSE-fig2sketch)
```

Rules:
- `crates/fig` and `crates/render` do no file-system or UI work beyond `Document::open`, so
  they build for wasm.
- Never panic on bad input: every malformed file returns `FigError`.
- Anything not drawn exactly is counted in `Stats` and shown to the user (app status bar, CLI).

## Phases

### P0 — Decoder (`crates/fig`)
- [x] zip and raw containers, deflate / zlib / zstd chunks
- [x] schema read from the file itself (format v20–v106 all decode)
- [x] owned `Val` tree, `Node`, parent/child order by fractional index, pages vs internal pages
- [x] embedded images, `meta.json` file name, stored thumbnail, 1 GiB size cap
- [x] tests: 7 fixtures decode, node counts match the TypeScript decoder, bad input rejected

**Done when:** `cargo test -p figview-fig` passes. ✅ 5/5 on 2026-10-08.

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
- [ ] gradient transforms verified on a real file with rotated gradients

**Done when:** every fixture renders without obvious differences from its thumbnail
(side-by-side sheet reviewed), and `cargo test -p figview-render` passes.

### P2 — Headless CLI (`apps/figview-cli`)
- [x] `info`, `tree --depth`, `render --page/--node/--scale/--background`, `thumbnail`
- [x] renders all 7 fixtures (3–366 ms each, release build)
- [ ] CLI tests (exit codes, bad arguments, output files)
- [ ] `render --all-pages` and JSON output for `info`

**Done when:** CLI tests pass and output is checked for every fixture.

### P3 — Desktop app (`apps/figview`)
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

**Done when:** on Linux the app opens every fixture, pan/zoom/select/export work, and an
Xvfb screenshot of each panel has been reviewed.

### P4 — Platforms
- [ ] Linux: release build runs on X11 and Wayland
- [ ] macOS: `cargo check --target aarch64-apple-darwin` here; real build and launch on a Mac
      or a GitHub `macos-latest` runner (needs a repo, see open questions)
- [ ] wasm (optional): `cargo check -p figview-fig -p figview-render --target wasm32-unknown-unknown`,
      then an eframe web shell that opens a dropped `.fig` file

**Done when:** each item is verified on its platform, not just compiled.

### P5 — Delivery
- [ ] README (build, run, CLI usage, known gaps)
- [ ] `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` clean
- [ ] decide repo location, push, open PR — only after the requester / owner agrees

## Verification commands

```
export PATH=$HOME/.cargo/bin:$PATH
cargo +1.99.0 test --workspace
cargo +1.99.0 clippy --workspace --all-targets -- -D warnings
cargo +1.99.0 build --release -p figview-cli
target/release/figview-cli render tests/fixtures/structure.fig -o /tmp/structure.png --scale 2
```

## Open questions

1. Repo location: new repo (e.g. `pelvo/figview`) or a crate inside `figma-read-mcp`?
   Creating a GitHub repo needs the owner's approval. Until then the work stays local.
2. macOS cannot be run on this machine; who tests on a Mac, or may we use a CI runner?
3. A real `.fig` from the requester's own work would show gaps the fixtures miss
   (variables, auto layout heavy files, many fonts).

## Risks

- Figma's text layout cannot be matched exactly; files saved without stored glyph outlines
  fall back to installed fonts, so line breaks may differ.
- `vello_cpu` 0.3 is young; filter support is limited to blur and drop shadow.
- Very large files: decoding keeps the whole message in memory; check memory on a big file.

## Progress log

- 2026-10-08 21:5x — Rust spike: kiwi-schema decode + tiny-skia render of 7 fixtures.
- 2026-10-08 22:0x — Requirements fixed (viewer, macOS + Linux + headless, wasm optional,
  Vello, PhotoCraft-style UI). Switched renderer to `vello_cpu`.
- 2026-10-08 22:1x — Workspace created. P0 done (5 tests pass). P1 renderer compiles with
  instances, clipping, masks, images, text, shadows. CLI renders all 7 fixtures; output not yet
  visually checked. GUI not started.
