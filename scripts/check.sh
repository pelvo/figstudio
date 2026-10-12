#!/usr/bin/env bash
# The local gate. Run it before every commit; all steps must pass.
#
#   scripts/check.sh            fmt, clippy (-D warnings), tests, release build
#   scripts/check.sh --cross    also `cargo check` for macOS (aarch64-apple-darwin)
#   scripts/check.sh --web      also build the browser app (scripts/web-build.sh; nightly)
#
# Toolchain: not pinned. egui 0.36 needs Rust 1.95+. If your default toolchain is older,
# set FIGSTUDIO_TOOLCHAIN, e.g. FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh
# Works with the bash 3.2 that ships with macOS.
set -euo pipefail
cd "$(dirname "$0")/.."

# Unquoted on purpose: empty, or "+<toolchain>".
TC=""
TC_FLAG=""
if [ -n "${FIGSTUDIO_TOOLCHAIN:-}" ]; then
  TC="+${FIGSTUDIO_TOOLCHAIN}"
  TC_FLAG="--toolchain ${FIGSTUDIO_TOOLCHAIN}"
fi

step() { printf '\n== %s\n' "$*"; }

step "toolchain"
rustc $TC --version

step "fmt"
cargo $TC fmt --all -- --check

step "clippy"
cargo $TC clippy --release --workspace --all-targets -- -D warnings
# The browser build's strip renderer (feature off on desktop).
cargo $TC clippy --release -p figstudio-render --features parallel --all-targets -- -D warnings
# GPU tile replay (vello_gpu; the app wires it). Its pixel tests skip without a GPU adapter.
cargo $TC clippy --release -p figstudio-render --features gpu --all-targets -- -D warnings
# The app without agent access (feature `mcp` off), as the browser build compiles it.
cargo $TC clippy --release -p figstudio --no-default-features --all-targets -- -D warnings

step "tests"
cargo $TC test --release --workspace
# `gpu` includes `parallel`: the replay tests plus the GPU-vs-CPU tile tests.
cargo $TC test --release -p figstudio-render --features gpu
# mcp-core as the app and the browser use it: no files, no system fonts.
cargo $TC test --release -p figstudio-mcp-core --no-default-features

step "release build"
cargo $TC build --release --workspace

step "mcp-core for wasm32"
if rustup target list --installed $TC_FLAG 2>/dev/null | grep -qx wasm32-unknown-unknown; then
  cargo $TC build -p figstudio-mcp-core --no-default-features --target wasm32-unknown-unknown
else
  echo "skipped: run 'rustup target add wasm32-unknown-unknown $TC_FLAG' first"
fi

if [ "${1:-}" = "--cross" ]; then
  step "macOS check (aarch64-apple-darwin)"
  if rustup target list --installed $TC_FLAG 2>/dev/null | grep -qx aarch64-apple-darwin; then
    cargo $TC check --release --workspace --target aarch64-apple-darwin
  else
    echo "skipped: run 'rustup target add aarch64-apple-darwin $TC_FLAG' first"
  fi
fi

if [ "${1:-}" = "--web" ]; then
  step "browser build (wasm32, threads)"
  scripts/web-build.sh
fi

step "all checks passed"
