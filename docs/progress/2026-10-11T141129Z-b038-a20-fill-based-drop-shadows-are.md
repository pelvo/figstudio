- 2026-10-11 — A20: fill-based drop shadows are drawn behind the node in list order, and
  inner shadows over its fills. Positive spread is applied; negative spread and non-normal
  blend modes are counted as approximate. Tests in `shadows.rs`. Not verified: live compare
  in Figma; spread on rounded shapes uses a stroked outline.
