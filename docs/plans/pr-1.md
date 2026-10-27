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
