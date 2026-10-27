- 2026-10-11 — A18: a uniformly scaled instance (`uniformScaleFactor`) now also scales the
  size and offset that a nested instance states in its own unscaled derived data; before,
  only the innermost scale applied to omitted offsets and sizes were never scaled, so stored
  outlines drawn at the scaled size sat off base-shape discs in tooth-chart-like icons.
  Test in `instance_overrides.rs`. Not verified: live compare in Figma.
