- 2026-10-16 — PR18 review round 3: relay loss always requests the next UI frame,
  including unproven expiry, so an idle canvas removes Connected and displays the
  disconnect notice. Refused ping sends retain the heartbeat's proof bit and retry
  once; the replacement starts unproven and cannot retry itself. The unmarked-socket
  frame guard tolerates a throwing close after failing pending calls, without granting
  readiness or authentication. Each regression failed without its own fix. Chromium
  observes a real draw in a later animation frame after self-close (not just an idle
  animation callback), and a real pong followed by injected send refusals produces
  exactly two sockets and two closes. Screenshot inspected: disconnect notice visible,
  Connected absent. The backward-clock panic claim is false: web-time 1.1.0's Wasm
  `duration_since` already uses `checked_duration_since(...).unwrap_or_default()`;
  native web-time re-exports the saturating std implementation. No clock change.
  Gates: Worker 32/32, heartbeat 8/8, foreground
  `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh --web` all checks passed with CJK fonts,
  final-build Chromium 6/6 against local workerd; fixture thumbnail sheet inspected.
  Not verified: production hibernation, real NAT expiry or background throttling,
  non-Chromium browsers, deployment or Access policies. No new dependencies.
