- 2026-10-11 — MCP L7: `download_assets` renders up to 20 nodes to PNG/JPG (JPG flattened onto
  white, same scale range and 4096 px cap as get_screenshot via a shared `render_node`) or
  returns the original image-fill bytes (`original=true`). 9 tests in `tests/assets.rs`, each
  proven by a mutation; 28 fixture PNGs unaffected (no render crate change); check.sh passed.
  Not verified: lazily opened files, very large originals, live clients.
