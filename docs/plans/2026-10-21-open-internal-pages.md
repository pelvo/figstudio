---
pr: 48
branch: feat/open-internal-pages
title: feat(app): open internal pages from the sidebar
---

## P3 — Desktop app (`apps/figstudio`)
- [x] internal pages open from a collapsed "Internal pages (N)" section in the Pages list;
      mouse and keyboard selection use the same navigation as normal pages

## Progress

### 2026-10-20T000000Z b231-2026-10-20-internal-pages-now
- 2026-10-20. Internal pages now open from an expandable "Internal pages (N)" section in the Pages list. The built-in egui disclosure reuses normal page rows and navigation, with per-tab expansion state and keyboard activation. The navigation regression failed against the old passive label; all three new UI tests pass in fast and debug profiles. `FIGSTUDIO_TOOLCHAIN=stable scripts/check.sh` ended with `all checks passed`, including the release build and wasm32 MCP-core build. Generated and inspected the public-fixture thumbnail sheet. No new dependencies or unsafe code. Not verified: native-window or browser interaction.

### 2026-10-21T050251Z b232-2026-10-21-refined-the-internal
- 2026-10-21. Refined the internal-pages disclosure with muted section typography, the sidebar's thin chevron, a full-width hover target and page rows aligned without extra indentation. The new layout regression failed against the short default header; all four internal-page tests pass in fast and debug profiles. Inspected native screenshots and manually opened the internal page and returned to the normal page using the public fixture. The full pre-commit check ended with `all checks passed`. Demo media stays local and is excluded from the PR diff. Browser interaction remains unverified.

### 2026-10-21T050251Z b233-pr-48-review-internal-pages-now
- 2026-10-21 — PR #48 review: internal pages now round-trip through browser state frames and the HTTP bridge, appear in browser status indexing and command-bar results, and reveal as pages rather than selected canvas nodes. Programmatic navigation opens the disclosure without overriding a later manual collapse; highlight animation is scoped to its list and tab. Defects were reproduced before fixes, including a mutation check for the highlight. All 10 focused app tests pass in fast/debug profiles and all four HTTP bridge tests pass. The full gate ended with `all checks passed`; the public-fixture sheet was inspected. Test documentation has its own internal-page subsection. Live browser interaction remains unverified; the main-branch conflict was not integrated.

### 2026-10-21T050251Z b234-rebased-pr-48-onto-latest-main
- 2026-10-21 — Rebased PR #48 onto latest `main` (`feba1e4`) as requested. Resolved only documentation conflicts, preserving workspace/session coverage and internal-page coverage. The rebased full gate ended with `all checks passed`, including release and wasm32 MCP-core builds. No application patch changes or demo media were introduced by the rebase; live browser interaction remains manually unverified.

### 2026-10-21T050251Z b235-pr-48-follow-up-review-the
- 2026-10-21 — PR #48 follow-up review: the browser page probe now resolves the same normal-then-internal indices published in status, rejecting out-of-range indices. Command-bar details distinguish internal pages, including the current page. Both regressions failed before the fixes; all 11 focused app tests pass and the full gate ended with `all checks passed`. Live browser interaction remains manually unverified.
