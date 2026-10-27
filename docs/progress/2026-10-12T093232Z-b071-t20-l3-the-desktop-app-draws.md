- 2026-10-12 — T20 L3: the desktop app draws canvas tiles with vello_gpu on eframe's wgpu device
  (render thread renders, UI registers), falling back to counted CPU tiles; FIGSTUDIO_RENDERER and
  a status-bar renderer label. Not verified: Vulkan/DX12/GL pixels, device loss, a sub-4096 GPU.
