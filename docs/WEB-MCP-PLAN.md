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
| bridge → tab | `welcome {version}` / `error {message}` | |
| bridge → tab | `reveal {id, tab, node}` | From `Command::Reveal`; bridge replies to rmcp on `reply` or 5 s timeout (`REVEAL_TIMEOUT`) |

Bridge writes each upload to a per-session temp dir (`tempfile`) as `<name>.fig` and decodes
it with `figstudio_fig`; `TabInfo.path` points there so lazy image reads work. Temp dir is
removed when the socket closes.

## Security

- `/mcp` keeps its loopback Host/Origin allowlist (`serve_http`, unchanged).
- `/bridge` checks `Origin` against an allowlist: default `https://figstudio.vtz.io`,
  `http://127.0.0.1:8765`, `http://localhost:8765`; extend with `--allow-origin URL`.
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
- [ ] `https://figstudio.vtz.io/#mcp=<token>` in Chrome (Local Network Access prompt?) and Safari
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
  Repeat against `https://figstudio.vtz.io/#mcp=<token>` in Chrome and Safari.

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

## L5 — remote agents (opt-in, token-guarded)

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
