# figstudio

Fast, offline-first Figma `.fig` viewer, renderer, version comparator, and Model Context
Protocol (MCP) server, written in Rust.

- **No Figma account or cloud upload required:** reads `.fig` archives directly from disk or
  a browser `File` handle, with lazy image streaming so multi-gigabyte files open in a few
  hundred milliseconds.
- **Runs natively and in the browser:** the desktop app (`eframe` + `wgpu`) and the browser
  WebAssembly build share the same parser (`figstudio-fig`), 2D renderer (`figstudio-render`
  on Vello + Parley), and MCP tool core (`figstudio-mcp-core`).
- **Built-in AI agent access (MCP):** inspect node trees, design tokens, CSS/Tailwind/SVG
  hints, and rendered PNG screenshots, compare `.fig` versions, or highlight nodes in the
  live canvas from Claude Code, Cursor, Codex, or any MCP client.

---

## Workspace layout

| Crate / directory | Purpose |
|---|---|
| `crates/fig` (`figstudio-fig`) | `.fig` archive reader/writer, Kiwi binary schema decoder, node tree and property model |
| `crates/render` (`figstudio-render`) | Vello + Parley 2D renderer (fills, strokes, effects, masks, auto-layout, text, vectors, PNG export) |
| `crates/diff` (`figstudio-diff`) | Structural and pixel diff between two `.fig` files (`report.json`, self-contained `report.html`, per-page PNGs) |
| `crates/mcp-core` (`figstudio-mcp-core`) | Shared MCP tool implementations and schemas used by both the native server and the browser Wasm build |
| `apps/figstudio` (`figstudio`) | Interactive desktop (`wgpu`) and browser (`wasm32` + WebGPU/CPU tile workers) viewer |
| `apps/figstudio-cli` (`figstudio-cli`) | Headless CLI for inspecting, rendering, stress-testing, and comparing `.fig` files |
| `apps/figstudio-mcp` (`figstudio-mcp`) | MCP server (`stdio`, Streamable HTTP `--http`, local browser bridge/proxy, and live desktop connection) |
| `web/` | Browser shell (`index.html`, `_headers`) and Cloudflare Worker + `McpRoom` Durable Object (`worker.mjs`) |

---

## Quick start

### 1. Desktop app (`figstudio`)

```sh
cargo run --release -p figstudio -- path/to/design.fig
```

- Pan and zoom the canvas, switch pages, search layers, inspect properties, and export any
  selection or page to PNG.
- When `figstudio` is open, it listens on a user-only local socket so `figstudio-mcp` can
  query the live selection or reveal a node (`reveal_node`) directly in the window.

### 2. Browser app (WebAssembly)

Build and serve `web/dist` locally on `http://127.0.0.1:8765` (with the
`Cross-Origin-Opener-Policy` and `Cross-Origin-Embedder-Policy` headers required for Wasm
worker threads):

```sh
./scripts/web-build.sh --serve
```

See [`apps/figstudio-mcp/README.md`](apps/figstudio-mcp/README.md) for deploying `web/dist`
either as a static site (paired with a local `figstudio-mcp` process) or with the Cloudflare
Worker + `McpRoom` Durable Object relay (`web/worker.mjs`).

### 3. Headless CLI (`figstudio-cli`)

```sh
# Document summary (pages, node counts)
cargo run --release -p figstudio-cli -- info path/to/design.fig

# Print the layer tree of page 1
cargo run --release -p figstudio-cli -- tree path/to/design.fig --page 1 --depth 3

# Render a page or specific node (e.g. 12:34) to PNG
cargo run --release -p figstudio-cli -- render path/to/design.fig --node 12:34 --scale 2 -o out.png

# Compare two versions of a .fig file (writes report.json, report.html, and diff PNGs)
cargo run --release -p figstudio-cli -- compare old.fig new.fig -o diff-report

# Shareable diagnostics (counts, timings, memory — no layer names, text, or paths)
cargo run --release -p figstudio-cli -- doctor path/to/design.fig
```

### 4. MCP server (`figstudio-mcp`)

Full setup and tool documentation is in
[`apps/figstudio-mcp/README.md`](apps/figstudio-mcp/README.md):

```sh
# Offline file mode (stdio)
claude mcp add figstudio -- /path/to/figstudio-mcp /path/to/design.fig

# Cloud relay mode (browser tab runs Wasm MCP; zero .fig upload)
claude mcp add --transport http figstudio https://<your-deploy-domain>/mcp/rm_<64 hex>
```

Agent skills for coding assistants are included under [`.agents/skills/`](.agents/skills/):
- [`compare-fig-versions`](.agents/skills/compare-fig-versions/SKILL.md) — compare two
  `.fig` files and triage structural vs rendering changes.
- [`cloud-relay-sessions`](.agents/skills/cloud-relay-sessions/SKILL.md) — connect to the
  browser cloud relay and route across multiple devices (`list_sessions`, `select_session`).

---

## Development and checks

Run the full formatting, lint, and test gate before opening a pull request:

```sh
# Rust fmt, clippy (-D warnings), and workspace unit/integration tests
./scripts/check.sh

# Include the wasm32 browser build and clippy checks
./scripts/check.sh --web

# Cloudflare Worker unit tests
node --test web/worker/*.test.mjs
```

---

## Third-party notices

- Sample `.fig` fixtures under `tests/fixtures/` from `fig2sketch` are MIT-licensed; see
  [`tests/fixtures/LICENSE-fig2sketch`](tests/fixtures/LICENSE-fig2sketch).
- Bundled fallback fonts in `crates/render/fonts/` include DejaVu Sans and Noto Sans /
  Source Han Sans subsets under their respective open font licenses (see
  `scripts/web-font.sh`).

---

## License

Licensed under either of

- Apache License, Version 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([`LICENSE-MIT`](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed
as above, without any additional terms or conditions.
