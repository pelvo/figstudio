---
pr: 19
branch: feat/compare-whole-file
title: feat(diff): add whole-file version compare CLI and MCP tools
---

## Progress

### 2026-10-15T000000Z b112-version-compare-phase-2-added-deterministic
- 2026-10-15 — Version compare Phase 2: added deterministic natural JSON and escaped script-free HTML reports with optional relative PNG links; removed reserved child rows, suppressed redundant move properties and counted ungrouped changes. Nine mutation gates and the finite-float precision regression failed as intended before passing; diff/fig crate tests (including binary roundtrips), targeted clippy and fmt passed. Natural JSON normalizes enum/byte variants and rejects nonfinite numbers. Not verified: full check.sh, browser display or large-file performance.

### 2026-10-15T000000Z b113-phase-2-review-fixes-nonfinite-floats
- 2026-10-15 — Phase 2 review fixes: nonfinite floats now round-trip losslessly through natural JSON as a sole `{"$float_bits":"<16 hex>"}` object (nested NaN inside arrays/objects preserved bit-for-bit; the HTML Debug fallback became unreachable and was deleted), and ungrouped added/removed nodes are counted in the summary alongside changed ones, with the coverage note reworded. New red/green tests for both; diff and fig crate tests, clippy and fmt pass. Not verified: full check.sh.

### 2026-10-15T000000Z b114-version-compare-phase-1-added-the
- 2026-10-15 — Version compare Phase 1: added the structural diff crate with global identity matching, subtree counts, moves, leaf properties and text/font deltas; the seven required tests failed before implementation and passed after it. All 21 crate tests, targeted clippy and workspace fmt passed, and FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh ended with all checks passed (exit 0). Not verified: large-file performance; serialization, rendering and app integrations remain out of scope.

### 2026-10-15T000000Z b115-version-compare-phase-3-added-the
- 2026-10-15 — Version compare Phase 3: added the render lane to the diff crate — both sides of a changed, added or removed page are drawn on the union-bounds canvas under a 16M-pixel budget (scaled down with a truncation note), compared per pixel at the ±8 channel threshold, with renderer counters stored in the report and PNG bytes returned to the caller; the diff image tints changed pixels red over the desaturated after side. Five render tests (recolor, unchanged, added, removed, oversized) failed before implementation and passed after; all 34 diff crate tests, targeted clippy and fmt passed. Not verified: full check.sh, PNG writing by callers, CLI integration or large-file performance.

### 2026-10-15T000000Z b116-phase-3-review-uniform-budget-scale
- 2026-10-15 — Phase 3 review: uniform budget scale, max-side notes, known page-bg ink, straight-alpha PNG, worst_channel_delta, two renderers. Not verified: full check.sh.

### 2026-10-15T000000Z b117-version-compare-phase-4-figstudio-cli
- 2026-10-15 — Version compare Phase 4: `figstudio-cli compare <old.fig> <new.fig> -o <dir>` writes `report.json`, `report.html` and per-page `render/<guid>-{before,after,diff}.png` (paths set in the report before HTML rendering so links appear), with `--no-render`, `--json-only`, `--scale`, `--no-system-fonts` (now plumbed through `RenderOpts`); stdout is one counts line plus the artifact dir, so names/text stay off the terminal; exit codes 0/1/2 match the other commands. The HTML render section shows pixels changed, worst channel delta and canvas size. Five CLI tests (empty pair, modified pair, flags, scale, usage/errors incl. a secrets scan) and one HTML test failed before implementation and passed after; all 39 diff-crate tests, all CLI tests, targeted clippy and fmt passed. Not verified: full check.sh, large-file performance, browser display of the HTML.

### 2026-10-15T000000Z b118-version-compare-phase-4-review-fixes
- 2026-10-15 — Version compare Phase 4 review fixes: `compare` prints fixed text for unreadable inputs ("cannot read the older/newer file") and for bad arguments ("invalid compare arguments" plus usage), so no member name, path or user value is echoed; the diff render lane now reads lazily held archive images (`attach_render_diffs_with_sink` / `render_page_pair_with_readers`, drawing with `render_complete`), streams each page's PNGs to the sink before the next page renders, and stops at `RenderOpts::max_pages` (default 512) with one "not rendered" note; the CLI writes PNGs per page as they are drawn. New tests: `compare_errors.rs` (4), `render_lazy.rs` (2), `recolor_pair_shows_the_change_in_the_pngs`, success-path stderr/secret scan; each seen failing without its fix. Diff and CLI crate tests, clippy and fmt passed. Not verified: full check.sh; equivalence on a real file above the 256 MiB lazy threshold (the in-memory `lazy_images` test covers the lane logic only).

### 2026-10-15T000000Z b119-version-compare-phase-5-native-compare
- 2026-10-15 — Version compare Phase 5: native `compare_files`/`compare_page` MCP tools in `apps/figstudio-mcp/src/tools/compare.rs` (figstudio-diff stays out of mcp-core so the wasm32 browser build cannot link the native renderer; wasm schema test lists both as explained omissions). `compare_files` writes `report.json`/`report.html` (render PNGs only with `includeRender`) under the assets dir and returns counts plus at most 100 page entries; `compare_page` re-runs the diff over registry-cached documents (no report store). Errors are fixed strings. Unit tests cover summary counts, the 100-page cap, page lookup, artifact-dir sanitization and artifact writing; an MCP round-trip test covers both tools over public fixtures. Not verified: full check.sh, rendering through the tools on a real file, large-file timing.

### 2026-10-15T000000Z b120-phase-5-fixes-on-disk-report
- 2026-10-15 — Phase 5 fixes: on-disk report.json now asserted with the diff crate's snake_case field names in tests, and `compare_page`'s page id is the `session:local` string (not serde's `[hi, lo]` array) so it matches the summary pages list and the pageId input; compare.rs split (summary/artifact helpers into `compare/summary.rs`), http/stdio tool-count tests updated to 16. All figstudio-mcp tests, fmt and clippy -D warnings pass; wasm32 no-default-features mcp-core build passes. Not verified: full check.sh.

### 2026-10-16T093900Z b121-phase-5-sol-re-review-rendering
- 2026-10-16 — Phase 5 Sol re-review: `rendering_for_path` matches `Path` equality (non-UTF8 unix names); MCP compare Guid fields are `session:local` strings (disk report.json unchanged); inline page names asserted at 128 bytes including `~`.

### 2026-10-16T093900Z b122-phase-5-review-fix-close-out
- 2026-10-16 — Phase 5 review-fix close-out: fixed-string load errors (no decoder text echoed), per-request tempfile artifact dirs (`compare-…`, never the predictable old name), render resolution by picked path (duplicate stems unambiguous), inline page names capped at 128 bytes with `nameTruncated` (disk report keeps full names), explicit http/stdio schema assertions for both tools, and integration tests on real fixture literals (previous counts were fabricated and never run). `cargo +1.99.0 test -p figstudio-mcp`, `clippy --all-targets -D warnings` and `fmt --check` all pass; figstudio-cli/fig/diff crate tests unaffected. Not verified: full check.sh.

### 2026-10-16T093900Z b123-bot-review-round-2-largest-row
- 2026-10-16 — Bot review round 2: largest-row inline trimming and name-truncation flags, off-page moved_out correction, and README alignment; regressions red then green, MCP 203 tests passed (2 ignored), diff 45 passed, fmt and touched-crate clippy passed; retention size cap (D) and TOCTOU (E) remain follow-ups; full check.sh and visual inspection not run.

### 2026-10-16T093900Z b124-compare-bot-review-fix-round-page
- 2026-10-16 — Compare bot review fix round: page counts and move notes, precise HTML labels and single style-run rows, standalone system fonts, bounded/schema-safe MCP pages, 24-hour artifact retention and root-symlink rejection; behavior regressions seen red then green; diff 44, CLI 28 and MCP 196 tests passed (2 ignored), targeted clippy and fmt passed. Not verified: full check.sh, browser display, real-font rendering or concurrent root replacement.
