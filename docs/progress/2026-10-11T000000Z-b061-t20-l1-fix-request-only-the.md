- 2026-10-11 — T20 L1 fix: request only the GPU limits the adapter has (WebGL2 with a 4096 texture
  limit started failing on wgpu); unit test seen failing first; headless Chrome with forced WebGL2/4096
  opens a fixture. Not verified: a real 4096-limit device, Linux, Windows.
