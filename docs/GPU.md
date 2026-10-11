# GPU rendering with vello_gpu (task T20)

Goal: the desktop and browser app draw the canvas on the GPU with `vello_gpu` (the crate
formerly called `vello_hybrid`), falling back to the current CPU renderer when no suitable
GPU is available. The CLI, the tests and PNG export stay on `vello_cpu`, so their output
stays reproducible.

## Facts this plan rests on (checked 2026-10-11)

- `vello_hybrid` was renamed `vello_gpu`. `vello_gpu 0.3.0` is on crates.io and matches our
  `vello_cpu 0.3.0` / `vello_common 0.3.0`; it uses `wgpu 30`, the same as eframe 0.36.
  So no git dependency is needed (the published `vello_hybrid 0.2` was the mismatched one).
- `vello_gpu` features: `wgpu` (Metal, Vulkan, DX12, WebGPU) and `webgl` (WebGL2 with GLSL).
- `vello_gpu` panics on mask layers, complex filter graphs and some non-isolated blend
  modes. Our renderer uses clip, opacity and simple filter layers (`GaussianBlur`,
  `DropShadow`, `DropShadowOnly`) and no mask or blend layers, but every call must be
  checked, and anything unsupported must take the CPU path, never panic (AGENTS rule 5).
- The app uses eframe with `glow` today; wgpu is not compiled. Sharing one wgpu device
  between egui and `vello_gpu` needs eframe's `wgpu` renderer.

## Design

1. `crates/render`: a `Backend` seam. Scene building stays as it is (`Ctx` drawing into the
   `Canvas` trait in `record.rs`). Add a `vello_gpu::Scene` implementation of `Canvas`
   behind a `gpu` cargo feature. `render_scene(doc, root, viewport) -> Scene` for the GPU, and
   the existing `render() -> Frame` (pixmap) for the CPU. Stats are identical: they come
   from the same walk.
2. `apps/figstudio`: eframe switches from `glow` to `wgpu` (desktop and web). A `GpuCanvas`
   owns a `vello_gpu::Renderer` on eframe's wgpu device and renders the scene into a
   wgpu texture registered with egui (`egui_wgpu::Renderer::register_native_texture`), so
   no pixels go back to the CPU. Hit testing, labels, overlays are unchanged.
3. Fallback: if wgpu adapter creation fails, or the device lacks what `vello_gpu` needs, the
   app keeps the CPU worker path (today's code) and says so once in the status bar.
   A setting/env var (`FIGSTUDIO_RENDERER=cpu|gpu`) forces either for debugging.
4. Browser: WebGPU where available, else WebGL2 through `vello_gpu`'s `webgl` feature, else
   the CPU path. The thread-based CPU workers stay for the fallback.

## Lanes (one at a time, each committed after its gate)

| # | Lane | Done when |
|---|---|---|
| L1 | eframe glow → wgpu, desktop + web, no rendering change | **Done 2026-10-11.** App runs on macOS with wgpu (Metal); web build passes; `scripts/check.sh` passes; window screenshot matches the glow build (only the status-bar ms text differs, rest ≤ 1/255) |
| L2 | `gpu` feature: `Canvas` for `vello_gpu::Scene`, headless test rendering a fixture on the GPU and comparing with the CPU pixmap | Fixtures differ from CPU by less than a small per-pixel tolerance (test skips with a message when no GPU adapter exists) |
| L3 | App draws the canvas with `vello_gpu` into an egui-registered texture, CPU fallback + `FIGSTUDIO_RENDERER` | Large real file pans/zooms on GPU; forcing `cpu` gives today's behaviour; no panics on any fixture |
| L4 | Browser: WebGPU, then WebGL2, then CPU fallback | `scripts/web-build.sh` build runs in a WebGPU browser and in one without WebGPU |
| L5 | Docs: PLAN, IMPLEMENTATION, TESTING updated; what is not verified | Docs list the GPU path, the fallback and the remaining gaps |

## L1 notes

- eframe features: `wgpu` (pulls `egui-wgpu/default`: wgpu `default` = Metal, Vulkan, DX12,
  GLES, WebGPU, plus `webgl`). Native backends are wgpu's `PRIMARY | GL` (or `WGPU_BACKEND`);
  on the web egui-wgpu tries WebGPU and falls back to WebGL2. `Renderer::Wgpu` is set
  explicitly in `NativeOptions` and `WebOptions`.
- No usable GPU adapter (no Vulkan/Metal/DX12 and no GL driver): `run_native` returns an
  error; the app prints `figstudio: could not start the window: <reason>` and exits 1. There is
  no CPU fallback for the window itself (wgpu's GL backend covers most machines; a software
  GL such as llvmpipe/Mesa also works). In the browser, `start` rejects and `index.html`
  shows "figstudio could not start". The vello CPU fallback for the canvas is L3.
  Checked on macOS with `WGPU_BACKEND=vulkan` (no such backend): `could not start the window:
  WGPU error: Failed to create surface for any enabled backend`, exit 1, no panic.
- Device limits: egui-wgpu asks for `max_texture_dimension_2d = 8192`, which a WebGL2 or weak
  GL adapter (often 4096) refuses. `apps/figstudio/src/gpu.rs` keeps egui-wgpu's other
  requested limits, asks for the adapter's full texture size, and lowers every limit to the
  adapter's (`Limits::or_worse_values_from`). Checked in headless Chrome with WebGPU hidden and
  WebGL2 `MAX_TEXTURE_SIZE` reported as 4096: the L1 build fails ("value 8192 is better than
  allowed 4096"); the fixed build opens and draws `structure.fig`.
- Surface size: eframe configures the window surface at the window's pixel size, and wgpu
  panics above the device limit (glow had no such limit). The app keeps it within:
  - browser: `web::cap_canvas` sets the canvas CSS `max-width`/`max-height` to
    `limit / devicePixelRatio` and clamps the backing store each frame (eframe 0.36
    `web/events.rs` sizes it from the ResizeObserver and rescales it on DPR change). A window
    larger than the limit leaves the canvas capped at the top left, the rest page background.
    Checked: forced WebGL2/4096, resizing 1000 → 5000 px panicked on the previous build and
    now gives a 4096 px canvas; DPR 2 caps it at 2048 CSS px; DPR change 1 → 2 at the cap
    stays live.
  - desktop: `gpu::cap_window` sends `ViewportCommand::MaxInnerSize(limit / pixels_per_point)`
    whenever the limit or scale changes (egui-winit 0.36 converts it back with the same pixels
    per point). Not reproducible on the test Mac (the screen is smaller than the limit).
- The canvas texture is the canvas area in pixels, inside the surface, so it needs no clamp
  of its own.
- Size (macOS arm64 release): app binary 10.08 MB (glow) → 15.32 MB (wgpu); `figstudio_bg.wasm`
  8.51 MB → 13.60 MB.

## Risks

- `vello_gpu` maturity: some failures panic. Every scene feature we emit must be known to be
  supported; unknown cases fall back to CPU for that frame.
- Filters on the GPU may differ slightly from the CPU (blur edges). The CPU stays the
  reference for tests and export.
- Browser WebGL2 limits (texture size, no compute) and wasm threads interplay.
- CI or machines without a GPU: GPU tests must skip, not fail.
