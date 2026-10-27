---
pr: 5
branch: feat/mcp
title: feat(mcp): add offline figstudio-mcp server for local .fig files
---

## P7 — Offline MCP server (`apps/figstudio-mcp`, plan in `docs/MCP-PLAN.md`)
- [x] L0 schema facts (auto-layout, text runs, tokens, FigJam; none of the fixtures has variables, FigJam or motion data)
- [x] L1 skeleton: rmcp server, stdio and Streamable HTTP, node ids, file registry, `whoami`, `list_files`
- [x] L2 `get_metadata`
- [x] L3 `get_screenshot`
- [x] L4 public token model (`fig::tokens`) and `get_variable_defs`
- [x] L5 public instance resolver (`fig::resolve`) and instance paths `I…;…`
- [x] L6 `get_design_context` (React + Tailwind, assets)
- [x] L7 `download_assets` (PNG/JPG, original image bytes)
- [x] L8 `get_figjam`
- [x] L9 Code Connect (local sidecar) and design-system search (`get_context_for_code_connect`,
      `get_code_connect_map`, `add_code_connect_map`, `search_design_system`, `get_libraries`)
- [x] L10 `create_design_system_rules` prompt
- [x] L11 merged into L1 (Streamable HTTP)
- [x] L12 docs: `apps/figstudio-mcp/README.md`, `docs/TESTING.md` section 12, code map and task board
- [ ] Open (task board M1–M6): `get_motion_context`, SVG export, ancestor masks on instance-child
      screenshots, nested-instance override visibility in `original=true`, live-client check,
      real FigJam and motion files
