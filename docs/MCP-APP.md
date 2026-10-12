# MCP inside the app (issue #9): implementation plan

Order: **L0 → L1 → L2 → L3 → L4 → L5 → L6 → L7 → L8 → L9**. One worker, one lane at a time; each lane ends
with its own gate and one commit (Conventional Commits, PII check per AGENTS.md). Web (Phase 3) is out of scope.

## Why this order

- **Phase 2b goes early (L2–L3).** It is app-only and gives users value right away. It needs only the link format and
  parser, and L1 puts those in `crates/mcp-core`. The standalone server reads the copied links from L1 on.
- **Phase 1 (L4–L5) still comes before Phase 2 (L7–L8)**, as the issue says.
- **L0 is first.** The golden harness must exist before any MCP code moves, so it records the "before" output.
- **L6 (region tools) comes after Phase 1.** It reuses the region function from L3 once it lives in core.

## Owner decisions (answers on the issue)

1. **Both link forms are accepted.**
   - `figstudio://<fileKey>?node-id=1-42`
   - `figstudio://<fileKey>?page=0-1&rect=x,y,w,h`
   - `https://(www.)figma.com/(design|file|board)/<key>/<slug>?node-id=…`
   - File choice: an explicit `fileKey` argument wins, then the link's file. A `figstudio://` file that matches no open
     file is a tool error.
   - A figma.com link maps to a file when `<key>` equals a file's stem, or `<slug>` equals the stem with spaces turned
     into `-`. Otherwise its file part is ignored and today's rule applies (the single file, or the active tab).
     Existing figma.com links keep working.
2. **The embedded server is off by default.** A toggle starts it. Its state is not saved: every launch starts off.
3. **Copy region copies everything:** the link, node IDs and names, and a PNG.
   - egui's clipboard holds one item, text **or** image: `ctx.copy_text` / `ctx.copy_image`.
     - Desktop: egui-winit → arboard set_image. `image-data` is already enabled; see `cargo tree -i arboard`.
     - Web: eframe writes `image/png` through `ClipboardItem` (eframe-0.36.2 `web/mod.rs:285`).
   - So the menu has two items: **Copy region** (text) and **Copy region as PNG**.
   - Failures are announced with a toast, never a panic:
     - On the desktop, egui-winit only logs a failed image copy (some Wayland setups cannot copy images). The toast
       says "Copied W×H image" and its hover text mentions this limit.
     - On the web, Safari may refuse an async write once the user gesture has expired. If the copy cannot start, the
       toast offers "Download PNG" (`web::download`).
   - Copying text and PNG in one item would need our own `ClipboardItem` with both MIME types. That is a later web
     task, not part of this plan.

## Invariants (every lane)

- **MCP output stays the same.** In L1, L4 and L5 the MCP output stays byte-identical, proven with the golden diff.
  `apps/figstudio-mcp/tests/**` stays unchanged in those lanes, apart from new tests. Only L6 and L7 may change tool
  descriptions and schemas.
- **The app is unchanged with the feature off or the toggle off.** No thread, no socket, no snapshot work.
- **Bad input never causes a panic.** Bad links return `CoreError`/`McpError`; bad app input shows a toast.
- No new `unsafe`.
- Source files stay at ≤ ~300 lines. `main.rs` (1053 lines) and `panels.rs` (469 lines) must not grow by more than
  about 15 lines each; new code goes in new modules.
- `doctor` is untouched. No MCP or clipboard data goes into `status_json` (`main.rs:705`) or stderr.
- No PII:
  - Tests use `tests/fixtures/*` or documents built in the test.
  - Docs examples use generic names ("Button / Primary").
  - Links contain the file **stem**, never a full path.
- Rust guidelines (M-* IDs, cited in commits):
  - canonical error structs (M-ERRORS-CANONICAL-STRUCTS);
  - `Debug` on public types (M-PUBLIC-DEBUG);
  - `#[expect]` instead of `#[allow]` (M-LINT-OVERRIDE-EXPECT).

## Shared gates

- **G-check:** `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh` must end with `all checks passed`.
- **G-wasm:**
  `cargo +1.99.0 build -p figstudio-mcp-core --no-default-features --target wasm32-unknown-unknown`.
  - Install the target once: `rustup target add wasm32-unknown-unknown --toolchain 1.99.0`.
  - L1 adds this to `check.sh` as a step that prints `skipped: …` when the target is missing, like `--cross` does.
- **G-golden:** `scripts/mcp-golden.sh <base-rev>` (from L0). The `diff -r` must be empty.
- **G-web** (lanes that touch the app): `scripts/check.sh --web` (pinned nightly), or a note that it was not run.
- **G-prove:** for every new test, write down the mutation that makes it fail, run it, and revert it
  (`docs/TESTING.md`, "Prove the test").

---

## L0: golden harness for the MCP tools (S, Haiku with this spec / Sonnet)

- **Goal:** record what every tool returns before anything moves.
- **New file: `apps/figstudio-mcp/tests/golden.rs`** (`#[ignore]`).
  - Starts `FigstudioServer` over `tokio::io::duplex` (as `tests/prompts.rs:22` does) with `Assets::Inline`.
  - Registers the 7 fixtures by **relative** path `../../tests/fixtures/*.fig`, so paths match across worktrees.
  - Calls, in order:
    - `tools/list`, `prompts/list`, `whoami`, `list_files`.
    - Per fixture:
      - `get_metadata` without `nodeId`, then with each page id at depth 2;
      - `get_screenshot` for each page at scale 0.25, and for the first 3 frames;
      - `get_design_context` and `get_variable_defs` for the first 3 frames;
      - `download_assets` (first 5 nodes, png);
      - `search_design_system("a")`, `get_libraries`, `get_figjam`;
      - `get_context_for_code_connect` (first component), `get_code_connect_map` (read-only);
      - one invalid id, `"x:y"`, per tool.
  - Node ids are taken from the metadata XML in document order, so the set is deterministic.
  - Writes each `CallToolResult` as pretty JSON to `$FIGSTUDIO_GOLDEN_DIR/<fixture>/<nn>-<tool>.json`.
- **New file: `scripts/mcp-golden.sh <rev>`.**
  - Runs `git worktree add target/golden-base <rev>`.
  - Runs the test in both trees (`--release --test golden -- --ignored`).
  - Diffs the two output directories, then removes the worktree.
- **Determinism:** run it twice on HEAD (`<rev>`=HEAD); the diff must be empty. Fix any nondeterminism in the harness,
  not in the tools.
- **Gate:** G-check, and G-golden against HEAD is empty.
- **Prove:** change one character in a description in `tools/metadata.rs`; the diff must show it.
- **Status:** done. Harness `apps/figstudio-mcp/tests/golden.rs`, FigJam board `tests/golden_figjam.rs`, script
  `scripts/mcp-golden.sh`. Run `scripts/mcp-golden.sh HEAD` (exit 0 = identical, 197 records; logs in `target/golden-*.log`).
  - **Recorded:** tools/list, prompts, whoami, list_files; per fixture: metadata, screenshots, design context, variable
    defs, `download_assets` (png and `original=true`), search, libraries, `get_figjam` (plain and `includeImages`),
    Code Connect context and map, and one `x:y` invalid id per node-taking tool. The 8th file is the synthetic FigJam
    board `apps/figstudio-mcp/figjam-golden.fig` (git-ignored, written by `golden_figjam.rs` before each run).
  - **Not recorded, by design:** `add_code_connect_map` (writes a sidecar next to the fixture).
  - **Determinism:** repeated runs on the same tree are identical; the board's bytes are identical across runs.
  - **Mutation proofs:** a tool description (tools/list); an output text (7 invalid-get_metadata records); the FigJam
    section status (both `get_figjam` records); the original-image MIME label (`original=true` records). Each diff
    showed the change and was reverted.
  - **Known gaps the golden shows:** `get_figjam` prints no text for the code block. The reader reads `sourceCode`;
    the fixtures' embedded schema has no such field (their code text is in `textData.characters`), but newer Figma
    schemas do have `sourceCode`. Check a real modern FigJam file before changing the reader: a fix should read
    `sourceCode` **or** `textData.characters`, not one alone. A fix will show as a golden diff. The harness always passes
    an explicit `imageScale`, so a change to `DEFAULT_IMAGE_SCALE` is not recorded.
  - **Not verified:** cross-machine byte equality; non-macOS.

## L1: `crates/mcp-core` seed with node IDs and links (M, Sonnet)

- **New crate `figstudio-mcp-core`** at `crates/mcp-core/` (workspace `crates/*` already includes it). Add a
  `[workspace.dependencies]` entry.
- **Dependencies:** `figstudio-fig`, `url`, `percent-encoding` (2.3.2 is already in `Cargo.lock`, so no new crate).
- **Move `apps/figstudio-mcp/src/node_id.rs` → `crates/mcp-core/src/node_id.rs`.**
  - The adapter keeps `pub mod node_id { pub use figstudio_mcp_core::node_id::*; }`.
- **New `CoreError`** (`crates/mcp-core/src/error.rs`): `InvalidNodeId`, `FileKey`, `Open(FigError)`,
  `InvalidLink`, each with a backtrace.
  - `McpError` (`apps/figstudio-mcp/src/error.rs`) gains `Kind::Core(CoreError)` plus `From`. It keeps the tokio
    `JoinError` variants.
  - The `Display` text must stay **identical**, and so must `is_invalid_node_id` / `is_open`.
- **New `crates/mcp-core/src/link.rs`:**
  - `pub struct Link { file: Option<FileHint>, node: Option<NodeId>, page: Option<Guid>, rect: Option<[f64; 4]> }`
  - `enum FileHint { Exact(String) /*figstudio://*/, Figma { key: String, slug: Option<String> } }`
  - `parse_link(&str) -> Result<Link, CoreError>`
  - `format_node_link(key, &NodeId) -> String` uses `1-42`, or `I1-2;3-4` percent-encoded.
  - `format_region_link(key, page, [x, y, w, h])`
- **Encoding and validation rules:**
  - The file key goes into the URL host percent-encoded (`NON_ALPHANUMERIC` minus `-._~`), so spaces, `/`, `?`, `#`,
    `%` and non-ASCII names round-trip.
  - The rect must be 4 finite numbers with w,h > 0 and |v| ≤ 1e7; formatting rounds to 2 decimals.
  - A node-id and rect pair, or duplicate keys, give `InvalidLink`.
- **`parse_node_id` accepts `figstudio://` too** and returns only the node. A region link is `InvalidNodeId` here.
- **Adapter (`apps/figstudio-mcp/src/lib.rs`):** add `FigstudioServer::file_for(file_key, node_id) -> Option<String>`,
  which applies the decision-1 rule against `FileRegistry::keys()`.
  - Call it at every `self.files().open(...)` / `key(...)` / `rendering(...)` site: `tools/{metadata, screenshot,
    variables, figjam, design_system, assets, codeconnect}.rs` and `design/mod.rs:78-97`, about 10 sites.
  - Inputs without links take the same path as before, so G-golden stays empty.
- **Tests (in `crates/mcp-core/tests/link.rs` and `apps/figstudio-mcp/tests/links.rs`):**
  - Round trip `parse(format(x)) == x` for odd keys and instance paths; old inputs still parse.
  - Rejects: `figstudio://` without a query, `rect=1,2,3`, NaN, `w=0`, `http://figma.com.evil/…`.
  - With two fixtures registered and no `fileKey`, `get_metadata` given `figstudio://sections?node-id=…` reads
    sections.
  - A figma.com URL with an unmatched key still works for a single file.
- **Prove:** drop the percent-encoding, and the space round trip fails; skip `file_for`, and the two-file test fails.
- **Gate:** G-check, G-wasm (new check.sh step), G-golden empty against L0.
- **Status:** done. `crates/mcp-core` (`error.rs`, `link.rs`, `node_id.rs`), adapter `apps/figstudio-mcp/src/file_for.rs`,
  tests `crates/mcp-core/tests/link.rs` and `apps/figstudio-mcp/tests/links.rs`, wasm step in `scripts/check.sh`.
  G-golden against 72083b7: identical (197 records).
  - **Grammar:** `figstudio://<percent-encoded key>?node-id=<id>` or `?page=<a-b>&rect=x,y,w,h`. Exactly one of the two
    forms; any other parameter, path, userinfo or port is `InvalidLink`. `node-id` is `1-42`, `1:42` or `I1-2;3-4`. figma.com
    (and www): `/design|file|board/<key>/<slug>?node-id=…`; other query parameters are ignored, and a URL without a
    file part gives `file: None`. Other hosts and schemes are `InvalidLink`.
  - **File rule (`file_for`):** explicit `fileKey` > `figstudio://` key (passed on as is, so an unknown key is the usual
    file-key tool error) > figma.com `<key>` equal to a stem > `<slug>` equal to a stem with spaces turned into `-` >
    `None` (today's rule). Multi-node `download_assets` uses its first node ID. `get_code_connect_map` /
    `add_code_connect_map` use `nodeId` too.
  - **Region links:** `format_region_link` returns `Result` (a rect that rounds to w or h = 0 is refused).

## L2: right-click Copy in Layers, canvas and Properties (M, Sonnet; Haiku workable)

- **App dependency:** `figstudio-mcp-core` for all targets. Wasm needs nothing extra: core has no tokio.
- **New `apps/figstudio/src/copy.rs`:**
  - Pure formatters:
    - `name(node)`
    - `text(node)`: the full `textData.characters`, not the 400-character preview at `panels.rs:289`
    - `node_id(id)`
    - `link(file_key, id)`, which calls `link::format_node_link`
    - `agent_block(tab, id)`, the 3-line block from the issue
  - `file_key(tab)` = the stem of `tab.path`, falling back to `doc.file_name`.
  - `node_menu(ui, tab, id) -> Option<String>`: the egui menu body that returns the chosen text. The caller runs
    `ctx.copy_text` and shows a toast ("Copied link").
  - Optional item **Copy properties as JSON**, only once L5 exposes `get_metadata` depth 0 as a core function.
- **Hook-ups:**
  - Layers: `panels.rs:118` (`flat_row`) and `panels.rs:130` (`row`). Call `response.context_menu(...)` on the row's
    response, which already exists.
  - Properties header `panels.rs:220-225` and the Text section `panels.rs:286-292`, plus a "Copy" button next to the
    text.
  - Canvas `canvas.rs:113`: on `response.secondary_clicked()`, `tab.select(tab.hover)` first, then
    `response.context_menu`.
- **Toasts:** route copy results through the existing `App::toast` (`main.rs:221`); return them through a small
  `Option<String>` out-param or a field on `Tab`.
- **Selectable labels:** egui 0.36 defaults `selectable_labels: true` (egui-0.36.2 `style.rs:1489`).
  - Check by hand whether Properties labels already select. If not, find the cause (a `Sense` or the scroll area)
    and fix it.
- **Instance paths:** layers and canvas show only document nodes (`Tab.selection: Option<Guid>`). "Copy node ID" uses
  the instance path only where one is known; L2 has none. Note this in `docs/PLAN.md`.
- **Tests:** unit tests in `copy.rs`, on a document built in the test and on `structure.fig`:
  - Every formatter.
  - `parse_link(copy::link(..))` round trip back to the same file and node.
  - `copy::text` keeps 1000+ characters intact.
- **Prove:** truncate `text` to 400; break the link format.
- **Gate:** G-check, G-web (or noted), and a manual check: right-click in each of the 3 places, paste into an editor.
- **Status:** done (commit `feat(app): right-click copy for name, text, id, link and agent block`).
  - **Files:** `apps/figstudio/src/copy.rs` (pure formatters), `copy_menu.rs` (egui glue), `copy_tests.rs`; hooks in
    `panels.rs`, `canvas.rs`, `tab.rs` (`pending_copy`) and `main.rs` (copy and toast).
  - **Menu:** Copy name, Copy text (TEXT nodes only, full `textData.characters`), Copy node ID, Copy link, Copy for
    agent. Copy properties as JSON is not added: it waits for L5.
  - **Selectable labels:** egui 0.36 defaults `selectable_labels: true`, and the theme does not change it, so the
    Properties values were already selectable. Only the Text section needed a Copy button.
  - **Proved:** truncate `text` to 400 (caught), break the link format (caught by 3 tests), drop the multi-line name
    flattening (caught), list Copy text for non-text nodes (caught). Each reverted.
  - **Gates:** G-check passed, G-web exit 0, G-golden identical (197 records), PII scan clean for this lane.
  - **GUI now driven headlessly** (`apps/figstudio/src/ui_tests.rs`, egui_kittest, see `docs/TESTING.md` section 7b): the right-click on a node opens Copy name / node ID / link / for agent / as PNG, and Copy link and Copy for agent put the exact text on egui's clipboard output. Still not covered: the real OS clipboard and paste into an editor/Preview, a real window manager and window focus.
  - **Not verified:** a real right-click in the window (Computer Use did not bind the app); paste into an editor; the
    web build's clipboard (`ctx.copy_text` on web was not exercised).

## L3: marquee and Copy region, text and PNG (M/L, Sonnet)

- **New `apps/figstudio/src/marquee.rs`.** The Select tool is not panning (`canvas.rs:73-76`), so a primary drag
  draws a marquee:
  - It is stored on `Tab.marquee: Option<egui::Rect>` in page coordinates.
  - Esc, a click or a page switch clears it.
  - `canvas.rs` gets only the calls (≤ 15 lines); drawing and hit logic live in `marquee.rs`.
- **Right-click inside the marquee** opens a region menu; outside it opens the node menu (L2).
- **New `crates/mcp-core/src/region.rs`:**
  - `nodes_in(doc, page, rect, limit=50, budget) -> Region { nodes: Vec<(Guid, name, kind)>, total, truncated }`
  - Breadth-first from the page's children, so top-level nodes come first. It skips invisible nodes and uses
    `figstudio_render::node_bounds` intersecting the rect.
  - A work budget stops it after 200k visited nodes with `truncated=true`, for big files.
  - This requires a core → `figstudio-render` dependency with `default-features = false`. That feature setting is
    wasm-safe.
- **Copy region (text)** gives the region link, the file and page lines, and `nodes (k of N, top-level first):` with
  one row per node: `1:42 "Name" (KIND)`.
- **Copy region as PNG:**
  - Render the page node with a viewport cropped to the rect at the current zoom.
  - Caps: longest side 4096 px and 16 MP total; the scale steps down to fit.
  - The render runs off the UI thread:
    - Desktop: `std::thread` like `Workers::export` (`worker.rs:384`) with the same `begin_export` guard.
    - Web: `Workers::run_export` (`worker.rs:518`).
  - Result path: channel, then `ctx.request_repaint()`, then `ctx.copy_image(ColorImage::from_rgba_premultiplied)`.
    Vello pixmaps are premultiplied.
  - The toast reports the size plus the render's `Stats` gaps, the way the status bar does (`main.rs:761`).
  - Also add **Copy as PNG** to the node menu (reuses `export_frame`, `main.rs:70`, at 2×).
- **Tests:**
  - `region::nodes_in` on a document built in the test: order, limit, invisible nodes skipped, budget cut.
  - Rect normalisation for a drag in any direction.
  - The region link round-trips through `parse_link`.
  - The PNG-size cap function.
- **Prove:** reverse the BFS order; remove the cap.
- **Gate:** G-check, G-wasm, G-web; manual on macOS: paste the PNG into Preview and the text into an editor.
- **Status:** done (commit `feat(app): marquee and copy region as text or PNG`).
  - **Files:** `apps/figstudio/src/marquee.rs` (drag, draw, hit logic), `region_copy.rs` (menu + `region_text`),
    `region_png.rs` (size cap `plan`, off-thread render, clipboard, web fallback), tests beside each, and
    `crates/mcp-core/src/region.rs` (`nodes_in`, tests in `crates/mcp-core/tests/region.rs`). Core depends on
    `figstudio-render` with `default-features = false`; the wasm32 core build passes.
  - **Behaviour:** Select tool + primary drag draws the marquee (kept if ≥ 4 screen points a side). Esc, a click or a
    page switch clears it. Right-click inside it opens **Copy region** / **Copy region as PNG**; outside it, L2's node
    menu. The node menu also has **Copy as PNG** (2x, within the same caps).
  - **Region text:** `figstudio://<stem>?page=<g>&rect=x,y,w,h`, then `file: <title> · page: <name>`, then
    `nodes (k of N, top-level first):` and rows `1:42 "Name" (KIND)`, at most 50 (`N` gets a `+` when the 200k visit
    budget ran out).
  - **PNG:** longest side ≤ 4096 px, ≤ 16 MP, scale stepped down; zoom × pixels-per-point; premultiplied RGBA to
    `ctx.copy_image`. Toast: `Copied W×H image · ⚠ <gaps>`; desktop adds a second line that a failed copy is only
    logged by egui-winit. Web: if there is no secure context or `ClipboardItem`, the PNG is downloaded instead.
  - **Proved:** reverse the BFS order, ignore `visible`, drop the budget, drop the pixel cap, drop side/pixel clamps,
    swap the drag rect for a non-normalising one: each caught, reverted.
  - **GUI now driven headlessly** (`apps/figstudio/src/ui_tests.rs`, egui_kittest, see `docs/TESTING.md` section 7b): a primary drag draws the marquee, a right-click inside it offers Copy region / Copy region as PNG (text starts with the `figstudio://…?page=…&rect=…` link and lists nodes; the PNG reaches the clipboard output as an image), a right-click outside opens the node menu. Still not covered: the real OS clipboard and paste into an editor/Preview, a real window manager and window focus.
  - **Not verified:** a real drag and right-click in the window; paste into Preview; the web clipboard and the
    download fallback in a browser; Safari's refusal after the gesture expires (cannot be detected up front).

## L4: Phase 1a, move the text tools to core (L, Sonnet; Haiku only if split per file)

- **Core output type** (`crates/mcp-core/src/output.rs`):
  - `pub struct Output { pub content: Vec<Block>, pub structured: Option<Value>, pub is_error: bool }`
  - `enum Block { Text(String), Image { data_b64, mime }, Blob { data_b64, uri, mime } }`
  - Constructors mirror the only rmcp ones in use: `success`, `error`, `structured` (rmcp-3.5.1 `model.rs:4155`; its
    content text is `value.to_string()`).
  - The adapter has `fn to_call(Output) -> CallToolResult` (≈ 25 lines). These blocks are all the ones in use: text
    ×28, image ×5, resource ×1 (`assets.rs:369`).
- **Move the logic** into `crates/mcp-core/src/{metadata/*, figjam, design_system, variables/*, codeconnect_context}`:
  - `tools/metadata.rs` and `metadata/{figjam, node, pages, resolved}.rs`
  - `tools/figjam.rs`, `tools/design_system.rs`
  - `tools/variables.rs` and `variables/format.rs`
  - `tools/codeconnect/context.rs`
- **What the adapter files keep:**
  - the `#[tool_router]` blocks and the `JsonSchema` input structs (schemas stay in the adapter);
  - `spawn_blocking`;
  - wrappers with the **old public signatures** returning `CallToolResult`, e.g. `tools::metadata::metadata`,
    `metadata_within`, `figjam`, `variable_defs` and `design_system::{search, summary}`, so the tests compile
    unchanged.
- **Code Connect sidecars** (`codeconnect/sidecar.rs`) and `persist_file` stay in the adapter: they are file storage,
  not document logic.
- **Core deps added:** `serde`, `serde_json`, `quick-xml`. `schemars` and `rmcp` stay out.
- **Gate:** G-check, G-wasm, G-golden empty against L3's commit, and the MCP tests unchanged and green. Note the change
  in line counts in the commit body.
- **Status:** done. Core: `output.rs` (`Output`, `Block`, `text_first`), `metadata.rs` + `metadata/{figjam,node,pages,resolved}`,
  `figjam.rs` (`board`: XML, pictured ids, validated scale), `design_system.rs`, `variables.rs` + `variables/format.rs`,
  `codeconnect_context.rs`; deps `serde_json`, `quick-xml`. Adapter: `tools::to_call(Output)` and old-signature wrappers.
  **Deviation:** `get_figjam`'s picture loop stays in the adapter (needs `Rendering`/`screenshot`, which move in L5);
  `Block::Blob` is defined but unused until L5. Sidecars and `persist_file` stay in the adapter as specified.

## L5: Phase 1b, move the rendering tools to core (L, Sonnet)

- **Move to core:**
  - `tools/rendering.rs`: `Rendering` keeps `new(path, images)` and `Default`. It reads lazy images with
    `std::fs::File`, which compiles on wasm and fails at run time there.
  - `tools/screenshot.rs`
  - `tools/assets.rs` and `assets/originals.rs`
  - `tools/design/*` (`mod`, `walk`, `jsx`, `layout`, `paint`, `style`, `text`, `assets`)
- **Core features:**
  - `default = ["fs", "system-fonts"]`
  - `fs = ["dep:tempfile"]`: `Assets::Dir` writes. Without it, `Assets::Dir` gives an error result
    "asset files are not available in this build", and `Inline` works.
  - `system-fonts = ["figstudio-render/system-fonts"]`
- **Core deps added:** `base64`, `image`, `html-escape`.
- **Adapter re-exports** keep the test paths (`figstudio_mcp::tools::rendering::Rendering`, `tools::design::{Assets,
  Options, design_context}`, `tools::assets::IMAGE_CAP_LABEL`, …).
- **Wasm check:** the wasm build of core uses `--no-default-features`, so it drops fontdb, memmap and tempfile.
- **Gate:** G-check, G-wasm, G-golden empty against L4. After this lane, `rg 'rmcp|tokio|axum' crates/mcp-core` must
  be empty.
- **Status:** done. Core now holds `rendering.rs` (`Rendering`, lazy reads via `std::fs::File`), `screenshot.rs`
  (`Output`-returning; `Root`, `Shot`, `render_node`, `layer_shot`, `approximations`), `assets.rs` + `assets/originals.rs`
  (`Blob` blocks for non-image originals), `design/*`, `figjam.rs` (picture loop; closes the L4 deviation) and `fs.rs`
  (`persist_file`, `fs` feature only). Features: `default = ["fs", "system-fonts"]`; the workspace dependency is
  `default-features = false` and `figstudio-mcp` turns both on. Without `fs`, `design_context` with `Assets::Dir` returns
  the error "asset files are not available in this build" (`Inline` works). The wasm32 build (`--no-default-features`)
  passes. `rg 'rmcp|tokio|axum' crates/mcp-core` is empty. Adapter keeps the `JsonSchema` input structs, routers, a
  schema-bearing `AssetFormat` that converts to core's, and old-signature wrappers. New test: `crates/mcp-core/tests/rendering.rs`.

## L6: region in MCP (M, Sonnet)

- **`get_screenshot` accepts a region link** in `nodeId`, which renders that part of the page. It reuses the L3
  viewport-crop function, now moved into core `screenshot.rs` so the app and MCP share it, with the same caps.
- **New tool `get_nodes_in_region`** (`fileKey?`, `region` link or `page` + `rect`). It returns structured JSON
  `{page, rect, nodes: [{nodeId, name, kind}], total, truncated}`, using `region::nodes_in`.
- **Update the descriptions and the server instructions** (`lib.rs:93`) to name `figstudio://` links. This is a
  deliberate golden change: regenerate the base and note it in the commit.
- **Tests:**
  - A region screenshot of a fixture page has the expected pixel size.
  - A bad rect is a tool error, not a protocol error.
  - Node list order and limit.
- **Status:** done. Core: `region_shot.rs` (`plan`, `plan_within`, `viewport`, `viewport_within`, `region_shot`; the app's
  `region_png` now calls these, so the link and the Copy-region PNG share one crop), `region_tool.rs`
  (`nodes_in_region`, `resolve`), `link::validate_rect`; `screenshot` sends a `figstudio://` region link to `region_shot`.
  Adapter: `tools/region.rs` (schema + `spawn_blocking`). Caps: region image ≤ 4096 px per side and 16 MP; without `scale`
  the longest side is 2048 px (as for nodes); `get_nodes_in_region` `limit` 1–500 (default 50), walk budget 200 000 nodes.
  Tests: `crates/mcp-core/tests/region_shot.rs`, `apps/figstudio-mcp/tests/region.rs`.

- **Gate:** G-check, G-wasm.

## L7: live mode in `figstudio-mcp`, with no app yet (L, Opus or Sonnet-high)

- **New `apps/figstudio-mcp/src/live.rs`:**
  - `pub struct Snapshot { pub tabs: Vec<TabInfo>, pub active: Option<u64> }`
  - `pub struct TabInfo { pub id: u64, pub path: PathBuf, pub doc: Arc<Document>, pub page: Guid,
    pub selection: Option<Guid>, pub viewport: Option<[f64; 4]> }`
  - `pub enum Command { Reveal { tab: u64, node: Guid, reply: tokio::sync::oneshot::Sender<Result<(), String>> } }`
  - `pub struct LiveFiles { snapshot: Arc<RwLock<Arc<Snapshot>>>, commands: std::sync::mpsc::Sender<Command>,
    wake: Arc<dyn Fn() + Send + Sync>, rendering: Mutex<HashMap<u64 /*doc.instance*/, Rendering>> }`
  - Each document gets its own `Images::new(256 MB)`, read from `TabInfo.path`. This is deliberately **not** the
    app's cache: shared in-flight image requests could hang when the app closes a document (`rendering.rs:21`).
    Entries are dropped when the instance leaves the snapshot.
- **`FileRegistry` (`files.rs`) becomes one variant of `enum Files { Registry(FileRegistry), Live(Arc<LiveFiles>) }`.**
  - It keeps the same methods (`open` / `key` / `rendering` / `list` / `keys`).
  - Live file keys: the stem, or the full path when stems collide. Reuse the rule from `files.rs:83` by extracting it
    into a function.
  - A missing `fileKey` means the active tab.
- **`FigstudioServer::live(files, …)`** adds `live_router()`:
  - `list_open_tabs` → `{tabs: [{tabId, fileKey, path, title, page: {id, name}, selection, active}]}`.
  - `get_selection` → `{fileKey, tabId, page, node: {nodeId, name, kind, bounds: {x, y, width, height}}}`, or
    `node: null`.
  - `reveal_node(nodeId | link, fileKey?)` sends `Command::Reveal`, calls `wake()`, and waits 5 s on the oneshot. On
    timeout the tool error reads "the app did not respond".
  - An instance path reveals its outer instance and says so.
- **Default `nodeId` from the selection** for `get_design_context`, `get_screenshot`, `get_variable_defs`,
  `download_assets` (when `nodeIds` is empty) and `get_context_for_code_connect`. Their inputs become
  `Option<String>`.
  - Standalone: a missing `nodeId` becomes the tool error "nodeId is required". This is a documented schema change.
  - `get_metadata` keeps "no nodeId = page list".
- **Standalone port fallback** (`main.rs:13-14, 63`):
  - Add `bind_first(candidates)`.
  - A bare `--http` tries 3845, then 3846–3855, and prints the chosen address to stderr.
  - An explicit `--http=ADDR` never falls back.
- **Tests** (`tests/live.rs`, duplex client, fake snapshot from fixtures):
  - list, selection, the defaults, reveal over a fake app loop that answers the oneshot, and the timeout.
  - **Snapshot replaced with an empty one while a `get_design_context` runs**: the call completes and the server
    stays up.
  - `bind_first` with an occupied ephemeral port.
- **Security:** `tests/http.rs` gains these assertions:
  - no `access-control-allow-origin` header on the response;
  - `Host: example.invalid` gives 403 (already there, `http.rs:129`);
  - `Origin: https://evil.example` gives 403 (`http.rs:150`).
  - The app path reuses `serve_http` (`lib.rs:99`), so it gets the same checks.
- **Gate:** G-check, G-golden (standalone) changes only where expected (tool list, nodeId schema); the commit body
  lists them.
- **Status:** done. `src/live.rs`: `Snapshot`, `TabInfo`, `Command::Reveal`, `LiveFiles::new(snapshot, commands, wake)`,
  `REVEAL_TIMEOUT` (5 s); `src/files.rs`: `enum Files { Registry, Live }` with `open`/`key`/`path`/`rendering`/`list`/`keys`
  (`rendering` now also takes the opened `&Document`, which picks the live per-instance cache) and the shared
  `find`/`stem` key rule. `FigstudioServer::live(Arc<LiveFiles>, no_system_fonts)` adds `src/tools/live.rs`
  (`list_open_tabs`, `get_selection`, `reveal_node`) and appends a live paragraph to the instructions; `whoami` says
  `mode: "live"`. `FigstudioServer::target` resolves file + node once per call (live: the picked tab's key, so a tab switch
  mid-call cannot mix tabs). `bind_first`, `default_http_addrs`, `HTTP_PORTS` are public for L8. Extra messages: live with no
  tabs "no file is open in the app", nothing selected "nodeId is required: nothing is selected in this tab", live tools
  standalone are not registered. Tests: `tests/live.rs` (9), `tests/stdio.rs` (+2 port tests, instructions text),
  `tests/http.rs` (no CORS header on any reply incl. preflight).

## L8: the app hosts the server (L, Opus or Sonnet-high)

- **`apps/figstudio/Cargo.toml`:**
  - `[features] default = ["mcp"]; mcp = ["dep:figstudio-mcp", "dep:tokio", "dep:tokio-util"]`
  - The optional deps live only in the `cfg(not(target_arch = "wasm32"))` table, so wasm never compiles them and
    `web-build.sh` needs no change.
  - Build time and binary size grow (tokio, axum, rmcp; all already in the lockfile).
  - `check.sh` adds `cargo clippy --release -p figstudio --no-default-features --all-targets -- -D warnings`.
- **New `apps/figstudio/src/mcp_host.rs`** (`#[cfg(all(feature = "mcp", not(target_arch = "wasm32")))]`):
  - `McpHost { state: Off | Running { addr, cancel: CancellationToken, snapshot, commands_rx, last_key } }`
  - `start(ctx)`:
    - Spawns `std::thread` "mcp", which builds a `new_multi_thread` runtime with 2 workers.
    - Binds with `bind_first(127.0.0.1:3845, then 3846–3855)`, always loopback, no address setting.
    - Runs `serve_http(listener, FigstudioServer::live(...), cancel)`.
    - A bind or runtime error comes back on a channel and shows as a toast; the toggle returns to off.
    - Ends with `runtime.shutdown_background()` so stuck stdin or SSE streams cannot block it.
  - `stop()` cancels; `eframe::App::on_exit` calls `stop()`.
  - `sync(&tabs, active)`, once per frame while running:
    - It builds a cheap key: tab ids, `Arc::as_ptr(doc)`, page, selection, and a viewport rounded to 1 unit from
      `tab.canvas_rect` + `view.to_page`.
    - It publishes a new `Arc<Snapshot>` only when the key changes.
  - `drain(&mut tabs, &mut active)`, per frame:
    - `Reveal` sets `active`, `tab.select(Some(id))`, and `tab.fit_node = Some(id)`.
    - The `fit_node` field is new on `Tab` (`tab.rs:39`). It is handled in `canvas::show` next to `fit_pending`, with
      the `fit_rect` code from `canvas.rs:120-131`.
    - Replies `Ok`, or `Err("tab closed")`.
  - `wake` = `ctx.request_repaint()`.
- **UI:**
  - A View-menu checkbox **"Agent access (MCP server)"**, off.
  - While running, the status bar (`main.rs:737`) shows `MCP 127.0.0.1:3845/mcp`, with hover text "Loopback only.
    Point your MCP client here."
  - `main.rs` gains ≤ 15 lines: the field, 2 calls in `ui()`, the menu item and the status label.
- **Threading:** the UI thread never blocks on MCP.
  - Snapshot writes take a lock held only for an `Arc` swap.
  - Tool work runs on tokio blocking threads, on `Arc<Document>` clones. Closing a tab only drops the app's `Arc`;
    calls already running finish on their own clone.
- **Tests:**
  - Unit: the snapshot key changes on selection, page and tab close, and stays the same on hover.
  - `drain` with a stale tab id answers `Err`.
  - `start` with a busy port falls back.
  - Desktop only: an integration test starts `McpHost` on an ephemeral port with a headless `egui::Context` and calls
    `list_open_tabs` over `reqwest`.
- **Manual "done when":**
  - Run `claude mcp add --transport http figstudio http://127.0.0.1:3845/mcp`.
  - Then list tabs, `get_selection`, `get_design_context` without `nodeId`, and `reveal_node` (the app moves).
  - Close the tab during a big call: no crash.
  - With the toggle off: `lsof -i :3845` is empty and the app behaves as before.
- **Gate:** G-check (both feature sets), G-web (the wasm app still builds), and the manual list above.
- **Status:** done. `apps/figstudio/src/mcp_host.rs` (`McpHost { on, addrs, running }`; `frame(&mut tabs, &mut active)` at the
  start of a frame polls the server thread's events and drains `Command::Reveal`; `sync(ctx, &tabs, active)` at the end
  starts/stops as the toggle says and swaps the snapshot only when `Key` changes; `stop()` cancels, never joins).
  `mcp_off.rs` is the same API with no thread (feature off, wasm). The server thread builds a 2-worker runtime, binds
  `bind_first(default_http_addrs())`, serves `FigstudioServer::live(..).with_assets(Assets::Inline)` (the app's working
  directory is arbitrary, so no asset files are written) and ends with `shutdown_background`. A bind/runtime error is a
  toast and turns the toggle off. Reveal: `Tab.fit_node`, fitted in `canvas::show` after `fit_pending` with the
  double-click code (now `canvas::fit_node`). The status bar's first item while running is `MCP 127.0.0.1:3845/mcp`.
  Tests: `src/mcp_host_tests.rs` (5: key, reveal/stale tab, busy-port fallback, no free port, HTTP client
  `list_open_tabs` + `reveal_node` + `get_selection` + port closed after toggle off), 11 mutations caught. Manual:
  curl session against the release app (temporary env hook, not committed): list, selection, design context without
  nodeId, reveal (selection moved), tab closed during 4×30 concurrent `get_screenshot` calls (no hang, server and app
  up); toggle off at launch: no listener on 3845. Not verified: `claude mcp add`, visual fit after reveal.
  **Menu toggle now covered** by `ui_mcp_tests.rs` (egui_kittest): View menu item unchecked at launch, checking it shows
  `MCP 127.0.0.1:<port>/mcp` in the status bar, the port answers and `list_open_tabs` sees the tab, unchecking removes the label and
  closes the port. Binary: 15.9 MB → 23.2 MB release (feature on); off = unchanged.
  Seen on the standalone server too (not L8): several concurrent calls on **one** session over curl appeared to stall.
  The L8 review could not reproduce it (2 and 4 concurrent calls on one session all answered); rmcp answers with an
  SSE stream, so a curl without `--max-time` can keep reading after the call finished. Open as a separate note, not
  part of this lane.

## L9: docs (S, Haiku)

- **`apps/figstudio-mcp/README.md`:**
  - the app mode;
  - the port fallback;
  - the `figstudio://` link grammar;
  - the selection defaults;
  - security: loopback, Host/Origin checks, no CORS.
- **`docs/IMPLEMENTATION.md`:** the `mcp-core` crate and its data flow.
- **`docs/TESTING.md`:** golden and live tests.
- **`docs/MCP-PLAN.md` and `docs/PLAN.md`:** tick boxes, with a progress line per lane. Each lane already ticks its
  own box; L9 fills the gaps.
- **Gate:** G-check, and `doc_drift` has no new drift.
- **Status:** done (commit `docs(mcp,app): document MCP inside the app`).
  - **Files:** `apps/figstudio-mcp/README.md` (app mode, `figstudio://` and figma.com links with the file
    rule, selection defaults, the three live tools, the security notes), `docs/IMPLEMENTATION.md` (the
    `crates/mcp-core` map entry, the copy menus, marquee and region copy, the app ↔ `mcp_host` ↔ server
    data flow), `docs/TESTING.md` (live tests, the golden harness, the `mcp-core` no-default-features run,
    GPU-free notes), `docs/PLAN.md` (P7 boxes and one progress line), `docs/MCP-PLAN.md` (a pointer to this
    plan for the app lanes).
  - **Checked against the code:** every tool, flag, port, link form, file rule and reply text named in the
    docs was grepped. Two wrong statements were found and corrected before the commit: that a region link is
    only for region tools (`get_screenshot` takes one too), and that the flags apply to the app (the app
    serves live tabs, with inline assets).
  - **Not verified:** a live client (`claude mcp add` and a tool call) against the app; the README was
    not rendered on GitHub.

## Risks

- **Golden flakiness:** float formatting and HashMap order. L0 runs on base twice to rule them out.
- **Wasm deps:** `tempfile` and `fontdb` are behind features; `image` and `quick-xml` build for wasm already, as the
  app does.
- **Clipboard image limits:**
  - arboard images on Wayland need data-control support (we can't detect a failure).
  - Safari's user-gesture expiry on async web image writes.
  - Large PNGs: capped at 16 MP.
- **The embedded server competes for CPU with tile rendering.** At most 2 tokio workers; renders already go through
  `spawn_blocking`.
- **Memory:** the per-document MCP image cache can reach 256 MB on top of the app's own cache. Sharing the app's cache
  is a later optimisation, once in-flight sharing is safe.
- **Schema change:** `nodeId` becomes optional (L7); old clients still work.
- **Port fallback:** clients configured for 3845 reach whichever process started first. The status bar and stderr show
  the actual address.
- **Reveal while minimised:** the app may not run frames, so `reveal_node` times out with a clear error.

## Model per lane

| Lane | Size | Model | Why |
|---|---|---|---|
| L0 | S | Haiku (with this spec) or Sonnet | test harness, deterministic node choice |
| L1 | M | Sonnet | URL encoding edge cases, error-type compatibility |
| L2 | M | Sonnet (Haiku workable) | egui menus, formatters |
| L3 | M/L | Sonnet | coordinate math, off-thread render, clipboard |
| L4 | L | Sonnet (Haiku only one file per pass) | large mechanical move, byte identity |
| L5 | L | Sonnet | features, wasm gate, lazy image reads |
| L6 | M | Sonnet | viewport crop, new tool |
| L7 | L | Opus / Sonnet-high | concurrency, file resolution, security tests |
| L8 | L | Opus / Sonnet-high | threads in eframe, shutdown, snapshot design |
| L9 | S | Haiku | docs only |
