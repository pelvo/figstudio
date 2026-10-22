# figstudio

[![Buy Me A Coffee](https://img.shields.io/badge/Buy%20Me%20a%20Coffee-ffdd00?style=for-the-badge&logo=buy-me-a-coffee&logoColor=black)](https://buymeacoffee.com/pelvo)

https://github.com/user-attachments/assets/11158bc3-24a5-4692-b4e4-579f66d30f60

Fast, offline-first Figma `.fig` viewer, renderer, version comparator, and Model Context
Protocol (MCP) server, written in Rust.

- **No Figma account or cloud upload required:** reads `.fig` archives directly from disk or
  a browser `File` handle. Archive files larger than 256 MiB (`LAZY_ABOVE`) stream image
  entries lazily in the desktop app (`Document::open_auto`), the browser app (for ZIP
  archives over 256 MiB), and `figstudio-cli compare` (~0.7 s desktop open and ~1.4 s
  browser open on a 1.7 GB file); smaller files and the other `figstudio-cli` commands read
  the file in full up to a 1 GiB (`MAX_FILE_BYTES`) ceiling.
- **Runs natively and in the browser:** the desktop app (`eframe` + `wgpu`) and the browser
  WebAssembly build share the same parser (`figstudio-fig`), 2D renderer (`figstudio-render`
  on Vello CPU with optional `vello_gpu` and custom text layout on `fontdb` / `ttf-parser`),
  and MCP tool core (`figstudio-mcp-core`).
- **Built-in AI agent access (MCP):** inspect node trees (`get_metadata`), React + Tailwind
  code hints (`get_design_context`), variables and tokens (`get_variable_defs`), rendered PNG
  screenshots (`get_screenshot`), and embedded raster images (`download_assets`; offline SVG
  and PDF export are not available), compare `.fig` versions, or highlight nodes in the live
  canvas from Claude Code, Cursor, Codex, or any MCP client.

## Try it

- **Website:** https://pelvo.dev
- **In the browser, nothing to install:** https://figstudio.pelvo.dev. Open or drop a `.fig`
  (Figma → File → Save local copy); it stays in the tab and nothing is uploaded.
- **macOS (Apple silicon) with Homebrew:**

  ```sh
  brew install --cask pelvo/tap/figstudio
  ```

  This installs `figstudio.app` in `/Applications` and puts `figstudio-cli` and
  `figstudio-mcp` on your `PATH`. Update with `brew upgrade --cask figstudio`; remove with
  `brew uninstall --cask figstudio` (add `--zap` to delete its settings as well). The app
  is ad-hoc signed, not notarized; the cask clears the download quarantine so it opens
  normally. The tap is [pelvo/homebrew-tap](https://github.com/pelvo/homebrew-tap).
- **Linux (x86_64 or arm64), CLI and MCP server:** each release has a static
  `figstudio-cli-mcp-vX.Y.Z-linux-<arch>.tar.gz` (any distro, Alpine too; from the release
  after v0.1.0). To check the latest one against the release's `SHA256SUMS` and put it in
  `~/.local/bin`:

  ```sh
  v=$(curl -fsSLo /dev/null -w '%{url_effective}' https://github.com/pelvo/figstudio/releases/latest); v=${v##*/}
  f=figstudio-cli-mcp-$v-linux-$(uname -m | sed 's/aarch64/arm64/').tar.gz
  (cd "$(mktemp -d)" && curl -fsSL -O "https://github.com/pelvo/figstudio/releases/download/$v/$f" -O "https://github.com/pelvo/figstudio/releases/download/$v/SHA256SUMS" &&
    awk -v f="$f" '$2 == f' SHA256SUMS | sha256sum -c - && mkdir -p ~/.local/bin && tar -xzf "$f" -C ~/.local/bin)
  ```

  If `~/.local/bin` is not on your `PATH` yet (Debian and Ubuntu add it only at login, and
  only if it existed then), run `export PATH="$HOME/.local/bin:$PATH"` in this shell and add
  the same line to your shell profile for later sessions. Registering the server by its full
  path (`~/.local/bin/figstudio-mcp`) also works without it.
- **Give it to Claude Code in one line** (needs Node.js; npm fetches a prebuilt binary for
  Linux, macOS or Windows, nothing is compiled):

  ```sh
  claude mcp add figstudio -- npx -y figstudio-mcp --browser-proxy
  ```

  The first time the agent needs a file it calls `open_figstudio_browser`, which opens
  the figstudio.pelvo.dev app paired with the local server; open your `.fig` in that tab.
  `figstudio-mcp` fetches the app from figstudio.pelvo.dev and serves it on its own
  `http://127.0.0.1:<port>`, so the page reaches the server without a local network
  access prompt (a headless browser cannot answer one). `--no-mirror` opens
  https://figstudio.pelvo.dev itself instead; Chrome then asks to let the page reach
  devices on your local network, and Safari cannot pair at all (it blocks an `https://`
  page from reaching `ws://127.0.0.1`).
  **What you trust:** that page's code comes from figstudio.pelvo.dev and reads the files
  you open in it, so "nothing is uploaded" holds as long as that deployment serves this
  repository's build. To keep everything on your machine, use the local build:
  serve it with `scripts/web-build.sh --serve` and register
  `claude mcp add figstudio -- npx -y figstudio-mcp --origin http://127.0.0.1:8765`;
  `open_figstudio_browser` then opens that address. To serve a file from disk instead,
  with no browser: `claude mcp add figstudio -- npx -y figstudio-mcp /path/to/design.fig`.

  Without Node, install `figstudio-mcp` with Homebrew or the Linux tarball above (from the
  release after v0.1.0; v0.1.0's `--browser-proxy` opens `http://127.0.0.1:8765` instead) and
  register `claude mcp add figstudio -- figstudio-mcp --browser-proxy`, or build it with Rust
  1.95 or newer: `cargo install --locked --git https://github.com/pelvo/figstudio figstudio-mcp && claude mcp add figstudio -- figstudio-mcp --browser-proxy`.

---

## Workspace layout

| Crate / directory | Purpose |
|---|---|
| `crates/fig` (`figstudio-fig`) | `.fig` archive reader/writer, Kiwi binary schema decoder, node tree and property model |
| `crates/render` (`figstudio-render`) | Vello CPU (`vello_cpu`) + optional `vello_gpu` 2D renderer with text layout on `fontdb` / `ttf-parser` |
| `crates/diff` (`figstudio-diff`) | Structural and pixel diff between two `.fig` files (`report.json`, self-contained `report.html`, per-page PNGs) |
| `crates/mcp-core` (`figstudio-mcp-core`) | Shared MCP tool implementations and schemas used by both the native server and the browser Wasm build |
| `apps/figstudio` (`figstudio`) | Interactive desktop (`wgpu`) and browser (`wasm32` + WebGPU/CPU tile workers) viewer, including the in-process desktop MCP server |
| `apps/figstudio-cli` (`figstudio-cli`) | Headless CLI for inspecting, rendering, stress-testing, and comparing `.fig` files |
| `apps/figstudio-mcp` (`figstudio-mcp`) | Standalone MCP server (`stdio`, Streamable HTTP `--http`, and local browser bridge `--bridge`) |
| `web/` | Browser shell (`index.html`, `_headers`) and Cloudflare Worker + `McpRoom` Durable Object (`worker.mjs`) |

---

## Quick start

These commands build from a checkout. To install a release instead, see [Try it](#try-it):
Homebrew on macOS (the app, `figstudio-cli` and `figstudio-mcp`) or the Linux tarball
(`figstudio-cli` and `figstudio-mcp`).

### 1. Desktop app (`figstudio`)

```sh
cargo run --release -p figstudio -- path/to/design.fig
```

- Pan and zoom the canvas, switch pages, search layers, inspect properties, and export any
  selection or page to PNG.
- Paste a `figma.com` or `figstudio://` URL into the command bar (⌘L, Ctrl+L elsewhere), then
  press Enter to open its matching local `.fig` copy and reveal the linked node. No download
  from Figma is needed. If several copies match, pick one in the link window.
- Agent access is off at launch. **View → Agent access…** opens Settings at its Agents tab;
  turn on **Agent access** there to start the in-process Streamable HTTP MCP server on
  loopback (`http://127.0.0.1:3845/mcp`, or `3846–3855` if busy) so an MCP client can
  query open tabs, read the live selection, or reveal a node (`reveal_node`) in the window.
  Settings itself is ⌘, on a Mac (Ctrl+, and File → Settings… elsewhere).

### 2. Browser app (WebAssembly)

Prerequisites for building `web/dist`:
- The pinned nightly Rust toolchain with `rust-src` and `wasm32-unknown-unknown`:
  ```sh
  rustup toolchain install nightly-2026-10-09 --component rust-src --target wasm32-unknown-unknown
  ```
- `wasm-bindgen-cli` at the exact `wasm-bindgen` version in `Cargo.lock` (`0.2.129`):
  ```sh
  cargo install --locked wasm-bindgen-cli --version 0.2.129
  ```
- Python `fontTools` plus a system Noto Sans CJK collection for subsetting CJK fallback
  fonts into `web/dist/fonts/` (or set `FIGSTUDIO_ALLOW_MISSING_CJK=1` to skip CJK font
  subsetting during local development).

Build and serve `web/dist` locally on `http://127.0.0.1:8765` (with the
`Cross-Origin-Opener-Policy` and `Cross-Origin-Embedder-Policy` headers required for Wasm
worker threads):

```sh
./scripts/web-build.sh --serve
```

See [`apps/figstudio-mcp/README.md`](apps/figstudio-mcp/README.md) for connecting an MCP
client via `--bridge` or deploying `web/dist` with the Cloudflare Worker + `McpRoom` Durable
Object relay (`web/worker.mjs`).

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

Run the server with `npx -y figstudio-mcp` (prebuilt binaries on npm), install it with
Homebrew or the Linux tarball (see [Try it](#try-it)), on its own with
`cargo install --locked --git https://github.com/pelvo/figstudio figstudio-mcp`, or build it
in a checkout with `cargo build --release -p figstudio-mcp`. Full setup and tool documentation
is in [`apps/figstudio-mcp/README.md`](apps/figstudio-mcp/README.md):

```sh
# Browser proxy (zero upload): the agent opens https://figstudio.pelvo.dev and pairs with it
claude mcp add figstudio -- npx -y figstudio-mcp --browser-proxy

# Offline file mode (stdio): serve files from disk, no browser
claude mcp add figstudio -- npx -y figstudio-mcp /path/to/design.fig

# Desktop app: turn on Settings → Agents → Agent access, then use the address the status
# pill shows (3845, or the first free port of 3846–3855 when 3845 is busy)
claude mcp add --transport http figstudio http://127.0.0.1:3845/mcp

# Cloud relay: only on your own deploy of web/worker.mjs (https://figstudio.pelvo.dev has none)
claude mcp add --transport http figstudio https://<your-deploy-domain>/mcp/rm_<64 hex>
```

Cursor (`.cursor/mcp.json`), VS Code (`.vscode/mcp.json`) and Codex
(`~/.codex/config.toml`) take the same command and arguments; see
[Client setup](apps/figstudio-mcp/README.md#client-setup).

Agent skills for coding assistants are included under [`.agents/skills/`](.agents/skills/):
- [`compare-fig-versions`](.agents/skills/compare-fig-versions/SKILL.md) — compare two
  `.fig` files and triage structural vs rendering changes.
- [`cloud-relay-sessions`](.agents/skills/cloud-relay-sessions/SKILL.md) — connect to the
  browser cloud relay and route across multiple devices (`list_sessions`, `select_session`).

---

## Development and checks

Requires Rust **1.95+** (`rust-version = "1.95"` in `Cargo.toml`; `egui` 0.36 requires
Rust 1.95+). If your default `rustc` is older, set `FIGSTUDIO_TOOLCHAIN` (for example
`FIGSTUDIO_TOOLCHAIN=1.99.0`).

```sh
# Format check, clippy (-D warnings), workspace tests, release build, and wasm32 build of figstudio-mcp-core
./scripts/check.sh

# Also build the browser bundle via scripts/web-build.sh
./scripts/check.sh --web

# Cloudflare Worker unit tests
node --test web/worker/*.test.mjs
```

---

## Third-party notices

- Sample `.fig` fixtures under `tests/fixtures/` from `fig2sketch` are MIT-licensed; see
  [`tests/fixtures/LICENSE-fig2sketch`](tests/fixtures/LICENSE-fig2sketch).
- Desktop text rendering uses installed system fonts via `fontdb` (`system-fonts`). For the
  browser build, `scripts/web-font.sh` subsets Noto Sans CJK Regular (SIL Open Font License
  1.1) at build time into `web/dist/fonts/` (`cjk-{jp,sc,tc,hk,kr}.otf` and
  `cjk-LICENSE.txt`). It also builds four UI-only monochrome fallbacks from Noto Sans
  Symbols, Noto Sans Symbols 2, Noto Sans Math and **Noto Emoji** (not Noto Color Emoji):
  `symbols.ttf`, `symbols2.ttf`, `math.ttf`, `emoji.ttf` plus `symbols-LICENSE.txt` (OFL 1.1).
  Install these families before a web build; the outline Noto Emoji font is available from
  [Google Fonts](https://github.com/google/fonts/tree/main/ofl/notoemoji). Override individual
  sources with `FIGSTUDIO_SYMBOLS_FONT`, `FIGSTUDIO_SYMBOLS2_FONT`, `FIGSTUDIO_MATH_FONT`
  or `FIGSTUDIO_EMOJI_FONT`. `FIGSTUDIO_ALLOW_MISSING_SYMBOLS=1` explicitly permits a local
  build without the symbol group; missing sources otherwise fail the build. The app fetches
  this group only when a file/page/layer name contains a symbol the active UI faces lack.
  Fonts are appended after CJK in every UI family. This does not enable browser canvas emoji:
  those remain skipped and counted. Subset output preserves every source scalar in the
  requested symbol/emoji blocks; fontTools validates coverage and monochrome outlines.

---

## Support

If `figstudio` saved you time or made your workflow smoother, you can support ongoing
development on [Buy Me a Coffee](https://buymeacoffee.com/pelvo).

---

## License

Licensed under the Apache License, Version 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE) or
<http://www.apache.org/licenses/LICENSE-2.0>).

Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in the work by you, as defined in the Apache-2.0 license, shall be licensed
as above, without any additional terms or conditions.
