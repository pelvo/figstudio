---
branch: fix/settings-note-and-a11y-names
pr: 81
title: fix(app): keep settings notes clear of wide controls and name every control
---

## P3 — Desktop app (`apps/figstudio`)
- [x] A settings note never runs under a wide control: when too little room is left beside
      the control (Text size's chips), the text takes the full width and the control a line
      under it (`a_settings_note_never_runs_under_a_wide_control`)
- [x] Every control of the motion player and of the Properties action buttons has a name and
      an automation id (`every_control_of_the_motion_player_has_a_name`)
- [x] A canvas layer named with spaces only is named by its kind
      (`a_layer_without_a_name_is_named_by_its_kind`)
- [x] Looked at the Settings → General tab and the motion player in the app

## Progress

### 2026-11-09T020909Z settings-note-and-a11y-names
- What changed and how it was checked: found in release QA for v0.3.0. Settings → General's
  Text size note was squeezed to three lines and ran under the size chips (`setting` floors
  the text's wrap at 80 pt, wider than what the chips leave); a row whose control leaves less
  than 160 pt now puts the text across the row and the control on a line under it (test
  measured the note 2 pt from the 100% chip before, red; green after; the existing
  `a_setting_s_note_stays_clear_of_its_control` still passes). The motion player's
  click-catching bar was an unnamed control and its Scrub slider and the Properties action
  buttons (`quiet_button`: Play, Export…, Copy CSS animation, Choose its zip…, Remove mapping)
  had no automation id; the bar is hidden from the tree as the tool pill's is, Scrub is
  `slider.scrub` and Properties buttons are `properties.<slug>`. Canvas layers named with
  spaces only had a blank name; they are named by their kind. Each test seen red first.
  `scripts/check.sh` passes. Looked at both in the app (macOS).
- Not verified: Windows and Linux; a screen reader session; the browser build (same code).
