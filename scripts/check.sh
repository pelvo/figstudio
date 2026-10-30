#!/usr/bin/env bash
# The local gate. Run it before every commit; all steps must pass.
#
#   scripts/check.sh            fmt, clippy (-D warnings), tests, release build
#   scripts/check.sh --cross    also `cargo check` for macOS and Windows
#   scripts/check.sh --web      also build the browser app (scripts/web-build.sh; nightly)
#   scripts/check.sh --lint     only fmt and clippy (CI job "fmt + clippy")
#   scripts/check.sh --test     tests, release build, wasm32 (CI job "build + test")
#   scripts/check.sh --lint-one NAME
#   scripts/check.sh --test-one NAME
#                               one slice, for a local run. CI does not call these.
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

# Each slice is one cargo invocation. A local run walks them in order in one target dir.
run_slice() {
  case "$1" in
    fmt)
      cargo $TC fmt --all -- --check
      ;;
    clippy-workspace)
      cargo $TC clippy --profile fast --workspace --all-targets -- -D warnings
      ;;
    # The browser build's strip renderer (feature off on desktop).
    clippy-parallel)
      cargo $TC clippy --profile fast -p figstudio-render --features parallel --all-targets -- -D warnings
      ;;
    # GPU tile replay (vello_gpu; the app wires it). Its pixel tests skip without a GPU adapter.
    clippy-gpu)
      cargo $TC clippy --profile fast -p figstudio-render --features gpu --all-targets -- -D warnings
      ;;
    # The app without agent access (feature `mcp` off), as the browser build compiles it.
    clippy-app-no-mcp)
      cargo $TC clippy --profile fast -p figstudio --no-default-features --all-targets -- -D warnings
      ;;
    # Screen readers: the shipped desktop app must build eframe with AccessKit. The UI tests
    # get it from egui_kittest (a dev-dependency), which a release build does not inherit,
    # so the tests alone cannot catch it missing. `-e normal` leaves dev-dependencies out;
    # `cargo tree` resolves for the host unless given a --target. The browser build has no
    # AccessKit; its "Read controls aloud" needs web_screen_reader, checked for wasm32.
    accesskit)
      features=$(cargo $TC tree -p figstudio -e normal -e features -i eframe)
      if ! grep -q 'eframe feature "accesskit"' <<<"$features"; then
        echo "the desktop app builds eframe without its accesskit feature (apps/figstudio/Cargo.toml)" >&2
        exit 1
      fi
      echo "eframe is built with accesskit"
      web=$(cargo $TC tree -p figstudio -e normal -e features -i eframe --target wasm32-unknown-unknown)
      if ! grep -q 'eframe feature "web_screen_reader"' <<<"$web"; then
        echo "the browser app builds eframe without web_screen_reader (apps/figstudio/Cargo.toml)" >&2
        exit 1
      fi
      echo "the browser build has web_screen_reader"
      ;;
    tests-workspace)
      cargo $TC test --profile fast --workspace
      ;;
    # `gpu` includes `parallel`: the replay tests plus the GPU-vs-CPU tile tests.
    tests-gpu)
      cargo $TC test --profile fast -p figstudio-render --features gpu
      ;;
    # mcp-core as the app and the browser use it: no files, no system fonts.
    tests-mcp-core)
      cargo $TC test --profile fast -p figstudio-mcp-core --no-default-features
      ;;
    # The browser's text: supplied fonts only, no installed-font discovery (`system-fonts` off).
    tests-browser-fonts)
      cargo $TC test --profile fast -p figstudio-render --no-default-features --features parallel --test registered_fonts
      ;;
    release)
      cargo $TC build --release --workspace
      ;;
    # The npm launcher (npm/figstudio-mcp) and scripts/npm-pack.mjs. The files, not the
    # folder: Node 22 (the CI runner's) loads a folder argument as a module and fails. An
    # unmatched pattern would reach node literally and pass with zero tests, so no match
    # is an error.
    tests-npm)
      if command -v node >/dev/null 2>&1; then
        npm_tests=(npm/figstudio-mcp/test/*.test.mjs)
        [ -e "${npm_tests[0]}" ] || { echo "no npm tests in npm/figstudio-mcp/test" >&2; exit 1; }
        node --test "${npm_tests[@]}"
      else
        echo "skipped: node is not installed"
      fi
      ;;
    wasm32)
      if rustup target list --installed $TC_FLAG 2>/dev/null | grep -qx wasm32-unknown-unknown; then
        cargo $TC build -p figstudio-mcp-core --no-default-features --target wasm32-unknown-unknown
      else
        echo "skipped: run 'rustup target add wasm32-unknown-unknown $TC_FLAG' first"
      fi
      ;;
    *)
      echo "unknown check slice: $1" >&2
      echo "lint: fmt clippy-workspace clippy-parallel clippy-gpu clippy-app-no-mcp accesskit" >&2
      echo "test: tests-workspace tests-gpu tests-mcp-core tests-npm tests-browser-fonts release wasm32" >&2
      exit 2
      ;;
  esac
}

MODE="${1:-}"
SLICE="${2:-}"
lint_names="fmt clippy-workspace clippy-parallel clippy-gpu clippy-app-no-mcp accesskit"
test_names="tests-workspace tests-gpu tests-mcp-core tests-npm tests-browser-fonts release wasm32"
if [ "$MODE" = "--lint-one" ] || [ "$MODE" = "--test-one" ]; then
  [ -n "$SLICE" ] || { echo "$MODE needs a slice name" >&2; exit 2; }
  case "$MODE" in
    --lint-one) allowed="$lint_names" ;;
    --test-one) allowed="$test_names" ;;
  esac
  ok=""
  for name in $allowed; do
    [ "$name" = "$SLICE" ] && ok=1
  done
  [ -n "$ok" ] || { echo "$MODE does not run $SLICE" >&2; echo "allowed: $allowed" >&2; exit 2; }
  step "$SLICE"
  run_slice "$SLICE"
  exit 0
fi

if [ "$MODE" != "--test" ]; then
  for name in $lint_names; do
    step "$name"
    run_slice "$name"
  done
fi
[ "$MODE" = "--lint" ] && { step "lint checks passed"; exit 0; }

# --test includes release and wasm32. A pull request never runs the macOS release job,
# and that job does not build wasm32, so dropping either here would leave the required
# check green for a break it used to catch.
for name in tests-workspace tests-gpu tests-mcp-core tests-npm tests-browser-fonts release wasm32; do
  step "$name"
  run_slice "$name"
done

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
