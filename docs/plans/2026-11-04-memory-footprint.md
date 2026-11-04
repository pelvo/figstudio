---
branch: perf/fig-inline-guids
pr:
title: Cut decoded document memory
---

## P1b — Large files (requester's file cannot be shared)
- [x] Decode `{sessionID, localID}` structs inline as `Val::Guid` (no heap object per GUID)
- [ ] Benchmark open / read / encode before and after on two large real files (before the PR)

## Progress

### 2026-11-04T000000Z memory-footprint
- What changed and how it was checked: a running app with two large files open (each
  about 150k nodes) held 5.5 GB, flat over time, and load/drop cycles do not grow, so no leak. The
  decoded tree is ~17× the encoded message (about 100-170 MB -> 1.9-2.7 GB); the biggest
  parts are `derivedSymbolData` (~45%) and one 96-byte object per GUID. GUIDs now decode to
  `Val::Guid`: about -2.5M allocations and
  -230 MB per file (~10%). Writer accepts both
  forms; JSON shape unchanged; connector parts and the compare report read GUIDs via
  `as_guid` (a changed reference is now one row, shown as `1:3`). Tests in
  `crates/fig/tests/roundtrip.rs`, `crates/render/tests/connector.rs` and
  `crates/diff/src/properties.rs` each failed without the change.
- Gate: every scripts/check.sh slice passed on 1.99.0 (the newest nightly trips a new
  recursion lint in `gpu_registry.rs`, unrelated). tests-npm "the test hold is capped"
  failed on main too (node start-up ~600 ms here); fixed in its own commit.
- Not verified: timing (machine was under load; benchmark planned before the PR); the desktop
  app by hand. Next: keep `derivedSymbolData` and other heavy fields encoded until read.

