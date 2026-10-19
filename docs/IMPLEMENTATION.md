# Implementation guide

How figstudio is built, how data flows through it, how to add things, and what is left to
do. Read `AGENTS.md` first. Testing is in `docs/TESTING.md`; status is in `docs/PLAN.md`.

## Native and drawn menus

`apps/figstudio/src/menu.rs` defines menu state and `Action`; `App::apply` handles
commands from clicks and shortcuts. `menu_egui.rs` renders Linux/web menus and the
headless/native-attach fallback behind the sidebar's "⋯" button (File, View and Help as
submenus); the command bar (`command.rs`) lists the same enabled commands. `menu_windows.rs` draws the
Settings window, where Agent access and About now live (View → Agent access… and Help → About
open those tabs).
`menu_native.rs` attaches muda on macOS/Windows in the eframe creation closure;
`menu_dispatch.rs` drains queued ids with repaint and synchronizes changed flags.
macOS owns Open, Close file, selection-export, command bar (⌘L), find (⌘F) and sidebar (⌘\)
accelerators; egui retains Shift+1
headless/native-attach fallback. `menu_windows.rs` hosts Agent access, About and Open Figma
link (`link_open.rs`).
`menu_native.rs` attaches muda on macOS/Windows in the eframe creation closure;
`menu_dispatch.rs` drains queued ids with repaint and synchronizes changed flags.
macOS owns Open, Open Figma link (Cmd+L), Close tab and selection-export accelerators; egui retains Shift+1
and Shift+0 (only outside text editing). Windows keeps egui shortcuts (winit does
not translate native accelerators). Context menus remain egui. Native attachment failure logs a
warning and keeps the drawn bar.

## 1. Map of the code

```
crates/fig/src/          no UI, no rendering; builds for wasm (browser app)
  lib.rs                 public exports
  doc.rs                 container (zip / raw), chunks, decompress, Document, Node, reindex
  read.rs                Kiwi bytes -> Val, driven by the schema stored in the file
  val.rs                 Val, Obj (field list), Guid, intern()
  write.rs               Val -> Kiwi bytes, canvas chunks, zip; Document::to_bytes / save
  tokens.rs, tokens/     read-only variable / style index (types.rs: Collection, Mode, Variable,
                         Style, Value); shared by the renderer and MCP tools
  resolve.rs, resolve/   public instance resolver: Overrides scope stack and merge (shared
                         with the renderer), component properties, swaps, walk / path lookup
crates/render/src/       Document -> pixels with vello_cpu; builds for wasm without fonts
  lib.rs                 Renderer, Stats, Viewport, the draw walk, culling, instances, masks,
                         bounds / hit test helpers; render_parallel (feature parallel)
  record.rs              Canvas trait (where drawing goes); Recorder for parallel strips
  geom.rs                transforms, sizes, path blobs, base shapes, clipping rules
  paint.rs               solid / gradient / image paints, image cache
  text.rs                stored glyph outlines; installed-font layout (feature system-fonts)
  focus.rs               render_path / path_bounds: draw one node inside an instance on its own,
                         laid out as the instance draws it (used by MCP instance-path shots)
crates/mcp-core/src/     the MCP tool logic, with no SDK types (shared by the standalone server
                         and the app). Features: `fs` (writes asset files, lazy reads via
                         std::fs) and `system-fonts`; both are on by default. The wasm32 build and
                         the browser use `--no-default-features`, where `fs` is off and
                         default design-context assets are inline PNGs.
  link.rs                figstudio:// and figma.com links: parse_link, format_node_link,
                         format_region_link, validate_rect
  node_id.rs             node IDs and instance paths (moved from the server)
  error.rs               CoreError (InvalidNodeId, FileKey, Open, InvalidLink)
  output.rs              Output and Block: the text / image split every tool reply uses
  metadata.rs, metadata/ get_metadata and get_figjam text (figjam.rs, node.rs, pages.rs)
  rendering.rs           Rendering: one document's renderer; path or custom EntryReader for
                         lazy images, sharing the decode cache and lazy-render lock
  wasm_rpc/              SDK-free browser MCP dispatcher over McpTabEnv snapshots, tool
                         schema data (native drift-tested), read-only tools and live hooks
  prompts.rs             shared design-system rules text, reused by native and browser MCP
  screenshot.rs          render_node, layer_shot and the caps shared with the app's PNG export
  region.rs              nodes_in: the nodes of a page that touch a rectangle, top-level first
  region_shot.rs         plan / viewport / region_shot: the region crop shared by the app and MCP
  region_tool.rs         get_nodes_in_region logic
  design/, variables/, design_system.rs, figjam.rs, assets.rs, codeconnect_context.rs
                         the rest of the tool logic (design/ = get_design_context)

apps/figstudio-cli/src/
  main.rs                argument parsing and commands
  doctor.rs              privacy-safe structure report
  synth.rs               stress-file generator
apps/figstudio-mcp/src/  offline MCP server (separate library and binary)
  main.rs                clap options, stdio / Streamable HTTP, Ctrl-C / SIGTERM shutdown
  lib.rs                 rmcp handler, tools/prompts capabilities, /mcp HTTP service
  prompts.rs             create_design_system_rules prompt and bounded local inventory
  files.rs               lazy shared local document registry and fileKey resolution
  node_id.rs             node IDs, Figma URLs, instance paths (figstudio_fig::resolve::InstancePath)
  error.rs               canonical error type; tools/mod.rs maps tool failures
  tools/mod.rs           whoami, list_files, tool_error; declares every tool module
  tools/metadata.rs      get_metadata: typed inputs, quick-xml, bounded iterative tree walk;
                         instances expand through fig::resolve::walk_instance (metadata/node.rs
                         writes one element, metadata/resolved.rs writes resolved instance
                         content into the body)
  tools/figjam.rs        get_figjam: metadata::run with FigJam Extras, optional small PNGs
                         (metadata/figjam.rs: per-kind attributes, text, undrawn-kind counts)
  tools/screenshot.rs    get_screenshot (render_node/Shot shared with assets and figjam); an instance path uses Renderer::render_path and
                         path_bounds (render/src/focus.rs: only that node, laid out as drawn)
  tools/assets.rs        download_assets: PNG/JPG renders (screenshot::render_node) or original
                         image bytes; assets/originals.rs finds image-fill hashes in a node
  tools/codeconnect.rs    component context + atomic local source mappings (named router)
    codeconnect/context.rs component sets, properties, instance resolution
    codeconnect/sidecar.rs  serde_json + tempfile persist, filters, symlink rejection
  tools/design_system.rs  capped local component/token search and summaries
  tools/variables.rs     get_variable_defs; walks resolved instance content
    variables/format.rs  stored style payloads -> text (colours, fonts, numbers)
  tools/design/          get_design_context: mod.rs (tool, output sections, Notes), walk.rs
                         (layers incl. resolved instances), jsx.rs (elements, picture assets),
                         layout.rs (position, size, stack → flex), paint.rs (colours,
                         gradients, var()), style.rs (border, radius, effects, blend),
                         text.rs (typography, runs, html-escape), assets.rs (stable names,
                         writes kept inside --assets-dir); PNGs come from
                         screenshot::layer_shot, the get_screenshot render path
                         New tools belong in tools/<name>.rs with a typed input struct.
                         Use #[tool_router(router = <name>_router, vis = "pub(crate)")]
                         on an impl FigstudioServer, declare the module here, and combine
                         Self::tool_router() + Self::<name>_router() in lib.rs new().
apps/figstudio-mcp/tests/  protocol tests (stdio.rs, http.rs, prompts.rs, files.rs) and one
                         file per tool (see docs/TESTING.md "MCP server"); README.md is the
                         user doc
apps/figstudio/src/      egui 0.36 app: desktop, and browser (wasm32, threads)
  main.rs                App: shortcuts, panel layout, message polling, export
  chrome.rs              Arc-style chrome: window gradient, sidebar (window controls, command
                         field, pages, layers, file switcher), title strip, peeking sidebar,
                         canvas card with tool / status pills and the warning chip, toasts,
                         empty state, drop overlay
  command.rs             the ⌘L command bar: search over commands, files, pages and layers
  anim.rs                springs and colour fades in egui memory; Reduce motion
  icons.rs               line icons painted on a 16-unit grid; the icon button
  vibrancy.rs            macOS only: the blur view behind the transparent window
  spaces.rs              a space's colour / emoji or icon / name, saved per file on this
                         computer (config folder JSON, localStorage in the browser)
  space_editor.rs        the space popovers: icon picker, rename, theme colour (pad + presets
                         + appearance), opened above the space's dot
  swipe.rs               two-finger sideways swipes over the sidebar switch spaces
  find.rs                ⌘F find bar: text on the page, inside instances too, and frame
                         names; highlights
  settings.rs            Settings (⌘,), Arc's layout: General, Canvas, Shortcuts, Agents
                         (agent access, the MCP server), About
  keymap.rs              shortcuts the user can change: Binding (label, group, default,
                         action) and the Keymap of changes, saved in Prefs
  canvas.rs              canvas input, render requests, frame re-projection, outlines
  tab.rs                 per-document state (Tab, with its tint) and the page<->screen mapping
  panels.rs              Pages list, Layers tree, Properties card
  worker.rs              background threads: file loading, rendering (OS threads or Web Workers)
  web.rs                 browser only: start(), file picker, dropped files, downloads
  theme.rs               light / dark palettes, per-file tints, visuals, UI font (system face
                         + semibold), CJK font fallback (desktop) / detection (web)
web/                     index.html (boot: init, thread pool, start), _headers (COOP/COEP)
scripts/                 check.sh, thumb-sheet.py, gui-smoke.sh, web-build.sh, web-serve.py
tests/fixtures/          7 sample .fig files (MIT, fig2sketch)
```

Dependencies are declared once in the root `Cargo.toml` (`[workspace.dependencies]`).
Do not add a dependency without a reason you can state in the commit message.

## 2. Data flow

### Tokens (`crates/fig/src/tokens.rs`)

`Tokens::new(&Document)` indexes variable collections, per-mode values and stored style
payloads by field name. GUID/asset lookups and bounded alias resolution are shared with
rendering; the renderer deliberately keeps the first stored mode. No fonts or new dependencies.

### Reading a file (`crates/fig`)

```
bytes ──► zip?  yes: canvas.fig + thumbnail.png + meta.json + images/<sha1> (+ other entries kept)
          no:   the bytes are canvas.fig ("fig-kiwi" prelude)
canvas.fig = 8-byte prelude + u32 version + [u32 length + chunk]*
chunk 0 = Kiwi schema   (deflate / zlib / zstd, codec remembered for writing)
chunk 1 = Kiwi message  (same)
chunks 2.. kept byte for byte
message ──► read.rs Reader, one node at a time ──► Vec<Node> (later duplicates replace earlier)
        ──► reindex(): children per parent sorted by fractional `position` (byte order),
            DOCUMENT root, CANVAS pages (internalOnly pages listed separately)
```

`Document` keeps everything needed to write the file back (`Source`: schema bytes, codecs,
extra chunks, other top-level message fields, other archive entries). Never drop a field
while decoding.

### Values

- `Val` is a schema-free copy of a Kiwi value: `Bool Int Float Str Enum Bytes Arr Obj`.
- `Obj` is a small `Vec<(&'static str, Val)>` in decode order. Field and enum names are
  interned (`intern()`), so they are `'static` and cheap. Lookups are linear; objects are small.
- Field names are **Figma's schema names** (camelCase). The ones the renderer uses most:

| Field | Meaning |
|---|---|
| `type` | node type: `CANVAS FRAME GROUP SECTION SYMBOL INSTANCE TEXT VECTOR RECTANGLE ROUNDED_RECTANGLE ELLIPSE BOOLEAN_OPERATION …` |
| `guid`, `parentIndex.{guid,position}` | id, parent, sibling order (fractional index string) |
| `transform.{m00 m01 m02 m10 m11 m12}`, `size.{x,y}` | local 2×3 transform, frame size |
| `visible`, `opacity`, `mask`, `blendMode` | layer flags |
| `fillPaints`, `strokePaints`, `strokeWeight` | paints: `type` SOLID / GRADIENT_* / IMAGE, `color`, `opacity`, `visible`, `transform`, `stops`, `image.hash` |
| `fillGeometry`, `strokeGeometry` | `[{commandsBlob, windingRule, styleID}]`; `commandsBlob` indexes `Document::blobs` |
| `vectorData.styleOverrideTable` | per-path paints for vectors, matched by `styleID` (0 = node paints) |
| `effects` | `type` DROP_SHADOW / INNER_SHADOW / FOREGROUND_BLUR / BACKGROUND_BLUR / …, `radius`, `offset`, `blurOpType` |
| `textData.characters`, `derivedTextData.glyphs`, `fontName`, `fontSize` | text content, stored glyph outlines |
| `symbolData.{symbolID,symbolOverrides}`, `derivedSymbolData`, `overriddenSymbolID` | instances |
| `frameMaskDisabled`, `stackMode`, `cornerRadius` | clipping, auto layout, corners |

To see what a real node contains, write a throwaway example (and delete it afterwards):

```rust
// crates/fig/examples/peek.rs   run: cargo run -p figstudio-fig --example peek -- file.fig 1:42
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let doc = figstudio_fig::Document::open(&a[1]).unwrap();
    let id: figstudio_fig::Guid = a[2].parse().unwrap();
    println!("{:#?}", doc.node(id).unwrap().data);
}
```

`figstudio-cli tree <file> --depth 4` lists ids, types and names to pick an id from.

### Writing a file (`crates/fig/src/write.rs`)

- `encode_message()` writes the top-level fields other than `nodeChanges` and `blobs` in
  schema order, then `nodeChanges`, then `blobs`. This is the order Figma uses; with it,
  every fixture encodes to the **original bytes**.
- Nested messages are written in schema field order; structs write every field.
- `Blob` is a Kiwi *struct* in Figma's schema (length + bytes, no tag or terminator).
- Unknown field names and wrong value types are `FigError`s that name the node, never panics.
- `to_bytes()` rebuilds the zip (entries stored, not compressed, like Figma); `save()` writes
  `<path>.saving` and renames it over the target.
- To edit: clone `doc.nodes`, change `node.data` (`Val::set`, `Obj::insert`), rebuild the node
  with `Node::from_data` if `guid` / parent / `type` / `name` changed, then `doc.set_nodes(nodes)`.
- **Unverified:** whether Figma opens a file we wrote. Only a person with Figma can check it.

### Rendering (`crates/render`)

`Renderer::render(doc, root, &Viewport { width, height, transform, background })` walks the
tree from `root` (a page or any node). For each node, `Ctx::draw`:

1. `patched()` applies instance overrides for the current override path (see below).
2. Hidden nodes and mask nodes return (masks are handled by the parent).
3. `world = parent * transform`. `off_screen()` may cull the node (see Culling).
4. Effects: one filter layer per node at most (drop shadow or layer blur). Everything else,
   and progressive blur, is counted in `stats.skipped_effects`.
5. Opacity layer if `opacity < 1`.
6. Fills: `fillGeometry` blobs; else text outlines for TEXT; else a base shape. Paths with a
   `styleID` use paints from `vectorData.styleOverrideTable`.
7. Strokes: `strokeGeometry` blobs; else per-side borders (`borderStrokeWeightsIndependent`)
   as an aligned ring; else stroke the base shape (`stroke.rs`: `strokeJoin`, `strokeCap`,
   `dashPattern`, miter limit 4). An instance override that changes a stroke field drops the
   stored `strokeGeometry` unless a derived patch supplies new geometry. A frame that clips
   its content draws its strokes after its children instead (step 8), so cells that fill a
   table cannot hide its border; a non-clipping frame keeps them below its children.
8. Children: clip layer if the frame clips; `draw_instance` for INSTANCE, else
   `draw_children`, which honors `stackReverseZIndex` and turns `mask` children into clip
   layers for the siblings above them.
9. Pop the layers.

Paints are resolved in `paint::resolve()`. SOLID, linear / radial / angular gradients and
IMAGE (stretch / fill / fit / tile) are drawn; diamond gradients are approximated as radial
(`approximated_paints`); a missing image draws a grey placeholder (`missing_images`).

**Instances.** The override merge lives in `figstudio_fig::resolve` and is shared by the
renderer and the MCP tools. `resolve_instance(doc, instance)` lists every node inside an
instance (Figma ids `I<instance>;<key>…`, merged data, level; capped at
`MAX_RESOLVED_NODES`), `walk_instance` adds depth and node caps, and
`resolve_path(doc, &InstancePath)` looks one node up. A walker that descends by itself (the
renderer) keeps an `Overrides` stack: `enter`/`leave` per instance, `matches` + `merge` (or
`resolve`) per node. Uniform instance scale is shared too: `resolve::scale_inherited` with
`Overrides::outer_scale` (owner-aware) scales translation, size, stroke weights and stored
glyphs, and `resolve` applies it, so tools report the geometry the renderer draws. The
renderer adds only its render-side steps on top of `merge`: constraint corrections and stale
glyph and stroke geometry detection. Any tool that reads node data inside instances must use
`fig::resolve`, never `symbolOverrides` directly. `draw_instance` pushes every `symbolOverrides` entry (and `derivedSymbolData`)
as a `Patch` keyed by the full override path (instance path + `guidPath.guids`), then draws
the component's children with that path. A node's override key is `Node::override_key()`.
Override paths include only instance ancestors and the target key, not intervening ordinary
frames/groups. Component boolean/text/instance-swap properties are resolved in the owning
instance scope; parent property definitions supply defaults. Paint-variable aliases resolve
by GUID or exact asset key/version; unresolved aliases use the stored colour and increment
`approximated_paints`. First stored variable mode is used (inherited mode selection is open).
Derived transforms are authoritative; when omitted, uniform instance scale updates translation
and CENTER/MAX/SCALE constraints adjust it for resized containers without double-scaling stored
geometry. Constraints skip in-flow children of an auto-layout parent (only `stackPositioning`
ABSOLUTE ones follow them): auto layout keeps those at the padding. Property/variable index preparation charges the shared render-work budget.
Patches from outer instances are applied last so they win; derived data is applied after
the overrides of the same instance. A text override without fresh derived glyphs marks the
stored glyphs stale, and text falls back to installed fonts. `MAX_RENDER_WORK` (2,000,000)
bounds fit preparation plus all drawing work; 12,418 units were measured for the large real page,
leaving over 160x headroom. Ordinary depth and `MAX_INSTANCE_DEPTH` (24) independently prevent
stack exhaustion and symbol cycles.

**Culling.** Only nodes whose drawing stays inside their frame are tested: leaves, clipping
frames and instances, boolean operations. Their frame is grown by `overhang()` (strokes ×4
for miters, effect radius ×3 + spread + offset, generous text overflow) and skipped when it
misses the output, or when a leaf is under a quarter pixel. **If you make anything draw
outside a node's frame, extend `overhang()`**, or culling will cut it off at the window edge.

**Stats.** Every approximation or skip increments a `Stats` field. The CLI prints them and
the app shows them in the warning chip on the canvas. Add a field when you add a new kind of
gap.

**Parallel strips (`render_parallel`, feature `parallel`).** The walk draws through the
`record::Canvas` trait. `render` passes a `RenderContext`; `render_parallel` passes a
`Recorder`, which stores the commands with the output area each fill and layer can change
(filter layers grow it by the effect's reach, `effect_reach()`). Then each horizontal strip
gets its own `RenderContext` on a Rayon thread and replays only the commands that reach its
rows. So the document is walked once (stats are exact) and only rasterizing is split. Vello
CPU's own `multithreading` feature cannot be used in the browser: it builds its own Rayon pool
with `std::thread::spawn`, which wasm does not have. **If you add a filter, give
`push_filter_layer` its reach**, or strips will cut it off at their edges.

### The desktop app (`apps/figstudio`)

- The UI thread never decodes or renders. `worker.rs` loads files on short-lived threads and
  renders tiles on one long-lived thread; a newer tile batch for a tab replaces the older one.
- The canvas is drawn from 512x512-pixel tiles (`tiles.rs` grid, levels and LRU cache;
  `tiles_view.rs` drawing and requests). Level `n` draws `2^(n/2)` pixels per page unit; the
  canvas shows the first level at least as sharp as zoom x pixels per point. Cached tiles of
  coarser and finer levels are drawn under the sharp ones, so a missing tile shows a
  stretched fallback instead of empty canvas. Missing tiles of the view plus a one-tile ring
  are requested nearest the centre first. Tiles are keyed by document instance, page, level
  and position; tiles drawn while images loaded are drawn again when more images arrive.
  Each tile is rendered with a 2-pixel gutter of its neighbours (only the inner 512 px are
  drawn), so linear filtering leaves no seam. The resident cache holds 256 MiB of tile
  textures, gutters included (128 MiB in the browser); tiles of the planned range are evicted
  last, fallback tiles on screen before them (those already covered by planned tiles first), and
  a view that would need more than half the cache uses a coarser level. Tiles on their way
  from the worker are outside that budget (at most one batch per tab). The worker skips
  tiles it sent that the UI has not acknowledged yet (`tile_queue.rs`); closing a tab drops
  its queued tiles, and closing a document marks it closed in the image cache
  (`Images::close_document`), which then refuses any image, failure or request for it under
  its lock, so a read or decode finishing late cannot store it back.
  The warning chip shows, per field, the largest count of any one sharp tile on screen
  (`Stats::max_with`): a lower bound for the view, not a sum.
- `Tab` holds the document (`Arc<Document>`), page, view, selection, hover, expanded tree
  nodes and stats. `Tab::select` switches page and expands ancestors so the Layers tree can
  scroll to the layer.
- Keyboard shortcuts are in `App::shortcuts`. Shift+digit uses `shifted_digit` because the
  logical key depends on the keyboard layout (Shift+1 is `!` on US layouts).
- `FIGSTUDIO_PROFILE=1` prints per-frame UI time, render time, drawn and culled counts.
- Tiles are drawn from a per-page, per-level recording (`recordings.rs`, `Renderer::record`):
  the first tile walks the page once, later tiles replay only the commands that touch them.
- **GPU canvas** (`gpu_tiles.rs`, `gpu_registry.rs`, shared `gpu_common.rs`; plan and notes
  in `docs/GPU.md`). eframe runs on wgpu; the render worker replays the recording into a
  `vello_gpu` scene, renders it on eframe's device into a texture and the UI registers it with
  egui. Tiles `vello_gpu` cannot draw (images, angular gradients, unsupported filters, nested
  filters too big for its intermediates, too many layers), render errors, a lost device or a
  device with textures under 4096 px fall back to CPU tiles, counted in
  `Stats::gpu_fallback_tiles` ("N tiles on the CPU"). `FIGSTUDIO_RENDERER=cpu` forces the CPU;
  the status pill names the renderer ("GPU (Metal)" or "CPU"). The CLI, tests and export always
  use the CPU renderer, so their output stays reproducible.

### The window chrome (`chrome.rs`, `theme.rs`, `anim.rs`, `icons.rs`, `command.rs`)

- Layout, outside in: the window gradient (painted on the background layer, the active
  file's `Tab::tint` crossfading on a file switch), a collapsible left panel (the sidebar) or,
  when it is hidden, a top title strip; a collapsible right panel framed as the Properties
  card; the central panel draws the canvas card (`App::canvas_card`) with the tool pill,
  status pill and warning chip on top. Toasts, the drop overlay, the peeking sidebar and the
  command bar are `Order::Foreground` areas.
- Colours come from `theme::of(ui)` / `theme::palette(ctx)` (light or dark), never from
  constants, except `theme::ACCENT` for canvas outlines. Tints are picked per file by name
  (`pick_tint`), skipping ones open files have. `theme::tests` checks text contrast on
  every tint: add a tint, run them.
- Motion: `anim::spring*` for movement and pops, `anim::fade` for colour, egui's own
  `animate_bool*` for hover fades. All of it stops when `anim::reduced` (egui's
  `animation_time` is 0: View → Reduce motion, `prefers-reduced-motion` in the browser, the
  test harness). Widget-heavy lists keep one hover state per list (`panels::hover_fade`).
- **Nothing just appears or vanishes.** Overlays (command bar, Settings, space popovers, the
  find bar and its highlights, the space name pill) use `anim::presence`: they pop in, and
  after closing keep drawing, `anim::inert` (no input), until they have faded and shrunk
  away; their state must outlive the close (the command bar keeps its results, a closed
  popover moves to `space_leaving`, find keeps its highlights). While fading they stay
  interactable areas, so a click or scroll on them reaches nothing underneath, and the
  shortcut guards hold until they are gone (`CommandBar::is_up`, `Settings::is_up`,
  `App::space_popover_up`, all via `anim::on_screen`). Each space popover has its own
  presence and area, keyed by when it opened, so a new one pops in while the last fades.
  Find's fading highlights stay only on the tab and page they were found on
  (`FindBar::shown_on`), and the bar still fades with no file open. New content uses
  `anim::arrive` (a short fade whenever its key changes: another file or page on the canvas,
  another selection in the properties, hover and selection outlines, a Settings tab's page,
  children of a layer just opened); `anim::pop_on` bounces what just became current (the
  find bar's match). The canvas crossfades: `chrome::Crossfade` keeps the last picture
  shown in full (file, page, view, page colour) and paints it again under the new file or
  page while that fades in (`Tiles::paint_cached`, without asking for tiles). A picture
  still fading in is never kept, so going back mid-fade fades from what was on screen.
  The underlay's tiles are marked drawn after the new page plans its own
  (`Tiles::keep_drawn`), so the cache does not evict them first, and a closed file's tiles
  stay until no fade paints them (`App::tiles_closed`). The first file fades in over its
  own page colour. It never fades from the card, which is near black in the dark theme. New file dots pop in, and the others slide over when one closes. A new
  change of state on screen should use one of these. egui's own popups (menus, tooltips)
  fade in but close at once.
- macOS runs frameless (`with_fullsize_content_view`, no title bar): the sidebar header and
  the title strip leave room for the traffic lights and move the window
  (`ViewportCommand::StartDrag`). The window is transparent with a blur behind it
  (`vibrancy.rs`, Sidebar material, following View → Appearance); the gradient is painted at
  `theme::TRANSLUCENT_TINT` over it, opaque everywhere else.
- **Spaces.** Each open file is a space: `Tab::tint` (picked from the name) unless
  `Tab::style` (`spaces::Style`) sets a preset or custom colour, an emoji or icon badge and a
  display name. A space dot's right-click menu (and the Space menu, and the command bar)
  opens Arc's three popovers above the dot: Change space icon, Rename space, Edit theme
  colour. Changes show at once; a click outside, Escape or Enter closes and saves them, and
  so do opening another popover, closing the file and quitting (`commit_space_editor`).
  Escape while renaming goes back; a reset closes the popover first. `spaces::Store` finds
  a file by `Tab::doc_key`, else by its name in the same folder (`folder_of`: a hash of the
  folder, never the path), else by its name in a row saved with no folder (before folders
  were), so a new copy keeps its look and a same-named file elsewhere does not; a reset is
  stored too, so it sticks. Writes go to a new uniquely named temporary file (`tempfile`)
  renamed over the store, readable by its owner only on Unix (0600; no ACL on Windows), so
  two figstudios writing at once never tear it. Reads are capped while reading and skip
  damaged entries; a store that cannot be read (too big, no permission) or does not parse
  (cut short, not text, not a JSON list) is kept as it is and this session's styles stay in
  memory, so nothing unread is written over and it can be repaired. An empty file is an
  empty store. The store is
  `<config dir>/figstudio/spaces.json` on the desktop and `localStorage` in the browser; tests
  use an in-memory store and never touch it. It holds file names, so it stays local: nothing
  (doctor included) prints or uploads it. Custom colours go through `theme::space_tint`,
  which mixes in only as much colour as keeps text readable.
- Menus and right-click menus use `theme::menu_style` (macOS look: rounded, blue highlight,
  white text); open new ones with `chrome::context_menu` or `.style(theme::menu_style)`.
- Two-finger sideways swipes over the sidebar move between spaces (`swipe.rs`), Arc's pager:
  the active space's page and its neighbour's sit side by side and follow the fingers, and
  the window colour blends between theirs. Letting go past a third of the sidebar, or a
  flick (900 pt/s, at least 40 pt), turns the page; the pages carry on from where the
  fingers left them. The momentum the trackpad sends after a lift (as a new start within
  25 ms, or in the very next frame up to 0.2 s when the UI thread hitched) is ignored up to
  its own end; one momentum per lift, so a start after it is new fingers. The moves of one
  frame share its time, so `Swipe::feed_frame` counts them as one sample (their sum over the
  frame time) for the flick speed; in the frame a swipe started in, that is the time since
  the last frame (with no 0.1 s ceiling), counted in full. It is a lower bound: a flick that
  lands whole in one frame turns when it is fast over the whole hitch (100 points in 50 ms,
  110 in 120 ms), while a short one in a long hitch (60 points in 80 ms) reads as a drag and
  springs back, rather than a slow drag the stall batched turning a page. A click on the next or previous dot turns a whole page
  too, and both pages stay side by side until the spring settles; only a far jump's short
  nudge fades (`App::slide_nudge`, `chrome::sidebar_pages`). The lists scroll vertically
  only.
- **Pages moved sideways must not take room in the panel.** egui grows a panel to fit what
  its content allocates, so a page drawn with `scope_builder` at an offset widened the
  sidebar on every switch; they are `new_child` UIs. The panel id changed
  (`chrome::SIDEBAR`) so a width inflated that way is not restored.
- eframe's `persistence` keeps the window, the panel widths and egui's options (appearance,
  Reduce motion) between launches; `App::save` adds the sidebar / properties visibility, the
  Pages/Layers split and Settings' `Prefs` (`UiState`): the translucent sidebar, the renderer
  (applied at the next start; `FIGSTUDIO_RENDERER` wins), frame names and whether the scroll
  wheel zooms (`canvas::Options`).
- **Settings** (`settings.rs`, ⌘, / the app menu on macOS / File → Settings… elsewhere / the
  command bar): a foreground window with Arc's tab row. Agent access (`mcp.menu`, `mcp.status`)
  and About live in it; View → Agent access… and Help → About open those tabs
  (`menu_windows.rs` only shows it now).
- **Settings look like Arc's** (`settings.rs`). Colours come from `Look`, measured from Arc's
  dark settings window (warm greys; light mirrors them), not from the app palette. A page is
  an outlined `list` of `section`s and `setting` rows (label and note left, control right)
  beside `explain` (blue keycap tiles, a title, a paragraph). `setting` wraps its text short
  of the control, using the control's width from the last frame (a change discards the
  frame), so text never runs under a control. Agent access on the desktop is rows too:
  `McpHost::access` lends its switches; the browser keeps its relay `menu`.
- **Shortcuts are data** (`keymap.rs`). A new shortcut is a `Binding` (label, group,
  default keys, the `Action` it runs); `App::shortcuts` runs every binding from the keymap
  (most modifiers first, keys without ⌘/Ctrl/Alt only while nothing takes typing), the drawn
  menus and the command bar show `keymap.for_action`, and the macOS/Windows menus get
  accelerators from it (`NativeMenu::set_keys`, none while a shortcut is being recorded so
  the menu does not swallow the keys; cleared in the same turn recording starts, and menu
  actions are ignored while it lasts). A key a menu cannot show (`!`, F13, Insert) stays
  with `App::shortcuts` even for a menu-owned action (`menu_dispatch::menu_runs`).
  Settings → Shortcuts records the next key press, first thing in the frame
  (`App::record_shortcut`, which takes every key and typed character meanwhile): modifier
  keys alone do not end it (egui sends them as keys), ⌘X/⌘C/⌘V arrive as Cut/Copy/Paste and
  are recorded too. Backspace removes, Escape keeps, keys another binding has move (it is
  told). On a Mac Control is kept beside ⌘ (`ctrl` duplicates `command` only elsewhere).
  Only changes are saved, so new defaults reach everyone else; they load leniently (an
  unreadable entry is dropped, not the window state). Keys are given out against the
  finished map, so a swap survives: saved changes first (a chord saved twice goes to the
  first; the other change falls back to its default), then defaults (one whose chord a
  change took has no shortcut). The sidebar's and the
  empty state's key hints read the keymap. Escape, Enter in the find bar and pointer
  gestures are fixed.
- **Find on page** (`find.rs`, ⌘F, View → Find on page…): matches text layers of the page
  and text drawn inside instances (`resolve::walk_instance` with overrides, boxed with
  `Renderer::path_bounds`) and the names drawn above frames and sections (`Hit::Name`, the
  frames `labels::named` lists), sorted top to bottom. Only what is drawn: the page walk
  skips hidden layers and everything in them, and `path_bounds` gives a layer hidden inside
  an instance no box. The canvas highlights them (a name on
  its label when the label is drawn, else round its frame); Enter / ⌘G and
  Shift for the previous one; a match is brought into view only when it is off screen or too
  small to read in the target view (`Tab::view`, where an ease ends). At most 5000 matches
  are kept, the top of the page (sorted before the cut); instances are searched top first
  and the walk stops once the kept matches all sit above the next one (`find_at_most`).
  Escape closes the bar whether or not its field has focus; the next Escape clears the
  selection.
- The canvas card has round corners: the page background is a rounded rect and tiles that
  reach into a corner are drawn as a polygon cut to it (`tiles_view::rounded_clip`).
- View jumps (fit, zoom buttons and keys, the Zoom tool, double-click) ease over 0.28 s:
  `Tab::view` is the target (the MCP and the next jump see it); `canvas::show` takes the
  eased `shown_view` (kept in `Tab::shown`), which zooms geometrically around the page point
  both views share (`view_between`), and uses it for hover, clicks, the marquee and the
  drawing, so they match the pixels; the tool pill's zoom percentage reads it too
  (`chrome::zoom_percent`). Tiles are planned and requested for the target, so an ease does
  not cancel batches as it crosses tiles. Drags, scrolls and pinches are never eased.
- The command bar is modal: `App::shortcuts` and the native menu ignore everything but its
  own shortcut (and Quit) while it is open or fading, and its results are keyed on the
  query, the active file and its page (an agent may turn the page meanwhile) and the number
  of files. Layer search scores names during the walk and keeps only the best 60 as it goes
  (ties in layer order). Ctrl+Tab does not reach the browser build (browsers keep it); the dots and
  the command bar switch files there.
- The status pill (with the MCP address and Copy token) moves above the tool pill when the
  card is narrow; the tool pill drops − and + first, then the zoom percentage. The file dots
  scroll sideways when they do not fit. As Arc's, every dot is the same size and only colour
  marks the active one (no ring): the others are drained toward grey and faded
  (`chrome::drained`) until hovered. A dot the pointer rests on for 0.5 s shows Arc's dark
  pill with the space's name and its ⌃1…⌃9 keys as caps (`chrome::space_pill`, timed by
  `pill_due`, counting from when the pointer last moved, as a tooltip, and asking for a
  repaint for when it is due; right after one shows, the next dot's shows at once; it rises
  and fades in, glides between dots and fades out). A long name is cut with an ellipsis so
  the pill fits the window. A dot sliding to its new slot is hit-tested, named and anchored
  where it is drawn (the layout slot is only allocated). Badges have nothing behind them, as
  Arc's. Closing is in a dot's right-click menu or a middle click. ⌃1…⌃9 are
  `Binding::Space(n)`, changeable like any shortcut.
- The window title is sent only when it changes: a viewport command asks for another frame,
  so sending it every frame kept the app redrawing while a file was open.
- On the window gradient, small text uses `text_dim` (≥ 4.5:1 on every tint); `text_faint`
  is for hints (≥ 3:1). `theme::tests` checks both, for presets and custom colours.

### Copy menus, marquee and region copy (`apps/figstudio`)

- **Right-click menus** (`copy_menu.rs`, text in `copy.rs`): in the Layers tree, on the canvas and
  in Properties, a node menu offers Copy name, Copy text (TEXT nodes only), Copy node ID, Copy
  link, Copy for agent, and Copy as PNG (2x, same caps as export). The formatters are pure
  functions in `copy.rs`, tested in `copy_tests.rs`.
- **Marquee** (`marquee.rs`): with the Select tool, a primary drag draws a rectangle in page
  coordinates (kept when at least 4 screen points on a side). Esc, a click or a page switch clears it.
- **Region menu** (`region_copy.rs`): a right-click inside the marquee offers Copy region (text:
  the `figstudio://` region link, the file and page, then up to 50 nodes, top-level first,
  from `figstudio_mcp_core::region::nodes_in`) and Copy region as PNG.
- **Region PNG** (`region_png.rs`): the region is rendered off the UI thread (desktop: a thread;
  web: the worker), capped at 4096 px per side and 16 MP, then copied with `ctx.copy_image`.
  The toast gives the size. On the desktop a failed image copy is only logged by egui-winit,
  so the toast says so. On the web, a missing secure context or `ClipboardItem` downloads the
  PNG instead.

### MCP inside the app (`mcp_host.rs`, desktop only)

- **View → Agent access…** opens Settings → Agents; the **Agent access** switch there starts
  the live server. It is off at every launch. `mcp_off.rs` is the same API with no thread, used when the `mcp` feature is off or on
  wasm32, so the UI code does not change.
- **Threads.** `McpHost::start` spawns one thread named `mcp`. It runs a 2-worker tokio runtime,
  binds the first free of 127.0.0.1:3845 to :3855 (`bind_first`), and serves `serve_http` with
  `FigstudioServer::live`. The runtime ends with `shutdown_background`, so a stuck stdin or SSE
  stream cannot block the app. A bind or runtime error comes back as an event, turns the toggle
  off, and shows a toast.
- **Snapshot (app to server).** Each frame while running, `sync` builds a `Key` from the tab ids,
  the document pointers (`Arc<Document>`), page, selection and the viewport in whole units. It publishes a new
  `Arc<Snapshot>` only when the key changes. The server reads the snapshot under a lock held
  only for the `Arc` clone, so the UI thread never waits for a tool call. A call that holds an
  old `Arc<Document>` finishes even when the tab closes.
- **Reveal (server to app).** A `reveal_node` call sends `Command::Reveal` over a channel and
  calls `wake` (`ctx.request_repaint`). The next `frame` drains it: the tab becomes active and
  `Tab::show_node` selects the node and sets `Tab.fit_node` (a page ID switches page instead). `canvas::fit_node` fits it on the next draw. The reply is
  `Ok`, or `Err("tab closed")`. The server waits at most 5 s (`REVEAL_TIMEOUT`).
- **Open link (server to app).** The app's `LiveFiles` is built `with_link_opening`, so
  `reveal_node` with a figma.com link (and no `fileKey`) always sends `Command::OpenLink` and
  waits up to 60 s (`OPEN_TIMEOUT`): the app resolves the link against its tabs and local
  copies alike. `drain` queues it; `frame_ui` hands it to `App::go_to_link` (below) with a
  `Done` callback. Its answer is final: there is no fallback to the active tab, which could
  hold the same node ID in another document. `McpHost::sync_then_answer` publishes the
  snapshot and only then sends the answers, so a caller told a file is open finds it. The
  browser bridge does not resolve links: there a figma.com link must name exactly one open
  tab, else the call fails.
- **Tests:** `src/mcp_host_tests.rs` (the key, reveal with a stale tab, an open-link request,
  answers after the snapshot, a busy-port fallback, no free port, and an HTTP client run);
  `src/ui_mcp_tests.rs` (`reveal_node` over HTTP opening a local copy in the real app).

### Open Figma link (`link_open.rs`)

- **File > Open Figma link…** (Cmd+L) opens a window with one text field: a figma.com or
  `figstudio://` link, or a bare node ID. `parse` turns it into a `FileHint` and a node (an
  instance path names its outer instance; a page ID switches page).
- **Which file** (`plan`): every open tab and every `.fig` directly inside the link folders
  (`figstudio_mcp_core::local_copy`: `FIGSTUDIO_LINK_DIRS`, else `~/Downloads`; **Choose
  folder…** in the window replaces the list for the session) is a candidate, ranked by
  `FileHint::match_stem`: the figma.com key beats the name (the slug with spaces as `-`,
  compared as is and then without a ` (N)` download suffix). A copy that is an open tab counts
  once. One best match is used; when several distinct files tie (two documents can share a
  name), the window lists them to pick, and an MCP request fails. Nothing is chosen by date.
  A `.fig` does not store its figma.com key, so the file name is usually all there is.
- **Off the UI thread:** a link that names a file starts a folder search on a `link-search`
  thread (`LinkOpen::search`, `local_copies` by default). The thread also resolves each
  copy's path (`real_paths`), and the loader resolves each tab's (`Tab::real_path`), so
  `plan`, run by `poll_link_searches` each frame against the tabs open then, recognises an
  open copy without I/O, including one opened while the folders were listed. A listing
  that comes back while a file whose name the link matches is loading is held until that
  load ends (the file is no tab yet, so it could not be told from a copy under another
  spelling); other loads do not hold it. A held listing still fails at the timeout
  (`StillLoading`). A copy whose load failed meanwhile (the worker sends its resolved path
  with `Loaded::Err`; failing that, the path's `.`/`..` are folded by text) still takes part
  in the ranking, so a worse match or the other half of a tie never takes its place; when it
  is the one chosen, the link answers `LoadFailed` and it is not loaded again. Reading a folder can block (a network drive, or macOS asking to allow reading
  `~/Downloads`), and the window keeps drawing meanwhile. `start_search` requests a frame and
  each poll with a search running requests another in 250 ms, so the timeout fires even in
  an idle window. After `SEARCH_TIMEOUT` (20 s) the link fails ("could not be read in time",
  with a permission hint on macOS only) rather than the MCP call ending with "the app did
  not respond"; timeouts are checked before results, so a late result is never used. A new
  link from the window (a file link, or a node ID, shown or not in the file) replaces the
  window's older search, Go on
  the link already being searched for starts nothing, and closing the window cancels its
  search; MCP calls each keep theirs. While a listing is still blocked past the timeout, new
  links fail at once instead of starting more blocked threads, and a link refused that way
  leaves the running search alone. `frame_ui` polls before taking MCP links, so a listing
  that has just come back no longer counts as blocked. The window stays open with its text while searching and
  after a miss, and closes once the link is shown or loading. In the browser the search runs
  inline (there are no folders).
- **Messages:** `LinkError::local` (toasts, the window) names files and folders;
  `LinkError::public` (MCP answers) never carries paths, folder names or file names.
- **Opening** pushes a `Waiting` entry and calls `App::open`; `poll` calls `link_loaded` when
  that path finishes loading (select + `fit_node`), or `link_failed`.
- **Tests:** `src/link_open_tests.rs` (parse, plan, ties and key wins, public messages, an open
  tab, a local copy loaded and answered, an MCP tie, the window and its pick list, a slow
  folder that neither blocks the window nor hangs the call, a replaced window search, a
  blocked listing, the frame a search schedules, a late result, a tab opened during the listing, a node ID or a refused link from the window,
  closing the window, a listing held while a file loads, an unrelated load, a held listing
  timing out, a copy whose load failed),
  `crates/mcp-core/tests/local_copy.rs` (ranking, `(N)` names, ties, folders).

### The browser app (`apps/figstudio` on wasm32)

Build with `scripts/web-build.sh` (nightly; std rebuilt with atomics; shared memory) into
`web/dist`, then serve it with COOP/COEP headers (`scripts/web-build.sh --serve` locally;
`web/_headers` on Cloudflare; a deploy config such as `web/wrangler.jsonc` is per machine
and git-ignored). `web/index.html` loads the module, starts the Rayon pool
(`initThreadPool`, one Web Worker per thread; `?threads=N` overrides) and then calls
`start(canvas)`. Differences from the desktop:

- **The UI thread must never wait on a lock**; browsers forbid blocking the main thread
  (`Atomics.wait` throws). So no worker touches the egui context, and no worker blocks on a
  channel the UI sends to: the render loop polls (running queued pool work while idle), and
  the UI repaints every 16 ms while `Workers::busy()`.
- Work runs on the Rayon pool (`rayon::spawn`), not `std::thread` (unsupported on wasm).
  The render loop uses `render_parallel`.
- **GPU canvas in the browser** (`gpu_scenes.rs`, `gpu_tiles_web.rs`): wgpu objects live on
  the main thread, so the worker builds `vello_gpu` scenes and the main thread renders at most
  4 tiles or 8 ms per frame, WebGPU first, then WebGL2. Queued scenes are pruned to the current
  view and capped (64 scenes); every scene reserves its estimated bytes from one shared 64 MiB
  budget before it is built. Tiles with any filter, over 8 MiB, or past the budget are drawn
  on the CPU, counted. `?renderer=cpu` forces the CPU. A panic aborts the page on wasm, so
  the support scan is the only guard against a `vello_gpu` panic there.
- Files arrive as browser `File`s (picker via `rfd::AsyncFileDialog` with
  `file-handle-inner`, drops via `DroppedFile::web_file`) through `web::Inbox`. Files up to
  256 MB are read whole; bigger archives open lazily (below). Exports render on a worker
  and download through a `Blob` (copied out of the shared memory first, since a Blob cannot
  view a `SharedArrayBuffer`).
- **JavaScript objects stay on the main thread.** A worker that needs bytes of a lazily
  opened file sends a request (`web::WebSource`) and polls for the answer; the main thread
  reads the slice asynchronously in `Inbox::serve_ranges`, called every frame.
- No installed fonts: text without stored glyph outlines is not drawn (counted). egui's
  built-in fonts are Latin only, so when a file's name, page or layer names or text contain
  CJK (`theme::cjk_need`, checked on the worker that opens it) the app fetches
  `fonts/cjk-<variant>.otf` and adds it as the last fallback (`theme::add_cjk_font`).
  View → CJK font selects JP, SC, TC, HK or KR, remembered in localStorage under
  `figstudio.cjk_font`; the initial choice follows the browser language (otherwise JP).
  Switching after first need fetches and replaces the fallback; stale selections are ignored.
  A file whose only CJK is Hangul (punctuation and full-width forms do not count; every
  face has them) switches to KR for the session (`Inbox::korean_override`): only KR has
  Hangul. It is not saved and applies to every tab, so Chinese and Japanese text uses the
  Korean glyph forms until the next menu pick or reload; the menu keeps showing the saved
  choice and says KR is in use. Any menu pick, the current one included, is saved and
  ends the override.
  `scripts/web-font.sh` (called by `web-build.sh`) subsets all five Noto Sans CJK Regular
  faces with `pyftsubset` (Latin, punctuation, kana, bopomofo, full-width and the main
  ideograph block; KR also includes Hangul). `FIGSTUDIO_CJK_FONT` overrides the Noto
  collection path: Regular-only collections use faces 0–4, all-weight `NotoSansCJK.ttc`
  uses 25–29. It must be a `.ttc`, and each output's full face name is checked, so a wrong
  source fails the build. `web-build.sh` clears `dist/fonts` first, so a removed font is not
  deployed again. Each variant is cached under `${CARGO_TARGET_DIR:-target}/web-font`.
  Without Noto CJK or fontTools the build fails (a deploy without the fonts shows CJK as
  boxes); `FIGSTUDIO_ALLOW_MISSING_CJK=1` opts out for a local dev build.
  The desktop app uses an installed CJK font
  (`theme::install_fonts`).

### Big files and images

- `Document::open` reads the whole file (capped at `MAX_FILE_BYTES`, 1 GiB).
  `Document::open_auto` (`crates/fig/src/open.rs`) opens archives over `LAZY_ABOVE` (256 MiB)
  with `Document::open_lazy`; the desktop app and the MCP server use it, the browser uses
  `Document::from_lazy`. A lazy open reads only the canvas, `meta.json` and the thumbnail;
  images stay in `doc.lazy_images` as archive entries (`fig::archive`). Such a document
  cannot be saved. The MCP server renders through `tools::rendering::Rendering`
  (`render_complete` / `render_path_complete`, images read from the registered path).
- Images are decoded per (image, level) in `render::images::Images`, a cache shared by all
  renders (`Renderer::with_images`). Level n is the image shrunk 2ⁿ times; the level comes
  from the paint's on-screen scale. Figma's `imageThumbnail` is used up to 256 px on screen.
  **Draw only the copy for the paint's own level** (fall back to the nearest copy only while
  that one is being read), or frames depend on decode order
  (`one_image_at_two_sizes_does_not_depend_on_decode_order`).
- A frame of a lazy document lists what it is missing in `Frame::wanted`. Every request
  must be answered with `Images::insert_encoded` or `Images::fail`, or it stays in flight
  and is never asked for again. `Renderer::render_complete` loops until nothing is wanted.
  The app's `worker::ImageQueue` reads requests (newest first, four at a time) and
  `canvas.rs` renders again when `Workers::images_arrived` changes while the shown frame
  has `images_loading > 0`.

### The CLI (`apps/figstudio-cli`)

Commands: `info`, `tree`, `render`, `thumbnail`, `doctor`, `resave`, `synth`. Parsing is in
`parse()`, commands in `run()`, help text in `USAGE`. Usage errors print the help and exit 2;
failures on a valid command print one line and exit 1. `doctor` opens the file itself so the
path never reaches an error message.

## 3. Recipes

### Support a new paint type or fix a paint
1. Look at the paint in a real file (peek example above).
2. Change `paint::resolve()`. If it cannot be drawn exactly, still return the best
   approximation and increment `approximated_paints`; if not at all, return `None` and
   increment `unsupported_paints`.
3. Render test with a pixel probe on a fixture that has the paint (`crates/render/tests`).
4. Prove the test fails without your change; run `scripts/thumb-sheet.py`, look at it.

### Support a new effect
1. Add a match arm in `Ctx::draw` (effects loop). `vello_cpu` 0.3 has Gaussian blur and drop
   shadow filters; anything else has to be built from layers and paths.
2. Only one filter layer per node is pushed today (`layers == 0` guards). Stacking several
   needs care; test it.
3. Extend `overhang()` if the effect draws outside the frame. Run the culling test.
4. Keep counting what is still skipped.

### Change the decoder or writer
1. Keep `encoded_message_matches_the_original_bytes` and
   `every_fixture_survives_write_then_read` green. They are the safety net for editing.
2. Errors are `FigError` with a short message that says what is wrong and where (node id),
   never document content beyond ids.
3. Mind memory: decode streams nodes one at a time; do not build a second full tree.

### Add a CLI command
1. Add to `USAGE`, `parse()` (new flags) and `run()`.
2. Add a test in `apps/figstudio-cli/tests/` that runs `env!("CARGO_BIN_EXE_figstudio-cli")`
   and checks exit code and output.
3. If it prints anything about a document, decide whether it may contain private content.

### Change `doctor`
Only counts, Figma schema names (types, field names, enum values), timings and memory may be
printed. If you print a new kind of thing, add its source strings to `secrets()` in
`apps/figstudio-cli/tests/doctor.rs` and prove the test catches a leak.

### Change the app
Keep work off the UI thread. Check it with `scripts/gui-smoke.sh` and look at the
screenshots. Things only a real desktop can check (file dialogs, Wayland, HiDPI, drag and
drop from a file manager) must be listed as unverified in your report.

### Logs and errors
Logs use `tracing`. Libraries only call the macros; each binary installs a `tracing-subscriber`
fmt layer on **stderr** (browser: `tracing-web`, the console) with default level `warn`.
`RUST_LOG=debug figstudio-cli info file.fig` shows more; `figstudio-mcp` defaults to
`warn,figstudio_mcp=info`. For the MCP server, prefer targeted filters such as
`RUST_LOG=figstudio_mcp=debug,figstudio_fig=debug`. rmcp is capped at info unless `rmcp` is
named in `RUST_LOG`, because it logs full requests and responses (`figstudio_mcp::cap_rmcp`,
`EnvFilter.and(..)`: one filter that can only lower rmcp and keeps the `EnvFilter`'s level hint;
used by the server and by the app that hosts it). The `listening on` line and the non-loopback
warning are plain stderr output, not log lines. stdout stays data (CLI output, MCP stdio). Never log
names, text, colours, hashes, node ids or paths: log counts, sizes and kinds.
Errors: libraries return their own structs (`FigError`, `CoreError`) built with `thiserror`,
and `source()` gives the real cause (`io::Error`, zip, codec). Apps use `anyhow` with
`.context("open document")`; no paths in context. `FigError`'s message already contains its
cause, so print an error for people with `figstudio_fig::one_line(&e)` (CLI exit message,
app toasts), never `{:#}`, which repeats it.
Debug spans (`RUST_LOG=debug`): `open`/`from_bytes` (bytes, node/page/image counts),
`render_budgeted` (viewport size), the GPU tile submit, and `tool_call{tool=..}` with
`elapsed_ms` in the MCP server. Keep spans off per-node loops.

## 4. Gotchas (all learned the hard way)

- **Path blobs** can continue drawing after a close without a new move. `geom::blob_path`
  starts the next segment from the subpath start. Without this, text in `noise_and_grain`
  disappears.
- **Vector colours** come from `vectorData.styleOverrideTable` by `styleID`, not only from
  `fillPaints`.
- **Progressive blur** (`blurOpType: PROGRESSIVE`) is skipped on purpose; a uniform blur
  looked worse than none.
- **Thumbnails** stored in the files show the canvas background (black in the fixtures); we
  draw the page colour. In `broken_images.fig` one image is a 7-byte placeholder and one is
  missing, so grey placeholders are correct even though its thumbnail shows photos.
- **NaN** appears in real files. `Val` equality treats identical NaN bits as equal.
- **Sibling order** is the byte order of the fractional index `position` string.
- **Image cache** entries are keyed by SHA-1 and shared between documents; a document that
  lacks an image must not cache "missing" for the others.
- **Vello CPU sizes are `u16`**; `MAX_SIDE` is 16 384 px. Large exports must be capped.
- **egui needs Rust 1.95+.** clippy and rustfmt must be installed for the toolchain you use.
- **Xvfb has no window manager**: focus the window before sending keys or modifier clicks.
- **Culling tests** must compare against a render that is not culled the same way (see
  `docs/TESTING.md`, "Prove the test").
- **wasm threads need explicit linker flags** on current rustc (`--shared-memory`,
  `--import-memory`, TLS exports; see `scripts/web-build.sh`). Without them the build
  succeeds but the memory is not shared and `initThreadPool` cannot work. Check the generated
  JS: `__wbg_init(module_or_path, memory)` means threads are on.
- **`cp -p` back a file you mutated keeps the old mtime**, and cargo then reuses the mutated
  build. `touch` the file after restoring it.
- **`Document.nodes` is public, but `doc.index` is not rebuilt by assigning it.** New or
  changed nodes are then invisible to `node()` and the renderer, with no error. Use
  `set_nodes`, which validates parent links and rebuilds the index. `to_bytes` reads `nodes`
  directly, so a test that only writes bytes can skip the index.
- **A tooltip on a button kills its popup.** `on_hover_text` and `Popup::menu` on the same
  response share the popup id; the menu never opens. The "⋯" button uses
  `icons::raw_button` (no tooltip).
- **Submenu labels carry an arrow.** In a popup, `ui.menu_button("View", …)` is a submenu
  whose accessible label is "View ⏵"; tests use `get_by_label_contains`.
- **The test harness turns egui's animations off before `App::new`.** `theme::apply` keeps a
  zero `animation_time`, so kittest frames stay deterministic (springs jump to their target).
- **A blur view inside the content view hides the whole UI under wgpu.** `window-vibrancy`
  adds its `NSVisualEffectView` to the content view, above wgpu's Metal layer. `vibrancy.rs`
  puts it in the window's frame view instead, below the content view; the window is
  transparent and egui clears to transparent, so the gradient's alpha decides how much blur
  shows. The one `unsafe` block turns eframe's raw `NSView` pointer into a reference.
- **`Fonts::has_glyph` says no for egui's own emoji.** It misses the bundled NotoEmoji, which
  does draw them. `spaces` tests compare the glyph's atlas spot with the box drawn for a
  missing character instead. Apple's colour emoji are not drawn at all (egui draws outlines),
  so the editor offers emoji from that font.
- **`usize::MAX + 1` wraps to 0.** Limits in `read_limited` use saturating arithmetic. A limit
  of `usize::MAX` once made `take()` read nothing, with no error.

## 5. Task board

Pick one, keep to it, and report against its "Done when". Ordered by usefulness.

| # | Task | Where | Done when |
|---|---|---|---|
| T1 | CLI tests (partial: usage/runtime errors and core outputs) | `apps/figstudio-cli/tests/cli.rs` | Still open: every command/error path and every fixture must be covered; tests fail when a command is broken |
| T2 | README: what it is, build (Linux deps, macOS), run, CLI usage, known gaps | `README.md` | A new user can build and run from it on Linux and macOS |
| T3 | `corners.fig`: two outline shapes are drawn below the rectangles but are not in the stored thumbnail. Find out whether they are hidden, off-page, or stale-thumbnail | `crates/render` | Explained in `docs/TESTING.md` known differences, or fixed with a test |
| T4 | Virtualise the Layers panel (draw only visible rows) | `apps/figstudio/src/panels.rs` | 100k-node stress file: Layers expand-all stays under 16 ms per frame (`FIGSTUDIO_PROFILE=1`) |
| T5 | Fit-to-page on huge pages (~0.4 s per frame today): level-of-detail or tile cache | `crates/render`, `apps/figstudio/src/canvas.rs` | 100k-node stress page pans smoothly at fit zoom; culling test still passes |
| T6 | Inner shadow, background blur, blend modes | `crates/render/src/lib.rs` | Each drawn and tested, or still counted; thumb sheet reviewed |
| T21 | MCP inside the app: shared `mcp-core`, embedded desktop server (off by default), right-click Copy and Copy region (issue #9); plan and lanes in `docs/MCP-APP.md` | `crates/mcp-core`, `apps/figstudio-mcp`, `apps/figstudio` | Lanes L0–L9 in `docs/MCP-APP.md` done |
| T20 | GPU canvas with `vello_gpu` (desktop + browser) with CPU fallback; plan and lanes in `docs/GPU.md` | `crates/render`, `apps/figstudio` | Lanes L1–L5 in `docs/GPU.md` done |
| T19 | Clear the approximation counters, one fix at a time (plan and measurements in `docs/APPROXIMATIONS.md`) | `crates/render/src` | Each row of the plan done or explained; counters re-measured and logged there |
| T7 | Progressive blur, noise / grain / texture | `crates/render/src/lib.rs` | `noise_and_grain.fig` matches its thumbnail more closely; skipped count drops |
| T8 | Rotated / skewed gradient check | `crates/render/src/paint.rs` | A fixture with rotated gradients compared with its thumbnail (needs a real Figma file) |
| T9 | ~~wasm build~~ done: browser app with threads (`scripts/web-build.sh`) | — | — |
| T14 | Browser: ~~CJK UI font~~ (done: fetched on demand), text without stored outlines | `apps/figstudio/src/web.rs`, `theme.rs` | Chinese layer names readable in the browser |
| T15 | Faster walk for `render_parallel`: it is single-threaded and is most of the time at fit zoom (`patched()` clones node data) | `crates/render/src/lib.rs` | Stress page at fit: render time falls with more threads |
| T10 | `render --all-pages`, `info --json` | CLI | Covered by T1-style tests |
| T11 | CI running `scripts/check.sh` on Linux and macOS | `.github/workflows` | **Ask the owner first** (private-repo minutes). Green on both |
| T12 | Figma accepts our files | — | A person with Figma opens a `resave` output and a lightly edited file; result recorded in `docs/PLAN.md` |
| T13 | Editing (P6) | all | **Not started until the requester asks** |
| T14 | Live GUI check of export, load dedup and the worker-lost toast (Linux `scripts/gui-smoke.sh`, macOS by hand) | `apps/figstudio/src` | Export runs without blocking; a second open of a loading file makes one tab; recorded in `docs/PLAN.md` |
| T15 | Doctor on a file with a parent cycle or oversized input (report U1 from the audit) | `apps/figstudio-cli/src/doctor.rs` | Doctor reports the cycle as an error and exits 1, with a test |
| T16 | Derive a shared render-work budget from real-file measurements and bound all traversal work | `crates/render/src/lib.rs` | Shared `MAX_RENDER_WORK` test bounds ordinary, mask, and instance work; page 4 completed without reported truncation. Attempt-count telemetry for the final real-file render remains pending. |
| T17 | Make the synth command's `set_nodes` error path a test (`apps/figstudio-cli/src/synth.rs`) | `apps/figstudio-cli/` | A test covers a synth that would create a cycle |
| M1 | MCP: `get_motion_context` (deferred: no public file with motion data to validate the schema) | `apps/figstudio-mcp/src/tools` | A real motion file decoded; the tool tested on that file's shape and on documents built in the test, proven by mutation |
| M2 | MCP: SVG export in `download_assets` | `apps/figstudio-mcp/src/tools/assets.rs`, `crates/render` | A vector writer (or a trustworthy crate) with tests; the tool description updated |
| M3 | MCP: ancestor masks and clips for instance-child screenshots (`I…;…`) | `crates/render/src/focus.rs` | A built document with a masked ancestor renders the same as the node inside the full instance |
| M4 | MCP: nested-instance override visibility in `download_assets original=true` (frames between nested instances use stored visibility, the `shortcut:` in `tools/assets/originals.rs`) | `apps/figstudio-mcp/src/tools/assets/originals.rs` | A hidden override on a nested instance hides its image fills; test proven by mutation |
| M5 | MCP: live-client check (Claude Code, Cursor, VS Code, Codex) over stdio and HTTP | — | A person runs each client; result recorded in `docs/PLAN.md` |
| M6 | MCP: real FigJam and motion files (field shapes come from the schema today) | `apps/figstudio-mcp/src/tools/metadata/figjam.rs` | Output compared with a real board; tests updated |
| T18 | Re-check the file-level RSS for a 54 MB input with the new per-item caps (measured sizes only so far) | `crates/fig` | Peak RSS recorded for the largest real file in `docs/PLAN.md` |

Things an agent cannot verify alone (say so in the report): opening our files in Figma,
running on a real Mac, real Wayland / HiDPI desktops, native file dialogs, private files.
