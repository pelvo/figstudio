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

## Build

```sh
cargo build --release -p figstudio-mcp
# binary: target/release/figstudio-mcp
```

Rust 1.95 or newer (see `AGENTS.md`). No Figma app or network is needed to run it.

## Run

```
figstudio-mcp [--http[=ADDR]] [--no-system-fonts] [--assets-dir DIR | --assets inline] FILE.fig...
```

| Flag | Meaning |
|---|---|
| `FILE.fig...` | One or more files (required). They open lazily on first use and stay cached. |
| `--http` | Serve Streamable HTTP on `127.0.0.1:3845`, path `/mcp`. Without it, the server talks over stdio. |
| `--http=ADDR` | Same, on a custom address, for example `--http=127.0.0.1:4567`. The `=` is required, so `--http FILE.fig` is not read as an address. |
| `--no-system-fonts` | Do not use installed fonts when a text layer has no stored glyph outlines. |
| `--assets-dir DIR` | Where `get_design_context` writes its PNG assets. Default `./figma-assets`, resolved against the current directory. |
| `--assets inline` | Return those assets as image content instead of files. Cannot be combined with `--assets-dir`. |
| `--version`, `--help` | As usual. |

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
- The server only accepts loopback `Host` headers. Binding to a non-loopback address
  prints a warning, and remote clients are still rejected. It is a local tool.
- All HTTP sessions share one file cache.

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

These examples are written from each client's documented config format. No live client
has been tested against this server yet.

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
tools accept them. There is no "current selection" offline, so `nodeId` is always
explicit.

## Tools

13 tools and 1 prompt. "Official" means Figma's server; "differs offline" lists what a
client should expect to be different.

| Tool | Inputs | Output | Differs offline / limits |
|---|---|---|---|
| `whoami` | none | `{mode:"offline", server, files}` | Stub: no account. |
| `list_files` | none | `{files:[{key,path,pages}]}` | Not in the official server. `pages` is null until the file is first opened. |
| `get_metadata` | `nodeId?`, `fileKey?`, `depth?` | Sparse XML: `id name x y width height`, parent-relative, rotation in degrees (counter-clockwise positive). No `nodeId` lists the pages. | Instances expand with overrides applied (`I…;…` ids, `componentId`). Max 5000 elements (`truncated="true"` when cut). Text cut at 200 characters plus `…`. An XML comment counts instances that could not be expanded. |
| `get_screenshot` | `nodeId`, `fileKey?`, `scale?` | PNG image plus a text line `WxH px, scale S` and an `approximations:` line | `scale` 0.01–4 (default 1); longest side capped at 4096 px. Cropped to the node's own box, so shadows and strokes outside it are cut. Transparent background (pages use the page colour). An instance path renders that node alone, as laid out inside its instance; ancestor masks and clips are not applied. |
| `get_variable_defs` | `nodeId`, `fileKey?` | Flat JSON map `name → value` of the variables and styles used under the node | All modes (extra modes as `name (Mode)`), aliases resolved, instances walked with overrides. Reads effective node data only, not component property storage. Colours `#RRGGBB(AA)`. |
| `get_design_context` | `nodeId`, `fileKey?`, `clientFrameworks?`, `clientLanguages?`, `maxNodes?` | React + Tailwind JSX (`data-node-id`, `data-name` on every element), agent instructions, an asset list and an `approximations:` line | Always React + Tailwind; the client hints are only echoed. No shader runtime. Vectors, boolean operations and image-filled leaves become PNG `<img>` assets at scale 2. Caps: `maxNodes` default 1000, at most 5000; 100 000 bytes of code; 30 assets; 4 MB of inline images. Past a cap the output is cut with a note: call `get_metadata`, then this tool on children. |
| `download_assets` | `nodeIds` (1–20), `fileKey?`, `format?` (`png`\|`jpg`), `scale?`, `original?` | One text line plus one image per node. With `original=true`, the stored bytes of the image fills inside each node. | No SVG or PDF. JPG (quality 90) is flattened onto white. Max 20 originals per call, each image once. One node failing does not fail the call. |
| `get_figjam` | `nodeId?`, `fileKey?`, `depth?`, `includeImages?`, `imageScale?` | `get_metadata` XML plus FigJam details: sticky text and colour, shape text, connector ends, section status, table and code-block text, widget and stamp names | The renderer does not draw FigJam kinds yet: `includeImages` shows them as plain boxes (max 20 images, `imageScale` 0.01–0.5, default 0.25), and an XML comment counts those kinds. Field shapes come from the schema, not a real FigJam file. |
| `get_context_for_code_connect` | `nodeId`, `fileKey?` | JSON: component properties, variants (component sets), nested instances | Component, component set, instance or instance path. Property lists capped at 100; unknown property types are kept as strings. |
| `get_code_connect_map` | `nodeId?`, `fileKey?`, `clientFrameworks?`, `clientLanguages?` | JSON map of `nodeId → mapping` | Reads the local sidecar, not Figma's cloud. Lookup falls back instance → component → component set; the requested id is the key. Framework filters match labels; language filters match source extensions or labels. |
| `add_code_connect_map` | `nodeId`, `componentName`, `source`, `label`, `snippet?`, `snippetImports?` | The stored mapping | Writes the sidecar atomically. An instance is stored under its component's id. `source` is stored, never fetched. Limits: source 4096 bytes, snippet 100 000 bytes, 100 imports of 4096 bytes. `imports` is accepted as an alias of `snippetImports`. |
| `search_design_system` | `query`, `kinds?`, `fileKey?` | Ranked matches | This file only (no team libraries). `kinds`: `component`, `component_set`, `style`, `variable`. Exact names first; at most 100 results. |
| `get_libraries` | `fileKey?` | Summary of this file's components, token collections, modes and styles | Local only; lists capped at 100, with totals. |

**Prompt: `create_design_system_rules`** (`prompts/get`; optional arguments
`clientFrameworks`, `clientLanguages`, `fileKey`, all strings). Returns guidance for a
design-system rules file, plus the file's token summary and component list (names cut at
200 characters). It writes no files.

Tool errors (bad id, unknown file, missing node) come back as results with
`isError: true`, not as protocol failures.

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
  on Unix). A symlink at that path or a sidecar over 10 MiB is refused. Concurrent writers
  from other processes are not coordinated (the last one wins). Writes from sessions of
  one server are serialized.
- **Asset PNGs:** `get_design_context` writes `<fileKey>_<id>.png` files into
  `--assets-dir` (or returns them inline with `--assets inline`).
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
- **Original image bytes** come from eagerly opened documents only; there is no size cap
  per image.
- **FigJam:** connector geometry beyond loose-end coordinates, table row/column structure,
  merged cells and sticky text style are not reported.
- **Code Connect and design system** cover this file only. No remote libraries.
- **Big files:** the whole node tree is held in memory (several GB for 50 MB+ files). Every
  text tool has an output cap; time and size on huge pages were not measured.

## Tests

See `docs/TESTING.md` section "MCP server". Run `scripts/check.sh` for the full gate.
