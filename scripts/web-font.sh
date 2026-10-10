#!/usr/bin/env bash
# Makes the CJK font the browser app loads when a file has Chinese / Japanese names:
# Noto Sans CJK HK Regular (SIL OFL 1.1), cut down to Latin, punctuation, kana, bopomofo,
# full-width forms and the CJK Unified Ideographs block (about 5 MB, 3.5 MB compressed).
# The browser has no system fonts the app can use, and egui's built-in fonts are Latin only.
#
#   scripts/web-font.sh <out-dir>     writes <out-dir>/cjk.otf and <out-dir>/cjk-LICENSE.txt
#
# Needs fontTools (`pip install fonttools`, gives `pyftsubset`) and Noto Sans CJK, either
# installed (Arch: noto-fonts-cjk; Debian: fonts-noto-cjk) or named with FIGSTUDIO_CJK_FONT
# (a NotoSansCJK-Regular.ttc, or a single-face .otf with FIGSTUDIO_CJK_FONT_INDEX=0).
# Missing either, it prints a warning and writes nothing: the app then shows CJK as boxes.
# The result is cached in target/web-font (rebuilt when this script changes).
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:?usage: scripts/web-font.sh <out-dir>}

src=${FIGSTUDIO_CJK_FONT:-}
if [ -z "$src" ]; then
  for f in /usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc \
           /usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc \
           /usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc \
           "$HOME/Library/Fonts/NotoSansCJK-Regular.ttc" \
           /Library/Fonts/NotoSansCJK-Regular.ttc; do
    if [ -f "$f" ]; then src=$f; break; fi
  done
fi
# Face 4 of the collection is Noto Sans CJK HK (0 JP, 1 KR, 2 SC, 3 TC).
index=${FIGSTUDIO_CJK_FONT_INDEX:-4}
if [ -z "$src" ] || ! command -v pyftsubset >/dev/null; then
  echo "web-font: no Noto Sans CJK or no pyftsubset; CJK names will show as boxes" >&2
  exit 0
fi

cache=target/web-font
mkdir -p "$cache" "$out"
if [ ! -f "$cache/cjk.otf" ] || [ "$0" -nt "$cache/cjk.otf" ] || [ "$src" -nt "$cache/cjk.otf" ]; then
  # No layout features or hints: egui needs neither, and they are a third of the size.
  pyftsubset "$src" --font-number="$index" \
    --unicodes="U+0020-007E,U+00A0-00FF,U+2000-206F,U+2190-21FF,U+2460-24FF,U+25A0-25FF,U+3000-303F,U+3040-30FF,U+3100-312F,U+31F0-31FF,U+4E00-9FFF,U+FE30-FE4F,U+FF00-FFEF" \
    --layout-features='' --no-hinting --name-IDs='*' --name-languages='*' \
    --output-file="$cache/cjk.otf.part"
  # Only a finished file counts (an interrupted run leaves the .part behind).
  cp "$cache/cjk.otf.part" "$cache/cjk.otf"
fi
cp "$cache/cjk.otf" "$out/cjk.otf"
license=""
for l in "$(dirname "$src")/LICENSE" /usr/share/licenses/noto-fonts-cjk/LICENSE \
         /usr/share/doc/fonts-noto-cjk/copyright; do
  if [ -f "$l" ]; then license=$l; break; fi
done
{
  echo "cjk.otf: a subset of Noto Sans CJK HK Regular (https://github.com/notofonts/noto-cjk),"
  echo "licensed under the SIL Open Font License 1.1 (https://openfontlicense.org)."
  if [ -n "$license" ]; then echo; cat "$license"; fi
} >"$out/cjk-LICENSE.txt"
