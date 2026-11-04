---
branch: perf/fig-inline-guids
pr:
title: Cut decoded document memory
---

## P1b — Large files (requester's file cannot be shared)
- [x] Decode `{sessionID, localID}` structs inline as `Val::Guid` (no heap object per GUID)
- [x] Keep `derivedSymbolData` and the two consumption maps encoded until first read
  (`Val::Lazy`); written back as the original bytes unless edited
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

### 2026-11-04T120000Z memory-footprint
- What changed and how it was checked: node fields `derivedSymbolData`,
  `parameterConsumptionMap` and `variableConsumptionMap` are skipped at open (with every
  check the decoder makes) and kept as their bytes in a shared `Val::Lazy`; `Obj` accessors
  decode on first read, copies of a node share the decoded value, an edit stores it decoded,
  and the writer copies unedited bytes when the schema is the document's own. On the same two
  large files: after open about -45% (eager 1.9-2.7 GB -> 1.2-1.6 GB); rendering one page
  peaks 0.7-1.1 GB lower and decodes a few hundred fields at most; `info` (walks every page)
  peaks 0.65-0.75 GB lower and decodes 20-50% of them. Tests, each seen failing without the
  change: fields stay encoded until read and every thread sees one value, a copied node
  shares the decode, an edited lazy field is written as edited (`crates/fig`).
- Not verified: timing (runs of one command varied 2x on this machine; benchmark before the
  PR); the desktop app by hand (search, layers, MCP may read more of these fields).

