---
pr: 39
branch: feat/web-relay-probe
title: feat(app,web): hide cloud relay on a static-only deploy
---

## Progress

### 2026-10-20T094718Z b186-static-only-web-deploy-the-page
- 2026-10-20 — Static-only web deploy: the page asks `GET /mcp/` once per load and offers or connects Cloud relay only when the relay Worker answers (`204` + `figstudio-relay`); elsewhere Settings → Agents shows Local bridge with a hint and no relay socket opens. The Worker no longer exports a string (workerd refused to start on it); a test now checks every named export. Checked in headless Chromium against `scripts/web-serve.py` (probe 404, no socket, Cloud relay hidden), the same with the previous build (socket to `/bridge/rm_…` that fails), and `wrangler dev` 4.151.2 (probe 204, socket stays open, "Connected to cloud relay"). Rust and Worker tests each failed under their mutation (header ignored, offered always, header renamed, probe route removed, string export). Not checked: a real Cloudflare deploy, behind Cloudflare Access, Safari or Firefox.

### 2026-10-20T094718Z b187-pr-39-review-comment-6286329876-a
- 2026-10-20 — PR #39 review (comment 6286329876): a Cloud relay connect now waits in `CloudGate` (`mcp_wire.rs`, pure) until the probe answers, so Connect, Join, Apply or Regenerate pressed before the answer opens the socket once it arrives. On an origin without the Worker it leaves Cloud relay with a toast and never falls through to the upload bridge; the page load's own connect leaves quietly there. A failed probe (thrown fetch, 5 s timeout, 5xx, 429) is `Unreachable`, not sticky: Cloud relay stays offered, one toast, and the next Connect asks again. Only `figstudio-relay: 1` counts. The probe's answer applies at frame start, so Settings → Agents never draws cloud controls for an origin without the relay. Eight mutations of the gate and probe classification each fail a test. Headless Chromium: `web-serve.py` (404, no socket, no toast, Local bridge + hint), `wrangler dev` (204, socket), a probe delayed 3 s (socket after it), aborted (toast, no socket; Connect re-probes and connects), hung (toast after the timeout, no socket). Not checked: a real Cloudflare deploy, behind Cloudflare Access, Safari or Firefox.

### 2026-10-20T094718Z b188-pr-39-re-review-comment-6287142121
- 2026-10-20 — PR #39 re-review (comment 6287142121, P1 0 · P2 0 · P3 2): frame start now only applies an `Absent` answer (`CloudGate::before_menu`); a waiting Cloud relay connect is released in `sync`, after the menu, so switching to Local bridge in that frame cancels it instead of opening the upload bridge. The probe clears its timer and aborts a request the 5 s deadline beat (an answered one is left to finish), so Connects on a black-holed server do not pile up open GETs. The new gate test fails when `before_menu` releases the connect. Headless Chromium: static 404 (no abort, no socket), `wrangler dev` (socket), a hung probe aborted at the deadline, then Connect probes again and connects. The one-frame mode switch was not reproduced in a browser.
