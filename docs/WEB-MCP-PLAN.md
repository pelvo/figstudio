# Web app + MCP: plan

Goal: open a `.fig` in the browser app (local `scripts/web-build.sh --serve` or the deployed
Worker) and let an MCP client (Claude Code, Cursor, …) read it and drive the view, the same
way the desktop app's "Agent access" (live mode) works today.

## Why a bridge

A browser tab cannot listen on a port, and the MCP tools live in `figstudio-mcp`
(tokio + rmcp), which does not build for wasm32. So a small local process does the MCP side
and the browser tab connects to it:

```
MCP client ──HTTP /mcp──▶ figstudio-mcp --bridge (127.0.0.1:3845) ◀──WebSocket /bridge── browser tab
                          LiveFiles + Snapshot (unchanged tools)      uploads file, sends state,
                                                                       applies reveal_node
```

The bridge reuses `figstudio_mcp::live::LiveFiles` unchanged: it builds `Snapshot`/`TabInfo`
from what the tab sends, exactly as `apps/figstudio/src/mcp_host.rs` does on desktop. All 14
tools + the 3 live tools (`list_open_tabs`, `get_selection`, `reveal_node`) work with no
tool code changes.

Rejected: MCP on the Cloudflare Worker + Durable Object relaying to the tab. It needs the
tools rebuilt for wasm, uploads private files to the cloud, and is much bigger. Revisit only
if a remote agent (not on the user's machine) must reach the tab.

## Protocol (`/bridge`, one WebSocket per browser tab)

JSON text frames (`serde_json`) plus binary frames for file bytes. Types live in
`crates/mcp-core/src/bridge.rs` (wasm-safe, shared by both sides). Node/page ids as `"s:l"`.

| Direction | Message | Purpose |
|---|---|---|
| tab → bridge | `hello {token, version}` | First frame; wrong token or version → close 4401/4426 |
| tab → bridge | `open {tab, name, size}` then binary chunks, then `end {tab}` | Upload one file (name keeps the file key = stem) |
| tab → bridge | `state {tabs: [{tab, page, selection, viewport}], active}` | Sent when the desktop-style `Key` changes |
| tab → bridge | `close {tab}` | Tab closed |
| tab → bridge | `reply {id, error?}` | Answer to a reveal |
| bridge → tab | `welcome {version}` / `error {message, tab?}` | `tab` names a failed upload (`open`/chunk/`end`); the tab drops only that one |
| bridge → tab | `reveal {id, tab, node}` | From `Command::Reveal`; bridge replies to rmcp on `reply` or 5 s timeout (`REVEAL_TIMEOUT`) |

Bridge writes each upload to a per-session temp dir (`tempfile`) as `<name>.fig` and decodes
it with `figstudio_fig`; `TabInfo.path` points there so lazy image reads work. Temp dir is
removed when the socket closes.

## Security

- `/mcp` keeps its loopback Host/Origin allowlist (`serve_http`, unchanged).
- `/bridge` checks `Origin` against an allowlist: default `http://127.0.0.1:8765`,
  `http://localhost:8765`; add a deployed site with `--allow-origin https://<your-deploy-domain>`.
- Pairing token: bridge prints a random token (from `getrandom`) at start. The web app takes
  it from the URL fragment `#mcp=<token>` (fragment never hits the server/logs) or a field in
  the View menu. Without it any site could push files at the bridge.
- One browser connection at a time; a second `hello` replaces the first (newest tab wins).
- Upload cap `--max-upload-bytes` (default 2 GiB); over it → `error`, file dropped.
- Never log names/text/paths (doctor-style privacy is not required here, but keep logs to
  counts and ids).

## Lanes (one worker each, commit after the gate passes)

### L1 — protocol types (`crates/mcp-core`) — complete

Verified: all three protocol tests fail under targeted mutations and pass restored;
core tests, nightly wasm check and `scripts/check.sh` passed. The default toolchain's
wasm target is absent (the gate skips it); the explicit nightly wasm check passed.
- `bridge.rs`: message enums with serde derive (add `serde` to mcp-core deps), `PROTOCOL_VERSION`,
  guid ↔ `"s:l"` helpers (reuse `node_id`).
- Tests (`crates/mcp-core/tests/bridge.rs`): round-trip every message; unknown `type` → error,
  not panic; bad id string → error.
- Gate: `cargo test -p figstudio-mcp-core`, wasm check
  `cargo +nightly-2026-10-09 check -p figstudio-mcp-core --no-default-features --target wasm32-unknown-unknown`.

### L2 — bridge server (`apps/figstudio-mcp`) — complete

Verified: fixture upload/selection/reveal/timeout/screenshot, disconnect cleanup,
Origin/token/version rejection, replacement, malformed JSON/files, filename traversal
and upload bounds via real WebSocket + rmcp HTTP clients. Each of the three tests
failed under a targeted mutation; restored tests and `scripts/check.sh` passed.
Uploads use numbered subdirectories (preserving stems and avoiding collisions),
1 MiB frame bounds, and a separate `bridge_upload.rs` to keep modules small.
Bridge design assets are inline, so disconnect leaves no asset files. Ripwire
reported structural matches for redacted Debug and snapshot assignment to existing
unrelated Debug implementations/test helpers; these are intentional, not reusable
production helpers. No MCP tool implementation was changed.
- `main.rs`: `--bridge` flag; `FILE.fig` no longer required when it is set (clap
  `required_unless_present`). Prints `MCP: http://127.0.0.1:PORT/mcp` and the web URL with
  `#mcp=<token>`.
- `src/bridge.rs` (state: snapshot `Arc<RwLock<Arc<Snapshot>>>`, uploads, pending reveals) and
  `src/bridge_ws.rs` (enable axum's `ws` feature; axum is already a dep, `ws` pulls tokio-tungstenite). Keep
  each under ~300 lines.
- `serve_http` gains an optional extra router (or a `serve_bridge` sibling) so `/mcp` and
  `/bridge` share one listener.
- Decode on a blocking task (`spawn_blocking`); malformed file → `error` frame, never panic.
- Tests (`apps/figstudio-mcp/tests/bridge.rs`, tokio-tungstenite client + rmcp HTTP client):
  1. upload `tests/fixtures/structure.fig` → `list_open_tabs` shows it with key `structure`;
  2. `state` with a selection → `get_selection` returns that node;
  3. `reveal_node` → tab receives `reveal`, answers `reply` → tool returns OK; no answer → timeout error;
  4. wrong token → closed; foreign Origin → 403; oversized upload → `error`;
  5. socket close → tabs gone, temp dir removed.
  Prove each by breaking the code once (docs/TESTING.md).
- Gate: `scripts/check.sh`.

### L3 — web client (`apps/figstudio`, wasm only) — complete

Verified: `scripts/check.sh --web` passed (toolchain 1.99.0). Native tests cover the shared
`mcp_state.rs` and the pure bridge helpers in `mcp_wire.rs` (state frame, `#mcp=` fragment,
slicing, filename, bad reveal ids, close codes); each failed under a targeted mutation. The
socket code (`mcp_web.rs`, `mcp_upload.rs`) only compiles for wasm and is checked by hand (L4).
Notes: the picked `File` is kept per tab name in `web.rs` (`UPLOADS`) so any open tab can be
streamed in 256 KiB slices (`read_slice`, back-pressure on `bufferedAmount`); only uploaded
tabs appear in `state` frames (the bridge rejects unknown ids). Files dropped/picked as bytes
are streamed from the same `File`. `serde_json` is added to the wasm deps for the frames.
- Extract from `mcp_host.rs` into a shared, cfg-free `mcp_state.rs`: `Key`/`key()` and the
  reveal logic (select node, switch tab, fit view). Desktop behaviour byte-identical; existing
  `ui_mcp_tests.rs` must stay green.
- `src/mcp_web.rs`: same API as `mcp_off.rs` (`menu`, `status`, `frame`, `sync`, `stop`).
  `main.rs`: on wasm32 use `mcp_web.rs` instead of `mcp_off.rs`.
  - `web_sys::WebSocket` (add features `WebSocket`, `MessageEvent`, `BinaryType`, `CloseEvent`);
    incoming frames into an `Rc<RefCell<VecDeque>>`, drained in `frame()`.
  - View menu: "Connect to local MCP" with URL (default `ws://127.0.0.1:3845/bridge`) and token;
    auto-connect when `#mcp=` is in the URL. Status bar: connected / uploading n% / error.
  - On connect and on each newly opened tab: upload. Small files from `Source::Bytes`; big
    files streamed with the existing `read_range` in chunks (no full copy in wasm memory).
  - `sync()`: send `state` only when `Key` changes.
  - Disconnect → toast, keep the document open; manual reconnect.
- Tests: native unit tests for the shared `mcp_state.rs`; wasm side checked by hand (L4).
- Gate: `scripts/check.sh` and `scripts/check.sh --web` (crates/app changed).

### L4 — docs and end-to-end check — complete except manual browser runs

Done: README "In the browser (bridge mode)", PLAN.md box + progress line, MCP-PLAN.md note,
and `apps/figstudio-mcp/tests/bridge_cli.rs`: spawns the real binary with `--bridge`, parses
the printed token and URL, pairs a WebSocket (as the browser would), uploads `structure.fig`
and lists it through an rmcp HTTP client (fails when the printed token is wrong).
Manual TODOs (not run; no browser available to the worker):
- [ ] local build: `scripts/web-build.sh --serve`, open `http://127.0.0.1:8765/#mcp=<token>`,
      open `structure.fig`, run `list_open_tabs`, select a node → `get_selection`,
      `reveal_node` → the view jumps, `get_screenshot`.
- [ ] `https://<your-deploy-domain>/#mcp=<token>` in Chrome (Local Network Access prompt?) and Safari
      (mixed-content block of `ws://127.0.0.1`?). If blocked, the README already says use the local build.
- [ ] a >256 MiB lazily opened file uploads without exhausting memory; disconnect/reconnect toasts.
- `apps/figstudio-mcp/README.md`: "In the browser (bridge mode)" section, client config
  (`{"type":"http","url":"http://127.0.0.1:3845/mcp"}`).
- `docs/PLAN.md` box + progress line; `docs/MCP-PLAN.md` lane note.
- Manual run, record what was and was not verified:
  ```sh
  cargo run --release -p figstudio-mcp -- --bridge         # prints token + URL
  scripts/web-build.sh --serve                             # http://127.0.0.1:8765/#mcp=<token>
  claude mcp add --transport http figstudio http://127.0.0.1:3845/mcp
  ```
  Open `structure.fig` in the tab, then from the client: `list_open_tabs`, select a node in
  the browser → `get_selection`, `reveal_node` → browser jumps, `get_screenshot`.
  Repeat against `https://<your-deploy-domain>/#mcp=<token>` in Chrome and Safari.

## Risks to check early (before L3)

- **https page → `ws://127.0.0.1`.** Chrome allows loopback as a secure context but newer
  versions show a "Local network access" permission prompt; Safari historically blocks it as
  mixed content. Spike first: a 10-line page on the Worker opening `ws://127.0.0.1:3845`. If
  Safari blocks, document "use Chrome/Firefox or the local `--serve` build".
- COEP `require-corp` does not apply to WebSocket (no change to `web/_headers` expected);
  confirm in the spike.
- Upload time for very large files over loopback; the cap and progress bar cover it.

## Out of scope

Editing/writing through MCP, remote (non-loopback) agents, multiple simultaneous browser
tabs, Cloudflare-side MCP.

## L5 — remote agents (opt-in, token-guarded) — complete

Done: `access.rs` (`HttpAccess`, `subtle` constant-time compare, axum middleware on `/mcp` only),
`serve_http_with`, `serve_bridge(.., access)`, `--allow-host`, `--allow-any-interface`, app checkbox +
Copy token, README "Remote agents", `tests/remote.rs`. Deviations: the bridge no longer refuses a
non-loopback listener (it now follows the same access rules; `/bridge` is still Origin+pairing-token
guarded); the old "non-loopback bind warning" is replaced by the token banner and its stderr test by
`remote.rs`; the app has a specific-address field and never offers a wildcard. Not verified: a real
Tailscale or SSH-tunnel session from another machine.

Use case: figstudio (desktop or web + bridge) runs on the user's PC; the coding agent runs on
another machine (SSH box) and reaches it over Tailscale. The SSH `-R 3845:127.0.0.1:3845`
tunnel stays the zero-config option and needs none of this; document it in the README.

- Default unchanged: loopback only, no token.
- Listen address: `figstudio-mcp --http=ADDR` (exists) and `--bridge` reuse it; desktop app
  gets "Allow remote agents" beside Agent access with an address field (not persisted on,
  like `on`). `0.0.0.0` / `::` refused unless `--allow-any-interface`.
- Host allowlist: `--allow-host NAME` (repeatable) added to the loopback list; the bound IP
  is added automatically. Origin rules unchanged (agents send no Origin).
- Bearer token required whenever the bind is not loopback: `Authorization: Bearer <token>`,
  random (getrandom), shown by the CLI on start and in the app with a copy button. Missing or
  wrong → 401, constant-time compare (crate: `subtle` or `tower-http` auth layer, not hand
  rolled). Starting non-loopback without a token is impossible (token always generated).
- One change point: `serve_http` takes a config (bind kind, extra hosts, optional token) used
  by CLI, bridge and app.
- Client: `claude mcp add --transport http figstudio http://HOST:3845/mcp --header "Authorization: Bearer <token>"`.
- Plain HTTP is acceptable over Tailscale (WireGuard); README says TLS is not provided and
  other networks need a tunnel.
- Tests (`apps/figstudio-mcp/tests/remote.rs`): default bind is loopback; non-loopback
  without/with wrong token → 401, right token → tools/list ok; allowed Host passes, unknown
  Host rejected; `0.0.0.0` refused without the flag; app toggle keeps loopback when off.
  Prove each by breaking the code once.
- Gate: `scripts/check.sh`. Docs: README "Remote agents" (Tailscale + SSH tunnel), PLAN.md.

## PR #16 review fixes (reviewer tested in a real browser at `0770896`)

All findings were reproduced by the reviewer; none are false positives. Fix in this order,
one commit per group, `scripts/check.sh` (+ `--web` for app changes) before each commit.

### F1 — blocker: uploads fail in the threads build (`apps/figstudio/src/mcp_upload.rs:~99`)
`ws.send_with_u8_array(&bytes)` passes a view of shared Wasm memory; `WebSocket.send` rejects
shared views. Copy into a fresh non-shared `js_sys::Uint8Array::new_with_length` +
`copy_from`, then `send_with_array_buffer(&chunk.buffer())` (reviewer's probe: 31 KB and
300 MiB uploads arrived byte-identical). No native test can catch this; say so in the commit.

### F2 — major: two files with the same name (`apps/figstudio/src/web.rs:~333`)
`UPLOADS` is keyed by file name, so picking two different `same.fig` files at once shows the
first in the tab but uploads the second. Key the kept `File` by tab (or carry it with the
opened document into the tab) so the bridge always gets the bytes the tab shows; reconnect
re-uploads use the same lookup. Smallest change that makes tab ↔ File one-to-one; extract the
mapping into a pure helper with a native unit test if practical.

### F3 — remote bind breaks pairing (`apps/figstudio-mcp/src/main.rs:~162`)
`--bridge --http=<tailscale-ip>:PORT` listens only on that IP, so the browser on the same PC
(default `ws://127.0.0.1:3845/bridge`) is refused and the printed links cannot pair. The
browser and bridge are always on the same PC; only the agent is remote. When the bind is
non-loopback and not unspecified, also listen on `127.0.0.1:<same port>` serving `/bridge`
(and loopback-only `/mcp`). Print the `ws://` bridge URL in the banner. Integration test:
`--bridge --http=<non-loopback>` pairs on `ws://127.0.0.1:<port>/bridge`.

### F4 — `0.0.0.0` + token still 403 by Host (`apps/figstudio-mcp/src/access.rs:~76`)
With an unspecified bind no address is in the Host allowlist, so LAN/Tailscale IPs get 403
even with the right token. Do not enumerate interfaces (needs a new crate); when the bind is
unspecified and `--allow-host` is empty, print a banner line saying clients must use a name or
IP passed with `--allow-host`, and say the same in the README Remote agents section.

### F5 — misleading connect error / no timeout (`apps/figstudio/src/mcp_web.rs:~176`)
On an https page Chromium's Local Network Access check closes with 1006 and the app says the
bridge isn't running; WebKit blocks as mixed content and fires no `close`. Change the 1006
message to also mention allowing local network access / that Safari blocks https → ws://
(use `http://127.0.0.1:8765` or Chrome). Add a connect timeout (8 s without `welcome` → close
and show that message). Put the message choice in `mcp_wire.rs` (pure) with a unit test.

### F6 — minors
- Upload error toasts print the whole JS stack (`format!("{e:?}")` on `JsValue`): show only the
  error message (`js_sys::Error` message or `as_string`), via a small helper.
- A client-side upload failure leaves a 0-byte in-progress upload on the bridge: send
  `close {tab}` (or the existing cancel path) when an upload fails in the tab; bridge test that
  `close` during an upload drops it.
- `#mcp=` is read only at startup: listen for `hashchange` and reconnect with the new token.
- `Authorization: bearer <token>` (lowercase scheme) → 401: match the scheme
  case-insensitively (RFC 7235). Unit test in `access.rs`.

Out of scope: a headless-browser smoke test (suggested by the reviewer; needs CI browser infra).

## Fix round 2 (cross-family review of F1–F6)

- R1 major — F2 still wrong for pick→pick→bind: `Uploads::pick` replaces a pending same-name
  entry before the first load binds. Bind the `File` to the load itself (a pick sequence id
  carried through `Picked`/`Loaded` into `bind`), not by name at load completion. Unit test
  for pick A, pick B, bind A → A's file.
- R2 major — F3 extra loopback listener is IPv4-only (`bridge.rs:~103`): for a V6 bind also
  listen on `[::1]:port`. Test with an IPv6 bind if the host has one; skip visibly otherwise.
- R3 minor — clear pending entries when an open fails; forget a closed tab's `File` even if it
  never started uploading.
- R4 minor — print the pairing banner only after both listeners are bound.
- R5 minor — the first listener error ends the bridge (`try_join!`/select + cancel), not at
  Ctrl-C.
- R6 minor — keep the F5 timeout closure in `Conn` and drop it with the connection (no
  `forget`).
- R7 minor — JS error text: also read a string `message` property (DOMException) via
  `js_sys::Reflect::get` before the fallback.
- R8 minor — move the welcomed/close-code message choice into `mcp_wire` with a test; print a
  visible skip line when the F3 test has no LAN address.
- R9 privacy — the deploy domain must not appear in the repo (main keeps web/wrangler.jsonc
  ignored for this reason). Remove it from `DEFAULT_ORIGINS` (keep only the local :8765
  origins), from the banner and from all docs. The deployed site is allowed with
  `--allow-origin https://<your-deploy-domain>`, and the banner prints a `Web:` line for each
  extra https origin given. Docs use the placeholder `<your-deploy-domain>`.
