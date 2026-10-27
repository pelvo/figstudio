---
pr: 27
branch: feat/figma-link-open
title: feat(app,mcp): open figma.com links from local copies, add app icon
---

## P3 — Desktop app (`apps/figstudio`)
- [x] paste a figma.com or `figstudio://` URL into the command bar, then Enter or click
      **Open Figma link** to open the matching local copy and reveal its node; use the existing
      link window for progress, ambiguous matches and failures
- [x] File → Open Figma link… (Cmd+Shift+L by default): paste a figma.com link; the matching tab, or the
      `<file name>.fig` in `~/Downloads` (`FIGSTUDIO_LINK_DIRS`, or a chosen folder), is opened
      and the node selected and fitted; when several files match equally, the window lists them

## P7 — Offline MCP server (`apps/figstudio-mcp`, plan in `docs/MCP-PLAN.md`)
- [x] `reveal_node` with a figma.com link whose file no tab shows: the app opens the local copy from its
      link folders, then reveals the node (`Command::OpenLink`)

## Progress

### 2026-10-19T000000Z b160-figma-links-file-open-figma-link
- 2026-10-19 — Figma links: File → Open Figma link… (Cmd+L) and `reveal_node` open the local copy a figma.com link names (newest `<name>.fig` in `FIGSTUDIO_LINK_DIRS` or `~/Downloads`, ` (N)` download suffix ignored) and reveal the node. New tests (mcp-core `local_copy`, MCP live open-link, app `link_open_tests`, `mcp_host_tests`) each seen failing without their code. Not verified: a real MCP client session against the running app, the native menu accelerator in a real window, Windows/Linux folder dialogs.

### 2026-10-19T000000Z b161-figma-links-review-fixes-reveal-node
- 2026-10-19 — Figma links review fixes: `reveal_node` checks the node ID before asking the app to open a file (a link without a valid `node-id` no longer opens one), keeps the instance-path note on the open path, and matches a figma.com link to the full-path keys of two tabs that share a stem; `Tab::show_node` makes a page ID switch page for `reveal_node`, the browser relay and Open Figma link alike, and the open answer reads `revealed 1:2 in tab 3 (opened Name.fig)`. Each new test failed with its fix reverted. Not verified: a real MCP client against the running app.

### 2026-10-19T000000Z b162-figma-links-manual-check-on-macos
- 2026-10-19 — Figma links, manual check on macOS: with the app's Agent access on, an MCP client over HTTP called `reveal_node` with a real figma.com link while no file was open; the app opened the local copy from `~/Downloads` in about 4.5 s, selected and fitted the node, and `list_open_tabs`/`get_selection` saw the new tab. Repeating it revealed in the open tab; links without or with a bad `node-id` failed without opening anything; an unknown file named the searched folder; a page ID switched page. Native Cmd+W and Cmd+L worked; pasting the link into the window opened the file and selected the node. Not verified: Windows and Linux.

### 2026-10-19T000000Z b163-figma-links-second-review-a-link
- 2026-10-19 — Figma links, second review: a link is shown only in the file it names. Candidates (open tabs and local copies alike) are ranked key over name, the name compared as is before the ` (N)` suffix is dropped; a tie lists the files in the window to pick and fails an MCP call, never guessing by date. In the app every figma.com `reveal_node` is resolved by the app with no active-tab fallback; in the browser bridge the link must name exactly one tab. MCP answers about links carry no paths or file names (`LinkError::public`). `McpHost::sync_then_answer` publishes the snapshot before answering; an HTTP test drives `reveal_node` through the real app. Each fix has a test that failed with it reverted (the HTTP test alone could not catch a swapped order, so a unit test does). Not verified: Windows and Linux.

### 2026-10-19T000000Z b164-figma-links-the-link-folder-search
- 2026-10-19 — Figma links: the link-folder search runs on a background thread and is polled each frame, so the window keeps drawing while a folder is slow or macOS asks for permission to read `~/Downloads` (a live check had frozen behind that prompt and the MCP call ended with "the app did not respond"). After 20 s the link fails with a message naming that likely cause. `a_slow_folder_does_not_block_the_window_and_the_link_fails_with_a_reason` failed with the search moved back onto the UI thread, and again with the timeout removed. Live on macOS: behind the prompt `reveal_node` failed after 20 s with that message; once access was allowed, a real link opened its local copy and revealed the node (about 2.4 s for four calls), an unknown file failed without touching the open tab, and a bad node ID failed. Not verified: Windows and Linux.

### 2026-10-19T000000Z b165-macos-bundle-icon-scripts-macos-app
- 2026-10-19 — macOS bundle icon: `scripts/macos-app.sh` builds `figstudio.icns` from `apps/figstudio/assets/icon.png` (sips + iconutil, 16–512 px with 2x up to 256) and sets `CFBundleIconFile`, so Finder and Launchpad show the app's icon, not the generic one. No `.fig` document type: winit 0.30 does not pass macOS open-file events to the app, so a double-click would only start an empty window (taking them needs an unsafe AppKit delegate). Verified: built under /bin/bash 3.2, `codesign --verify --deep --strict` passed, the `.icns` holds 9 sizes, and `NSWorkspace` renders the bundle with the icon, the generic icon without the key and file. Not verified: the release workflow on GitHub.

### 2026-10-19T000000Z b166-figma-links-third-review-a-figma
- 2026-10-19 — Figma links, third review: a figma.com link that several files match equally now fails every `file_for` tool (`get_metadata`, `get_screenshot`, …) with "pass fileKey" instead of taking the first tab. Link search: a new window link replaces the window's older search, a listing still blocked past 20 s makes new links fail at once (no more blocked threads), `start_search` requests the frame that watches it, timeouts are checked before results so a late one is never used, the window keeps its text while searching and after a miss, which copies are open tabs is resolved on the search thread (no `canonicalize` on the UI thread), and the timeout text names macOS permission only on macOS. Each change has a test that failed with it reverted. Not verified: the Linux/Windows timeout text on those systems. Seen in passing, not changed: with a tab open the app sends the window title every frame, which keeps it repainting.

### 2026-10-19T000000Z b167-figma-links-fourth-review-tabs-keep
- 2026-10-19 — Figma links, fourth review: tabs keep the path their loader resolved (`Tab::real_path`), and each local copy carries its own resolved path, so a file opened while the folders are listed is recognised as that tab instead of being loaded again (`plan` uses the tabs open when the result arrives). From the window, a node ID also replaces a running search, a repeated Go starts no thread, a link refused because a listing is blocked leaves the running search alone, and closing the window cancels its search. `frame_ui` polls searches before taking MCP links. The slow-folder test expects the permission hint on macOS only (it failed on Linux). `scripts/macos-app.sh` removes its icon temp folder from an EXIT trap (seen leaking one folder with a failing `iconutil` stub, none after). Each app change has a test that failed with it reverted, except the poll order in `frame_ui`, which has none. Not verified: the Linux and Windows runs.

### 2026-10-19T000000Z b168-figma-links-fifth-review-a-listing
- 2026-10-19 — Figma links, fifth review: a listing that comes back while a file is loading is held until the loads end, so a file opened just before (no tab yet) is not loaded a second time under the listed spelling; the timeout stops counting once the listing is back. A node ID from the window replaces the running search even when the file lacks that node. Both tests failed with their change reverted. Not changed: a tab whose path the loader could not resolve keeps its open path, so it matches a copy only under the same spelling.

### 2026-10-19T000000Z b169-figma-links-sixth-review-a-returned
- 2026-10-19 — Figma links, sixth review: a returned listing is held only while a file whose name the link matches is loading (unrelated loads no longer hold it), a held listing still fails at the 20 s timeout with "still opening", and a copy whose load just failed is not decoded again (the worker sends the resolved path with `Loaded::Err`; the link answers `LoadFailed`). Each change has a test that failed with it reverted.

### 2026-10-19T000000Z b170-figma-links-seventh-review-a-copy
- 2026-10-19 — Figma links, seventh review: a copy whose load just failed is no longer removed before ranking (that let a name match, or the other file of a tie, be opened in its place); it is ranked as before and the link fails only when it is chosen. A failed path the worker could not resolve is matched against both the listed and the resolved spelling with `.`/`..` folded by text. The test failed with either change reverted.

### 2026-10-19T000000Z b171-figma-links-eighth-review-choosing-another
- 2026-10-19 — Figma links, eighth review: choosing another folder forgets the listings of the old folders, so a listing stuck on an unreachable share no longer refuses every later link for the session. Listings are stamped when they finish, so one that finished in time is used even if it is polled after the deadline (one that finished late still never is). An MCP request still waiting for its copy to load is answered "still opening" at 55 s, before the server's 60 s limit, and the file still opens and shows the node. Each change has a test that failed with it reverted.

### 2026-10-20T060007Z b172-figma-links-ninth-review-choosing-a
- 2026-10-20 — Figma links, ninth review: choosing a folder, the same one included, gives up on every running listing, so a share that hung and has woken can be retried without a restart; at most three given-up listings may still be waiting before links are refused. `Command::OpenLink` carries the tool's send time and the 55 s early answer counts from it, so time queued before a frame takes the request is inside the budget. The link-search notes in IMPLEMENTATION.md now describe the drain-then-deadline order. Each change has a test that failed with it reverted.

### 2026-10-20T060007Z b173-figma-links-tenth-review-lexical-no
- 2026-10-20 — Figma links, tenth review: `lexical` no longer lets a second `..` erase the first (`../../a` stays distinct from `a`; `/..` is `/`). New tests: a key tie where one file just failed still answers "several files match" and opens neither, and a failed best match reported with its resolved path (the case `local_copies` produces) does not open the worse match. Each failed with its fix or check reverted. Not covered: a listed path that cannot be resolved against a failure reported by its resolved path (the listing always yields resolvable paths).

### 2026-10-20T060007Z b174-figma-links-eleventh-review-the-cap
- 2026-10-20 — Figma links, eleventh review: the cap on given-up listings counts only stuck ones (past the 20 s timeout and still not back), so a few links in flight when a folder is chosen no longer refuse the new folder's first search. The MCP budget (55 s from the caller's send time) now also answers a request whose folders are still being listed, not only one whose copy is loading. Each change has a test that failed with it reverted; the send-time test now checks the waiting load's deadline directly (6/6 runs).

### 2026-10-20T060007Z b175-figma-links-cursor-review-of-f980af4
- 2026-10-20 — Figma links, cursor review of f980af4: `lexical` drops a `..` only above a root, so Windows' rootless `C:..\KEY.fig` no longer folds to `C:KEY.fig` (a Windows-only test type-checks with a Windows cross `cargo check`; not run). The resolved-path test lists its copy through a symlink, so it needs the resolved-path comparison on every Unix (it passed on Linux without it before); it failed with that comparison removed.

### 2026-10-20T060007Z b176-figma-links-twelfth-review-an-mcp
- 2026-10-20 — Figma links, twelfth review: an MCP search whose budget runs out while it is held for a matching file's load is answered "still opening" and kept (marked `answered`, so it is not taken for the window's search), and still shows the node once the load ends; before, it was dropped and the node never shown. An end-to-end test drives a request past its budget through the search into its load, expects "still opening" and the node shown. Each change has a test that failed with it reverted (5 mutations); both new tests passed 6/6 runs.

### 2026-10-20T060007Z b177-figma-links-thirteenth-review-an-answered
- 2026-10-20 — Figma links, thirteenth review: an answered held search is no longer cut off by its 20 s search deadline (it waits for the load like a waiting load), answered searches and loads end quietly (no "Opening…" or node-missing toast, no window, no second answer), and the window's "Searching…" label counts only its own search. The end-to-end budget test holds the listing instead of sleeping and checks for a second answer. Each change has a test that failed with it reverted (5 mutations); the timing tests passed 6/6 and 3/3 runs.

### 2026-10-20T060007Z b178-figma-links-approval-nits-reaching-the
- 2026-10-20 — Figma links, approval nits: reaching the abandoned-listing cap now tells the caller to wait or restart figstudio, including after choosing a healthy folder. The cap regression failed without that guidance and passes with it; the new public message is covered by the redaction test. The already-answered timeout toast was already prevented by the answered-search guard; removing that guard made the existing regression fail. Not verified: recovery against a real hung network share.

### 2026-10-20T060007Z b179-cap-message-follow-up-preserve-the
- 2026-10-20 — Cap-message follow-up: preserve the macOS permission hint alongside wait/restart guidance, clarify current-folder timeout versus abandoned-listing cap, and assert the exact cap error before and after switching folders. The platform-message test failed without the permission hint and passes with it. Live permission-dialog recovery was not rerun.

### 2026-10-20T060007Z b180-rebase-integration-with-the-arc-ui
- 2026-10-20 — Rebase integration with the Arc UI and browser proxy: preserve settings, command-bar shortcuts, tab colours, space styles and the link-open hooks. Open Figma link now uses configurable Cmd+Shift+L so Cmd+L stays with the command bar; its window reads the current theme palette. The shortcut regression failed with the conflicting Cmd+L binding and passes after restoration. Native menu interaction and the combined UI were not manually exercised.

### 2026-10-20T060007Z b181-command-bar-links-on-pr-27
- 2026-10-20 — Command-bar links on PR #27: a pasted Figma or figstudio URL offers **Open Figma link**; Enter or a click submits its saved URL through the existing link opener. The link window keeps progress, ambiguous choices and failed input; ordinary searches remain searches. Five regressions failed before implementation and pass after it. Paste and Enter in one frame uses raw input and fails if the post-edit result refresh is removed. All 318 app tests passed in fast and debug profiles. Real native-menu paste and browser interaction were not manually exercised.

### 2026-10-20T060007Z b182-pr-27-queued-review-comment-6282294066
- 2026-10-20 — PR #27 queued review (comment 6282294066): an answered search no longer schedules deadline repaints while it waits for a loader. Failed loads now belong to each search, so an old held search cannot refuse a later retry of a repaired file. The two regressions failed before the fix. Added coverage for answered searches ending with a failed best copy, a tie or a missing node; all three tests fail if quiet handling is bypassed. All 52 link tests pass and app clippy is clean. A real hung file load and an idle native window were not manually exercised.

### 2026-10-20T060007Z b183-pr-27-command-bar-review-comment
- 2026-10-20 — PR #27 command-bar review (comment 6283187553): only parsed links with a file hint become link actions; file-less Figma URLs stay searches and cannot reveal nodes in an unrelated active file. Same-frame edits preserve clicks only when the row's target is unchanged. The shortcut test now presses both command-bar and Open Figma link bindings and checks their windows. Three regressions failed before implementation; all 55 link tests pass. Missing-binding and stale-click-guard mutations each fail their dedicated test. Native clipboard, menu and browser interactions were not manually exercised.
