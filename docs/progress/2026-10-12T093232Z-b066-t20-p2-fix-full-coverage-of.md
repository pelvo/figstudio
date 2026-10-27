- 2026-10-12 — T20 P2 fix: full coverage of views over half the cache (coarser level, on-screen
  tiles protected), evicted tiles re-requested (worker skips only unacknowledged tiles), 2-px
  tile gutters (seam 37/255 → ≤3/255 in a CPU model of the GPU's sampling), tab close cancels its
  batch and closed documents store no images. Tests seen failing first. Not verified: GPU seams.
