# figstudio-mcp

An offline [MCP](https://modelcontextprotocol.io) server for Figma `.fig` local copies, from
[figstudio](https://github.com/pelvo/figstudio). This package runs the prebuilt binary for
your computer (Linux x64 and arm64, macOS arm64 and x64, Windows x64); nothing is compiled
and no install script runs.

## Claude Code

```sh
claude mcp add figstudio -- npx -y figstudio-mcp --browser-proxy
```

When the agent first needs a file it opens the figstudio.pelvo.dev app in your browser,
paired with this local server; open your `.fig` in that tab (Figma → File → Save local
copy). The file stays in the tab and nothing is uploaded, as long as figstudio.pelvo.dev
serves figstudio's own build: that page's code reads your file, so you trust that
deployment (to keep everything local, use the local build below).

This server fetches the app from figstudio.pelvo.dev and serves it on its own
`http://127.0.0.1:<port>`, so the page reaches the server without Chrome's local network
access prompt, which a headless browser cannot answer. With `--no-mirror` it opens
https://figstudio.pelvo.dev itself: Chrome then asks to let the page reach devices on your
local network, and Safari cannot pair (it blocks an `https://` page from reaching
`ws://127.0.0.1`).

Or, from a [figstudio](https://github.com/pelvo/figstudio) checkout, serve the local build
with `scripts/web-build.sh --serve` and pair with it over plain `http`:

```sh
claude mcp add figstudio -- npx -y figstudio-mcp --origin http://127.0.0.1:8765
```

To serve files from disk instead, with no browser:

```sh
claude mcp add figstudio -- npx -y figstudio-mcp /path/to/design.fig
```

**Also works with `bunx`:** `bunx figstudio-mcp --browser-proxy` runs the same launcher.
`bin/figstudio-mcp.js` only uses Node's built-in `fs`/`os`/`child_process` modules (no
npx-only syntax), and `bun install` resolves the matching `@pelvo/figstudio-mcp-<os>-<cpu>`
optional dependency the same way npm does.

## Cursor, VS Code, Codex

`.cursor/mcp.json` (VS Code: `.vscode/mcp.json`, with `servers` as the top-level key):

```json
{
  "mcpServers": {
    "figstudio": { "command": "npx", "args": ["-y", "figstudio-mcp", "--browser-proxy"] }
  }
}
```

`~/.codex/config.toml`:

```toml
[mcp_servers.figstudio]
command = "npx"
args = ["-y", "figstudio-mcp", "--browser-proxy"]
```

Every flag and tool is documented in
[apps/figstudio-mcp/README.md](https://github.com/pelvo/figstudio/blob/main/apps/figstudio-mcp/README.md).
Licensed under Apache-2.0.
