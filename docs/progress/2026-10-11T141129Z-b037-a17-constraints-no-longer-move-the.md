- 2026-10-11 — A17: constraints no longer move the in-flow children of an auto-layout frame
  that an instance resized; auto layout keeps them at the padding, so a label in a
  hug-width chip stays centred instead of drifting left. Absolute-positioned children still
  follow constraints. Test in `auto_layout.rs`. Not verified: live compare in Figma. A full
  re-run of auto layout for instances without derived transforms was tried and dropped: it
  moved stored positions that already follow property defaults (see the report).
