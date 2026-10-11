# Implementation guide

How figstudio is built, how data flows through it, how to add things, and what is left to
do. Read `AGENTS.md` first. Testing is in `docs/TESTING.md`; status is in `docs/PLAN.md`.

## 1. Map of the code

```
crates/fig/src/          no UI, no rendering; builds for wasm (browser app)
  lib.rs                 public exports
  doc.rs                 container (zip / raw), chunks, decompress, Document, Node, reindex
  read.rs                Kiwi bytes -> Val, driven by the schema stored in the file
  val.rs                 Val, Obj (field list), Guid, intern()
  write.rs               Val -> Kiwi bytes, canvas chunks, zip; Document::to_bytes / save
  resolve.rs, resolve/   public instance resolver: Overrides scope stack and merge (shared
                         with the renderer), component properties, swaps, walk / path lookup
crates/render/src/       Document -> pixels with vello_cpu; builds for wasm without fonts
  lib.rs                 Renderer, Stats, Viewport, the draw walk, culling, instances, masks,
                         bounds / hit test helpers; render_parallel (feature parallel)
  record.rs              Canvas trait (where drawing goes); Recorder for parallel strips
  geom.rs                transforms, sizes, path blobs, base shapes, clipping rules
  paint.rs               solid / gradient / image paints, image cache
  text.rs                stored glyph outlines; installed-font layout (feature system-fonts)
apps/figstudio-cli/src/
  main.rs                argument parsing and commands
  doctor.rs              privacy-safe structure report
  synth.rs               stress-file generator
apps/figstudio-mcp/src/  offline MCP server (separate library and binary)
  main.rs                clap options, stdio / Streamable HTTP, Ctrl-C / SIGTERM shutdown
  lib.rs                 rmcp handler, capabilities, /mcp HTTP service
  files.rs               lazy shared local document registry and fileKey resolution
  node_id.rs             node IDs, Figma URLs, unresolved instance paths
  error.rs               canonical error type; tools/mod.rs maps tool failures
  tools/mod.rs           whoami and list_files; metadata router declared here
  tools/metadata.rs      get_metadata: typed inputs, quick-xml, bounded iterative tree walk
                         New tools belong in tools/<name>.rs with a typed input struct.
                         Use #[tool_router(router = <name>_router, vis = "pub(crate)")]
                         on an impl FigstudioServer, declare the module here, and combine
                         Self::tool_router() + Self::<name>_router() in lib.rs new().
apps/figstudio/src/      egui 0.36 app: desktop, and browser (wasm32, threads)
  main.rs                App: menus, shortcuts, panel layout, message polling, export
  canvas.rs              canvas input, render requests, frame re-projection, outlines
  tab.rs                 per-document state (Tab) and the page<->screen mapping (View)
  panels.rs              Pages, Layers tree, Properties
  worker.rs              background threads: file loading, rendering (OS threads or Web Workers)
  web.rs                 browser only: start(), file picker, dropped files, downloads
  theme.rs               colours, spacing, CJK font fallback (desktop) / detection (web)
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
`resolve`) per node. The renderer adds only its render-side steps on top of `merge`:
uniform-scale and constraint corrections, glyph/stroke scaling, stale glyph and stroke
geometry detection. `draw_instance` pushes every `symbolOverrides` entry (and `derivedSymbolData`)
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
the app shows them in the status bar. Add a field when you add a new kind of gap.

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
  renders on one long-lived thread that keeps only the newest request per tab.
- `canvas::show` builds a key from page, zoom, pan, size and pixel ratio; when it changes it
  sends a `RenderJob` carrying the `View` it was made for. `canvas::accept` keeps the newest
  frame; until a new frame lands the old texture is re-projected from its `View` to the
  current one, so pan and zoom stay smooth.
- `Tab` holds the document (`Arc<Document>`), page, view, selection, hover, expanded tree
  nodes and stats. `Tab::select` switches page and expands ancestors so the Layers tree can
  scroll to the layer.
- Keyboard shortcuts are in `App::shortcuts`. Shift+digit uses `shifted_digit` because the
  logical key depends on the keyboard layout (Shift+1 is `!` on US layouts).
- `FIGSTUDIO_PROFILE=1` prints per-frame UI time, render time, drawn and culled counts.

### The browser app (`apps/figstudio` on wasm32)

Build with `scripts/web-build.sh` (nightly; std rebuilt with atomics; shared memory) into
`web/dist`, then serve it with COOP/COEP headers (`scripts/web-build.sh --serve` locally;
`web/_headers` on Cloudflare). `web/index.html` loads the module, starts the Rayon pool
(`initThreadPool`, one Web Worker per thread; `?threads=N` overrides) and then calls
`start(canvas)`. Differences from the desktop:

- **The UI thread must never wait on a lock**; browsers forbid blocking the main thread
  (`Atomics.wait` throws). So no worker touches the egui context, and no worker blocks on a
  channel the UI sends to: the render loop polls (running queued pool work while idle), and
  the UI repaints every 16 ms while `Workers::busy()`.
- Work runs on the Rayon pool (`rayon::spawn`), not `std::thread` (unsupported on wasm).
  The render loop uses `render_parallel`.
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
  CJK (`theme::needs_cjk`, checked on the worker that opens it) the app fetches
  `fonts/cjk.otf` once and adds it as the last fallback (`theme::add_cjk_font`).
  `scripts/web-font.sh` (called by `web-build.sh`) makes that file: Noto Sans CJK HK
  Regular cut down to ~5 MB with `pyftsubset` (Latin, punctuation, kana, bopomofo,
  full-width, the main ideograph block; no Hangul). Without Noto CJK or fontTools the
  build skips it and CJK shows as boxes. The desktop app uses an installed CJK font
  (`theme::install_fonts`).

### Big files and images

- `Document::open` reads the whole file (capped at `MAX_FILE_BYTES`, 1 GiB). The app opens
  archives over 256 MB with `Document::open_lazy` (desktop) / `Document::from_lazy`
  (browser): only the canvas, `meta.json` and the thumbnail are read; images stay in
  `doc.lazy_images` as archive entries (`fig::archive`). Such a document cannot be saved.
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
| T18 | Re-check the file-level RSS for a 54 MB input with the new per-item caps (measured sizes only so far) | `crates/fig` | Peak RSS recorded for the largest real file in `docs/PLAN.md` |

Things an agent cannot verify alone (say so in the report): opening our files in Figma,
running on a real Mac, real Wayland / HiDPI desktops, native file dialogs, private files.
