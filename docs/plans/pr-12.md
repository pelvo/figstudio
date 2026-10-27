---
pr: 12
branch: chore/tracing-errors
title: feat(fig,render,cli,mcp,app): tracing logs, spans and errors that keep their cause
---

## Progress

### 2026-10-13T125638Z b086-logging-and-errors-docs-logging-plan
- 2026-10-13 — Logging and errors (docs/LOGGING-PLAN.md lanes 1-3): `tracing` to stderr in the CLI, MCP binary, desktop and browser app (default `warn`, `RUST_LOG` overrides); `FigError` is a canonical struct whose `source()` returns the I/O, zip or codec cause (Display text unchanged); CLI and MCP binary use `anyhow` with context, exit codes kept. Tests: `opening_a_missing_file_keeps_the_io_error_as_source`, CLI trace-logging stdout/privacy, truncated-file and cause-printed-once tests (each seen failing). check.sh passed. Not verified: MCP `listening on` line by hand, lanes 4 (spans) and 5 (remaining `Result<_, String>`).

### 2026-10-13T125638Z b087-logging-lanes-4-5-debug-spans
- 2026-10-13 — Logging lanes 4-5: debug spans on document open/decode, render, GPU tile submit and MCP tool calls (tool name and duration only; test `tracing_spans.rs` checks counts are logged and no layer names); `mcp-core` render/asset functions return `CoreError` with real causes (test `errors.rs`); app internals use anyhow; `figstudio_fig::one_line` prints an error chain without repeating causes (CLI and app toasts; tests seen failing). check.sh and web-build passed; MCP stdio `whoami` keeps stdout pure JSON and logs `tool_call` at debug; `--http` still prints `listening on`. Not verified: app toasts in a real window, browser console output.

### 2026-10-13T125638Z b088-logging-follow-up-png-encode-failures
- 2026-10-13 — Logging follow-up: PNG encode failures in mcp-core use `CoreError::encode` (cause kept, text unchanged; test `an_encode_error_keeps_its_cause_and_the_old_text`); app `ImageSource::read`, `ExportDone::result`, region PNG, web.rs and the reveal reply use anyhow, formatted with `one_line` at the UI/tool boundary (test `a_missing_file_keeps_its_io_cause`, seen failing). check.sh and web-build passed. Not verified: browser paths run in a real browser.

### 2026-10-13T125638Z b089-review-fixes-for-pr-12-listening
- 2026-10-13 — Review fixes for PR #12: `listening on` is now plain stderr (survives `RUST_LOG=error`; test `listening_line_survives_a_quiet_rust_log`, seen failing with the tracing version); rmcp is capped at info unless `RUST_LOG` names it (test `trace_logs_do_not_leak_requests_or_layer_names`, seen failing without the cap); tool errors log `ok=false`; CLI drops the `open document`/`parse background`/`save document` context prefixes (text unchanged); over-inflated archive entries are `malformed`. Docs updated. Checked: the new and changed tests, `cargo test` for mcp, cli and fig on 1.99.0. Not verified: `scripts/check.sh` as a whole (see the final gate run), a real MCP client reading the port, and the `ok=` log field in a live run.
