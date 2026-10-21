---
name: compare-fig-versions
description: "Compare two versions of a Figma .fig file with figstudio and produce a changelog (what was added, removed, changed, moved; properties, text, fonts, shapes, render). Use for: diff two fig files, compare fig versions, fig changelog, report.html, figstudio-cli compare, compare_files, compare_page MCP tool, what changed between design versions."
---
## When to Use
Two .fig files of the same project (older, newer) and someone wants to know what changed. Use the CLI when you have a shell; MCP only when you are talking to a running figstudio-mcp server without one. Not for unrelated files: node ids and screen name/size matching cannot establish meaningful version history.

## Procedure
1. Build once from the repo root: `cargo build --release -p figstudio-cli` (Rust 1.95+; else `cargo +1.99.0 ...`).
2. Use a NEW empty folder: `target/release/figstudio-cli compare OLD.fig NEW.fig -o OUT_DIR`. Older file first.
3. Images default to lossless WebP; `--format png` opts out (`--format webp` is explicit). Flags: `--no-render` no pictures (fast, big files); `--json-only` only report.json; `--scale 2` sharper whole-page images (screens use their own budget; area crops use 2×); `--no-system-fonts` ignore installed fonts.
4. Exit 0 = ok (stdout: counts line, then `OUT_DIR/`); 1 = read/write failure; 2 = bad arguments. The CLI never prints layer names or text.
5. Open `report.html`: screens match by id, then kind + name + size, and compare in their own coordinates, ignoring canvas moves/re-parenting.
   Each changed screen has sharp Before/After areas, a screen locator and `+ - ~` lines; unchanged screens are hidden. Use the one global Side by side / Overlay switch (top right); whole-page pictures are folded.
   Repeated visual changes appear once under “Shared changes” (`shared_changes` / region `shared` in JSON), with links to affected screens; shared-only screens fold into a muted page summary.
   Lines belong to an area when it covers at least half their screen-space box; unmatched lines appear in screen or page “Other changes”.
6. Share the WHOLE OUT_DIR (report.html + render/), e.g. Artifact Server with `--entry report.html`. `--no-render` / `--json-only` reports have no pictures.
7. report.json: `summary`, `complete`, `truncated`, `pages[]` with raw `added`/`removed`/`changed`, whole-page `render`, `screens[]` (`matched_by`, thumbnails, `regions[].nodes` in screen space), `screens_unchanged`/`screens_moved`. Screen `nodes` and page `other_nodes` assign line ownership. Each changed node keeps `changes: [{field, old, new, category}]` (`null` = absent) and `text`/`moved`.
8. MCP: start `figstudio-mcp OLD.fig NEW.fig`, call `compare_files` with both keys (file stem, or full path if names clash), optionally `format: "png"` (compare files default to WebP), then `compare_page` by page id for detail; read `reportJson` for bulk. Compare tools are native only.
9. Answer with counts first, then page by page.

## Pitfalls
- Swapped arguments turn additions into removals.
- Unrelated files look like a total redesign; say so.
- A reused folder keeps stale report.html and images from earlier runs.
- The renderer is figstudio's, not Figma's; trust property rows over pictures.
- `complete: false` or notes in `truncated` (render limit, scaled-down pages) must be mentioned to the user.
- Privacy: files are often private. Names, text and values go only to the person who supplied the files, never into issues, commits, PRs, docs or tests.

## Verification
1. Exit code 0 and OUT_DIR holds report.html and report.json (and render/ when drawn).
2. Self-check (fresh folder, same file twice):
   ```sh
   mkdir -p .pi/tasks; OUT_DIR=$(mktemp -d .pi/tasks/compare-self-check.XXXXXX)
   target/release/figstudio-cli compare tests/fixtures/structure.fig tests/fixtures/structure.fig -o "$OUT_DIR"
   ```
   Expect all zeros in the counts line and in report.json `summary`.
