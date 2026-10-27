---
pr: 36
branch: feat/persist-session-settings
title: feat(app): persist workspace and mcp access settings
---

## P3 — Desktop app (`apps/figstudio`)
- [x] desktop Agent access, remote access and bind address survive a restart; older saved
      preferences still default to access off, and bearer tokens are not stored
- [x] desktop open files/spaces reopen after restart in saved order, with the active space
      restored; missing files do not block others, and closed spaces stay closed
- [x] persistence review: requested MCP enablement survives plan/bind/runtime failures, with
      a live minted-token test proving credentials are not persisted
- [x] persistence review: workspace paths use a separate private atomic store; malformed or
      oversized workspace metadata cannot reset other preferences
- [x] persistence review: saved files and startup decoders are bounded; canonical aliases,
      interim closes and failed-active files preserve the correct restore/focus intent
- [x] persistence re-review: failed private migration retains restorable legacy metadata
      without blocking other preference saves; successful manual aliases keep focus during restore
- [x] persistence warning re-review: unreadable private metadata explains current-session loss
      and repair/restart for future saves; alias coverage includes third-tab focus while loading
- [x] persistence warning follow-up: oversized legacy fallback retains recovery guidance
      alongside the saved-path limit warning without overwriting damaged metadata

## Progress

### 2026-10-20T000000Z b220-desktop-mcp-access-settings-now-persist
- 2026-10-20 — Desktop MCP access settings now persist with the existing serde/eframe preferences: on/off, remote access and bind address restore before the first frame. Fresh and older preferences default off; endpoints and per-start bearer tokens are not stored. Regression proofs: the save/reload test failed on the old code, and the older-preferences test failed without `Prefs`' `#[serde(default)]`; both pass after restoration. `FIGSTUDIO_TOOLCHAIN=stable scripts/check.sh` passed with both new tests in the workspace output. A forced fresh release build passed and the binary contains the updated settings copy, avoiding another worktree's shared cached artifact. Applied M-PUBLIC-DEBUG; no new dependencies or unsafe. Not verified: on-disk persistence across an actual OS app restart or the full browser app build/interaction.

### 2026-10-20T000000Z b221-2026-10-20-desktop-workspace-restoration
- 2026-10-20. Desktop workspace restoration now saves open file paths and the active space with existing serde/eframe storage. Worker completion order does not change saved space order; explicit startup files merge without duplicate paths, changed focus wins, pending restores survive an early save, missing files show a toast, and closed spaces stay closed. Existing space styles are reused; page/zoom/selection restoration is out of scope. Both headless session tests failed on the old code; removing startup sorting made the deterministic reverse-completion test fail, and all three passed after restoration. `FIGSTUDIO_TOOLCHAIN=stable scripts/check.sh` passed in an isolated build directory, including all three workspace and both MCP preference regressions, a fresh release app build and the wasm32 MCP-core build. Applied M-PUBLIC-DEBUG; no new dependencies or unsafe. Not verified: actual OS quit/relaunch with on-disk preferences or the full browser app build/interaction.

### 2026-10-20T000000Z b222-2026-10-20-prepared-the-desktop
- 2026-10-20. Prepared the desktop persistence changes on current main after #27 merged. The full gate passed in a clean, persistence-only worktree with all five new regressions, a fresh release build and wasm32 MCP-core build. A subsequent upstream README-only update was fast-forwarded without changing the persistence patch. Concurrent UI edits were excluded. Actual OS quit/relaunch and full browser app build/interaction remain unverified.

### 2026-10-20T000000Z b223-2026-10-20-pr-36-persistence
- 2026-10-20. PR #36 persistence review: MCP failures retain requested access without a retry loop; a live minted-token test checks actual credential exclusion. Workspace paths moved to private atomic JSON with independent corruption handling and legacy cleanup. Saved metadata is capped at 500 paths/4 MiB and startup at four decoders; queued paths, canonical aliases, interim closes and failed-active fallback preserve restore intent. Baseline regressions failed before fixes; insecure-writer, disabled-cap and token-persistence mutations each failed their dedicated test and were restored byte-for-byte. The full `FIGSTUDIO_TOOLCHAIN=stable scripts/check.sh` passed, including all 347 app tests, workspace/GPU tests, a fresh release app build and wasm32 MCP-core. Reused serde_json/tempfile; applied M-PUBLIC-DEBUG, no new dependencies or unsafe. Actual OS quit/relaunch, non-Unix permissions and full browser app interaction remain unverified.

### 2026-10-20T000000Z b224-2026-10-20-pr-36-re
- 2026-10-20. PR #36 re-review: legacy workspace paths survive preference saves until a private atomic write succeeds, including unreadable private metadata and a failed initial import. Other preferences still save; newly opened paths never enter the legacy backup, and malformed legacy metadata stays isolated. Successful aliases now retain original automatic/manual intent while focusing the kept tab. All three regressions failed at their intended assertions before the fixes; all 20 workspace tests and the full `FIGSTUDIO_TOOLCHAIN=stable scripts/check.sh` passed, including 350 app tests, workspace/GPU tests, release build and wasm32 MCP-core. Reused serde/serde_json/tempfile; applied M-PUBLIC-DEBUG, no new dependencies or unsafe. The deterministic write-failure setup is Unix-only. Actual OS quit/relaunch, non-Unix failure behavior, full browser app interaction and integration with newer main remain unverified.

### 2026-10-20T000000Z b225-2026-10-20-pr-36-warning
- 2026-10-20. PR #36 warning follow-up: unreadable workspace metadata now warns that workspace edits cannot persist until the file is repaired and the app restarted; other settings still save. The warning regression failed on the previous message. Alias tests also cover third-tab selection after manual or automatic alias completion while another restore is pending; disabling the existing focus guard failed the new case, and the source was restored byte-for-byte. All 20 workspace tests, all 350 debug-profile app tests and the full `FIGSTUDIO_TOOLCHAIN=stable scripts/check.sh` passed, including 350 app tests, workspace/GPU tests, release build and wasm32 MCP-core. No retry-policy or alias-algorithm change, new dependencies or unsafe. Actual OS quit/relaunch, non-Unix behavior, browser interaction and integration with newer main were not rerun.

### 2026-10-20T000000Z b226-2026-10-20-pr-36-rebased
- 2026-10-20. PR #36 rebased onto current main (536b331). Only appended PLAN history conflicted; both histories were kept, and the three review-fix patches were unchanged. The full `FIGSTUDIO_TOOLCHAIN=stable scripts/check.sh` passed with 362 app tests, workspace/GPU tests, release build and wasm32 MCP-core; all 362 debug-profile app tests also passed. No new behavior changes. Actual OS quit/relaunch, non-Unix behavior and browser interaction were not rerun.

### 2026-10-20T000000Z b227-2026-10-20-pr-36-warning
- 2026-10-20. PR #36 warning follow-up clarifies that workspace edits from a read-disabled session are lost; repair and restart apply to future writes. A capped legacy fallback now adds its skipped-file warning without replacing recovery guidance. Both regressions failed at their intended assertions before the fixes; all 21 workspace tests, 363 debug-profile app tests and the full `FIGSTUDIO_TOOLCHAIN=stable scripts/check.sh` passed, including 363 app tests, workspace/GPU tests, release build and wasm32 MCP-core. No retry, overwrite or legacy-backup policy change, new dependencies or unsafe. Actual OS quit/relaunch, non-Unix behavior and browser interaction were not rerun.

### 2026-10-21T034204Z b228-2026-10-21-pr-36-rebased
- 2026-10-21. PR #36 rebased onto current main at 28f448e2. The only conflict was appended PLAN history; both histories were kept. All five later patches are unchanged, including the warning fixes. The full `FIGSTUDIO_TOOLCHAIN=stable scripts/check.sh` passed with 435 app tests, workspace/GPU tests, release build and wasm32 MCP-core. The required debug app run also passed all 435 tests; both app runs ignored one upstream manual benchmark. All seven public-fixture thumbnail comparisons were inspected; known missing-image, effect, background and outline differences remain. No new behavior, dependencies or unsafe. Actual OS quit/relaunch, non-Unix behavior and full browser interaction were not rerun.

### 2026-10-21T034204Z b229-2026-10-21-pr-36-rebased
- 2026-10-21. PR #36 rebased onto onboarding/Recents main at c1295fd. All seven commits replayed; conflicts preserve onboarding progress, private migration and the shared byte writer. Restored loads locate their tab by id after saved-order sorting, so Recents records that file even when another space stays active. A public-fixture worker regression covers both completion orders; the wrong-active-index mutation failed its intended assertion and was restored byte for byte. The full gate and required debug app suite each passed 478 tests with one ignored manual benchmark. All seven public-fixture render comparisons were inspected and match the previous sheet; existing rendering gaps remain. No new dependencies or unsafe. Actual OS quit/relaunch, non-Unix behavior and full browser interaction were not rerun.

### 2026-10-21T034204Z b230-2026-10-21-pr-36-review
- 2026-10-21. PR #36 review: automatic workspace restoration now leaves Recents history, timestamps, keys and pictures unchanged instead of shuffling or evicting them. Explicit startup/manual loads still record the loaded tab; explicit canonical-alias links refresh the kept tab's card after changing its page. Both reported regressions failed before the fix; four focused tests now pass, including a full 60-entry disk history in both completion orders and the actual linked-page PNG. The full gate and required debug app suite each passed 481 tests with one ignored manual benchmark. All seven public-fixture comparisons were inspected and match the previous sheet; known rendering gaps remain. No new dependencies or unsafe. OS quit/relaunch, non-Unix behavior and full browser interaction were not rerun.
