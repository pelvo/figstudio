- 2026-10-11 — MCP L6: `get_design_context` writes React + Tailwind JSX (flex auto-layout,
  absolute children, fills/gradients, borders, radii, shadows/blurs, opacity/blend, text with
  style runs, var(--name,fallback)) for nodes and instance paths, with rendered PNG assets in
  `--assets-dir` (default ./figma-assets) or `--assets inline`, node/size caps and counted
  approximations. Built-document and fixture tests, each mutation-proven; check.sh passed.
  Not verified: live clients, private/large files, visual fidelity of the generated code.
