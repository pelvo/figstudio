# figstudio-mcp

An offline [MCP](https://modelcontextprotocol.io) server for local Figma `.fig` files. It
answers the read tools of Figma's official MCP server from a file on disk, so a coding
agent (Claude Code, Cursor, VS Code, Codex) can work from a local copy without the Figma
app, a Figma account or a network connection.

- It **never writes the `.fig`**. The only files it writes are the Code Connect sidecar
  and the `get_design_context` asset PNGs (see [Writes](#writes)).
- The tool names and inputs follow the official server as researched on **2026-10-11**
  (37 tools at that date). The official surface changes often. Tool names here are pinned
  to that date.
- Tools that cannot work offline are **not registered**. See [Not replicated](#not-replicated).

## Install

On macOS (Apple silicon), Homebrew installs it together with the desktop app and
`figstudio-cli`, and links both binaries into `$(brew --prefix)/bin`, so `figstudio-mcp`
is on your `PATH`:

```sh
brew install --cask pelvo/tap/figstudio
```

On Linux (x86_64 or arm64), each release has a static tarball with `figstudio-mcp` and
`figstudio-cli`; see [Try it](../../README.md#try-it) for the download steps, which check it
against the release's `SHA256SUMS`.

Or install it with cargo (Rust 1.95 or newer):

```sh
cargo install --locked --git https://github.com/pelvo/figstudio figstudio-mcp
```

That puts `figstudio-mcp` in `~/.cargo/bin`. To give it to Claude Code with the public
browser app in one line (see [the browser proxy](#in-the-browser-local-zero-upload-proxy---browser-proxy)):

```sh
cargo install --locked --git https://github.com/pelvo/figstudio figstudio-mcp && claude mcp add figstudio -- figstudio-mcp --origin https://figstudio.pelvo.dev
```

## Build

```sh
cargo build --release -p figstudio-mcp
# binary: target/release/figstudio-mcp
```

Or download it: each GitHub release has `figstudio-cli-mcp-vX.Y.Z-macos-arm64.tar.gz`, and
the macOS app carries it at `figstudio.app/Contents/MacOS/figstudio-mcp`
(`scripts/macos-app.sh` builds that bundle locally).

Rust 1.95 or newer (see `AGENTS.md`). No Figma app or network is needed to run it.

## Run

```
figstudio-mcp [--http[=ADDR]] [--no-system-fonts] [--assets-dir DIR | --assets inline] [--max-output-bytes N] FILE.fig...
```

| Flag | Meaning |
|---|---|
| `FILE.fig...` | One or more files (required). They open lazily on first use and stay cached. |
| `--http` | Serve Streamable HTTP on `127.0.0.1:3845`, path `/mcp`; when 3845 is busy, the first free port of 3846–3855 (the chosen address is printed to stderr). Without it, the server talks over stdio. |
| `--http=ADDR` | Same, on exactly this address, for example `--http=127.0.0.1:4567`; a busy address is an error, never another port. The `=` is required, so `--http FILE.fig` is not read as an address. |
| `--no-system-fonts` | Do not use installed fonts when a text layer has no stored glyph outlines. |
| `--assets-dir DIR` | Where `get_design_context` writes its PNG assets. Default `./figma-assets`, resolved against the current directory. |
| `--assets inline` | Return those assets as image content instead of files. Cannot be combined with `--assets-dir`. |
| `--max-output-bytes N` | Byte cap of the text from `get_metadata`, `get_design_context` and `get_figjam` (default 24000, accepted 4000–100000). Claude Code delivered 20,166- and 25,810-character replies and rejected 44,674 and 74,213 characters, so the default stays under the delivered sizes. Raising it risks a rejected reply. |
| `--allow-host NAME` | With a non-loopback `--http=ADDR`: another `Host` name to accept (repeatable), for example a Tailscale name. The bound IP is accepted automatically. See [Remote agents](#remote-agents). |
| `--allow-any-interface` | Allow `--http=0.0.0.0:PORT` or `[::]:PORT`. Without it those are refused. |
| `--version`, `--help` | As usual. |

### Inside the figstudio app (live mode)

The desktop app can host this server with the files it has open. Turn it on in **Settings →
Agents** (⌘, / Ctrl+,; **View → Agent access…** opens the same tab) with **Agent access (MCP
server)**. On Linux and in the browser the menus are behind the **⋯** button at the top of the
sidebar; ⌘L / Ctrl+L finds them too. Desktop Agent access is off on first launch. Its
on/off state, **Allow remote agents** and bind address are saved between launches, so an
enabled server starts again automatically. Remote bearer tokens are not saved and change
when the server starts; copy the new token to your client after restarting. A startup or
runtime failure leaves the saved access setting enabled but stops that attempt. Turn access
off and on to retry, or restart the app once the address/port is available.

- While it runs, the status pill at the bottom left of the canvas shows `MCP 127.0.0.1:3845/mcp`. Point your client there.
  The app tries 3845, then 3846–3855, and shows the address it got.
- It is loopback only unless you tick **Allow remote agents** (see [Remote agents](#remote-agents)).
- Tabs are the files: `fileKey` is the file's stem (or its full path when two open files
  share a stem), as `list_open_tabs` shows.
- The app serves every tool of this server, plus the three live tools (below), on the open
  tabs. Assets come back inline, so the app writes no PNG files.

```sh
claude mcp add --transport http figstudio http://127.0.0.1:3845/mcp
```

### In the browser (bridge mode)

The browser app (`scripts/web-build.sh --serve`, or the deployed one) cannot host a server,
so a small local process does: `figstudio-mcp --bridge` serves `/mcp` for your client and
`/bridge`, a WebSocket the browser tab pairs with. The tab uploads each file it has open
(into a temporary directory that is deleted when the tab disconnects), reports its page,
selection and viewport, and applies `reveal_node`. All tools and the three live tools work.

```sh
figstudio-mcp --bridge          # prints the MCP URL, a pairing token and the web URL
claude mcp add --transport http figstudio http://127.0.0.1:3845/mcp
```

Open the printed `Local:` or `Web:` URL (`...#mcp=<token>`). The token travels in the URL
fragment, which the page's server never sees; or paste it in **Settings → Agents** (View →
Agent access…) and press Connect. The status pill shows `MCP connected`.

| Flag | Meaning |
|---|---|
| `--bridge` | Bridge mode; no `FILE.fig` needed. Default loopback; non-loopback needs the bearer token (see [Remote agents](#remote-agents)). `--http=ADDR` picks the address (default 3845, then 3846–3855). |
| `--allow-origin URL` | Another exact browser origin allowed to pair. Default: `http://127.0.0.1:8765`, `http://localhost:8765`. Allow a deployed site with `--allow-origin https://<your-deploy-domain>`; the banner then prints its `Web:` link. |
| `--max-upload-bytes N` | Largest upload (default 2 GiB); over it the bridge answers with an error and drops the file. |

Security: the bridge checks the `Origin` header against the list above and the random
pairing token (a wrong one closes the socket with 4401), so another web page cannot push
files at it. One browser connection at a time: a newer tab replaces the older one. Logs
show counts only, never names or contents.

Not verified: whether Safari or a newer Chrome blocks an `https://` page opening
`ws://127.0.0.1` (mixed content / Local Network Access prompt). If your browser blocks it,
use the local `--serve` build over `http://127.0.0.1:8765`.

### Deploying the browser app (two methods)

Build the browser bundle first with `scripts/web-build.sh` (output in `web/dist`, including
`web/dist/_headers` which sets `Cross-Origin-Opener-Policy: same-origin` and
`Cross-Origin-Embedder-Policy: require-corp` required for Wasm threads). You can deploy
`web/dist` in two ways depending on where your MCP client runs:

1. **Static-only deployment (for local `--browser-proxy` or `--bridge`):**
   - Serve `web/dist` from any static host or Cloudflare Workers static assets
     (`"assets": { "directory": "web/dist" }` in `wrangler.jsonc`, with no `main` Worker
     script or Durable Object needed).
   - If the site sits behind an access gateway (such as Cloudflare Access with Google
     sign-in), keep the whole origin protected with **no `/mcp/*` bypass**: your local agent
     spawns `figstudio-mcp --browser-proxy` over `stdio`, and your already-signed-in browser
     connects outbound to `ws://127.0.0.1:<port>/bridge`.
   - Nothing to configure for the app: once per page load it asks `GET /mcp/`, which only
     the relay Worker answers (`204` with a `figstudio-relay` header). Anywhere else that
     path is a missing asset, so **Settings → Agents** hides **Cloud relay** and opens no
     relay socket; **Local bridge** and `--browser-proxy` links work as usual. If the
     check itself fails (offline, a server error, no answer within 5 s), Cloud relay stays
     offered and **Connect** asks again.
2. **Cloud relay Worker deployment (for remote agents and multi-device HTTP MCP):**
   - Deploy `web/worker.mjs` and the `McpRoom` Durable Object alongside `web/dist` using
     the repo's `wrangler.jsonc` (`npx wrangler deploy`).
   - Browser tabs connect to `wss://<your-deploy-domain>/bridge/rm_<64 hex>` and MCP clients
     call `https://<your-deploy-domain>/mcp/rm_<64 hex>` directly over Streamable HTTP
     without running `figstudio-mcp` locally.

### In the browser (local zero-upload proxy, `--browser-proxy`)

When your coding agent and browser run on the same computer, `--browser-proxy` (implied
whenever `--origin` is passed without `FILE.fig` or `--bridge`) combines the zero-upload
Wasm RPC executor with a local `stdio` proxy inspired by `googlecolab/colab-mcp` — no `.fig`
upload, no Cloudflare Access bypass, and no manual token pasting.

**One-line install and setup (`colab-mcp` style):**

```sh
cargo install --locked --git https://github.com/pelvo/figstudio figstudio-mcp
claude mcp add figstudio -- figstudio-mcp --origin https://figstudio.pelvo.dev
```

`https://figstudio.pelvo.dev` is the public browser build (static, no cloud relay, no
sign-in). Use your own deploy's origin in its place if you run one.

Or in `.mcp.json` / Cursor / Windsurf:

```json
{
  "mcpServers": {
    "figstudio": {
      "command": "figstudio-mcp",
      "args": ["--origin", "https://figstudio.pelvo.dev"]
    }
  }
}
```

- `figstudio-mcp --origin https://figstudio.pelvo.dev` (or `figstudio-mcp --browser-proxy`
  for a local `http://127.0.0.1:8765` web build; without `--origin` that local address is
  what `open_figstudio_browser` opens) speaks `stdio` to the agent, binds
  `ws://127.0.0.1:<port>/bridge` on loopback (trying `3845..=3855` first, then an ephemeral
  port), and mints a 256-bit process token (never printed to `stderr`).
- When the agent calls `open_figstudio_browser`, the proxy opens the first `--origin` as
  `https://figstudio.pelvo.dev/#mcpProxyPort=<port>&mcpProxyToken=<token>` (your own
  deploy's origin if you passed that instead) in your default browser (`$BROWSER`, `open`,
  `xdg-open`, or `rundll32 url.dll,FileProtocolHandler`). The page strips the `#fragment`
  immediately, and the proxy waits up to 60 s for the tab's `ready` v2 frame before emitting
  `notifications/tools/list_changed`.
- **Safari cannot pair with an `https://` origin:** WebKit blocks `ws://127.0.0.1` from
  `https://` pages as mixed content (Chromium, Edge and Firefox allow loopback). With
  Safari as the default browser, set `BROWSER` to another one, for example
  `claude mcp add figstudio -e 'BROWSER=open -a "Google Chrome"' -- figstudio-mcp --origin https://figstudio.pelvo.dev`,
  or use `--browser-proxy` with the local `http://127.0.0.1:8765` build.
- `list_sessions` and `select_session` work identically when multiple tabs connect.

### In the browser (cloud relay and multi-device sessions)

When deployed behind the Cloudflare Worker (`web/worker.mjs` with the `McpRoom` Durable
Object), the browser tab runs the MCP server in Wasm and pairs with `/bridge/rm_<64 hex>`
over WebSocket. Files never leave the browser.

1. **First machine (one-time MCP client setup):** open **Settings → Agents** (View → Agent access…), keep
   **Cloud relay** selected, optionally set **Device label** (for example `Office · Chrome`),
   and click **Copy MCP URL**:
   ```sh
   claude mcp add --transport http figstudio https://<your-deploy-domain>/mcp/rm_<64 hex>
   ```
   The 256-bit executor token (`ex_<64 hex>`) is saved in
   `localStorage["figstudio.mcp.executor.v2"]`; the room (`rm_<64 hex>`) is derived from it
   by SHA-256, so the MCP URL stays the same across reloads and files.
2. **Additional machines (same MCP URL, no extra tokens):** on the first machine click
   **Copy room link** (`https://<your-deploy-domain>/#room=rm_…&exec=ex_…`), open that link
   on the second machine, and confirm **Join that room** (or paste `ex_…` and click
   **Apply**). Both machines now share the same `/mcp/rm_…` endpoint while keeping their own
   `device.id` (`localStorage["figstudio.mcp.device.v1"]`) and **Device label**.
3. **Routing across sessions:**
   - When only **one device** is connected (even with multiple tabs), tool calls route
     automatically to its focused or newest tab.
   - When **multiple devices** are connected to the same room, an unpinned call without
     `session` returns JSON-RPC error `-32002` with the candidate sessions in `error.data`.
   - Call `list_sessions` to inspect connected devices and open `.fig` documents (`key` is
     the first 16 hex of SHA-256 over the file size and first 4 MiB), then either pin a
     target for the whole MCP client session with `select_session` (`{"session": "s_…"}`,
     keyed by the server-issued `Mcp-Session-Id` header from `initialize`) or pass
     `"session": "s_…"` on individual tool calls.
   - Every forwarded `tools/call` result carries `_meta["figstudio/session"]` and
     `_meta["figstudio/device"]`.
   - Unreloaded legacy (protocol 1) tabs each count as a separate session until reloaded
     onto protocol 2.
   - See the [cloud-relay-sessions agent skill](../../.agents/skills/cloud-relay-sessions/SKILL.md)
     for the step-by-step agent workflow.

### Remote agents

Use this when the coding agent runs on another machine than figstudio (for example an SSH
box) and the app or `figstudio-mcp --bridge` runs on your PC. Nothing changes unless you ask.

**Tailscale (token).** Bind your Tailscale address; the server generates a random bearer
token each start and refuses any request without it (401, constant-time compare). `Host`
must be the bound IP or a name you allow.

```sh
figstudio-mcp --http=100.64.0.1:3845 --allow-host mypc.tailnet.ts.net design.fig   # prints the token
figstudio-mcp --bridge --http=100.64.0.1:3845                                      # browser bridge, same rules
claude mcp add --transport http figstudio http://100.64.0.1:3845/mcp \
  --header "Authorization: Bearer <token>"
```

A remote `--bridge` also listens on `127.0.0.1` at the same port, so the browser on your PC
pairs with `ws://127.0.0.1:<port>/bridge` (the banner prints it). The browser never needs the
remote address. That loopback listener serves `/mcp` without a token, as on any loopback bind.

With `--allow-any-interface` the server accepts any address on every interface, but it cannot
know your LAN or Tailscale addresses: a client that sends one as `Host` gets 403 even with the
token. Name each one with `--allow-host` (for example `--allow-host 100.64.0.1` or a MagicDNS
name). The banner prints this when no `--allow-host` is given.

In the desktop app tick **Settings → Agents → Allow remote agents**, type the address and
port, then turn Agent access on; the status pill has a **Copy token** button. The setting is
not saved. `0.0.0.0` and `::` are refused (`--allow-any-interface` overrides on the CLI, the
app never listens on every interface). In bridge mode `/bridge` keeps its own Origin and
pairing-token checks.

**SSH tunnel (no token, nothing exposed).** Keep the default loopback bind and forward the
port from your PC to the agent's machine:

```sh
ssh -R 3845:127.0.0.1:3845 agent-box     # run on your PC
claude mcp add --transport http figstudio http://127.0.0.1:3845/mcp   # run on agent-box
```

The connection is plain HTTP: Tailscale (WireGuard) encrypts it, but on any other network
use the tunnel. TLS is not provided.

### stdio

The default. The client starts the binary and speaks JSON-RPC lines over stdin/stdout.
Stdout carries protocol messages only; diagnostics go to stderr. Ctrl-C and SIGTERM stop
the server.

### Streamable HTTP

`--http` serves the MCP Streamable HTTP transport at `http://127.0.0.1:3845/mcp`. That is
the address of the Figma desktop app's local server, so a client config that points at it
works unchanged.

- Protocol versions 2025-03-26, 2025-06-18, 2025-11-25 and 2026-07-28 are accepted (the
  SDK, `rmcp` 3.5, negotiates them).
- The legacy 2024-11-05 HTTP+SSE-only transport is **not** supported.
- The server only accepts loopback `Host` headers (more only on a token-guarded remote bind).
  It also checks `Origin`: requests with no `Origin` (CLI and native clients) pass,
  `http://localhost:*`, `http://127.0.0.1:*` and `http://[::1]:*` pass, and any other origin,
  including `null`, gets 403. It is a local tool; see [Remote agents](#remote-agents) for the opt-in exception.
- All HTTP sessions share one file cache.

**Security (both servers).**

- By default both bind loopback with no token. A non-loopback bind is opt-in and always
  requires a bearer token (below).
- `Host` must be loopback (or one allowed on a remote bind) and `Origin` (when sent) must be
  loopback. Anything else is 403.
- No CORS: no reply carries `Access-Control-Allow-Origin`, and a preflight from another
  origin is refused. A web page cannot read the server.

## Client setup

Use your own paths in place of `/path/to/...`.

**Claude Code**

```sh
# stdio: Claude starts the server
claude mcp add figstudio -- /path/to/figstudio-mcp /path/to/design.fig
# HTTP: start the server yourself first (figstudio-mcp --http design.fig)
claude mcp add --transport http figstudio http://127.0.0.1:3845/mcp
```

**Cursor** (`.cursor/mcp.json`) and **VS Code** (`.vscode/mcp.json`; the top-level key is
`servers` there, not `mcpServers`)

```json
{
  "mcpServers": {
    "figstudio": {
      "command": "/path/to/figstudio-mcp",
      "args": ["/path/to/design.fig"]
    }
  }
}
```

HTTP form:

```json
{
  "mcpServers": {
    "figstudio": { "url": "http://127.0.0.1:3845/mcp" }
  }
}
```

**Codex** (`~/.codex/config.toml`)

```toml
[mcp_servers.figstudio]
command = "/path/to/figstudio-mcp"
args = ["/path/to/design.fig"]
```

For the browser proxy instead of a file, replace the arguments with
`["--origin", "https://figstudio.pelvo.dev"]` (Codex: `args = ["--origin", "https://figstudio.pelvo.dev"]`).

These examples are written from each client's documented config format. With Claude Code
2.1.304, `claude mcp add -s project figstudio -- figstudio-mcp --origin https://figstudio.pelvo.dev`
writes this `.mcp.json` entry (without `-s project`, the default `local` scope stores the
same entry privately for that project):

```json
{
  "mcpServers": {
    "figstudio": {
      "type": "stdio",
      "command": "figstudio-mcp",
      "args": ["--origin", "https://figstudio.pelvo.dev"],
      "env": {}
    }
  }
}
```

No client has been run against the server end to end yet.

## Files and IDs

**`fileKey`.** Optional when one file is registered. Otherwise pass the file's stem
(`design` for `design.fig`) or its full path as listed by `list_files`. Two files with
the same stem need the full path. A missing, unknown or ambiguous key is a tool error.

**`nodeId`.** Every tool that takes one accepts:

| Form | Example |
|---|---|
| Colon | `1:2` |
| Dash | `1-2` |
| Figma URL with `node-id` | `https://www.figma.com/design/KEY/Name?node-id=1-2` (host `figma.com` or `www.figma.com`; exactly one `node-id`) |
| Instance path | `I1:2;3:4` |

An instance path names a node *inside* an instance, with that instance's overrides
applied. `get_metadata` prints such ids for the children of every instance; the other
tools accept them.

**Links (`figstudio://`).** The app's Copy menus put these on the clipboard. A node link
works wherever a `nodeId` is taken, and `reveal_node` takes one too. A region link works in
`get_screenshot` (`nodeId`) and `get_nodes_in_region` (`region`); other tools reject it:

| Form | Example |
|---|---|
| Node | `figstudio://structure?node-id=1-42` |
| Region | `figstudio://structure?page=0-1&rect=0,0,400,300` |

- The host is the file's key, percent-encoded. A file stem such as `structure` is the usual
  key. Exactly one of `node-id` or `page` + `rect` is allowed; `rect` is `x,y,w,h` with
  width and height above 0.
- A `figma.com` link (`/design`, `/file` or `/board`, host `figma.com` or `www.figma.com`)
  also works.
- **File choice** (`fileKey` argument first, then the link):
  - an explicit `fileKey` wins;
  - a `figstudio://` link names its file, and an unknown key is a tool error;
  - a figma.com link names a file when its key equals a file's stem, or else its slug
    equals the stem with spaces turned into `-` (compared as is, then without a browser's
    ` (1)` suffix on a repeated download; when two tabs share a stem, their full-path keys
    are matched by stem); a key match beats a name match, and when several files tie (say
    `AbC123` and `AbC123 (1)`) the call fails and asks for `fileKey` rather than guess;
  - otherwise the link's file part is ignored and the usual rule applies: the single file,
    or the active tab in live mode.

**Selection defaults (live mode only).** In the app, a missing `nodeId` falls back to the
node selected in the tab, for `get_design_context`, `get_screenshot`, `get_variable_defs`,
`download_assets` (when `nodeIds` is empty) and `get_context_for_code_connect`. With nothing
selected the reply is `nodeId is required: nothing is selected in this tab`. Standalone, a
missing `nodeId` is `nodeId is required`. `get_metadata` with no `nodeId` still lists the
pages.

## Tools

16 tools and 1 prompt, plus 3 live tools in the app. "Official" means Figma's server;
"differs offline" lists what a client should expect to be different. The two
`compare_*` tools are native only (the diff crate renders through the native
renderer), so the browser MCP build does not register them.

| Tool | Inputs | Output | Differs offline / limits |
|---|---|---|---|
| `whoami` | none | `{mode:"offline", server, files}` | Stub: no account. |
| `list_files` | none | `{files:[{key,path,pages}]}` | Not in the official server. `pages` is null until the file is first opened. |
| `get_metadata` | `nodeId?`, `fileKey?`, `depth?` | Sparse XML: `id name x y width height`, parent-relative, rotation in degrees (counter-clockwise positive). No `nodeId` lists the pages. | Instances expand with overrides applied (`I…;…` ids, `componentId`). Max 5000 elements and 24 kB of XML by default (`--max-output-bytes`) (`truncated="true"` when cut; call again with a child `nodeId` or a smaller `depth`). Text cut at 200 characters plus `…`. An XML comment counts instances that could not be expanded. |
| `get_screenshot` | `nodeId`, `fileKey?`, `scale?` | PNG image plus a text line `WxH px, scale S` and an `approximations:` line | `scale` 0.01–4 (default 1). Without `scale` the longest side is capped at 2048 px (about 1–2 MB PNGs, under Claude's 1568 px downscale threshold and the 5 MB base64 limit); with an explicit `scale` at 4096 px (3.1–3.7 MB PNGs measured). Cropped to the node's own box, so shadows and strokes outside it are cut. Transparent background (pages use the page colour). An instance path renders that node alone, as laid out inside its instance; ancestor masks and clips are not applied. A `figstudio://…?page=…&rect=…` region link in `nodeId` renders that rectangle of the page on the page colour, with the same scale and size caps. |
| `get_nodes_in_region` | `region?` or `page?` + `rect?`, `fileKey?`, `limit?` | JSON `{page, rect, nodes: [{nodeId, name, kind}], total, truncated}` | Visible nodes of a page whose box overlaps the rectangle, top-level first. `region` is a `figstudio://<key>?page=0-1&rect=x,y,w,h` link as the app's Copy region gives; `rect` is `[x, y, w, h]` in page units. `limit` 1–500, default 50. A bad rect, an unknown page or both forms at once is a tool error. |
| `get_variable_defs` | `nodeId`, `fileKey?` | Flat JSON map `name → value` of the variables and styles used under the node | All modes (extra modes as `name (Mode)`), aliases resolved, instances walked with overrides. Reads effective node data only, not component property storage. Colours `#RRGGBB(AA)`. |
| `get_design_context` | `nodeId`, `fileKey?`, `clientFrameworks?`, `clientLanguages?`, `maxNodes?` | React + Tailwind JSX (`data-node-id`, `data-name` on every element), agent instructions, an asset list and an `approximations:` line | Always React + Tailwind; the client hints are only echoed. No shader runtime. Vectors, boolean operations and image-filled leaves become PNG `<img>` assets at scale 2. Caps: `maxNodes` default 1000, at most 5000; 24 000 bytes of code by default (`--max-output-bytes`); 30 assets; 4 MB of inline images. Past a cap the output is cut with a note: call `get_metadata`, then this tool on children. |
| `download_assets` | `nodeIds` (1–20), `fileKey?`, `format?` (`png`\|`jpg`), `scale?`, `original?` | One text line plus one image per node. With `original=true`, the stored bytes of the image fills inside each node. | No SVG or PDF. JPG (quality 90) is flattened onto white. Max 12 originals per call (`MAX_ORIGINALS`: Claude Code cuts a reply at 25k tokens and each image costs about 1.5k), each image once. A reply holds at most 10 MiB of image data (`MAX_PAYLOAD_BYTES`, the stdio line limit) and no image over 3.75 MB raw (`MAX_IMAGE_BYTES`: 5 MB of base64, the per-image limit on Bedrock and Vertex); a larger one is reported with its size and not sent. The text lines come before the images in every reply. `get_figjam` `includeImages` shares the 10 MiB budget. One node failing does not fail the call. |
| `get_figjam` | `nodeId?`, `fileKey?`, `depth?`, `includeImages?`, `imageScale?` | `get_metadata` XML plus FigJam details: sticky text and colour, shape text, connector ends, section status, table and code-block text, widget and stamp names | The renderer does not draw FigJam kinds yet: `includeImages` shows them as plain boxes (max 20 images, `imageScale` 0.01–0.5, default 0.25), and an XML comment counts those kinds. Field shapes come from the schema, not a real FigJam file. |
| `get_context_for_code_connect` | `nodeId`, `fileKey?` | JSON: component properties, variants (component sets), nested instances | Component, component set, instance or instance path. Property lists capped at 100; unknown property types are kept as strings. |
| `get_code_connect_map` | `nodeId?`, `fileKey?`, `clientFrameworks?`, `clientLanguages?` | JSON map of `nodeId → mapping` | Reads the local sidecar, not Figma's cloud. Lookup falls back instance → component → component set; the requested id is the key. Framework filters match labels; language filters match source extensions or labels. |
| `add_code_connect_map` | `nodeId`, `componentName`, `source`, `label`, `snippet?`, `snippetImports?` | The stored mapping | Writes the sidecar atomically. An instance is stored under its component's id. `source` is stored, never fetched. Limits: source 4096 bytes, snippet 100 000 bytes, 100 imports of 4096 bytes. `imports` is accepted as an alias of `snippetImports`. |
| `search_design_system` | `query`, `kinds?`, `fileKey?` | Ranked matches | This file only (no team libraries). `kinds`: `component`, `component_set`, `style`, `variable`. Exact names first; at most 100 results. |
| `get_libraries` | `fileKey?` | Summary of this file's components, token collections, modes and styles | Local only; lists capped at 100, with totals. |
| `compare_files` | `oldFile?`, `newFile?`, `includeRender?` (default false) | JSON `{old, new, summary, complete, truncated, pages, pagesTotal, pagesTruncated, reportJson, reportHtml}` | Writes `report.json`/`report.html` (plus `render/<guid>-{before,after,diff}.png` with `includeRender`) under `<assets-dir>/compare/compare-<random>/` (`$TMPDIR/figstudio-compare/compare-<random>/` in inline mode). At most 100 page entries inline; rendering is capped by the diff crate. Not an official tool; native server only. |
| `compare_page` | `oldFile?`, `newFile?`, `pageId` | One page's `PageDiff` JSON capped at 64 KiB; whole entries omitted as needed, with per-list `Total`/`Omitted` counts and a `truncated` flag | Re-runs the comparison (documents are cached); the report is not cached. MCP schema Guids are `session:local` strings; disk `report.json` keeps native Guid arrays and full entries. When the inline page JSON is over 64 KiB, an `old_name` or `new_name` longer than 128 bytes is shortened to at most 128 bytes, ending in `~`, before any row is dropped; that cut sets `nameTruncated: true`. An unchanged or unknown page is a tool error. Not an official tool; native server only. |
| `list_open_tabs` *(app only)* | none | `{tabs: [{tabId, fileKey, path, title, page: {id, name}, selection, active}]}` | Lists the app's open tabs. `fileKey` is what the other tools take. |
| `get_selection` *(app only)* | `fileKey?` | `{fileKey, tabId, page, node: {nodeId, name, kind, bounds: {x, y, width, height}}}`, or `node: null` | The selected node of a tab (default the active tab). Bounds are in page units. |
| `reveal_node` *(app only)* | `nodeId` (or a link), `fileKey?` | One line: the node is revealed in tab N | Selects the node in the app; the tab becomes active and the view fits the node. A page ID switches to that page. An instance path reveals its outer instance and says so. Waits 5 s for the app; then the error "the app did not respond" (a minimised window may not run frames, so the reveal can time out). A figma.com link (without `fileKey`) is shown only in the file it names, never in some other active tab. The desktop app resolves it: the matching open tab, or else the local copy named after the link (`<file name>.fig`) in its link folders (`FIGSTUDIO_LINK_DIRS`, a path list, else `~/Downloads`; or the folder chosen in File → Open Figma link…), which it opens first (`revealed 1:2 in tab 3 (opened the local copy)`); it waits up to 60 s for that. When several files match equally (two documents can share a name), the call fails and asks for a pick in the app. The node ID is checked first, so a link without a valid `node-id` never opens a file. Replies about links carry no paths or file names. In the browser bridge, which cannot open local copies, the link must name exactly one open tab. |
| `list_sessions` *(cloud relay only)* | none | `{sessions: [{session, device: {label}, focused, docs: [{tab, name, fileKey, key, pages}], secondsSinceReady}], selectedSession, socketsInUse, socketCap}` | Answered by the relay Durable Object even when no tab is connected. `docs[].fileKey` is the tool `fileKey` argument; `docs[].key` is the first 16 hex of SHA-256 over the 8-byte little-endian file size and first 4 MiB. `device.id` is never exposed. |
| `select_session` *(cloud relay / browser proxy)* | `session` (`s_…` or `""` to clear) | `{selected: <sessionSummary>}` or `{cleared: true, ...}` | Pins a target session in memory for subsequent tool calls from this MCP client (keyed by the `Mcp-Session-Id` header issued on `initialize`, capped at 64 clients). If the pinned tab disconnects, subsequent calls return `-32002`; if the isolate restarts or evicts the `Mcp-Session-Id`, requests carrying the stale header return HTTP 404 (`-32002`) so the client re-initializes instead of falling through to another device. |
| `open_figstudio_browser` *(`--browser-proxy` only)* | `timeoutSeconds?` (1–120, default 60) | `{connected, launched, sessions}` | Opens `<origin>/#mcpProxyPort=<port>&mcpProxyToken=<token>` in the system browser if no tab is connected yet and waits for the tab to send `ready` v2. |

### Comparing versions

The [compare-fig-versions agent skill](../../.agents/skills/compare-fig-versions/SKILL.md)
covers CLI reports and the native MCP compare tools. From the repo root, after building
the CLI, run `target/release/figstudio-cli compare OLD.fig NEW.fig -o OUT_DIR`
(use a fresh output folder). Claude Code reads `.claude/skills/`; users can symlink
or copy `.agents/skills/compare-fig-versions` there.

**Prompt: `create_design_system_rules`** (`prompts/get`; optional arguments
`clientFrameworks`, `clientLanguages`, `fileKey`, all strings). Returns guidance for a
design-system rules file, plus the file's token summary and component list (names cut at
200 characters). It writes no files.

Tool errors (bad id, unknown file, missing node) come back as results with
`isError: true`, not as protocol failures. `nodeId` is optional in the schemas of
`get_screenshot`, `get_variable_defs`, `get_design_context` and `get_context_for_code_connect`
(and `nodeIds` of `download_assets`) because the app's live mode fills it in from the
selection; this server, run on its own, answers a call without it with the tool error
`nodeId is required` (`download_assets`: `nodeIds must hold 1 to 20 ids`).

## Not replicated

These official tools are not registered. Registering them only to fail would waste the
client's tool budget.

| Tool | Why not |
|---|---|
| `use_figma`, `create_new_file`, `generate_figma_design` | They write to Figma's canvas or cloud. This server is read-only. |
| `generate_image` | Needs a hosted image model. |
| shader tools, generative-plugin tools | Need Figma's plugin runtime or cloud. |
| `weave_*` | Figma Weave is a cloud product. |
| `get_code_connect_suggestions` | Needs Figma's cloud. |
| `get_motion_context` | Deferred, not impossible. No public file with motion data exists to validate the schema against. |

SVG and PDF export are also unavailable (the renderer produces pixels).

## Privacy

By design this server returns layer names, text, colours and image bytes from your file
to your local client. That is its job. It is separate from `figstudio-cli doctor`, whose
output is meant to be pasted in public and must never contain such content. Do not paste
MCP output from a private file into a public place.

## Writes

- **Code Connect sidecar:** `add_code_connect_map` writes `<file>.fig.codeconnect.json`
  beside the registered file (atomic replace, existing permissions kept, new files 0644
  on Unix). A symlink at that path or a sidecar over 10 MiB is refused. Each add holds an
  exclusive lock on the sibling `<file>.fig.codeconnect.json.lock` (an advisory
  `File::lock`, kept between runs, never deleted) from before it reads the sidecar until
  it has written it, so adds from several server processes do not lose each other's
  mappings. Other tools that edit the sidecar must take the same lock. A symlink at the
  lock path is refused.
- **Asset PNGs:** `get_design_context` writes `<fileKey>_<id>.png` files into
  `--assets-dir` (or returns them inline with `--assets inline`).
- **Comparison artifacts:** `compare_files` writes `report.json`, `report.html`
  and, with `includeRender`, `render/<guid>-{before,after,diff}.png` under
  `<assets-dir>/compare/compare-<random>/` (or
  `$TMPDIR/figstudio-compare/compare-<random>/` in inline mode).
- The `.fig` itself is never opened for writing.
- **Cache:** a file is decoded once and kept. Changing it on disk is not noticed;
  restart the server to read the new version.

## Known limits

Things that are approximated or not checked. Where a tool knows, its reply says so in an
`approximations:` line or an XML comment.

- **Not verified.** No live Claude Code, Cursor, VS Code or Codex session, no capture of the
  official server's output (shapes follow its documented descriptions), no private or
  very large files, no Windows run (signals, file replacement), no pixel comparison for
  `get_screenshot`.
- **Rendering** is figstudio's renderer, so its approximations apply (see
  `docs/APPROXIMATIONS.md`). Text without stored glyph outlines uses installed fonts, so
  line breaks may differ from Figma.
- **Instance children** rendered alone (`I…;…`) do not apply ancestor masks or clips.
  `download_assets original=true` judges frames between nested instances by their stored
  visibility only.
- **`get_design_context`** is a reference, not pixel-exact code: image fills behind
  children are dropped, mirrored layers and grid stacks are approximated (all counted), and
  fields that differ between a component root and an instance only through
  `derivedSymbolData` take the instance's own value.
- **Big files** (over 256 MiB) open lazily: images are read from the file when a render or
  `original=true` needs them. An original over the 3.75 MB per-image limit, or bigger than
  what is left of the 10 MiB reply budget, is omitted without being read.
- **FigJam:** connector geometry beyond loose-end coordinates, table row/column structure,
  merged cells and sticky text style are not reported.
- **Code Connect and design system** cover this file only. No remote libraries.
- **Big files:** the whole node tree is held in memory (several GB for 50 MB+ files). Every
  text tool has an output cap; time and size on huge pages were not measured.

## Tests

See `docs/TESTING.md` section "MCP server". Run `scripts/check.sh` for the full gate.
