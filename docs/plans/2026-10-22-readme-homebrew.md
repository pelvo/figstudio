---
pr: 55
branch: docs/readme-homebrew
title: feat(ci,docs): linux release binaries and homebrew install docs
---

## Progress

### 2026-10-22T093636Z b289-readme-and-the-mcp-readme-document
- 2026-10-22: README and the MCP README document `brew install --cask pelvo/tap/figstudio` (macOS arm64). Checked on Homebrew 7.0.9: a fresh tap and install from GitHub puts the app in /Applications, links `figstudio-cli` and `figstudio-mcp`, clears quarantine, and the app opens a fixture; `brew upgrade --cask` works. Not run: `--zap`, Intel Macs, Linux.

### 2026-10-22T093636Z b290-releases-also-ship-static-linux-cli
- 2026-10-22: Releases also ship static Linux CLI + MCP tarballs (`figstudio-cli-mcp-vX.Y.Z-linux-{x86_64,arm64}.tar.gz`, musl, native `ubuntu-latest` and `ubuntu-24.04-arm` runners); a last `checksums` job writes one `SHA256SUMS` for every asset. README shows the Linux download. Checked: actionlint, the job steps in clean ubuntu:24.04 containers for both arches, the binaries on other distros, and glibc vs musl render speed (same within noise). Not run: the workflow itself on GitHub (next release).
