---
pr: 5
branch: feat/mcp
title: feat(mcp): add offline figstudio-mcp server for local .fig files
---

## P7 — Offline MCP server (`apps/figstudio-mcp`, plan in `docs/MCP-PLAN.md`)
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

## Progress

### 2026-10-11T000000Z b012-mcp-l4-public-read-only-token
- 2026-10-11: MCP L4: public read-only token/style model and `get_variable_defs` with all
  stored modes, bounded aliases, legacy/current refs and text-run bindings. Foreground
  `scripts/check.sh` passed; seven public fixture PNGs byte-identical at scale 0.25.
  Synthetic and SDK protocol tests pass (mutation-proven). Not checked: wasm target absent,
  private/library documents, full-scale visual inspection; instance expansion remains L5.

### 2026-10-11T000000Z b042-mcp-l1-add-the-offline-rmcp
- 2026-10-11 — MCP L1: add the offline rmcp server, stdio and Streamable HTTP,
  lazy local-file registry, node-ID parser, whoami/list_files; nine failing-first tests,
  workspace gate and clean-stdout release smoke passed. Not verified: live client setup,
  OS-signal shutdown or independent review; no renderer or decoder changes.

### 2026-10-11T000000Z b043-mcp-l1-review-nonblocking-oncecell-cache
- 2026-10-11 — MCP L1 review: nonblocking OnceCell cache, cause-bearing tool errors,
  unambiguous --http=ADDR, non-loopback warning, SIGTERM shutdown and stdio purity tests.
  scripts/check.sh passed; regression and mutation tests proven. Not verified: live clients, real Ctrl-C,
  Windows or HTTP OS signals; no renderer or decoder changes.

### 2026-10-11T000000Z b044-mcp-l2-get-metadata-page-lists
- 2026-10-11 — MCP L2: get_metadata page lists and sparse parent-relative XML, quick-xml
  escaping, depth/5 000-node bounds, Unicode text cap and instance leaves. Public fixture,
  synthetic and end-to-end tests proven by mutations; scripts/check.sh: all checks passed.
  Not verified: live clients, official capture, private files or instance expansion (L5).

### 2026-10-11T000000Z b045-mcp-l2-review-correct-figma-counter
- 2026-10-11 — MCP L2 review: correct Figma counter-clockwise rotation, omit page/document
  geometry and unsized dimensions, sanitize XML controls, mark truncated text, filter internal
  subtrees and self-close empty leaves. Regression tests proven failing first;
  scripts/check.sh: all checks passed. Not verified: live clients, official capture,
  private files or L5 expansion.

### 2026-10-11T000000Z b046-mcp-l3-get-screenshot-renders-a
- 2026-10-11 — MCP L3: get_screenshot renders a node to a base64 PNG with size and
  approximation lines (4096 px cap, base64 crate). Tests decode the PNG and were
  proven by mutations. Not verified: pixels, live clients, huge pages.

### 2026-10-11T000000Z b047-mcp-l3-review-screenshots-are-cropped
- 2026-10-11 — MCP L3 review: screenshots are cropped to the node's own box (381x191 for the
  fixture node), the 4096 px cap works below the old 0.01 scale floor and reports the real
  scale, approximations come from Stats' fields incl. culled. New tests proven by mutations;
  scripts/check.sh: all checks passed. Not verified: live clients, private files, L5 expansion.

### 2026-10-11T000000Z b048-mcp-l4-review-read-real-number
- 2026-10-11 — MCP L4 review: read real Number messages and mode sort positions, ignore
  detached style sentinels, remove f32 decimal noise, qualify cross-collection collisions,
  share font weights and preserve first-stored-mode/valued-only asset rendering. Regression
  proofs and the full gate pass; parallel clippy passes; seven scale-0.25 PNGs byte-match
  before/after and thumbnail sheet inspected. Renderer equivalence also passes on fb03bf8.
  Not verified: wasm, live clients, private/library documents, full-scale visual accuracy.

### 2026-10-11T000000Z b049-mcp-l5-public-instance-resolver-fig
- 2026-10-11 — MCP L5: public instance resolver `fig::resolve` shared by the renderer and the
  MCP tools; get_metadata expands instances (I…;… ids, overrides), and instance paths resolve
  in get_metadata, get_screenshot (instance cropped to the child box) and get_variable_defs.
  28 fixture renders byte-identical before/after, thumb sheet viewed, new tests proven by
  mutations, scripts/check.sh passed. Not verified: private/large files, screenshot box of
  scaled or resized instances, live clients.

### 2026-10-11T000000Z b050-mcp-l5-review-instance-paths-resolve
- 2026-10-11 — MCP L5 review: instance paths resolve by tree position (metadata depth
  counts tree levels), instance-child screenshots draw only that node with the renderer's
  scaled layout (Renderer::render_path / path_bounds), variable defs ignore override and
  property storage, swapped instances report their new component, the walk queues children
  lazily. Each regression failed on 4518cbc; 28 fixture PNGs byte-identical; check.sh passed.
  Not verified: private/large files, live clients, ancestor masks in child screenshots.

### 2026-10-11T000000Z b051-mcp-l6-get-design-context-writes
- 2026-10-11 — MCP L6: `get_design_context` writes React + Tailwind JSX (flex auto-layout,
  absolute children, fills/gradients, borders, radii, shadows/blurs, opacity/blend, text with
  style runs, var(--name,fallback)) for nodes and instance paths, with rendered PNG assets in
  `--assets-dir` (default ./figma-assets) or `--assets inline`, node/size caps and counted
  approximations. Built-document and fixture tests, each mutation-proven; check.sh passed.
  Not verified: live clients, private/large files, visual fidelity of the generated code.

### 2026-10-11T000000Z b052-mcp-l7-download-assets-renders-up
- 2026-10-11 — MCP L7: `download_assets` renders up to 20 nodes to PNG/JPG (JPG flattened onto
  white, same scale range and 4096 px cap as get_screenshot via a shared `render_node`) or
  returns the original image-fill bytes (`original=true`). 9 tests in `tests/assets.rs`, each
  proven by a mutation; 28 fixture PNGs unaffected (no render crate change); check.sh passed.
  Not verified: lazily opened files, very large originals, live clients.

### 2026-10-11T000000Z b053-mcp-l8-get-figjam-get-metadata
- 2026-10-11 — MCP L8: `get_figjam` = get_metadata XML plus FigJam attributes and text
  (sticky colour/text, shape type, connector endpoints and magnets, section status, code, table
  cell, widget), optional small screenshots (max 20, scale <= 0.5) and an approximations
  comment for kinds the renderer does not draw. 5 tests on an in-test board, mutation-proven;
  check.sh passed. Not verified: real FigJam files, connector geometry, table structure.

### 2026-10-11T000000Z b054-mcp-l9-adds-offline-code-connect
- 2026-10-11: MCP L9 adds offline Code Connect sidecars and local design-system discovery; mutation-proven tests and full gate passed. Not verified: live parity, Windows, cross-process/concurrent filesystem writers.

### 2026-10-11T000000Z b055-mcp-l10-adds-create-design-system
- 2026-10-11: MCP L10 adds create_design_system_rules prompt with capped local inventory; SDK protocol mutation proofs and full gate passed. Not verified: live rule-file generation or official text parity.

### 2026-10-11T000000Z b056-mcp-l9-l10-second-pass-review
- 2026-10-11 — MCP L9/L10 second-pass review: real component-set fields and variants,
  normalized/capped properties, bounded permission-preserving sidecars, requested map keys,
  snippet imports and bounded prompt names; failing-first regressions and mutation proofs;
  foreground `scripts/check.sh`: all checks passed.
  Not verified: live clients, official-server parity, private files, Windows or concurrent writers.

### 2026-10-11T000000Z b057-mcp-l12-apps-figstudio-mcp-readme
- 2026-10-11 — MCP L12: `apps/figstudio-mcp/README.md` (flags, transports, client setup for Claude Code,
  Cursor/VS Code and Codex, tool table, node ids, not-replicated list, privacy, writes, limits);
  TESTING.md section 12; IMPLEMENTATION.md map and tasks M1–M6; MCP-PLAN status. Tool list checked
  against the release binary over stdio (13 tools, 1 prompt); docs only, foreground
  `scripts/check.sh`: all checks passed. Not verified: client configs against a live client.

### 2026-10-12T093231Z b058-mcp-lazy-documents-pr-5-review
- 2026-10-12 — MCP lazy documents (PR #5 review lane 4): `FileRegistry` opens via `open_auto_above`
  (`with_lazy_above` for tests); all MCP renders use `render_complete`/`render_path_complete` with a
  per-file shared `Images` cache and on-demand `archive::read_entry`; `download_assets original=true`
  reads lazy originals after a size check against the budget. `tests/lazy.rs` proven by mutations;
  CLI render of 7 fixtures at 0.25 and 1 byte-identical to 407102c; foreground `scripts/check.sh`:
  all checks passed. Not verified: a real >256 MiB file, the wasm UI (`web.rs` unchanged).

### 2026-10-12T093231Z b059-2026-10-12-pr-5-review
- 2026-10-12 PR #5 review round 2 (figstudio-mcp): five lanes, one commit each. Text caps
  24 000 with `--max-output-bytes` (37c13f3); download_assets 10 MiB reply and 7.5 MiB per
  image (12624bf); default screenshots 2048 px (81e9dad); wrapped auto-layout
  `content-start`/`content-between` (d644547); sidecar cross-process lock (849d195). Each
  behaviour change proven by a test that fails with the change removed; foreground
  `scripts/check.sh` all checks passed before each commit. Not verified: a live Claude Code
  session against the 1.7 GB file (the reviewer's measurements are the evidence for the
  limits); the sidecar lock on network filesystems; the wasm UI (unchanged).
