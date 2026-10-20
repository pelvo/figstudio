# figstudio-mcp

An offline [MCP](https://modelcontextprotocol.io) server for Figma `.fig` local copies, from
[figstudio](https://github.com/pelvo/figstudio). This package runs the prebuilt binary for
your computer (Linux x64 and arm64, macOS arm64 and x64, Windows x64); nothing is compiled
and no install script runs.

## Claude Code

```sh
claude mcp add figstudio -- npx -y figstudio-mcp --browser-proxy
```

When the agent first needs a file it opens https://figstudio.pelvo.dev in your browser,
paired with this local server; open your `.fig` in that tab (Figma → File → Save local
copy). The file stays in the tab and nothing is uploaded. If the browser asks to let the
page reach devices on your local network, allow it: that is this server on `127.0.0.1`.

**Safari cannot pair:** it blocks an `https://` page from reaching `ws://127.0.0.1`
(Chrome, Edge and Firefox allow it). If Safari is your default browser, have this server
open another one, for example Chrome (`open -a "Microsoft Edge"` or `open -a Firefox` for
those):

```sh
claude mcp add figstudio -e 'BROWSER=open -a "Google Chrome"' -- npx -y figstudio-mcp --browser-proxy
```

Or, from a [figstudio](https://github.com/pelvo/figstudio) checkout, serve the local build
with `scripts/web-build.sh --serve` and pair with it over plain `http`:

```sh
claude mcp add figstudio -- npx -y figstudio-mcp --origin http://127.0.0.1:8765
```

To serve files from disk instead, with no browser:

```sh
claude mcp add figstudio -- npx -y figstudio-mcp /path/to/design.fig
```

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
