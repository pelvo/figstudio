# Working on figstudio (read this first)

figstudio opens Figma `.fig` local copies: a Rust decoder/writer, a Vello CPU renderer, a
headless CLI and an egui desktop app. This file is the entry point for any agent or person
picking up work. Read the three documents below before changing code.

| Document | What it is for |
|---|---|
| `docs/PLAN.md` | Requirements, phases, **what is done and what is open**, progress log |
| `docs/IMPLEMENTATION.md` | Architecture, data flow, how to add a feature, gotchas, task board |
| `docs/TESTING.md` | Every test and check, how to write new ones, how to prove a test works |

## Ground rules

1. **Pick a task from the task board** in `docs/IMPLEMENTATION.md` (or the open boxes in
   `docs/PLAN.md`). Keep to its scope; do not refactor unrelated code.
2. **Run `scripts/check.sh` before every commit.** It must end with `all checks passed`
   (fmt, clippy `-D warnings`, all tests, release build). If your default Rust is older than
   1.95: `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh` (any 1.95+ works).
3. **Every behaviour change comes with a test**, and you must have seen that test fail
   without your change (see "Prove the test" in `docs/TESTING.md`).
4. **Rendering changes need a picture.** Run `scripts/thumb-sheet.py` and look at the sheet.
   Green tests do not mean the render is right.
5. **Never panic on a bad file.** Malformed input returns `FigError`; the app shows a toast.
6. **Unsupported means counted, not hidden.** Anything not drawn exactly increments a field
   in `Stats` so the app status bar and the CLI show it.
7. **Keep the document writable.** Do not drop fields when decoding; `crates/fig` must keep
   writing the fixtures back byte for byte (`encoded_message_matches_the_original_bytes`).
8. **Privacy.** `figstudio-cli doctor` output is pasted publicly by users with private
   files. It must never print names, text, fonts, image hashes, colours or paths. Do not add
   such output; `apps/figstudio-cli/tests/doctor.rs` checks this.
9. **No PII or private-file content in the repo.** Before every commit and push, remove it
   from code, tests, comments, docs and commit messages. That covers anything taken from a
   user's private `.fig`: layer, page, frame and component names, text content, node ids,
   colours, image hashes, file paths and screenshots, plus people's names, chat names and
   local paths. Describe a bug in neutral terms ("a table frame on a print-layout page").
   Test with public fixtures or documents built in the test, never a private file. Notes
   that need real names (such as a local gap list) stay untracked and git-ignored.
10. **No new `unsafe`.** The workspace denies it; the one FFI call is in `doctor.rs`.
11. **Update `docs/PLAN.md`** when you finish something: tick the box only after its check
    really passed, and add one line to the progress log. Report what you did **not** verify.

## Quick start

```sh
scripts/check.sh                                        # the gate (first run compiles everything)
cargo run --release -p figstudio-cli -- info tests/fixtures/structure.fig
cargo run --release -p figstudio-cli -- render tests/fixtures/structure.fig -o /tmp/s.png --scale 2
cargo run --release -p figstudio -- tests/fixtures/structure.fig   # desktop app
scripts/web-build.sh --serve                            # browser app (nightly) on :8765
```

Linux needs the usual eframe/rfd system libraries (X11 or Wayland, OpenGL, GTK/portal for
the file dialog). macOS needs only Xcode command line tools.

## Commits and branches

- Small commits with a body that says what changed and how it was checked.
- `main` is the integration branch. Work on a branch and open a PR unless told otherwise.
- Do not commit `target/`, generated PNGs, stress files (`synth` output) or private `.fig`
  files. Only the 7 fixtures in `tests/fixtures` (MIT, from fig2sketch) belong in the repo.
- Do not put people's names, chat names, phone numbers or local paths in commits, issues or
  docs. Say "the requester" or "a user".
- **PII check before every commit and push** (ground rule 9). Run
  `git diff --cached` and `git log -p origin/main..HEAD` and look for names, Japanese or
  other UI text from a private file, node ids (`12345:67890`), paths and screenshots, in
  the code, the tests, the comments and the commit messages. If one slipped into an
  unpushed commit, rewrite that commit before pushing; a pushed one needs the owner.
