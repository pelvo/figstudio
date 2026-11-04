---
branch: feat/prototype-preview
pr: 71
title: feat: prototype Preview in the app, browser, CLI and MCP
---

## P3 — Desktop app (`apps/figstudio`)
- [ ] Prototype Preview (⇧Space, View → Preview prototype): the flow's or selection's screen
  alone, fitted on the prototype background; "On click" navigate and back links play; ←,
  Backspace, R and Esc; a click on no link flashes the links; unplayed links counted in the bar
- [ ] Shared model `figstudio_render::prototype` (flows, screens, links, player), one draw-order
  walk per screen (`Inspect::Links`), links inside instances with overrides
- [ ] `figstudio-cli prototype` lists flows, reachable screens and links; `-o` renders them
- [ ] MCP `get_prototype`, native and through the browser bridge

## Progress

### 2026-11-04T090000Z prototype-preview
- What changed and how it was checked: Preview in the app (desktop and browser share the code), `figstudio-cli prototype`, MCP `get_prototype` (native and browser bridge), one shared model in `figstudio_render::prototype`. Tests: render (6), CLI (3), MCP protocol (2) plus the stdio, http and wasm RPC tool lists, app kittest (2). Five deliberate breaks each failed a test (Back not played, own links ignored, missing destination played, tiles drawing the page, a click not followed). fmt and workspace clippy `-D warnings` clean on nightly 1.101. The CLI render of a screen was looked at. On local private files (counts only): links inside instances match `path_bounds` for 3,036 of 3,036; a 834-link screen reads in about 0.1 s.
- Not verified: full `scripts/check.sh` (stopped for time; CI runs it); the desktop window by eye; the browser build and any browser run; GPU tests; a test run of every binary after the last fix (the http tool-count test was fixed and rerun alone).
