- 2026-10-15 — PR18 threads-only relay fixes: the worker accepts the 202/null
  notification frame `rpc_reply` builds (was a 502 "Invalid browser reply"; the new
  worker test round-trips it, seen failing 502 !== 202, and pins that only 202 may have
  a null body), and the encode-failure fallback now carries the caller's id
  (`encode_failed_reply`; the old inline frame sent `id: null`, which the worker's
  `body.id` match can never accept). A reveal whose worker already expired is no longer
  applied by a late UI frame: the UI must win an atomic claim (pending -> claimed)
  before `apply_reveal` mutates, and the worker expires only unclaimed reveals — a
  claimed one reports its real outcome even past the deadline (native test seen failing
  `Some(Guid(1,25)) != None` before the fix). The tab bounds in-flight relay jobs at 8
  and answers the rest with the worker's own busy shape (503; native test seen failing
  `9 != 8` replies), so a burst or 30 s-timeout retries cannot stack renders on the
  shared Rayon pool; stopping a render whose call already 504'd remains open
  architecture (needs a cancel path the render loop checks, worker- or tab-initiated).
  The relay tab now sends a 30 s keepalive `ping` driven by the repaint cadence (the
  DO auto-response only ever answered pings nothing sent; plan doc corrected), and the
  worker comment no longer claims the 30 s deadline bounds eviction-dropped requests —
  a hard eviction cancels the timers with the isolate, which a new Node test pins
  (revived isolate ignores the late reply; a fresh call still works). Focused checks:
  `node --test web/worker/` 22 pass; `cargo +1.99.0 test -p figstudio` 164 pass;
  clippy -D warnings (release, default and no-default-features), fmt and a wasm32
  check of the touched app code pass (verified in an isolated worktree at 3084f07 plus
  these changes, because executor-auth work was landing concurrently in the shared
  tree). Not verified: the full `scripts/check.sh --web` gate, the browser e2e (pings
  are already tolerated by the spec), real Cloudflare eviction/keepalive behavior.
