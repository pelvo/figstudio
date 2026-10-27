---
pr: 44
branch: feat/onboarding
title: feat(app): onboarding and a recents page
---

## P3 — Desktop app (`apps/figstudio`)
- [x] onboarding (2026-10-20): a welcome sheet on the first launch (the app, spaces, the
      command field) and one-time tips beside the command field, the space dots and the
      sidebar button; done when dismissed or used; Help → Welcome tour shows them again
- [x] Recents page (2026-10-20, desktop): with no file open, the files opened before as
      cards in their space colours with a picture of their page (or rows), a filter, Open,
      Show in Finder, Remove from recents; moved files marked missing

## Progress

### 2026-10-20T000000Z b212-onboarding-a-first-launch-welcome-sheet
- 2026-10-20 — Onboarding: a first-launch welcome sheet in the Arc style (three pages: the app, spaces, the command field; Skip/Back, Continue/Get started, Escape and arrow keys) and one tip at a time once a file is open, pointing at the command field, the space dots and the sidebar button. A tip is done after Got it, Skip tips, or when the person does what it shows. Progress is kept in `UiState` (older states load with nothing seen); Help → Welcome tour, the command bar and Settings → General show it again. 7 unit and 6 kittest tests; each was seen failing with its part removed. Checked by eye in the macOS app (dark and light, empty window and structure.fig, all three pages and tips). Not checked: Windows/Linux, the browser build, native-menu use while the sheet is up.

### 2026-10-20T000000Z b213-recents-page-with-no-file-open
- 2026-10-20 — Recents page: with no file open the canvas card lists the files opened before, newest first, as Arc-style cards (space gradient, a picture of the page rendered on a background thread, badge, name, when opened) or rows, with a filter, Open file…, and a right-click menu (Open, Show in Finder, Remove from recents). The list and pictures stay in the config folder, owner-only; a moved file shows as missing and toasts instead of opening. First launch shows the old empty state under the Recents title. 6 unit and 6 kittest tests (7 kittests with the slate change below), each seen failing with its part removed. Checked by eye in the macOS app, dark and light, grid, rows, menu, a missing file; Show in Finder selected the file. Not checked: Windows and Linux (the reveal call there), a list of 60 files, a very large file's picture time.

### 2026-10-20T000000Z b214-no-file-colour-the-window-is
- 2026-10-20 — No-file colour: the window is Slate instead of Lavender when no file is open, and on the Recents page it takes the colour of the card under the pointer, back to Slate off it. The test failed on both parts before each change. Checked by eye in the macOS app, dark and light. Not checked: Windows, Linux.

### 2026-10-20T000000Z b215-welcome-sheet-fix-the-page-s
- 2026-10-20 — Welcome sheet fix: the page's picture (the drawn search field on the last page) stayed fully opaque while the sheet faded out, then vanished at once; its painter set the page fade instead of multiplying it into the sheet's. A new test reads every frame of the fade and failed before the fix. Not checked by eye: the fade lasts 0.2 s and a screen recording did not catch it.

### 2026-10-20T000000Z b216-pr-44-review-comment-6288055290-recents
- 2026-10-20 — PR #44 review (comment 6288055290): Recents pictures are drawn with their own image cache, not the canvas's (no shared eviction clock, in-flight count or failed marks); a picture that finishes after Remove from recents is not written; it is queued after a link moved the file to its node's page. Only files on disk are listed (bytes dropped without a path are not), and a click opens a file that is there now, else marks it missing, whatever the page last saw. The header drops the filter, then the view toggle, then narrows Open so it stays inside a narrow card. While open, the welcome sheet takes the wheel, pinches, touches, typing and focus (a sideways swipe no longer switches spaces under it, a focused field no longer gets typed into), learns nothing from what happens under it, and stops being modal once it fades. A tip whose control is hidden (the sidebar closed) waits; the next one shows. 10 new tests, each failing before its fix (the wheel-and-pinch test passed before too: the backdrop already kept the canvas from being hovered; it guards that). Not checked: a link landing on another page (no fixture has a second visible page), by eye.

### 2026-10-20T000000Z b217-pr-44-second-review-comment-6288895045
- 2026-10-20 — PR #44 second review (comment 6288895045, P2 1 non-critical, P3 2): while the Recents page stays up it looks for moved or deleted files again each second (with the next frame), so a card's label and its menu's Open and Show in Finder follow the disk, not only a click; each picture job gets a number and only the newest for a file writes, so an older one finishing last no longer replaces it; the sidebar tip shown first with the sidebar hidden says how to show it, and tips are numbered in the order they are met ("Tip 1 of 3"). Each fix has a test that failed before it (the job one by mutation). From comment 6293348180: `Icon::Grid`/`List` no longer warn as dead code in the browser build. Browser Recents (file handles, IndexedDB) left for a follow-up PR, as that comment suggests.

### 2026-10-20T000000Z b218-pr-44-third-review-comment-6304534762
- 2026-10-20 — PR #44 third review (comment 6304534762, P2 1 critical, P3 1): the Recents page's look for moved or deleted files runs on its own thread, one at a time, and the next waits a second after the last returned, so a path that takes seconds to `stat` (a network mount, a spun-down disk) no longer stalls every frame; a look and a click agree that "there" means `is_file`, so a folder where the file was stays missing; a click drops a look begun before it. Two unit tests (a hanging probe, a folder), each caught by a mutation (the wait stamped when a look begins; `exists` for the probe); 8 repeated runs of the recents tests, no flakes.

### 2026-10-20T000000Z b219-pr-44-fourth-review-comment-6305521598
- 2026-10-20 — PR #44 fourth review (comment 6305521598, P2 2 non-critical, P3 2; approved): the Recents look is one long-lived thread, asked for the next look only once it answered, so clicks while a `stat` hangs no longer start a thread each; looks carry a generation that a click, a load (`note` now too), a failed open and the page appearing raise, and an older answer is dropped instead of replacing what those saw. The slow-disk test now clicks and loads while the probe hangs; a new test opens a file while an older look that saw it gone is running. Mutations caught: a new thread per drop, `note` not raising the generation, any answer stored; 8 repeated runs, no flakes.
