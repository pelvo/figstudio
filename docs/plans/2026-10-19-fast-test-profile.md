---
pr: 29
branch: chore/fast-test-profile
title: chore: lint and test with a no-lto profile to speed up the gate
---

## Progress

### 2026-10-19T152050Z b129-faster-gate-scripts-check-sh-lints
- 2026-10-19 — Faster gate: `scripts/check.sh` lints and tests with a new `fast` profile (release optimisation, no LTO, incremental) instead of `--release`. Thin LTO linked each of the ~100 test binaries; after a one-line change in `crates/fig`, `cargo test --release --workspace --no-run` took 20 min 24 s, while `--profile fast` takes 36 s. The whole gate after that change took 8 min 10 s, 4 min of it the LTO release build, which stays in the gate. All checks passed on Rust 1.99.1. Not measured: the full gate before the change (the test build step alone was 20 min).
