---
branch: perf/fig-lazy-symbol-overrides
pr:
title: Keep instance overrides encoded until entered
---

## P1b — Large files (requester's file cannot be shared)
- [x] Keep `symbolData.symbolOverrides` encoded until the instance is entered

## Progress

### 2026-11-09T073000Z lazy-symbol-overrides
- What changed and how it was checked: the lazy fields become a per-schema table; nested
  `(SymbolData, symbolOverrides)` joins the node record fields. Test
  `instance_overrides_stay_encoded_until_read` fails with the table entry removed. check.sh
  on 1.99.0: all checks passed (225 test groups, wasm32). Both large real files write back
  byte-identical (SHA-256, same as before the stack). Against the shared-values PR, two
  large files, 4 interleaved runs, medians: memory after open 597 -> 568 MB and 796 -> 773 MB
  (-5% / -3%); render CPU -2% / -5%; compare CPU and peak unchanged. `info` CPU looked +15%
  / +5% over 4 runs; 8 more interleaved runs gave +3% / +2% with the runs overlapping (load
  60-110), so at most a couple of percent (`info` enters every instance, which now decodes
  its overrides there).
- Smaller than estimated (~70 MB): equal `symbolData` values are already stored once.
- Not verified: the desktop app and the browser build by hand.

