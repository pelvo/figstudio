---
pr: 14
branch: chore/cloudflare-deploy
title: feat(web): deploy to Cloudflare with a selectable CJK font
---

## Progress

### 2026-10-13T125639Z b090-browser-cjk-font-is-now-a
- 2026-10-13 — Browser CJK font is now a choice: View → CJK font picks Noto Sans CJK JP, SC, TC, HK or KR (`scripts/web-font.sh` writes `fonts/cjk-<code>.otf` for each; KR adds Hangul). The default follows the browser language (JP when nothing matches) and the choice is kept in localStorage; changing it replaces the loaded font. Tests: language → variant mapping (seen failing with a broken mapping) and font replacement. check.sh passed; web-build.sh wrote all five fonts (fontTools: right face names, kana and ideographs present, Hangul only in KR). Not verified: clicking the menu in a real browser.

### 2026-10-13T125639Z b091-cjk-font-review-fixes-a-file
- 2026-10-13 — CJK font review fixes: a file whose only CJK is Hangul switches to KR for the session (`theme::cjk_need`; test `hangul_without_other_cjk_asks_for_the_korean_face`, seen failing); `web-font.sh` refuses a non-`.ttc` override and checks every output's full face name (tried: a single-face `.otf` and wrong face indexes both fail); `web-build.sh` clears `dist/fonts` so the old `cjk.otf` is not deployed. cjk tests, native clippy and web-build passed. Not verified: full check.sh, Hangul-only file in a real browser.

### 2026-10-13T125639Z b092-cjk-font-second-review-cjk-punctuation
- 2026-10-13 — CJK font second review: CJK punctuation and full-width forms no longer stop a Hangul-only file from getting KR (`CjkNeed::Shared`, every face has them); the session KR override is separate from the saved menu choice (`Inbox::korean_override`), the menu shows the saved choice plus a note, and any pick (the current one too) is saved and ends the override; the font-replacement test clears texture deltas so it passes in debug builds too. `web/wrangler.jsonc` is now a local, git-ignored file (it names one account's domain). Tests: `hangul_without_other_cjk_asks_for_the_korean_face` seen failing without the punctuation rule and with a fold that stops at Hangul. theme/cjk tests (debug), native clippy and web-build passed. Not verified: full check.sh, the menu note in a real browser.
