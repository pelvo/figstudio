---
name: cloud-relay-sessions
description: "Connect an MCP client to the figstudio browser app through the Cloudflare Worker relay, link multiple machines to one room without adding extra tokens, and route tool calls across browser sessions. Use for: figstudio cloud relay, /mcp/rm_, Copy MCP URL, Copy room link, list_sessions, select_session, -32002 Several figstudio sessions are connected across devices."
---
## When to Use
Use when the user opens `.fig` files in the browser build of figstudio on a deployed origin (or `wrangler dev`) and wants an MCP client to inspect or reveal layers without uploading `.fig` files or running `figstudio-mcp --bridge` locally. Also use when the same person has figstudio open on more than one computer (for example an office machine and a home machine) sharing one room token, or when a tool call returns JSON-RPC error `-32002` (`Several figstudio sessions are connected across devices`).

## Procedure
1. **Configure the MCP client once (first machine).**
   - In the browser app, open **View → Agent access**, select **Cloud relay**, and click **Copy MCP URL**.
   - The URL has the form `https://<host>/mcp/rm_<64 lowercase hex>`. Register that single URL in the MCP client:
     ```sh
     claude mcp add --transport http figstudio https://<host>/mcp/rm_<64 hex>
     ```
   - The browser persists its 256-bit executor token (`ex_<64 hex>`) in `localStorage["figstudio.mcp.executor.v2"]`, so reloading the tab, restarting the browser, or opening different `.fig` files keeps the same `/mcp/rm_…` URL.

2. **Link a second machine to the same room (no second MCP URL or token needed).**
   - On the first machine, open **View → Agent access** and click **Copy room link** (`https://<host>/#room=rm_…&exec=ex_…`).
   - Open that link in the browser on the second machine and confirm **Join that room** (or paste the `ex_…` token into the **executor token or room link** box and click **Apply**). The URL fragment `#room=…&exec=…` is stripped from the address bar immediately.
   - Set a clear **Device label** on each machine (for example `Office · Chrome` and `Home · Mac`); it is saved in `localStorage["figstudio.mcp.device_label.v1"]`. Each machine keeps its own random `device.id` in `localStorage["figstudio.mcp.device.v1"]`, while sharing the same room capability.

3. **Single-device vs multi-device routing (what the agent does).**
   - **One machine connected:** call tools (`list_open_tabs`, `get_metadata`, `get_screenshot`, `get_design_context`, `reveal_node`, …) directly with no `session` argument. Even if several browser tabs are open on that same machine, the relay automatically routes to the focused tab (or newest ready tab on that device).
   - **Multiple machines connected:** an unpinned tool call without `session` returns JSON-RPC error `-32002` (`Several figstudio sessions are connected across devices; pass session or call select_session`), with `error.data.sessions` listing every connected session.

4. **Inspect and choose a session when multiple machines are connected.**
   - Call `list_sessions` (or read `error.data.sessions` from a `-32002` error). Each entry has:
     - `session`: `s_<32 lowercase hex>` (stable for the lifetime of that browser tab's instance)
     - `device`: `{"label": "<Device label>"}` (`device.id` is kept internal and never exposed)
     - `focused`: whether that browser tab currently has DOM focus
     - `docs`: array of `{"tab": <u64>, "name": "<file.fig>", "key": "<16 hex>", "pages": <usize>}` where `key` is the first 16 hex of SHA-256 over the 8-byte little-endian file size and the first 4 MiB of the file (identical when the same `.fig` is open on two machines)
     - `secondsSinceReady`: seconds since the tab's last state/focus update
   - **Pin for the whole MCP session:** call `select_session` with `{"session": "s_…"}`. The relay binds the pin to the caller's server-issued `Mcp-Session-Id` header (minted on `initialize`), so background reconnects or selection changes on the other machine will not steal subsequent calls. Pass `{"session": ""}` to clear the pin.
   - **Target a single call:** pass `"session": "s_…"` in any tool's `arguments` (for example `get_metadata` with `{"session": "s_…", "nodeId": "1:2"}`). The relay strips `session` before forwarding to the browser tab (unless a tool's native schema defines its own `session` property).
   - Every forwarded `tools/call` reply includes `result._meta["figstudio/session"]` and `result._meta["figstudio/device"]` so the caller can verify which machine answered.

5. **Revoke access if a room link is leaked.**
   - Click **Regenerate room token** under **View → Agent access**. This mints a new `ex_…` / `rm_…` pair, disconnecting the old room; update the MCP client URL and re-link any machines you keep.

## Pitfalls
- **Do not give the MCP client the `#room=…&exec=…` join link.** The MCP client only needs `https://<host>/mcp/rm_<64 hex>` (caller capability). The `#exec=ex_…` token is the executor capability for browsers joining the room.
- **`select_session` requires the `Mcp-Session-Id` header issued by `initialize`.** Standard Streamable HTTP MCP clients echo the `Mcp-Session-Id` response header from `initialize` automatically. If a custom script does not send `Mcp-Session-Id` (or sends a client-invented value not issued by `initialize`), `select_session` returns `isError: true`; pass `"session": "s_…"` on each tool call instead.
- **Pinned session disconnects do not silently switch machines.** If the browser tab you pinned with `select_session` is closed while another machine is still open in the room, the next tool call returns `-32002` (`Pinned session '…' is no longer connected`) with the remaining live sessions in `error.data.sessions`. Call `select_session` again (or clear it with `{"session": ""}`) before continuing.
- **Browser tab must stay open.** Files are never uploaded to Cloudflare; the Wasm worker inside the browser tab executes the MCP tools against the local `File` handle. If all browser tabs close, the relay returns HTTP 503 (`No browser tab connected`).
- **`compare_files` and `compare_page` are native-only.** They are not registered in the browser Wasm build.

## Verification
1. Run the Worker unit tests from the repo root:
   ```sh
   node --test web/worker/*.test.mjs
   ```
   Expect 39/39 passing tests covering `ready` v2 bounds, `sha256Hex`, `list_sessions` (with no `device.id` leak), single-device focus routing, multi-device `-32002` ambiguity refusal, `Mcp-Session-Id` minting on `initialize`, `select_session` isolation, and dead-pin refusal.
2. Live check against a connected room: `list_sessions` returns `sessions` with `session` matching `^s_[0-9a-f]{32}$`, `device.label`, and `docs[].key` matching `^[0-9a-f]{16}$` for open `.fig` files; every forwarded `tools/call` result contains `_meta["figstudio/session"]` and `_meta["figstudio/device"]`.
