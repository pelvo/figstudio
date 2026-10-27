---
pr: 47
branch: feat/web-recents
title: feat(app,web): keep recents in the browser
---

## P3 — Desktop app (`apps/figstudio`)
- [x] Recents page in the browser (2026-10-21): the list, handles and pictures in
      IndexedDB; Chrome/Edge open a card from its file handle (picker in its folder if the
      file moved), Firefox/Safari open the picker with a toast naming the file

## Progress

### 2026-10-21T083127Z b236-browser-recents-the-browser-app-shows
- 2026-10-21 — Browser Recents: the browser app shows the Recents page when no file is open. The list, file handles and PNG page pictures live in IndexedDB (`apps/figstudio/js/recents.js`, called through wasm-bindgen). Chrome and Edge pick with the File System Access picker and reopen a card from its handle; a moved file is marked missing and the picker starts in its folder; a refused permission says to click again. Firefox and Safari: a card opens the picker and names the file. Pictures are drawn on the rayon pool. Files opened before the saved list loads stay in front; a moved file picked again replaces its missing card. Desktop store moved to `recents_disk.rs`, unchanged. Tests: 2 unit tests (merge, moved file), `web/e2e/recents.spec.mjs` (module, 5) and `recents-app.spec.mjs` (built app, 2), 3 runs in a row; mutations caught for each. Checked by eye in Chromium (both kinds) and WebKit (no handles). Not checked: Firefox (Playwright's build did not start here), real Chrome's picker and permission prompt by hand, the relay e2e (needs a Wrangler worker).

### 2026-10-21T083127Z b237-pr-47-review-comment-6306929845-p1
- 2026-10-21 — PR #47 review (comment 6306929845, P1 1, P2 2 non-critical, P3 2): a picked file's handle travels with its own pick to its tab (`Uploads`), not by name, so of two files of one name picked together the card keeps the handle of the one that opened; a late IndexedDB read no longer blanks or replaces a picture drawn since (`recents_pictures.rs`, reads stamped with the last placed job); a re-picked moved file replaces only the card it was picked for; drop handles match only the latest drop, all of its files; the page shows no welcome until the browser's saved list is read. Each fix has a test seen failing first (the two-files e2e 3/3 on the old build; the picture rules by mutation; the drop tests on the old module). The late-read race itself did not reproduce in headless Chromium (the picture is drawn before the page asks).

### 2026-10-21T083127Z b238-the-sidebar-s-shows-the-recents
- 2026-10-21 — The sidebar's + shows the Recents page instead of the file picker, desktop and browser, with files still open: the page is a place after the last space (Next space goes to the first, Previous to the last, closing a space keeps it, a dot or a card leaves it); on the page the + opens the picker as before. Window title is the app's there and the sidebar says "Recent files". Three kittests, each seen failing first; mutations of the close and space-order rules caught. Checked by eye in Chromium. Not checked by eye: the desktop app.

### 2026-10-21T083127Z b239-pr-47-review-comment-6307616595-p2
- 2026-10-21 — PR #47 review (comment 6307616595, P2 non-critical): which missing card a moved file replaces now travels with the picker's own files (`web::Origin` on the pick's `Upload`), not a latch the next open took; a cancelled relocate picker or an unrelated open of the same name no longer forgets the card and its handle. Test seen failing on the latch first; a mutation of the still-missing check caught.

### 2026-10-21T083127Z b240-recent-files-beside-the-app-the
- 2026-10-21 — Recent files beside the app: the macOS Dock icon's menu and a Windows taskbar jump list list the newest five files not known to be missing. A Dock pick opens like a card (a file gone since is marked missing with a toast and leaves the list); a jump list item starts figstudio with the file (a file gone by then is marked missing). Two approved `unsafe` blocks (`dock_mac.rs`, `jump_list.rs`), listed in AGENTS.md. Linux: none (no common dock menu). Four tests, each seen failing first; the missing filter caught by mutation. Windows checked by `cargo clippy --target x86_64-pc-windows-msvc` only; not run on Windows.

### 2026-10-21T083127Z b241-pr-47-review-comment-6308060819-and
- 2026-10-21 — PR #47 review (comment 6308060819 and approval 5594973574; P2 2 non-critical, P3 1): the workspace saved on the Recents page keeps the file the + was pressed on as active; a file without a handle (the plain picker, when the browser refuses its own) never replaces a moved card that has one; two app e2e tests lock the relocate wiring (a cancelled picker, a refused one). The session test and the handle rule failed first; the cancel e2e fails on a build where every same-named open replaces the card.

### 2026-10-21T083127Z b242-pr-47-review-comment-6308263408-inline
- 2026-10-21 — PR #47 review (comment 6308263408; inline 4333121646, 4333121654, 4333121677): a file the person removes from the jump list leaves Recents, so it stays off; removed items are read in full (32 768-unit buffer) and a refused category commits an empty list instead of keeping a stale one; files of one name beside the app show their folder, and a Dock title keeps its `&`. The Dock/jump list code is gated to macOS/Windows, which fixes the Linux clippy failure on 7cc8c51. The second-process race on `recents.json` is noted as a known limit. Three tests failed first.

### 2026-10-21T083127Z b243-one-figstudio-per-person-a-launch
- 2026-10-21 — One figstudio per person: a launch with files hands them to the figstudio already running (a per-user local socket through `interprocess`: a named pipe on Windows, a socket file in the config folder elsewhere), which opens them in its window and comes forward; the launch exits. So a jump list item opens in the same window and one process keeps `recents.json` (PR #47 thread r4333121677). A launch without files starts as before; with no answer in two seconds a launch opens the files itself; a socket left by a crash is taken over. Six tests, each seen failing first; mutations of the stale-socket takeover and of opening what arrives caught. Checked in the built macOS app (hand-over, and after a kill -9). Not run on Windows or Linux.

### 2026-10-21T083127Z b244-pr-47-review-comment-6308423295-p3
- 2026-10-21 — PR #47 review (comment 6308423295, P3 3): closing the file the Recents page came from hands its place to the one beside it, so the saved workspace keeps a neighbour as active (test failed first); the `pick` doc says only the browser's picker replaces a moved card; the refused-relocate e2e checks the picker was asked three times.

### 2026-10-21T083127Z b245-pr-47-review-comment-6308564236-threads
- 2026-10-21 — PR #47 review (comment 6308564236; threads 4333247613, 4333248314): a jump list Windows refuses is aborted (the shown one stays) and tried again without one file at a time, so an unread removal no longer leaves the list blank; the file whose removal unblocks it leaves Recents. Labels of same-named files whose folders share a name or have none use the folder's whole path. Two tests failed first; a mutation without the retries caught. Not run on Windows.

### 2026-10-21T083127Z b246-pr-47-review-comment-6308753668-p1
- 2026-10-21 — PR #47 review (comment 6308753668, P1 2, P2 3, P3 2): the single instance is a lock file and an inbox folder in the person's config folder instead of a local socket. Windows named pipes had no timeouts in `interprocess` (the hand-over never worked there) and a guessable global name; both are gone, as are the stale-socket takeover race and the socket's umask. The lock is taken before the window; a request is taken by one rename, so a launch that gives up and the running figstudio never both open it; paths that cannot be made absolute or written stay with the launch. `interprocess` dropped. The window takes requests itself (a thread only wakes it), so a hung window leaves the files to the launch (comment 6308788394). Mutations caught: no lock, a timeout taken as success, a window that never takes. Not run on Windows.

### 2026-10-21T083127Z b247-pr-47-review-comment-6308980226-p2
- 2026-10-21 — PR #47 review (comment 6308980226, P2 1, P3 1): a jump list session error aborts that session and stops the tries instead of counting as a refusal (a later shorter list could be committed and the removal lost); only a taken list that held files names the file left out. The other points listed there were fixed in a0e85cd. Two tests failed first (they did not compile against the old `publish`); mutations caught.

### 2026-10-21T083127Z b248-pr-47-review-comment-6309105207-p2
- 2026-10-21 — PR #47 review (comment 6309105207, P2 4, P3 1): a launch counts its files as taken only when its request is known gone (a stat error no longer counts) and retries taking it back; it waits for the running figstudio to be up (`ready`, up to 20 s) before the 2 s for its window, so a slow first start no longer leaves two windows; requests over 1 MiB stay with the launch; on Unix the folder and inbox are `0700`, the lock `0600`, and a linked folder is refused; a figstudio taking the lock clears `.part` files and re-queues `.taken` ones. Five tests failed first; four mutations caught.

### 2026-10-21T083127Z b249-pr-47-review-comment-6309205931-p2
- 2026-10-21 — PR #47 review (comment 6309205931, P2 1): a jump list retry never publishes an empty list (with one file refused, that cleared Windows' removed list and the file could come back); the list shown stays. The other open items listed there were fixed in 155b67d. Test failed first.

### 2026-10-21T083127Z b250-pr-47-review-comment-6309421578-threads
- 2026-10-21 — PR #47 review (comment 6309421578; threads r4333607800, r4333608567, r4333608553): a launch that gives up opens its files only once it has deleted its request or moved it aside (retried up to 5 s), so they never open twice; `ready` holds the lock holder's id (`holder`), so a mark left by an ended figstudio does not skip the start wait; a taken request is read up to five times. Three tests failed first; three mutations caught.

### 2026-10-21T083127Z b251-windows-a-launch-that-hands-its
- 2026-10-21 — Windows: a launch that hands its files over passes the foreground to the running figstudio (`AllowSetForegroundWindow` with the pid from its `holder` id), so its window comes forward instead of only flashing; one more `unsafe` block, approved by the owner and listed in AGENTS.md. The pid parsing is tested (failed first); the call itself was checked only with clippy for `x86_64-pc-windows-msvc`. Not run on Windows.

### 2026-10-21T083127Z b252-pr-47-review-comment-6309697459-p2
- 2026-10-21 — PR #47 review (comment 6309697459, P2 2, P3 1): a request a launch cannot take back within 5 s stays queued and the launch leaves it, so the files never open twice; the holder writes its id before clearing an ended figstudio's marks and removes its own on a normal exit (a stale pair is left only by a crash); a request unreadable when taken goes back for a later look instead of five reads on the window's thread. Three tests failed first; three mutations caught. Remaining: a launch in the moment between another's lock and its id write after a crash.

### 2026-10-21T083127Z b253-pr-47-review-comment-6309822879-threads
- 2026-10-21 — PR #47 review (comment 6309822879, threads 4333785758, 4333785776): the foreground goes only to the running figstudio's pid when its `ready` mark matches `holder`, never `ASFW_ANY` (`foreground_for`, tested); on Windows the window asks for the foreground itself with `SetForegroundWindow` on its HWND (`foreground_win.rs`, owner approved) instead of egui's Focus, whose Alt `SendInput` can cancel the right. Restored `the_holders_process_is_read_from_its_id`, dropped by mistake in 198c74e. Checked with clippy for Windows only; not run there.

### 2026-10-21T083127Z b254-pr-47-review-comment-6310070806-threads
- 2026-10-21 — PR #47 review (comment 6310070806; threads 4333909488, 4333910542, 4333910544, 4333910557): a launch that cannot take its request back marks it cancelled and opens the files itself, and a window drops a cancelled request (no files stuck behind a holder that never takes, none opened twice); a request that is not text is kept aside, not put back every frame; a figstudio that cannot write its `holder` id takes no files and drops the lock; the stat-error test is back (lost in 198c74e). Four tests failed first; four mutations caught.

### 2026-10-21T083127Z b255-pr-47-review-comment-6310269962-p2
- 2026-10-21 — PR #47 review (comment 6310269962, P2 1): on Windows a minimized window is restored (`ShowWindow(SW_RESTORE)`) before it asks for the foreground, on its own thread, so the right a launch passed on is not spent on an iconic window. Windows-only; checked with clippy for `x86_64-pc-windows-msvc`, not run there.

### 2026-10-21T083127Z b256-pr-47-review-comment-6310400011-p2
- 2026-10-21 — PR #47 review (comment 6310400011, P2 2, P3 2): a request a launch gives up on is decided by one atomic `create_new` of `{id}.decided`, which the launch and the window both try, so the files open in exactly one place (the cancel mark could make them open nowhere: a stat error, or the window's rename landing first); a request that cannot be opened goes back before anything is decided; TESTING.md and `hand_over`'s doc match the code again. Three tests failed first; mutations caught (launch ignores the mark, window opens anyway).

### 2026-10-21T083127Z b257-pr-47-review-comment-6310537092-p2
- 2026-10-21 — PR #47 review (comment 6310537092, P2 critical): handed-over files are taken in `App::logic`, which eframe runs also for a minimized window (it skips `ui` then), so a minimized figstudio restores and opens them instead of the launch giving up into a second window. Test calling only `logic` failed first.

### 2026-10-21T083127Z b258-pr-47-review-comment-6310786888-p2
- 2026-10-21 — PR #47 review (comment 6310786888, P2 2, P3 1): the window decides a request only after reading and parsing it (an unreadable one goes back undecided); when its launch decided first, the mark goes only once the request is gone; the sweep keeps a mark while its request waits. Three tests failed first; mutations caught.

### 2026-10-21T083127Z b259-pr-47-review-comment-6311117316-p3
- 2026-10-21 — PR #47 review (comment 6311117316, P3 2, after approval): tests now cover a read that fails after the open (put back undecided, opened on a later look; a `CANNOT_READ` test seam) and a mark kept while its request waits as `.taken`; both catch the mutations named in the review.
