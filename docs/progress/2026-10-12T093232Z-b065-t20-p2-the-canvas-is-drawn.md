- 2026-10-12 — T20 P2: the canvas is drawn from cached 512 px tiles per zoom level (LRU, 256 MiB
  native / 128 MiB web) with coarser/finer fallback under missing tiles and centre-first requests;
  overscan.rs removed; 12 unit tests seen failing under mutations; scripted pan/zoom logged no
  uncovered tile during pans. Not verified: real hand-tool drag, web by eye, large file by eye.
