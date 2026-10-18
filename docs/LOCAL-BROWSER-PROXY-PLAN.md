# Local Browser Proxy (`--browser-proxy`) — Design Plan

Status: plan only. Base: `feat/multi-device-sessions` (cloud relay + Wasm RPC executor).

---

## 1. Problem statement

`figstudio` currently offers two ways for an MCP client to talk to the browser app, and
each has a trade-off when the browser app is hosted behind an access gateway (such as
Cloudflare Access) on the same machine as the coding agent:

| Surface | How it works today | Pain points |
|---|---|---|
| **Local upload bridge** (`figstudio-mcp --bridge`) | Local HTTP server on `127.0.0.1:3845`; browser connects over `ws://127.0.0.1:3845/bridge` and **uploads the entire `.fig` file** (up to 2 GiB) in 256 KiB chunks so the native process can decode it. | 1. Uploading a 1.7 GB `.fig` over WebSocket into `$TMPDIR` duplicates memory and takes seconds.<br>2. HTTP-only (`--http`): the user must start `figstudio-mcp --bridge` manually in a terminal and copy `#mcp=<token>` into the browser on every run. |
| **Cloud relay** (`web/worker.mjs` + `McpRoom` DO) | Browser runs the MCP tools in Wasm (`mcp_relay.rs`, zero upload) and connects to `wss://<host>/bridge/rm_<room>`; agent POSTs to `https://<host>/mcp/rm_<room>`. | 1. When `<host>` is protected by Cloudflare Access (Google login), a CLI agent calling `/mcp/rm_…` hits the 302 login redirect unless `/mcp/*` is bypassed or Access tokens are configured.<br>2. The user still has to copy the MCP URL into the agent config once. |

Meanwhile, [`googlecolab/colab-mcp`](https://github.com/googlecolab/colab-mcp) solves the
exact same problem for Google Colab notebooks:

1. The agent spawns `colab-mcp` over **stdio** (zero URLs or tokens in the agent config).
2. `colab-mcp` binds `ws://127.0.0.1:<ephemeral_port>` and generates a one-time random token.
3. When the agent calls `open_colab_browser_connection` (or on startup), `colab-mcp` opens
   the browser at `https://<web-origin>/...#mcpProxyToken=<token>&mcpProxyPort=<port>` and
   waits up to 60 s for the browser tab to connect back to `ws://127.0.0.1:<port>`.
4. Because the user's browser is **already signed in** to the web origin, it loads the app
   immediately, reads `#mcpProxyToken` and `#mcpProxyPort` from `location.hash` (which
   never leaves the browser), connects to `ws://127.0.0.1:<port>`, and executes MCP
   requests in-page.
5. Once the WebSocket connects, `colab-mcp` emits `notifications/tools/list_changed` and
   forwards JSON-RPC calls directly over the WebSocket.

Because `figstudio` already has a complete in-browser Wasm JSON-RPC executor
(`apps/figstudio/src/mcp_relay.rs` + `crates/mcp-core/src/wasm_rpc`), we can apply the
`colab-mcp` pattern directly on top of `mcp_relay.rs` with zero file uploads.

---

## 2. Goals and non-goals

### Goals

- **Zero token copy-paste:** the agent config is a static `stdio` command
  (`figstudio-mcp --browser-proxy --origin https://figstudio.example.com`); the user never
  copies a token into the agent or into the browser.
- **Zero `.fig` upload:** the browser tab executes MCP tools in Wasm (`mcp_relay.rs`)
  against its open `File` handles; `.fig` bytes never cross the WebSocket.
- **Works behind Cloudflare Access with zero Access bypass:** the CLI agent talks to local
  `figstudio-mcp` over `stdio`; the browser (already signed into Access) connects outbound
  to `ws://127.0.0.1:<port>/bridge`.
- **Strict local isolation:** other users who have Access to the web deployment cannot see
  or reach your local session (`127.0.0.1` loopback + `Origin` check + 256-bit token).
- **Supports both auto-launch and already-open tabs:**
  - If no tab is connected, calling `open_figstudio_browser` launches the browser with
    `#mcpProxyPort=<port>&mcpProxyToken=<token>` and waits up to 60 s for `ready`.
  - If several tabs on the machine connect to the same local proxy, `list_sessions` and
    `select_session` work identically to the cloud relay.

### Non-goals

- Removing the Cloud Relay (`web/worker.mjs`): Cloud Relay remains the right transport when
  the agent runs on a remote server/container rather than the same machine as the browser.
- Removing `compare_files` / `compare_page` from native CLI mode.

---

## 3. Architecture

```text
┌──────────────────────────┐
│ Coding Agent             │
│ (Claude / Cursor / Codex)│
└────────────┬─────────────┘
             │ MCP over stdio (or --http)
             │ (no URL or token needed in agent config)
             ▼
┌──────────────────────────────────────────────────────────┐
│ figstudio-mcp --browser-proxy                            │
│  • Binds ws://127.0.0.1:<port>/bridge (tries 3845..3855, │
│    falls back to ephemeral port 0)                       │
│  • Generates 256-bit random token                        │
│  • Exposes `open_figstudio_browser` + `list_sessions`    │
│    even before a tab connects                            │
│  • Emits `notifications/tools/list_changed` when the     │
│    first tab says `ready`                                │
└────────────┬─────────────────────────────────┬───────────┘
             │ 1. `open_figstudio_browser`     │ 2. WebSocket upgrade
             │    launches default browser:    │    ws://127.0.0.1:<port>/bridge
             │    <origin>/#mcpProxyPort=…     │    Origin: <allowed-origin>
             │    &mcpProxyToken=…             │    Sec-WebSocket-Protocol:
             ▼                                 │      figstudio-exec-v1, <token>
┌──────────────────────────────────────────────┴───────────┐
│ User's Browser Tab (already signed into Access)          │
│  • Reads & strips `#mcpProxyPort` + `#mcpProxyToken`     │
│  • Switches `McpHost` to Wasm relay mode (`mcp_relay.rs`)│
│  • Sends `ready` v2 (device, instance, focused, docs)    │
│  • Executes `rpc` frames in Wasm Rayon pool (0 upload)   │
└──────────────────────────────────────────────────────────┘
```

### 3.1 CLI surface (`apps/figstudio-mcp`)

Add `--browser-proxy` to `figstudio-mcp`:

```sh
figstudio-mcp --browser-proxy [--origin https://figstudio.example.com] [--http[=ADDR]]
```

- **Default transport is `stdio`** (matching `colab-mcp`), so MCP clients launch it on
  demand without a background daemon. Passing `--http` (or `--http=ADDR`) additionally
  serves Streamable HTTP on `/mcp` for clients that prefer HTTP.
- **`--origin <URL>`** (repeatable, or `FIGSTUDIO_WEB_ORIGIN` env var): allowed browser
  `Origin`s, plus the target URL opened by `open_figstudio_browser`. Defaults to
  `http://127.0.0.1:8765` and `http://localhost:8765`.
- **Port selection:** tries `127.0.0.1:3845..=3855` first, then `127.0.0.1:0` (ephemeral
  OS port), and binds `[::1]:<same_port>` on a best-effort basis so `localhost` resolves
  cleanly on both IPv4 and IPv6.

### 3.2 Handshake and wire protocol (reusing `mcp_relay.rs` and `EXEC_PROTOCOL`)

Instead of inventing a third wire protocol, `--browser-proxy` speaks the **exact same
WebSocket protocol** that the browser Wasm app already uses for the Cloud Relay
(`apps/figstudio/src/mcp_wire.rs` + `mcp_relay.rs`):

1. **Fragment handoff (`apps/figstudio/src/mcp_join.rs`):**
   - Parse `#mcpProxyPort=<u16>&mcpProxyToken=<64 hex>` from `location.hash`.
   - Strip both keys from the address bar immediately via `history.replaceState`
     (`mcp_room::replace_fragment`) so the token never lingers in the URL bar or browser
     history.
   - Because the fragment was supplied in the navigation triggered by the user's local
     `figstudio-mcp` process and connects only to `ws://127.0.0.1:<port>/bridge` (zero
     file upload — read-only Wasm RPC), the tab connects immediately without requiring a
     manual token paste.
2. **WebSocket upgrade (`ws://127.0.0.1:<port>/bridge`):**
   - `Origin` header must match one of the configured `--origin` values.
   - `Sec-WebSocket-Protocol: figstudio-exec-v1, <token>` (constant-time comparison against
     the 256-bit process token; the 101 response echoes `Sec-WebSocket-Protocol: figstudio-exec-v1`).
3. **Frames after upgrade:**
   - Browser → Proxy:
     - `{"type":"ready","protocol":2,"device":…,"instance":…,"focused":…,"docs":…}`
     - `{"type":"rpc_reply","id":"…","status":200,"body":{…}}`
     - `"ping"` → Proxy auto-replies `"pong"`
   - Proxy → Browser:
     - `{"type":"rpc","id":"…","body":{…}}`

### 3.3 Bootstrap tool (`open_figstudio_browser`) and dynamic tool list

Following `colab-mcp`'s `open_colab_browser_connection` + `ColabProxyMiddleware`:

1. **`tools/list` when no tab is connected:**
   - Returns `open_figstudio_browser`, `list_sessions`, and `select_session` (plus the
     standard read-only tool schemas so agents that do not support
     `notifications/tools/list_changed` can still see the tool signatures).
2. **`open_figstudio_browser` tool:**
   - If a ready browser tab is already connected, returns immediately with the connected
     session summary (`list_sessions`).
   - Otherwise, opens `<primary_origin>/#mcpProxyPort=<port>&mcpProxyToken=<token>` in the
     system browser (`open` on macOS, `xdg-open` on Linux, `cmd /c start` on Windows) and
     waits up to 60 s (`UI_CONNECTION_TIMEOUT`) for the first `ready` v2 frame.
   - Once the tab sends `ready` v2, emits `notifications/tools/list_changed` and returns
     the connected session and open documents (`docs`).
   - If a normal tool (`get_metadata`, `list_open_tabs`, …) is called while no tab is
     connected, returns a clear tool error instructing the agent to call
     `open_figstudio_browser` (or telling the user to load a `.fig` file in the opened tab).

### 3.4 Multi-tab routing on the same machine

When multiple browser tabs connect to the same local `--browser-proxy` process:

- Single device (`device.id` shared across tabs of the same browser): automatically routes
  to the newest focused tab (`focused: true`), or the newest ready tab.
- `list_sessions`, `select_session`, explicit `arguments.session`, and
  `result._meta["figstudio/session"]` work identically to `web/worker.mjs`.

---

## 4. Browser limitations and fallback

- **Chromium / Edge / Firefox (`https://` → `ws://127.0.0.1`):** W3C Mixed Content marks
  `ws://127.0.0.1` and `ws://localhost` as Potentially Trustworthy origins, so `https://`
  pages can open loopback WebSockets. Chromium 142+ may show a one-time "Local Network
  Access" permission prompt per origin; once allowed, subsequent connections are silent.
- **Safari:** WebKit blocks `ws://127.0.0.1` from `https://` origins as mixed content. On
  Safari, users either run the local web build (`http://127.0.0.1:8765`) or use the Cloud
  Relay (`wss://<host>/bridge/rm_…`).

---

## 5. Phases and acceptance

| Phase | Scope | Acceptance |
|---|---|---|
| **1 — Browser fragment & local Wasm relay** | `mcp_join.rs` parses and strips `#mcpProxyPort=<port>&mcpProxyToken=<token>`; `McpHost` connects to `ws://127.0.0.1:<port>/bridge` using `Sec-WebSocket-Protocol: figstudio-exec-v1, <token>` and runs `mcp_relay.rs` (zero upload) instead of `mcp_upload.rs` | Unit tests in `mcp_join` / `mcp_wire` for fragment parsing, stripping, port bounds, and subprotocol offering |
| **2 — `figstudio-mcp --browser-proxy` server** | New `proxy.rs` in `apps/figstudio-mcp`: `stdio` (default) + `--http` MCP front-end, loopback WebSocket `/bridge` listener with `Origin` + subprotocol token check, `open_figstudio_browser`, `list_sessions`, `select_session`, `notifications/tools/list_changed` | Integration tests: unauthorized `Origin`/token rejected (403); `open_figstudio_browser` waits for `ready` v2; `get_metadata` / `get_screenshot` / `reveal_node` round-trip over WebSocket without uploading `.fig` bytes |
| **3 — Chromium E2E & docs** | Playwright Chromium E2E: `figstudio-mcp --browser-proxy` spawns, opens Chromium with `#mcpProxyPort=…&mcpProxyToken=…`, drops `structure.fig`, and answers `list_open_tabs` + `get_metadata` over `stdio`; update `apps/figstudio-mcp/README.md` and `.agents/skills/` | `FIGSTUDIO_TOOLCHAIN=1.99.0 FIGSTUDIO_WEB_TOOLCHAIN=nightly scripts/check.sh --web` all checks passed + Playwright E2E green |
