- 2026-10-12 — Text with an inside/outside stroke: stored `strokeGeometry` (a centred band of twice the
  weight) is now clipped to the glyph outlines like box shapes, instead of drawn unclipped, which
  made thin grey text look bold and smeared. Test `stored_inside_text_stroke_is_clipped_to_the_glyphs`
  (seen failing without the fix). check.sh passed; checked against a private file render.
  Not verified: outside text strokes on a real file.
