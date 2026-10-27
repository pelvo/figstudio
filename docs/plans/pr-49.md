---
pr: 49
branch: fix/bridge-cli-ci
title: test(mcp): keep bridge stderr open after reading the banner
---

## Progress

### 2026-10-22T014307Z b001-pr-49-review-replace-the-borrowed
- 2026-10-22 — PR #49 review: replace the borrowed-reader regression with an owned
  stream passed to the same banner helper as the child process. The helper drains stderr
  in a task until EOF; a late write exceeding the duplex capacity verifies both ownership
  and draining. Removing the drain failed with `BrokenPipe` with the test unchanged;
  a mismatched parser prefix failed at the new two-second timeout. All five bridge tests
  and Rust 1.99.0 `scripts/check.sh` passed (Noto Emoji supplied through
  `FIGSTUDIO_EMOJI_FONT`). Applied M-INTEGRATION-TESTS; no new dependency or production
  change. Not verified: macOS/Windows or a real browser/client.
