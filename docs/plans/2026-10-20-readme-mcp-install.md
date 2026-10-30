---
pr: 40
branch: docs/readme-mcp-install
title: docs: add a one-line Claude Code install and the public browser build
---

## Progress

### 2026-10-20T163131Z b193-readme-a-try-it-section-website
- 2026-10-20 — README: a Try it section (website, the public browser build at https://figstudio.pelvo.dev, and a one-line `cargo install … && claude mcp add …` for Claude Code), and the MCP server section now lists the browser proxy, file, desktop and cloud relay setups. figstudio-mcp README gains an Install section and uses the public origin in its browser-proxy examples. Checked: the `cargo install --git` line built and installed figstudio-mcp 0.1.0 from main into a scratch root; `claude mcp add -s project … --origin https://figstudio.pelvo.dev` (Claude Code 2.1.304) wrote the expected stdio entry; the installed binary answered initialize and listed 17 tools including open_figstudio_browser. Not checked: an end-to-end Claude Code session, and the Local Network Access prompt with a real click.

### 2026-10-20T163131Z b195-pr-40-review-comment-6287424481-the
- 2026-10-20 — PR #40 review (comment 6287424481): the one-line install now says Safari cannot pair (WebKit blocks `ws://127.0.0.1` from an `https://` page) and how to get around it: `-e 'BROWSER=open -a "Google Chrome"'` on the `claude mcp add` line, or the local `:8765` build. The `open_figstudio_browser` bullet names the first `--origin` (the public build) instead of `<your-deploy-domain>`. The desktop URL says the status pill's port (3845, or 3846–3855 when busy). The client-setup note shows the `.mcp.json` entry `claude mcp add -s project` writes. R9 (`docs/WEB-MCP-PLAN.md`) gains an amendment: the public static build may be named in the repo; private and account deploy domains still may not. Checked: `claude mcp add -s project` (Claude Code 2.1.304) in a scratch directory writes the documented entry, and with `-e 'BROWSER=…'` it stores the value with its quotes intact; `parse_browser_entry` already splits `open -a "Google Chrome"` (its test in `browser_proxy.rs`). Not checked: Safari itself, or a real Chrome launch through `BROWSER`.

### 2026-10-20T163131Z b196-pr-40-re-review-comment-6287941045
- 2026-10-20 — PR #40 re-review (comment 6287941045): the local-build route in the Safari note is `figstudio-mcp --browser-proxy` (dropping `--origin` left no file and no proxy flag, so the server exited), served by `scripts/web-build.sh --serve`; the Chrome example names its own `open -a` value and gives the Edge and Firefox ones. Checked against `DEFAULT_ORIGINS[0]` (`http://127.0.0.1:8765`) and the `files` `required_unless_present_any` rule in `main.rs`.

### 2026-10-20T163131Z b201-pr-40-review-thread-comment-4325126866
- 2026-10-20 — PR #40 review thread (comment 4325126866, P3): the Safari note's local-build line said `claude mcp add … --browser-proxy` "opens" `http://127.0.0.1:8765`; registering only stores the stdio entry, and `open_figstudio_browser` opens the page later. Reworded as the reviewer suggested; docs only.
