# Pony audit fixes

Completed the 15 non-false-positive findings in `target/ponyaudit/report.md` across L1 privacy/trust, L2 rendering, and L3 desktop/synth/lean. No push, PR, or issue was created. `.pi/` was not staged or committed.

## Changes and evidence

### L1 — privacy and trust
- Doctor uses the bounded file reader and content-free read/decode errors. Format output is allowlisted rather than echoing the file prelude. Regressions cover private path/member errors and a valid document with an unexpected prelude.
- Decoder rejects node records without IDs, bounds schema identifiers and process-wide interning, avoids reserving arrays from untrusted lengths, limits decoded items and node records before reserve, and validates blob shapes. Existing encoded-file round-trip regression remains.
- Verified `cargo test -p figstudio-cli --test doctor` and `cargo test -p figstudio-fig`; full workspace gate passed.

### L2 — rendering
- Iterative traversal in CLI tree and Layers view; fit bounds include visible descendants outside unclipped groups; cache accounts for retained source bytes; instance override matching is indexed; unsupported geometry/blend omissions are counted; frame clipping and mask scope have pixel assertions.
- Verified focused render/CLI tests and full workspace gate. Thumbnail sheet was not generated: Pillow is unavailable.

### L3 — desktop, synth, and lean
- Synth copied nodes receive unique local IDs; integration test checks uniqueness and reopened document counts.
- A disconnected render worker now fails submissions visibly and the canvas reports rendering unavailable instead of suggesting reopening the file will recover it.
- Removed synth forwarding wrappers, merged duplicate open-action tests while retaining retry/focus cases, removed the cast-only UINT64 test while keeping codec round-trip coverage, and removed the dead renderer limit alias. The impossible optional pixmap cache state had already been removed in L2.
- Verified focused app/CLI/render tests and the full gate.

## Gates and commits

All commits are local; the branch is not pushed.

| Lane | Commit | Gate |
|---|---|---|
| L1 | `ba336ff` | `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh` passed |
| L2 | `247b6e4` | `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh` passed |
| L3/decoder follow-up | `b533a4e`, `891b983`, `bae1d43` | Focused tests, final full gate, and private smoke passed |

Final validation: `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh` ended with `all checks passed` (fmt, clippy, all tests, release build). `scripts/local/private-smoke.sh` passed info, doctor, render, thumbnail, resave, reopen, and input-unchanged checks. `git diff --check` passed. `tools.mcp__ripwire__quality_delta` reported no gating regressions; its two non-minor entries were generated `.pi/` task artifacts, intentionally left untouched.

## Not verified / limits

- No live GUI interaction, renderer-worker crash injection, visual thumbnail-sheet/pixel-parity review, platform/Wayland check, fuzz campaign, or destructive memory-exhaustion test.
- No performance benchmark was run for the wide override case; the index change was verified by the existing renderer tests.
- Private-file smoke output stayed under `target/private-smoke/`; the input hash remained unchanged. No private filename or document content is included here.
