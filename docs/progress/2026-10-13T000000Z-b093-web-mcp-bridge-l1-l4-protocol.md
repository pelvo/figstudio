- 2026-10-13 — Web MCP bridge L1–L4: protocol in mcp-core, `figstudio-mcp --bridge` (Origin + token checked
  WebSocket, uploads to a temp dir), wasm client in the app (View → Agent access, `#mcp=` auto-connect),
  README section. Tests: protocol, bridge, shared view state, pure client helpers, and a binary-level
  end-to-end (`bridge_cli.rs`), each seen failing under a mutation. check.sh --web passed. Not verified: a
  real browser session (see WEB-MCP-PLAN.md L4 TODOs).
