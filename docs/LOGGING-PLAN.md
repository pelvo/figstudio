# Plan: tracing for logs, thiserror/anyhow for errors

Status 2026-10-13: lanes 1-3 in PR #12; lanes 4 and 5 on `chore/tracing-lanes-4-5`.
Still open: `Result<_, String>` in `web.rs` (wasm), `ImageSource::read`, `ExportDone::result`,
the `live.rs` reveal reply, `crates/fig` internal `read::R`, and PNG encode failures in
`mcp-core` `screenshot()`/`figjam.rs` not yet using `CoreError::Encode`.

Branch: `chore/tracing-errors` (from `origin/main`). Goal: easier debugging. Every log
line goes through `tracing` with a level and a filter, and every error keeps its cause chain
and context instead of a flattened `String`.

## Where we are

- No logging library. 13 `eprintln!` (app, CLI, MCP, `gpu_common.rs`), no levels, no filter.
- `FigError(pub String)` in `crates/fig/src/doc.rs`: 31 construction sites plus 20 `err(..)`
  calls. The cause (`io::Error`, zip, kiwi) is formatted into the string and lost.
- `figstudio-cli`: `run() -> Result<(), String>`, plus `parse`, `pick_page` and more.
- `figstudio-mcp`: `main() -> Result<(), Box<dyn Error>>`.
- Already good: `CoreError` (`crates/mcp-core/src/error.rs`) and `apps/figstudio-mcp/src/error.rs`
  are canonical structs with a private `Kind` and a `Backtrace`. Keep them.
- `tracing` and `thiserror` are already in `Cargo.lock` (pulled in by rmcp/wgpu), so the
  dependencies are mostly free.

## Which crate where (rule 12 and the Pragmatic Rust Guidelines)

| Layer | Errors | Logs |
|---|---|---|
| Libraries `crates/fig`, `crates/render`, `crates/mcp-core` | Canonical structs (M-ERRORS-CANONICAL-STRUCTS). `thiserror` only for the `Display` and `source()` boilerplate. **No `anyhow` in a public API.** | `tracing` macros only. A library never installs a subscriber. |
| Apps `figstudio`, `figstudio-cli`, `figstudio-mcp` | `anyhow::Result` with `.context(..)` (M-APP-ERROR) | `tracing-subscriber` with `EnvFilter`, installed once in `main` |

Crates: `tracing 0.1`, `tracing-subscriber 0.3` (`fmt`, `env-filter`), `thiserror 2`,
`anyhow 1`, and for the web build `tracing-web` (logs to the browser console). All are
widely used and maintained. Add them to `[workspace.dependencies]`.

## Rules that do not change

1. **Privacy (rules 8 and 9).** Log fields never hold layer, page or font names, text,
   colours, image hashes, node ids or paths. Log counts, sizes, kinds and durations only. A
   test runs the CLI with `RUST_LOG=trace` on a fixture and checks that no fixture name
   appears on stderr.
2. **stdout is data.** CLI output and the MCP stdio protocol stay on stdout. All logs go to
   stderr. `doctor` output stays byte for byte the same (`tests/doctor.rs`).
3. **No panics on bad input.** Error messages that users or tests see keep their text, or
   the tests are updated on purpose.
4. **Default level `warn`.** Normal runs print what they print today. Use `RUST_LOG=debug`
   to see more.
5. **Rendering stays as fast as it is.** No spans in per-node or per-path hot loops. Spans go
   on decode, page render, tile batch and MCP tool call.

## Lanes (one commit each, `scripts/check.sh` green after each)

### Lane 1: logging in the apps
- Add the workspace deps. Add `fn init_tracing()` to each app's `main`:
  `tracing_subscriber::fmt().with_writer(std::io::stderr).with_env_filter(EnvFilter::from_default_env().add_directive(Level::WARN.into()))`.
  In wasm, use `tracing-web` in `web.rs`.
- Replace the 13 `eprintln!` with `warn!`/`error!`/`info!`. Keep the text. Exception: the
  MCP `listening on …` line stays visible at the default level (`info!` plus a default
  directive, or keep it as plain output). Decide this in review.
- Check: `RUST_LOG=debug figstudio-cli info tests/fixtures/structure.fig` prints logs on
  stderr only, and stdout matches the run without `RUST_LOG`.

### Lane 2: `FigError` as a canonical struct
- `pub struct FigError { kind: Kind, backtrace: Backtrace }` with a private
  `#[derive(thiserror::Error)] enum Kind { Io(#[source] io::Error), Archive(..), Schema(..), Limit{..}, Malformed(String) }`.
  Model it on `CoreError`. Add `is_*()` methods only where callers need them.
- **Breaking:** `FigError(pub String)` is a public tuple constructor used in 31 places.
  Replace them with constructors (`FigError::malformed(..)`, `From<io::Error>`). Keep
  `Display` text the same so the CLI, app toasts and tests do not change.
- `source()` now returns the real cause, so `{:#}` / `anyhow` chains show it.
- Proof (rule 3): a test opens a missing or truncated file and checks that
  `error.source()` is `Some(io::Error)`. Show that it fails on the old code.
- Rule 7: run `encoded_message_matches_the_original_bytes`. It is unaffected, but must stay
  green.

### Lane 3: `anyhow` in the CLI and the MCP binary
- `figstudio-cli`: `run`, `pick_page` and the helpers return `anyhow::Result`, with
  `.context("render page")` and similar at each step. `main` maps errors to exit codes: usage
  errors to `2` through a small `UsageError` marker type that `downcast_ref` finds, the rest
  to `FAILURE`. Print with `{:#}` so the chain shows on one line.
- `figstudio-mcp`: `main` and `run` return `anyhow::Result<()>`.
- **Do not** let raw paths reach `.context(..)` strings (privacy). `doctor` stays as it is.
- Check: the existing CLI exit-code tests (2 for usage, 1 for failure) still pass. One new
  test checks that a corrupt file's error message includes the cause.

### Lane 4: spans at the main boundaries (optional, after 1 to 3)
- `#[instrument(skip_all, fields(bytes = len))]` on `Document::open` / `open_lazy`, page
  render, GPU tile batch and each MCP tool handler (tool name and duration only).
- `debug!` where `Stats` counters increase is **not** in scope. `Stats` stays the
  user-visible count (rule 6).

### Lane 5: the remaining `Result<_, String>` (39 sites)
- `crates/mcp-core` (screenshot, assets, link, region_*): move them into `CoreError` kinds.
- App internals (`worker.rs`, `source.rs`, `web.rs`, `mcp_host.rs`, `region_*`): `anyhow`.
- `crates/fig/src/val.rs`: fold into `FigError`.
- This lane is mechanical, so it can be split per file.

## Out of scope
- OpenTelemetry, JSON log output, log files, metrics.
- Changing user-visible error wording on purpose.
- `tracing` in `crates/render` inner loops.

## Done checks
- `scripts/check.sh` ends with `all checks passed` after each lane.
- `rg 'eprintln!' apps crates --glob '!**/tests/**'` finds only deliberate user output (if any).
- `rg 'FigError\(' crates apps` finds 0 tuple constructions.
- The privacy test (stderr at `trace` has no fixture names) and `tests/doctor.rs` pass.
- `scripts/thumb-sheet.py` looks the same as before (no rendering change expected).
- Add a line to the `docs/PLAN.md` progress log, and a "Logging and errors" section to
  `docs/IMPLEMENTATION.md`.
