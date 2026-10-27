- 2026-10-11 — MCP L5 review: instance paths resolve by tree position (metadata depth
  counts tree levels), instance-child screenshots draw only that node with the renderer's
  scaled layout (Renderer::render_path / path_bounds), variable defs ignore override and
  property storage, swapped instances report their new component, the walk queues children
  lazily. Each regression failed on 4518cbc; 28 fixture PNGs byte-identical; check.sh passed.
  Not verified: private/large files, live clients, ancestor masks in child screenshots.
