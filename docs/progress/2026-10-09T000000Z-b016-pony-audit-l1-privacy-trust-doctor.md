- 2026-10-09 — Pony audit L1 privacy/trust: doctor now bounds its input reads and keeps archive
  paths/schema prelude out of output; Kiwi list lengths and schema interning are bounded, and
  ID-less node records are rejected rather than dropped. L2: iterative layer/CLI walks, descendant
  fit bounds, retained source-byte cache accounting, indexed override lookup, unsupported draw
  telemetry and clip/mask pixel checks. Thumb sheet skipped (Pillow unavailable). Both lanes passed
  `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh`; L3 synth IDs, render-worker unavailable state,
  wrapper/test cleanup, decoded-item and schema limits, and doctor prelude privacy passed the gate.
  Private-file smoke passed info/doctor/render/thumbnail/resave/reopen with the input unchanged.
  Pony audit follow-up: all 15 non-false-positive findings fixed; see `target/ponyfix/report.md`.
