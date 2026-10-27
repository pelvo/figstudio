---
pr: 16
branch: feat/web-mcp-bridge
title: feat(mcp,app,web): drive the browser app from MCP via a local bridge
---

## P7 — Offline MCP server (`apps/figstudio-mcp`, plan in `docs/MCP-PLAN.md`)
- [x] Web app + MCP bridge (`docs/WEB-MCP-PLAN.md`, L1–L4): `figstudio-mcp --bridge`, the browser tab pairs over a
      token-guarded WebSocket and uploads its files; automated end-to-end check `tests/bridge_cli.rs`
- [x] Remote agents (WEB-MCP-PLAN L5): `--http=ADDR` non-loopback needs a bearer token + Host allowlist, app "Allow remote agents"
- [ ] Web bridge and remote agents, open: a real Tailscale/SSH-tunnel session; manual runs in a real browser (local :8765, a deployed site in Chrome and Safari)

## Progress

### 2026-10-13T000000Z b093-web-mcp-bridge-l1-l4-protocol
- 2026-10-13 — Web MCP bridge L1–L4: protocol in mcp-core, `figstudio-mcp --bridge` (Origin + token checked
  WebSocket, uploads to a temp dir), wasm client in the app (View → Agent access, `#mcp=` auto-connect),
  README section. Tests: protocol, bridge, shared view state, pure client helpers, and a binary-level
  end-to-end (`bridge_cli.rs`), each seen failing under a mutation. check.sh --web passed. Not verified: a
  real browser session (see WEB-MCP-PLAN.md L4 TODOs).

### 2026-10-13T000000Z b094-web-mcp-l5-opt-in-remote
- 2026-10-13 — Web MCP L5: opt-in remote agents (bearer token via `subtle`, Host allowlist, wildcard refused without a flag, app checkbox, README Tailscale + SSH `-R` tunnel). `tests/remote.rs` and unit tests seen failing under mutations; check.sh passed. Not verified: a session from another machine.

### 2026-10-13T000000Z b095-web-mcp-review-fixes-constant-time
- 2026-10-13 — Web MCP review fixes: constant-time pairing-token compare, `error {tab}` names the failed upload (the tab drops it by id), 1 s bound on bridge sends so a stalled peer cannot block re-pairing; tests for hello timeout, error tab, stalled peer, remote `--bridge` bearer, each seen failing under a mutation. Not verified: a real browser session.

### 2026-10-13T000000Z b096-pr-16-f1-browser-uploads-copy
- 2026-10-13 — PR #16 F1: browser uploads copy each slice into a non-shared buffer before `WebSocket.send` (threads build). Wasm-only, no native test; check.sh and check.sh --web passed. Not verified: an upload in a real browser (reviewer confirmed the fix on the reviewed build).

### 2026-10-13T000000Z b097-pr-16-f2-uploads-are-bound
- 2026-10-13 — PR #16 F2: uploads are bound to the tab id, not the file name, so two picked files with one name each stream their own bytes (`mcp_wire::Uploads`; test `two_files_with_one_name_stay_bound_to_their_own_tabs`, seen failing with a name-keyed mutation). check.sh and check.sh --web passed. Not verified: picking two same-name files in a real browser.

### 2026-10-13T000000Z b098-pr-16-f3-a-remote-bridge
- 2026-10-13 — PR #16 F3: a remote `--bridge --http=<addr>` also listens on `127.0.0.1:<port>` for `/bridge` (and loopback `/mcp`), so the browser on this PC pairs; the banner prints `Browser bridge: ws://127.0.0.1:<port>/bridge` once it is listening. Test `a_remote_bridge_still_pairs_the_browser_on_loopback` (skips with no LAN address); seen failing with the loopback branch disabled. README Remote agents updated. check.sh passed. Not verified: a browser on a remote-bound bridge.

### 2026-10-13T000000Z b099-pr-16-f4-a-wildcard-bind
- 2026-10-13 — PR #16 F4: a wildcard bind (`0.0.0.0`/`[::]`) with no `--allow-host` prints a hint that clients must send a name or IP passed with `--allow-host` (pure `access::wildcard_hint`, test `a_wildcard_bind_hints_at_allow_host_only_when_none_is_given`, seen failing when disabled); test `a_wildcard_bind_refuses_an_unnamed_lan_host_even_with_the_token` (403 for an unnamed LAN Host, 200 for the loopback one). No interface enumeration (no new crate). README Remote agents updated. Not verified: a LAN client in a real network.

### 2026-10-13T000000Z b100-pr-16-f5-a-socket-that
- 2026-10-13 — PR #16 F5: a socket that never says `welcome` gives up after 8 s (`CONNECT_TIMEOUT_MS`, stale timers ignored by generation) and the 1006 close shows the same message: it names local network access and that Safari blocks https -> ws:// (`mcp_wire::connect_failed_message`; test `a_failed_connect_names_local_network_access_and_the_https_limit`, seen failing with the hint removed). check.sh and check.sh --web passed. Not verified: the timeout and the 1006 message in Chrome (Local Network Access) or Safari (mixed content).

### 2026-10-13T000000Z b101-pr-16-f6-upload-errors-show
- 2026-10-13 — PR #16 F6: upload errors show the message only (`mcp_wire::js_error_text`, not a JS stack); a tab whose upload fails sends `close` so the bridge drops the partial file (bridge test `close_during_an_upload_drops_it`, seen failing with the discard removed; the tab side is wasm-only, compile-checked); `#mcp=` changes are picked up by a `hashchange` listener and reconnect with the new token; the `Bearer` scheme matches case-insensitively (RFC 7235; test `the_bearer_scheme_is_case_insensitive`, seen failing under the case-sensitive match). check.sh and check.sh --web passed. Not verified: the toast text, the hashchange reconnect and the partial-file cleanup in a real browser.

### 2026-10-13T000000Z b105-pr-16-fix-round-2-r1
- 2026-10-13 — PR #16 fix round 2 (R1–R9): an uploaded `File` is tied to its own load by a pick id (`Uploads::pick/start/discard/bind/fail`, tests incl. pick A → pick B → bind A); a remote IPv6 bind also listens on `[::1]`, the banner prints after every listener is bound and the first listener error ends the bridge; the connect timeout is cleared with its connection; `js_error_text` reads a DOMException `message`; `closed_message` is pure and tested; the deploy domain is gone from the repo (`--allow-origin https://<your-deploy-domain>` prints its `Web:` line). Each test seen failing with its fix reverted. check.sh and `check.sh --web` passed. Not verified: the wasm-only wiring (`web.rs`, `main.rs`, `mcp_web.rs`) beyond the build, and any real-browser run.
