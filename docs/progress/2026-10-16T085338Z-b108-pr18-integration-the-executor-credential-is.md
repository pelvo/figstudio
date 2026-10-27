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
