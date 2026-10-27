- 2026-10-11 — A20 review: inner clips install their fill rule; drop masks draw each path
  under its own rule; transparent solid fills no longer cast opaque geometry shadows.
  Compound inner masks, complex paint/stroke/child silhouettes and anisotropic blur are
  counted once per effect. No-fill nodes retain the alpha-preserving first-drop filter,
  counting unsupported spread/blend and every skipped later drop or inner shadow. Shadow
  attempts use the work budget, with at most eight per node admitted. Synthetic pixel,
  counter and mutation checks cover clipping, union, transparency, order, blur and spread.
  Not verified: live Figma parity or exact rendering of the counted approximations.
