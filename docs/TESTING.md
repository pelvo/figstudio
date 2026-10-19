# Testing guide

What is tested, how to run it, how to write new tests, and how to prove that a test can
fail. Read `AGENTS.md` first.

## Browser Agent access (manual)

`mcp_web.rs` is wasm-only and uses browser Window/WebSocket APIs, so desktop
`egui_kittest` cannot instantiate it. The desktop window test covers the shared
menu-to-window action, not the browser bridge controls. After a browser build:

- View → Agent access opens the window directly (no nested bridge menu).
- Click and type in both URL and pairing-token fields; neither closes the window.
- Connect and Disconnect leave the window open; status reflects the connection.

This browser GUI checklist remains unverified; a successful wasm build only proves
compilation.

## Native menus (manual, desktop)

Use an extra instance with public fixtures, agent access off; do not disturb a live
app or tunnel. Linux/web and headless tests retain the drawn bar. Check macOS:

- App, File, View, Window and Help appear in the OS bar; the drawn bar is absent.
- About opens the help window from both the app and Help menus; Hide/Hide Others/
  Show All and Window Minimize/Zoom work.
- Open by click and Cmd+O opens exactly one picker; cancel it. Cmd+W closes one tab.
- Export page at each scale; export selection only when selected. Cmd+Shift+E acts once.
- No-tab and no-selection items are disabled; changing tabs updates them.
- Shift+1 fits once and Shift+0 restores actual size via egui (native menu labels
  show hints only); text editing does not trigger zoom.
- Panel checks follow state and clicks toggle once; Agent access opens its window.
- Custom Quit / Cmd+Q closes through eframe; no orphan server is left.
- Forced attach failure retains a usable drawn menu (build-time failure injection).
  `menu_dispatch::tests` covers the pure attached/fallback visibility decision and
  macOS/Windows shortcut ownership; it does not exercise OS attachment.

Windows: repeat the item/state checklist; Ctrl shortcuts remain egui-owned because
winit does not call TranslateAcceleratorW. Check Close, resize, minimize and Quit,
and verify the menu does not outlive its HWND. Native behavior is not headless-tested.

## 1. The gate

```sh
scripts/check.sh            # fmt --check, clippy -D warnings, all tests, release build
scripts/check.sh --cross    # also cargo check for aarch64-apple-darwin
FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh   # if the default Rust is older than 1.95
```

It must end with `== all checks passed` before you commit. If clippy or rustfmt is missing:
`rustup component add clippy rustfmt --toolchain <version>`.

`check.sh` lints and tests with `--profile fast` (release optimisation, no LTO,
incremental); use it yourself too so the build is shared. Run one test with
`cargo test --profile fast -p <crate> <name>`. Avoid `cargo test --release`: it thin-LTO
links each of the ~100 test binaries, so a change in `crates/fig` costs ~20 minutes instead
of ~40 seconds.

## 2. What exists today (50 tests in the gate)

| File | Test | Guards |
|---|---|---|
| `crates/fig/tests/decode.rs` | `decodes_every_fixture_with_its_own_schema` | all 7 fixtures decode; node counts |
| | `builds_the_tree_in_sibling_order` | parent/child links, fractional-index order |
| | `reads_metadata_images_and_blobs` | `meta.json` name, images, blobs, thumbnail |
| | `refuses_files_that_are_not_fig` | bad input is an error, not a panic |
| | `guid_parses_and_prints` | `12:34` ids |
| `crates/fig/tests/roundtrip.rs` | `every_fixture_survives_write_then_read` | write → read gives the same document; second write is byte-identical |
| | `encoded_message_matches_the_original_bytes` | our Kiwi message equals Figma's bytes for every fixture |
| | `edits_are_written` | renamed / hidden layer survives a save |
| | `unknown_fields_and_wrong_types_are_errors_not_panics` | writer validation |
| `crates/render/tests/render.rs` | `every_fixture_renders_with_content` | nothing renders blank |
| | `vector_paths_use_their_style_override_paints` | red / yellow / purple areas in `vector.fig` |
| | `stored_glyph_text_is_drawn` | text from stored outlines; skipped effects are counted |
| | `instances_expand_their_component` | instance count, no missing components, pixels drawn |
| | `frames_clip_their_children` | frame background drawn |
| | `hit_test_finds_the_deepest_layer` | click → deepest layer; empty space → none |
| | `culling_never_changes_what_is_visible` | window render equals the same area of a full render |
| | `malformed_path_blobs_do_not_panic` | close-then-line, truncated and unknown commands |
| | `hidden_layers_are_not_drawn` | `visible: false` |
| | `parallel_strips_match_a_single_render` (feature `parallel`) | strips on 4 threads: same `Stats`, pixels within 1 per channel, every fixture at 1× and 2.5× |
| | `parallel_strips_keep_shadows_that_cross_strip_edges` (feature `parallel`) | large drop shadows drawn into the strip below their layer |
| `apps/figstudio-cli/src/doctor.rs` | `scrub_hides_hashes_only`, `numbers_get_separators` | helpers |
| `apps/figstudio-cli/tests/doctor.rs` | `doctor_output_contains_no_document_content` | no names, text, fonts, hashes, paths in the report |
| `crates/fig/tests/cycles.rs` | `a_parent_cycle_is_refused_on_load` | parent cycle is an error, not a stack overflow |
| `crates/fig/tests/limits.rs` | `a_member_that_inflates_past_the_chunk_limit_is_refused` | decompression bomb refused |
| `crates/fig/tests/save.rs` | `a_failed_rename_leaves_no_temp_file_behind` | failed save removes its temp file |
| `crates/fig/tests/preserve.rs` | `a_present_but_empty_image_survives_a_save` | empty image kept on save |
| `crates/fig/tests/integers.rs` | `a_uint_wider_than_32_bits_is_refused_not_wrapped` | out-of-range integer is an error |
| `crates/render/tests/budget.rs` | `nested_instances_stop_at_the_work_budget_and_the_skip_is_counted`, `wide_symbols_and_masks_share_the_same_attempt_budget` | shared attempt budget includes ordinary and instance work; skipped work counted |
| `crates/render/tests/image_cache.rs` | `the_cache_stays_under_its_byte_bound_across_many_documents`, `failed_decodes_are_not_retained_and_evicted_arc_remains_valid` | cache bounded by bytes and entries; failed decodes not retained; Arc survives eviction |
| | `a_document_without_the_bytes_does_not_poison_the_cache_for_one_that_has_them` | missing image does not poison cache |
| `apps/figstudio/src/worker.rs` (tests) | `worker_admits_only_one_export_and_releases_slot_on_completion`, `export_writes_a_png_of_the_page`, `export_reports_a_failed_write_instead_of_panicking` | admission bounded at worker boundary; PNG decodes with dimensions; completion and error |
| `apps/figstudio/src/state.rs` (tests) | `a_disconnected_worker_is_not_an_empty_queue`, `a_dead_worker_is_reported_once_not_every_frame`, `opening_a_file_twice_starts_one_load_and_focuses_an_open_tab`, `a_second_open_of_a_pending_file_is_ignored_until_it_finishes`, `byte_backed_drop_uses_open_action_for_pending_and_loaded_tabs` | disconnect is not empty; report once; dedup, focus, retry for file and byte-backed opens |
| | `word_matching` | the leak matcher itself |
| | `doctor_errors_do_not_echo_the_path` | error output hides the path |

Not covered by automated tests: the desktop app (use `scripts/gui-smoke.sh`), the browser
app (see "Browser" below), CLI commands other than `doctor` (task T1), Figma opening our
files (needs a person with Figma).

The `parallel` tests run with `cargo test -p figstudio-render --features parallel`;
`scripts/check.sh` runs them. Proven: ignoring a filter's reach in `Recorder` makes the shadow
test fail (a channel off by 60).

The `gpu` tests (`crates/render/tests/gpu.rs` and `gpu_refuse.rs`, `cargo test -p figstudio-render --features gpu`,
run by `scripts/check.sh`) render recorded tiles with `vello_gpu` on this machine's GPU and
compare them with the CPU tiles (tolerances and measurements in `docs/GPU.md`, L2 notes). With
no GPU adapter they print `SKIPPED: ...` and pass; `WGPU_BACKEND=vulkan` on a Mac forces that.
Proven: an opacity layer drawn opaque on the GPU fails (126/255); dropping the image refusal
fails the refusal test and vello_gpu panics on the fixture tile.
Nested filters: checking each filter alone fails `nested_filters_that_outgrow_gpu_intermediates_are_refused`;
`every_tile_the_scan_accepts_renders_on_the_gpu` sweeps blur radii across the limit and renders
every accepted tile.

## 3. Fixtures

`tests/fixtures/*.fig` come from fig2sketch (MIT, `tests/fixtures/LICENSE-fig2sketch`).

| File | Exercises (from `figstudio-cli doctor`) |
|---|---|
| `broken_images.fig` | 2 image fills: one image is a 7-byte placeholder, one is missing from the file |
| `corners.fig` | rounded rectangles and 4 vectors in frames (see T3 about two of them) |
| `noise_and_grain.fig` | 2 text layers with stored outlines, radial gradient fills, progressive foreground blur, noise and grain effects |
| `sections.fig` | a section with frames and 6 main components (symbols drawn in place) |
| `stacks_wrap.fig` | 6 auto-layout (wrap) frames with 48 children |
| `structure.fig` | 1 component with 2 instances and 4 overrides, 2 image fills, text without stored outlines (installed fonts), a drop shadow, vectors |
| `vector.fig` | one vector network with per-path colours (`styleOverrideTable`) |

Rules: never commit a private or client file. A new fixture must be small, made in Figma
(or with a clear licence), and listed here. A file made with our own writer is **not** a
substitute for a Figma-made file when the test is about matching Figma.

### Browser MCP core (Lane 1)

`cargo test -p figstudio-mcp-core --test wasm_rpc` exercises the SDK-free JSON-RPC
boundary against the public `structure.fig` fixture. Repeat with `--no-default-features`
for filesystem-free inline assets. `cargo test -p figstudio-mcp --test wasm_schema`
compares the browser names, property names and required inputs to rmcp's actual live
server, explicitly documenting browser omissions, and checks shared prompt text.

Mutation controls for the new tests (restore source before running the gate):

| Test in `crates/mcp-core/tests/wasm_rpc.rs` | Targeted mutation |
|---|---|
| `protocol_negotiates_and_returns_json_rpc_errors_without_notification_replies` | change initialize's protocol version |
| `tools_dispatch_core_outputs_and_inline_png_assets` | use directory assets instead of inline PNGs |
| `live_tab_routing_selection_and_reveal_use_the_same_snapshot` | send reveal to tab ID plus one |
| `prompts_supply_shared_guidance_and_inventory_with_data_preferences` | remove both prescribed rules paths from shared text |
| `byte_backed_reader_supplies_lazy_originals_and_never_falls_through` | discard successful custom entry reads |
| `default_design_assets_are_inline_without_filesystem_support` | default to directory assets without `fs` |

The native drift test fails when `search_design_system.query` is no longer required
in the browser schema. Every control must fail at a runtime assertion, not compilation.
These tests do not exercise a real browser Web Worker, HTTP relay or UI reveal queue;
those belong to the later browser/relay lanes. The core reveal hook reports admission,
not completion of a UI frame. Entry readers return decompressed bytes using the existing
archive parser; a failed custom read does not fall through to a native path.

## 4. Writing tests

### Decoder / writer (`crates/fig/tests`)
- Use the `fixture(name)` helper pattern from `roundtrip.rs`.
- Compare documents node by node (`assert_same`) so a failure names the node.
- For writer changes, the byte-identical test is the real check; keep it green.
- Malformed input: assert an `Err`, and assert the message is useful (`e.to_string()`).

### Renderer (`crates/render/tests/render.rs`)
- `render_page(doc, origin, w, h)` renders the first page at 1:1 with `origin` at pixel (0, 0)
  and installed fonts off (`system_fonts = false`), so results do not depend on the machine.
- Probe pixels with `pixel()` or count matching pixels with `count()`.
- Thresholds: measure the value on a correct render, write it in a comment, and assert a
  bound with margin (about 60–70% of the measured count). Example:
  `// Areas measured on a correct render: red ~1570, purple ~380, yellow ~120 px.`
- Allow small per-channel tolerance (±8) for anti-aliasing; never compare whole images exactly.
- Check `Stats` too: a feature that is counted as skipped must stay counted until it is drawn.

### CLI (`apps/figstudio-cli/tests`)
- Run the real binary: `Command::new(env!("CARGO_BIN_EXE_figstudio-cli"))`.
- Check the exit code (0 ok, 1 failure, 2 usage) and the important lines of output.
- Write outputs under `std::env::temp_dir()`; do not leave files in the repo.

### App
Keep logic that can be tested (geometry, view maths) in plain functions in `tab.rs` /
`canvas.rs`. The UI itself is driven headlessly, see section 7b.

## 5. Prove the test

A test only counts if you have seen it fail. Before committing:

1. Break the code in the smallest way that only your test should catch.
2. Run that test and see it fail with a clear message.
3. Restore the code (`git diff` must show only your intended change) and see it pass.

Do it on a copy you can restore (`cp file /tmp/x.bak`, then copy back), and say in the
commit body that you did it. Examples from this repo:

| Test | Break | Result |
|---|---|---|
| `culling_never_changes_what_is_visible` | grow-bounds replaced by `w * 0.2` (frame shrunk) | failed: 77 312 pixels differ ✅ |
| same | `|| w > 30.0` added to the cull condition | **did not fail**: the full render and the window render were both culled the same way. A comparison test must have one side that the mutation does not touch |
| `doctor_output_contains_no_document_content` | print the longest layer name | failed with the leaked name ✅ |
| workspace `unsafe_code` lint | an `unsafe` block in `synth.rs` | build error ✅ |

## 6. Visual check (renderer changes)

```sh
cargo build --release -p figstudio-cli
scripts/thumb-sheet.py -o /tmp/sheet.png          # needs Pillow
```

Left: the thumbnail Figma stored in the file. Right: our render. Open the image and compare
each row. Known differences that are not bugs:

- Thumbnails show Figma's canvas background (black here); we use the page colour.
- `broken_images.fig`: the thumbnail shows photos, but the file has a 7-byte placeholder and a
  missing image → grey boxes are correct.
- `noise_and_grain.fig`: progressive blur and noise/grain are not drawn yet (counted).
- `corners.fig`: two outline shapes below the rectangles appear in our render but not in the
  thumbnail. **Not yet explained** (task T3).

The script prints each file's `Stats`; anything "skipped", "missing" or "approximated" there
is a known gap, not a pass.

## 7. GUI smoke test (Linux)

```sh
cargo build --release -p figstudio
scripts/gui-smoke.sh /tmp/gui tests/fixtures/structure.fig
GUI_SMOKE_CLICK="900 560" scripts/gui-smoke.sh /tmp/gui tests/fixtures/structure.fig  # hits a card
```

Runs the app on a virtual screen (Xvfb, 1400×880), clicks, zooms with Ctrl+wheel, presses
Shift+1, and saves `1-open.png 2-select.png 3-zoom.png 4-fit.png` plus `profile.log`.
Look at every screenshot: file loaded, selection outline and Properties filled after the
click, zoom percentage changed in the tool pill, fit restored. `GUI_SMOKE_SECONDS` and
`GUI_SMOKE_LOAD_WAIT` adjust timing for big files.

Manual checks only a real desktop can do (list them as unverified unless you did them):
File → Open and Export dialogs, drag and drop from a file manager, Wayland session, HiDPI /
Retina scaling, trackpad pinch, macOS menu and shortcuts (Cmd).

## 7b. Headless UI tests (egui_kittest)

`apps/figstudio/src/ui_tests.rs` (+ `ui_copy_tests.rs`, `ui_mcp_tests.rs`, `ui_chrome_tests.rs`) run the real `App`
on `tests/fixtures/structure.fig` inside `cargo test`: no display, X server, GPU or xdotool, on
macOS, Linux and CI. They use `egui_kittest` (dev-dependency, `eframe` feature only; no wgpu,
no snapshot images). `App::new` takes `eframe::CreationContext`, which kittest builds with no
wgpu state, so tiles render on the CPU path.

```sh
cargo test -p figstudio ui_tests          # 28 tests, about 2 s
```

How to write one: `harness()` returns a `Harness<App>` with the file loaded and fitted;
`centre_of(&h, guid)` turns a node into a screen point; `right_click`, `drag`, `click_label`
send pointer events; `step` also collects what the frame sent to the clipboard (egui
`OutputCommand::CopyText` / `CopyImage`), which is how tests read "copied" text. Find widgets
with `h.get_by_label("Copy link")` (plain `ui.label`s are matched by their accesskit value, see
`shown_addr`). Menus open a frame or two after the click: use `wait_for`, not a fixed step count.
Tests that start the MCP server use whatever port `bind_first` gives and read it from the status pill.
Painted widgets (icon buttons, rows, pills) set an accesskit label with `widget_info`, so
`get_by_label("Hand tool")` finds them; the drawn menus sit behind the "Menu" button.

Cannot be covered here: the real OS clipboard (paste into an editor or Preview), a real
window manager, native file dialogs, GPU tiles, HiDPI and trackpad gestures.

## 8. Performance

```sh
target/release/figstudio-cli synth tests/fixtures/structure.fig -o /tmp/big.fig --copies 4500
target/release/figstudio-cli doctor /tmp/big.fig       # decode, render, write-back, memory
FIGSTUDIO_PROFILE=1 target/release/figstudio /tmp/big.fig 2>/tmp/profile.log
```

`synth` copies the first page's layers N times (`--pages` for more pages). The 4500-copy
file has about 103 000 nodes. Baseline on the development machine (release build):

| Measure | Value |
|---|---|
| decode | 0.30 s |
| memory after opening | 470 MB |
| whole page at fit | 0.40–0.45 s |
| 1400×880 window at 100% | 6–11 ms, ~9 000 layers culled |
| write back | 0.75 s, identical |
| GUI frame (9 000 Layers rows) | ~11 ms median, 19 ms max |

Numbers depend on the machine: compare before and after on the **same** machine. A
regression over 20% needs a reason in the commit message. Do not commit stress files.

### Browser

`scripts/web-build.sh --serve` serves `web/dist` on http://127.0.0.1:8765 with the headers
threads need. In a browser: the console shows no errors, `crossOriginIsolated` is true, drop
a fixture on the page, and the status pill shows layers drawn and the render time. Compare
`?threads=1` with the default. Headless Chromium works (`--use-angle=swiftshader`); drop a file
by dispatching `dragenter`/`dragover`/`drop` with a `DataTransfer` on the canvas.

Open big files the way a user drops them, without the page holding the bytes: serve the
file next to `web/dist` (a symlink in a copy of the directory), then in the page
`fetch()` it, `blob()` it (Chromium keeps big bodies on disk), wrap it in a `File` and
dispatch the drop events. Playwright's `wait_for_function` and `wait_for_selector` hang on
this page (cross-origin isolated); poll `page.evaluate` instead. `window.figstudioStatus`
holds numbers only (tabs, page, drawn, culled, images loading / missing / read, image cache
and Wasm memory bytes, errors); `figstudioPage(n)` switches page. Never screenshot a
private file.

Measured on the development machine (32 threads, headless Chromium, 1400×880 window,
4500-node `synth --copies 400` of structure.fig): 100% zoom 15 ms on 1 thread, 6–7 ms on 8+;
fit zoom 70–90 ms whatever the thread count, because the single-threaded document walk
dominates there (task T15).

## 9. Privacy

`doctor` output is meant to be pasted by users whose files are private. After any change to
`doctor.rs`, run its tests and read one full report yourself
(`target/release/figstudio-cli doctor tests/fixtures/structure.fig`). Never paste anything
else from a private file (no screenshots, no `tree` or `info` output) unless the owner says so.

## 10. Writer and Figma

```sh
target/release/figstudio-cli resave in.fig -o out.fig   # reads back and compares
```

`resave` checks our own round trip only. Whether Figma opens `out.fig` must be checked by a
person with Figma (task T12); record the result and the Figma version in `docs/PLAN.md`.

## 11. Reporting your work

End every task with a short report (commit body or PR description):

```
What changed:      <one or two lines>
Checks run:        scripts/check.sh → all checks passed (N tests)
                   <test name> proven to fail when <mutation>
Looked at:         thumb sheet / GUI screenshots (which ones)
Not verified:      <real desktop, macOS, Figma, big private file, …>
Numbers:           <performance before → after, same machine>   (if relevant)
```

## 12. MCP server (`apps/figstudio-mcp/tests`)

All tests are integration tests under `tests/` and run in the gate. They use the public
fixtures (`structure.fig`, `sections.fig`) or documents **built inside the test** (variables,
FigJam and most design-context mappings; the fixtures have no variables or FigJam kinds). No private file is used.

**Protocol tests**
- `stdio.rs` spawns the real binary (`CARGO_BIN_EXE_figstudio-mcp`), writes JSON-RPC lines
  to stdin and checks stdout: only protocol lines, one `initialize` reply, a tool listing and
  calls and SIGTERM shutdown.
- `remote.rs` (remote agents): loopback needs no token; a non-loopback bind returns 401 for a
  missing, wrong, same-length-wrong, longer or shorter token and 200 for the right one (plus an
  rmcp client with the token); `Host` allowlist; wildcard bind refused; the binary prints a
  token only for a remote bind. `access.rs` unit tests cover the rules and that `Debug` hides the token.
- `http.rs` starts the server on port 0 and sends plain POSTs (reqwest). One test per protocol
  version (2025-03-26, 2025-06-18, 2025-11-25, 2026-07-28): `initialize`, `tools/list`,
  `whoami`, an unknown tool, and a non-loopback `Host` that must be refused.
- `bridge*.rs` (browser bridge): a tokio-tungstenite client plays the browser against
  `serve_bridge` in process; `bridge_cli.rs` spawns the real `--bridge` binary, parses its printed
  token and pairs a socket. The wasm client itself (`mcp_web.rs`) is checked by hand; its pure
  helpers (`mcp_wire.rs`, `mcp_state.rs`) have native unit tests in the app crate.
- `prompts.rs`, `files.rs` and most per-tool files use rmcp's client over an in-process duplex
  pipe against the same handler the transports use.

**Tool tests** (one file or pair per tool: `metadata*.rs`, `screenshot.rs`, `variables*.rs`,
`instances*.rs`, `design*.rs`, `assets.rs`, `figjam.rs`, `codeconnect*.rs`, `lazy.rs` for
forced-lazy opens matching eager ones). They check output
shape, every nodeId form, errors as `isError` results, caps at their exact boundary, and
unchanged fixture bytes. `*_review.rs` files hold regressions from the cross-family reviews;
each failed on the code it reviewed. Code Connect tests write sidecars into a temp directory
(copy of a fixture), never beside the repo fixture.

Prove each test as in section 5 (mutate the code, see it fail).

**Manual smoke test** with the release binary (stdio, one JSON object per line):

```sh
cargo build --release -p figstudio-mcp
{ printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_metadata","arguments":{"nodeId":"1:38"}}}'
  sleep 2; } | target/release/figstudio-mcp tests/fixtures/structure.fig
```

Keep stdin open (`sleep`) until the replies arrive; closing it ends the session. Expect 14
tools in the listing (the 3 live tools appear only in the app). For HTTP, run `target/release/figstudio-mcp --http tests/fixtures/structure.fig`
and POST to `http://127.0.0.1:3845/mcp` with `Accept: application/json, text/event-stream`.
Do not run `add_code_connect_map` against a fixture in `tests/fixtures`: it writes a sidecar
beside the file. Copy the fixture to a temp directory first.

**Live mode tests** (`live.rs` and `live/support.rs`, 9 tests) use a fake snapshot built from
the fixtures and a fake app loop that answers each `reveal_node` command. They check the
tab lists, `get_selection`, the `nodeId` defaults from the selection, the standalone
"nodeId is required" error, reveal (and its 5 s timeout), a snapshot replaced while a
call runs, and `bind_first` on a busy port. `tests/http.rs` asserts that no reply carries
`Access-Control-Allow-Origin`, including preflight.

**Golden harness** (`tests/golden.rs`, `tests/golden_figjam.rs`, both `#[ignore]`d). It
records every tool's reply for the 7 fixtures and a synthetic FigJam board, 197 records. To
compare a change with a base revision, run `scripts/mcp-golden.sh <rev>` (for example
`scripts/mcp-golden.sh HEAD`). It builds the base in a temporary worktree under
`target/golden-base`, records both trees, and exits 0 only when `diff -r` is empty. Logs
are in `target/golden-*.log`. An intended output change (a tool description or a schema)
shows as a diff; name it in the commit message and regenerate the base.

**Desktop app (`apps/figstudio`, `src/mcp_host_tests.rs`).** Five tests: the snapshot key,
reveal with a stale tab, a busy-port fallback, no free port, and an HTTP client that lists
tabs, reveals and selects over a real port. The HTTP test runs only with the `mcp` feature
(the default). `check.sh` also runs clippy with `--no-default-features` for the app.

**`mcp-core` without default features.** `check.sh` runs its tests with
`--no-default-features`, as the browser app uses it (no files, no system fonts). The wasm32
build of the same crate is a separate `check.sh` step; it is skipped with a message when the
target is not installed.

**GPU-free.** None of the MCP or copy tests needs a GPU. The app's GPU renderer tests are
in `docs/GPU.md`. The CLI and export always render on the CPU, so the golden output does not
depend on the GPU.

Not covered: a live Claude Code, Cursor, VS Code or Codex client, real Ctrl-C, Windows, a
real right-click or drag in the app window, paste into an editor or Preview, and the browser
clipboard.
