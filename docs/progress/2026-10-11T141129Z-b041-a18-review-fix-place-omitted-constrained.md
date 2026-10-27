- 2026-10-11 — A18 review fix: place omitted constrained translations before inherited
  scaling; preserve explicit placement and auto-layout flow. Scale inherited stored glyphs
  and stroke weights by their field owners, keeping outer-stated values untouched; remaining
  inherited ink approximations are counted in status. Synthetic red/green and ownership
  mutation tests, workspace/wasm gates, PII scan and local render comparisons passed.
  Not verified: live Figma comparison, fallback font or stored-outline recomputation.
