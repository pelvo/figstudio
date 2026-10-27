# Working on figstudio (read this first)

figstudio opens Figma `.fig` local copies: a Rust decoder/writer, a Vello CPU renderer, a
headless CLI and an egui desktop app. This file is the entry point for any agent or person
picking up work. Read the three documents below before changing code.

| Document | What it is for |
|---|---|
| `docs/PLAN.md` | Requirements, phases, **what is done and what is open**, per-PR checklists and progress (`docs/plans/`) |
| `docs/IMPLEMENTATION.md` | Architecture, data flow, how to add a feature, gotchas, task board |
| `docs/TESTING.md` | Every test and check, how to write new ones, how to prove a test works |
| `apps/figstudio-mcp/README.md` | The offline MCP server: install, client setup, tools, limits |
| `.agents/skills/compare-fig-versions/SKILL.md` | How an agent compares two .fig versions with the CLI or MCP |
| `docs/MCP-PLAN.md` | MCP server plan, lane notes, what shipped and what is open |
| `docs/APPROXIMATIONS.md` | What each status-bar "approximated"/"skipped" counter is made of, and the plan to clear it |

## Ground rules

1. **Pick a task from the task board** in `docs/IMPLEMENTATION.md` (or the open boxes in
   `docs/PLAN.md`). Keep to its scope; do not refactor unrelated code.
2. **Run `scripts/check.sh` before every commit.** It must end with `all checks passed`
   (fmt, clippy `-D warnings`, all tests, release build). If your default Rust is older than
   1.95: `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh` (any 1.95+ works). CI runs the same
   halves (`--lint`, `--test`) as the two required checks.
3. **Every behaviour change comes with a test**, and you must have seen that test fail
   without your change (see "Prove the test" in `docs/TESTING.md`).
4. **Rendering changes need a picture.** Run `scripts/thumb-sheet.py` and look at the sheet.
   Green tests do not mean the render is right.
5. **Never panic on a bad file.** Malformed input returns `FigError`; the app shows a toast.
6. **Unsupported means counted, not hidden.** Anything not drawn exactly increments a field
   in `Stats` so the app (warning chip and status pill) and the CLI show it.
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
10. **No new `unsafe`.** The workspace denies it. The only exceptions, each one block with a
    SAFETY comment and `#[expect(unsafe_code)]`: the FFI call in `doctor.rs`, the Windows
    menu attach in `menu_native.rs`, the macOS blur view in `vibrancy.rs`, the macOS Dock
    menu hook in `dock_mac.rs`, the Windows jump list in `jump_list.rs`, the Windows
    foreground permission in `handoff.rs` and the Windows foreground request in
    `foreground_win.rs` (approved by the owner). Ask before adding another.
11. **Update the plan** when you finish something: start your branch's file with
    `scripts/plan.sh new <slug> "<title>"` (`docs/plans/<date>-<slug>.md`; no PR needed), tick
    the box in it only after its check really passed, and add a progress entry with
    `scripts/plan.sh note <slug>`. One file per branch, so branches do not conflict; add the PR
    number later with `plan.sh link`. `docs/PLAN.md` keeps phases and prose only. Report what
    you did **not** verify.
12. **Use trustworthy crates; do not hand-roll.** For protocols, formats and encodings
    (JSON, MCP/JSON-RPC, HTTP, XML, base64, CLI parsing in new code, …) use a well-known,
    maintained crate (official SDK, high download count, active repo) instead of writing
    your own. State the crate and the reason in the commit message. Hand-written code is
    only for figstudio's own logic.
13. **Follow the Rust guidelines.** New Rust code follows Microsoft's Pragmatic Rust
    Guidelines (the `ms-rust-guidelines` skill, `M-*` IDs): canonical error structs,
    no panics on bad input, `Debug` on public types, integration tests under `tests/`,
    `#[expect]` for lint overrides. Workers cite the `M-*` IDs they applied; reviewers check
    against them.

## Quick start

```sh
scripts/check.sh                                        # the gate (first run compiles everything)
cargo run --release -p figstudio-cli -- info tests/fixtures/structure.fig
cargo run --release -p figstudio-cli -- render tests/fixtures/structure.fig -o /tmp/s.png --scale 2
cargo run --release -p figstudio -- tests/fixtures/structure.fig   # desktop app
scripts/macos-app.sh --install                          # macOS: figstudio.app in ~/Applications
scripts/web-build.sh --serve                            # browser app (nightly) on :8765
```

Linux needs the usual eframe/rfd system libraries (X11 or Wayland, a Vulkan or OpenGL/EGL
driver for wgpu, GTK/portal for the file dialog). macOS needs only Xcode command line tools.

## Commits and branches

- Small commits with a body that says what changed and how it was checked.
- PR titles use Conventional Commits: `type(scope): summary`, for example
  `fix(render): draw inner shadows and every drop shadow`. Types: `feat`, `fix`, `perf`,
  `refactor`, `test`, `docs`, `chore`. Scope is the crate or app (`fig`, `render`, `cli`,
  `app`, `web`) or several joined with commas. Summary is lower case, imperative, no full
  stop, under 72 characters.
- `main` is the integration branch. Work on a branch and open a PR unless told otherwise.
- Releases: release-please (`.github/workflows/release-please.yml`) keeps one release PR
  open from the Conventional Commit titles on `main`. Merging it bumps the version in
  `Cargo.toml`/`Cargo.lock`, writes `CHANGELOG.md`, tags `vX.Y.Z` and attaches the macOS
  app (`figstudio-vX.Y.Z-macos-arm64.zip`) and the CLI/MCP binaries to the GitHub release.
  Do not bump versions or edit `CHANGELOG.md` by hand.
- Do not commit `target/`, generated PNGs, stress files (`synth` output) or private `.fig`
  files. Only the 7 fixtures in `tests/fixtures` (MIT, from fig2sketch) belong in the repo.
- Do not put people's names, chat names, phone numbers or local paths in commits, issues or
  docs. Say "the requester" or "a user".
- **PII check before every commit and push** (ground rule 9). Run
  `git diff --cached` and `git log -p origin/main..HEAD` and look for names, Japanese or
  other UI text from a private file, node ids (`12345:67890`), paths and screenshots, in
  the code, the tests, the comments and the commit messages. If one slipped into an
  unpushed commit, rewrite that commit before pushing; a pushed one needs the owner.
