---
pr: 25
branch: chore/macos-release
title: chore(app): build a macos app bundle and release it with release-please
---

## Progress

### 2026-10-19T044851Z b127-macos-app-bundle-and-releases-scripts
- 2026-10-19 — macOS app bundle and releases: `scripts/macos-app.sh` builds an ad-hoc signed `figstudio.app` (app + figstudio-mcp + figstudio-cli; `--install` copies it to ~/Applications); release-please (simple strategy, TOML extra-files for the workspace version and the figstudio entries in Cargo.lock) keeps a release PR, and on release a macos-latest job attaches the zipped app, a CLI/MCP tarball and SHA256SUMS. Verified locally: bundle built, `codesign --verify --deep --strict` passed, unzipped app launched, both helper binaries ran, the TOML updaters bumped exactly the 8 version fields. Not verified: the workflow on GitHub (needs the Actions PR permission), Gatekeeper on a downloaded copy (not notarized), Intel Macs, opening a .fig by double-click.

### 2026-10-19T044851Z b128-pr-review-fixes-for-the-macos
- 2026-10-19 — PR review fixes for the macOS release: `macos-app.sh` runs under macOS Bash 3.2 without FIGSTUDIO_TOOLCHAIN (old script failed with an unbound `TC[@]`, seen red then green with a cargo stub), rejects unknown arguments, builds `--locked` and honours CARGO_TARGET_DIR; the workflow scopes permissions per job, pins actions to SHAs, hashes only the two archives and can rebuild an existing tag by hand (`workflow_dispatch`). actionlint 1.7.12 clean, real build under /bin/bash passed codesign. Not verified: the workflow on GitHub, the CARGO_TARGET_DIR path.
