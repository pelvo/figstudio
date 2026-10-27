- 2026-10-12 — MCP lazy documents (PR #5 review lane 4): `FileRegistry` opens via `open_auto_above`
  (`with_lazy_above` for tests); all MCP renders use `render_complete`/`render_path_complete` with a
  per-file shared `Images` cache and on-demand `archive::read_entry`; `download_assets original=true`
  reads lazy originals after a size check against the budget. `tests/lazy.rs` proven by mutations;
  CLI render of 7 fixtures at 0.25 and 1 byte-identical to 407102c; foreground `scripts/check.sh`:
  all checks passed. Not verified: a real >256 MiB file, the wasm UI (`web.rs` unchanged).
