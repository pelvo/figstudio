---
pr: 32
branch: release-please--branches--main--components--figstudio
title: chore(main): release 0.1.0
---

## Progress

### 2026-10-22T014307Z b002-pr-32-ci-bridge-test-keep
- 2026-10-22 — PR #32 CI bridge test: keep the child's stderr open after parsing the
  pairing banner, so later startup output cannot race a closed pipe and reset the
  WebSocket connection. The late-write regression failed with `BrokenPipe` before the
  fix; all five bridge tests and 100 consecutive pairing runs passed afterwards.
  Rust 1.99.0 `scripts/check.sh` ended `all checks passed` after supplying the missing
  Noto Emoji font through `FIGSTUDIO_EMOJI_FONT` (the first run failed the web-font
  prerequisite). Applied M-INTEGRATION-TESTS; reused Tokio's duplex stream, no new
  dependency. Not verified: macOS/Windows or a real browser/client; GitHub CI awaits
  the fix PR.
