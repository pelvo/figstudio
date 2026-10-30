---
pr: 18
branch: feat/browser-wasm-mcp-relay
title: feat(mcp,web): execute browser tools through a hibernating relay
---

## Progress

### 2026-10-14T000000Z b006-join-an-existing-mcp-room-agent
- 2026-10-14 — Join an existing MCP room: Agent access gained Copy room link and Use room
  token (bare token, MCP URL or room link; invalid input toasts and changes nothing), and
  a `#room=` fragment joins on startup and `hashchange`, consumed with
  `history.replaceState` so the token leaves the address bar. Pure helper tests with five
  mutation controls and a Playwright second-browser join test shipped; full
  `scripts/check.sh --web` gate and 3 e2e tests passed. Menu Apply itself not e2e-driven;
  Safari/Firefox and deployed hibernation unverified.
  (Superseded 2026-10-15: the fragment path now asks before joining — see the
  room-link join guard entry.)

### 2026-10-14T000000Z b007-browser-mcp-lane-1-byte-backed
- 2026-10-14 — Browser MCP Lane 1: byte-backed entry readers, SDK-free tab JSON-RPC,
  shared prompt text, native schema drift guard and no-fs inline assets shipped. Core
  tests, seven mutation controls, pinned nightly wasm32 check and Rust 1.99.0 full gate
  passed; public-fixture sheets inspected. Browser workers, UI reveal and relay unverified.

### 2026-10-14T000000Z b102-browser-wasm-mcp-lane-2-persistent
- 2026-10-14 — Browser Wasm MCP Lane 2: persistent Web-Crypto room capability, same-origin relay and local-bridge selector, copy/regenerate controls, Rayon RPC snapshots and main-thread reveal/replies; split browser modules below 300 lines. Eight helper mutations and a generated-JS persistence mutation failed as intended, then restored tests passed (17 client helpers, 17 relay contracts, 2 Chromium e2e). `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh --web` ended `all checks passed`; final-build e2e passed against local Wrangler. Not verified: deployment/hibernation, non-Chromium browsers, real local upload after the split, large-file lazy images or disconnect during RPC. Optional CJK web fonts unavailable.

### 2026-10-14T000000Z b103-browser-relay-review-fixes-b1-b3
- 2026-10-14 — Browser relay review fixes (B1–B3, S1, S3, S5, S7, S9, S10 + dep nit): oversized replies become an `isError` result under a 7.5 MiB tab budget and the worker fails only that call (not the socket); `reveal_node` waits for the UI ack (polled with a `web_time` deadline because std `Instant` panics on wasm32 — caught by the e2e) and a UI error/timeout fails the call; `notifications/*` requests with an id return -32601; the newest-ready-tab routing is documented and pinned (menu text, Node tests; the 16-tab cap already counted open sockets); plan doc updated (native-parity tools, JSON `body`, `ready` without tab title, Cloudflare Access coverage of `/mcp/*`+`/bridge/*`, eviction drops in-flight calls); `rpc_reply` status is never optional; stalled-peer bridge test renamed to what it proves; `getrandom` is out of native release builds and `hex` is dropped. Every new test seen failing under a mutation, then restored. `node --test web/worker/` 20 pass; Chromium e2e passed 3× after the rebuild (one earlier run served a stale cached wasm); `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh --web` ended `all checks passed`. Not verified: deployment/real hibernation or Cloudflare Access, non-Chromium browsers, the reveal 2 s timeout in a real browser (native test only).

### 2026-10-15T000000Z b104-room-link-join-guard-4762d52-findings
- 2026-10-15 — Room-link join guard (4762d52 findings): a `#room=` link at startup or `hashchange` no longer saves/connects silently — it stages a `RoomProposal` (`mcp_join.rs`) behind a confirmation dialog (masked room, says it grants access to open documents; Stay is the Enter/Escape default, Join takes a click or Tab+Enter); declining changes nothing, the sensitive fragment key is stripped on staging even when a changed `#mcp=` pairing wins, an unchanged `#mcp=` no longer discards the link, same-room links ask nothing, and the `Use room token` field is password-masked. Pre-fix build failed the new e2e (silent join: stored room + bridge socket with no consent); five mutations (masking, precedence, strip, Enter-default, decline-joins) failed focused tests before restore; kittest covers the dialog keyboard/masking. `node --test web/worker/` 20 pass; all 3 Chromium e2e pass on the final build (Wrangler 8791); `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh --web` ended `all checks passed` (one flaky bridge_cli run passed on rerun and standalone). Not verified: the masked field's dots in a real screenshot, a same-room link in e2e (native test only), Safari/Firefox.

### 2026-10-15T000000Z b106-cjk-font-deploy-fix-a-rebuild
- 2026-10-15 — CJK font deploy fix: a rebuild on a machine with the Noto TTC but no
  fontTools made `scripts/web-font.sh` warn and exit 0, `web-build.sh` had already
  cleared `dist/fonts`, and the deploy served `/fonts/cjk-jp.otf` as 404 (verified with
  an authenticated production fetch; not an SPA HTML fallback), so CJK names showed as
  boxes. The script now fails on missing inputs by default and only
  `FIGSTUDIO_ALLOW_MISSING_CJK=1` opts out for a local dev build; new test
  `apps/figstudio-cli/tests/web-font.rs` (missing input seen exiting 0 before the fix)
  covers failure, the opt-out and the success path (five OTTO faces, ideographs in all,
  kana in JP, Hangul only in KR, OFL license). fontTools provisioned once via
  `uv tool install fonttools`. Verified in a real (headless Chromium) browser against a
  served dist: `/fonts/cjk-jp.otf` answers 200 `font/otf` with the exact dist bytes, the
  browser decodes it (`FontFace.load`) and rasterizes あ through that face. Not verified:
  the deploy itself (owner decides; the production 404 stands until a redeploy from a
  font-ready machine), and the app's own lazy fetch when a CJK-named file is open
  (unchanged code).

### 2026-10-15T000000Z b107-pr18-threads-only-relay-fixes-the
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

### 2026-10-16T085338Z b108-pr18-integration-the-executor-credential-is
- 2026-10-16 — PR18 integration: the executor credential is wired into the Worker. The
  outer Worker now routes `/bridge/*` through `checkBridgeUpgrade`
  (`web/executor-auth.mjs`) before `idFromName`/`get`, so a caller that only knows the
  room (the MCP URL) can no longer create or reach a Durable Object through the bridge;
  the Durable Object re-verifies the same proof itself (one SHA-256; it never trusts a
  header claiming the outer Worker already checked, which would be forgeable) and its
  101 echoes only the constant `figstudio-exec-v1` marker as the selected subprotocol —
  never the offered executor hex. A socket is no longer the executor on connect:
  attachments start `{ready:false, order}` and only the `ready` frame stores
  `{ready:true, order}` (a restored hibernated attachment without `ready` — including
  one written by an earlier deploy — is ineligible), so an admitted-but-silent client
  cannot take the executor role. The forwarded 202/null notification ack now returns the
  same body-less 202 as the direct path (was a literal JSON `null` body). New gates,
  each seen failing before its fix: worker-level proof-before-DO (403 with no
  `idFromName` for missing proof, forged Origin, marker-only, or a mismatched secret),
  DO-level re-verification, ready-gating (503 before `ready`, restore-ineligible), and
  a Chromium e2e proving the caller room alone cannot open the bridge while the
  two-subprotocol proof can (`web/e2e/relay.spec.mjs`; the e2e also needed two spec
  fixes: `expect.poll(...).toBeTruthy()` resolves with undefined, so values are re-read
  after the poll, and the post-join socket assertion became a poll — the join saves
  before the reconnect frame). `sha2` 0.10 (RustCrypto) is the new dependency for the
  in-app room derivation: trusted, maintained, synchronous and wasm-safe where a
  hand-rolled SHA-256 would be review burden (ground rule 12). Residual, reported not
  solved: the proof stops caller impersonation, but anyone can still POST to well-formed
  `/mcp/<rm_…>` names or mint their own pairs and spend Worker requests and Durable
  Object invocations; a public deployment needs Cloudflare Access over `/mcp/*` and
  `/bridge/*` and/or rate limits (documented in `docs/BROWSER-WASM-MCP-PLAN.md`, no
  false safe-public claim), and an in-flight render whose call already 504'd still runs
  to completion (cancellation remains open architecture). Gates: focused
  `cargo +1.99.0 test -p figstudio` 168 pass; `node --test web/worker/` 29 pass;
  `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh --web` all checks passed (CJK fonts
  present, no opt-out); Chromium e2e 4/4 against `wrangler@4 dev` (workerd) on :8793.
  Not verified: a live deploy, Access policies, real Cloudflare eviction.

### 2026-10-16T085338Z b109-pr18-review-round-2-two-review
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

### 2026-10-16T085338Z b110-pr18-review-round-3-relay-loss
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

### 2026-10-16T085338Z b111-pr18-review-round-4-test-only
- 2026-10-16 — PR18 review round 4 (test-only): the blackhole spec's init script wraps
  `WebSocket.close` before the paint hooks and skips a missing `GPUQueue` or WebGL2
  interface, so a browser without WebGPU still records the close instead of timing out;
  it also asserts at least one draw hook was installed. Chromium with `GPUQueue` deleted:
  old script left `atClose` unset, new one records it. The rAF-id finding is false: the
  expression-bodied arrow already returns the native id, and Chromium cancels the
  callback through it. Final-build Chromium 6/6 against local workerd. No product change.
