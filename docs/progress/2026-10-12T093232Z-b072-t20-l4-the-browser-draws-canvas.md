- 2026-10-12 — T20 L4: the browser draws canvas tiles with vello_gpu (WebGPU, else WebGL2 through
  wgpu's GL backend, else CPU): the worker builds scenes, the main thread renders ≤ 4 per frame;
  `?renderer=cpu|gpu`. Headless Chrome, structure.fig: both GPU paths within 1/255 of the CPU.
  Not verified: a real non-Apple GPU, device loss, a late wgpu error scope firing, Firefox/Safari.
