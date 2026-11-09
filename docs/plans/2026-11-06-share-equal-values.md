---
branch: perf/fig-share-equal-values
pr: 82
title: Store equal node field values once
---

## P1b — Large files (requester's file cannot be shared)
- [x] Store equal node field values once (`Val::Shared`; equal lazy fields share one `Lazy`)
- [x] Replace or remove shared and lazy fields without decoding them

## Progress

### 2026-11-07T033000Z share-equal-values
- What changed and how it was checked: node fields that own memory are looked up by their
  encoded bytes while node records are read; equal bytes of the same field reuse one value
  (`Val::Shared`, or one shared `Lazy`). An independent review found three issues, fixed in
  the second commit: `insert`/`remove` decoded a lazy field only to drop it (the renderer did
  this on every draw), the item budget was charged twice for a shared field, and the
  first-occurrence table kept a list per hash. Tests, each seen failing without its change:
  equal paints are one value; editing one node's shared field leaves the others (in memory
  and in the written file); replacing or removing a lazy field does not decode it. Both large
  real files write back byte-identical (SHA-256). check.sh on 1.99.0: every slice passed
  (225 test groups; wasm32 rerun alone after the disk filled during the first run).
  Benchmark against the first PR, two large files, 5 interleaved runs, medians: memory
  after open -46/-45%, after reading every field -53/-45%; render peak -19/-26%, `info` peak
  -19/-45%; compare (CPU) -26/-17% time, -53/-44% peak. Wall times were noisy (load ~45), so
  render and `info` were also measured in CPU time (4 interleaved runs): render -5/-5%,
  `info` -13/-23%. Open and save within the spread; reading every field -68%.
- Not verified: the desktop app and the browser build by hand.

