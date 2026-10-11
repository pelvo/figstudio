# MCP server plan (figstudio-mcp)

Goal: an offline MCP server that reads local `.fig` files and answers the same tools as
Figma's official MCP server, so a coding agent (Claude Code, Cursor, Codex, VS Code) can
work from a local copy without the Figma app or an account.

Branch `feat/mcp`, cut from `fix/known-gaps` at `f87989d`. Research notes (untracked) are in
`target/mcp-research/figma-mcp-tools.md` (the official tools) and
`target/mcp-research/codebase-recon.md` (what figstudio already has).

## 1. Official tools and what we can do offline

"Selection" does not exist offline. Every tool takes `nodeId` (`123:456`, `123-456`, or a
Figma URL with `node-id=`) and an optional `fileKey` (the loaded file's stem or path).

| Official tool | Offline | Source in figstudio | Lane |
|---|---|---|---|
| `get_metadata` | full | `Document` tree, `absolute_transform`, `size` | L2 |
| `get_screenshot` | full | `Renderer::render_fit` + `into_png` | L3 |
| `get_variable_defs` | full (all modes) | `VARIABLE` / `styleType` nodes, `render/variables.rs` | L4 |
| `get_design_context` | partial: React + Tailwind from stored data, no shader runtime | node fields, instance resolver | L6 |
| `download_assets` | partial: PNG/JPG renders and original image bytes, no SVG/PDF | renderer, `Document.images` | L7 |
| `get_figjam` | partial: decode only, FigJam kinds are not drawn yet | generic `Node.data` | L8 |
| `get_context_for_code_connect` | full: component property definitions | `componentPropDefs` | L9 |
| `get_code_connect_map` / `add_code_connect_map` | partial: local sidecar JSON instead of Figma's cloud | new | L9 |
| `search_design_system` / `get_libraries` | partial: components and styles in this file only | `SYMBOL` / style nodes | L9 |
| `create_design_system_rules` (prompt) | full: prompt text plus a summary of the file's tokens | L4 output | L10 |
| `get_motion_context` | unknown: needs keyframe fields in the schema | L0 checks | later |
| `whoami` | stub only (`{"mode":"offline"}`) | none | L1 |
| `use_figma`, `create_new_file`, `generate_figma_design`, `generate_image`, shader tools, generative-plugin tools, `weave_*`, `get_code_connect_suggestions` | no: they write to Figma's canvas or need Figma's cloud | none | not registered |

Tools that cannot work offline are **not registered**, and the README lists them. Registering
them only to fail would waste the client's tool budget.

Output shapes we copy from the official server (to be checked against a real capture before
L2/L4 are frozen; see the open questions):
- `get_metadata`: XML, one element per node (`<frame id name x y width height>`), with
  `x`/`y` relative to the parent. With no `nodeId` it returns the page list. An unknown
  `nodeId` returns an error that includes the page list.
- `get_variable_defs`: a flat JSON object of `name → value`. Colours are `#RRGGBB(AA)`,
  numbers stay numbers, and text styles are `Font(family: …, style: …, size: …, weight: …,
  lineHeight: …, letterSpacing: …)`. Extra modes become `name (Mode)` keys.
- `get_design_context`: JSX with `data-node-id` and `data-name`, followed by agent
  instructions and asset references.
- `get_screenshot`: an MCP `image` content item (base64 PNG).

## 2. Design decisions (defaults, to be confirmed)

1. **Layout.** New package `apps/figstudio-mcp` with `src/lib.rs` (tools: pure functions from
   `&Document` to text, JSON or PNG) and a thin `src/main.rs` (transport). It depends only on
   `figstudio-fig` and `figstudio-render`, and is added to `default-members`.
2. **Transport (crate: `rmcp`, the official MCP Rust SDK, 3.5.x).** Both transports are
   required: **stdio** (`transport-io`) and **Streamable HTTP** (`transport-streamable-http-server`,
   served with `axum` on `127.0.0.1:3845/mcp` by default, `--http[=ADDR]`), so existing
   Figma-desktop client configs work unchanged. Protocol versions 2025-03-26, 2025-06-18,
   2025-11-25 and 2026-07-28 are negotiated by rmcp. The legacy 2024-11-05 HTTP+SSE-only
   transport is **not** supported. A stdio session that sends 2024-11-05 still gets rmcp's
   normal negotiation.
3. **Dependencies (AGENTS.md rule 12: trustworthy crates, no hand-rolled protocols or
   formats).** `rmcp` (server, macros, transport-io, transport-streamable-http-server),
   `tokio` (rt-multi-thread, macros, io-std, net, signal), `axum`, `serde` + `serde_json`,
   `schemars` (tool input schemas, through rmcp), `base64` (the screenshot payload; rmcp
   re-exports one), `quick-xml` (get_metadata / get_figjam XML writer), `clap` (derive, for
   the new binary's arguments). Rendering and decoding stay in the existing crates. Code
   follows `ms-rust-guidelines` (AGENTS.md rule 13).
4. **Files.** `figstudio-mcp [--no-system-fonts] FILE.fig...`. Files open lazily on first
   use and are cached. An `open_file` tool loads more files. The `.fig` is never written, apart
   from the L9 sidecar `FILE.fig.codeconnect.json`.
5. **Data model goes into the `fig` crate.** Read-only token and style models
   (`fig::tokens`) and the instance resolver (`fig::resolve` or `render::resolve`) become
   public APIs that the renderer also uses, so the two never disagree.
6. **Privacy.** The server returns names and text to the local client by design. That is not
   `doctor`, and the doctor no-PII rule is unchanged. Tests and docs use only the 7 public
   fixtures or documents built inside the test (ground rule 9).
7. **Unsupported means counted.** Every tool response ends with a short `approximations:`
   line built from `Stats` (screenshot) or skipped-field counters (design context), so the
   agent knows what is not exact.

## 3. Lanes (one worker per lane; commit after each lane passes `scripts/check.sh`)

Each lane: ≤ ~150 new lines per turn, source files ≤ ~300 lines, every behaviour has a test
seen failing first (docs/TESTING.md "Prove the test").

**L0 findings (done 2026-10-11, `target/mcp-research/schema-facts.md`):**
- Padding: there is no `stackPaddingLeft`/`Top`. Left = `stackHorizontalPadding`,
  top = `stackVerticalPadding`, right = `stackPaddingRight`, bottom = `stackPaddingBottom`
  (right and bottom fall back to the axis value when absent). Alignment:
  `stackPrimaryAlignItems`/`stackCounterAlignItems` (old `stackAlign`/`stackJustify` are
  legacy fallbacks). Sizing: `stackPrimarySizing`/`stackCounterSizing`.
  `stackCounterSpacing` can be NaN, meaning unset.
- Schemas differ per file (format v20 to v106), so read fields **by name only**; never by id.
- Text runs: `textData.styleOverrideTable[]` holds NodeChange fragments, keyed by
  `characterStyleIDs`. They merge the same way as instance overrides.
- Styles are bound by `styleIdFor{Fill,StrokeFill,Text,Effect,Grid}` (StyleId {guid,assetRef})
  and by the legacy `inherit*StyleID` fields.
- **No fixture has `VARIABLE`/`VARIABLE_SET` nodes, FigJam kinds, or motion data.** The
  L4, L8 and motion tests must build their documents inside the test. `get_motion_context`
  stays deferred until we have a real motion file to validate against.
- `ComponentPropType` has 18 values in v106 (VARIANT, NUMBER, IMAGE, SLOT, …). L9 must
  handle unknown values as plain strings.

**L0: schema facts (scout, read-only, about 1 hour).** Dump the embedded Kiwi schema from a
fixture and record the exact field names for: auto-layout (`stackSpacing`,
`stackHorizontalPadding`/`stackPaddingRight`/…, `stackPrimaryAlignItems`,
`stackCounterAlignItems`, sizing and grow fields); text style runs (`textData.styleOverrideTable`,
`characterStyleIDs`); variable collections, modes and `variableConsumptionMap`; text and effect
style payloads; animation or keyframe fields (for `get_motion_context`); FigJam kinds
(`STICKY`, `CONNECTOR`, `SHAPE_WITH_TEXT`, `TABLE`, …). Output:
`target/mcp-research/schema-facts.md`. **Done:** every name has a fixture node id that has it,
or is marked "absent from all fixtures".

**L1: skeleton and both transports.** [x] Complete. The package; an rmcp `ServerHandler` using the
`#[tool_router]` macros; `figstudio-mcp [--http[=ADDR]] FILE.fig...` (clap; stdio by default);
Streamable HTTP through rmcp's `StreamableHttpService` on axum; error mapping (`McpError` for
protocol faults, `isError: true` results for tool faults, never a panic; a canonical error
struct, M-ERRORS-CANONICAL-STRUCTS); node-id parsing (`1:2`, `1-2`, a URL, `I1:2;3:4` instance paths); file registry;
`whoami`. **Done:** integration tests under `tests/` (M-INTEGRATION-TESTS) drive (a) stdio, by
spawning the binary or using rmcp's client over a duplex pipe, and (b) Streamable HTTP, by
starting the server on port 0 and using rmcp's streamable-http client or plain POSTs:
initialize → tools/list → tools/call whoami → unknown tool, for protocol versions 2025-03-26
and 2025-06-18 plus the latest.

L1 verified: nine tests pass with failing-first mutation proofs, the workspace gate
ends with `all checks passed`, and a release-binary initialize smoke test produces
exactly one JSON-RPC stdout line with empty stderr. The registry lazily caches public
fixture documents; `list_files` reports page counts only after opening. CLI accepts
`--no-system-fonts`; `--http FILE.fig` uses the default address, and custom addresses
require `--http=ADDR`. Non-loopback binds warn on stderr: rmcp's default allowed_hosts
rejects non-loopback Host headers; binding externally does not enable remote clients. `rmcp` 3.5.1 negotiates an initialize request for latest down
to 2025-11-25 (with a session); actual 2026-07-28 requests use stateless per-request
metadata and method/name headers. HTTP tests exercise both behaviors, older sessions,
unknown tools, and the unchanged localhost Host allowlist. Duplicate dependency review:
only SDK-required syn 2/3 versions plus host/target copies of identical versions.
L1 review fixes: per-file OnceCell initialization retains completed decodes after request
cancellation, list_files never waits for decoding, and tool load errors include their cause.
Named tool routers compose in new() (see IMPLEMENTATION). HTTP versions have separate test
cases; stdio purity, CLI ambiguity, concurrent cache sharing, cancellation retention,
nonblocking listing, warning and Unix
SIGTERM exit are tested. Ctrl-C and SIGTERM cancel both transports; runtime shutdown avoids
waiting on an uncancellable stdin read. Tests use public inputs only.
Not verified: a live Claude/Cursor client, real Ctrl-C, Windows signals, HTTP OS-signal
shutdown, or an independent reviewer.
No renderer/decoder changes; wasm and visual checks were not needed. Instance-path
resolution and tools beyond `whoami` / `list_files` remain for later lanes.

**L2: `get_metadata`.** The page list when there is no node; an XML subtree with
`id name type x y width height`, plus `hidden="true"` and `rotation` when non-zero; text
nodes carry their characters (truncated at 200); XML escaping. Instances stay leaves with
`componentId` until L5. Optional `depth` param (default unlimited, capped at 5 000 elements
with a `truncated` marker). **Done:** structure tests on `structure.fig`; an escaping test on
a built document; an unknown id returns the page list.

**L3: `get_screenshot`.** `render_fit`, `scale` param (default 1, longest side clamped to
4096), base64 `image` content plus a text line with size and approximations. **Done:** decode
the returned base64 to a PNG with the right dimensions; an id that does not exist gives
`isError`.

**L4: tokens and `get_variable_defs`.** Move or extend `render/variables.rs` into a public
`fig::tokens`: collections, modes, the value for every mode, alias chains (depth-capped), and
colour, float, string and boolean values; paint, text, effect and grid styles with resolved
payloads. Then walk the subtree, collect the style ids and variable bindings it uses
(`styleIdFor*`, `colorVar`, `variableConsumptionMap`) and emit the flat map. The renderer
switches to `fig::tokens` with no change to its pictures. **Done:** unit tests on a built
document with 2 modes and an alias; a fixture test; render tests unchanged; thumb sheet
unchanged.

**L5: public instance resolver (highest risk, frontier-tier worker).** Pull the
override-merge logic out of the private `Renderer::patched` into a public
`resolve_instance(doc, instance) -> ResolvedTree` (symbol children with `symbolOverrides`,
component-property assignments and swaps applied; ids `I<instance>;<child>` like Figma). The
renderer keeps its scale and glyph corrections but builds on the same merge. **Done:** every
existing render test and thumb sheet unchanged; new tests on built documents for text
override, fill override, swap and nested instance; `get_metadata` now expands instances.

**L6: `get_design_context`.** Node → JSX + Tailwind:
- frames with auto-layout become `flex`, with gap, padding, alignment and grow;
  absolute children become `absolute` with `left`/`top`;
- fills become `bg-[#..]`, gradients become `bg-[linear-gradient(..)]`, image fills become
  an asset;
- strokes become `border`, radii become `rounded-[..]`, effects become `shadow-[..]` or
  `blur`, opacity and blend are carried over;
- text becomes `<p>` with font family, weight, size, line-height and tracking; style runs
  become `<span>`;
- vectors and boolean operations become an `<img>` of a rendered PNG asset (no SVG writer yet).

Variables appear as `var(--name,fallback)` when bound. `clientFrameworks` and
`clientLanguages` are accepted and echoed in the instructions. Output is the code, then
"instructions for the agent", then assets (written to `--assets-dir`, default
`./figma-assets/`, or inline base64 if `--assets inline`). Files are kept under 300 lines.
**Done:** snapshot-style string tests on built documents for each mapping, a fixture smoke
test, and an output size cap with a truncation note pointing to `get_metadata`.

**L7: `download_assets`.** `nodeIds` (up to 20), `format` png|jpg, `scale` 0.01–4, and
`original=true` for image-fill source bytes by hash. **Done:** tests decode each format and
compare the original bytes to `Document.images`.

**L8: `get_figjam`.** The L2 XML with FigJam extras: sticky text and colour, connector
endpoints, and per-node screenshots (L3) at small scale. **Done:** a built FigJam-kind document
in the test (no public FigJam fixture), plus an approximation count for kinds that are not
drawn.

**L9: Code Connect and design system (local).** `get_context_for_code_connect` (property
definitions and variants of a component); `get_code_connect_map` / `add_code_connect_map`
backed by the sidecar JSON, written atomically; `search_design_system(query)` and
`get_libraries` over local components and styles. **Done:** round-trip add → get; a search
test on a fixture.

**L10: `create_design_system_rules` prompt.** `prompts/get` returns Figma-style rule-file
guidance plus the file's token summary (L4) and component list. **Done:** a protocol test.

**L11:** merged into L1 (Streamable HTTP is required).

**L12: docs.** README client setup (Claude Code `claude mcp add`, Cursor, Codex), the
not-replicable list, `docs/PLAN.md` phase and log, `docs/IMPLEMENTATION.md` map and task
board, `docs/TESTING.md` protocol tests.

Order: L0 → L1 → L2 → L3 → L4 → L5 → L6 → L7, and L8/L9/L10 in any order → L12. L2, L3 and L4
can run in parallel worktrees after L1 lands (different files); L5 and L6 are sequential.

## 4. Verification for every lane

- `scripts/check.sh` ends with `all checks passed` (fmt, clippy `-D warnings`, tests,
  release build). Run `--web` when `crates/fig` or `crates/render` change, because they build
  for wasm.
- For lanes that touch the renderer (L4, L5): thumb sheet before and after, with no visual
  diff.
- Manual: `claude mcp add figstudio -- target/release/figstudio-mcp tests/fixtures/structure.fig`
  and call each new tool once.
- Before each commit: the PII check from AGENTS.md (`git diff --cached`).
- Reviewer per lane: a different model family from the worker; report only.

## 5. Open questions

1. Exact official output shapes for `get_variable_defs` and `get_design_context`: is there a
   real capture available (from someone with a Figma Dev seat)? Without one we follow the
   documented descriptions above.
Decided 2026-10-11 by the owner: crates over hand-rolled code (`serde_json`, `rmcp`, …);
both stdio and Streamable HTTP are required, and the 2024 SSE-only transport is not;
a separate `figstudio-mcp` binary; no official output capture is available, so we follow
the documented descriptions (question 1 is closed).

## 6. Risks

- L5 surgery on `Renderer::patched` can shift pictures. It is guarded by the full render
  suite and the thumb sheet, and kept behind the same merge function.
- Design-context quality depends on auto-layout field names that no code reads today (L0
  closes this).
- Big files (50+ MB, several GB RAM): files load lazily, only on demand, and there is an
  output size cap on every text tool.
- The official tool surface changes often (37 tools at research time). Pin the names we copy
  and record the research date in the README.
