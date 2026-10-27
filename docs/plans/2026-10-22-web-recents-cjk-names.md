---
pr: 51
branch: fix/web-recents-cjk-names
title: fix(web): draw cjk file names on the recents page
---

## Progress

### 2026-10-22T000000Z b274-browser-recents-page-draws-cjk-file
- 2026-10-22: Browser Recents page draws CJK file names: with no file open, no document had asked for the CJK font, so a Japanese name showed as boxes. The page now asks for it from its card names (and folders in list view), once per level, so a View menu pick is not undone. Native test for the names' need; desktop kittest checks each character of a Japanese card name has its own glyph in the regular and semibold families (failed with the installed CJK face removed, passes on macOS; skipped on a machine with no CJK font). Real Chromium: before, the card showed boxes and fetched no font; after, it fetched `cjk-jp.otf` and drew the name (a system font stood in for the Noto subsets, which were not built here). Full gate passed. Not checked: Windows and Linux desktop, Safari and Firefox.

### 2026-10-22T000000Z b275-pr-51-review-comment-6332058935-two
- 2026-10-22 — PR #51 review (comment 6332058935, two P2s): Recents names with Hangul next to kana or ideographs now ask for KR (`cjk_need_of` returns `HangulOnly` when any name has Hangul; `names_rank` puts it above `Any`), and KR that only Recents names asked for no longer pins the session: `cjk::KoreanFor` records why KR is in use, and a file that needs the saved face takes it back. Checked: the theme and `KoreanFor` tests (the mixed-list assertion fails on the previous code); native clippy; browser-build clippy (pinned nightly, wasm32). Not checked: a real browser run.

### 2026-10-22T000000Z b276-pr-51-re-review-comment-6333020179
- 2026-10-22 — PR #51 re-review (comment 6333020179, two P2s on the fix above): after a Japanese file took the saved face back, Recents never asked for KR again, so Hangul cards stayed boxes after that file closed; and "nothing asked" also meant "a file asked for the saved face", so Recents opened with `+` could switch an open file to KR. `cjk::FaceFor` now tells them apart (`Unasked`, `File`, `Picked`, `HangulFile`, `HangulName`): an open file's need wins over the names, a menu pick is never undone by them, and when the last file closes the Recents page asks again. Checked: the `FaceFor` transition test; native and browser-build clippy. Not checked: a real browser run.

### 2026-10-22T000000Z b277-pr-51-review-thread-4344828672-p2
- 2026-10-22 — PR #51 review thread 4344828672 (P2): the reset on close only ran when a Recents paint had seen open files, and the Recents page is not painted while a file is current, so closing the last file still left Hangul cards as boxes. The face state is now `cjk::Face` (reason plus what the names asked), and `Face::recents` resets a file's reason on any Recents paint with no file open. Checked: `files_decide_the_face_while_open_and_recents_names_after` drives `Face` through the reported cycles and fails with the reset removed; native and browser-build clippy.

### 2026-10-22T000000Z b278-the-recents-names-now-ask-for
- The Recents names now ask for the CJK face again once none of the files that chose the saved face is open, also while another tab without CJK stays open (it used to wait until every tab closed). `cjk::Face` records which files asked; the test walks a Japanese file closing beside a Latin one and failed before the change.

### 2026-10-22T000000Z b279-the-cjk-face-records-the-tabs
- The CJK face records the tabs, by id, whose files asked for the saved face, so a closed file is not taken for another open under the same name (browser paths are display names).
