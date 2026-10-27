- 2026-10-11 — A16: a frame that clips its content now draws its stroke above its children,
  so a table's outer border is no longer hidden by the cells that fill it; non-clipping frames
  keep the stroke below. Test in `stroke_order.rs`. Not verified: live compare in Figma. The
  grey grid lines and header fill are the file's own stored paints and are left as stored.
