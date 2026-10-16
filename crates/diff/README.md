# Structural document diff

For CLI and MCP usage, follow the [compare-fig-versions agent skill](../../.agents/skills/compare-fig-versions/SKILL.md).

`diff_documents(&old, &new)` compares stored fields across decoded documents.
`DiffReport::to_json()` writes deterministic compact JSON; `render_html(&report)`
builds a self-contained escaped HTML report. Neither writes files or renders pixels.

- GUIDs match globally, including across visible and internal pages.
- Unchanged pages are omitted. Page order follows the old document, then newly
  added pages in new-document order. Node rows use fractional sibling positions,
  with GUIDs breaking ties; properties use lexical dotted-path order.
- Added/removed rows represent unmatched subtree roots. `descendants` is the
  document's subtree size excluding the root, including any matched descendants
  which moved into/out of it. Summary additions/removals count only unmatched
  nodes, not page or document containers. Moves count as changed nodes too.
- Changes are flat within each page; there is no children field. A cross-page
  move appears once on its destination page and marks both pages changed.
  The source page's additive `moved_out` IDs explain its change without counting
  those nodes twice.
- Every stored differing leaf is retained, except redundant `parentIndex*` rows
  when a node has a `Move`; those rows are excluded from property counts.
  Arrays use dotted numeric indices. Container type changes and empty containers
  are atomic values, so an empty object appearing is not lost. Absence is `None`.
- `Val` equality supplies exact numeric comparison, including same-bit NaNs.
- `complete` concerns grouping of stored node changes only. Ungrouped changes
  (document-root or orphan nodes) set it false and add a count-only coverage note.
  Matched ungrouped nodes still contribute to `nodes_changed`, `nodes_moved` and
  `properties_changed`; their rows are not included under a page.
  Image/blob bytes and document metadata are not node fields and are not diffed.

## Phase 1 choices

The plan does not define `NodeRef` or `RunChange`. `NodeRef` carries identity,
kind, name, ancestor path and descendant count. `RunChange` carries a stored
`textData.styleOverrideTable` array index and property deltas. It does not infer
rendered text spans or resolve instance overrides. Font-only changes still carry
unchanged characters in `TextChange`; absent characters use an empty string there,
while the canonical property deltas preserve absence.

`FileMeta.path_label` is empty because `Document` has no source path. Its
`file_name` is a document title, not a basename. Page counts include internal
pages. `PageDiff.render` defaults to None; the optional render lane populates PNG
links, pixel measures and renderer statistics. The crate depends on
`figstudio-render` with `system-fonts` enabled (other default features disabled).
Existing workspace `serde`/`serde_json` implement JSON and derives;
`html-escape` handles user text without hand-rolled encoding.

Public types derive `Debug` (M-PUBLIC-DEBUG), and integration tests exercise
the public API (M-INTEGRATION-TESTS).
The installed skills do not include `ms-rust-guidelines`; these project-requested
rules and the available Rust guidelines were applied directly.

## JSON and HTML contract

Struct keys follow declaration order and object keys sort lexically without
changing decoder order or binary encoding. `Guid` is a two-integer array.
Natural JSON cannot distinguish enum names from strings, or byte buffers from
integer arrays: deserialization normalizes these to `Val::Str` and `Val::Arr`.
Round-trip equality therefore holds for the JSON-native subset, not every `Val`.
Numeric equality in structural comparison is unchanged. The diff dependency enables
serde_json's `float_roundtrip` parser for bit-exact finite f64 decoding; other serde
consumers need the same feature if they require that guarantee. NaN/infinity serialize
as `$float_bits` objects preserving their IEEE-754 bits (never null). HTML uses the
same JSON serialization, escaped for display, rather than a Debug representation.
`null` is reserved for absent optional fields; it is not a valid `Val`.
Object deserialization rejects duplicate keys and observes the process-wide
schema identifier count/byte budget before interning keys.

HTML escapes every user-provided string, has no scripts or external assets, and
sets `default-src 'none'` with inline styles allowed. Optional PNG links accept
relative paths only; they are links, not automatically loaded images. Artifact
labels, node names and text remain sensitive local report content.

## Checks

```sh
cargo +1.99.0 test -p figstudio-diff
cargo +1.99.0 clippy -p figstudio-diff --all-targets -- -D warnings
```

Tests build every compared node in code. A public fixture provides only the
otherwise-private document container/schema, using `Document::set_nodes` as in
existing integration tests. No private project content is used.
