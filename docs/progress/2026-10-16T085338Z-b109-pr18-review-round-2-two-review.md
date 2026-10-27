- 2026-10-16 — PR18 review round 2: two review gaps closed, verified together. (1) The
  old-socket auth bypass: a hibernated socket stored by an earlier deploy (`{order}` or
  `{ready,order}`, accepted when the room alone admitted the bridge) could be
  re-promoted by a re-sent `ready`. Attachments now carry an
  `authenticatedProtocol:"figstudio-exec-v1"` marker written only by a proved
  `connect`; the constructor closes every restored socket without the exact marker
  (`1008`), every frame requires it before processing (pending calls fail, never
  promote), `activeTab` needs marker AND `ready`, and `ready` only flips readiness —
  client-claimed auth fields are ignored. The unused `executorFromInput` JS copy (it
  disagreed with the Rust parser on duplicated fragment keys and had no production
  callers) is deleted. (2) The keepalive was send-only: a black-holed relay socket kept
  its "connected" status forever. New `mcp_heartbeat.rs` state machine (pure,
  synthetic-clock tested): pings every 30 s counted from the last pong, a 10 s
  `PONG_DEADLINE` drops the connection ("Cloud relay stopped responding"; a refused
  send drops it immediately), pongs are generation-tagged so a stale socket's answer
  never clears a new connection's wait, events drain before the deadline tick so a
  throttled background tab can only lengthen the deadline, and expiry retries at most
  once per connection that had proven itself (storm bound). The keepalive row in
  `docs/BROWSER-WASM-MCP-PLAN.md` now states the pong-timeout semantics; the local
  bridge path is untouched. Each fix seen failing first: worker red 6 failures on the
  old worker (round2-red.log), heartbeat red 8/8 on stub bodies, and the new e2e
  "keepalives go unanswered is closed by the page" red against the previous build
  (round2-e2e-red.log). Mutations killed and reverted (round2-mutations.log): M1
  constructor close, M2 frame guard, M3 activeTab check, M4 the heartbeat deadline
  branch. Gates: `node --test web/worker/` 32 pass; `cargo +1.99.0 test -p figstudio
  mcp` 61 pass; `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh --web` all checks passed
  (round2-fullgate.log; CJK fonts present, no opt-out); Chromium e2e 5/5 against
  `wrangler@4 dev` (workerd) on :8795 including the blackhole test
  (round2-e2e-green.log). Not verified: a live deploy, Access policies, real
  Cloudflare eviction and hibernation restore in production.
