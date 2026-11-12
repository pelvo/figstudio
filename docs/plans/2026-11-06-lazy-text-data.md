---
branch: perf/fig-lazy-text-data
pr: 80
title: Keep stored glyph outlines encoded until first read
---

## P1b — Large files (requester's file cannot be shared)
- [x] Keep `derivedTextData` (stored glyph outlines) encoded until first read
- [x] `Obj::contains_key` answers without decoding a lazy field

## Progress

### 2026-11-06T083000Z lazy-text-data
- What changed and how it was checked: `derivedTextData` joins the lazy node fields;
  `Obj::contains_key` reads the stored value, so it no longer decodes a lazy field. Test
  `stored_glyphs_stay_encoded_until_read` fails with either change undone. check.sh on 1.99.0:
  all checks passed (225 test groups, wasm32 built). Benchmark against the base commit, two
  large real files, 5 interleaved runs, medians: memory after open -8/-9%, render peak
  -3/-5%, compare (CPU, 2 rounds) -7/-9% time and -7/-8% peak; open, save, render and
  `info` times within the run spread (±5%). Reading every field of every node costs
  +0.08/+0.12 s once (the glyphs decode then).
- Not verified: the desktop app and the browser build by hand (no reader of the field
  changed).


### 2026-11-09T013607Z lazy-text-data
- What changed and how it was checked: review follow-up. A lazy field could fail to decode
  after passing the open-time walk: a list of zero-byte items (structs with no fields) was
  checked against the bytes left in the whole document at open but against the field's own
  bytes later, and the failure read as an empty value. Decoding walked bytes no longer
  repeats that length check. Test `a_lazy_list_of_empty_structs_decodes_like_it_was_checked`
  fails without the fix. No fixture schema has a struct with no fields. check.sh on 1.99.0:
  all checks passed.
- Not verified: a failed lazy decode is still not counted in `Stats`; it cannot happen now
  (comment on `Lazy::decode`).
