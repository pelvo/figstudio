#!/usr/bin/env bash
# Builds the browser app into web/dist (threads: shared Wasm memory + Web Workers).
#
#   scripts/web-build.sh            release build
#   scripts/web-build.sh --serve    then serve web/dist on http://127.0.0.1:8765 with the
#                                   cross-origin isolation headers threads need
#
# Needs: a nightly toolchain with rust-src and the wasm32-unknown-unknown target
#   rustup toolchain install nightly --component rust-src --target wasm32-unknown-unknown
# and wasm-bindgen-cli at the same version as the wasm-bindgen crate in Cargo.lock:
#   cargo install --locked wasm-bindgen-cli --version <version>
# Override the toolchain with FIGSTUDIO_WEB_TOOLCHAIN (default: nightly).
set -euo pipefail
cd "$(dirname "$0")/.."

TC="${FIGSTUDIO_WEB_TOOLCHAIN:-nightly}"
OUT=web/dist

want=$(awk '/^name = "wasm-bindgen"$/{getline; gsub(/"/,"",$3); print $3; exit}' Cargo.lock)
have=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}')
if [ "$want" != "$have" ]; then
  echo "wasm-bindgen-cli $want needed (found: ${have:-none}):" >&2
  echo "  cargo install --locked wasm-bindgen-cli --version $want" >&2
  exit 1
fi

# atomics + bulk-memory: threads (std is rebuilt with them). The linker flags make the memory
# shared and imported, and export the TLS symbols wasm-bindgen needs to start each worker
# (current rustc no longer adds these by itself; same flags as wasm-bindgen's
# raytrace-parallel example). simd128: Vello CPU's SIMD paths. max-memory: up to 4 GiB.
link=""
for a in --shared-memory --import-memory --max-memory=4294967296 --export=__heap_base \
         --export=__wasm_init_tls --export=__tls_size --export=__tls_align --export=__tls_base; do
  link="$link -C link-arg=$a"
done
export RUSTFLAGS="-C target-feature=+atomics,+bulk-memory,+simd128$link"
cargo "+$TC" build --release -p figstudio --target wasm32-unknown-unknown \
  -Z build-std=panic_abort,std

mkdir -p "$OUT"
wasm-bindgen --target web --no-typescript --out-dir "$OUT" \
  target/wasm32-unknown-unknown/release/figstudio.wasm
cp web/index.html web/_headers "$OUT"/
# CJK font, fetched by the app only when a file has CJK names (see scripts/web-font.sh).
scripts/web-font.sh "$OUT/fonts"
ls -l "$OUT" "$OUT/fonts" 2>/dev/null | awk 'NF>5 {printf "%10d  %s\n", $5, $9}'

if [ "${1:-}" = "--serve" ]; then
  exec python3 scripts/web-serve.py "$OUT" 8765
fi
