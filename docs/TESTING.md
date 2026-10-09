# Testing guide

What is tested, how to run it, how to write new tests, and how to prove that a test can
fail. Read `AGENTS.md` first.

## 1. The gate

```sh
scripts/check.sh            # fmt --check, clippy -D warnings, all tests, release build
scripts/check.sh --cross    # also cargo check for aarch64-apple-darwin
FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh   # if the default Rust is older than 1.95
```

It must end with `== all checks passed` before you commit. If clippy or rustfmt is missing:
`rustup component add clippy rustfmt --toolchain <version>`.

`check.sh` runs tests in release mode; use `--release` yourself too so the build is shared.
Run one test with `cargo test --release -p <crate> <name>`.

## 2. What exists today (23 tests)

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
| | `word_matching` | the leak matcher itself |
| | `doctor_errors_do_not_echo_the_path` | error output hides the path |

Not covered by automated tests: the desktop app (use `scripts/gui-smoke.sh`), the browser
app (see "Browser" below), CLI commands other than `doctor` (task T1), Figma opening our
files (needs a person with Figma).

The `parallel` tests run with `cargo test -p figstudio-render --features parallel`;
`scripts/check.sh` runs them. Proven: ignoring a filter's reach in `Recorder` makes the shadow
test fail (a channel off by 60).

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
No unit tests yet. Keep logic that can be tested (geometry, view maths) in plain functions
in `tab.rs` / `canvas.rs` so it can be tested without a window.

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
click, zoom percentage changed in the status bar, fit restored. `GUI_SMOKE_SECONDS` and
`GUI_SMOKE_LOAD_WAIT` adjust timing for big files.

Manual checks only a real desktop can do (list them as unverified unless you did them):
File → Open and Export dialogs, drag and drop from a file manager, Wayland session, HiDPI /
Retina scaling, trackpad pinch, macOS menu and shortcuts (Cmd).

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
a fixture on the page, and the status bar shows layers drawn and the render time. Compare
`?threads=1` with the default. Headless Chromium works (`--use-angle=swiftshader`); drop a file
by dispatching `dragenter`/`dragover`/`drop` with a `DataTransfer` on the canvas.

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
