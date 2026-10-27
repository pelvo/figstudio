- 2026-10-12 — T20 L2: feature `gpu` of figstudio-render replays a recorded tile into a
  `vello_gpu` scene (`Recording::gpu_scene`) after a scan that refuses image paints, angular
  gradients, other filter graphs, huge filter reach and >10k layers (`GpuUnsupported`, counted as
  `Stats::gpu_fallback_tiles` once L3 wires the app). GPU vs CPU tiles on Metal: ≤ 2/255 (filters ≤ 5/255).
  Not verified: Vulkan/DX12/GL/WebGPU/WebGL2 pixels, GPU-side panics beyond the scanned cases.
