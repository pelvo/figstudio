---
pr: 7
branch: feat/vello-hybrid
title: feat(app,render,web): smooth tiled canvas and vello_gpu rendering
---

## Progress

### 2026-10-11T000000Z b060-t20-l1-app-runs-eframe-on
- 2026-10-11 — T20 L1: app runs eframe on wgpu (desktop + web) instead of glow; window screenshot
  on macOS matches the glow build; web build and `scripts/check.sh` passed. Not verified: Linux,
  Windows, a browser run (WebGPU or WebGL2).

### 2026-10-11T000000Z b061-t20-l1-fix-request-only-the
- 2026-10-11 — T20 L1 fix: request only the GPU limits the adapter has (WebGL2 with a 4096 texture
  limit started failing on wgpu); unit test seen failing first; headless Chrome with forced WebGL2/4096
  opens a fixture. Not verified: a real 4096-limit device, Linux, Windows.

### 2026-10-11T000000Z b062-t20-l1-fix-2-keep-the
- 2026-10-11 — T20 L1 fix 2: keep the GPU surface within the device texture limit (browser canvas
  cap, desktop max window size) and request the adapter's full texture size; unit tests seen failing;
  forced WebGL2/4096 resize to 5000 px no longer panics. Not verified: desktop cap on a large screen.

### 2026-10-12T093232Z b009-pr-7-review-linux-amd-wgpu
- 2026-10-12 — PR #7 review (Linux, AMD): wgpu uncaptured errors are logged and turn the GPU
  tiles off instead of panicking (desktop and web); software adapters (`DeviceType::Cpu`) use the
  CPU canvas unless the GPU is asked for; GPU.md records the Linux results. Tests seen failing first.

### 2026-10-12T093232Z b010-t20-l5-docs-for-the-gpu
- 2026-10-12 — T20 L5: docs for the GPU canvas (IMPLEMENTATION desktop + browser, GPU.md open
  gaps). L1–L4 and P1–P3 reviewed until clean; branch feat/vello-hybrid.

### 2026-10-12T093232Z b011-t20-l1-wgpu-reviewed-three-times
- 2026-10-12 — T20 L1 (wgpu) reviewed three times; GPUs with a texture limit below the
  window size can still crash on some resize/fullscreen paths. Accepted and documented in
  `docs/GPU.md` ("Known limit"); the L3/L4 CPU fallback is the planned answer.

### 2026-10-12T093232Z b063-t20-p1-the-canvas-renders-a
- 2026-10-12 — T20 P1: the canvas renders a margin (half the canvas per side, capped by texture
  limit, u16 and 32 Mpx) and requests a new frame only near the margin's edge or on zoom/size/page
  change; 6 unit tests seen failing under 5 mutations; scripted pan showed no empty edge and one
  request per ~320 pt of pan. Not verified: real hand-tool drag (no input tool), web by eye.

### 2026-10-12T093232Z b064-t20-p1-fix-overscan-frames-stay
- 2026-10-12 — T20 P1 fix: overscan frames stay within the pixel cap (32 Mpx native, 16 Mpx web)
  and the texture limit by rendering an over-cap canvas at lower resolution; one planner decides
  requests, so an over-limit canvas no longer re-requests every update; web-build.sh exits 0
  without the CJK font. Tests seen failing under mutations. Not verified: over-limit canvas by eye.

### 2026-10-12T093232Z b065-t20-p2-the-canvas-is-drawn
- 2026-10-12 — T20 P2: the canvas is drawn from cached 512 px tiles per zoom level (LRU, 256 MiB
  native / 128 MiB web) with coarser/finer fallback under missing tiles and centre-first requests;
  overscan.rs removed; 12 unit tests seen failing under mutations; scripted pan/zoom logged no
  uncovered tile during pans. Not verified: real hand-tool drag, web by eye, large file by eye.

### 2026-10-12T093232Z b066-t20-p2-fix-full-coverage-of
- 2026-10-12 — T20 P2 fix: full coverage of views over half the cache (coarser level, on-screen
  tiles protected), evicted tiles re-requested (worker skips only unacknowledged tiles), 2-px
  tile gutters (seam 37/255 → ≤3/255 in a CPU model of the GPU's sampling), tab close cancels its
  batch and closed documents store no images. Tests seen failing first. Not verified: GPU seams.

### 2026-10-12T093232Z b067-t20-p2-fix-2-planned-tiles
- 2026-10-12 — T20 P2 fix 2: planned tiles evict drawn fallback tiles (covered ones first), so a
  cache full of fallback settles instead of re-rendering; closed documents are tombstoned in the
  image cache and checked under its lock after decode. Tests seen failing first.

### 2026-10-12T093232Z b068-t20-p3-tiles-of-one-page
- 2026-10-12 — T20 P3: tiles of one page and zoom level replay one recording of the page (walked
  once per batch area); over-limit or budget-cut recordings fall back to direct renders, counted as
  "tiles not recorded". Tests seen failing first. Not verified: scripted window pan, wasm timing.

### 2026-10-12T093232Z b069-t20-l2-feature-gpu-of-figstudio
- 2026-10-12 — T20 L2: feature `gpu` of figstudio-render replays a recorded tile into a
  `vello_gpu` scene (`Recording::gpu_scene`) after a scan that refuses image paints, angular
  gradients, other filter graphs, huge filter reach and >10k layers (`GpuUnsupported`, counted as
  `Stats::gpu_fallback_tiles` once L3 wires the app). GPU vs CPU tiles on Metal: ≤ 2/255 (filters ≤ 5/255).
  Not verified: Vulkan/DX12/GL/WebGPU/WebGL2 pixels, GPU-side panics beyond the scanned cases.

### 2026-10-12T093232Z b070-t20-l2-fix-1-the-gpu
- 2026-10-12 — T20 L2 fix 1: the GPU scan sizes nested filter layers as vello_gpu does (each filter
  grows the intermediate viewport), refusing tiles that would fail with `TooLarge`;
  `gpu_scene` takes the texture limit. Test seen failing first. Not verified: non-Metal GPUs.

### 2026-10-12T093232Z b071-t20-l3-the-desktop-app-draws
- 2026-10-12 — T20 L3: the desktop app draws canvas tiles with vello_gpu on eframe's wgpu device
  (render thread renders, UI registers), falling back to counted CPU tiles; FIGSTUDIO_RENDERER and
  a status-bar renderer label. Not verified: Vulkan/DX12/GL pixels, device loss, a sub-4096 GPU.

### 2026-10-12T093232Z b072-t20-l4-the-browser-draws-canvas
- 2026-10-12 — T20 L4: the browser draws canvas tiles with vello_gpu (WebGPU, else WebGL2 through
  wgpu's GL backend, else CPU): the worker builds scenes, the main thread renders ≤ 4 per frame;
  `?renderer=cpu|gpu`. Headless Chrome, structure.fig: both GPU paths within 1/255 of the CPU.
  Not verified: a real non-Apple GPU, device loss, a late wgpu error scope firing, Firefox/Safari.
