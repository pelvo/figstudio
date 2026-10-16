---
name: compare-fig-versions
description: "Compare two versions of a Figma .fig file with figstudio and produce a changelog (what was added, removed, changed, moved; properties, text, fonts, shapes, render). Use for: diff two fig files, compare fig versions, fig changelog, report.html, figstudio-cli compare, compare_files, compare_page MCP tool, what changed between design versions."
---
## When to Use
Use when someone hands you two .fig files of the same project (older and newer) and wants to know what changed, or wants an HTML/JSON changelog for front end engineers. Use the CLI when you can run shell commands; it writes everything to a folder in one step. Use the MCP tools only when you are already talking to a running figstudio-mcp server and have no shell. Not for comparing two unrelated files: node matching is by Figma id (Guid), so unrelated files show as mass add/remove.

## Procedure
1. Pick the path. Shell available: use the CLI (steps 2-7). Only MCP available: use the MCP tools (steps 8-12).
2. CLI: Run commands from the repo root. Build once: `cargo build --release -p figstudio-cli`. The binary is `target/release/figstudio-cli`. Needs Rust 1.95+; if the default toolchain is older, run `cargo +1.99.0 ...` (any installed 1.95+), as with `FIGSTUDIO_TOOLCHAIN` in `scripts/check.sh`.
3. CLI: Make a NEW empty output folder each run (old files in a reused folder are not cleaned up). Then run: `target/release/figstudio-cli compare OLD.fig NEW.fig -o OUT_DIR`. Order matters: older file first, newer file second.
4. CLI: Pick flags only if needed. Default draws before/after/diff PNGs for every changed page. `--no-render` skips pictures (much faster, use for big files or a quick look). `--json-only` writes only report.json. `--scale 2` makes sharper PNGs. `--no-system-fonts` stops using installed fonts for text without stored outlines.
5. CLI: Check the result. Exit 0 = success; stdout has two lines: `pages added N, removed N, changed N, nodes added N, removed N, changed N, moved N`, then `OUT_DIR/`. Exit 1 = a read or artifact-write failure (read errors: `cannot read the older file` / `cannot read the newer file`). Exit 2 = bad arguments (`invalid compare arguments` plus usage). The CLI never prints layer names or text on purpose.
6. CLI: Read the output folder: `report.html` is the human changelog, open it in a browser, one section per page with Added, Removed, Changed and Moved out. `report.json` is the full machine record. `render/<session>-<local>-before.png`, `-after.png`, `-diff.png` (red = changed pixels) exist only for pages that were rendered; an identical pair has no PNGs. Added/removed pages have only their present side.
7. CLI: To answer questions, read report.json. Top level: `old`, `new` (file info), `summary` (snake_case counts: pages_added, pages_removed, pages_changed, nodes_added, nodes_removed, nodes_changed, nodes_moved, properties_changed), `complete`, `truncated` (notes), `pages`. Each page: `id`, `old_name`, `new_name`, `status` (Added/Removed/Changed), `added`, `removed`, `changed`, `moved_out`, `render`. Unchanged pages are omitted. Each changed node has `changes`: list of `{field, old, new, category}`; `old: null` means the field did not exist before, `new: null` means it was removed. `text` holds old/new characters and per-run font/style changes.
8. MCP: Install the server: `cargo build --release -p figstudio-mcp`; binary: `target/release/figstudio-mcp`. See [client setup](../../../apps/figstudio-mcp/README.md#client-setup). Start it with BOTH files: `target/release/figstudio-mcp --assets-dir ./figma-assets OLD.fig NEW.fig` (or use the native app with both files open). The compare tools are native only; the browser bridge does not have them.
9. MCP: Find the file keys. A key is the file name without `.fig` (the stem). If both files have the same name (common for versions, e.g. two `design.fig`), the key is the FULL PATH. If unsure, call `list_files` and use the registered full paths.
10. MCP: Call `compare_files` with `{"oldFile": "<old key>", "newFile": "<new key>"}`. Always pass both. Add `"includeRender": true` only if pictures are needed (slower, more disk). The reply has `summary` (`pagesAdded`, `pagesRemoved`, `pagesChanged`, `nodesAdded`, `nodesRemoved`, `nodesChanged`, `nodesMoved`, `propertiesChanged`), `pages` (at most 100; `pagesTotal` is the full count and `pagesTruncated` says if more exist), and `reportJson` / `reportHtml` paths on disk. Hand `reportHtml` to the user.
11. MCP: For one page's detail, call `compare_page` with `{"oldFile": ..., "newFile": ..., "pageId": "0:1"}` using an `id` from the `pages` list. Ids are strings like `0:1`.
12. MCP: If the `compare_page` reply has `truncated: true`, names were shortened and/or rows were dropped to stay under 64 KiB. Check `nameTruncated`, and `addedOmitted` / `removedOmitted` / `changedOmitted` / `moved_outOmitted` against `addedTotal` / `removedTotal` / `changedTotal` / `moved_outTotal`. For full detail, read the `reportJson` path from `compare_files` instead of calling again. Unchanged or unknown pages return a tool error.
13. Write the answer for the user: start with the counts, then go page by page: added nodes, removed nodes, changed nodes with each property old -> new, moves. Point to report.html as the shareable artifact.

## Pitfalls
- Swapped arguments invert the report: additions become removals. Older file first, always.
- Two files that are not versions of the same project match almost nothing by id and look like everything was removed and re-added. Say so instead of reporting it as a redesign.
- Reusing an output folder: a run with `--json-only` or `--no-render` leaves old report.html or PNGs from an earlier run. Use a fresh folder.
- Field name styles differ by surface: report.json uses snake_case (`pages_changed`) and status `Changed`; MCP summaries use camelCase (`pagesChanged`) and lowercase status (`changed`). MCP page detail keeps snake_case fields such as `old_name` and `moved_out`; only its status and schema ids are converted. Node and page ids are `[a, b]` arrays in report.json but `"a:b"` strings in MCP replies.
- `complete: false` in the report means some changed nodes sit outside any page and could not be grouped. Render notes such as a page scaled down or the 512 page render limit appear in `truncated`; mention them to the user.
- The render lane shows what figstudio's renderer draws, not Figma itself. Trust the property rows over the pictures; renderer approximations are counted in each page's `render.old_stats` / `new_stats`.
- `descendants` on an added or removed row excludes the root itself; it includes matched children that moved into or out of the subtree. Do not add it to the summary totals. Moves also count in `nodes_changed`; do not add `nodes_moved` to it.
- MCP: `compare_page` re-runs the whole diff each call. Do not loop over every page with it; use report.json for bulk reading.
- Privacy: these files are often private. Never paste layer names, text, colours, image hashes or file paths from the report into public places (issues, commits, chat). Describe changes in neutral terms if sharing outside the user.
- Files over 256 MiB open lazily; that is fine. Very large pages render scaled down to a 16M pixel budget; the report notes it.

## Verification
1. CLI: exit code is 0 and OUT_DIR contains report.json and report.html (only report.json with --json-only). PNGs exist only when pages were rendered.
2. Self-check from the repo root (creates a fresh output folder):
   ```sh
   mkdir -p .pi/tasks
   OUT_DIR=$(mktemp -d .pi/tasks/compare-self-check.XXXXXX)
   target/release/figstudio-cli compare tests/fixtures/structure.fig tests/fixtures/structure.fig -o "$OUT_DIR"
   ```
   Expect `pages added 0, removed 0, changed 0, nodes added 0, removed 0, changed 0, moved 0`, then the output folder. All eight JSON summary counts must be zero, including `properties_changed` (not printed on stdout). A file compared with itself has no changes.
3. report.json parses as JSON and its `summary` counts match the stdout line.
4. MCP: `compare_files` reply has non-empty `reportJson` and `reportHtml` paths that exist on disk; any `compare_page` id came from that reply's `pages` list.
