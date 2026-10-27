- 2026-10-11 — MCP L2 review: correct Figma counter-clockwise rotation, omit page/document
  geometry and unsized dimensions, sanitize XML controls, mark truncated text, filter internal
  subtrees and self-close empty leaves. Regression tests proven failing first;
  scripts/check.sh: all checks passed. Not verified: live clients, official capture,
  private files or L5 expansion.
