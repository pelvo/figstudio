---
pr: 28
branch: feat/arc-redesign
title: feat(app): redesign the UI in the Arc browser style
---

## P3 — Desktop app (`apps/figstudio`)
- [x] Arc-style redesign (2026-10-19): per-file gradient (light and dark, follows the system,
      View → Appearance), sidebar with window controls, command field, pages, layers and a
      file switcher; frameless window on macOS; the canvas as a rounded card with tool and
      status pills; Properties as a card; ⌘L command bar (commands, files, pages, layers);
      hidden sidebar peeks from the left edge; zoom jumps ease around their anchor; springs
      and fades, View → Reduce motion
      (the browser follows `prefers-reduced-motion`)
- [x] spaces (2026-10-19): per file a preset or custom colour, an emoji or icon, a name;
      edited in Arc-style popovers from a space dot's menu (or the Space menu / command bar),
      saved per file on this computer, never in the `.fig`; two-finger swipes over the
      sidebar move between spaces
- [x] ⌘F finds text on the page (inside instances too), highlighted; ⌘L opens the command bar
- [x] Settings (⌘,) in Arc's layout: General (appearance, Reduce motion, translucent sidebar,
      panels), Canvas (renderer for the next start, frame names, scroll wheel pans or zooms),
      Shortcuts (searchable, keycaps; every shortcut can be changed, removed or reset),
      Agents (agent access), About
- [x] the window, panel widths, sidebar state and appearance survive a restart (eframe
      persistence)
- [x] macOS vibrancy (translucent sidebar, 2026-10-19): an `NSVisualEffectView` behind the
      content view of a transparent window (`vibrancy.rs`; one `unsafe` block, approved by the
      owner); the gradient is 60% opaque over it. Checked by hand in light and dark

## Progress

### 2026-10-19T000000Z b130-arc-style-ui-redesign-chrome-rs
- 2026-10-19 — Arc-style UI redesign (`chrome.rs`, `command.rs`, `anim.rs`, `icons.rs`, new `theme.rs`): per-file tinted gradient in light and dark, sidebar (window controls, command field, pages with a sliding pill, layer tree with rotating chevrons, file switcher dots), frameless macOS window, rounded canvas card (tiles cut at the corners), tool / status pills and a warning chip, Properties card, ⌘K command bar, peeking sidebar, eased zoom jumps, springs and fades with Reduce motion. New tests seen red first (the UI tests' labels and actions did not exist; the clip, tint, contrast, score and zoom-anchor tests failed on purposely broken code); 192 app tests, `cargo check` for wasm32, full check.sh; screenshots checked in light and dark on macOS (command bar, peek, hidden sidebar, zoom easing). Not verified: Linux and Windows look, the browser build in a real browser, window dragging from the sidebar by hand. Vibrancy followed the same day (next line)

### 2026-10-19T000000Z b131-macos-vibrancy-the-window-is-transparent
- 2026-10-19 — macOS vibrancy: the window is transparent and an `NSVisualEffectView` (Sidebar material, BehindWindow, following View → Appearance) sits in the frame view below the content view, so it no longer covers wgpu's Metal layer the way `window-vibrancy` did; the gradient is painted at 60% over it. One `unsafe` block (raw `NSView` → reference, `superview`), approved by the owner; AGENTS.md rule 10 lists it. Tests: the gradient's alpha (seen red with the alpha ignored) and no blur and no panic without a window (seen red with an unwrap). Checked by hand: light and dark screenshots show the blurred desktop through the sidebar. Not verified: an older macOS, "Reduce transparency" on, full screen

### 2026-10-19T000000Z b132-spaces-each-open-file-s-space
- 2026-10-19 — Spaces: each open file's space can have a preset or custom colour (custom ones keep text readable, `theme::space_tint`), an emoji or icon badge and a name, edited in the Edit space window (File menu, sidebar space header, a file dot's menu) and shown in the sidebar, file dots, title strip and command bar. Saved per file on this computer (`spaces.rs`: config-folder JSON via `dirs`, localStorage in the browser; by content key, then file name), never written into the `.fig`. Tests seen red first: store lookup / tidy / restart / cap, custom-colour contrast, emoji drawable, editor saves and a reopened file gets its style. Not verified: typing a name or emoji by hand (IME), the custom colour picker by hand, Windows/Linux config paths, the browser store

### 2026-10-19T000000Z b133-arc-style-space-editing-and-more
- 2026-10-19 — Arc-style space editing and more: the sidebar's space header is gone; a space dot's right-click menu (macOS menu look, `theme::menu_style`, now used by every menu) offers Change space icon / Rename space / Edit theme colour (popovers above the dot: icon grid, in-place name with Escape to go back, a colour pad with presets, Auto and System/Light/Dark), Reset and Close space, also in a new Space menu. Two-finger sideways swipes over the sidebar switch spaces (`swipe.rs`). ⌘L opens the command bar (was ⌘K). ⌘F finds text on the page, including text inside instances with overrides (`find.rs`), with highlights, n of m, Enter / ⌘G. eframe persistence keeps the window and panel widths; `App::save` the sidebar state. Tests seen red first: swipe (unit and through the app), find (fixture text incl. an override, stepping), space menu flow (colour, icon, rename, Escape reverts, reset), restore after restart. Screenshots of the colour popover and the find bar checked. Not verified: a real trackpad swipe, typing in the popovers with an IME, persistence across a real restart, the native ⌘L/⌘F accelerators by hand. The feat/figma-link-open branch also uses ⌘L (Open Figma link) and needs another key when it lands

### 2026-10-19T000000Z b134-sidebar-fixes-from-use-switching-spaces
- 2026-10-19 — Sidebar fixes from use: switching spaces widened the sidebar (the slid page was allocated in the panel; now a `new_child`), so its width never seemed to stick; the panel id changed to drop inflated saved widths, and the default is 232 px. Swiping is Arc's pager now: both spaces' pages follow the fingers side by side, the colour blends with them, a third of the width or a flick turns the page, the pages carry on from the release, the trackpad's momentum is ignored. Tests seen red first: the width test (236 → 272 px on a switch), mid-swipe pages and colour, turn rules and momentum (unit). Not verified by hand: the swipe feel on a real trackpad

### 2026-10-19T000000Z b135-pr-28-review-fixes-the-command
- 2026-10-19 — PR #28 review fixes: the command bar is modal (no ⌘W / Ctrl+Tab / menu actions under it) and its list is keyed on the active file; layer search scores during the walk; eased zooms hit-test, hover and marquee the view on screen and request tiles for the target; the peek card owns its rectangle; a narrow card keeps the status pill (above the tools) and the zoom percentage; file dots scroll and show a × on hover; section labels on the gradient use `text_dim`, `text_faint` darkened to ≥ 3:1 on every tint; custom accents keep button text at 4.5:1; the dot ring stays in its slot; the space store keeps resets, scopes the name fallback to the folder (hashed), writes atomically with 0600 and skips damaged entries; a space change is saved when another popover opens, its file closes or the app quits; the window clears opaque without the blur; the browser follows a later `prefers-reduced-motion` change. A new swipe after the trackpad's momentum works without a pause. Tests seen red first for each, except the peek click-through, which the harness did not show before the fix either (egui already gave the peek's layer the pointer there); the change is kept as hardening

### 2026-10-19T000000Z b136-settings-in-arc-s-layout-settings
- 2026-10-19 — Settings in Arc's layout (`settings.rs`): General, Canvas, Shortcuts, Agents and About tabs, ⌘, (the app menu on macOS), View → Agent access… and Help → About open their tabs (the two old windows are gone). Prefs persist with the window state; the renderer choice applies at the next start. A typed space icon keeps its first grapheme (`unicode-segmentation`, already in the tree; a saved "dddd…" showed as text over the dot). Tests seen red first: settings shortcut and tabs, shortcut search, General/Canvas changes (the scroll-zoom one against reverted code), prefs restore, one-grapheme icons. Screenshot of the General tab checked. Not verified: the native ⌘, accelerator by hand, the renderer choice across a real restart

### 2026-10-19T000000Z b137-changeable-shortcuts-and-settings-proportions-keymap
- 2026-10-19 — Changeable shortcuts and Settings proportions: `keymap.rs` (18 bindings with defaults; changes saved with the window state), recorded in Settings → Shortcuts by clicking a keycap (Backspace removes, Escape keeps, taken keys move and say so; per-row reset, Reset All Shortcuts, a Customized section first); `App::shortcuts`, the drawn menus, the command bar and the native menu accelerators (any letter, digit, F-key or punctuation now maps) all read it. Tabs sized to their names, the explanation column narrowed to about a third, smaller switches and segments. Settings persistence checked across a real quit (`app.ron` held the prefs). Tests seen red first: change/remove/move/reset through the UI (and the app following ⌘J, not ⌘L, against code reading the defaults), keymap rules and saving, prefs restore with a changed shortcut. Not verified: the native menu showing a changed accelerator by hand, the Shortcuts tab on screen

### 2026-10-19T000000Z b138-settings-restyled-after-arc-s-measured
- 2026-10-19 — Settings restyled after Arc's, measured from a screenshot of Arc's dark Settings (toolbar 85 pt, warm grey colours, an outlined list at about 60% beside blue keycap tiles, 48 pt rows, white 62×24 keycaps, Reset All as a grey button at the foot); every tab is rows in one outlined list, Agents included (switches through `McpHost::access`). A setting's note no longer runs under its control (test seen red against the old row: "Auto" over the renderer note). Checked by screenshots of every tab, dark and light. Not verified: the browser's Agents tab on screen

### 2026-10-19T000000Z b139-space-dots-as-arc-s-no
- 2026-10-19 — Space dots as Arc's: no × on a hovered dot (Close space in its menu, or a middle click), no ring round the active one, all the same size; spaces you are not in are drained toward grey and faded until hovered; hovering shows a dark pill with the space's name and its keys as caps. ⌃1…⌃9 go to spaces 1–9 (`Binding::Space`, changeable in Settings → Shortcuts). Tests seen red first: ⌃2/⌃1 switch and the pill sits above the dot with the name and keys (against the old footer); no close cross, Close space from the menu; colours drained and faded. Screenshot of the footer with a pill checked. Not verified: a real pointer hover in the app, ⌃digit against macOS's own Mission Control shortcuts if those are turned on

### 2026-10-19T000000Z b140-f-also-finds-the-names-drawn
- 2026-10-19 — ⌘F also finds the names drawn above frames and sections (`labels::named`, the frames the canvas labels), highlighted on the label (or round the frame when its label is not drawn); matching stays a browser's (the words as typed, any case). Test seen red first (a frame name found, a near miss not). Screenshot of a name match checked. Not verified: a frame name with labels turned off, by hand

### 2026-10-19T000000Z b141-the-space-name-pill-waits-until
- 2026-10-19 — The space name pill waits until the pointer has rested on a dot for 0.5 s, as Arc's; for 0.4 s after one shows, the next dot's shows at once (running along the row). Test seen red first (the pill came up at once). Not verified: the timing by hand

### 2026-10-19T000000Z b142-motion-everywhere-a-state-changes-the
- 2026-10-19 — Motion everywhere a state changes: the command bar, Settings, space popovers, the find bar with its highlights and the space name pill fade and shrink out after closing (no input meanwhile) instead of vanishing; the pill rises in and glides between dots; the canvas fades in on another file or page, the properties on another selection, hover and selection outlines when they move, a Settings tab's page, a shortcut's recording ring and notice, a layer's children when it opens; the current find match pops; new file dots pop in and the others slide over when one closes (`anim::presence`, `arrive`, `pop_on`, `inert`). Space badges lost the disc behind them. Tests seen red first: each overlay still on screen two frames after Escape and gone after 0.6 s (each proven separately), the pill fading out, the helpers' timing, no disc behind a badge. Screenshot of the footer checked. Not verified: how each motion feels by hand; egui's own menus and tooltips still close without a fade

### 2026-10-20T030312Z b143-pr-28-review-fixes-shortcuts-recording
- 2026-10-20 — PR #28 review fixes, shortcuts: recording ignores modifier keys pressed alone and takes the chord from the next key (⌘J from a real keyboard is ⌘J, not "Super"); ⌘X/⌘C/⌘V (Cut/Copy/Paste events) can be recorded; recording takes keys first thing in the frame; menu actions are ignored while recording and the Mac menu's accelerators are cleared in the same turn; a key a menu cannot show stays with egui; Control is kept beside ⌘ on a Mac (also in the accelerator); a damaged saved keymap drops the bad entry instead of the whole window state; the sidebar and empty-state hints follow the keymap; a long emoji is kept whole or dropped, never cut. Tests seen red first for each. Not verified: a Mac menu accelerator by hand while recording (the harness has no native menu); the search-field letter leak did not reproduce (egui drops the field's focus on the keycap click), so the event filter there is hardening

### 2026-10-20T030312Z b144-pr-28-review-fixes-space-dots
- 2026-10-20 — PR #28 review fixes, space dots: the name pill no longer panics (`f32::clamp`) when it is wider than the window, and a long name is cut with an ellipsis; the pill's wait restarts while the pointer moves on the dot; a dot sliding into its new slot is clicked and named where it is drawn; the window title is sent only when it changes, which stops the app redrawing every frame while a file is open (on `main` too) and lets a test show that the pill wakes the app by itself. Tests seen red first for each, the old clamp panicking

### 2026-10-20T030312Z b145-pr-28-review-fixes-overlay-exits
- 2026-10-20 — PR #28 review fixes, overlay exits: the command bar, Settings and space popovers stay interactable (inert) while they fade, so a click or scroll on them no longer lands on the app underneath, and their shortcut guards hold until they are gone; a new space popover pops in while the last one fades out (each has its own presence and area); find's fading highlights are cleared on any other tab or page and the bar finishes fading when the last file closes. Tests seen red first for each. Not changed: a popover reopened after its file closed already started from 0 (`SpaceEditor::new` reset it; measured)

### 2026-10-20T030312Z b146-pr-28-review-fixes-find-escape
- 2026-10-20 — PR #28 review fixes, find: Escape closes the bar also after its field lost focus (a canvas click), before it clears the selection; the 5000 matches kept are the top of the page (sorted before the cut, not the first walked), instances are searched top first and the walk stops once the kept matches sit above the next instance. Tests seen red first. Kept as is: the "already on screen" check uses the target view, where the ease ends; the search still runs on the UI thread, bounded by the 200 000-node instance budget

### 2026-10-20T030312Z b147-pr-28-review-fixes-sidebar-pager
- 2026-10-20 — PR #28 review fixes, sidebar pager: a page turn keeps the page it left beside it until the spring settles (it was dropped, and the page dipped to 15% opacity, in the last 36 pt); only a far jump's nudge fades; the moves of one frame count as one speed sample, so a quick flick sent as several events a frame still turns the page; the trackpad's momentum is recognised in the frame after a lift even when the UI thread hitched, and a start after that momentum is a new swipe. Tests seen red first for each

### 2026-10-20T030312Z b148-pr-28-review-fixes-space-store
- 2026-10-20 — PR #28 review fixes, space store: written through a new uniquely named temporary file (`tempfile`) with owner-only permissions set on it, so a second writer or a leftover temporary file cannot tear the store or widen its mode; a store that cannot be read (too big, no permission; in the browser too) is left as it is and this session keeps styles in memory; reads are capped while reading; rows saved before folders were are found by name; a reset while renaming is locked by a test (it already closed the popover). Tests seen red first. Not verified: two processes at once (a thread race did not hit the old window, so it is not a test)

### 2026-10-20T030312Z b149-pr-28-review-fixes-settings-the
- 2026-10-20 — PR #28 review fixes, Settings: the lit tab's name is drawn in the text colour and its icon in a blue that reads on the lit pill; words and glyphs on the white keycap and the tiles use a darker blue; the chosen segment (dark) and the notice (light) were darkened; the Shortcuts search hint uses the title grey (4.04:1 before, found by the new test); `settings::look_tests` checks every Settings text and mark against 4.5:1 or 3:1 in both themes. The remote switch's note says clients send the access token. The browser's room-token field leaves room for Apply. Tests seen red first, except the token row (browser only: checked with the wasm build, not drawn). Not changed: the remote switch was already reported disabled while the server runs (egui marks a widget in a disabled scope; now asserted)

### 2026-10-20T030312Z b150-pr-28-review-fixes-misc-the
- 2026-10-20 — PR #28 review fixes, misc: the tool pill's zoom percentage reads the view on screen during an ease; the command bar's results are keyed on the page too, so a page turned under the open bar (by an agent) refreshes its layers; layer search keeps only the best 60 while it walks. Tests seen red first

### 2026-10-20T030312Z b151-the-canvas-crossfades-when-the-space
- 2026-10-20 — The canvas crossfades when the space or page changes. It used to fade the new picture in from nothing, so in the dark theme the near-black card showed for the first frames (a light page went dark and came back). The last picture is now painted again from the tile cache under the new one (`chrome::Crossfade`, `Tiles::paint_cached`); the first file fades in over its own page colour. Tests seen red first: the colour under the canvas stays between the two pages' colours on every frame of a switch, and the first file never shows the card. Checked in the app with the fade held at a quarter (temporary hook, not committed): the canvas read (123, 123, 124) before and (244, 244, 244) after, on a (245, 245, 245) page. Not verified: how it feels by hand at full speed

### 2026-10-20T030312Z b152-pr-28-review-fixes-crossfade-comment
- 2026-10-20 — PR #28 review fixes, crossfade (comment 6280181813): the last picture is kept only once shown in full, so switching back mid-fade no longer flashes a page that was barely drawn; the underlay's tiles are marked drawn after the new page plans its own so the cache keeps them; a closed file's tiles stay until its fade-out ends. The tests now lock a real crossfade (a built two-page file with different page colours through `Tab::set_page`: no cut, steady progress, a quick return; the last file's tiles under the next and kept; a closed file fading out). Four tests that dropped egui's texture changes failed under the debug profile (`cargo test -p figstudio`, not the gate's `fast` profile); they clear them now. Each new test seen red with its fix undone

### 2026-10-20T030312Z b153-pr-28-review-fixes-keymap-review
- 2026-10-20 — PR #28 review fixes, keymap (review thread on `keymap.rs`): two shortcuts swapped in Settings (Hand on V, Select on H) reverted on the next launch, because each saved change was checked against defaults the other change replaces. Keys are now given out against the finished map: saved changes first, then the defaults left; a default whose chord a change took has no shortcut. Tests seen red first: every swap of two default chords survives a save and load (351 pairs), a damaged keymap gives each chord to one binding, and a swap survives eframe's RON storage

### 2026-10-20T030312Z b154-pr-28-review-fixes-space-store
- 2026-10-20 — PR #28 review fixes, space store (review thread on `spaces.rs`): a store that did not parse (cut-short JSON, not UTF-8) became an empty store still bound to the file, and the next space change wrote over every saved style. It is now left as it is and this session's styles stay in memory, as for a store that cannot be read; the browser does the same with `localStorage`. An empty file is still an empty store. Test seen red first, for both the cut JSON and the non-UTF-8 case. Not verified: the browser path in a browser (wasm build checked)

### 2026-10-20T030312Z b155-pr-28-review-fixes-find-review
- 2026-10-20 — PR #28 review fixes, find (review thread on `find.rs`): find on page matched text layers that are hidden or inside a hidden frame (the page walk did not check `visible`). It now skips hidden layers and everything in them, which also spares walking hidden instances. Text hidden inside an instance was already left out (`path_bounds` gives no box when the node or an ancestor is hidden); a test now locks it. Test seen red first with the page walk unchanged

### 2026-10-20T030312Z b156-pr-28-review-fixes-comment-6281107196
- 2026-10-20 — PR #28 review fixes (comment 6281107196): a sidebar flick whose Start, moves and End all arrive in one frame (a UI hitch) now turns the page: moves in the frame their swipe started in are timed over the time since the last frame and, with nothing before them, count in full. Test seen red first, both without the timing and with the first sample smoothed. Docs: README and IMPLEMENTATION.md now say View → Agent access… opens Settings → Agents, where the switch starts the server; `menu_windows.rs` draws the Settings window; ⌘, (Ctrl+, elsewhere) opens Settings, not the Agents tab

### 2026-10-20T030312Z b157-pr-28-review-fixes-comment-6281631609
- 2026-10-20 — PR #28 review fixes (comment 6281631609): a flick that lands whole in one frame after a hitch of 0.1 s or more turned nothing, because the 0.1 s ceiling meant for stale gaps between events dropped its only sample; that path has no ceiling now. Its speed stays a lower bound by choice (a short flick in a long hitch springs back rather than a stalled slow drag turning a page), stated in IMPLEMENTATION.md and locked by a test (60 points in 80 ms on a 220-wide sidebar stays). Test seen red first with the ceiling back

### 2026-10-20T030312Z b158-pr-28-review-note-comment-6281714626
- 2026-10-20 — PR #28 review note (comment 6281714626): `every_offered_emoji_and_icon_can_be_drawn` failed where a system font has a glyph for its "missing" emoji (U+1F9E9); its reference characters are now plane-16 private-use code points no font covers. Not reproduced here (no such glyph on this machine); the check still fails when the "missing" character has a glyph

### 2026-10-20T030312Z b159-pr-28-review-note-comment-6282033901
- 2026-10-20 — PR #28 review note (comment 6282033901): corrected the emoji test's explanation. Either member of its old reference pair could have a real glyph, including U+E000 in a UI font; the comment no longer claims the loaded fonts cover every emoji. No behaviour change
