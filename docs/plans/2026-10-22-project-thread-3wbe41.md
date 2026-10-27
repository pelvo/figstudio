---
pr: 53
branch: claude/project-thread-3wbe41
title: test(cli): keep the cjk web-font test independent of the emoji fonts
---

## Progress

### 2026-10-22T000000Z b327-scripts-check-sh-failed-on-any
- 2026-10-22 — `scripts/check.sh` failed on any machine with Noto Sans CJK and fontTools but no Noto Symbols/Math/Emoji: `writes_all_five_faces_and_the_license` (apps/figstudio-cli/tests/web-font.rs) skipped only without CJK or fontTools, then ran web-font.sh, whose symbol group (added in #46) needs four more fonts. Recent commits passed only with `FIGSTUDIO_ALLOW_MISSING_SYMBOLS=1` or `FIGSTUDIO_EMOJI_FONT`. The test is about the CJK group, so it now sets the symbol group's own opt-out. Checked on Linux with fonts-noto-cjk and fontTools, no symbol fonts: failed before ("missing Noto Sans Symbols"), passes after with the real CJK subset. Not checked: macOS.

### 2026-10-22T000000Z b328-review-on-the-web-font-test
- Review on the web-font test fix: the opt-out also hid a broken symbol subset on a machine that has the fonts. The test now accepts a skipped symbol group only when the script reports a missing font, and otherwise checks that symbols, symbols2, math and emoji .ttf were written. With all four sources pointed at a non-Noto face the new check failed and the previous one passed.
