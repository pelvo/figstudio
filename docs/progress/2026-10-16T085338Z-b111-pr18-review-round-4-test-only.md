- 2026-10-16 — PR18 review round 4 (test-only): the blackhole spec's init script wraps
  `WebSocket.close` before the paint hooks and skips a missing `GPUQueue` or WebGL2
  interface, so a browser without WebGPU still records the close instead of timing out;
  it also asserts at least one draw hook was installed. Chromium with `GPUQueue` deleted:
  old script left `atClose` unset, new one records it. The rAF-id finding is false: the
  expression-bodied arrow already returns the native id, and Chromium cancels the
  callback through it. Final-build Chromium 6/6 against local workerd. No product change.
