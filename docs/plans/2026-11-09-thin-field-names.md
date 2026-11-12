---
branch: perf/fig-thin-field-names
pr: 85
title: Store object field names as one pointer
---

## P1b — Large files (requester's file cannot be shared)
- [x] Store object field names as one pointer (`Name`), 40-byte fields instead of 48
- [x] The writer matches schema fields by interned name, not by text

## Progress

### 2026-11-09T090000Z thin-field-names
- What changed and how it was checked: measured first on top of the lazy overrides: 8.8M
  and 12.4M field slots (each shared value counted once), 133 / 189 MB of 16-byte names.
  `Obj` now holds a `Name` (one pointer to the interned, leaked `Box<str>`); its public API
  is unchanged. The extra pointer hop made saving 5-8% slower (the writer looks up every
  schema field of every object by text), so the writer now matches by interned name.
  Tests: `a_field_is_a_name_pointer_and_a_value` fails with a `&'static str` name; the
  roundtrip tests fail with the named lookup broken. Both large real files write back
  byte-identical. Against the lazy-overrides PR, CPU time per step, medians of 6-12
  interleaved runs (load 60-150): memory after open 568 -> 518 MB and 772 -> 704 MB (-9%);
  saving -48% / -48%; reading every field -7% / -4%; lookups by name -5% / 0%; opening
  +1% / -8% (within the spread).
- check.sh on 1.99.0: all checks passed (225 test groups, wasm32), after a clippy
  `borrowed_box` `#[expect]` on `Name`. Quiet-machine CPU (load 4-14): render and `info`
  within 1%; compare time unchanged, peak -8% / -7%.
- Not verified: the desktop app and the browser build by hand.

