- 2026-10-12 — T20 L1 (wgpu) reviewed three times; GPUs with a texture limit below the
  window size can still crash on some resize/fullscreen paths. Accepted and documented in
  `docs/GPU.md` ("Known limit"); the L3/L4 CPU fallback is the planned answer.
