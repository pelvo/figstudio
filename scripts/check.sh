#!/usr/bin/env bash
# The local gate. Run it before every commit; all steps must pass.
#
#   scripts/check.sh            fmt, clippy (-D warnings), tests, release build
#   scripts/check.sh --cross    also `cargo check` for macOS and Windows
#   scripts/check.sh --web      also build the browser app (scripts/web-build.sh; nightly)
#   scripts/check.sh --lint     only fmt and clippy (CI job "fmt + clippy")
#   scripts/check.sh --test     only tests and builds (CI job "build + test")
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

# Lints and tests use the `fast` profile (Cargo.toml): release optimisation, no LTO,
# incremental. `--release` here would thin-LTO link every test binary (~20 min after a
# change in crates/fig). The "release build" step below still checks the shipped LTO build.

MODE="${1:-}"
if [ "$MODE" != "--test" ]; then
step "fmt"
cargo $TC fmt --all -- --check

step "clippy"
cargo $TC clippy --profile fast --workspace --all-targets -- -D warnings
# The browser build's strip renderer (feature off on desktop).
cargo $TC clippy --profile fast -p figstudio-render --features parallel --all-targets -- -D warnings
# GPU tile replay (vello_gpu; the app wires it). Its pixel tests skip without a GPU adapter.
cargo $TC clippy --profile fast -p figstudio-render --features gpu --all-targets -- -D warnings
# The app without agent access (feature `mcp` off), as the browser build compiles it.
cargo $TC clippy --profile fast -p figstudio --no-default-features --all-targets -- -D warnings
fi
[ "$MODE" = "--lint" ] && { step "lint checks passed"; exit 0; }

step "tests"
cargo $TC test --profile fast --workspace
# `gpu` includes `parallel`: the replay tests plus the GPU-vs-CPU tile tests.
cargo $TC test --profile fast -p figstudio-render --features gpu
# mcp-core as the app and the browser use it: no files, no system fonts.
cargo $TC test --profile fast -p figstudio-mcp-core --no-default-features

step "release build"
cargo $TC build --release --workspace

step "mcp-core for wasm32"
if rustup target list --installed $TC_FLAG 2>/dev/null | grep -qx wasm32-unknown-unknown; then
  cargo $TC build -p figstudio-mcp-core --no-default-features --target wasm32-unknown-unknown
else
  echo "skipped: run 'rustup target add wasm32-unknown-unknown $TC_FLAG' first"
fi

if [ "$MODE" = "--cross" ]; then
  step "macOS check (aarch64-apple-darwin)"
  if rustup target list --installed $TC_FLAG 2>/dev/null | grep -qx aarch64-apple-darwin; then
    cargo $TC check --release --workspace --target aarch64-apple-darwin
  else
    echo "skipped: run 'rustup target add aarch64-apple-darwin $TC_FLAG' first"
  fi
  step "Windows check (x86_64-pc-windows-msvc)"
  if rustup target list --installed $TC_FLAG 2>/dev/null | grep -qx x86_64-pc-windows-msvc; then
    cargo $TC check -p figstudio --target x86_64-pc-windows-msvc
  else
    echo "skipped: run 'rustup target add x86_64-pc-windows-msvc $TC_FLAG' first"
  fi
fi

if [ "$MODE" = "--web" ]; then
  step "browser build (wasm32, threads)"
  scripts/web-build.sh
fi

step "all checks passed"
