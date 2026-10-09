#!/usr/bin/env bash
# GUI smoke test on a virtual X screen (Linux only). Nothing appears on your desktop.
#
#   scripts/gui-smoke.sh <out-dir> [file.fig]
#
# Needs: xvfb-run, xdotool, ImageMagick `import`, and a release build of the app
# (cargo build --release -p figstudio). Writes screenshots and the per-frame profile log:
#   <out-dir>/1-open.png    after the file (or the empty state) has loaded
#   <out-dir>/2-select.png  after a click at GUI_SMOKE_CLICK (screen "x y", default "700 450",
#                           the canvas centre; for structure.fig use "900 560" to hit a card)
#   <out-dir>/3-zoom.png    after Ctrl+wheel zoom in at the centre
#   <out-dir>/4-fit.png     after Shift+1 (zoom to fit)
#   <out-dir>/profile.log   FIGSTUDIO_PROFILE output (UI ms per frame, render ms, culled)
# Open the PNGs and look at them; the script only drives the app.
#
# Lessons baked in: the window must be focused before keys and modifier clicks work
# (there is no window manager); the app is stopped with `timeout`, not by sending signals
# by hand; Shift+1 arrives in egui as '!' on US layouts (the app handles both).
set -euo pipefail

if [ "${1:-}" = "--inner" ]; then
  shift
  out=$1
  file=${2:-}
  app="$(cd "$(dirname "$0")/.." && pwd)/target/release/figstudio"
  export LIBGL_ALWAYS_SOFTWARE=1 WINIT_UNIX_BACKEND=x11 FIGSTUDIO_PROFILE=1
  if [ -n "$file" ]; then
    timeout "${GUI_SMOKE_SECONDS:-20}" "$app" "$file" >"$out/profile.log" 2>&1 &
  else
    timeout "${GUI_SMOKE_SECONDS:-20}" "$app" >"$out/profile.log" 2>&1 &
  fi
  sleep "${GUI_SMOKE_LOAD_WAIT:-5}"
  wid=$(xdotool search --name figstudio | head -1 || true)
  if [ -z "$wid" ]; then
    echo "gui-smoke: window did not appear; see $out/profile.log" >&2
    wait || true
    exit 1
  fi
  xdotool windowfocus --sync "$wid"
  import -window root "$out/1-open.png"
  # shellcheck disable=SC2086
  xdotool mousemove ${GUI_SMOKE_CLICK:-700 450} click 1
  sleep 1
  import -window root "$out/2-select.png"
  xdotool mousemove 700 450 sleep 0.3 keydown Control_L
  for _ in 1 2 3 4 5 6; do xdotool click 4; sleep 0.15; done
  xdotool keyup Control_L
  sleep 1.5
  import -window root "$out/3-zoom.png"
  xdotool key shift+1
  sleep 1.5
  import -window root "$out/4-fit.png"
  wait || true
  echo "gui-smoke: screenshots in $out"
  exit 0
fi

out=${1:?usage: scripts/gui-smoke.sh <out-dir> [file.fig]}
file=${2:-}
mkdir -p "$out"
out=$(cd "$out" && pwd)
if [ -n "$file" ]; then
  file=$(cd "$(dirname "$file")" && pwd)/$(basename "$file")
fi
for tool in xvfb-run xdotool import; do
  command -v "$tool" >/dev/null || { echo "gui-smoke: $tool not installed" >&2; exit 1; }
done
[ -x "$(dirname "$0")/../target/release/figstudio" ] || {
  echo "gui-smoke: build first: cargo build --release -p figstudio" >&2
  exit 1
}
xvfb-run -a -s "-screen 0 1400x880x24" "$0" --inner "$out" "$file"
