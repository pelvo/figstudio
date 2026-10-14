# Browser Wasm MCP Executor + Hibernating Relay: Plan

Goal: let any MCP client (Claude Code, Cursor, QwenPaw, …) drive a `.fig` opened in the
deployed browser app (`https://<your-deploy-domain>`) **without running a local CLI and
without uploading `.fig` bytes out of the browser tab**.

---

## 1. Architecture & Why It Replaces File Upload

In the browser build (`apps/figstudio`), the tab already holds:
- `Tab.doc: Arc<Document>` — the parsed Figma tree in Wasm memory;
- `Tab.source: Option<Arc<dyn ImageSource>>` (`WebSource`) — synchronous-on-Rayon-worker
  byte-range reads from the browser `File` via `Blob.slice().arrayBuffer()`;
- `figstudio-mcp-core` — already compiles for `wasm32-unknown-unknown` with
  `--no-default-features`.

Instead of uploading a 100 MB–1.7 GB `.fig` to a server to parse it a second time, **the
browser tab executes MCP tools directly in Wasm on its Rayon pool**, and a stateless
**Cloudflare Worker + Hibernating Durable Object (`McpRoom`)** only relays small JSON-RPC
messages (~1–500 KB) between the MCP client and the browser tab:

```text
Claude Code / Cursor
   │
   │  POST https://<deploy-domain>/mcp/<room_token>   (MCP Streamable HTTP JSON-RPC)
   ▼
Cloudflare Worker + Durable Object `McpRoom` (idFromName(room_token))
   │  • WebSocket Hibernation (`ctx.acceptWebSocket(ws)` -> 0 GB-s idle cost)
   │  • Zero storage, zero `.fig` bytes
   │
   │  WSS https://<deploy-domain>/bridge/<room_token> (same-origin, works in Safari/Chrome/iPad)
   ▼
Browser Tab (Rust Wasm)
   ├─ Read-only tools (`get_metadata`, `get_design_context`, `get_screenshot`,
   │  `get_variable_defs`, `get_figjam`, `get_nodes_in_region`, `get_node_images`,
   │  `search_design_system`, `get_code_connect_map`, …)
   │    ──► run on Rayon worker against `Arc<Document>` + `WebSource` + shared `Images`
   └─ Live UI tools (`list_open_tabs`, `get_selection`, `reveal_node`)
        ──► read/update `McpHost` state & set `tab.fit_node` / `tab.selection`
```

### Multi-User & Multi-Tab Isolation
- On first load, the browser generates a 256-bit random `room_token` (`rm_<64 hex chars>`)
  and stores it in `localStorage["figstudio.mcp.room"]` (with a **Regenerate** button in
  `View -> Agent access`).
- Each `room_token` maps to its own isolated Durable Object instance
  `env.MCP_ROOMS.idFromName(room_token)`.
- The user clicks **Copy MCP URL** in the browser menu, getting:
  `https://<your-deploy-domain>/mcp/rm_<64 hex chars>`
- That capability URL is both the user/room identifier and the 256-bit secret. Different
  users (or different browser profiles) never share a room.
- If a user opens multiple tabs in the same browser, the newest connected tab (or the tab
  most recently focused) becomes the active executor for that room, or tabs can be
  aggregated by tab ID.
- The existing local `ws://127.0.0.1:3845/bridge` mode (`figstudio-mcp --bridge`) remains
  available as an offline option.

### Cost Profile (Cloudflare Free / Paid Plan)
- **WebSocket Hibernation API (`ctx.acceptWebSocket`)**: the Durable Object hibernates
  between MCP tool calls, so idle tabs consume **0 GB-s** of duration.
- **20:1 WebSocket message billing**: 20 JSON-RPC messages over the WebSocket count as 1
  request.
- **0 bytes of Durable Object storage**: no `.fig` or image is ever persisted in Cloudflare.
- Fits well inside the Cloudflare Workers **Free tier** (100k req/day, 13,000 GB-s/day).

---

## 2. Wire Protocol Between `McpRoom` Durable Object and Browser Tab

Same-origin WebSocket: `wss://<deploy-domain>/bridge/<room_token>`

| Direction | Frame (JSON text) | Purpose |
|---|---|---|
| tab -> DO | `{"type":"ready","protocol":1,"tab_title":"..."}` | Sent on open and when tabs/selection change |
| DO -> tab | `{"type":"rpc","id":"<req_uuid>","body":<JSON-RPC request>}` | Forwarded from `POST /mcp/<room_token>` |
| tab -> DO | `{"type":"rpc_reply","id":"<req_uuid>","status":200,"body":<JSON-RPC response>}` | Tool/initialize/list result from Wasm |
| tab <-> DO | WebSocket auto-response ping/pong (`setWebSocketAutoResponse`) | Keeps socket alive without waking the hibernating DO |

### MCP JSON-RPC Methods Handled in Wasm (`crates/mcp-core/src/wasm_rpc.rs`)
Because `rmcp` depends on `tokio` and is native-only, `crates/mcp-core` gains a small,
dependency-free JSON-RPC 2.0 dispatcher (`wasm_rpc.rs`, `serde_json` only) that handles:
1. `initialize` -> returns `protocolVersion: "2025-03-26"`, `capabilities: { tools: {}, prompts: {} }`, `serverInfo: { name: "figstudio-web-mcp", version }`.
2. `notifications/initialized` / `ping` -> `202 Accepted` / `{}`.
3. `tools/list` -> returns the canonical tool schemas (shared with `apps/figstudio-mcp`).
4. `tools/call` -> dispatches to `figstudio-mcp-core` functions (`metadata`, `screenshot`,
   `region_tool`, `variables`, `design`, `assets`, `figjam`, `design_system`,
   `codeconnect_context`, plus live `list_open_tabs`, `get_selection`, `reveal_node`),
   returning MCP `content` blocks (`text` and base64 `image/png`).
5. `prompts/list` & `prompts/get` -> returns the existing design/inspect prompts.

---

## 3. Enabling Lazy Image Rendering in Wasm (`crates/mcp-core/src/rendering.rs`)

Currently `Rendering::read(&self, entry: &Entry)` opens `self.path: Option<PathBuf>` with
`std::fs::File::open`, which always returns `None` on `wasm32`.

Change `Rendering` to accept an optional custom entry reader closure:

```rust
pub type EntryReader = Arc<dyn Fn(&Entry) -> Option<Vec<u8>> + Send + Sync>;

#[derive(Clone, Default)]
pub struct Rendering {
    pub system_fonts: bool,
    pub path: Option<PathBuf>,
    pub reader: Option<EntryReader>,
    pub images: Images,
    lazy_lock: Arc<Mutex<()>>,
}
```

- On desktop/CLI (`figstudio-mcp`), `Rendering::new(path, images)` continues to read from
  `path` via `std::fs::File`.
- In the browser (`apps/figstudio`), `Rendering::with_reader(reader, images)` wraps
  `tab.source` (`WebSource::read`), and `tools/call` runs on the Rayon pool
  (`rayon::spawn`) where `WebSource::read` already blocks on the main-thread `Blob.slice()`
  channel.
- Result: `get_screenshot`, `get_design_context` (with inline assets) and `get_node_images`
  render lazy 1.7 GB `.fig` files in the browser with zero code duplication.

---

## 4. Implementation Lanes

### Lane 1 — Custom Entry Reader & Wasm JSON-RPC Dispatcher (`crates/mcp-core`)
- **Files**: `crates/mcp-core/src/rendering.rs`, `crates/mcp-core/src/wasm_rpc.rs`,
  `crates/mcp-core/src/lib.rs`, `crates/mcp-core/tests/wasm_rpc.rs`.
- **Work**:
  1. Add `EntryReader` and `Rendering::with_reader` in `rendering.rs`; fallback to
     `self.path` when `reader` is `None`.
  2. Add `wasm_rpc::handle_request(req_json: &str, env: &dyn McpTabEnv) -> Option<String>`
     implementing `initialize`, `ping`, `tools/list`, `tools/call` (all read-only tools +
     `list_open_tabs`, `get_selection`, `reveal_node`), and `prompts/list` + `prompts/get`.
  3. Inline PNG assets in `get_design_context` when `fs` feature is disabled.
- **Gate**:
  - `cargo test -p figstudio-mcp-core`
  - `cargo +nightly check -p figstudio-mcp-core --no-default-features --target wasm32-unknown-unknown`
  - Every new test in `tests/wasm_rpc.rs` seen failing under a targeted mutation.

### Lane 2 — Browser Wasm Relay Client & Rayon Tool Runner (`apps/figstudio`)
- **Files**: `apps/figstudio/src/mcp_web.rs`, `apps/figstudio/src/mcp_wire.rs`,
  `apps/figstudio/src/mcp_wire_tests.rs`.
- **Work**:
  1. Generate/load persistent 256-bit `room_token` in `localStorage["figstudio.mcp.room"]`
     and derive default relay URL `wss://<location.host>/bridge/<room_token>` when served
     over `http(s)://` (keeping `ws://127.0.0.1:3845/bridge` selectable for local CLI bridge).
  2. In `View -> Agent access`, add:
     - **Copy MCP URL** button (`https://<location.host>/mcp/<room_token>`)
     - **Regenerate Room Token** button
     - Connection status indicator (`Connected to Cloud Relay` / `Connected to Local Bridge`).
  3. When a `{"type":"rpc","id":...,"body":...}` message arrives on the WebSocket:
     - Snapshot the open `Tab`s (`id`, `path` stem, `Arc<Document>`, `Option<Source>`,
       `page`, `selection`, `viewport`).
     - Spawn `wasm_rpc::handle_request` on the Rayon pool so `WebSource::read` can fetch
       lazy image slices without blocking the main UI thread; apply any `reveal_node`
       command on the next UI frame; send `{"type":"rpc_reply","id":...,"body":...}` back.
- **Gate**:
  - `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh --web`
  - Unit tests in `mcp_wire_tests.rs` for room token validation, URL derivation, and RPC
    envelope round-trip.

### Lane 3 — Cloudflare Worker + Hibernating Durable Object (`web/worker.mjs`)
- **Files**: `web/worker.mjs` (or repo-external deploy worker in `figstudio-deploy`),
  `apps/figstudio-mcp/tests/relay_e2e.rs`.
- **Work**:
  1. Add a lightweight Worker script in front of static assets (`env.ASSETS.fetch(req)` for
     all non-`/mcp/*` and non-`/bridge/*` paths, preserving COOP/COEP headers).
  2. Route `GET /bridge/:room` (`Upgrade: websocket`) and `POST /mcp/:room` to Durable
     Object `McpRoom` (`env.MCP_ROOMS.idFromName(room)` after validating `room` matches
     `^rm_[0-9a-f]{64}$`).
  3. Implement `McpRoom` with the **WebSocket Hibernation API**:
     - `ctx.acceptWebSocket(serverWs, ["tab"])`
     - `ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair("ping", "pong"))`
     - On `POST /mcp/:room`: if the body is a notification (no `id`), return `202 Accepted`;
       otherwise pick the active `"tab"` WebSocket, send `{"type":"rpc","id":uuid,"body":...}`,
       and await the matching `rpc_reply` in `webSocketMessage` (with a 30 s timeout).
  4. Configure `new_sqlite_classes = ["McpRoom"]` in `wrangler.jsonc` (supported on both
     Cloudflare Free and Paid plans).
- **Gate**:
  - End-to-end headless Chromium + Playwright / Rust rmcp client test: open the web app in
    headless Chromium, open `structure.fig`, connect an MCP HTTP client to `/mcp/<room>`,
    and verify `list_open_tabs`, `get_metadata`, `get_screenshot`, and `reveal_node`
    succeed with 0 `.fig` bytes uploaded.

#### Lane 3 implementation notes

- `web/worker.mjs` is a transport-only relay using platform Fetch, Web Crypto, JSON
  and Cloudflare's WebSocket Hibernation APIs (no custom protocol parser or packages).
  Static asset responses are returned untouched, including the `_headers` isolation
  headers. Invalid room paths return 404 before a DO lookup; no room tokens are logged.
- `McpRoom` has no storage calls. Socket attachments retain the active-tab ordering across
  hibernation; newest connect or protocol-1 `ready` wins. Pending calls live only in memory,
  with a 30-second deadline, socket-bound reply matching, and disconnect/send-error cleanup.
  Requests are capped at 1 MiB (including streamed bodies), incoming frames at 8 MiB,
  admission at 128 concurrent RPCs and 16 connected browser tabs per room.
  Notifications/responses return 202, batches return a JSON-RPC error, and GET returns 405.
  HTTP requests and WebSocket upgrades with a foreign Origin are refused. The room URL remains a
  capability secret: anyone possessing it can drive the tab, so do not publish it.
- Node 22+ tests (no installs/network): `node --test web/worker/*.test.mjs`.
  Fake Cloudflare sockets/context exercise forwarding, routing, isolation headers, deadline,
  disconnects, malformed/oversized messages, hibernation ordering and auto-response setup.
  Targeted mutations disabling room validation and returning 200 for notifications each
  made the corresponding test fail; both were restored before the full passing run.
  Wrangler 4 dry-run bundled the Worker and confirmed both bindings (assets override used
  test files, not a built app); no deployment occurred.
- `web/e2e/relay.spec.mjs` is the Chromium gate for the combined Lane 1–3 build. It opens
  the public fixture through the browser picker, reads the localStorage room, exercises
  initialize/list_open_tabs/get_metadata/get_screenshot/reveal_node, checks PNG content
  and selection, and inspects outgoing HTTP bodies and WebSocket frames for uploads.
  This script is written but **not run**; Lane 2 is not present in this worktree.

#### Deploy the relay

The committed template is **`wrangler.jsonc` at the repository root**, separate from the
ignored per-machine `web/wrangler.jsonc`. It uses a placeholder Worker name, no account IDs
or domains, `web/dist` assets, and a SQLite-class migration for `McpRoom` (no storage use).
Always select this config explicitly to avoid picking the older static-only web config.

```sh
scripts/web-build.sh
# Set the placeholder name to your own Worker name in a local copy if needed.
npx wrangler@4 dev --config wrangler.jsonc
npx wrangler@4 deploy --config wrangler.jsonc
```

`web-build.sh` copies `web/_headers` into `web/dist`; do not omit it, as the Wasm threads
need COOP/COEP/CORP. `run_worker_first: true` lets the Worker route the two relay prefixes;
all other paths use `ASSETS.fetch`. Enable cloud Agent access in the built Lane 2 app and
copy its MCP URL. There is no SSE endpoint, external CORS access, file upload, or database.

Optional browser gate (install the official Playwright runner only in its subdirectory):

```sh
npm --prefix web/e2e install
cd web/e2e
npx playwright install chromium
RELAY_BASE_URL=http://localhost:8787 npm test
```

The gate assumes the Lane 2 app connects to cloud relay on load (or that its saved Agent
access choice enables it). Deployment, real hibernation/eviction, and the combined browser
session have not been verified by the Node doubles.
