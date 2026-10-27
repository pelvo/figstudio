---
pr: 1
branch: web-threads
title: Browser build with threads, large-file loading and CJK font; rendering fixes from fix/c1-rendering
---

## P1b — Large files (requester's file cannot be shared)
- [x] **Image-heavy files bigger than the Wasm memory** (requester's 1.7 GB file, 194k
      layers, 1 893 images = 1.5 GB of the file). Before: the browser read the whole file
      into the 4 GiB Wasm memory, kept every image's bytes and decoded them at full size,
      so it ran out of memory (desktop needed 4.7 GB to open and grew to 8 GB rendering).
      Now:
      - `fig::archive` reads the zip piece by piece (end record, central directory, zip64,
        stored and deflated entries); `Document::open_lazy` / `from_lazy` read only the
        canvas, `meta.json` and the thumbnail, and list images in `lazy_images`. A lazily
        opened document cannot be saved (the writer refuses).
      - `render::images`: decoded images are cached per (image, level), level n being the
        image shrunk 2ⁿ times, at the level the view needs; Figma's `imageThumbnail`
        (≤ 512 px) is drawn when the image is ≤ 256 px on screen. 1 GiB budget, least
        recently drawn dropped first. The choice depends only on the paint and the view,
        so a frame never depends on what was decoded earlier (before, page 7 of the file
        rendered differently on the second render).
      - Frames list unread images in `Frame::wanted`; the app reads them on workers (four
        at a time) and renders again as they arrive; `render_complete` reads them inline
        (exports). The browser keeps the `File` on the main thread and reads slices with
        `Blob.slice().arrayBuffer()`; files over 256 MB open this way, smaller ones as before.
      Result on the requester's file (numbers only): browser opens in 1.4 s at 537 MB of
      Wasm memory; all 9 pages render with no missing images (each settles in ≤ 1.1 s,
      e.g. 532 images read for one page); zooming to 260% peaks at 906 MB. Desktop opens in
      0.7 s at 672 MB. Lazy and whole renders of every page match pixel for pixel at fit and
      8× zoom. CLI commands other than the app still use `Document::open` (1 GiB cap).

## P2 — Headless CLI (`apps/figstudio-cli`)
- [ ] CLI tests (partial: usage/runtime errors and core command output checks; T1 remains open)

## P4 — Platforms
- [x] wasm (optional): browser app with threads (Web Workers + shared memory,
      `scripts/web-build.sh`); opens dropped `.fig` files; checked in headless Chromium
      (crossOriginIsolated, N workers, renders fixtures and a stress file). Not checked:
      Firefox / Safari, the file picker dialog, export download in a real browser

## Progress

### 2026-10-09T000000Z b015-rust-audit-fixes-f1-f11-see
- 2026-10-09 — Rust audit fixes (F1–F11, see `target/rustaudit/report.md`, not tracked).
  Loader rejects parent cycles (iterative, all nodes), `page_of` and `descendant_count` bounded.
  Decompression capped per item (512 MiB) and per archive (1 GiB) with `take(limit+1)`. Save
  uses a unique temp file and removes it on failure. Empty images kept; malformed blobs error.
  Integer writes checked (u8/i32/u32). Render has a shared 2,000,000-attempt work budget
  across bounded bounds preparation and traversal; ordinary recursion and hit testing are guarded,
  and omitted work is reported in `Stats`. This leaves >160x headroom over the measured real-file
  page render (12,418 work units from a prior measurement; final attempted-work telemetry not
  collected directly). Image cache is bounded by 256 MiB and 256 successful entries;
  conflicting bytes under one image key invalidate cache hits. App exports admit one in-flight job
  and release admission on completion/failure/unwind; byte-backed drops share open deduplication.
  `scripts/check.sh` passed (fmt, clippy, tests, release build); focused package tests and CLI
  privacy/output tests passed. The review follow-up is recorded in `target/auditfix/fix2-report.md`.
  Not done: live GUI check, Pillow thumb sheet, pixel-parity review, and RSS measurement.

### 2026-10-09T000000Z b016-pony-audit-l1-privacy-trust-doctor
- 2026-10-09 — Pony audit L1 privacy/trust: doctor now bounds its input reads and keeps archive
  paths/schema prelude out of output; Kiwi list lengths and schema interning are bounded, and
  ID-less node records are rejected rather than dropped. L2: iterative layer/CLI walks, descendant
  fit bounds, retained source-byte cache accounting, indexed override lookup, unsupported draw
  telemetry and clip/mask pixel checks. Thumb sheet skipped (Pillow unavailable). Both lanes passed
  `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh`; L3 synth IDs, render-worker unavailable state,
  wrapper/test cleanup, decoded-item and schema limits, and doctor prelude privacy passed the gate.
  Private-file smoke passed info/doctor/render/thumbnail/resave/reopen with the input unchanged.
  Pony audit follow-up: all 15 non-false-positive findings fixed; see `target/ponyfix/report.md`.

### 2026-10-09T000000Z b017-cli-validation-unknown-commands-now-fail
- 2026-10-09 — CLI validation: unknown commands now fail as usage errors before file access;
  black-box tests check usage/runtime errors and core command outputs. PNG outputs are decoded
  and dimension-checked; doctor missing-file usage and resave confirmation are asserted. CLI,
  doctor privacy and `scripts/check.sh` pass; T1 remains open.

### 2026-10-09T000000Z b026-2026-10-09-13-xx-browser
- 2026-10-09 13:xx — Browser build with threads ("method A"): `wasm-bindgen-rayon` pool,
  render loop and decoding on Web Workers, UI thread never blocks. Renderer records the walk
  once and replays it per strip on the pool (`render_parallel`, exact stats; pixels within 1).
  Vello's own multithreading cannot run in wasm (it spawns std threads). 100% zoom 15 → 6 ms
  in Chromium; fit zoom unchanged (walk-bound, T15).

### 2026-10-09T000000Z b027-2026-10-09-14-xx-requester
- 2026-10-09 14:xx — Requester's 1.7 GB file ran out of memory in the browser. Lazy archive
  reading + per-level image cache + images read on demand (see P1b). Opens in 1.4 s at
  537 MB Wasm memory, all pages render, peak 906 MB; lazy matches whole pixel for pixel.
  Found and fixed: frames depended on the decode order of shared images.

### 2026-10-09T000000Z b028-2026-10-09-14-5x-requester
- 2026-10-09 14:5x — Requester's browser showed CJK page and layer names as boxes. The
  browser app now fetches a 5 MB Noto Sans CJK HK subset (`scripts/web-font.sh`) only when
  a file has CJK names or text; desktop fallback list gained macOS / Windows fonts
  (PingFang HK, Heiti TC, Hiragino Sans GB, Arial Unicode MS, Microsoft YaHei; not checked
  on a Mac). Checked in headless Chromium: Latin file fetches nothing, CJK file renders.

### 2026-10-09T000000Z b029-correct-instance-override-paths-through-ordinary
- 2026-10-09 — Correct instance override paths through ordinary frames/groups. Synthetic
  nested-instance pixel regression fails without the fix; private visual comparison improves
  state selection and chart layout. Full workspace gate recorded in the local render report.

### 2026-10-09T000000Z b030-added-scoped-component-properties-exact-version
- 2026-10-09 — Added scoped component properties, exact-version paint-variable alias resolution,
  and omitted derived translations for scaled/resized instances. Synthetic pixel regressions
  proved red/green; visual comparison restores table values and improves radial leaders.
  Reference still differs in some component states and right-panel structure; not claimed exact.

### 2026-10-09T000000Z b031-strengthened-stale-property-text-regression-with
- 2026-10-09 — Strengthened stale-property-text regression with a stored old glyph so it proves
  stale outlines are rejected rather than only proving an empty text node remains empty.

### 2026-10-09T000000Z b032-honor-stackreversezindex-when-painting-siblings-restorin
- 2026-10-09 — Honor `stackReverseZIndex` when painting siblings, restoring the second chart
  formerly covered by a history list. Synthetic overlapping-instance pixel regression proved
  red/green; private visual comparison confirms chart content. State-colour investigation open.

### 2026-10-09T000000Z b033-2026-10-09-20-xx-merged
- 2026-10-09 20:xx — Merged `fix/c1-rendering` into `web-threads`. Its byte-bounded image cache
  is replaced by the per-level cache (`render::images`), which now keys entries by document
  (`Document::instance`) so equal hashes with different bytes never alias; its cache tests
  are rewritten against it. Work budget kept on both the serial and parallel paths; masks
  use `push_clip_path`, now recorded and replayed per strip (pixel test against serial).
  Lazy archive reads keep the decoder's size limits. Desktop exports run in the background
  and read lazy images. Big file: lazy matches fully read pixel for pixel on every page at
  fit and 8x; in headless Chromium it opens in 1.5 s, all pages draw, peak 869 MB.
