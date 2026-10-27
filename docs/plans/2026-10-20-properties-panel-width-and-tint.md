---
pr: 38
branch: fix/properties-panel-width-and-tint
title: fix(app): keep the properties panel resizable and tint it per space
---

## Progress

### 2026-10-20T080525Z b184-properties-panel-a-long-value-an
- 2026-10-20 — Properties panel: a long value (an instance's main component name with all its variant properties) is truncated with the full text on hover instead of widening the panel, so the panel can be dragged narrower again and the value no longer overlaps its key; the Properties card and its wells now take the space's colour (`Palette::on_tint`), crossfading with the window. Tests seen red first (`a_long_value_does_not_widen_its_row`, `properties_card_takes_the_space_colour_and_stays_readable`, which also checks text contrast on every preset and several custom colours); release app checked by eye on macOS dark with a preset space. Not verified: a long name in the running app (no public fixture has one), light mode by eye, Windows/Linux

### 2026-10-20T080525Z b185-pr-38-review-comment-6285375310-the
- 2026-10-20 — PR #38 review (comment 6285375310): the Properties wells keep their neutral step from the card (about 1.10:1) instead of flattening to about 1.05:1 when both took the same tint; an app test checks the rendered card and section wells use the tinted colours; the `kv` test now checks the value sits beside its key and shows its full text on hover. Each new check failed against its mutation (well write removed, old well mix, no truncation) and passes with the fix; light and dark checked by eye on macOS. The note that other rows can still widen the panel did not hold: a 2000-character unbroken run wraps inside a 200 px column for the wrapped header and the text preview
