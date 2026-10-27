# figstudio — implementation plan

A Rust app for Figma `.fig` local copies. Version 1 opens and displays files; editing may
come later, so the design must not block it. (`figstudio` is a working name.)

This file holds the phases and their "Done when" checks. The checklists are one file per PR in
`docs/plans/` (see its README; `scripts/plan.sh show|pr|status`), and the log of what happened
is one file per entry in `docs/progress/`. Tick a box only after its **Done when** check has
actually passed, in your own PR's file; never edit this file for either.

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
Checklist: `scripts/plan.sh show P0` (items live in `docs/plans/`, one file per PR).

**Done when:** `cargo test -p figstudio-fig` passes. ✅ 5/5 on 2026-10-08.

### P1 — Renderer (`crates/render`)
Checklist: `scripts/plan.sh show P1` (items live in `docs/plans/`, one file per PR).

Known differences that are **not** bugs: Figma thumbnails show the canvas background
(black in these fixtures) while we use the page colour; `broken_images.fig` really has
7-byte placeholder images, so grey placeholders are correct.

**Done when:** every fixture renders without obvious differences from its thumbnail
(side-by-side sheet reviewed), and `cargo test -p figstudio-render` passes.

### P1b — Large files (requester's file cannot be shared)
Checklist: `scripts/plan.sh show P1b` (items live in `docs/plans/`, one file per PR).

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


### P2 — Headless CLI (`apps/figstudio-cli`)
Checklist: `scripts/plan.sh show P2` (items live in `docs/plans/`, one file per PR).

**Done when:** CLI tests pass and output is checked for every fixture.

### P3 — Desktop app (`apps/figstudio`)
Checklist: `scripts/plan.sh show P3` (items live in `docs/plans/`, one file per PR).

**Checked so far (Xvfb + xdotool, 1400×880):** empty state, structure.fig open, tree expand,
canvas click selects `social-twitter 1` and fills Properties, Ctrl+wheel 73%→241%,
`=` zoom, ⇧0 to 100%, bad file toast, File → Export submenus.

### P4 — Platforms
Checklist: `scripts/plan.sh show P4` (items live in `docs/plans/`, one file per PR).

**Done when:** Linux and wasm checks pass here; macOS item is ticked only after the
requester confirms it runs.

### P5 — Delivery
Checklist: `scripts/plan.sh show P5` (items live in `docs/plans/`, one file per PR).

### Deferred image formats
Checklist: `scripts/plan.sh show Deferred` (items live in `docs/plans/`, one file per PR).

### P6 — Editing (later, not started until asked)
Checklist: `scripts/plan.sh show P6` (items live in `docs/plans/`, one file per PR).

### P7 — Offline MCP server (`apps/figstudio-mcp`, plan in `docs/MCP-PLAN.md`)
Checklist: `scripts/plan.sh show P7` (items live in `docs/plans/`, one file per PR).

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
