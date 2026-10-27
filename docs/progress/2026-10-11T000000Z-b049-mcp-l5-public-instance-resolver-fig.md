- 2026-10-11 — MCP L5: public instance resolver `fig::resolve` shared by the renderer and the
  MCP tools; get_metadata expands instances (I…;… ids, overrides), and instance paths resolve
  in get_metadata, get_screenshot (instance cropped to the child box) and get_variable_defs.
  28 fixture renders byte-identical before/after, thumb sheet viewed, new tests proven by
  mutations, scripts/check.sh passed. Not verified: private/large files, screenshot box of
  scaled or resized instances, live clients.
