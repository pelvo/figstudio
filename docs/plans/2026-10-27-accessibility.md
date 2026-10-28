---
branch: fix/accessibility
pr: 67
title: Accessibility: screen readers, focus, contrast, text size
---

## P3 — Desktop app (`apps/figstudio`)
- [x] The shipped desktop build has AccessKit (`scripts/check.sh --lint-one accesskit`)
- [x] Every control in the main states has a name; menus, the canvas and layer rows have roles and state (`ui_a11y_tests.rs`)
- [x] Visible keyboard focus on painted widgets; Tab stays inside Settings and the command bar
- [x] Text colours at WCAG AA (4.5:1) on every surface, presets and custom space colours
- [x] Text size: Settings → General, and ⌘+ / ⌘− no longer zoom the canvas too
- [ ] A person checks the app with VoiceOver (macOS), Narrator (Windows) and Orca (Linux)

## P4 — Platforms
- [x] Browser: opt-in "Read controls aloud" (egui's speech output; eframe has no AccessKit on the web)
- [ ] Browser: a real screen-reader tree (waits for AccessKit support in eframe's web backend)

## Progress

### 2026-10-28T025748Z accessibility
- What changed and how it was checked: the desktop app now builds eframe with `accesskit`
  (before, only the kittest dev-dependency turned it on, so shipped builds gave screen readers
  no tree). The `accesskit` lint slice in `scripts/check.sh` guards it and failed before the
  fix. Every control in the main states has a name. Menus, the canvas (page, zoom, selection)
  and layer rows (expanded or collapsed) have roles. Pointer-only areas are hidden and skipped
  by Tab. Painted widgets draw a focus ring. Tab stays inside Settings and the command bar.
  Text colours meet 4.5:1 on every surface. Settings has a Text size row, and ⌘+ no longer
  zooms the canvas too. The browser has an opt-in "Read controls aloud". Action icon buttons
  were announced as toggles; they are plain buttons now. Each behaviour has a test in
  `ui_a11y_tests.rs`, `theme.rs` or `settings.rs` that failed without the change.
  `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh` ended with all checks passed. In the
  macOS bundle (`scripts/macos-app.sh`), the system accessibility tree listed the named
  sidebar, rows, splitters, canvas and tools, and Tab showed the focus ring.
- Not verified: a real screen reader session (VoiceOver, Narrator, Orca); Windows and Linux
  AccessKit; speech in a browser; the Text size choice surviving a restart; the browser
  build beyond `cargo check` (no `scripts/web-build.sh` run).
