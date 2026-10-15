#!/usr/bin/env bash
# Subset the five Noto Sans CJK Regular variants for the browser's lazy fallback.
# Usage: scripts/web-font.sh <out-dir> (cjk-{jp,sc,tc,hk,kr}.otf + cjk-LICENSE.txt).
# Needs fontTools (pyftsubset) and Noto Sans CJK (Debian: fonts-noto-cjk;
# Arch: noto-fonts-cjk; macOS: brew install --cask font-noto-sans-cjk).
# FIGSTUDIO_CJK_FONT overrides the collection path: NotoSansCJK-Regular.ttc
# uses faces 0..4; NotoSansCJK.ttc (all weights) uses Regular faces 25..29.
# It must be a .ttc collection; each output's face name is checked, so a wrong
# source fails the build instead of shipping one face under five names.
# Missing font or tool fails the build (a deploy without these fonts shows CJK
# names as boxes); FIGSTUDIO_ALLOW_MISSING_CJK=1 opts out for a local dev build.
# Cache: $CARGO_TARGET_DIR/web-font.
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:?usage: scripts/web-font.sh <out-dir>}

src=${FIGSTUDIO_CJK_FONT:-}
if [ -z "$src" ]; then
  for f in /usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc \
           /usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc \
           /usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc \
           "$HOME/Library/Fonts/NotoSansCJK-Regular.ttc" \
           /Library/Fonts/NotoSansCJK-Regular.ttc \
           "$HOME/Library/Fonts/NotoSansCJK.ttc"; do
    if [ -f "$f" ]; then src=$f; break; fi
  done
fi
subset=$(command -v pyftsubset || true)
if [ -z "$subset" ] && [ -x "$HOME/.local/bin/pyftsubset" ]; then
  subset=$HOME/.local/bin/pyftsubset
fi
# fontTools' own Python (pyftsubset may be a pipx/uv tool, not the system python3).
py=$(sed -n '1s/^#! *//p' "$subset" 2>/dev/null || true)
if [ -z "$py" ] || ! "$py" -c 'import fontTools' 2>/dev/null; then py=python3; fi
if [ ! -f "$src" ] || [ -z "$subset" ]; then
  if [ "${FIGSTUDIO_ALLOW_MISSING_CJK:-0}" = "1" ]; then
    echo "web-font: no Noto Sans CJK or no pyftsubset; CJK names will show as boxes" \
      "(allowed by FIGSTUDIO_ALLOW_MISSING_CJK=1)" >&2
    exit 0
  fi
  echo "web-font: no Noto Sans CJK or no pyftsubset; CJK names would show as boxes," \
    "so the build fails" >&2
  echo "web-font: install them (Debian: fonts-noto-cjk, Arch: noto-fonts-cjk," \
    "macOS: brew install --cask font-noto-sans-cjk; and fontTools: 'uv tool" \
    "install fonttools'), or set FIGSTUDIO_ALLOW_MISSING_CJK=1 for a local dev" \
    "build without CJK names" >&2
  exit 1
fi
case "$src" in
  *.ttc) ;;
  *) echo "web-font: $src is not a .ttc collection; need all five Noto Sans CJK faces" >&2
     exit 1 ;;
esac
base=0
if [ "$(basename "$src")" = NotoSansCJK.ttc ]; then base=25; fi
cache=${CARGO_TARGET_DIR:-target}/web-font
mkdir -p "$cache" "$out"
unicodes="U+0020-007E,U+00A0-00FF,U+2000-206F,U+2190-21FF,U+2460-24FF,U+25A0-25FF,U+3000-303F,U+3040-30FF,U+3100-312F,U+31F0-31FF,U+4E00-9FFF,U+FE30-FE4F,U+FF00-FFEF"
for variant in jp sc tc hk kr; do
  case "$variant" in
    jp) index=$base ;;
    kr) index=$((base + 1)) ;;
    sc) index=$((base + 2)) ;;
    tc) index=$((base + 3)) ;;
    hk) index=$((base + 4)) ;;
  esac
  ranges=$unicodes
  if [ "$variant" = kr ]; then ranges="$ranges,U+1100-11FF,U+3130-318F,U+AC00-D7AF"; fi
  font=$cache/cjk-$variant.otf
  # The source marker also invalidates the cache when the override changes.
  if [ ! -f "$font" ] || [ "$0" -nt "$font" ] || [ "$src" -nt "$font" ] \
     || [ "$(cat "$font.source" 2>/dev/null || true)" != "$src" ]; then
    "$subset" "$src" --font-number="$index" --unicodes="$ranges" \
      --layout-features='' --no-hinting --name-IDs='*' --name-languages='*' \
      --output-file="$font.part"
    want="Noto Sans CJK $(printf %s "$variant" | tr a-z A-Z)"
    # Full name (name ID 4) must match exactly: a Thin or Bold face shares the family name.
    got=$("$py" -c 'import sys; from fontTools.ttLib import TTFont
print(TTFont(sys.argv[1])["name"].getDebugName(4))' "$font.part")
    if [ "$got" != "$want" ]; then
      echo "web-font: face $index of $src is not $want" >&2
      rm -f "$font.part"
      exit 1
    fi
    mv "$font.part" "$font"
    printf '%s\n' "$src" >"$font.source"
  fi
  cp "$font" "$out/cjk-$variant.otf"
done
license=""
for l in "$(dirname "$src")/LICENSE" /usr/share/licenses/noto-fonts-cjk/LICENSE \
         /usr/share/doc/fonts-noto-cjk/copyright; do
  if [ -f "$l" ]; then license=$l; break; fi
done
{
  echo "cjk-*.otf: subsets of Noto Sans CJK Regular (https://github.com/notofonts/noto-cjk),"
  echo "licensed under the SIL Open Font License 1.1 (https://openfontlicense.org)."
  if [ -n "$license" ]; then echo; cat "$license"; fi
} >"$out/cjk-LICENSE.txt"
